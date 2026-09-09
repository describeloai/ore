//! Cerrar y abrir, hablándole al **cliente** de la nube.
//!
//! # Por qué un subproceso y no una biblioteca
//!
//! Es el patrón de [`ore-read-bigquery`], que lo dice en una frase:
//!
//! > *«este programa no habla con BigQuery, habla con el [cliente]»*
//!
//! Aquí compra más que allí. Un cliente de KMS en Rust traería TLS, OAuth y la
//! resolución de credenciales del entorno — tres cosas cuyo modo de fallo es
//! silencioso y en la dirección insegura, que es la misma frase que este árbol
//! ya tiene escrita dos veces sobre la verificación de firmas.
//!
//! Y la autenticación sale gratis: `gcloud` resuelve Workload Identity contra el
//! servidor de metadatos, igual que en las copias y en el driver. **No hay una
//! sola llave en el clúster.**
//!
//! # ⛔ El valor en claro NO toca el disco
//!
//! `gcloud kms` admite `-` como fichero, que es «la entrada estándar» y «la
//! salida estándar». Escribirlo en un temporal sería dejar el secreto en el
//! sistema de ficheros del pod para que lo lea el proceso siguiente — que es
//! exactamente lo que `ore-serve` se niega a hacer cuando rechaza una URL con
//! credencial dentro.
//!
//! # Y no hay sobre
//!
//! Se cifra el valor **directamente con la KEK de la organización**. La `021`
//! dice por qué: una contraseña cabe de sobra en los 64 KiB que un KMS simétrico
//! admite, rotar crea una versión nueva que sigue abriendo lo viejo sola, y una
//! DEK nuestra habría significado escribir criptografía.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// A qué cliente se le habla y dónde viven las llaves.
pub struct Kms {
    /// El binario. Es una ruta y no un nombre para que un despliegue no dependa
    /// del `PATH` del contenedor — la misma razón que `ore-serve` da para su
    /// `--ore`.
    pub programa: PathBuf,
    /// La región del llavero. Es la CARRETERA: `iam.organizacion.kek` guarda el
    /// nombre, y de dónde se alcanza es configuración del despliegue.
    pub lugar: String,
}

/// `<llavero>/<clave>`, tal como lo guarda `iam.organizacion.kek`.
fn partir(kek: &str) -> Result<(&str, &str), String> {
    kek.split_once('/')
        .filter(|(a, b)| !a.is_empty() && !b.is_empty() && !b.contains('/'))
        .ok_or_else(|| format!("`{kek}` no tiene la forma `<llavero>/<clave>`"))
}

impl Kms {
    fn correr(&self, verbo: &str, kek: &str, entrada: &[u8]) -> Result<Vec<u8>, String> {
        let (llavero, clave) = partir(kek)?;
        let mut hijo = Command::new(&self.programa)
            .args([
                "kms",
                verbo,
                "--location",
                &self.lugar,
                "--keyring",
                llavero,
                "--key",
                clave,
                // ⛔ `-` en los dos: nada toca el disco.
                if verbo == "encrypt" {
                    "--plaintext-file=-"
                } else {
                    "--ciphertext-file=-"
                },
                if verbo == "encrypt" {
                    "--ciphertext-file=-"
                } else {
                    "--plaintext-file=-"
                },
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                format!(
                    "no se pudo ejecutar `{}`: {e}. Es el cliente de la nube, y este \
                     programa no habla con el KMS: habla con el",
                    ruta_de(&self.programa)
                )
            })?;

        hijo.stdin
            .take()
            .ok_or("no se pudo escribir en el cliente")?
            .write_all(entrada)
            .map_err(|e| format!("no se pudo escribir en el cliente: {e}"))?;

        let salida = hijo
            .wait_with_output()
            .map_err(|e| format!("el cliente no termino: {e}"))?;
        if !salida.status.success() {
            // ⚠️ El error del cliente, ENTERO y sin el valor. Un `PERMISSION_DENIED`
            //   sobre una clave dice exactamente qué falta, y resumirlo a «no se
            //   pudo» manda a mirar la red.
            let e = String::from_utf8_lossy(&salida.stderr);
            return Err(format!(
                "el KMS se nego a {verbo} con `{kek}`: {}",
                e.trim().lines().next().unwrap_or("sin motivo")
            ));
        }
        Ok(salida.stdout)
    }

    pub fn cerrar(&self, kek: &str, claro: &[u8]) -> Result<Vec<u8>, String> {
        self.correr("encrypt", kek, claro)
    }

    pub fn abrir(&self, kek: &str, cifrado: &[u8]) -> Result<Vec<u8>, String> {
        self.correr("decrypt", kek, cifrado)
    }
}

pub fn ruta_de(p: &Path) -> String {
    p.display().to_string()
}

#[cfg(test)]
mod prueba {
    use super::partir;

    #[test]
    fn la_forma_de_la_llave() {
        assert_eq!(partir("ore/demo"), Ok(("ore", "demo")));
        // ⛔ Sin llavero, sin clave, o con un segmento de mas: todo eso acabaria
        //   componiendo el nombre de un recurso de la nube.
        assert!(partir("demo").is_err());
        assert!(partir("ore/").is_err());
        assert!(partir("/demo").is_err());
        assert!(partir("ore/otro/demo").is_err());
    }
}
