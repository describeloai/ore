//! Registro de códigos de error de OOS.
//!
//! Transcripción de `vendor/oos/spec/v1alpha1/99-errors.md`, que es la fuente
//! autoritativa. Un código presente aquí y ausente allí es un defecto de este
//! fichero.
//!
//! Los códigos importan más de lo que su apariencia sugiere. La suite de
//! conformidad afirma **qué** error se produce, no solo que algo falla: sin
//! códigos estables, dos implementaciones «conformes» podrían rechazar cosas
//! distintas y nadie lo detectaría.

macro_rules! codes {
    ($( $variant:ident = $lit:literal, $family:ident, $desc:literal ; )*) => {
        /// Un código de error de OOS.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[non_exhaustive]
        pub enum Code { $( $variant, )* }

        impl Code {
            /// La forma textual: `OOS4001`.
            pub const fn as_str(self) -> &'static str {
                match self { $( Self::$variant => $lit, )* }
            }

            /// Qué clase de fallo describe.
            pub const fn family(self) -> Family {
                match self { $( Self::$variant => Family::$family, )* }
            }

            /// Descripción normativa, en una línea.
            pub const fn description(self) -> &'static str {
                match self { $( Self::$variant => $desc, )* }
            }

            /// Todos los códigos activos.
            pub const ALL: &'static [Code] = &[ $( Self::$variant, )* ];

            /// Resuelve desde su forma textual. Lo usa el runner de conformidad
            /// para leer `expects:` de un caso.
            pub fn parse(s: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|c| c.as_str() == s)
            }
        }

        impl core::fmt::Display for Code {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

/// Familia de un código. Determina en qué fase se detecta y en qué directorio
/// de la suite vive su caso.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Family {
    /// Sintaxis y esquema. Fase de análisis.
    Syntax,
    /// Referencias e integridad. Fase de enlazado.
    Reference,
    /// Sistema de tipos.
    Type,
    /// Gobernanza y flujo de información. **La familia que define el producto.**
    Flow,
    /// Compatibilidad y cambios rompedores. Se verifica en `diff/`.
    Compatibility,
    /// Forma canónica.
    Canonical,
}

