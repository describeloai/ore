//! **¿Puede ESTA cuenta leer ESTE dataset?** Los tres permisos, uno a uno.
//!
//! El `check` de antes era un `SELECT 1` en el proyecto y decía `ok` aunque la
//! cuenta no pudiera ver el dataset (medido el 2026-09-27 con la cuenta de la
//! celda sobre `ore_e2e_ajeno`): contestaba a «¿hay proyecto?» y se leía como
//! «¿hay acceso?». Lo que la consola necesita para decirle al cliente qué
//! conceder son **tres respuestas**, porque son tres roles y se conceden en dos
//! sitios distintos:
//!
//! | permiso | qué se prueba | rol | dónde |
//! |---|---|---|---|
//! | `jobs` | un `SELECT 1` sin job | `roles/bigquery.jobUser` | el proyecto que paga |
//! | `datos` | listar las tablas del dataset | `roles/bigquery.dataViewer` | el dataset |
//! | `lectura` | abrir una sesión de Storage Read sobre una tabla | `roles/bigquery.readSessionUser` | el proyecto que paga |
//!
//! Nada de esto lee una fila ni cuesta: la sesión se abre y se suelta, y
//! `SELECT 1` no factura bytes. `datasets` va de regalo —los que esta cuenta
//! ve en el proyecto—: es lo que el wizard ofrece para elegir.
use crate::rest::{self, Transporte};
use ore_core::json::Json;

pub struct Permiso {
    pub nombre: &'static str,
    pub rol: &'static str,
    pub donde: String,
    /// `None` si no se pudo probar (un dataset sin tablas no deja abrir una
    /// sesión): ni sí ni no, y se dice.
    pub ok: Option<bool>,
    pub porque: Option<String>,
}

pub fn comprobar(t: &dyn Transporte, url: &str) -> Result<String, String> {
    let proyecto = crate::proyecto(url)?;
    let dataset = url
        .strip_prefix("bigquery://")
        .and_then(|r| r.split_once('/'))
        .map(|(_, d)| d.trim_matches('/').to_string())
        .filter(|d| !d.is_empty());

    let mut permisos = Vec::new();
    // ① jobs: lo mismo que el `check` de siempre.
    let jobs = rest::consultar(
        t,
        &proyecto,
        &rest::Consulta {
            texto: "SELECT 1 AS ok",
            parametros: &[],
            sin_job: true,
        },
        |_, _| Ok(()),
    );
    permisos.push(Permiso {
        nombre: "jobs",
        rol: "roles/bigquery.jobUser",
        donde: format!("proyecto {proyecto}"),
        ok: Some(jobs.is_ok()),
        porque: jobs.err(),
    });

    // ② datos, y ③ lectura sobre la primera tabla que se vea.
    if let Some(d) = &dataset {
        let r = t.get(
            &format!("projects/{proyecto}/datasets/{d}/tables"),
            &[("maxResults", "1".to_string())],
        );
        let primera = r.as_ref().ok().and_then(|v| {
            v["tables"][0]["tableReference"]["tableId"]
                .as_str()
                .map(String::from)
        });
        permisos.push(Permiso {
            nombre: "datos",
            rol: "roles/bigquery.dataViewer",
            donde: format!("dataset {proyecto}.{d}"),
            ok: Some(r.is_ok()),
            porque: r.err(),
        });
        let (ok, porque) = match &primera {
            None => (
                None,
                Some(
                    "sin una tabla visible en el dataset no se puede abrir una sesión de prueba"
                        .to_string(),
                ),
            ),
            Some(tabla) => {
                let ruta = format!("projects/{proyecto}/datasets/{d}/tables/{tabla}");
                match t
                    .token()
                    .and_then(|k| crate::flecha::sesion_de_prueba(&k, &proyecto, &ruta))
                {
                    Ok(()) => (Some(true), None),
                    Err(e) => (Some(false), Some(e)),
                }
            }
        };
        permisos.push(Permiso {
            nombre: "lectura",
            rol: "roles/bigquery.readSessionUser",
            donde: format!("proyecto {proyecto}"),
            ok,
            porque,
        });
    }

    let datasets = rest::datasets(t, &proyecto).unwrap_or_default();
    Ok(respuesta(&permisos, dataset.as_deref(), &datasets))
}

