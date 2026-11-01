//! ⭐ **LAS FUSIONES CON EL CATÁLOGO** (0079 §19.2, E1a·5c·5c·3): con los punteros fuera de git,
//! fusionar una rama ya no mueve ficheros de punteros. Los datos se fusionan en el catálogo, con
//! el mismo protocolo que retirar y renombrar (`crate::definicion`) más un cerrojo:
//!
//! 1. **El plan**, sin escribir nada: `plan_fusion` del catálogo, con la regla de siempre
//!    (promoción, reconstruir, conflicto). Lo no elegido es el
//!    409 de hoy, con los mismos `datos`.
//! 2. **El cerrojo** sobre lo que el plan toca, en `main` y en la rama ([`cerrar`]): quien escriba
//!    ahí recibe 503 y reintenta. Caduca solo ([`PLAZO`]).
//! 3. **Git**: el merge de los documentos (la forja), con el plan al final del mensaje (el bloque
//!    `Punteros:`).
//! 4. **El catálogo aplica el plan** con el sha del merge (`idempotencia = git:<sha>`) y suelta el
//!    cerrojo ([`aplicar`]). Si el cerrojo caducó y algo se movió, se aplica como fusión tardía:
//!    gana lo que `main` escribió entretanto y lo demás se dice como pendiente.
//!
//! Lo que es «reconstruir» se encola como copia en `main`, como hoy. Con ficheros, nada de esto.

use std::collections::BTreeMap;
use std::time::Duration;

use ore_catalogo::almacen::{FalloAplicar, FalloCerrar, PlanFusion};
use ore_core::json::Json;
use ore_core::punteros::grabadora::{Bloque, bloque};
use ore_entrada::http::Respuesta;

/// Cuánto dura el cerrojo de una fusión: lo que tardan la validación y el merge con holgura.
pub(crate) const PLAZO: Duration = Duration::from_secs(300);

/// El nombre corto de una ruta de puntero (`datasets/b/s/n.json` → `b.s.n`), con el puntero por
/// si es uno de antes.
fn activo_de(ruta: &str, puntero: Option<&str>) -> String {
    ore_catalogo::puerta::nombre_de_la_ruta(ruta, puntero)
}

/// Un resumen de lo que `main` escribió (sus filas y quién): lo que se pierde si gana la rama.
fn resumen(t: Option<&str>) -> Option<String> {
    let n = ore_core::parse::parse(t?).ok()?;
    let c = |k: &str| n.get(k).and_then(|(_, v)| v.as_str()).map(String::from);
    Some(format!(
        "{} fila(s){}",
        c("filas").unwrap_or_else(|| "?".into()),
        c("escrito_por")
            .map(|p| format!(", escrito por {p}"))
            .unwrap_or_default()
    ))
}

/// Los `datos` de una fusión: por activo, su caso y lo que queda.
pub(crate) fn datos(plan: &PlanFusion) -> Vec<Json> {
    plan.pasos
        .iter()
        .map(|p| {
            let mut m = vec![
                ("ruta", Json::s(&p.ruta)),
                (
                    "activo",
                    Json::s(activo_de(&p.ruta, p.rama.as_deref().or(p.main.as_deref()))),
                ),
                ("caso", Json::s(&p.caso)),
                ("resultado", Json::s(&p.resultado)),
            ];
            if p.caso == "conflicto"
                && p.resultado != "main"
                && let Some(s) = resumen(p.main.as_deref())
            {
                m.push((
                    "se_pierde",
                    Json::s(format!("lo de main desde el punto de salida: {s}")),
                ));
            }
            Json::obj(m)
        })
        .collect()
}

/// ¿Algún dato se reconstruye en `main`?
pub(crate) fn reconstruye(plan: &PlanFusion) -> bool {
    plan.pasos.iter().any(|p| p.caso == "reconstruir")
}

/// **El plan** de `rama` en `main`. `rutas`: el alcance (`None`, todo lo propio). `elegidos`, por
/// nombre.
pub(crate) fn planear(
    rama: &str,
    rutas: Option<&[String]>,
    elegidos: &BTreeMap<String, String>,
) -> Result<PlanFusion, Respuesta> {
    crate::lugar::en_el_catalogo(Some(rama), |c| c.plan_fusion(rutas, elegidos))
        .and_then(|x| x)
        .map_err(|e| Respuesta::error(e.codigo(), format!("el plan de los datos: {e}")))
}

