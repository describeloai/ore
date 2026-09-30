//! **S3 sin red**: la firma SigV4, el bucket y la URL de una fuente (0046 E9·3).
//!
//! Salió de `ore-s3` para que algo pudiera **firmar sin poder llamar**: el
//! firmante de `ore-serve` (`ore-firmar-s3`) prefirma las URLs de los ítems de
//! una colección virtual con la credencial de su fuente, y la imagen del plano
//! de control promete que no puede leer un origen. Una promesa así sólo vale si
//! es del artefacto: este crate depende de `sha2` y de nada más, y
//! `ore-cli/tests/dependencias.rs` lo vigila en el cierre del firmante.
//!
//! `ore-s3` —las lecturas, con su cliente HTTP— lo reexporta: `ore_s3::firma`,
//! `ore_s3::Bucket` y `ore_s3::Credencial` siguen siendo los mismos.

pub mod firma;
pub mod fuente;

pub use firma::{Credencial, base64, hex, sha256};

/// Dónde está el bucket y con qué se firma.
#[derive(Clone, Debug)]
pub struct Bucket {
    /// `https://host` sin barra final. En AWS, el del bucket virtual
    /// (`https://<bucket>.s3.<region>.amazonaws.com`); en un S3 compatible
    /// (R2, MinIO), el del servicio, y el bucket va en la ruta.
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
    /// El bucket en la ruta (`/<bucket>/<clave>`) y no en el host.
    pub en_ruta: bool,
    pub credencial: Credencial,
}

impl Bucket {
    /// Un bucket de AWS, con el host virtual de su región.
    pub fn de_aws(bucket: &str, region: &str, credencial: Credencial) -> Bucket {
        Bucket {
            endpoint: format!("https://{bucket}.s3.{region}.amazonaws.com"),
            bucket: bucket.to_string(),
            region: region.to_string(),
            en_ruta: false,
            credencial,
        }
    }

    pub fn host(&self) -> String {
        self.endpoint
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_end_matches('/')
            .to_string()
    }

    /// La ruta canónica de una clave (o del bucket, con `None`): cada segmento
    /// codificado una vez, las barras intactas.
    pub fn ruta(&self, clave: Option<&str>) -> String {
        let clave = clave.map(|c| c.split('/').map(firma::uri).collect::<Vec<_>>().join("/"));
        match (self.en_ruta, clave) {
            (true, Some(c)) => format!("/{}/{c}", self.bucket),
            (true, None) => format!("/{}", self.bucket),
            (false, Some(c)) => format!("/{c}"),
            (false, None) => "/".to_string(),
        }
    }

    /// El ARN del bucket y el de sus objetos: lo que una política nombra, y lo
    /// que un fallo de permisos tiene que decir.
    pub fn arn(&self) -> String {
        format!("arn:aws:s3:::{}", self.bucket)
    }
}