codes! {
    // ── OOS1xxx · sintaxis y esquema ────────────────────────────────────────
    Oos1001 = "OOS1001", Syntax, "YAML mal formado";
    Oos1002 = "OOS1002", Syntax, "apiVersion ausente o no soportada";
    Oos1003 = "OOS1003", Syntax, "kind desconocido";
    Oos1004 = "OOS1004", Syntax, "el documento no valida contra su esquema JSON";
    Oos1005 = "OOS1005", Syntax, "clave desconocida sin prefijo de extensión x-";

    // ── OOS2xxx · referencias e integridad ──────────────────────────────────
    Oos2002 = "OOS2002", Reference, "ciclo en el grafo de dependencias";
    Oos2003 = "OOS2003", Reference, "duplicado en un campo declarado como conjunto";
    Oos2004 = "OOS2004", Reference, "datasourceRef no declarado en el manifiesto raíz";
    Oos2005 = "OOS2005", Reference, "referencia a una entidad o propiedad inexistente";
    Oos2006 = "OOS2006", Reference, "uso de un nombre declarado en reserved";
    Oos2007 = "OOS2007", Reference, "version no es semver 2.0.0 válido";
    Oos2008 = "OOS2008", Reference, "status fuera del vocabulario de ODCS";
    Oos2009 = "OOS2009", Reference, "owner ausente o mal formado";
    Oos2010 = "OOS2010", Reference, "nature entity sin primaryKey, o event sin timeKey";
    Oos2011 = "OOS2011", Reference, "el mapeo no cubre la primaryKey de la entidad destino";
    Oos2012 = "OOS2012", Reference, "secreto de conexión presente en un documento";
    Oos2013 = "OOS2013", Reference, "artefacto generado desincronizado con su fuente";

    // ── OOS3xxx · sistema de tipos ──────────────────────────────────────────
    Oos3001 = "OOS3001", Type, "tipo fuera del conjunto";
    Oos3002 = "OOS3002", Type, "Money o Quantity sin unidad o sin precisión";
    Oos3003 = "OOS3003", Type, "temporal declarado sin validTime";
    Oos3004 = "OOS3004", Type, "incompatibilidad de unidades en una derivación";
    Oos3005 = "OOS3005", Type, "cardinalidad incoherente con las claves declaradas";

    // ── OOS4xxx · gobernanza y flujo ────────────────────────────────────────
    Oos4001 = "OOS4001", Flow, "violación de la regla de flujo por propagación";
    Oos4002 = "OOS4002", Flow, "etiqueta por encima de la autorización del conducto";
    Oos4003 = "OOS4003", Flow, "etiqueta que no pertenece a ningún retículo";
    Oos4006 = "OOS4006", Flow, "desclasificador fuera del conjunto cerrado";
    Oos4007 = "OOS4007", Flow, "aggregate sin minGroupSize o por debajo del umbral";
    Oos4008 = "OOS4008", Flow, "propiedad derivada que declara etiqueta en vez de computarla";
    Oos4011 = "OOS4011", Flow, "conducto sin autorización declarada";
    Oos4012 = "OOS4012", Flow, "propiedad que rebaja la etiqueta heredada de su entidad";
    Oos4014 = "OOS4014", Flow, "examples no sintéticos en propiedad etiquetada";

    // ── OOS5xxx · compatibilidad ────────────────────────────────────────────
    Oos5001 = "OOS5001", Compatibility, "propiedad eliminada sin moved ni reserved";
    Oos5002 = "OOS5002", Compatibility, "tipo estrechado o valor retirado de un enum";
    Oos5003 = "OOS5003", Compatibility, "cardinalidad endurecida";
    Oos5006 = "OOS5006", Compatibility, "primaryKey cambiada";
    Oos5007 = "OOS5007", Compatibility, "entidad o relación eliminada";
    Oos5008 = "OOS5008", Compatibility, "oos.maturity rebajada en una entidad STABLE";
    Oos5009 = "OOS5009", Compatibility, "etiqueta de una propiedad elevada";
    Oos5010 = "OOS5010", Compatibility, "unidad o precisión de un tipo paramétrico cambiada";
    Oos5011 = "OOS5011", Compatibility, "etiqueta de una propiedad rebajada";
    Oos5012 = "OOS5012", Compatibility, "autorización de un conducto elevada";
    Oos5013 = "OOS5013", Compatibility, "permit que amplía el acceso efectivo";
    Oos5014 = "OOS5014", Compatibility, "forbid eliminado o debilitado";
    Oos5015 = "OOS5015", Compatibility, "conjunto de finalidades ampliado";
    Oos5016 = "OOS5016", Compatibility, "minGroupSize de aggregate reducido";
    Oos5017 = "OOS5017", Compatibility, "desclasificador añadido donde no lo había";
    Oos5018 = "OOS5018", Compatibility, "primaryKey o clave de join materializada cambiada";
    Oos5019 = "OOS5019", Compatibility, "binding físico de una propiedad indexada cambiado";
    Oos5020 = "OOS5020", Compatibility, "modo de materialización cambiado";
    Oos5021 = "OOS5021", Compatibility, "la versión declarada no corresponde a los cambios";
    Oos5022 = "OOS5022", Compatibility, "cambio rompedor sin el periodo de aviso del SLA";

    // ── OOS6xxx · forma canónica ────────────────────────────────────────────
    Oos6003 = "OOS6003", Canonical, "pérdida de precisión: decimal sin representación en cadena";
}

#[cfg(test)]
mod tests {
    use super::*;

    /// La cobertura declarada en `99-errors.md` §10: 52 códigos activos.
    #[test]
    fn el_registro_tiene_52_codigos_activos() {
        assert_eq!(Code::ALL.len(), 52);
    }

    #[test]
    fn cada_codigo_se_resuelve_desde_su_forma_textual() {
        for &c in Code::ALL {
            assert_eq!(Code::parse(c.as_str()), Some(c));
        }
    }

    /// Un código, una vez publicado, no se reutiliza con otro significado
    /// (`99-errors.md` §9). Aquí eso se traduce en: sin duplicados.
    #[test]
    fn no_hay_codigos_repetidos() {
        let mut vistos: Vec<&str> = Code::ALL.iter().map(|c| c.as_str()).collect();
        vistos.sort_unstable();
        let antes = vistos.len();
        vistos.dedup();
        assert_eq!(vistos.len(), antes, "código duplicado en el registro");
    }
}
