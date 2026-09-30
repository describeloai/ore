//! El almacén: dónde vive el MATERIAL de un secreto desde la 0024-⑤.
//!
//! # Lo que esto sustituye
//!
//! Hasta el 2026-09-14 el valor cifrado vivía en `cofre.material`, en la base
//! **central** de `iam`, y el cofre —que corre EN el inquilino— lo alcanzaba por
//! `5432` hacia `identidad`. Medido en `medida-el-cofre-y-su-almacen.py`: el
//! claro sólo existía en memoria del pod del inquilino, pero el cifrado y la
//! llave estaban en el plano de control. Lo que hacen los que viven de BYOC es
//! lo contrario y está escrito —Redpanda: *«those secrets never leave the data
//! plane account or network»*—, y la 0024-⑤ lo adopta: **el material va al
//! Secret Manager de la celda**, y el plano de control se queda con el metadato
//! (`cofre.secreto`, `iam.concesion`).
//!
//! # Y la llave no cambia de sitio: cambia quién la aplica
//!
//! `kms.rs` cerraba el valor a mano con la KEK de la organización. Aquí esa
//! misma llave —`iam.organizacion.kek`, `<llavero>/<clave>`— pasa a ser la
//! **CMEK del secreto**: el Secret Manager la aplica solo, y rotar la llave
//! sigue sin ser una caída porque cada versión queda cifrada con la que había.
//! Es la `021` («sin sobre») llevada a su conclusión: ni sobre ni cifrado
//! propio — ninguna criptografía escrita por nosotros.
//!
//! # ⛔ El aislamiento entre inquilinos, MEDIDO y no supuesto
//!
//! En `compartido` el almacén es UN proyecto para todos. Lo que separa a un
//! cofre de otro es una **condición IAM por prefijo** sobre la cuenta
//! `ore-cofre-<n>`:
//!
//! ```text
//! roles/secretmanager.admin  si  resource.name.startsWith("projects/<n>/secrets/t-<inq>-cofre-")
//! ```
//!
//! `medida-el-almacen-por-inquilino.py` lo probó desde dentro del pod: crea el
//! suyo, NO crea con el prefijo de otro (el `create` también obedece a la
//! condición), NO lee el testigo de otro, NO lista el proyecto. Por eso el
//! nombre en el almacén lleva el inquilino DELANTE: `t-<inq>-cofre-<nombre>`.
//! No es una convención de nombres — es el límite que la condición evalúa.
//!
//! # Por su API, y no por el cliente (0046 E9·3, medido el 2026-09-30)
//!
//! Hasta aquí se le hablaba al **cliente** (`gcloud secrets …`) por un
//! subproceso, con la frase de `kms.rs`. Medido en el pod del custodio de
//! victor: arrancar `gcloud` —Python— cuesta **3,6 s** antes de hacer nada, y
//! `leer` eran DOS llamadas (`access` y `describe`): **~8 s** por secreto
//! resuelto. Es lo que tardaba en servirse un ítem de una colección virtual
//! (ore-serve pide la credencial de la fuente aquí en cada petición). Por la
//! API REST, con el token de la cuenta que corre: **0,35 s**, y `access` ya dice
//! la versión, así que es UNA llamada.
//!
//! Lo que la frase protegía se queda, y dónde vive cada cosa:
//!
//! - **La autenticación** sigue siendo Workload Identity: el token lo da el
//!   metadata server a través de `ore-gcp`, que es la misma puerta de
//!   `ore-store` y `ore-read-bigquery` (que ya dejó `bq` por REST, 0042). No
//!   hay una sola llave en el clúster.
//! - **El TLS** es el de la plataforma (`native-tls`), como en esos dos: no hay
//!   criptografía escrita aquí.
//! - **El valor no toca el disco**: viaja en el cuerpo de la petición y de la
//!   respuesta, y la política de réplica ya no pasa por un fichero temporal.
//! - **El aislamiento** es el mismo, porque es IAM y no el cliente: la
//!   condición por prefijo evalúa el nombre del recurso igual venga de donde
//!   venga la llamada.
//!
//! ⚠️ El KMS (`kms.rs`) sigue con `gcloud`: sólo lo usa la mudanza, que corre
//!   una vez, y por eso la imagen del custodio lo conserva.

