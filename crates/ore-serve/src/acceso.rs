//! **`ore-serve` pregunta al puente** (0047 A5, P2 de 0044 B.7).
//!
//! Con `--acceso <ore-iam>`, tres cosas dejan de ser de «cualquiera con sesión» y
//! pasan a ser de quien tenga la potestad en su organización:
//!
//! | ruta | potestad | sin ella |
//! |---|---|---|
//! | `POST /fuentes` | `fuente:crear` | 403 |
//! | `PUT /ramas/main/proteccion` | `rama:proteger` | 403 |
//! | fusionar en una `main` protegida sin la aprobación de otra persona | `propuesta:fusionar-sin-revision` | la regla de siempre (422) |
//!
//! Y en las tres, lo hecho va a la huella de la organización (`hizo`). Saltarse
//! la revisión se anota ANTES de fusionar: si no se puede anotar, no se fusiona.
//!
//! Sin `--acceso`, nada cambia: es lo que el árbol era, y lo que un banco sin
//! `ore-iam` necesita.
//!
//! # El token de quien pide, por hilo
//!
//! `puede` necesita el token con el que la persona llegó (`Ore-Sujeto`), y la
//! `Identidad` no lo guarda. `ore-entrada` atiende cada petición en su hilo, así
//! que el token se deja en una variable del hilo al entrar y se quita al salir
//! ([`con_testigo`]), en vez de pasarlo por todas las firmas del camino.

use crate::rutas::Servidor;
use ore_acceso::{Decision, Evento, Hecho, Recurso};
use ore_core::json::Json;
use ore_entrada::http::{Peticion, Respuesta};
use ore_entrada::identidad::Identidad;
use std::cell::RefCell;

