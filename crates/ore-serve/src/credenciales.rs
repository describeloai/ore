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
/// celda (`--cuenta-driver`), y `medios` la de `ore-medios` (`--cuenta-medios`,
/// que GCS también pide); sin ellas el modo se enseña igual, sin email, y se
/// dice por qué falta.
pub fn de(tipo: &str, cuenta: Option<&str>, medios: Option<&str>) -> Json {
    if tipo == "gcs" {
        return gcs(cuenta, medios);
    }
    if tipo == "azure" {
        return azure(id_de("ORE_ID_DRIVER"), id_de("ORE_ID_MEDIOS"));
    }
    if tipo == "sftp" {
        let v = |k: &str| {
            std::env::var(k)
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        };
        return sftp(v("ORE_SFTP_CLAVE_PUBLICA"), v("ORE_IP_SALIDA"));
    }
    de_con(tipo, cuenta, ids_de_la_celda())
}

/// ⭐ ADR 0061 O4·3 (D-O4) · **SFTP: la clave de esta celda.** ORE genera una
/// Ed25519 por celda y el cliente sólo ve la pública, que pega en el
/// `authorized_keys` del usuario con el que se lee (lo que hace Fivetran: nada
/// suyo que guardar). La huella del host se fija siempre: la prueba del alta
/// enseña la vista y el usuario la confirma. De recurso, una contraseña en la
/// URL, que va al custodio. Sólo colecciones mantenidas (D-O1).
fn sftp(publica: Option<String>, ip: Option<String>) -> Json {
    let mut celda = vec![
        ("modo", Json::s("clave-de-la-celda")),
        ("recomendado", Json::Bool(true)),
        (
            "dice",
            Json::s(
                "Esta celda entra con su propia clave SSH: pega su parte pública en el \
                 authorized_keys del usuario con el que se lee (mejor uno que sólo lea esa ruta). \
                 Al probar la conexión verás la huella de tu servidor: confírmala, y si un día \
                 cambia, la celda se negará a conectar hasta que la vuelvas a confirmar.",
            ),
        ),
        (
            "formato",
            Json::s("sftp://<usuario>@<host>[:<puerto>]/<ruta>?huella=SHA256:<huella del host>"),
        ),
        (
            "pasos",
            Json::Arr(vec![
                Json::obj([
                    (
                        "para",
                        Json::s("dejar entrar a la celda, sólo con su clave"),
                    ),
                    (
                        "comando",
                        Json::s(format!(
                            "echo '{}' >> ~<usuario>/.ssh/authorized_keys",
                            publica
                                .as_deref()
                                .unwrap_or("<la clave pública de la celda>")
                        )),
                    ),
                ]),
                Json::obj([
                    (
                        "para",
                        Json::s(
                            "comprobar en el servidor la huella que la prueba enseñe, antes de \
                             confirmarla",
                        ),
                    ),
                    (
                        "comando",
                        Json::s("ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub"),
                    ),
                ]),
                Json::obj([
                    (
                        "para",
                        Json::s("abrir el puerto SSH a la IP de salida de esta celda"),
                    ),
                    (
                        "ip",
                        Json::s(ip.as_deref().unwrap_or("<la IP de salida de la celda>")),
                    ),
                ]),
            ]),
        ),
    ];
    match &publica {
        Some(p) => celda.push(("clavePublica", Json::s(p))),
        None => celda.push((
            "sinClave",
            Json::s(
                "esta celda no dice todavía su clave pública (`ORE_SFTP_CLAVE_PUBLICA`): \
                 pregúntala a quien opera la celda",
            ),
        )),
    }
    if ip.is_none() {
        celda.push((
            "sinIp",
            Json::s("esta celda no dice todavía su IP de salida (`ORE_IP_SALIDA`)"),
        ));
    }
    Json::obj([
        ("tipo", Json::s("sftp")),
        (
            "modos",
            Json::Arr(vec![
                Json::obj(celda),
                Json::obj([
                    ("modo", Json::s("contraseña")),
                    ("recomendado", Json::Bool(false)),
                    (
                        "dice",
                        Json::s(
                            "Si tu servidor no admite claves: la contraseña va dentro de la URL, \
                             cifrada en el custodio. La huella del host se fija igual.",
                        ),
                    ),
                    (
                        "formato",
                        Json::s(
                            "sftp://<usuario>:<contraseña>@<host>[:<puerto>]/<ruta>?huella=SHA256:<huella>",
                        ),
                    ),
                ]),
            ]),
        ),
        (
            "noAdmitidos",
            Json::Arr(vec![
                Json::obj([
                    ("modo", Json::s("cualquier-huella")),
                    (
                        "porque",
                        Json::s(
                            "aceptar la clave de cualquier servidor es dejar que otro se haga \
                             pasar por el tuyo y reciba lo que la celda envía",
                        ),
                    ),
                ]),
                Json::obj([
                    ("modo", Json::s("coleccion-virtual")),
                    (
                        "porque",
                        Json::s(
                            "un SFTP no versiona: sus colecciones son mantenidas, y lo que se \
                             sirve sale de la copia en el lago",
                        ),
                    ),
                ]),
            ]),
        ),
    ])
}

