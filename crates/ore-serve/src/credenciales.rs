//! **Con qué credencial lee una fuente, y qué tiene que conceder el cliente.**
//!
//! `GET /fuentes/credenciales/{tipo}`: lo que el wizard de alta enseña en el
//! paso de conexión. La tabla de roles vive AQUÍ y solo aquí: si la consola la
//! copiara, el día que cambie habría dos que divergen, y la que el cliente lee
//! sería la vieja.
//!
//! # BigQuery: la cuenta de la celda, y no una clave (medido el 2026-09-27)
//!
//! El driver se autentica con la cuenta de Google de la celda,
//! `ore-driver-<inquilino>@…`, por Workload Identity: no hay secreto que
//! guardar ni que filtrar. El cliente le concede tres roles en SU proyecto y
//! el driver lee. Las claves JSON de cuenta de servicio están **prohibidas por
//! defecto** en las organizaciones de Google creadas desde 2024
//! (`iam.disableServiceAccountKeyCreation`, activa en la nuestra): pedirle una
//! al cliente sería pedirle que rebaje su seguridad. Se dice como no admitida,
//! con el porqué, en vez de ofrecerla.
//!
//! Los tres roles son obligatorios —el catálogo es una consulta a
//! `INFORMATION_SCHEMA`, así que `jobUser` no es opcional aquí— y se conceden
//! en dos sitios: dos en el proyecto que paga, uno en cada dataset.
use ore_core::json::Json;

/// Un rol que el cliente concede: cuál, en qué nivel, para qué, y el comando
/// con `{proyecto}`, `{dataset}` y `{cuenta}` por rellenar.
struct Rol {
    rol: &'static str,
    nivel: &'static str,
    para: &'static str,
    comando: &'static str,
}

const BIGQUERY: &[Rol] = &[
    Rol {
        rol: "roles/bigquery.jobUser",
        nivel: "proyecto",
        para: "ejecutar las consultas del catálogo y de las vistas, en el proyecto que paga",
        comando: "gcloud projects add-iam-policy-binding {proyecto} --member=serviceAccount:{cuenta} --role=roles/bigquery.jobUser --condition=None",
    },
    Rol {
        rol: "roles/bigquery.readSessionUser",
        nivel: "proyecto",
        para: "leer las tablas en Arrow por la Storage Read API, en el proyecto que paga",
        comando: "gcloud projects add-iam-policy-binding {proyecto} --member=serviceAccount:{cuenta} --role=roles/bigquery.readSessionUser --condition=None",
    },
    Rol {
        rol: "roles/bigquery.dataViewer",
        nivel: "dataset",
        para: "ver las tablas y leer sus filas, solo en ese dataset",
        comando: "bq add-iam-policy-binding --member=serviceAccount:{cuenta} --role=roles/bigquery.dataViewer {proyecto}:{dataset}",
    },
];

/// La respuesta de `GET /fuentes/credenciales/{tipo}`. `cuenta` es la de la
/// celda (`--cuenta-driver`); sin ella el modo se enseña igual, sin email, y
/// se dice por qué falta.
pub fn de(tipo: &str, cuenta: Option<&str>) -> Json {
    match tipo {
        "bigquery" => {
            let mut celda = vec![
                ("modo", Json::s("celda")),
                ("recomendado", Json::Bool(true)),
                (
                    "dice",
                    Json::s(
                        "El driver lee con la cuenta de Google de esta celda: no hay clave \
                         que guardar. Concédele estos roles en tu proyecto.",
                    ),
                ),
                (
                    "roles",
                    Json::Arr(
                        BIGQUERY
                            .iter()
                            .map(|r| {
                                Json::obj([
                                    ("rol", Json::s(r.rol)),
                                    ("nivel", Json::s(r.nivel)),
                                    ("para", Json::s(r.para)),
                                    ("comando", Json::s(r.comando)),
                                ])
                            })
                            .collect(),
                    ),
                ),
            ];
            match cuenta {
                Some(c) => celda.push(("cuenta", Json::s(c))),
                None => celda.push((
                    "sinCuenta",
                    Json::s(
                        "este servidor no sabe la cuenta del driver (`--cuenta-driver`): \
                         pregúntala a quien opera la celda",
                    ),
                )),
            }
            Json::obj([
                ("tipo", Json::s("bigquery")),
                ("modos", Json::Arr(vec![Json::obj(celda)])),
                (
                    "noAdmitidos",
                    Json::Arr(vec![Json::obj([
                        ("modo", Json::s("clave-json")),
                        (
                            "porque",
                            Json::s(
                                "Google prohíbe por defecto crear claves de cuenta de servicio \
                                 en las organizaciones nuevas (iam.disableServiceAccountKeyCreation); \
                                 pedirla sería pedirte que rebajes tu seguridad",
                            ),
                        ),
                    ])]),
                ),
            ])
        }
        // Las demás familias llevan su credencial dentro de la cadena de
        // conexión, que va al custodio: no hay nada que conceder aparte.
        otro => Json::obj([
            ("tipo", Json::s(otro)),
            (
                "modos",
                Json::Arr(vec![Json::obj([
                    ("modo", Json::s("cadena")),
                    ("recomendado", Json::Bool(true)),
                    (
                        "dice",
                        Json::s(
                            "la credencial va dentro de la cadena de conexión, cifrada en el custodio",
                        ),
                    ),
                ])]),
            ),
        ]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BigQuery: la cuenta de la celda con sus tres roles, y la clave JSON
    /// dicha como no admitida con su porqué.
    #[test]
    fn bigquery_ensena_la_cuenta_y_los_tres_roles() {
        let j = de(
            "bigquery",
            Some("ore-driver-demo@p.iam.gserviceaccount.com"),
        )
        .jcs();
        for r in [
            "roles/bigquery.jobUser",
            "roles/bigquery.readSessionUser",
            "roles/bigquery.dataViewer",
        ] {
            assert!(j.contains(r), "{j}");
        }
        assert!(
            j.contains("\"cuenta\":\"ore-driver-demo@p.iam.gserviceaccount.com\""),
            "{j}"
        );
        assert!(j.contains("\"modo\":\"clave-json\""), "{j}");
        assert!(j.contains("{proyecto}:{dataset}"), "{j}");
    }

    /// Sin `--cuenta-driver` el modo se enseña igual y se dice qué falta.
    #[test]
    fn sin_cuenta_se_dice() {
        let j = de("bigquery", None).jcs();
        assert!(j.contains("sinCuenta") && !j.contains("\"cuenta\""), "{j}");
    }

    /// Las demás familias: la credencial va en la cadena.
    #[test]
    fn postgres_va_en_la_cadena() {
        assert!(de("postgres", None).jcs().contains("\"modo\":\"cadena\""));
    }
}