use ore_core::json::Json;
use std::time::Duration;

/// La API de Secret Manager. La réplica es `userManaged` en UNA región, pero el
/// recurso es global: su punto es éste, no uno regional.
const API: &str = "https://secretmanager.googleapis.com/v1";

/// ⚠️ Sólo para `pruebas-de-fuego/el-cofre.sh`, que levanta un almacén de
///   mentira por HTTP (con `ORE_GCP_TOKEN` fijo). En un pod no se pone: sin
///   ella se habla con Google.
const API_DE_PRUEBA: &str = "ORE_SECRETOS_API";

/// Cuánto se espera a Secret Manager. Holgado: una lectura mide 0,35 s, y un
/// custodio que se cuelga cuelga a quien le pide la credencial.
const PLAZO: Duration = Duration::from_secs(30);

/// Con qué habla y en qué proyecto y región vive el almacén.
pub struct Almacen {
    /// `API`, o la de `ORE_SECRETOS_API` en la prueba.
    pub api: String,
    /// El token de la cuenta que corre (Workload Identity), renovado.
    pub credencial: ore_gcp::Credencial,
    /// HTTPS con el TLS de la plataforma.
    pub agente: ureq::Agent,
    /// El proyecto de la celda. Hace falta ENTERO para nombrar la CMEK:
    /// `projects/<p>/locations/<lugar>/keyRings/<llavero>/cryptoKeys/<clave>`.
    pub proyecto: String,
    /// La región: de la réplica del secreto y del llavero. Una sola a
    /// propósito — una llave regional exige réplica regional.
    pub lugar: String,
}

/// Cómo se llama un secreto en el almacén. El inquilino DELANTE: es lo que la
/// condición IAM evalúa, y por eso no es decorativo.
pub fn nombre_en_almacen(inquilino: &str, nombre: &str) -> String {
    format!("t-{inquilino}-cofre-{nombre}")
}

/// ⛔ Un nombre de secreto va DENTRO de una URL: sólo lo que Secret Manager
///   admite (`[A-Za-z0-9_-]`, hasta 255). Otra cosa —una `/`, un `?`— cambiaría
///   el recurso al que se le habla, y el nombre es lo que la condición IAM mira.
fn admisible(nombre: &str) -> Result<&str, String> {
    if !nombre.is_empty()
        && nombre.len() <= 255
        && nombre
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        Ok(nombre)
    } else {
        Err(format!("`{nombre}` no es un nombre de secreto"))
    }
}

/// El mensaje de Google, literal: un `PERMISSION_DENIED` con el prefijo dice
/// exactamente qué condición no se cumple, y resumirlo lo esconde.
fn mensaje(cuerpo: &str) -> String {
    ore_core::parse::parse(cuerpo)
        .ok()
        .and_then(|n| {
            n.get("error")
                .and_then(|(_, e)| e.get("message"))
                .and_then(|(_, m)| m.as_str().map(String::from))
        })
        .unwrap_or_else(|| cuerpo.trim().chars().take(200).collect())
}

impl Almacen {
    pub fn del_entorno(proyecto: String, lugar: String) -> Result<Almacen, String> {
        Ok(Almacen {
            api: std::env::var(API_DE_PRUEBA)
                .ok()
                .filter(|a| !a.is_empty())
                .unwrap_or_else(|| API.to_string()),
            credencial: ore_gcp::Credencial::del_entorno(),
            agente: ore_gcp::cliente()?,
            proyecto,
            lugar,
        })
    }

