//! Lo que el ciclo le pide a un almacén, y nada más: unos verbos sobre claves y
//! bytes. Ni entidades, ni conductos, ni vistas — el almacén no sabe qué guarda.
//!
//! Desde W3.6a (2026-09-20) el almacén también es **el suelo de un lago**: la
//! tabla Iceberg de un dataset vive en él como objetos con nombre (`ore/v2/…/
//! data/*.parquet`, `…/metadata/*.avro`, `…/metadata/*.metadata.json`), y
//! `lago.rs` le da a `iceberg` este mismo trato de almacén como su `Storage`.
//! Por eso el rasgo es `Send + Sync` —el escritor de Iceberg es asíncrono— y
//! por eso sabe decir su **base** (`gs://<bucket>`, `s3://<bucket>`): los
//! metadatos de una tabla Iceberg nombran sus ficheros por URI absoluta, y esa
//! URI es lo que un lector ajeno (DuckDB en el puesto, PyIceberg) va a pedir.

/// **Una credencial prestada** (0031 §11 ③): lo que un escritor de fuera
/// —PyIceberg, DuckDB, el agente del puesto— recibe del catálogo al cargar
/// una tabla para escribir sus ficheros **sólo bajo el prefijo de esa tabla**.
/// `config` lleva las claves que la spec REST de Iceberg nombra
/// (`gcs.oauth2.token`, `s3.access-key-id`, …); `caduca_ms`, cuándo deja de
/// valer, si deja.
pub struct Prestamo {
    pub config: std::collections::BTreeMap<String, String>,
    pub caduca_ms: Option<i64>,
    /// Si la credencial está de verdad acotada al prefijo, o es la de siempre.
    pub acotada: bool,
}

/// **Un blob que se sube** (0046 E8·2): su nombre es su contenido, y lleva lo
/// que el almacén necesita para cotejarlo **antes** de que el objeto exista.
pub struct Blob {
    pub clave: String,
    /// El `Content-Type` con el que se servirá.
    pub tipo: String,
    pub tamano: u64,
    pub sha256: [u8; 32],
    pub crc32c: u32,
    pub cuerpo: Cuerpo,
}

/// Dónde están los bytes de un blob: en memoria si es pequeño, en un fichero
/// temporal si no (la memoria no crece con el fichero).
pub enum Cuerpo {
    Memoria(Vec<u8>),
    Fichero(std::path::PathBuf),
}

impl Cuerpo {
    /// Los bytes enteros. Sólo para almacenes que no saben subir en flujo.
    pub fn bytes(&self) -> Result<Vec<u8>, String> {
        match self {
            Cuerpo::Memoria(b) => Ok(b.clone()),
            Cuerpo::Fichero(p) => std::fs::read(p)
                .map_err(|e| format!("el temporal `{}` no se pudo leer: {e}", p.display())),
        }
    }

    /// Un lector de los bytes, en flujo.
    pub fn lector(&self) -> Result<Box<dyn std::io::Read + '_>, String> {
        match self {
            Cuerpo::Memoria(b) => Ok(Box::new(&b[..])),
            Cuerpo::Fichero(p) => std::fs::File::open(p)
                .map(|f| Box::new(std::io::BufReader::with_capacity(1 << 20, f)) as Box<_>)
                .map_err(|e| format!("el temporal `{}` no se pudo abrir: {e}", p.display())),
        }
    }
}

/// Un almacén de objetos con nombre.
pub trait Almacen: Send + Sync {
    /// La raíz por la que este almacén se nombra desde fuera, sin barra final:
    /// `gs://<bucket>`, `s3://<bucket>`. `base() + "/" + clave` es la URI de un
    /// objeto, y es lo que va escrito dentro de los metadatos de Iceberg.
    fn base(&self) -> String;
    /// Un objeto pequeño, como texto.
    fn leer(&self, clave: &str) -> Result<Option<String>, String>;
    /// ¿Está? Sin bajarlo.
    fn existe(&self, clave: &str) -> Result<bool, String>;
    /// Cuántos bytes tiene, sin bajarlo. `None` es que no está.
    fn tamano(&self, clave: &str) -> Result<Option<u64>, String> {
        Ok(self.leer_bytes(clave)?.map(|b| b.len() as u64))
    }
    /// Sube si no estaba. `Ok(false)` = ya estaba, y no se toca.
    fn subir(&self, clave: &str, cuerpo: &[u8]) -> Result<bool, String>;
    /// Las claves bajo un prefijo.
    fn listar(&self, prefijo: &str) -> Result<Vec<String>, String>;
    /// Borra; borrar lo que no está no es un error.
    fn borrar(&self, clave: &str) -> Result<(), String>;
    /// **Sube aunque estuviera.** Borrar y subir, y no un tercer verbo por
    /// almacén. Ningún fichero de una tabla Iceberg se reescribe —cada nombre
    /// lleva un UUID—, así que hoy sólo lo usan las pruebas.
    fn sobrescribir(&self, clave: &str, cuerpo: &[u8]) -> Result<(), String> {
        self.borrar(clave)?;
        self.subir(clave, cuerpo).map(|_| ())
    }
    /// Un objeto entero.
    fn leer_bytes(&self, clave: &str) -> Result<Option<Vec<u8>>, String>;
    /// **Sube un blob si no estaba**, cotejado por el servidor antes de que
    /// exista (medido en 0046 E8·2 A1: la subida `media` de GCS **ignora** el
    /// hash que se le manda; la multiparte y la reanudable lo cotejan).
    /// `Ok(false)` = ya estaba, y como el nombre es el contenido, lo mismo.
    /// Por defecto, `subir` con los bytes enteros.
    fn poner_blob(&self, b: &Blob) -> Result<bool, String> {
        self.subir(&b.clave, &b.cuerpo.bytes()?)
    }
    /// Un rango de un objeto (`inicio..=fin`), o entero. `None`: no está.
    fn leer_rango(
        &self,
        clave: &str,
        rango: Option<(u64, u64)>,
    ) -> Result<Option<Vec<u8>>, String> {
        Ok(self.leer_bytes(clave)?.map(|b| match rango {
            None => b,
            Some((a, z)) => {
                let a = (a as usize).min(b.len());
                let z = (z as usize).saturating_add(1).min(b.len()).max(a);
                b[a..z].to_vec()
            }
        }))
    }
    /// **Presta una credencial acotada a `prefijo`** (leer y crear, nunca
    /// borrar ni sobrescribir): lo que el catálogo devuelve al cargar una
    /// tabla con `X-Iceberg-Access-Delegation: vended-credentials`. Un almacén
    /// que no sabe acotar lo dice.
    fn prestar(&self, prefijo: &str) -> Result<Prestamo, String> {
        Err(format!(
            "este almacén no presta credenciales acotadas (se pidió `{prefijo}`)"
        ))
    }

    /// **Presta una credencial acotada a `prefijo` sólo para leer** (0031
    /// W3.7 gobierno ②b): lo que `datos_del_puesto` devuelve para que el SDK
    /// lea el dataset con ella y no con la identidad del pod, que desde ②b no
    /// ve los datasets del bucket. Por defecto, la misma que para escribir:
    /// un almacén que no distingue presta lo que tiene.
    fn prestar_lectura(&self, prefijo: &str) -> Result<Prestamo, String> {
        self.prestar(prefijo)
    }
}
