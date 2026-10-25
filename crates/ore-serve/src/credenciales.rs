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
    if tipo == "sharepoint" {
        return sharepoint(id_de("ORE_ID_DRIVER"), id_de("ORE_ID_MEDIOS"));
    }
    if tipo == "sftp" || tipo == "postgres" || tipo == "postgresql" {
        let v = |k: &str| {
            std::env::var(k)
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        };
        return if tipo == "sftp" {
            sftp(clave_publica(v), v("ORE_IP_SALIDA"))
        } else {
            postgres(clave_publica(v), v("ORE_IP_SALIDA"), cuenta)
        };
    }
    de_con(tipo, cuenta, ids_de_la_celda())
}

/// La pública de la celda: `ORE_SFTP_CLAVE_PUBLICA`, o el fichero que nombra
/// `ORE_SFTP_CLAVE_PUBLICA_FICHERO`, que es como llega en una celda: el
/// contenedor de inicio la trae del almacén, donde la dejó el aprovisionador.
/// Se lee en cada petición: es una línea.
fn clave_publica(v: impl Fn(&str) -> Option<String>) -> Option<String> {
    v("ORE_SFTP_CLAVE_PUBLICA").or_else(|| {
        let f = v("ORE_SFTP_CLAVE_PUBLICA_FICHERO")?;
        let s = std::fs::read_to_string(f).ok()?;
        Some(s.trim().to_string()).filter(|s| !s.is_empty())
    })
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

/// La *federated identity credential* de una cuenta de la celda en la app del
/// cliente (D-O3, D-O5), con su ID único puesto o el hueco `{id<quien>}`.
fn credencial_federada(quien: &str, id: &Option<String>) -> String {
    format!(
        "az ad app federated-credential create --id {{app}} --parameters \
         '{{\"name\":\"ore-{quien}\",\"issuer\":\"https://accounts.google.com\",\"subject\":\"{}\",\
         \"audiences\":[\"api://AzureADTokenExchange\"]}}'",
        id.as_deref().unwrap_or(&format!("{{id{quien}}}"))
    )
}

/// El id del permiso de aplicación `Sites.Selected` de Microsoft Graph (la API
/// `00000003-0000-0000-c000-000000000000`), para `az ad app permission add`.
/// Sin comprobar contra un tenant (deuda de O5).
const SITES_SELECTED: &str = "883ea226-0bf2-4a8f-9f9d-92c9162a727d";

/// ⭐ ADR 0061 O5·3 (D-O5) · **SharePoint: las cuentas de esta celda, federadas,
/// y sólo los sitios que el cliente elige.** La app es como la de Azure (puede
/// ser la misma): las dos *federated identity credentials* con los IDs puestos.
/// Lo que cambia es el permiso: `Sites.Selected` de Graph, con el
/// consentimiento de un administrador —que por sí solo no abre ningún sitio— y
/// la concesión `read` en cada sitio, que da un administrador de SharePoint.
/// Sin `Sites.Read.All` (todo el tenant), sin un usuario, sin secretos.
fn sharepoint(driver: Option<String>, medios: Option<String>) -> Json {
    let mut m = vec![
        ("modo", Json::s("federada")),
        ("recomendado", Json::Bool(true)),
        (
            "dice",
            Json::s(
                "Crea en tu tenant una app que confíe en las dos cuentas de Google de esta celda \
                 (la de Azure vale), dale el permiso Sites.Selected de Microsoft Graph y concédele \
                 lectura sólo en los sitios que quieras: ninguno más se ve. La URL nombra el sitio, \
                 la biblioteca y la app, y ningún secreto.",
            ),
        ),
        (
            "formato",
            Json::s(
                "sharepoint://<tenant>.sharepoint.com/sites/<sitio>/<biblioteca>[/<carpeta>]?tenant=<id del tenant>&cliente=<id de la app>",
            ),
        ),
        (
            "pasos",
            Json::Arr(vec![
                Json::obj([
                    (
                        "para",
                        Json::s("la app que confía en la celda (o la de Azure)"),
                    ),
                    (
                        "comando",
                        Json::s("az ad app create --display-name ore-lector"),
                    ),
                ]),
                Json::obj([
                    ("para", Json::s("el driver de la celda: catalogar y copiar")),
                    ("comando", Json::s(credencial_federada("driver", &driver))),
                ]),
                Json::obj([
                    ("para", Json::s("los medios de la celda: servir los ítems")),
                    ("comando", Json::s(credencial_federada("medios", &medios))),
                ]),
                Json::obj([
                    (
                        "para",
                        Json::s(
                            "el permiso Sites.Selected de Graph, con el consentimiento de un \
                             administrador: por sí solo no abre ningún sitio",
                        ),
                    ),
                    (
                        "comando",
                        Json::s(format!(
                            "az ad app permission add --id {{app}} --api 00000003-0000-0000-c000-000000000000 \
                             --api-permissions {SITES_SELECTED}=Role && az ad app permission admin-consent --id {{app}}"
                        )),
                    ),
                ]),
                Json::obj([
                    (
                        "para",
                        Json::s(
                            "leer UN sitio (un administrador de SharePoint, una vez por sitio)",
                        ),
                    ),
                    (
                        "comando",
                        Json::s(
                            "Grant-PnPEntraIDAppSitePermission -AppId {app} -DisplayName ORE \
                             -Site https://{tenant}.sharepoint.com/sites/{sitio} -Permissions Read",
                        ),
                    ),
                ]),
            ]),
        ),
    ];
    con_ids(&mut m, driver, medios);
    let no = |modo: &str, porque: &str| {
        Json::obj([("modo", Json::s(modo)), ("porque", Json::s(porque))])
    };
    Json::obj([
        ("tipo", Json::s("sharepoint")),
        ("modos", Json::Arr(vec![Json::obj(m)])),
        (
            "noAdmitidos",
            Json::Arr(vec![
                no(
                    "todo-el-tenant",
                    "Sites.Read.All o Files.Read.All leen todos los sitios y todos los OneDrive \
                     de la empresa: Sites.Selected lee sólo los que concedas",
                ),
                no(
                    "usuario",
                    "un permiso delegado lee como una persona: la ingesta pararía cuando esa \
                     persona se vaya o cambie su contraseña",
                ),
                no(
                    "secreto-app",
                    "un secreto o un certificado de la app es justo lo que la federación evita \
                     guardar y rotar",
                ),
            ]),
        ),
    ])
}

/// Los IDs de las dos cuentas, o el aviso de que faltan.
fn con_ids(m: &mut Vec<(&str, Json)>, driver: Option<String>, medios: Option<String>) {
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
    let fic = credencial_federada;
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
    con_ids(&mut m, driver, medios);
    let no = |modo: &str, porque: &str| {
        Json::obj([("modo", Json::s(modo)), ("porque", Json::s(porque))])
    };
    // ⭐ D-O6 · Un contenedor público (Azure Open Datasets, o lo que un socio
    //   publica): sin identidad, sin pasos, sin URLs firmadas.
    let publico = Json::obj([
        ("modo", Json::s("publico")),
        ("recomendado", Json::Bool(false)),
        (
            "dice",
            Json::s(
                "A public container (public access level `container`) is read without identity:                  nothing to create in your tenant, and no token is ever sent. Its items open                  through ORE, without signed URLs.",
            ),
        ),
        (
            "formato",
            Json::s("az://<cuenta>/<contenedor>[/<prefijo>]?publico=1"),
        ),
    ]);
    Json::obj([
        ("tipo", Json::s("azure")),
        ("modos", Json::Arr(vec![Json::obj(m), publico])),
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
/// ⭐ ADR 0065 PG1 (D-PG1) · **Postgres: la credencial en la cadena, y TLS
/// siempre verificado.** Sin `sslmode`, el driver verifica la cadena y el
/// nombre del host; con una CA que no es pública (RDS, Cloud SQL, Supabase,
/// Aiven), su PEM va en la URL (`sslrootcert`). En claro, no.
///
/// ⭐ ADR 0065 PG4 (D-PG3) · **Detrás de un bastión**: la misma clave SSH de
/// la celda que SFTP, en el `authorized_keys` de un bastión que ve la base, y
/// su huella fijada. TLS sigue de punta a punta por dentro del túnel.
///
/// ⭐ ADR 0065 PG5 (D-PG2) · **Cloud SQL y AlloyDB, sin contraseña**: el
/// token de Google de la cuenta de la celda (la de BigQuery y GCS) va como
/// contraseña. El usuario IAM es la cuenta sin `.gserviceaccount.com`.
fn postgres(publica: Option<String>, ip: Option<String>, cuenta: Option<&str>) -> Json {
    let cuenta_o = cuenta.unwrap_or("{cuenta}");
    let usuario_iam = cuenta_o
        .trim_end_matches(".gserviceaccount.com")
        .to_string();
    let paso = |para: &str, comando: String| {
        Json::obj([("para", Json::s(para)), ("comando", Json::s(comando))])
    };
    let mut iam = vec![
        ("modo", Json::s("iam-gcp")),
        ("recomendado", Json::Bool(false)),
        (
            "dice",
            Json::s(
                "Cloud SQL o AlloyDB: la celda entra con su cuenta de Google, sin contraseña que \
                 guardar. Activa la autenticación IAM en la instancia, crea el usuario IAM de la \
                 cuenta de la celda y dale lectura dentro de la base.",
            ),
        ),
        (
            "formato",
            Json::s(format!(
                "postgres://{}@<host>[:<puerto>]/<base>?iam=gcp",
                usuario_iam.replace('@', "%40")
            )),
        ),
        ("usuario", Json::s(&usuario_iam)),
        (
            "pasos",
            Json::Arr(vec![
                paso(
                    "activar IAM en la instancia (Cloud SQL; `--database-flags` sustituye las que ya tenga: añádelas)",
                    "gcloud sql instances patch <instancia> --database-flags=cloudsql.iam_authentication=on".into(),
                ),
                paso(
                    "crear el usuario IAM de la celda",
                    format!(
                        "gcloud sql users create {usuario_iam} --instance=<instancia> --type=cloud_iam_service_account"
                    ),
                ),
                paso(
                    "dejarla entrar en la instancia",
                    format!(
                        "gcloud projects add-iam-policy-binding <proyecto> --member=serviceAccount:{cuenta_o} --role=roles/cloudsql.instanceUser --condition=None"
                    ),
                ),
                paso(
                    "darle lectura dentro de la base",
                    format!(
                        "GRANT USAGE ON SCHEMA public TO \"{usuario_iam}\"; GRANT SELECT ON ALL TABLES IN SCHEMA public TO \"{usuario_iam}\";"
                    ),
                ),
                paso(
                    "en AlloyDB, en vez de los dos primeros",
                    format!(
                        "gcloud alloydb instances update <instancia> --cluster=<clúster> --region=<región> --database-flags=alloydb.iam_authentication=on && gcloud alloydb users create {usuario_iam} --cluster=<clúster> --region=<región> --type=IAM_BASED"
                    ),
                ),
            ]),
        ),
    ];
    if cuenta.is_none() {
        iam.push((
            "sinCuenta",
            Json::s(
                "esta celda no dice todavía su cuenta (`--cuenta-driver`): los comandos salen con su hueco",
            ),
        ));
    }
    let no = |modo: &str, porque: &str| {
        Json::obj([("modo", Json::s(modo)), ("porque", Json::s(porque))])
    };
    Json::obj([
        ("tipo", Json::s("postgres")),
        (
            "modos",
            Json::Arr(vec![
                Json::obj([
                    ("modo", Json::s("cadena")),
                    ("recomendado", Json::Bool(true)),
                    (
                        "dice",
                        Json::s(
                            "La credencial va dentro de la cadena de conexión, cifrada en el custodio;                          mejor un usuario que sólo lee. TLS se verifica siempre (la cadena y el                          nombre del host). Si tu proveedor firma con su propia CA (RDS, Cloud SQL,                          Supabase, Aiven), pega su PEM, codificado en la URL, en `sslrootcert`; si                          el certificado no lleva el nombre por el que conectas, `sslmode=verify-ca`.",
                        ),
                    ),
                    (
                        "formato",
                        Json::s(
                            "postgres://<usuario>:<contraseña>@<host>[:<puerto>]/<base>[?sslmode=verify-full|verify-ca][&sslrootcert=<PEM de la CA, codificado>]",
                        ),
                    ),
                ]),
                Json::obj(iam),
                Json::obj([
                    ("modo", Json::s("tunel-ssh")),
                    ("recomendado", Json::Bool(false)),
                    (
                        "dice",
                        Json::s(
                            "Si tu base no está en internet: la celda entra por un bastión SSH tuyo que                          sí la ve, con su propia clave. Pega su parte pública en el authorized_keys                          del usuario del bastión (que pueda reenviar, `AllowTcpForwarding`) y fija                          la huella del bastión, que la prueba te enseña. El host de la URL es el de                          la base visto desde el bastión, y TLS se verifica contra él.",
                        ),
                    ),
                    (
                        "formato",
                        Json::s(
                            "postgres://<usuario>:<contraseña>@<host de la base>[:<puerto>]/<base>?ssh=<usuario>@<bastión>[:<puerto>]&ssh_huella=SHA256:<huella del bastión>",
                        ),
                    ),
                    (
                        "pasos",
                        Json::Arr(vec![Json::obj([
                            (
                                "para",
                                Json::s("dejar entrar a la celda en el bastión, sólo con su clave"),
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
                        ])]),
                    ),
                    ("clavePublica", Json::s(publica.clone().unwrap_or_default())),
                    (
                        "ipDeSalida",
                        Json::s(ip.unwrap_or_else(|| {
                            "esta celda no dice todavía su IP de salida (`ORE_IP_SALIDA`)".into()
                        })),
                    ),
                ]),
            ]),
        ),
        (
            "noAdmitidos",
            Json::Arr(vec![
                no(
                    "en-claro",
                    "`sslmode=disable`, `allow` o `prefer` dejan la contraseña y los datos a la                      vista de quien esté en medio; y `prefer` baja a texto plano sin avisar",
                ),
                no(
                    "cualquier-certificado",
                    "aceptar el certificado de cualquiera es dejar que otro se haga pasar por tu                      base y reciba la contraseña",
                ),
                no(
                    "fichero",
                    "`sslrootcert` con una ruta: el driver corre en un Job sin tus ficheros; el PEM                      va dentro de la URL",
                ),
            ]),
        ),
    ])
}

pub fn de_con(tipo: &str, cuenta: Option<&str>, ids: Option<(String, String)>) -> Json {
    match tipo {
        "postgres" | "postgresql" => postgres(None, None, cuenta),
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
        // Cassandra (ADR 0070 M3): un clúster propio, 4.x o 5.x. La credencial va en la
        // cadena, al custodio; lo que se enseña es el rol que solo lee, el acceso de red
        // y `contacto=solo`, que es donde un clúster detrás de un balanceador, de NAT o de
        // Kubernetes se atasca la primera vez (el driver usa la dirección que cada nodo
        // anuncia, y esa no se alcanza desde fuera: medido en X0).
        "cassandra" => Json::obj([
            ("tipo", Json::s("cassandra")),
            (
                "modos",
                Json::Arr(vec![Json::obj([
                    ("modo", Json::s("cadena")),
                    ("recomendado", Json::Bool(true)),
                    (
                        "dice",
                        Json::s(
                            "La credencial va dentro de la cadena de conexión, cifrada en el custodio. Crea un rol que solo lea, con `SELECT` sobre el keyspace que quieres conectar. TLS se verifica siempre.",
                        ),
                    ),
                    (
                        "formato",
                        Json::s(
                            "cassandra://<usuario>:<contraseña>@<host>[:<puerto>][,<host>…]/<keyspace>[?tls=true&consistency=LOCAL_QUORUM&dc=<centro>&contacto=solo]",
                        ),
                    ),
                    (
                        "pasos",
                        Json::Arr(vec![
                            Json::obj([
                                ("para", Json::s("El rol que solo lee")),
                                (
                                    "comando",
                                    Json::s(
                                        "CREATE ROLE ore_lector WITH PASSWORD = '<contraseña>' AND LOGIN = true; GRANT SELECT ON KEYSPACE <keyspace> TO ore_lector;",
                                    ),
                                ),
                            ]),
                            Json::obj([
                                ("para", Json::s("Que la celda llegue al clúster")),
                                (
                                    "comando",
                                    Json::s(
                                        "Abre el puerto nativo (9042, o el de la URL) a la IP de salida de la celda ({ipSalida}; pregúntala a quien la opera)",
                                    ),
                                ),
                            ]),
                            Json::obj([
                                (
                                    "para",
                                    Json::s("Si hay un balanceador, NAT o Kubernetes delante"),
                                ),
                                (
                                    "comando",
                                    Json::s(
                                        "Añade `contacto=solo` a la cadena: sin ella el driver descubre los nodos y usa la dirección interna que cada uno anuncia, que desde fuera no se alcanza",
                                    ),
                                ),
                            ]),
                        ]),
                    ),
                ])]),
            ),
            (
                "noAdmitidos",
                Json::Arr(vec![
                    Json::obj([
                        ("modo", Json::s("tls-inseguro")),
                        (
                            "porque",
                            Json::s(
                                "`tlsInsecure`, `tlsAllowInvalidCertificates` o `verify=false` aceptan el certificado de cualquiera: otro se haría pasar por tu clúster y recibiría la contraseña",
                            ),
                        ),
                    ]),
                    Json::obj([
                        ("modo", Json::s("ca-por-fichero")),
                        (
                            "porque",
                            Json::s(
                                "`tlsCAFile` con una ruta: el driver corre en un Job sin tus ficheros. Una CA privada todavía no se admite",
                            ),
                        ),
                    ]),
                    Json::obj([
                        ("modo", Json::s("rol-con-escritura")),
                        (
                            "porque",
                            Json::s(
                                "una fuente lee: con un superusuario o un rol con `MODIFY` el driver podría más de lo que hace, y con `SELECT` basta",
                            ),
                        ),
                    ]),
                    Json::obj([
                        ("modo", Json::s("astra-bundle")),
                        (
                            "porque",
                            Json::s(
                                "DataStax Astra DB se conecta con un *secure connect bundle*, que es un fichero: todavía no hay cómo llevarlo hasta el driver",
                            ),
                        ),
                    ]),
                ]),
            ),
        ]),
        // Phoenix (ADR 0071 M2): Apache Phoenix por el Phoenix Query Server (Avatica sobre HTTP). La
        // credencial va en la cadena, al custodio, y **exige TLS**: HTTP *Basic* iría legible por la red.
        // Lo que se enseña es el acceso de red al PQS y lo que no se admite (Kerberos, TLS inseguro).
        "phoenix" => Json::obj([
            ("tipo", Json::s("phoenix")),
            (
                "modos",
                Json::Arr(vec![Json::obj([
                    ("modo", Json::s("cadena")),
                    ("recomendado", Json::Bool(true)),
                    (
                        "dice",
                        Json::s(
                            "La credencial va dentro de la cadena de conexión, cifrada en el custodio, y el Phoenix Query Server tiene que ir por HTTPS (`tls=true`): una contraseña por HTTP en claro no se admite. Usa un usuario que solo lea, con permiso de lectura sobre el esquema que quieres conectar. TLS se verifica siempre.",
                        ),
                    ),
                    (
                        "formato",
                        Json::s(
                            "phoenix://<usuario>:<contraseña>@<host>[:8765][/<ESQUEMA>]?tls=true[&tenant=<inquilino>&timeoutMs=<ms>]",
                        ),
                    ),
                    (
                        "pasos",
                        Json::Arr(vec![
                            Json::obj([
                                ("para", Json::s("El servidor al que se conecta")),
                                (
                                    "comando",
                                    Json::s(
                                        "Es el Phoenix Query Server (el puerto 8765 por defecto), no HBase ni ZooKeeper: pregunta a quien opera el clúster por su dirección. Phoenix distingue mayúsculas: el esquema y las tablas se escriben como el catálogo (`ESQUEMA.TABLA`)",
                                    ),
                                ),
                            ]),
                            Json::obj([
                                ("para", Json::s("Que la celda llegue al servidor")),
                                (
                                    "comando",
                                    Json::s(
                                        "Abre el puerto del Phoenix Query Server a la IP de salida de la celda ({ipSalida}; pregúntala a quien la opera)",
                                    ),
                                ),
                            ]),
                        ]),
                    ),
                ])]),
            ),
            (
                "noAdmitidos",
                Json::Arr(vec![
                    Json::obj([
                        ("modo", Json::s("tls-inseguro")),
                        (
                            "porque",
                            Json::s(
                                "`tlsInsecure`, `tlsAllowInvalidCertificates` o `verify=false` aceptan el certificado de cualquiera: otro se haría pasar por tu servidor y recibiría la contraseña",
                            ),
                        ),
                    ]),
                    Json::obj([
                        ("modo", Json::s("contrasena-sin-tls")),
                        (
                            "porque",
                            Json::s(
                                "una contraseña por HTTP en claro cruzaría la red legible: la autenticación del Phoenix Query Server exige `tls=true`",
                            ),
                        ),
                    ]),
                    Json::obj([
                        ("modo", Json::s("kerberos")),
                        (
                            "porque",
                            Json::s(
                                "la autenticación SPNEGO (Kerberos) del Phoenix Query Server todavía no se admite: se entra con usuario y contraseña (HTTP Basic) sobre TLS",
                            ),
                        ),
                    ]),
                    Json::obj([
                        ("modo", Json::s("ca-por-fichero")),
                        (
                            "porque",
                            Json::s(
                                "`tlsCAFile` con una ruta: el conector corre en un Job sin tus ficheros. Una CA privada todavía no se admite",
                            ),
                        ),
                    ]),
                ]),
            ),
        ]),
        // HBase (ADR 0072 N7): el protocolo nativo (ZooKeeper y RPC a los RegionServers), sin PQS. SIMPLE no tiene
        // secreto —el cliente *afirma* un usuario y nada viaja cifrado—, asi que se admite solo reconociendolo en la
        // URL, y no hay credencial que guardar. Kerberos funciona en el driver (ficha 0072, N6), pero el keytab es un
        // fichero y todavia no hay como llevarlo hasta un Job.
        "hbase" => Json::obj([
            ("tipo", Json::s("hbase")),
            (
                "modos",
                Json::Arr(vec![Json::obj([
                    ("modo", Json::s("red-de-confianza")),
                    ("recomendado", Json::Bool(true)),
                    (
                        "dice",
                        Json::s(
                            "HBase se lee por su protocolo nativo con la autenticación SIMPLE: el cliente afirma un usuario, sin contraseña, y lo que viaja no va cifrado. Solo para una red que ya es de confianza (un enlace privado o una VPN hasta el clúster), y la URL lo reconoce con `simple=acepto`. No hay credencial que guardar. Dale al clúster un usuario que solo lea.",
                        ),
                    ),
                    (
                        "formato",
                        Json::s(
                            "hbase://<zk1>[:2181][,<zk2>[:2181]…][/<espacio>]?simple=acepto[&znode=/hbase&paralelo=<n>&timeoutMs=<ms>&familia=<f>&clave=<nombre>&traduccion=<anunciado:puerto>=<real:puerto>]",
                        ),
                    ),
                    (
                        "pasos",
                        Json::Arr(vec![
                            Json::obj([
                                ("para", Json::s("El servidor al que se conecta")),
                                (
                                    "comando",
                                    Json::s(
                                        "El quórum de ZooKeeper del clúster (el puerto 2181 por defecto), no un RegionServer: de ahí sale dónde está todo lo demás. Pregunta a quien opera el clúster por `hbase.zookeeper.quorum` y por `zookeeper.znode.parent` si no es `/hbase`",
                                    ),
                                ),
                            ]),
                            Json::obj([
                                ("para", Json::s("Que la celda llegue a los RegionServers")),
                                (
                                    "comando",
                                    Json::s(
                                        "Abre a la IP de salida de la celda ({ipSalida}; pregúntala a quien la opera) el quórum de ZooKeeper y los RegionServers (el puerto 16020 por defecto). ZooKeeper dice los RegionServers por el nombre que ellos anuncian: si la celda no lo resuelve, `traduccion=` lo lleva a donde sí se llega",
                                    ),
                                ),
                            ]),
                            Json::obj([
                                ("para", Json::s("Los tipos")),
                                (
                                    "comando",
                                    Json::s(
                                        "HBase guarda bytes sin tipo: el catálogo ve texto o bytes. Declara el tipo de cada columna en la tabla y su codificación en `physicalType` (`i32be`, `i64be`, `epoch-ms`, `decimal`…); lo que Java escribió con `Bytes.toBytes` se lee con su valor",
                                    ),
                                ),
                            ]),
                        ]),
                    ),
                ])]),
            ),
            (
                "noAdmitidos",
                Json::Arr(vec![
                    Json::obj([
                        ("modo", Json::s("simple-sin-reconocer")),
                        (
                            "porque",
                            Json::s(
                                "sin `simple=acepto` el driver no conecta: la autenticación SIMPLE no es un secreto, y la URL tiene que decir que sabes que el canal no va cifrado",
                            ),
                        ),
                    ]),
                    Json::obj([
                        ("modo", Json::s("kerberos")),
                        (
                            "porque",
                            Json::s(
                                "el driver habla Kerberos (SASL/GSSAPI, con las tres protecciones), pero el keytab es un fichero y el conector corre en un Job sin tus ficheros: todavía no hay cómo llevarlo hasta él. Mientras, en un despliegue propio se monta el keytab y `krb5.conf` en la carpeta de los drivers",
                            ),
                        ),
                    ]),
                ]),
            ),
        ]),
        // Oracle (ADR 0074): Oracle Database 12.1 o posterior, por el driver thin (el protocolo nativo, sin
        // Instant Client). La credencial va en la cadena, al custodio; lo que se enseña es el usuario que solo
        // lee, el nombre de **servicio** (no el SID), el acceso de red al listener y lo que el thin no cubre
        // (11g, el cifrado nativo de red, Kerberos): medido el 2026-10-25.
        "oracle" => Json::obj([
            ("tipo", Json::s("oracle")),
            (
                "modos",
                Json::Arr(vec![Json::obj([
                    ("modo", Json::s("cadena")),
                    ("recomendado", Json::Bool(true)),
                    (
                        "dice",
                        Json::s(
                            "La credencial va dentro de la cadena de conexión, cifrada en el custodio. Crea un usuario que solo lea, con `SELECT` sobre las tablas que quieres conectar: el catálogo solo muestra lo que ese usuario puede leer. Con `tls=true` la conexión va cifrada y el certificado se verifica siempre.",
                        ),
                    ),
                    (
                        "formato",
                        Json::s(
                            "oracle://<usuario>:<contraseña>@<host>[:1521]/<SERVICIO>[?tls=true&esquema=<ESQUEMA>&timeoutMs=<ms>&fetch=<filas>] — o, sin servicios, con el SID: oracle://<usuario>:<contraseña>@<host>[:1521]/?sid=<SID>",
                        ),
                    ),
                    (
                        "pasos",
                        Json::Arr(vec![
                            Json::obj([
                                ("para", Json::s("El usuario que solo lee")),
                                (
                                    "comando",
                                    Json::s(
                                        "CREATE USER ore_lector IDENTIFIED BY \"<contraseña>\"; GRANT CREATE SESSION TO ore_lector; GRANT SELECT ON <ESQUEMA>.<TABLA> TO ore_lector;",
                                    ),
                                ),
                            ]),
                            Json::obj([
                                ("para", Json::s("El nombre de servicio, o el SID si la base no tiene servicios")),
                                (
                                    "comando",
                                    Json::s(
                                        "SELECT value FROM v$parameter WHERE name = 'service_names'; (o `lsnrctl services`). En una base multitenant es el de la PDB (`FREEPDB1`, `ORCLPDB1`…), no el de la CDB. Solo si la base no anuncia servicios, su SID: SELECT instance_name FROM v$instance; (se pone como `sid=` y sin ruta)",
                                    ),
                                ),
                            ]),
                            Json::obj([
                                ("para", Json::s("Que la celda llegue al listener")),
                                (
                                    "comando",
                                    Json::s(
                                        "Abre el puerto del listener (1521, o el de la URL) a la IP de salida de la celda ({ipSalida}; pregúntala a quien la opera)",
                                    ),
                                ),
                            ]),
                            Json::obj([
                                ("para", Json::s("Que el catálogo no recorra de más")),
                                (
                                    "comando",
                                    Json::s(
                                        "Añade `esquema=<ESQUEMA>` (el dueño de las tablas, en mayúsculas). Sin él se catalogan todas las que el usuario puede leer, menos las que mantiene Oracle",
                                    ),
                                ),
                            ]),
                        ]),
                    ),
                ])]),
            ),
            (
                "noAdmitidos",
                Json::Arr(vec![
                    Json::obj([
                        ("modo", Json::s("oracle-anterior-a-12-1")),
                        (
                            "porque",
                            Json::s(
                                "Oracle 11g y anteriores usan un protocolo que el driver (thin, sin librerías de Oracle) no habla: la conexión se rechaza. Hay que actualizar la base, o esperar a un segundo driver sobre el cliente nativo",
                            ),
                        ),
                    ]),
                    Json::obj([
                        ("modo", Json::s("cifrado-nativo-kerberos")),
                        (
                            "porque",
                            Json::s(
                                "el cifrado nativo de red de Oracle (NNE), Kerberos, RADIUS y LDAP no los habla el driver: se entra con usuario y contraseña, y el cifrado es TLS (`tls=true`)",
                            ),
                        ),
                    ]),
                    Json::obj([
                        ("modo", Json::s("tls-inseguro")),
                        (
                            "porque",
                            Json::s(
                                "`tlsInsecure`, `tlsAllowInvalidCertificates` o `verify=false` aceptan el certificado de cualquiera: otro se haría pasar por tu base y recibiría la contraseña",
                            ),
                        ),
                    ]),
                    Json::obj([
                        ("modo", Json::s("wallet-por-fichero")),
                        (
                            "porque",
                            Json::s(
                                "`wallet` y `tlsCAFile` con una ruta: el conector corre en un Job sin tus ficheros. Una wallet (Autonomous Database) o una CA privada todavía no se admiten",
                            ),
                        ),
                    ]),
                    Json::obj([
                        ("modo", Json::s("usuario-con-escritura")),
                        (
                            "porque",
                            Json::s(
                                "una fuente lee: con un usuario que pueda escribir o con `SELECT ANY TABLE` el driver podría más de lo que hace, y con `SELECT` sobre las tablas basta",
                            ),
                        ),
                    ]),
                ]),
            ),
        ]),
        // MongoDB (ADR 0069 M3): Atlas o cualquier despliegue. La credencial va
        // en la cadena, al custodio; lo que se enseña es el usuario que solo
        // lee y el acceso de red, que es donde Atlas se atasca la primera vez.
        "mongodb" | "mongodb_srv" => Json::obj([
            ("tipo", Json::s("mongodb")),
            (
                "modos",
                Json::Arr(vec![Json::obj([
                    ("modo", Json::s("cadena")),
                    ("recomendado", Json::Bool(true)),
                    (
                        "dice",
                        Json::s(
                            "La credencial va dentro de la cadena de conexión, cifrada en el                              custodio. Crea un usuario que solo lea, con el rol `read` sobre la                              base que quieres conectar. TLS se verifica siempre; Atlas firma con                              una CA pública y no hace falta más.",
                        ),
                    ),
                    (
                        "formato",
                        Json::s(
                            "mongodb+srv://<usuario>:<contraseña>@<cluster>.<id>.mongodb.net/<base>  ·                               mongodb://<usuario>:<contraseña>@<host>[:<puerto>][,<host>…]/<base>[?replicaSet=<nombre>]",
                        ),
                    ),
                    (
                        "pasos",
                        Json::Arr(vec![
                            Json::obj([
                                ("para", Json::s("Atlas: el usuario que solo lee")),
                                (
                                    "comando",
                                    Json::s(
                                        "Database Access → Add New Database User → Specific Privileges → rol `read` sobre `<base>`",
                                    ),
                                ),
                            ]),
                            Json::obj([
                                ("para", Json::s("Atlas: que la celda llegue al clúster")),
                                (
                                    "comando",
                                    Json::s(
                                        "Network Access → Add IP Address → la IP de salida de la celda ({ipSalida}; pregúntala a quien la opera)",
                                    ),
                                ),
                            ]),
                            Json::obj([
                                (
                                    "para",
                                    Json::s("Un despliegue propio: el usuario que solo lee"),
                                ),
                                (
                                    "comando",
                                    Json::s(
                                        "db.getSiblingDB(\"<base>\").createUser({user: \"ore_lector\", pwd: \"<contraseña>\", roles: [{role: \"read\", db: \"<base>\"}]})",
                                    ),
                                ),
                            ]),
                        ]),
                    ),
                ])]),
            ),
            (
                "noAdmitidos",
                Json::Arr(vec![
                    Json::obj([
                        ("modo", Json::s("tls-inseguro")),
                        (
                            "porque",
                            Json::s(
                                "`tlsInsecure` y `tlsAllowInvalidCertificates` aceptan el certificado de                                  cualquiera: otro se haría pasar por tu clúster y recibiría la contraseña",
                            ),
                        ),
                    ]),
                    Json::obj([
                        ("modo", Json::s("ca-por-fichero")),
                        (
                            "porque",
                            Json::s(
                                "`tlsCAFile` con una ruta: el driver corre en un Job sin tus ficheros.                                  Una CA privada todavía no se admite",
                            ),
                        ),
                    ]),
                    Json::obj([
                        ("modo", Json::s("usuario-con-escritura")),
                        (
                            "porque",
                            Json::s(
                                "una fuente lee: con `atlasAdmin`, `root` o `readWrite` el driver podría                                  más de lo que hace, y con `read` basta",
                            ),
                        ),
                    ]),
                    Json::obj([
                        ("modo", Json::s("aws-iam")),
                        (
                            "porque",
                            Json::s("la autenticación por IAM de AWS (`MONGODB-AWS`) aún no está"),
                        ),
                    ]),
                ]),
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
            r#""modo":"publico""#,
            "?publico=1",
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

    /// SharePoint (0061 O5·3): la app federada con los IDs puestos,
    /// Sites.Selected con consentimiento, la concesión por sitio, y lo que no
    /// se admite.
    #[test]
    fn sharepoint_pide_sites_selected_y_la_concesion_por_sitio() {
        let j = sharepoint(Some("111".into()), Some("333".into())).jcs();
        for x in [
            r#""tipo":"sharepoint""#,
            r#"\"subject\":\"111\""#,
            r#"\"subject\":\"333\""#,
            "admin-consent",
            SITES_SELECTED,
            "Grant-PnPEntraIDAppSitePermission",
            "-Permissions Read",
            r#""modo":"todo-el-tenant""#,
            r#""modo":"usuario""#,
            "tenant=<id del tenant>&cliente=<id de la app>",
        ] {
            assert!(j.contains(x), "{x}: {j}");
        }
        assert!(
            !j.contains("sinIds") && !j.contains("Sites.Read.All=Role"),
            "{j}"
        );
        let j = sharepoint(None, None).jcs();
        assert!(
            j.contains("ORE_ID_DRIVER") && j.contains("{iddriver}"),
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

    /// La pública, del entorno o del fichero que trae el contenedor de inicio;
    /// un fichero vacío o que no está es «sin clave».
    #[test]
    fn la_clave_publica_llega_por_fichero() {
        let d = std::env::temp_dir().join(format!("ore-sftp-pub-{}", std::process::id()));
        std::fs::write(&d, "ssh-ed25519 AAAAC3Nza ore-victor\n").unwrap();
        let f = d.to_string_lossy().to_string();
        let por = |k: &'static str, val: String| move |n: &str| (n == k).then(|| val.clone());
        assert_eq!(
            clave_publica(por("ORE_SFTP_CLAVE_PUBLICA_FICHERO", f.clone())).as_deref(),
            Some("ssh-ed25519 AAAAC3Nza ore-victor")
        );
        assert_eq!(
            clave_publica(por("ORE_SFTP_CLAVE_PUBLICA", "ssh-ed25519 X".into())).as_deref(),
            Some("ssh-ed25519 X")
        );
        std::fs::write(&d, "\n").unwrap();
        assert_eq!(
            clave_publica(por("ORE_SFTP_CLAVE_PUBLICA_FICHERO", f.clone())),
            None
        );
        std::fs::remove_file(&d).unwrap();
        assert_eq!(
            clave_publica(por("ORE_SFTP_CLAVE_PUBLICA_FICHERO", f)),
            None
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

    /// MongoDB: la credencial va en la cadena, el usuario solo lee, y lo que el
    /// driver rechaza se dice con su porqué (ADR 0069 M3).
    #[test]
    fn mongodb_va_en_la_cadena_con_un_usuario_que_solo_lee() {
        for tipo in ["mongodb", "mongodb_srv"] {
            let j = de(tipo, None, None).jcs();
            for x in [
                "\"tipo\":\"mongodb\"",
                "\"modo\":\"cadena\"",
                "mongodb+srv://",
                "rol `read`",
                "Network Access",
                "\"tls-inseguro\"",
                "\"usuario-con-escritura\"",
                "\"aws-iam\"",
            ] {
                assert!(j.contains(x), "{tipo}: {x}: {j}");
            }
        }
    }

    /// Cassandra: la credencial va en la cadena, el rol solo lee, `contacto=solo` para lo que
    /// hay detrás de un balanceador, y lo que el driver rechaza se dice con su porqué
    /// (ADR 0070 M3).
    #[test]
    fn cassandra_va_en_la_cadena_con_un_rol_que_solo_lee() {
        let j = de("cassandra", None, None).jcs();
        for x in [
            "\"tipo\":\"cassandra\"",
            "\"modo\":\"cadena\"",
            "cassandra://",
            "GRANT SELECT ON KEYSPACE",
            "contacto=solo",
            "{ipSalida}",
            "\"tls-inseguro\"",
            "\"rol-con-escritura\"",
            "\"astra-bundle\"",
        ] {
            assert!(j.contains(x), "{x}: {j}");
        }
    }

    /// Oracle: la credencial va en la cadena, el usuario solo lee, el servicio no es el SID, y lo que el thin no
    /// cubre se dice con su porqué (ADR 0074).
    #[test]
    fn oracle_va_en_la_cadena_con_un_usuario_que_solo_lee() {
        let j = de("oracle", None, None).jcs();
        for x in [
            "\"tipo\":\"oracle\"",
            "\"modo\":\"cadena\"",
            "oracle://",
            "GRANT SELECT ON",
            "o el SID si la base no tiene servicios",
            "?sid=<SID>",
            "{ipSalida}",
            "esquema=",
            "\"oracle-anterior-a-12-1\"",
            "\"cifrado-nativo-kerberos\"",
            "\"tls-inseguro\"",
            "\"wallet-por-fichero\"",
            "\"usuario-con-escritura\"",
        ] {
            assert!(j.contains(x), "{x}: {j}");
        }
    }

    /// Phoenix: la credencial va en la cadena y exige TLS; Kerberos y el TLS inseguro se dicen con su
    /// porqué (ADR 0071 M2).
    #[test]
    fn phoenix_va_en_la_cadena_y_exige_tls() {
        let j = de("phoenix", None, None).jcs();
        for x in [
            "\"tipo\":\"phoenix\"",
            "\"modo\":\"cadena\"",
            "phoenix://",
            "tls=true",
            "{ipSalida}",
            "\"tls-inseguro\"",
            "\"contrasena-sin-tls\"",
            "\"kerberos\"",
        ] {
            assert!(j.contains(x), "{x}: {j}");
        }
    }

    /// HBase: SIMPLE reconocido en la URL, sin credencial; Kerberos y SIMPLE sin reconocer se dicen con su porqué
    /// (ADR 0072 N7).
    #[test]
    fn hbase_va_por_el_protocolo_nativo_y_no_guarda_ninguna_credencial() {
        let j = de("hbase", None, None).jcs();
        for x in [
            "\"tipo\":\"hbase\"",
            "\"modo\":\"red-de-confianza\"",
            "hbase://",
            "simple=acepto",
            "{ipSalida}",
            "traduccion=",
            "physicalType",
            "\"simple-sin-reconocer\"",
            "\"kerberos\"",
        ] {
            assert!(j.contains(x), "{x}: {j}");
        }
    }

    /// Postgres: la credencial va en la cadena, y TLS siempre (ADR 0065 PG1).
    #[test]
    fn postgres_va_en_la_cadena_y_verifica_tls() {
        let j = de("postgres", None, None).jcs();
        for x in [
            "\"modo\":\"cadena\"",
            "sslrootcert",
            "verify-ca",
            "\"en-claro\"",
            "\"cualquier-certificado\"",
            "\"tunel-ssh\"",
            "ssh_huella",
        ] {
            assert!(j.contains(x), "{x}: {j}");
        }
    }
}
