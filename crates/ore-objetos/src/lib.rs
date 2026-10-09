//! **Un almacén de objetos como origen, de cualquier proveedor** (ADR 0061).
//!
//! Un bucket de S3, un contenedor de Azure, un bucket de GCS o una carpeta de
//! un SFTP guardan **ficheros**: se listan, se leen por rangos, y de cada uno
//! se sabe una versión (o sólo su ETag) y, a veces, una huella de contenido.
//! El catálogo, las filas de un Parquet, las versiones de una colección y la
//! bajada de sus bytes (`ore-read-objetos`) sólo necesitan eso, detrás de
//! [`Origen`]; cada proveedor lo implementa sobre su protocolo (`ore-s3` para
//! S3 y los que hablan su API).
//!
//! Lo que un proveedor sabe hacer —cómo fija una lectura, si firma URLs, qué
//! huella da sin bajar— lo dicen sus [`Capacidades`]: quien sirve una colección
//! no promete lo que el origen no da.

pub mod huella;
pub mod memoria;

use std::io::Read;

/// Un objeto de un listado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Objeto {
    pub clave: String,
    pub tamano: u64,
    /// El validador del origen (en S3, entre comillas). **No** es la huella
    /// del contenido.
    pub etag: String,
    /// ISO-8601.
    pub modificado: String,
}

/// **Una versión de un objeto, o una marca de borrado** (0046 E8·1): lo que
/// una colección necesita para fijar cada ítem a lo que era, y para saber si
/// lo retirado se sigue pudiendo leer. Un origen sin versionado da una por
/// objeto, la vigente, con su validador por versión (ver [`Fija`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub clave: String,
    /// La del origen. En S3, `null` si el objeto se subió antes de activar el
    /// versionado, o si el bucket nunca lo tuvo.
    pub version: String,
    /// La versión vigente de su clave.
    pub actual: bool,
    /// Una marca de borrado: la clave no se lista, y sus versiones siguen.
    pub marca: bool,
    pub tamano: u64,
    pub etag: String,
    pub modificado: String,
}

/// Lo que el origen dice de una versión al abrirla.
#[derive(Debug, Clone, Default)]
pub struct Abierto {
    pub tamano: Option<u64>,
    pub tipo: Option<String>,
    /// Su huella de contenido, si el origen la da al abrir, como la escribe
    /// la colección (`crc64nvme:<b64>`, ver [`huella`]).
    pub huella: Option<String>,
}

/// **Lo que el catálogo, las filas y la colección le piden a un almacén**:
/// listar y leer, fijado a lo que el listado dijo.
pub trait Origen {
    /// Todo lo que hay bajo un prefijo.
    fn listar(&self, prefijo: &str) -> Result<Vec<Objeto>, String>;
    /// Un rango de bytes: `0-15`, o el sufijo `-8`.
    fn rango(&self, clave: &str, rango: &str) -> Result<Vec<u8>, String>;
    /// **Leer (0046 E6)**: el objeto entero, en flujo, de la versión que el
    /// listado dijo (`etag`); si cambió, falla.
    fn abrir(&self, clave: &str, etag: &str) -> Result<Box<dyn Read + '_>, String>;
    /// Un rango de la versión que el listado dijo.
    fn rango_de(&self, clave: &str, rango: &str, etag: &str) -> Result<Vec<u8>, String>;
    /// **Las versiones y las marcas** bajo un prefijo (0046 E8·1).
    fn listar_versiones(&self, prefijo: &str) -> Result<Vec<Version>, String>;
    /// La huella de contenido de UNA versión, si el origen la da sin bajarla
    /// (en S3, su CRC64NVME `FULL_OBJECT`), o `None`.
    fn huella_de(&self, clave: &str, version: &str) -> Result<Option<String>, String>;
    /// **Una versión entera, en flujo** (E8·2): lo que la colección mantenida
    /// copia, con lo que el origen dice de ella al abrirla.
    fn abrir_version(
        &self,
        clave: &str,
        version: &str,
    ) -> Result<(Box<dyn Read + '_>, Abierto), String>;

    /// **Leer un ítem fijado** (0049 B3·1, ADR 0061 O0·3): lo que `ore-medios`
    /// sirve de una virtual. La `version` (vacía si el origen no versiona), el
    /// `etag` como precondición (`If-Match`, vacío si no) y el `rango` tal cual
    /// llegó (`bytes=…`). Lo que el origen contesta, ya en el idioma del
    /// contrato ([`Rechazo`]). Por defecto, este origen no sabe.
    fn leer_fijado(
        &self,
        clave: &str,
        version: &str,
        etag: &str,
        rango: Option<&str>,
    ) -> Result<Leido, Rechazo> {
        let _ = (clave, version, etag, rango);
        Err(Rechazo::Origen(
            "este origen no sabe servir un ítem fijado".into(),
        ))
    }

    /// **Una URL firmada de un ítem** (0049 H3), sin red: fijada a su versión,
    /// con el tipo y la disposición dentro de la firma, viva `segundos`.
    /// `None`: este origen no da URLs firmadas (se abre por `content`).
    fn firmar(
        &self,
        clave: &str,
        version: &str,
        tipo: &str,
        disposicion: &str,
        segundos: u64,
    ) -> Option<String> {
        let _ = (clave, version, tipo, disposicion, segundos);
        None
    }
}

/// Una lectura fijada que salió bien: el estado (`200`/`206`), sus cabeceras
/// (en minúscula) y el cuerpo **sin leer**.
pub struct Leido {
    pub estado: u16,
    pub cabeceras: Vec<(String, String)>,
    pub lector: Box<dyn Read + Send>,
}

impl Leido {
    pub fn cabecera(&self, k: &str) -> Option<&str> {
        self.cabeceras
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(k))
            .map(|(_, v)| v.as_str())
    }
}