/// El 409 de un plan con algo sin elegir: el mensaje y los `datos` de hoy.
pub(crate) fn sin_elegir(rama: &str, plan: &PlanFusion) -> Respuesta {
    let nombres: Vec<String> = plan
        .pasos
        .iter()
        .filter(|p| p.resultado == "sin elegir")
        .map(|p| {
            format!(
                "`{}`",
                activo_de(&p.ruta, p.rama.as_deref().or(p.main.as_deref()))
            )
        })
        .collect();
    Respuesta {
        codigo: 409,
        cuerpo: Json::obj([
            (
                "error",
                Json::s(format!(
                    "`main` y `{rama}` escribieron los dos {} desde que se separaron, y no se puede reconstruir: elige cuál queda (`datos: {{\"<dataset>\": \"main\" | \"rama\"}}`)",
                    nombres.join(", ")
                )),
            ),
            ("datos", Json::Arr(datos(plan))),
        ]),
    }
}

/// **Cierra** lo que el plan toca, en `main` y en `rama`, a nombre de `dueno`. Un plan vacío no
/// cierra nada.
pub(crate) fn cerrar(rama: &str, plan: &PlanFusion, dueno: &str) -> Result<(), Respuesta> {
    let rutas: Vec<String> = plan.pasos.iter().map(|p| p.ruta.clone()).collect();
    if rutas.is_empty() {
        return Ok(());
    }
    let refs = [rama.to_string(), "main".to_string()];
    match crate::lugar::en_el_catalogo(Some(rama), |c| c.cerrar(&refs, &rutas, dueno, PLAZO)) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(FalloCerrar::Ocupado { ruta, dueno, .. })) => Err(Respuesta::error(
            429,
            format!(
                "`{ruta}` está en otra fusión en curso (`{dueno}`): reintenta en unos segundos"
            ),
        )),
        Ok(Err(FalloCerrar::Otro(m))) => Err(Respuesta::error(503, m)),
        Err(e) => Err(Respuesta::error(e.codigo(), e.to_string())),
    }
}

/// Suelta lo de `dueno` (una fusión que no llegó a aplicar).
pub(crate) fn soltar(rama: &str, dueno: &str) {
    let _ = crate::lugar::en_el_catalogo(Some(rama), |c| c.soltar_de(dueno));
}

/// El mensaje del merge con el plan al final.
pub(crate) fn mensaje(mensaje: &str, rama: &str, plan: &PlanFusion) -> String {
    let plan_j = ore_core::parse::parse(&plan.texto)
        .map(|n| Json::de_node_fiel(&n))
        .unwrap_or_else(|_| Json::Crudo(plan.texto.clone()));
    bloque(
        mensaje,
        &Bloque::Fusion {
            rama: rama.to_string(),
            plan: plan_j,
        },
    )
}

/// **Aplica el plan** que viajó en el merge `sha` y suelta el cerrojo. Si algo se movió con el
/// cerrojo puesto (caducó), como fusión tardía. Lo que pasó, para la respuesta.
pub(crate) fn aplicar(
    rama: &str,
    sha: Option<&str>,
    sujeto: &str,
    plan: &PlanFusion,
    dueno: &str,
) -> Json {
    let Some(sha) = sha else {
        soltar(rama, dueno);
        return Json::s("pendiente: no se supo el sha del merge");
    };
    let hecho = crate::lugar::en_el_catalogo(Some(rama), |c| {
        let r = match c.aplicar_plan(sha, sujeto, &plan.texto, false) {
            Err(FalloAplicar::Protocolo(_)) => c.aplicar_plan(sha, sujeto, &plan.texto, true),
            r => r,
        };
        let _ = c.soltar_de(dueno);
        r
    });
    match hecho {
        Ok(Ok(a)) => Json::obj([
            ("commit", Json::s(sha)),
            (
                "aplicados",
                Json::Arr(
                    a.aplicados
                        .iter()
                        .map(|r| Json::s(activo_de(r, None)))
                        .collect(),
                ),
            ),
            ("tardia", Json::Bool(a.tardia)),
            (
                "pendientes",
                Json::Arr(
                    a.pendientes
                        .iter()
                        .map(|p| Json::s(activo_de(&p.ruta, p.se_queda.as_deref())))
                        .collect(),
                ),
            ),
        ]),
        Ok(Err(e)) => {
            eprintln!(
                "⚠ 0079 ·5c·5c·3: la fusión de `{rama}` quedó en git ({sha}) y NO en el catálogo: {e:?}"
            );
            Json::s(format!("pendiente: {e:?}"))
        }
        Err(e) => {
            eprintln!(
                "⚠ 0079 ·5c·5c·3: la fusión de `{rama}` quedó en git ({sha}) y NO en el catálogo: {e}"
            );
            Json::s(format!("pendiente: {e}"))
        }
    }
}