    fn secretos(&self) -> String {
        format!("{}/projects/{}/secrets", self.api, self.proyecto)
    }

    /// Una llamada: `Ok(Some(cuerpo))` si 2xx, `Ok(None)` si el código está en
    /// `tolera` (el «ya existe» de crear, el «no está» de borrar).
    fn llamar(
        &self,
        que: &str,
        r: ureq::Request,
        cuerpo: Option<&Json>,
        tolera: &[u16],
    ) -> Result<Option<String>, String> {
        let r = r
            .set(
                "authorization",
                &format!("Bearer {}", self.credencial.token()?),
            )
            .timeout(PLAZO);
        let respuesta = match cuerpo {
            Some(c) => r
                .set("content-type", "application/json")
                .send_string(&c.jcs()),
            None => r.call(),
        };
        match respuesta {
            Ok(ok) => ok
                .into_string()
                .map(Some)
                .map_err(|e| format!("el almacen contesto a `{que}` y no se pudo leer: {e}")),
            Err(ureq::Error::Status(c, _)) if tolera.contains(&c) => Ok(None),
            Err(ureq::Error::Status(c, r)) => Err(format!(
                "el almacen se nego a `{que}` ({c}): {}",
                mensaje(&r.into_string().unwrap_or_default())
            )),
            Err(e) => Err(format!("no se pudo hablar con el almacen (`{que}`): {e}")),
        }
    }

    /// La CMEK de una organización, a partir de su `kek` (`<llavero>/<clave>`).
    fn cmek(&self, kek: &str) -> Result<String, String> {
        let (llavero, clave) = kek
            .split_once('/')
            .filter(|(a, b)| !a.is_empty() && !b.is_empty() && !b.contains('/'))
            .ok_or_else(|| format!("`{kek}` no tiene la forma `<llavero>/<clave>`"))?;
        Ok(format!(
            "projects/{}/locations/{}/keyRings/{llavero}/cryptoKeys/{clave}",
            self.proyecto, self.lugar
        ))
    }

    /// Crea el secreto (sin versión) cifrado con la CMEK de la organización.
    /// Idempotente: si ya existe (409), no pasa nada — un `emitir` que se quedó
    /// a medias entre el almacén y la base se termina volviendo a llamar.
    ///
    /// ⚠️ Réplica `userManaged` en `lugar`: la CMEK es regional a propósito, y
    ///   la réplica automática exigiría una llave global.
    pub fn crear(&self, nombre: &str, kek: &str, inquilino: &str) -> Result<(), String> {
        let cuerpo = Json::obj([
            (
                "replication",
                Json::obj([(
                    "userManaged",
                    Json::obj([(
                        "replicas",
                        Json::Arr(vec![Json::obj([
                            ("location", Json::s(&self.lugar)),
                            (
                                "customerManagedEncryption",
                                Json::obj([("kmsKeyName", Json::s(self.cmek(kek)?))]),
                            ),
                        ])]),
                    )]),
                )]),
            ),
            (
                "labels",
                Json::obj([
                    ("proyecto", Json::s("ore")),
                    ("inquilino", Json::s(inquilino)),
                ]),
            ),
        ]);
        let r = self
            .agente
            .post(&self.secretos())
            .query("secretId", admisible(nombre)?);
        self.llamar("create", r, Some(&cuerpo), &[409]).map(|_| ())
    }

    /// Añade una versión con el valor. Devuelve el número que el almacén le dio.
    pub fn anadir(&self, nombre: &str, valor: &[u8]) -> Result<i64, String> {
        let url = format!("{}/{}:addVersion", self.secretos(), admisible(nombre)?);
        let cuerpo = Json::obj([(
            "payload",
            Json::obj([("data", Json::s(ore_gcp::base64(valor)))]),
        )]);
        let r = self
            .llamar("versions add", self.agente.post(&url), Some(&cuerpo), &[])?
            .unwrap_or_default();
        version_de(&campo(&r, &["name"]).unwrap_or_default())
    }

