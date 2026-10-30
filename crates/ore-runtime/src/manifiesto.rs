//! **Qué runtime es** (0081 §7.4, propiedad 4). El runtime sabe qué es y lo publica: su versión y la de cada
//! pieza. Viaja en el protocolo y queda en cada ejecución.
//!
//! Las versiones que el código no puede preguntar a sus dependencias (arrow-rs, la rev de iceberg-rust) se
//! declaran aquí, y una prueba las compara con `Cargo.lock`: una subida que no las toque rompe la prueba, y
//! el manifiesto no puede mentir.

use std::fmt;

/// La versión de arrow-rs del árbol (0081 S1: una sola).
pub const ARROW: &str = "59.3.2";

/// La rev de iceberg-rust del árbol (0081 S1: `main`, con #3483).
pub const ICEBERG_REV: &str = "4dd8c946432f5e56a3ccc88e5b438a356ba28287";

/// Las versiones de lo que es este runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifiesto {
    /// La del ORE Runtime.
    pub ore_runtime: &'static str,
    /// La de DataFusion, la que dice el propio DataFusion.
    pub datafusion: &'static str,
    /// La de arrow-rs.
    pub arrow: &'static str,
    /// La rev de iceberg-rust.
    pub iceberg: &'static str,
    /// El commit de ORE, si la compilación lo selló (`ORE_COMMIT`, como `ore`). Una compilación local dice
    /// honestamente que no viene de un commit conocido.
    pub commit: Option<&'static str>,
}

impl Manifiesto {
    /// El de este binario.
    pub fn actual() -> Self {
        Manifiesto {
            ore_runtime: env!("CARGO_PKG_VERSION"),
            datafusion: datafusion::DATAFUSION_VERSION,
            arrow: ARROW,
            iceberg: ICEBERG_REV,
            commit: option_env!("ORE_COMMIT"),
        }
    }
}

impl fmt::Display for Manifiesto {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ORE Runtime {} (DataFusion {}, arrow-rs {}, iceberg-rust {}, {})",
            self.ore_runtime,
            self.datafusion,
            self.arrow,
            &self.iceberg[..8],
            self.commit.unwrap_or("sin sellar")
        )
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn lock() -> String {
        std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock"),
        )
        .expect("Cargo.lock")
    }

    /// Las versiones de un paquete en el lock, y su `source`.
    fn del_lock(nombre: &str) -> Vec<(String, Option<String>)> {
        lock()
            .split("[[package]]")
            .skip(1)
            .filter_map(|b| {
                let campo = |k: &str| {
                    b.lines()
                        .find_map(|l| l.trim().strip_prefix(k))
                        .map(|v| v.trim().trim_matches('"').to_string())
                };
                (campo("name = ")? == nombre)
                    .then(|| (campo("version = ").unwrap(), campo("source = ")))
            })
            .collect()
    }

    #[test]
    fn el_manifiesto_dice_lo_que_hay_en_el_lock() {
        let m = Manifiesto::actual();
        let datafusion: Vec<String> = del_lock("datafusion").into_iter().map(|(v, _)| v).collect();
        assert_eq!(datafusion, vec![m.datafusion.to_string()]);
        let arrow = del_lock("arrow-array");
        assert_eq!(
            arrow.len(),
            1,
            "arrow-array con varias versiones: {arrow:?}"
        );
        assert_eq!(arrow[0].0, m.arrow, "sube `manifiesto::ARROW` con arrow-rs");
        let iceberg = del_lock("iceberg");
        assert_eq!(
            iceberg.len(),
            1,
            "iceberg con varias versiones: {iceberg:?}"
        );
        let fuente = iceberg[0].1.as_deref().unwrap_or_default();
        assert!(
            fuente.ends_with(&format!("#{}", m.iceberg)),
            "sube `manifiesto::ICEBERG_REV` con iceberg-rust: el lock dice `{fuente}`"
        );
    }

    #[test]
    fn el_manifiesto_se_lee_en_una_linea() {
        let m = Manifiesto::actual();
        let s = m.to_string();
        assert!(s.starts_with("ORE Runtime "), "{s}");
        assert!(s.contains(&format!("DataFusion {}", m.datafusion)), "{s}");
        assert!(s.contains(&m.iceberg[..8]), "{s}");
    }
}
