//! **La política de las ramas** (P1 de la rama protegida): si `main` es libre o
//! está protegida. Un fichero del árbol, `.arbol/ramas.yaml`, que lee ore-serve.
//!
//! ```yaml
//! main:
//!   protegida: true
//! ```
//!
//! # Por qué un fichero, y por qué ése
//!
//! Medido (P1.0, 2026-09-28): cambiarla ha de pasar por una propuesta —con
//! `main` protegida, quitarla exige la revisión de otra persona—, y una
//! propuesta sólo lleva contenido del árbol. Ni la protección de rama de la
//! forja (un ajuste, sin historia ni PR) ni IAM (ore-serve no lo ve) sirven.
//!
//! En una **carpeta oculta** porque el compilador no entra en ellas
//! (`validate.rs`, `recolectar`): es maquinaria del árbol, como `.github/`, y
//! no ontología —otra implementación de OOS sin forja no tiene ramas—, así que
//! no es un `kind` de la spec. Y no en `.ore/`, que es la caché del compilador
//! y está en el `.gitignore`: la política no llegaría nunca a la forja.
//!
//! # Siempre la de `main`
//!
//! Se lee de la rama por defecto y nunca de la rama de una propuesta: si no,
//! una PR podría aflojar la regla que se le aplica a ella misma.
//!
//! Sin fichero, o sin `protegida: true`, `main` es **libre**: lo que el árbol
//! ya era antes de que esto existiera.
use std::path::Path;

/// Dónde vive, desde la raíz del árbol.
pub(crate) const RUTA: &str = ".arbol/ramas.yaml";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Politica {
    /// Con `main` protegida no se escribe en ella sin propuesta, y fusionar
    /// exige la aprobación de otra persona.
    pub protegida: bool,
}

impl Politica {
    /// Del texto del fichero. Lo que no se entiende es libre: un fichero roto
    /// no puede encerrar a nadie fuera de su árbol.
    pub(crate) fn de_texto(texto: &str) -> Politica {
        let protegida = ore_core::parse::parse(texto)
            .ok()
            .and_then(|n| {
                n.get("main")
                    .and_then(|(_, m)| m.get("protegida"))
                    .and_then(|(_, p)| p.as_str().map(|s| s.trim() == "true"))
            })
            .unwrap_or(false);
        Politica { protegida }
    }

    /// De un árbol en disco (un clon de `main`, o el directorio del banco).
    pub(crate) fn de_raiz(raiz: &Path) -> Politica {
        std::fs::read_to_string(raiz.join(RUTA))
            .map(|t| Politica::de_texto(&t))
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protegida_solo_si_lo_dice() {
        assert!(Politica::de_texto("main:\n  protegida: true\n").protegida);
        assert!(!Politica::de_texto("main:\n  protegida: false\n").protegida);
        assert!(!Politica::de_texto("main: {}\n").protegida);
        assert!(!Politica::de_texto("").protegida);
        assert!(!Politica::de_texto("otra:\n  protegida: true\n").protegida);
    }

    #[test]
    fn un_fichero_roto_no_encierra() {
        assert!(!Politica::de_texto("main: [protegida: true\n").protegida);
    }

    #[test]
    fn sin_fichero_es_libre() {
        let d = std::env::temp_dir().join(format!("politica-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join(".arbol")).unwrap();
        assert_eq!(Politica::de_raiz(&d), Politica::default());
        std::fs::write(d.join(RUTA), "main:\n  protegida: true\n").unwrap();
        assert!(Politica::de_raiz(&d).protegida);
        let _ = std::fs::remove_dir_all(&d);
    }
}
