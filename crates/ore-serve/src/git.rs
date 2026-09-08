//! El árbol vive en la forja, y este proceso **no se lo queda**.
//!
//! # La forma: clonar por petición
//!
//! No hay copia de trabajo de larga vida. Cada petición que toca el árbol
//! clona, opera, y —si escribió— empuja y tira el clon. Suena caro y está
//! medido que no lo es: el árbol de un almacén de 200 tablas con su historia
//! entera son **750 KB**, y la forja está a un salto dentro del clúster.
//!
//! Lo que compra es que **el servidor no tiene estado**. Se puede matar, se
//! puede replicar, y dos réplicas no divergen porque no hay nada que diverja:
//! el sistema de registro es la forja.
//!
//! # Y la carrera se resuelve sola, que es todo el motivo
//!
//! Dos personas contestan la misma cola de decisiones a la vez. Las dos clonan
//! el mismo commit, las dos deciden, las dos empujan. **La segunda es
//! rechazada** —no avanza la referencia— y eso llega aquí como un `409`, no
//! como una ontología con las dos respuestas mezcladas.
//!
//! Esto no lo hemos escrito nosotros: es lo que hace git, y es la razón por la
//! que el árbol vive aquí y no en un volumen. Un fichero compartido habría
//! aceptado las dos escrituras y ninguna prueba lo habría notado.
//!
//! # El testigo no viaja en `argv`
//!
//! Un `git push http://usuario:token@host/…` deja la credencial en la línea de
//! órdenes, que lee cualquier proceso de la máquina. Va por `GIT_CONFIG_*`, que
//! git lee del **entorno** del hijo. Es la misma frontera que `source add`
//! traza con `connectionEnv`: el secreto se dice dónde está, no se escribe.

use crate::identidad::Identidad;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A dónde empujar, y con qué.
pub struct Forja {
    /// La URL del repositorio, con `.git`.
    pub url: String,
    /// El testigo. Sale de una variable de entorno y **nunca** de `argv`.
    ///
    /// No hay campo de usuario, y no es un olvido: la cabecera `Authorization:
    /// token …` no lleva ninguno. Un `usuario` aquí sería un dato que nadie
    /// mira y que el día que alguien lo cambiara no cambiaría nada.
    pub testigo: String,
}

#[derive(Debug)]
pub enum Fallo {
    /// No se pudo hablar con la forja.
    NoResponde(String),
    /// El empujón fue rechazado: alguien escribió antes.
    Adelantado(String),
    /// Cualquier otra cosa que git dijo.
    Git(String),
}

impl std::fmt::Display for Fallo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Fallo::NoResponde(m) => write!(f, "la forja no responde: {m}"),
            Fallo::Adelantado(m) => write!(
                f,
                "alguien escribió en el árbol mientras se decidía esto, así que \
                 el empujón se rechazó y NADA se perdió. Hay que volver a leer y \
                 volver a decidir sobre lo que hay ahora: {m}"
            ),
            Fallo::Git(m) => write!(f, "git: {m}"),
        }
    }
}

/// Un directorio temporal que se borra al soltarlo.
///
/// Sin esto, un servidor que atiende mil peticiones deja mil clones en el
/// disco. Y no se limpia «al final»: se limpia cuando el valor muere, incluidos
/// los caminos de error, que son justo los que se olvidan.
pub struct Prestado(PathBuf);

impl Prestado {
    pub fn ruta(&self) -> &Path {
        &self.0
    }
}

impl Drop for Prestado {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl Forja {
    /// Los ajustes que van por el ENTORNO y no por la línea de órdenes.
    fn entorno(&self) -> Vec<(String, String)> {
        vec![
            ("GIT_TERMINAL_PROMPT".into(), "0".into()),
            ("GIT_CONFIG_COUNT".into(), "2".into()),
            ("GIT_CONFIG_KEY_0".into(), "http.extraheader".into()),
            (
                "GIT_CONFIG_VALUE_0".into(),
                format!("Authorization: token {}", self.testigo),
            ),
            // El clon lo crea este proceso, así que el dueño coincide y esto no
            // haría falta. Se pone porque el día que el directorio venga de un
            // volumen el fallo es «dubious ownership», que no se parece en nada
            // a su causa y cuesta media hora encontrar.
            ("GIT_CONFIG_KEY_1".into(), "safe.directory".into()),
            ("GIT_CONFIG_VALUE_1".into(), "*".into()),
        ]
    }