/// **La huella de los datos** de `rama` en `rutas` (lo propio de la rama, entero): lo que entra en
/// la huella de una propuesta para que reescribir un puntero del alcance invalide la aprobación,
/// como cuando el puntero era un fichero del parche (§11.5 punto 7). `None` sin catálogo o sin
/// rutas.
pub(crate) fn huella_de_datos(rama: &str, rutas: &[String]) -> Option<String> {
    if rutas.is_empty() {
        return None;
    }
    let propios = crate::lugar::en_el_catalogo(Some(rama), |c| c.propios())
        .and_then(|x| x)
        .ok()?;
    let lineas: Vec<String> = propios
        .iter()
        .filter(|p| rutas.contains(&p.ruta))
        .map(|p| format!("{}={}", p.ruta, p.despues.as_deref().unwrap_or("∅")))
        .collect();
    // FNV-1a: corto y estable, como el nombre de una derivada.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in lineas.join("\n").bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    Some(format!("datos-{h:016x}"))
}

/// El sha del merge de la propuesta `pr` ya fusionada (lo que dice la forja).
pub(crate) fn sha_del_merge(pr: &Json) -> Option<String> {
    match pr {
        Json::Obj(m) => match m.get("merge_commit_sha") {
            Some(Json::Str(s)) if !s.is_empty() => Some(s.clone()),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use ore_catalogo::almacen::PasoFusion;

    fn paso(ruta: &str, caso: &str, resultado: &str) -> PasoFusion {
        PasoFusion {
            ruta: ruta.into(),
            caso: caso.into(),
            resultado: resultado.into(),
            rama: Some("{\"filas\": \"3\", \"escrito_por\": \"ana\"}".into()),
            main: Some("{\"filas\": \"9\", \"escrito_por\": \"luis\"}".into()),
            propio: true,
        }
    }

    /// Los `datos` y el 409 dicen lo mismo que decía la fusión por ficheros.
    #[test]
    fn los_datos_de_un_plan_como_los_de_hoy() {
        let plan = PlanFusion {
            sin_elegir: true,
            pasos: vec![
                paso("datasets/ventas/default/a.json", "promocion", "rama"),
                paso("datasets/ventas/eu/b.json", "conflicto", "sin elegir"),
                paso("datasets/ventas/default/c.json", "reconstruir", "main"),
            ],
            texto: "{\"estado\": \"sin elegir\"}".into(),
        };
        let d = datos(&plan);
        assert_eq!(d.len(), 3);
        assert!(d[0].jcs().contains("\"activo\":\"ventas.a\""));
        assert!(d[1].jcs().contains("ventas.eu.b"));
        assert!(d[1].jcs().contains("9 fila(s), escrito por luis"));
        assert!(!d[0].jcs().contains("se_pierde"));
        assert!(reconstruye(&plan));
        let r = sin_elegir("ana/x", &plan);
        assert_eq!(r.codigo, 409);
        assert!(r.cuerpo.jcs().contains("`ventas.eu.b`"));
        assert!(!r.cuerpo.jcs().contains("`ventas.a`"));
    }

    #[test]
    fn el_plan_va_en_el_mensaje_del_merge() {
        let plan = PlanFusion {
            sin_elegir: false,
            pasos: vec![],
            texto: "{\"estado\": \"plan\", \"rama\": \"ana/x\", \"pasos\": []}".into(),
        };
        let m = mensaje("Fusionar #3 (ana)", "ana/x", &plan);
        match ore_core::punteros::grabadora::leer_bloque(&m) {
            Some(Ok(Bloque::Fusion { rama, plan })) => {
                assert_eq!(rama, "ana/x");
                assert!(plan.jcs().contains("\"estado\":\"plan\""));
            }
            otro => panic!("{otro:?}"),
        }
    }

    #[test]
    fn el_sha_del_merge_lo_dice_la_forja() {
        let pr = Json::obj([("merge_commit_sha", Json::s("abc123"))]);
        assert_eq!(sha_del_merge(&pr).as_deref(), Some("abc123"));
        assert_eq!(
            sha_del_merge(&Json::obj([("merge_commit_sha", Json::Bool(false))])),
            None
        );
    }
}
