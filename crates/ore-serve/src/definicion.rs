//! ⭐ **LAS ESCRITURAS DE DEFINICIÓN CON EL CATÁLOGO** (0079 §19.2, E1a·5c·5c·2): retirar un
//! `Dataset`, retirar una base y renombrar un schema.
//!
//! Cambian a la vez un documento (en git) y punteros que ya existen (en el catálogo), sin una
//! transacción común. La regla: **git primero, y git lleva la intención.**
//!
//! 1. La ruta corre como siempre, por [`Servidor::definiendo`]: con el catálogo, toda puerta que
//!    se abra mientras tanto **graba** en vez de escribir (`crate::lugar::Grabando`, también la
//!    heredada de `ore`, que es quien renombra). Lo grabado es el plan.
//! 2. La costura (`escribiendo`, `escribiendo_en`) publica el commit de los documentos con el plan
//!    al final del mensaje ([`con_plan`]: el bloque `Punteros:` de
//!    `ore_core::punteros::grabadora`). Si git dice que no, no hay nada que aplicar.
//! 3. Con el sha, [`aplicar`]: el catálogo escribe lo grabado con `idempotencia = git:<sha>`
//!    (aplicarlo dos veces no hace nada). Si eso no llega a pasar —una caída justo después de
//!    git—, lo aplica el reconciliador (·5c·5c·4) leyendo el bloque del commit.
//!
//! **Renombrar cierra su schema** mientras graba ([`Cierre`]): graba el contenido de cada puntero
//! en el momento de leerlo, y una copia que escribiera entre la grabación y la aplicación se
//! perdería. Con el cerrojo, esa copia recibe 503 (se reintenta) y el cerrojo se suelta en la
//! misma transacción que aplica. Retirar no cierra: retirar gana a cualquier escritura.
//!
//! Con ficheros, nada de esto: [`Servidor::definiendo`] es `escribiendo_en`.

use std::cell::RefCell;
use std::path::Path;
use std::time::Duration;

use ore_core::json::Json;
use ore_core::punteros::grabadora::{Bloque, bloque};
use ore_core::punteros::{Cambio, Contexto, Punteros};
use ore_entrada::http::Respuesta;
use ore_entrada::identidad::Identidad;

use crate::rutas::Servidor;

/// Cuánto dura el cerrojo de renombrar: lo que tarda git con holgura. Si ore-serve cae, caduca.
pub(crate) const PLAZO: Duration = Duration::from_secs(120);

/// **Lo que una definición cierra** mientras graba: los punteros de un schema (los de su nombre
/// viejo y los del nuevo), en la ref en curso.
pub(crate) struct Cierre {
    pub(crate) base: String,
    pub(crate) de: String,
    pub(crate) a: String,
}

