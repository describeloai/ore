//! La coordenada de un bucket vive en `ore-sigv4` (sin red, 0046 E9·3): el
//! firmante de `ore-serve` la lee igual que este lector.
//!
//! ⭐ Salvo una cosa (0046 E9b): si la fuente es un ROL (`role_arn`), el lector
//! lo canjea aquí —él sí habla por la red— por una credencial temporal de una
//! hora, con la identidad de la cuenta de su celda. Un Job de más de una hora
//! tendría que volver a pedirla: no se renueva.
pub use ore_sigv4::fuente::{Fuente, publica};

pub fn leer(url: &str) -> Result<Fuente, String> {
    let (url, _) = ore_sts::resolver(url, "ore-driver")?;
    ore_sigv4::fuente::leer(&url)
}