/// El ID único (numérico) de una cuenta de la celda, si el despliegue lo dio.
fn id_de(variable: &str) -> Option<String> {
    std::env::var(variable)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
}

/// ⭐ ADR 0061 O3·3 (D-O3) · **Azure: las cuentas de esta celda, federadas.** El
/// cliente crea en su tenant una app (o una managed identity) con una
/// *federated identity credential* por cuenta de la celda —issuer
/// `https://accounts.google.com`, subject = su ID único, audiencia
/// `api://AzureADTokenExchange`— y le da `Storage Blob Data Reader` sobre el
/// contenedor y `Storage Blob Delegator` sobre la cuenta. Los comandos salen
/// con los IDs ya puestos: uno mal pegado se crea sin error y falla después,
/// en silencio. Sin claves de la cuenta, sin SAS del cliente, sin secretos.
fn azure(driver: Option<String>, medios: Option<String>) -> Json {
    let fic = |quien: &str, id: &Option<String>| {
        format!(
            "az ad app federated-credential create --id {{app}} --parameters \
             '{{\"name\":\"ore-{quien}\",\"issuer\":\"https://accounts.google.com\",\"subject\":\"{}\",\
             \"audiences\":[\"api://AzureADTokenExchange\"]}}'",
            id.as_deref().unwrap_or(&format!("{{id{quien}}}"))
        )
    };
    let alcance = "/subscriptions/{suscripcion}/resourceGroups/{grupo}/providers/Microsoft.Storage/storageAccounts/{cuenta}";
    let mut m = vec![
        ("modo", Json::s("federada")),
        ("recomendado", Json::Bool(true)),
        (
            "dice",
            Json::s(
                "Crea en tu tenant una app (o una managed identity) que confíe en las dos cuentas \
                 de Google de esta celda, y dale solo lectura sobre tu contenedor. La URL nombra la \
                 app y ningún secreto: la celda pide un token de Storage cada vez que lee.",
            ),
        ),
        (
            "formato",
            Json::s(
                "az://<cuenta>/<contenedor>[/<prefijo>]?tenant=<id del tenant>&cliente=<id de la app>",
            ),
        ),
        (
            "pasos",
            Json::Arr(vec![
                Json::obj([
                    ("para", Json::s("la app que confía en la celda")),
                    (
                        "comando",
                        Json::s("az ad app create --display-name ore-lector"),
                    ),
                ]),
                Json::obj([
                    ("para", Json::s("el driver de la celda: catalogar y copiar")),
                    ("comando", Json::s(fic("driver", &driver))),
                ]),
                Json::obj([
                    (
                        "para",
                        Json::s("los medios de la celda: servir los ítems y firmar sus URLs"),
                    ),
                    ("comando", Json::s(fic("medios", &medios))),
                ]),
                Json::obj([
                    ("para", Json::s("leer el contenedor (y sus versiones)")),
                    (
                        "comando",
                        Json::s(format!(
                            "az role assignment create --assignee {{app}} --role \"Storage Blob Data Reader\" \
                             --scope {alcance}/blobServices/default/containers/{{contenedor}}"
                        )),
                    ),
                ]),
                Json::obj([
                    (
                        "para",
                        Json::s(
                            "firmar las URLs de los ítems (la clave de delegación es de la cuenta); \
                             sin esto se cataloga y se copia igual",
                        ),
                    ),
                    (
                        "comando",
                        Json::s(format!(
                            "az role assignment create --assignee {{app}} --role \"Storage Blob Delegator\" \
                             --scope {alcance}"
                        )),
                    ),
                ]),
            ]),
        ),
    ];
    let falta: Vec<&str> = [("ORE_ID_DRIVER", &driver), ("ORE_ID_MEDIOS", &medios)]
        .iter()
        .filter(|(_, v)| v.is_none())
        .map(|(k, _)| *k)
        .collect();
    if falta.is_empty() {
        m.push((
            "ids",
            Json::obj([
                ("driver", Json::s(driver.unwrap_or_default())),
                ("medios", Json::s(medios.unwrap_or_default())),
            ]),
        ));
    } else {
        m.push((
            "sinIds",
            Json::s(format!(
                "esta celda no dice todavía los IDs de sus cuentas ({}): los comandos salen con sus \
                 huecos, y un subject mal pegado se crea sin error y falla después",
                falta.join(", ")
            )),
        ));
    }
    let no = |modo: &str, porque: &str| {
        Json::obj([("modo", Json::s(modo)), ("porque", Json::s(porque))])
    };
    Json::obj([
        ("tipo", Json::s("azure")),
        ("modos", Json::Arr(vec![Json::obj(m)])),
        (
            "noAdmitidos",
            Json::Arr(vec![
                no(
                    "clave-cuenta",
                    "la clave de la cuenta lo puede todo en todos sus contenedores, y Azure \
                     recomienda apagarla (AllowSharedKeyAccess=false)",
                ),
                no(
                    "sas",
                    "una SAS es un secreto al portador que caduca, y con la clave de la cuenta \
                     apagada ya no vale",
                ),
                no(
                    "secreto-app",
                    "el secreto de una app es justo lo que la federación evita guardar y rotar",
                ),
            ]),
        ),
    ])
}