/// **Por qué no se leyó un ítem fijado**, en el idioma del contrato
/// (`docs/media.md` §3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rechazo {
    /// La versión fijada ya no es la del origen (`media/cambiado`, 412).
    Cambiado(String),
    /// El rango no cabe en el ítem (`media/rango`, 416).
    Rango(String),
    /// El origen falló, o no contesta (`media/origen`, 502).
    Origen(String),
}

/// **Cómo fija un origen una lectura** a lo que el listado dijo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fija {
    /// Por su versión (S3 `versionId`, GCS `generation`, Azure `versionid`):
    /// lo de antes se sigue leyendo mientras exista.
    Version,
    /// Sólo por su validador (`If-Match`): si cambió, la lectura falla —nunca
    /// da otros bytes—, pero lo de antes ya no se lee.
    Etag,
    /// Nada (un SFTP): la versión es la copia que se hizo en el lago.
    Nada,
}

impl Fija {
    pub fn nombre(self) -> &'static str {
        match self {
            Fija::Version => "version",
            Fija::Etag => "etag",
            Fija::Nada => "ninguna",
        }
    }
}

/// **Lo que un origen de objetos sabe hacer** (ADR 0061, decisión 3): sale en
/// el verbo `capacidades` de su driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capacidades {
    pub fija: Fija,
    /// Si sabe dar una URL firmada de un ítem, sin red.
    pub firma: bool,
    /// La huella de contenido que da sin bajar (`crc64nvme`, `crc32c`, `md5`).
    pub huella: Option<&'static str>,
    /// Si su credencial es corta (STS, una SAS, un token federado).
    pub credencial_corta: bool,
}

impl Capacidades {
    /// Como va en `capacidades`: `{"fija":…,"firma":…,"huella":…,"credencial_corta":…}`.
    pub fn json(&self) -> String {
        format!(
            "{{\"credencial_corta\":{},\"fija\":\"{}\",\"firma\":{},\"huella\":{}}}",
            self.credencial_corta,
            self.fija.nombre(),
            self.firma,
            self.huella
                .map(|h| format!("\"{h}\""))
                .unwrap_or_else(|| "null".into())
        )
    }
}

/// **La versión de un objeto de un origen que no versiona** (ADR 0061 O1·1):
/// `etag:<su ETag>`. Cambia cuando cambian los bytes —así la colección ve el
/// cambio—, y nunca viaja como versión al origen: cada lectura la convierte en
/// `If-Match` ([`etag_de`]), y si el objeto cambió es un `412`, nunca otros
/// bytes (D-O1). Medido en Garage (como R2): ignora `versionId` —uno inventado
/// también da la actual— y respeta `If-Match`.
pub fn version_por_etag(etag: &str) -> String {
    format!("etag:{}", etag.trim_matches('"'))
}

/// El ETag, entre comillas para `If-Match`, de una versión que sólo es su ETag
/// ([`version_por_etag`]); `None` si es una versión del origen.
pub fn etag_de(version: &str) -> Option<String> {
    version
        .strip_prefix("etag:")
        .filter(|e| !e.is_empty())
        .map(|e| format!("\"{e}\""))
}

/// Hexadecimal en minúsculas.
pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// SHA-256 de unos bytes.
pub fn sha256(b: &[u8]) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    Sha256::digest(b).to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn una_version_por_etag_va_y_vuelve_y_una_del_origen_no() {
        let v = version_por_etag("\"294c48\"");
        assert_eq!(v, "etag:294c48");
        assert_eq!(etag_de(&v).as_deref(), Some("\"294c48\""));
        for del_origen in ["null", "01M4GA28E0PY7", "3HL4kqtJlcpXroDTDmJ", "", "etag:"] {
            assert_eq!(etag_de(del_origen), None, "{del_origen}");
        }
    }

    #[test]
    fn las_capacidades_se_dicen_en_json() {
        let c = Capacidades {
            fija: Fija::Etag,
            firma: true,
            huella: None,
            credencial_corta: false,
        };
        assert_eq!(
            c.json(),
            r#"{"credencial_corta":false,"fija":"etag","firma":true,"huella":null}"#
        );
    }
}
