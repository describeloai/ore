//! **El lago de cada tabla** (0081 §7.5, C10·2). En las pruebas y en un proceso de confianza hay un solo lago, con la
//! credencial del bucket. En un puesto no: el pod no ve el bucket (0031 W3.7 gobierno ②b), y cada tabla se lee y se
//! escribe con la credencial que `/v1` presta para ella, a la persona del puesto (*credential vending*). Por eso el
//! catálogo, el sink y el codec piden el lago **de una tabla**: su nombre y dónde está.
//!
//! Pedirlo no lee nada: un lago prestado pide su credencial la primera vez que la necesita, y la renueva antes de
//! que caduque ([`crate::v1::LagosV1`]). Así un plan que viaja se decodifica sin E/S (C8) y cada ejecutor canjea la
//! suya: la credencial no viaja en el plan (DF7).

use crate::foto::Nombre;
use ore_store::lago::Lago;
use std::fmt::Debug;

/// **De dónde sale el lago de una tabla.**
pub trait Lagos: Send + Sync + Debug {
    /// El lago de `nombre`, cuyos ficheros están bajo `ubicacion` (`gs://bucket/…`). No lee nada.
    fn de(&self, nombre: &Nombre, ubicacion: &str) -> Result<Lago, String>;

    /// Lo que `/v1` acaba de prestar para `nombre` (el `storage-credentials` de un `loadTable` o de un esbozo): así no
    /// se vuelve a pedir. Por defecto, nada.
    fn sembrar(&self, nombre: &Nombre, prestado: &serde_json::Value) {
        let _ = (nombre, prestado);
    }
}

/// Un solo lago para todas las tablas: el de las pruebas, o el de un proceso con la credencial del bucket.
impl Lagos for Lago {
    fn de(&self, _nombre: &Nombre, _ubicacion: &str) -> Result<Lago, String> {
        Ok(self.clone())
    }
}

/// El bucket de una ubicación: `gs://bucket/…` → `bucket`.
pub fn bucket_de(ubicacion: &str) -> Result<String, String> {
    let (_, resto) = ubicacion
        .split_once("://")
        .ok_or_else(|| format!("`{ubicacion}` no es una URI de un almacén"))?;
    let bucket = resto.split('/').next().unwrap_or_default();
    if bucket.is_empty() {
        return Err(format!("`{ubicacion}` no nombra un bucket"));
    }
    Ok(bucket.to_string())
}
