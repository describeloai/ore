//! **El retículo de etiquetas** (CI C4: vocabulario): un eje (confidencialidad o
//! integridad), sus niveles en orden y lo que exige cada nivel, y `oos.maturity`. El flujo
//! de etiquetas por el árbol (la familia `OOS4xxx`) vive en `ore_core::flow`, que re-exporta
//! esto.

use std::collections::BTreeMap;

// ── Retículos ───────────────────────────────────────────────────────────────

/// El eje de un retículo. Decide qué se compara y con qué combinador.
///
/// Confidencialidad pregunta *cuánto daño si esto se filtra*; integridad,
/// *cuánto daño si esto es falso*. Son ortogonales: un dato puede ser público y
/// crítico a la vez —el estado de un pedido no es secreto y escribirlo mal
/// cuesta dinero— y por eso hacen falta los dos.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Axis {
    /// Gobierna lecturas. Combina por `join` = máximo.
    #[default]
    Confidentiality,
    /// Gobierna escrituras. Combina por `meet` = mínimo.
    Integrity,
}

impl Axis {
    /// El combinador que el eje implica. **No se declara** — es derivable, y un
    /// campo derivable no es declarable (P2).
    pub const fn combinador(self) -> &'static str {
        match self {
            Axis::Confidentiality => "max",
            Axis::Integrity => "min",
        }
    }
}

/// Un retículo: etiquetas y su orden parcial. El orden **es** la secuencia.
#[derive(Debug, Clone)]
pub struct Lattice {
    pub qname: String,
    pub levels: Vec<String>,
    pub axis: Axis,
    /// Qué exige la clasificación, por nivel: `nivel → naturalezas`.
    ///
    /// Vive en el retículo y no en la regla porque es lo que hace que
    /// **importar la clasificación importe su exigencia**. Y nombra **clases**
    /// de regla, no solo un nivel: sin eso una comprobación de nulos
    /// descargaría lo que un paquete de protección de datos pedía como
    /// política, que es el error de categoría más frecuente
    /// (`v1alpha3/01-gobierno` §6.1).
    pub requires_governance: BTreeMap<String, Vec<String>>,
}

impl Lattice {
    pub fn index(&self, level: &str) -> Option<usize> {
        self.levels.iter().position(|l| l == level)
    }

    /// ¿Está `nivel` en `piso` o por encima?
    ///
    /// `None` si alguno de los dos no pertenece al retículo — que no es lo
    /// mismo que `false`, y confundirlos convertiría una etiqueta mal escrita
    /// en una propiedad que parece no seleccionada.
    pub fn ge(&self, nivel: &str, piso: &str) -> Option<bool> {
        Some(self.index(nivel)? >= self.index(piso)?)
    }
}

/// `oos.maturity` es estándar de la especificación y está siempre activo, lo
/// declare el paquete o no.
///
/// El orden es ASCENDENTE POR RESTRICTIVIDAD, igual que todo retículo, y por
/// eso `STABLE` es el fondo: es lo que puede servirse a cualquier consumidor.
/// Tres partes normativas lo fijan en esa dirección y no en la contraria:
///
/// - `ore promote` es un **desclasificador** y BAJA `DRAFT` a `REVIEWED` a
///   `STABLE` (`04-flow.md` §3, §5). Desclasificar es bajar; luego
///   `STABLE ⊑ REVIEWED ⊑ DRAFT`.
/// - La suite —normativa— razona en `diff/downgrade-maturity` que `DRAFT` es
///   **invisible para los consumidores de producción**. Un `contextSurface`
///   que admite `STABLE` y rechaza `DRAFT` solo es expresable con este orden.
/// - Las autorizaciones de ejemplo de `04-flow.md` §4 solo son coherentes así:
///   `cache: STABLE` admite únicamente lo estable, y `log: DEPRECATED` —el
///   techo— lo admite todo. Con el orden inverso, `cache` aceptaría un
///   borrador y `contextSurface` rechazaría lo estable.
pub fn maturity() -> Lattice {
    Lattice {
        qname: "oos.maturity".into(),
        levels: ["STABLE", "REVIEWED", "DRAFT", "DEPRECATED"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        axis: Axis::Confidentiality,
        // `oos.maturity` no exige gobierno: es el ciclo de vida de un
        // documento, no una clasificación de sensibilidad. Obligar a cubrir
        // todo lo que no sea STABLE convertiría un borrador en un error.
        requires_governance: BTreeMap::new(),
    }
}
