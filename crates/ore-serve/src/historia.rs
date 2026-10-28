//! ⭐ **LA HISTORIA DE UN ACTIVO** (Escrituras E4).
//!
//! | | Qué |
//! |---|---|
//! | `GET /historia/{ref}` | por versión publicada de un dataset o una colección, de la más nueva a la más vieja: cuándo, quién, filas, operación y **la escritura** (desde dónde, la sentencia, lo leído) |
//!
//! Como la historia de las promesas (0063 S7f), sale del historial del
//! puntero en git: cada versión dejó su commit, y la escritura viaja en él
//! (E2). Nada que guardar aparte. Un puntero de antes de E2 dice su
//! `procedencia`, que se lee igual (`Escritura::de_json`).

use crate::rutas::Servidor;
use ore_core::json::Json;
use ore_entrada::http::Respuesta;
use std::path::Path;

/// Cuántas versiones enseña la entrada *History* del Overview.
const VERSIONES: usize = 50;

impl Servidor {
    /// **`GET /historia/{ref}`**.
    pub(crate) fn historia_del_activo(&self, raiz: &Path, referencia: &str) -> Respuesta {
        let qn = match referencia.split_once(':') {
            Some(("dataset" | "collection", qn)) => qn,
            _ => {
                return Respuesta::error(
                    422,
                    format!("`{referencia}` is not `dataset:b.s.n` or `collection:b.s.n`"),
                );
            }
        };
        // Por la puerta (0079 ·5b·3): lo heredado de `main` tiene la historia
        // de `main`, y la ruta de antes cuenta. Sin historia (un directorio sin
        // git), ninguna versión, como antes.
        let corto = ore_core::normalize::a_corto(qn);
        let historia = match crate::lugar::Lugar::de(raiz)
            .punteros()
            .historia(&corto, VERSIONES * 4)
        {
            Ok(h) => h,
            Err(ore_core::punteros::Fallo::Nombre(_)) => {
                return Respuesta::error(422, format!("`{qn}` is not `base.schema.name`"));
            }
            Err(_) => Vec::new(),
        };
        let punteros = historia.into_iter().map(|v| (v.id, v.cuando, v.texto));
        Respuesta::ok(Json::obj([
            ("ref", Json::s(referencia)),
            ("versiones", Json::Arr(versiones(punteros))),
        ]))
    }
}

/// Las versiones, de los punteros del más nuevo al más viejo (`commit`,
/// fecha del commit, texto). Una versión es un snapshot (un dataset) o una
/// transacción (una colección); varios commits pueden decir la misma (una
/// comprobación, la recogida): cuenta una vez, con lo del commit que la trajo.
pub(crate) fn versiones(punteros: impl Iterator<Item = (String, String, String)>) -> Vec<Json> {
    let mut v: Vec<(String, Json)> = Vec::new();
    for (commit, fecha, texto) in punteros {
        let Ok(n) = ore_core::parse::parse(&texto) else {
            continue;
        };
        let p = Json::de_node_fiel(&n);
        let Json::Obj(m) = &p else { continue };
        let txt = |k: &str| match m.get(k) {
            Some(Json::Str(s)) if !s.is_empty() => Some(s.clone()),
            _ => None,
        };
        let Some(version) = txt("snapshot").or_else(|| txt("transaccion")) else {
            continue;
        };
        let mut e = vec![
            ("version", Json::s(&version)),
            ("cuando", Json::s(txt("cuando").unwrap_or(fecha))),
            ("commit", Json::s(&commit)),
        ];
        for k in ["escrito_por", "operacion", "estado", "datos_de"] {
            if let Some(s) = txt(k) {
                e.push((k, Json::s(s)));
            }
        }
        if let Some(f @ Json::Int(_)) = m.get("filas") {
            e.push(("filas", f.clone()));
        }
        if let Some(x) = ore_core::escritura::del_puntero(&p) {
            e.push(("escritura", x.a_json()));
        }
        let fila = Json::obj(e);
        if let Some((u, f)) = v.last_mut()
            && *u == version
        {
            *f = fila;
        } else if v.len() >= VERSIONES {
            break;
        } else {
            v.push((version, fila));
        }
    }
    v.into_iter().map(|(_, f)| f).collect()
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn p(h: &str, t: &str) -> (String, String, String) {
        (h.into(), "2026-10-23T10:00:00+02:00".into(), t.into())
    }

    /// E4 · Cada versión una vez, con lo que dijo el commit que la trajo, y la
    /// escritura en su forma nueva aunque el puntero sea de antes (procedencia).
    #[test]
    fn cada_version_una_vez_con_su_escritura() {
        let nueva = r#"{"snapshot":"3","filas":10,"operacion":"overwrite","escrito_por":"user:ana",
            "cuando":"2026-10-23T09:00:00Z",
            "escritura":{"desde":{"tipo":"query","id":"queries/r.sql","sentencia":"create or replace dataset v.s.r as select 1"},
                         "lee":[{"nombre":"v.s.p","en_vivo":false}]}}"#;
        let comprobada = r#"{"snapshot":"3","filas":10,"comprobado":"x"}"#;
        let vieja = r#"{"snapshot":"2","filas":4,"procedencia":{"puesto":"p","leidas":["v.s.q"]}}"#;
        let sin = r#"{"estado":"error","motivo":"no"}"#;
        let v = versiones(
            [
                p("c4", comprobada),
                p("c3", nueva),
                p("c2", sin),
                p("c1", vieja),
            ]
            .into_iter(),
        );
        assert_eq!(v.len(), 2, "{v:?}");
        let j = v[0].jcs();
        assert!(j.contains(r#""commit":"c3""#), "{j}");
        assert!(j.contains(r#""cuando":"2026-10-23T09:00:00Z""#), "{j}");
        assert!(
            j.contains(r#""sentencia":"create or replace dataset v.s.r as select 1""#),
            "{j}"
        );
        assert!(j.contains(r#""escrito_por":"user:ana""#), "{j}");
        let j = v[1].jcs();
        assert!(j.contains(r#""version":"2""#) && j.contains("v.s.q"), "{j}");
        // Sin `cuando` en el puntero, la fecha del commit.
        assert!(j.contains("2026-10-23T10:00:00+02:00"), "{j}");
    }
}
