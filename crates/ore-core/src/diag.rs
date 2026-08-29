//! Diagnósticos.
//!
//! El formato del mensaje **no es normativo**; el código y la cadena causal sí
//! (`99-errors.md` §1). Pero la especificación también dice que una
//! implementación **DEBERÍA** emitir ruta y línea, y la tesis del proyecto es
//! que *el error es el producto*: un código sin sitio donde mirar convierte una
//! garantía en una molestia.

use crate::Code;
use std::fmt;
use std::path::{Path, PathBuf};

/// Dónde ocurrió algo. Línea y columna en base 1, como las cuentan los editores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
}

impl fmt::Display for Pos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.col)
    }
}

/// Un fallo, con todo lo que hace falta para arreglarlo.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub code: Code,
    /// Qué ocurre, en una línea y en el vocabulario del usuario.
    pub message: String,
    pub file: PathBuf,
    pub pos: Option<Pos>,
    /// Qué hacer al respecto. Opcional, y vale más que el mensaje cuando existe.
    pub help: Option<String>,
}

impl Diagnostic {
    pub fn new(code: Code, file: impl Into<PathBuf>, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            file: file.into(),
            pos: None,
            help: None,
        }
    }

    pub fn at(mut self, pos: Pos) -> Self {
        self.pos = Some(pos);
        self
    }

    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// Renderiza relativo a una raíz, para que la ruta que se lee sea la que el
    /// usuario reconoce y no una absoluta con el nombre de su portátil.
    pub fn render(&self, root: &Path) -> String {
        let file = self
            .file
            .strip_prefix(root)
            .unwrap_or(&self.file)
            .display()
            .to_string();
        let file = file.replace('\\', "/");
        let loc = match self.pos {
            Some(p) => format!("{file}:{p}"),
            None => file,
        };
        let mut s = format!("error[{}]: {}\n  → {}", self.code, self.message, loc);
        if let Some(h) = &self.help {
            s.push_str(&format!("\n  ayuda: {h}"));
        }
        s
    }
}