thread_local! {
    static TESTIGO: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Atiende con el token de la petición a mano, y lo quita al terminar.
pub fn con_testigo<T>(p: &Peticion, f: impl FnOnce() -> T) -> T {
    let t = p
        .cabeceras
        .get("authorization")
        .and_then(|v| {
            v.strip_prefix("Bearer ")
                .or_else(|| v.strip_prefix("bearer "))
        })
        .map(|v| v.trim().to_string());
    TESTIGO.with(|c| *c.borrow_mut() = t);
    let r = f();
    TESTIGO.with(|c| *c.borrow_mut() = None);
    r
}

fn testigo() -> Option<String> {
    TESTIGO.with(|c| c.borrow().clone())
}

/// Lo que dice el puente sobre saltarse una revisión.
pub enum Salto {
    /// No hay puente, o no tiene la potestad: la regla de siempre.
    No,
    /// Tiene la potestad; el `id` de la decisión va en la huella.
    Si(String),
    /// No hubo quien decidiera: 503, no la regla de siempre.
    SinRespuesta(Respuesta),
}

impl Servidor {
    /// Exige una potestad de la organización. `Ok(None)` sin puente (lo de
    /// siempre); `Ok(Some(decision))` si puede; `Err` con la respuesta que toca.
    pub(crate) fn exigir(
        &self,
        sujeto: &Identidad,
        accion: &str,
        ruta: &str,
    ) -> Result<Option<String>, Respuesta> {
        let Some(acceso) = self.acceso.as_ref() else {
            return Ok(None);
        };
        let Some(t) = testigo() else {
            return Err(Respuesta::error(
                401,
                "esta ruta necesita el token de quien pide",
            ));
        };
        match acceso.puede(&t, &sujeto.persona, accion, Recurso::ORGANIZACION, ruta) {
            Decision::Permite { id } => Ok(Some(id)),
            d => Err(respuesta_de(&d)),
        }
    }

    /// ¿Puede saltarse la revisión? Para fusionar en una `main` protegida.
    pub(crate) fn saltarse_la_revision(&self, sujeto: &Identidad, ruta: &str) -> Salto {
        let (Some(acceso), Some(t)) = (self.acceso.as_ref(), testigo()) else {
            return Salto::No;
        };
        match acceso.puede(
            &t,
            &sujeto.persona,
            "propuesta:fusionar-sin-revision",
            Recurso::ORGANIZACION,
            ruta,
        ) {
            Decision::Permite { id } => Salto::Si(id),
            Decision::Niega { .. } => Salto::No,
            d => Salto::SinRespuesta(respuesta_de(&d)),
        }
    }

    /// «¿Podría saltarse la revisión?», para la ficha de una propuesta: una
    /// consulta, que no deja huella si niega (nadie intentó nada).
    pub(crate) fn podria_saltarse_la_revision(&self, sujeto: &Identidad, ruta: &str) -> Salto {
        let (Some(acceso), Some(t)) = (self.acceso.as_ref(), testigo()) else {
            return Salto::No;
        };
        match acceso.podria(
            &t,
            &sujeto.persona,
            "propuesta:fusionar-sin-revision",
            Recurso::ORGANIZACION,
            ruta,
        ) {
            Decision::Permite { id } => Salto::Si(id),
            Decision::Niega { .. } => Salto::No,
            d => Salto::SinRespuesta(respuesta_de(&d)),
        }
    }

    /// Lo que se hizo, a la huella de la organización. No hace fallar lo hecho:
    /// si no llega, espera y se reintenta; si ni eso, se dice en el registro.
    pub(crate) fn contar(&self, e: Evento) {
        let Some(acceso) = self.acceso.as_ref() else {
            return;
        };
        match acceso.hizo(testigo().as_deref(), &e) {
            Ok(Hecho::Encolado(m)) => eprintln!("acceso · `{}` espera: {m}", e.operacion),
            Ok(_) => {}
            Err(m) => eprintln!("acceso · ✗ `{}` sin huella: {m}", e.operacion),
        }
    }

    /// Lo que se va a hacer y no puede quedarse sin rastro, ANTES de hacerlo.
    /// `Err` con un 503: la ruta no actúa.
    pub(crate) fn contar_antes(&self, e: &Evento) -> Result<(), Respuesta> {
        let Some(acceso) = self.acceso.as_ref() else {
            return Ok(());
        };
        acceso.hizo_antes(testigo().as_deref(), e).map(|_| ()).map_err(|m| {
            Respuesta {
                codigo: 503,
                cuerpo: Json::obj([
                    (
                        "error",
                        Json::s(format!(
                            "no se pudo dejar la huella de `{}` antes de actuar, así que no se actúa: {m}",
                            e.operacion
                        )),
                    ),
                    ("reintentar", Json::Bool(true)),
                ]),
            }
        })
    }
}

/// Un evento con lo de siempre relleno.
pub(crate) fn evento(
    operacion: &str,
    sobre: &str,
    resultado: &str,
    decision: Option<String>,
    commit: Option<String>,
) -> Evento {
    Evento {
        id: ore_acceso::nuevo_id(),
        operacion: operacion.to_string(),
        sobre: sobre.to_string(),
        resultado: resultado.to_string(),
        decision,
        commit,
        abre: None,
        detalle: None,
    }
}

/// Lo que ve la persona cuando no pasa: 403 con lo que le falta y la decisión,
/// 503 si no hubo quien decidiera, 401 si su token dejó de valer.
fn respuesta_de(d: &Decision) -> Respuesta {
    match d {
        Decision::Niega { id, motivo } => Respuesta {
            codigo: 403,
            cuerpo: Json::obj([("error", Json::s(motivo)), ("decision", Json::s(id))]),
        },
        Decision::SinRespuesta { motivo } => Respuesta {
            codigo: 503,
            cuerpo: Json::obj([("error", Json::s(motivo)), ("reintentar", Json::Bool(true))]),
        },
        Decision::SujetoInvalido { motivo } => Respuesta::error(401, motivo.clone()),
        Decision::Permite { .. } => Respuesta::error(500, "un permiso no es un error"),
    }
}

/// El `commit` de una respuesta de `escribiendo`, si lo trae.
pub(crate) fn commit_de(r: &Respuesta) -> Option<String> {
    match &r.cuerpo {
        Json::Obj(m) => match m.get("commit") {
            Some(Json::Str(c)) => Some(c.clone()),
            _ => None,
        },
        _ => None,
    }
}
