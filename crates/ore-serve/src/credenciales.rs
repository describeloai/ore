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

/// La política de un usuario IAM que lee un bucket, con `{bucket}` por
/// rellenar: listar (y sus versiones) sobre el bucket, leer (y una versión)
/// sobre sus objetos. Es lo que `ore-read-s3 check` prueba, acción por acción.
const POLITICA_S3: &str = r#"{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Action": ["s3:ListBucket", "s3:ListBucketVersions"],
      "Resource": "arn:aws:s3:::{bucket}"
    },
    {
      "Effect": "Allow",
      "Action": ["s3:GetObject", "s3:GetObjectVersion"],
      "Resource": "arn:aws:s3:::{bucket}/*"
    }
  ]
}"#;

/// ⭐ 0046 E9b · La confianza del rol que el cliente crea en SU cuenta de AWS: sólo
/// las dos cuentas de Google de ESTA celda —la del driver, que lee, y la de
/// `ore-serve`, que sirve— por su ID único (`sub`, y `aud` = `azp`), y la
/// audiencia del token (`oaud`). `AssumeRoleWithWebIdentity` no admite
/// `ExternalId`: lo que aísla a un cliente de otro es que otra celda corre con
/// otras cuentas (medido: la de demo, `AccessDenied`).
const CONFIANZA_S3: &str = r#"{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Principal": { "Federated": "accounts.google.com" },
      "Action": "sts:AssumeRoleWithWebIdentity",
      "Condition": {
        "StringEquals": {
          "accounts.google.com:aud": ["{idDriver}", "{idServe}"],
          "accounts.google.com:sub": ["{idDriver}", "{idServe}"],
          "accounts.google.com:oaud": "sts.amazonaws.com"
        }
      }
    }
  ]
}"#;

/// Los IDs únicos de las dos cuentas de la celda, si el despliegue los dio
/// (`ORE_ID_DRIVER` y `ORE_ID_SERVE`, del ConfigMap `ids-de-la-celda` que
/// escribe el aprovisionador). No son secretos: son lo que el cliente pega en
/// la confianza de su rol.
pub fn ids_de_la_celda() -> Option<(String, String)> {
    let v = |k: &str| {
        std::env::var(k)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
    };
    Some((v("ORE_ID_DRIVER")?, v("ORE_ID_SERVE")?))
}

/// La respuesta de `GET /fuentes/credenciales/{tipo}`. `cuenta` es la de la
/// celda (`--cuenta-driver`); sin ella el modo se enseña igual, sin email, y
/// se dice por qué falta.
pub fn de(tipo: &str, cuenta: Option<&str>) -> Json {
    de_con(tipo, cuenta, ids_de_la_celda())
}

