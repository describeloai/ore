//! Lo que el ciclo le pide a un almacén, y nada más: seis verbos sobre claves y
//! bytes. Ni entidades, ni conductos, ni vistas — el almacén no sabe qué guarda.

/// Un almacén de objetos con nombre. Las claves son las de `sobre.rs`
/// (`ore/v1/<digest>`, `ore/v1/plan/...`): el nombre ES el contenido, así que
/// `subir` puede devolver `false` —ya estaba, y era lo mismo— sin mirar dentro.
pub trait Almacen {
    /// Un objeto pequeño, como texto. Sólo se usa para el recibo.
    fn leer(&self, clave: &str) -> Result<Option<String>, String>;
    /// ¿Está? Sin bajarlo.
    fn existe(&self, clave: &str) -> Result<bool, String>;
    /// Sube si no estaba. `Ok(false)` = ya estaba lo mismo.
    fn subir(&self, clave: &str, cuerpo: &[u8]) -> Result<bool, String>;
    /// Las claves bajo un prefijo.
    fn listar(&self, prefijo: &str) -> Result<Vec<String>, String>;
    /// Borra; borrar lo que no está no es un error.
    fn borrar(&self, clave: &str) -> Result<(), String>;
    /// Un objeto entero: la copia anterior, para fundir.
    fn leer_bytes(&self, clave: &str) -> Result<Option<Vec<u8>>, String>;
}