    /// Borra el secreto entero del almacén, con todas sus versiones. Que no
    /// esté (404) ya no es un error: una baja que se quedó a medias entre el
    /// almacén y la base se termina volviendo a llamar, como `crear`.
    pub fn borrar(&self, nombre: &str) -> Result<(), String> {
        let url = format!("{}/{}", self.secretos(), admisible(nombre)?);
        self.llamar("delete", self.agente.delete(&url), None, &[404])
            .map(|_| ())
    }

    /// El valor de la última versión, y cuál es. UNA llamada: `access` devuelve
    /// el nombre de la versión junto al valor (en base64).
    pub fn leer(&self, nombre: &str) -> Result<(Vec<u8>, i64), String> {
        let url = format!(
            "{}/{}/versions/latest:access",
            self.secretos(),
            admisible(nombre)?
        );
        let r = self
            .llamar("versions access", self.agente.get(&url), None, &[])?
            .unwrap_or_default();
        de_acceso(&r)
    }
}

/// Un campo de texto anidado de una respuesta JSON.
fn campo(json: &str, camino: &[&str]) -> Option<String> {
    let n = ore_core::parse::parse(json).ok()?;
    let mut v = &n;
    for k in camino {
        v = v.get(k)?.1;
    }
    v.as_str().map(String::from)
}

/// Lo que `access` devuelve → el valor y su versión.
fn de_acceso(json: &str) -> Result<(Vec<u8>, i64), String> {
    let datos = campo(json, &["payload", "data"])
        .ok_or("el almacen no devolvio el valor (`payload.data`)")?;
    let valor = ore_gcp::de_base64(&datos)?;
    Ok((
        valor,
        version_de(&campo(json, &["name"]).unwrap_or_default())?,
    ))
}

/// `projects/…/secrets/<n>/versions/<v>` → `v`.
fn version_de(nombre: &str) -> Result<i64, String> {
    nombre
        .trim()
        .rsplit_once("/versions/")
        .and_then(|(_, v)| v.parse().ok())
        .ok_or_else(|| format!("el almacen no dijo que version es: `{}`", nombre.trim()))
}

#[cfg(test)]
mod prueba {
    use super::{admisible, de_acceso, nombre_en_almacen, version_de};

    #[test]
    fn el_inquilino_va_delante() {
        // Es lo que la condición IAM evalúa: `t-<inq>-cofre-` como prefijo.
        assert_eq!(
            nombre_en_almacen("demo", "fuente-pg"),
            "t-demo-cofre-fuente-pg"
        );
    }

    #[test]
    fn la_version_sale_del_nombre() {
        assert_eq!(
            version_de("projects/1/secrets/t-demo-cofre-x/versions/7\n"),
            Ok(7)
        );
        assert!(version_de("projects/1/secrets/t-demo-cofre-x").is_err());
    }

    #[test]
    fn el_acceso_trae_el_valor_y_la_version_en_una() {
        // La forma de `versions/latest:access`: el valor en base64 y el nombre de
        // la versión que resultó ser «latest».
        let r = r#"{"name":"projects/1/secrets/t-demo-cofre-x/versions/3","payload":{"data":"czM6Ly9hOmJAYw==","dataCrc32c":"1"}}"#;
        assert_eq!(de_acceso(r), Ok((b"s3://a:b@c".to_vec(), 3)));
        assert!(de_acceso(r#"{"name":"projects/1/secrets/x/versions/3"}"#).is_err());
    }

    #[test]
    fn un_nombre_no_cambia_el_recurso() {
        // ⛔ El nombre va en la URL: una `/` o un `:` hablarían con OTRO recurso.
        assert!(admisible("t-demo-cofre-fuente-s3_demo").is_ok());
        for malo in ["", "t-demo/../otro", "x:access", "a?b", "a b"] {
            assert!(admisible(malo).is_err(), "{malo}");
        }
    }
}
