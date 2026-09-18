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
//! # Por un subproceso, como el KMS
//!
//! Misma razón que `kms.rs`: este programa no habla con el Secret Manager,
//! habla con el **cliente**, que resuelve Workload Identity contra los
//! metadatos. Y el valor entra por la entrada estándar (`--data-file=-`) y sale
//! por la salida estándar: **no toca el disco**.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// A qué cliente se le habla y en qué proyecto y región vive el almacén.
pub struct Almacen {
    /// El binario, como ruta. Ver `kms.rs`.
    pub programa: PathBuf,
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

impl Almacen {
    fn correr(&self, args: &[&str], entrada: Option<&[u8]>) -> Result<Vec<u8>, String> {
        let mut hijo = Command::new(&self.programa)
            .args(["secrets"])
            .args(args)
            .args(["--project", &self.proyecto])
            .stdin(if entrada.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                format!(
                    "no se pudo ejecutar `{}`: {e}. Es el cliente de la nube, y este \
                     programa no habla con el almacen: habla con el",
                    self.programa.display()
                )
            })?;
        if let Some(bytes) = entrada {
            hijo.stdin
                .take()
                .ok_or("no se pudo escribir en el cliente")?
                .write_all(bytes)
                .map_err(|e| format!("no se pudo escribir en el cliente: {e}"))?;
        }
        let salida = hijo
            .wait_with_output()
            .map_err(|e| format!("el cliente no termino: {e}"))?;
        if !salida.status.success() {
            // ⚠️ El error del cliente, entero y sin el valor. Un `PERMISSION_DENIED`
            //   con el prefijo dice exactamente qué condición no se cumple.
            let e = String::from_utf8_lossy(&salida.stderr);
            return Err(format!(
                "el almacen se nego a `{}`: {}",
                args.first().copied().unwrap_or("?"),
                e.trim().lines().next().unwrap_or("sin motivo")
            ));
        }
        Ok(salida.stdout)
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
    /// Idempotente: si ya existe, no pasa nada — un `emitir` que se quedó a
    /// medias entre el almacén y la base se termina volviendo a llamar.
    ///
    /// ⚠️ La política de réplica va por FICHERO porque `--kms-key-name` sólo
    ///   vale con réplica automática, que exige una llave global; la nuestra es
    ///   regional a propósito. El fichero no es secreto: dice dónde y con qué
    ///   llave, no qué.
    pub fn crear(&self, nombre: &str, kek: &str, inquilino: &str) -> Result<(), String> {
        let politica = format!(
            r#"{{"userManaged":{{"replicas":[{{"location":"{}","customerManagedEncryption":{{"kmsKeyName":"{}"}}}}]}}}}"#,
            self.lugar,
            self.cmek(kek)?
        );
        let fichero = std::env::temp_dir().join(format!("replica-{nombre}.json"));
        std::fs::write(&fichero, politica)
            .map_err(|e| format!("no se pudo escribir la politica de replica: {e}"))?;
        let r = self.correr(
            &[
                "create",
                nombre,
                &format!("--replication-policy-file={}", fichero.display()),
                &format!("--labels=proyecto=ore,inquilino={inquilino}"),
            ],
            None,
        );
        let _ = std::fs::remove_file(&fichero);
        match r {
            Ok(_) => Ok(()),
            Err(e) if e.contains("already exists") || e.contains("ALREADY_EXISTS") => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Añade una versión con el valor, que entra por la entrada estándar y no
    /// toca el disco. Devuelve el número de versión que el almacén le dio.
    pub fn anadir(&self, nombre: &str, valor: &[u8]) -> Result<i64, String> {
        let salida = self.correr(
            &[
                "versions",
                "add",
                nombre,
                "--data-file=-",
                "--format=value(name)",
            ],
            Some(valor),
        )?;
        version_de(&String::from_utf8_lossy(&salida))
    }

    /// Borra el secreto entero del almacén, con todas sus versiones. Que no
    /// esté ya no es un error: una baja que se quedó a medias entre el almacén
    /// y la base se termina volviendo a llamar, como `crear`.
    pub fn borrar(&self, nombre: &str) -> Result<(), String> {
        match self.correr(&["delete", nombre, "--quiet"], None) {
            Ok(_) => Ok(()),
            Err(e) if e.contains("NOT_FOUND") || e.contains("not found") => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// El valor de la última versión, y cuál es. Dos llamadas y no una: el
    /// `access` devuelve el valor crudo, sin envolver, y así no hay que
    /// descodificar nada aquí; el `describe` dice el número.
    pub fn leer(&self, nombre: &str) -> Result<(Vec<u8>, i64), String> {
        let valor = self.correr(&["versions", "access", "latest", "--secret", nombre], None)?;
        let cual = self.correr(
            &[
                "versions",
                "describe",
                "latest",
                "--secret",
                nombre,
                "--format=value(name)",
            ],
            None,
        )?;
        Ok((valor, version_de(&String::from_utf8_lossy(&cual))?))
    }
}

/// `projects/…/secrets/<n>/versions/<v>` → `v`.
fn version_de(nombre: &str) -> Result<i64, String> {
    nombre
        .trim()
        .rsplit_once("/versions/")
        .and_then(|(_, v)| v.parse().ok())
        .ok_or_else(|| format!("el almacen no dijo que version es: `{}`", nombre.trim()))
}

pub fn ruta_de(p: &Path) -> String {
    p.display().to_string()
}

#[cfg(test)]
mod prueba {
    use super::{nombre_en_almacen, version_de};

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
}