/// `ok` es la conjunción de lo que se pudo probar; sin dataset en la URL no se
/// afirma el acceso a datos que no se han mirado, así que `ok` es falso y se
/// dice por qué.
pub fn respuesta(permisos: &[Permiso], dataset: Option<&str>, datasets: &[String]) -> String {
    let fallo = permisos.iter().find(|p| p.ok == Some(false));
    let ok = dataset.is_some() && fallo.is_none();
    let porque = match (dataset, fallo) {
        // Un 404 es «no existe» o «no se ve»: BigQuery no distingue, así que
        // se dicen las dos cosas.
        (_, Some(p)) if p.porque.as_deref().is_some_and(|m| m.contains(" 404")) => Some(format!(
            "no existe, o esta cuenta no lo ve (`{}` en {}): {}",
            p.rol,
            p.donde,
            p.porque.as_deref().unwrap_or("")
        )),
        (_, Some(p)) => Some(format!(
            "falta `{}` en {}: {}",
            p.rol,
            p.donde,
            p.porque.as_deref().unwrap_or("denegado")
        )),
        (None, None) => Some("la URL no nombra dataset: `bigquery://<proyecto>/<dataset>`".into()),
        _ => None,
    };
    let permisos = Json::Obj(
        permisos
            .iter()
            .map(|p| {
                let mut c = vec![("donde", Json::s(&p.donde)), ("rol", Json::s(p.rol))];
                // Lo no probado no lleva `ok`: la forma canónica no tiene
                // nulos, y un `ok: false` diría que falta un rol que nadie miró.
                match p.ok {
                    Some(b) => c.push(("ok", Json::Bool(b))),
                    None => c.push(("probado", Json::Bool(false))),
                }
                if let Some(m) = &p.porque {
                    c.push(("porque", Json::s(m)));
                }
                (p.nombre.to_string(), Json::obj(c))
            })
            .collect(),
    );
    let mut o = vec![
        ("ok", Json::Bool(ok)),
        ("permisos", permisos),
        (
            "datasets",
            Json::Arr(datasets.iter().map(Json::s).collect()),
        ),
    ];
    if let Some(p) = porque {
        o.push(("porque", Json::s(p)));
    }
    Json::obj(o).jcs()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(nombre: &'static str, ok: Option<bool>) -> Permiso {
        Permiso {
            nombre,
            rol: "roles/x",
            donde: "proyecto p".into(),
            ok,
            porque: ok.filter(|b| !b).map(|_| "403".into()),
        }
    }

    /// Un permiso que falta hace `ok: false` y el motivo nombra el rol y dónde.
    #[test]
    fn un_permiso_que_falta_nombra_su_rol() {
        let r = respuesta(
            &[p("jobs", Some(true)), p("datos", Some(false))],
            Some("d"),
            &[],
        );
        assert!(r.contains("\"ok\":false"), "{r}");
        assert!(r.contains("falta `roles/x` en proyecto p: 403"), "{r}");
    }

    /// Lo que no se pudo probar no es un fallo ni un acierto: `probado: false`.
    #[test]
    fn lo_que_no_se_pudo_probar_es_null() {
        let r = respuesta(&[p("jobs", Some(true)), p("lectura", None)], Some("d"), &[]);
        assert!(r.contains("\"ok\":true"), "{r}");
        assert!(
            r.contains("\"lectura\":{\"donde\":\"proyecto p\",\"probado\":false"),
            "{r}"
        );
    }

    /// Sin dataset no se afirma el acceso a datos que nadie miró.
    #[test]
    fn sin_dataset_no_se_afirma_nada() {
        let r = respuesta(&[p("jobs", Some(true))], None, &["ventas".into()]);
        assert!(
            r.contains("\"ok\":false") && r.contains("no nombra dataset"),
            "{r}"
        );
        assert!(r.contains("\"datasets\":[\"ventas\"]"), "{r}");
    }
}