    fn git(&self, dir: Option<&Path>, args: &[&str]) -> Result<String, Fallo> {
        let mut c = Command::new("git");
        if let Some(d) = dir {
            c.current_dir(d);
        }
        for (k, v) in self.entorno() {
            c.env(k, v);
        }
        let s = c
            .args(args)
            .output()
            .map_err(|e| Fallo::Git(format!("no se pudo ejecutar `git`: {e}")))?;
        let err = String::from_utf8_lossy(&s.stderr).into_owned();
        if s.status.success() {
            return Ok(String::from_utf8_lossy(&s.stdout).into_owned());
        }
        let bajo = err.to_ascii_lowercase();
        Err(
            if bajo.contains("non-fast-forward") || bajo.contains("fetch first") {
                Fallo::Adelantado(primera(&err))
            } else if bajo.contains("could not resolve host") || bajo.contains("connection refused")
            {
                Fallo::NoResponde(primera(&err))
            } else {
                Fallo::Git(primera(&err))
            },
        )
    }

    /// Un clon fresco, en un directorio que se borra solo.
    pub fn clonar(&self) -> Result<Prestado, Fallo> {
        let destino = temporal();
        std::fs::create_dir_all(&destino)
            .map_err(|e| Fallo::Git(format!("no se pudo crear el directorio: {e}")))?;
        let prestado = Prestado(destino.clone());
        self.git(
            None,
            &["clone", "--quiet", &self.url, &destino.to_string_lossy()],
        )?;
        Ok(prestado)
    }

    /// ¿Cambió algo? Un `commit` vacío es ruido en la historia, y la historia
    /// **es** la auditoría: un commit por petición que no cambió nada convierte
    /// «quién cambió qué» en una lista de quién pasó por aquí.
    pub fn hay_cambios(&self, dir: &Path) -> bool {
        self.git(Some(dir), &["status", "--porcelain"])
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
    }

    /// Confirma lo que haya y lo empuja. Devuelve el commit.
    ///
    /// **El autor es el sujeto de la petición y el committer es el servidor.**
    /// No es un detalle de estilo: es la forma de RFC 8693 —`sub` más `act`—
    /// escrita donde se puede leer sin un sistema aparte. Quien decidió y qué
    /// lo ejecutó son dos cosas, y un registro que las funde no sirve para
    /// contestar ninguna de las dos.
    pub fn publicar(&self, dir: &Path, sujeto: &Identidad, mensaje: &str) -> Result<String, Fallo> {
        self.git(Some(dir), &["add", "-A"])?;

        let mut c = Command::new("git");
        c.current_dir(dir);
        for (k, v) in self.entorno() {
            c.env(k, v);
        }
        c.env("GIT_AUTHOR_NAME", &sujeto.persona)
            .env("GIT_AUTHOR_EMAIL", correo(&sujeto.persona))
            .env(
                "GIT_COMMITTER_NAME",
                sujeto.agente.clone().unwrap_or_else(|| "ore-serve".into()),
            )
            .env("GIT_COMMITTER_EMAIL", "ore-serve@ore.dev");
        let s = c
            .args(["commit", "--quiet", "-m", mensaje])
            .output()
            .map_err(|e| Fallo::Git(format!("no se pudo ejecutar `git`: {e}")))?;
        if !s.status.success() {
            return Err(Fallo::Git(primera(&String::from_utf8_lossy(&s.stderr))));
        }

        self.git(Some(dir), &["push", "--quiet", "origin", "HEAD"])?;
        Ok(self
            .git(Some(dir), &["rev-parse", "--short", "HEAD"])?
            .trim()
            .to_string())
    }
}

/// El correo de un sujeto que no tiene correo.
///
/// Git exige uno y no hay ninguno que sea cierto: el sujeto es un identificador,
/// no una dirección. Se compone uno **que se ve que no es una dirección**, en
/// vez de inventar algo que parezca real y acabe en un `mailto:`.
fn correo(persona: &str) -> String {
    let limpio: String = persona
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '.'
            }
        })
        .collect();
    format!("{limpio}@sujeto.invalid")
}

fn primera(s: &str) -> String {
    s.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
        .to_string()
}

fn temporal() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "ore-serve-arbol-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ))
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn el_testigo_no_aparece_en_los_argumentos() {
        let f = Forja {
            url: "http://forja/x.git".into(),
            testigo: "SECRETO".into(),
        };
        // Va en el entorno, y sólo ahí.
        let e = f.entorno();
        assert!(e.iter().any(|(_, v)| v.contains("SECRETO")));
        assert!(!f.url.contains("SECRETO"));
    }

    /// Un correo compuesto tiene que verse compuesto. `.invalid` está reservado
    /// por el RFC 2606 justo para esto.
    #[test]
    fn el_correo_del_sujeto_no_finge_ser_uno() {
        assert_eq!(correo("persona:ana"), "persona.ana@sujeto.invalid");
        assert!(correo("cualquiera").ends_with("@sujeto.invalid"));
    }

    #[test]
    fn el_prestado_se_borra_solo() {
        let d = temporal();
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("a"), "x").unwrap();
        let camino = d.clone();
        {
            let _p = Prestado(d);
            assert!(camino.exists());
        }
        assert!(!camino.exists(), "el clon sobrevivió a quien lo pidió");
    }
}