thread_local! {
    /// El dueño del cerrojo de la definición en curso, si cerró algo.
    static DUENO: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Lo grabado por la definición en curso: los cambios, su contexto y el dueño de su cerrojo.
pub(crate) struct Plan {
    cambios: Vec<Cambio>,
    ctx: Contexto,
    dueno: Option<String>,
}

/// El plan de la definición en curso en este hilo, si grabó algo.
pub(crate) fn grabado() -> Option<Plan> {
    let (cambios, ctx) = crate::lugar::cinta_en_curso()?.grabado()?;
    Some(Plan {
        cambios,
        ctx,
        dueno: DUENO.with(|d| d.borrow().clone()),
    })
}

/// `mensaje` con el plan al final, si lo hay.
pub(crate) fn con_plan(mensaje: &str, rama: Option<&str>, plan: Option<&Plan>) -> String {
    match plan {
        Some(p) => bloque(
            mensaje,
            &Bloque::Definicion {
                r: rama.unwrap_or("main").to_string(),
                operacion: p.ctx.operacion.clone(),
                cambios: p.cambios.clone(),
            },
        ),
        None => mensaje.to_string(),
    }
}

/// **Aplica el plan** que git confirmó en `sha` (`None`: no hubo commit —un árbol que es un
/// directorio, o nada que publicar—, y se escribe sin él). Si no puede, la respuesta lo dice
/// (`punteros: pendiente`): git ya dijo que sí, y el reconciliador lo terminará.
pub(crate) fn aplicar(
    plan: &Plan,
    rama: Option<&str>,
    sha: Option<&str>,
    sujeto: &Identidad,
    r: &mut Respuesta,
) {
    let ctx = Contexto {
        autor: sujeto.persona.clone(),
        operacion: plan.ctx.operacion.clone(),
        idempotencia: None,
    };
    let hecho = crate::lugar::en_el_catalogo(rama, |c| match sha {
        Some(sha) => c.aplicar_definicion(sha, &ctx, &plan.cambios, plan.dueno.as_deref()),
        None => {
            if let Some(d) = &plan.dueno {
                c.soltar_de(d)?;
            }
            c.escribir(&plan.cambios, &ctx)
        }
    })
    .map(|r| r.and_then(|x| x));
    let dicho = match hecho {
        None => Json::s("sin catálogo"),
        Some(Ok(())) => Json::obj([
            ("aplicados", Json::Int(plan.cambios.len() as i64)),
            ("commit", sha.map(Json::s).unwrap_or(Json::Bool(false))),
        ]),
        Some(Err(e)) => {
            eprintln!(
                "⚠ 0079 ·5c·5c·2: `{}` en `{}` quedó en git ({}) y NO en el catálogo: {e}",
                plan.ctx.operacion,
                rama.unwrap_or("main"),
                sha.unwrap_or("sin commit")
            );
            Json::s(format!("pendiente: {e}"))
        }
    };
    if let Json::Obj(m) = &mut r.cuerpo {
        m.insert("punteros".into(), dicho);
    }
}

/// Tras un commit publicado: aplica con su sha entero. Si no se pudo saber, no se aplica con otra
/// clave (el reconciliador lo aplicaría otra vez con la buena): queda pendiente para él.
pub(crate) fn tras_el_commit(
    plan: &Plan,
    rama: Option<&str>,
    sha: Result<String, crate::git::Fallo>,
    sujeto: &Identidad,
    r: &mut Respuesta,
) {
    match sha {
        Ok(sha) => aplicar(plan, rama, Some(&sha), sujeto, r),
        Err(e) => {
            if let Json::Obj(m) = &mut r.cuerpo {
                m.insert(
                    "punteros".into(),
                    Json::s(format!("pendiente: no se supo el sha del commit: {e}")),
                );
            }
        }
    }
}

/// Lo que una definición cerró y no llegó a aplicar (la ruta falló, git dijo que no), suelto.
fn soltar(rama: Option<&str>) {
    if let Some(d) = DUENO.with(|x| x.borrow_mut().take()) {
        let _ = crate::lugar::en_el_catalogo(rama, |c| c.soltar_de(&d));
    }
}

/// Las rutas de los punteros de `base.de.*` y sus nombres en `base.a.*`, en la ref.
fn rutas_del_schema(c: &ore_catalogo::Catalogo, cierre: &Cierre) -> Result<Vec<String>, String> {
    let todos = c.todos().map_err(|e| e.to_string())?;
    let mut rutas = Vec::new();
    for k in todos.keys() {
        let Some((b, s, n)) = ore_core::punteros::partes(k) else {
            continue;
        };
        if b != cierre.base || s != cierre.de {
            continue;
        }
        for nombre in [
            ore_core::normalize::corto(b, &cierre.de, n),
            ore_core::normalize::corto(b, &cierre.a, n),
        ] {
            rutas.extend(ore_catalogo::puerta::rutas_del_nombre(&nombre).unwrap_or_default());
        }
    }
    rutas.sort();
    rutas.dedup();
    Ok(rutas)
}

impl Servidor {
    /// **Una escritura de definición**: `escribiendo_en`, grabando con el catálogo (ver el
    /// módulo). `cierre`: lo que se cierra mientras graba (renombrar).
    pub(crate) fn definiendo(
        &self,
        rama: Option<&str>,
        sujeto: &Identidad,
        mensaje: &str,
        cierre: Option<Cierre>,
        f: impl FnOnce(&Path) -> Respuesta,
    ) -> Respuesta {
        let _g = crate::lugar::Grabando::abrir();
        let en = rama.filter(|r| !self.es_la_rama_por_defecto(r));
        if let Some(cierre) = cierre
            && crate::lugar::con_catalogo()
        {
            let dueno = format!(
                "renombrar:{}.{}:{}",
                cierre.base,
                cierre.de,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
            );
            let r = en.unwrap_or("main").to_string();
            let cerrado = crate::lugar::en_el_catalogo(en, |c| {
                let rutas = rutas_del_schema(c, &cierre)?;
                if rutas.is_empty() {
                    return Ok(false);
                }
                c.cerrar(&[r], &rutas, &dueno, PLAZO)
                    .map(|()| true)
                    .map_err(|f| match f {
                        ore_catalogo::almacen::FalloCerrar::Ocupado { ruta, dueno, .. } => {
                            format!("`{ruta}` está en una operación en curso (`{dueno}`)")
                        }
                        ore_catalogo::almacen::FalloCerrar::Otro(m) => m,
                    })
            });
            match cerrado {
                Some(Ok(Ok(true))) => DUENO.with(|d| *d.borrow_mut() = Some(dueno)),
                Some(Ok(Ok(false))) | None => {}
                Some(Ok(Err(m))) => {
                    return Respuesta::error(429, format!("{m}: reintenta en unos segundos"));
                }
                Some(Err(e)) => return Respuesta::error(e.codigo(), e.to_string()),
            }
        }
        let r = self.escribiendo_en(rama, sujeto, mensaje, f);
        // Lo que cerró y no aplicó (aplicar ya lo soltó: entonces es nada).
        soltar(en);
        r
    }
}
