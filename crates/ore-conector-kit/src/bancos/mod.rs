//! **Lo que cada origen aporta al kit**: cargar la semilla, decir dónde está y
//! contestar lo que sólo el origen sabe (si una consulta sigue viva, cuántas
//! sesiones hay, si algo se escribió).
//!
//! Lo que un origen no puede contestar devuelve `None`, y el caso que lo
//! necesita sale «no aplica» con su motivo: un bucket no tiene sesiones ni
//! consultas que cancelar.

use crate::semilla::Tabla;
use ore_core::json::Json;
use std::collections::BTreeMap;

pub mod azure;
pub mod bigquery;
pub mod gcs;
pub mod postgres;
pub mod s3;
pub mod sftp;
pub mod sharepoint;

pub trait Banco {
    /// `postgres`, `s3`…
    fn familia(&self) -> &'static str;

    /// Carga la semilla. Idempotente: lo que ya está como debe, se deja.
    fn cargar(&mut self) -> Result<(), String>;

    /// La URL del origen, con la credencial de lectura.
    fn url(&self) -> String;

    /// Otra credencial al mismo origen (caso 11), si la hay.
    fn url_alternativa(&self) -> Option<String>;

    /// La URL con una credencial que el origen rechaza (caso 12). `None` si
    /// el origen de pruebas no comprueba credenciales (el S3 de mentira).
    fn url_mala(&self) -> Option<String>;

    /// El objeto de una tabla de la semilla.
    fn objeto(&self, tabla: Tabla) -> String;

    /// Lo que la petición de una tabla lleva además (el `fichero` de S3).
    fn completar(&self, _tabla: Tabla, _peticion: &mut BTreeMap<String, Json>) {}

    /// Un objeto que tarda en dar su primera fila (casos 8 y 10), si lo hay.
    fn lenta(&self) -> Option<String> {
        None
    }

    /// Un objeto cuya lectura intentaría escribir en el origen (caso 9).
    fn que_escribe(&self) -> Option<String> {
        None
    }

    /// Cuántas escrituras llegaron a hacerse; `Err` si lo que las cuenta ya no
    /// está (alguien lo borró). `None` si el origen no se puede escribir.
    fn escrituras(&mut self) -> Option<Result<u64, String>> {
        None
    }

    /// Cuántas consultas del conector siguen ejecutándose en el origen sobre
    /// `objeto` (caso 10).
    fn consultas_vivas(&mut self, _objeto: &str) -> Option<u64> {
        None
    }

    /// Cuántas sesiones del conector hay abiertas en el origen (caso 11).
    fn sesiones(&mut self) -> Option<u64> {
        None
    }

    /// Lo que el origen cobró desde la última vez que se preguntó (caso 14).
    fn facturado(&mut self) -> Option<u64> {
        None
    }

    /// Cierra lo que un caso dejara vivo en el origen.
    fn limpiar(&mut self) {}
}