/// Lo mismo, con los IDs de la celda dados (las pruebas no tocan el entorno).
pub fn de_con(tipo: &str, cuenta: Option<&str>, ids: Option<(String, String)>) -> Json {
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
        // S3 (0046 E5): la clave de un usuario IAM va en la URL, al custodio;
        // lo que se enseña es la política que ese usuario necesita, que es
        // SOLO de lectura. Las cuatro acciones son las que `check` prueba y
        // `leer` usa; ninguna escribe.
        //
        // ⭐ 0046 E9b: y antes, el ROL —recomendado—: el cliente crea en su cuenta
        //   un rol con la misma política de lectura y una confianza en las dos
        //   cuentas de esta celda; la URL lleva su ARN y ningún secreto.
        "s3" => Json::obj([
            ("tipo", Json::s("s3")),
            (
                "modos",
                Json::Arr(vec![
                    modo_rol_s3(ids),
                    Json::obj([
                        ("modo", Json::s("cadena")),
                        ("recomendado", Json::Bool(false)),
                        (
                            "dice",
                            Json::s(
                                "La clave de acceso de un usuario IAM va dentro de la URL, cifrada \
                             en el custodio. Dale a ese usuario solo esta política: lectura, \
                             sobre este bucket.",
                            ),
                        ),
                        (
                            "formato",
                            Json::s(
                                "s3://<bucket>[/<prefijo>]?region=<región>&access_key_id=<clave>&secret_access_key=<secreto>",
                            ),
                        ),
                        ("politica", Json::s(POLITICA_S3)),
                    ]),
                ]),
            ),
            (
                "noAdmitidos",
                Json::Arr(vec![Json::obj([
                    ("modo", Json::s("clave-raiz")),
                    (
                        "porque",
                        Json::s(
                            "la clave de la cuenta raíz lo puede todo en todos los buckets: \
                             una fuente lee, y con un usuario que solo lee basta",
                        ),
                    ),
                ])]),
            ),
        ]),
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

/// ⭐ 0046 E9b · El modo «rol» de S3: la confianza ya rellena con los IDs de la
/// celda (o con sus huecos, y dicho por qué), la misma política de lectura, y la
/// forma de la URL, que no lleva ningún secreto.
fn modo_rol_s3(ids: Option<(String, String)>) -> Json {
    let mut m = vec![
        ("modo", Json::s("rol")),
        ("recomendado", Json::Bool(true)),
        (
            "dice",
            Json::s(
                "Crea en tu cuenta de AWS un rol que confíe en las dos cuentas de esta celda y \
                 dale solo esta política: lectura, sobre este bucket. La URL lleva su ARN y ningún \
                 secreto: la celda pide una credencial de una hora cada vez que lee.",
            ),
        ),
        (
            "formato",
            Json::s(
                "s3://<bucket>[/<prefijo>]?region=<región>&role_arn=arn:aws:iam::<cuenta>:role/<rol>",
            ),
        ),
        ("politica", Json::s(POLITICA_S3)),
    ];
    match ids {
        Some((driver, serve)) => {
            m.push((
                "confianza",
                Json::s(
                    CONFIANZA_S3
                        .replace("{idDriver}", &driver)
                        .replace("{idServe}", &serve),
                ),
            ));
            m.push((
                "ids",
                Json::obj([("driver", Json::s(&driver)), ("serve", Json::s(&serve))]),
            ));
        }
        None => {
            m.push(("confianza", Json::s(CONFIANZA_S3)));
            m.push((
                "sinIds",
                Json::s(
                    "esta celda no dice todavía los IDs de sus cuentas (`ids-de-la-celda`): la \
                     confianza sale con sus huecos, `{idDriver}` y `{idServe}`",
                ),
            ));
        }
    }
    Json::obj(m)
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

    /// S3: la clave en la URL, y la política del usuario, que solo lee.
    #[test]
    fn s3_ensena_la_politica_de_solo_lectura() {
        let j = de("s3", None).jcs();
        for a in [
            "s3:ListBucket",
            "s3:ListBucketVersions",
            "s3:GetObject",
            "s3:GetObjectVersion",
            "arn:aws:s3:::{bucket}/*",
        ] {
            assert!(j.contains(a), "{a}: {j}");
        }
        assert!(!j.contains("Put") && !j.contains("Delete"), "{j}");
        assert!(j.contains("\"modo\":\"cadena\""), "{j}");
    }

    /// ⭐ 0046 E9b · El rol va primero y recomendado; su confianza nombra las dos
    /// cuentas de la celda por su ID, y la URL no lleva ningún secreto.
    #[test]
    fn s3_recomienda_el_rol_con_la_confianza_de_la_celda() {
        let j = de_con("s3", None, Some(("111".into(), "222".into()))).jcs();
        let rol = j.find("\"modo\":\"rol\"").expect("modo rol");
        let cadena = j.find("\"modo\":\"cadena\"").expect("modo cadena");
        assert!(rol < cadena, "el rol va primero: {j}");
        for x in [
            "sts:AssumeRoleWithWebIdentity",
            "accounts.google.com:oaud",
            "\\\"111\\\"",
            "\\\"222\\\"",
            "role_arn=",
        ] {
            assert!(j.contains(x), "{x}: {j}");
        }
        assert!(!j.contains("{idDriver}") && !j.contains("sinIds"), "{j}");
        // Sin IDs, los huecos y el porqué.
        let j = de_con("s3", None, None).jcs();
        assert!(j.contains("{idDriver}") && j.contains("sinIds"), "{j}");
    }

    /// Las demás familias: la credencial va en la cadena.
    #[test]
    fn postgres_va_en_la_cadena() {
        assert!(de("postgres", None).jcs().contains("\"modo\":\"cadena\""));
    }
}