/// Lo que se concede sobre un bucket de GCS: leer, y nada más. Es lo que
/// `ore-read-gcs check` pregunta (`storage.objects.list`, `storage.objects.get`).
const LECTOR_GCS: &str = "roles/storage.objectViewer";

/// Un rol de GCS que se concede: cuál, en qué nivel, para qué y con qué.
fn rol_gcs(rol: &str, nivel: &str, para: &str, comando: &str) -> Json {
    Json::obj([
        ("rol", Json::s(rol)),
        ("nivel", Json::s(nivel)),
        ("para", Json::s(para)),
        ("comando", Json::s(comando)),
    ])
}

/// ⭐ ADR 0061 O2·3 (D-O2) · **GCS: las cuentas de esta celda, sin clave.** El
/// cliente concede lectura sobre SU bucket a las dos que leen —la del driver,
/// que cataloga y copia, y la de `ore-medios`, que sirve los ítems y firma sus
/// URLs—. O, si prefiere que se lea como una cuenta suya, les deja
/// suplantarla: un token de una hora, de sólo lectura, cada vez. Nunca una
/// cuenta compartida entre celdas: otra celda corre con otras cuentas.
fn gcs(driver: Option<&str>, medios: Option<&str>) -> Json {
    let con_cuentas = |mut m: Vec<(&'static str, Json)>| {
        let mut cuentas = Vec::new();
        let mut falta = Vec::new();
        for (quien, para, v, flag) in [
            ("driver", "catalogar y copiar", driver, "--cuenta-driver"),
            (
                "medios",
                "servir los ítems y firmar sus URLs",
                medios,
                "--cuenta-medios",
            ),
        ] {
            match v {
                Some(e) => cuentas.push(Json::obj([
                    ("quien", Json::s(quien)),
                    ("cuenta", Json::s(e)),
                    ("para", Json::s(para)),
                ])),
                None => falta.push(flag),
            }
        }
        m.push(("cuentas", Json::Arr(cuentas)));
        if !falta.is_empty() {
            m.push((
                "sinCuenta",
                Json::s(format!(
                    "este servidor no sabe todas las cuentas de la celda ({}): pregúntalas a \
                     quien opera la celda",
                    falta.join(", ")
                )),
            ));
        }
        Json::obj(m)
    };
    let celda = con_cuentas(vec![
        ("modo", Json::s("celda")),
        ("recomendado", Json::Bool(true)),
        (
            "dice",
            Json::s(
                "Esta celda lee con sus propias cuentas de Google: no hay clave que guardar. \
                 Concede a las dos solo lectura sobre tu bucket.",
            ),
        ),
        ("formato", Json::s("gs://<bucket>[/<prefijo>]")),
        (
            "roles",
            Json::Arr(vec![rol_gcs(
                LECTOR_GCS,
                "bucket",
                "listar los objetos y leerlos, con sus generaciones",
                "gcloud storage buckets add-iam-policy-binding gs://{bucket} \
                 --member=serviceAccount:{cuenta} --role=roles/storage.objectViewer",
            )]),
        ),
    ]);
    let suplantar = con_cuentas(vec![
        ("modo", Json::s("suplantar")),
        ("recomendado", Json::Bool(false)),
        (
            "dice",
            Json::s(
                "Se lee como una cuenta de servicio tuya: dale a ella lectura sobre el bucket, y a \
                 las dos cuentas de esta celda permiso para pedir su token. Cada vez se pide uno \
                 de una hora y de solo lectura; quitas ese permiso y deja de leer.",
            ),
        ),
        (
            "formato",
            Json::s(
                "gs://<bucket>[/<prefijo>]?suplantar=<cuenta>@<proyecto>.iam.gserviceaccount.com",
            ),
        ),
        (
            "roles",
            Json::Arr(vec![
                rol_gcs(
                    LECTOR_GCS,
                    "bucket",
                    "tu cuenta: listar y leer los objetos",
                    "gcloud storage buckets add-iam-policy-binding gs://{bucket} \
                     --member=serviceAccount:{suya} --role=roles/storage.objectViewer",
                ),
                rol_gcs(
                    "roles/iam.serviceAccountTokenCreator",
                    "cuenta",
                    "cada cuenta de la celda: pedir el token de la tuya y firmar como ella",
                    "gcloud iam service-accounts add-iam-policy-binding {suya} \
                     --member=serviceAccount:{cuenta} --role=roles/iam.serviceAccountTokenCreator",
                ),
            ]),
        ),
    ]);
    Json::obj([
        ("tipo", Json::s("gcs")),
        ("modos", Json::Arr(vec![celda, suplantar])),
        (
            "noAdmitidos",
            Json::Arr(vec![Json::obj([
                ("modo", Json::s("clave-json")),
                (
                    "porque",
                    Json::s(
                        "Google prohíbe por defecto crear claves de cuenta de servicio en las \
                         organizaciones nuevas (iam.disableServiceAccountKeyCreation); pedirla \
                         sería pedirte que rebajes tu seguridad",
                    ),
                ),
            ])]),
        ),
    ])
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
            None,
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

    /// GCS (0061 O2·3): las dos cuentas de la celda con lectura sobre el bucket,
    /// o suplantando una del cliente; la clave JSON, no; y sin una de las dos
    /// cuentas se dice cuál falta.
    #[test]
    fn gcs_ensena_las_dos_cuentas_y_como_suplantar() {
        let j = de(
            "gcs",
            Some("ore-driver-demo@p.iam.gserviceaccount.com"),
            Some("ore-medios-demo@p.iam.gserviceaccount.com"),
        )
        .jcs();
        for x in [
            r#""modo":"celda""#,
            r#""modo":"suplantar""#,
            r#""modo":"clave-json""#,
            "ore-driver-demo@p.iam.gserviceaccount.com",
            "ore-medios-demo@p.iam.gserviceaccount.com",
            "roles/storage.objectViewer",
            "roles/iam.serviceAccountTokenCreator",
            "?suplantar=",
        ] {
            assert!(j.contains(x), "{x}: {j}");
        }
        assert!(!j.contains("sinCuenta"), "{j}");
        let j = de("gcs", Some("d@p.iam.gserviceaccount.com"), None).jcs();
        assert!(
            j.contains("--cuenta-medios") && !j.contains("--cuenta-driver"),
            "{j}"
        );
    }

    /// Azure (0061 O3·3): la app federada con los IDs de las dos cuentas ya en
    /// los comandos, los dos roles, y lo que no se admite; sin IDs, los huecos
    /// y el aviso.
    #[test]
    fn azure_da_los_comandos_con_los_ids_puestos() {
        let j = azure(Some("111".into()), Some("333".into())).jcs();
        for x in [
            r#""modo":"federada""#,
            r#"\"subject\":\"111\""#,
            r#"\"subject\":\"333\""#,
            "api://AzureADTokenExchange",
            "https://accounts.google.com",
            "Storage Blob Data Reader",
            "Storage Blob Delegator",
            r#""modo":"clave-cuenta""#,
            r#""modo":"sas""#,
            "tenant=<id del tenant>&cliente=<id de la app>",
        ] {
            assert!(j.contains(x), "{x}: {j}");
        }
        assert!(!j.contains("sinIds"), "{j}");
        let j = azure(Some("111".into()), None).jcs();
        assert!(
            j.contains("ORE_ID_MEDIOS") && j.contains("{idmedios}"),
            "{j}"
        );
    }

    /// SFTP (0061 O4·3): la clave pública de la celda en el comando, confirmar
    /// la huella, la IP de salida, la contraseña de recurso, y lo que no se
    /// admite; sin clave ni IP, se dice.
    #[test]
    fn sftp_da_la_clave_publica_y_pide_confirmar_la_huella() {
        let j = sftp(
            Some("ssh-ed25519 AAAAC3Nza ore-demo".into()),
            Some("34.1.2.3".into()),
        )
        .jcs();
        for x in [
            r#""modo":"clave-de-la-celda""#,
            "echo 'ssh-ed25519 AAAAC3Nza ore-demo' >> ~<usuario>/.ssh/authorized_keys",
            "ssh-keygen -lf",
            "34.1.2.3",
            r#""modo":"contraseña""#,
            r#""modo":"cualquier-huella""#,
            r#""modo":"coleccion-virtual""#,
        ] {
            assert!(j.contains(x), "{x}: {j}");
        }
        assert!(!j.contains("sinClave") && !j.contains("sinIp"), "{j}");
        let j = sftp(None, None).jcs();
        assert!(
            j.contains("ORE_SFTP_CLAVE_PUBLICA") && j.contains("ORE_IP_SALIDA"),
            "{j}"
        );
    }

    /// Sin `--cuenta-driver` el modo se enseña igual y se dice qué falta.
    #[test]
    fn sin_cuenta_se_dice() {
        let j = de("bigquery", None, None).jcs();
        assert!(j.contains("sinCuenta") && !j.contains("\"cuenta\""), "{j}");
    }

    /// S3: la clave en la URL, y la política del usuario, que solo lee.
    #[test]
    fn s3_ensena_la_politica_de_solo_lectura() {
        let j = de("s3", None, None).jcs();
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
        assert!(
            de("postgres", None, None)
                .jcs()
                .contains("\"modo\":\"cadena\"")
        );
    }
}
