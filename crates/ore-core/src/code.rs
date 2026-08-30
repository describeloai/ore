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
    /// Gobierno — reglas que apuntan por clasificación. **Borrador de v1alpha3.**
    ///
    /// `Flow` gobierna lo que se puede saber y `Effect` lo que se puede causar;
    /// esta gobierna **qué debe sostenerse y quién responde**. Su código
    /// central, `8001`, es el `4001` de este plano: el defecto **no está
    /// escrito en ninguna parte** — es la ausencia de una línea que nadie
    /// escribió, así que no hay diff donde mirarlo.
    Governance,
    /// Significado — qué es la misma cosa. **Borrador de v1alpha4.**
    ///
    /// La fila que faltaba debajo de todas las demás: `Flow` compara etiquetas
    /// y `Governance` exige que estén cubiertas, y **ninguna comprueba que la
    /// clasificación sea consistente**, porque hasta v1alpha4 no había forma de
    /// decir que dos propiedades son la misma. Gobierna lo que alguien acertó a
    /// etiquetar.
    Meaning,
    /// Efectos e integridad — el dual de `Flow`. **Borrador de v1alpha2.**
    ///
    /// `Flow` gobierna lo que se puede saber; esta familia, lo que se puede
    /// causar. La simetría de los códigos es deliberada: `7001` frente a
    /// `4001`, `7005` frente a `4011`, `7006` frente a `4008`.
    Effect,
}

codes! {
    // ── OOS1xxx · sintaxis y esquema ────────────────────────────────────────
    Oos1001 = "OOS1001", Syntax, "YAML mal formado";
    Oos1002 = "OOS1002", Syntax, "apiVersion ausente o no soportada";
    Oos1003 = "OOS1003", Syntax, "kind desconocido";
    Oos1004 = "OOS1004", Syntax, "el documento no valida contra su esquema JSON";
    Oos1005 = "OOS1005", Syntax, "clave desconocida sin prefijo de extensión x-";

    // ── OOS2xxx · referencias e integridad ──────────────────────────────────
    //
    // `OOS2001` lo RESERVÓ v1alpha1 sin poder alcanzarlo —toda referencia de
    // aquella versión tenía código propio— dejando escrito que lo activarían
    // los tipos de referencia nuevos de `Function`, `Resolution` y `Test`. El
    // primero que llega es el `call` de un deber, en v1alpha3. Por eso no
    // cuenta entre los 52 de v1alpha1 y su caso vive en el otro árbol.
    Oos2001 = "OOS2001", Reference, "referencia a un nombre cualificado inexistente";
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
    // Lo introduce v1alpha2 al promover `expression` de prosa a CEL, y vive en
    // esta familia porque lo que está en juego es la solidez de la propagación.
    // No cuenta entre los 52 de v1alpha1 — como `OOS2001`.
    Oos4015 = "OOS4015", Flow, "la expresión lee una propiedad que derivedFrom no declara";

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
    // Los introduce v1alpha3. Viven en esta familia porque lo que rompen es la
    // compatibilidad de la GOBERNANZA, que es lo que el eje POLICY mide, y no
    // cuentan entre los 52 de v1alpha1 — como `OOS2001` y `OOS4015`.
    //
    // Son dos y no cinco a propósito. `OOS5023` compara **lo que cada propiedad
    // tiene cubierto**, no la sintaxis de las reglas: quitar un `Ruleset`,
    // estrechar un objetivo, borrar una aserción, rebajarla a `warning`, quitar
    // una máscara o cambiar una etiqueta de forma que la selección encoja son
    // seis cambios distintos y **un solo síntoma** — esta propiedad ha perdido
    // esta clase de gobierno. Un código por síntoma, no por causa.
    Oos5023 = "OOS5023", Compatibility, "una propiedad pierde una clase de gobierno que tenía";
    Oos5024 = "OOS5024", Compatibility, "la clasificación exige menos gobierno que antes";

    // ── OOS6xxx · forma canónica ────────────────────────────────────────────
    Oos6003 = "OOS6003", Canonical, "pérdida de precisión: decimal sin representación en cadena";

    // ── OOS7xxx · efectos e integridad ──────────────────────────────────────
    //
    // Borrador de v1alpha2 (`spec/v1alpha2/01-efectos.md` §5). Registrados aquí
    // antes de tener implementación porque el registro es lo que impide que dos
    // familias se pisen un número, y porque la simetría con OOS4xxx solo se ve
    // mirándolas juntas.
    Oos7001 = "OOS7001", Effect, "violación de la regla de integridad por propagación";
    Oos7002 = "OOS7002", Effect, "la función no alcanza la integridad que exige su destino";
    Oos7003 = "OOS7003", Effect, "etiqueta de integridad fuera de todo retículo de eje integrity";
    Oos7004 = "OOS7004", Effect, "endosante fuera del vocabulario cerrado";
    Oos7005 = "OOS7005", Effect, "destino de un efecto sin integridad declarada";
    Oos7006 = "OOS7006", Effect, "efecto sobre una propiedad derivedFrom";
    Oos7007 = "OOS7007", Effect, "join declarado incoherente con el axis del retículo";
    Oos7008 = "OOS7008", Effect, "efectos de una función sobre más de una fuente física";
    // OOS7010 · RETIRADO al escribir el esquema de `Resolution`. Existía para
    // `confidence` en una estrategia determinista; el campo resultó no
    // significar nada en ninguna estrategia —lo que decía lo dice el eje de
    // integridad— así que desapareció del vocabulario y el fallo pasó a ser una
    // clave desconocida, que ya tiene código.
    Oos7009 = "OOS7009", Effect, "estrategia probabilística sin conducto declarado";
    Oos7011 = "OOS7011", Effect, "integridad por encima del techo de la estrategia";

    // ── OOS8xxx · gobierno ──────────────────────────────────────────────────
    //
    // Borrador de v1alpha3 (`spec/v1alpha3/02-ruleset.md` §8). **Cinco códigos
    // y cuatro familias reutilizadas** —`OOS1004`, `OOS2001`, `OOS4003` y
    // `OOS7001`—, y esa proporción es la señal de que la partición «¿hay
    // sujeto?» estaba bien hecha: casi nada hizo falta inventarlo.
    Oos8001 = "OOS8001", Governance, "propiedad que exige gobierno sin ninguna regla que la cubra";
    Oos8002 = "OOS8002", Governance, "objetivo que no casa con ninguna propiedad";
    Oos8003 = "OOS8003", Governance, "máscara que no baja demostrablemente la etiqueta del objetivo";
    // OOS8004 · RETIRADO al escribir los casos, antes de implementarse. Existía
    // para un deber que no resuelve a una `Function`; `OOS2001` lleva reservado
    // desde v1alpha1 para exactamente eso. Activar una reserva es mejor que
    // inflar una familia.
    Oos8005 = "OOS8005", Governance, "aserción sql cuyo objetivo abarca más de una fuente física";
    Oos8006 = "OOS8006", Governance, "objetivo sobre un retículo de eje integrity";

    // ── OOS9xxx · significado ───────────────────────────────────────────────
    //
    // Borrador de v1alpha4 (`spec/v1alpha4/01-significado.md` §7). El alcance
    // anunció **cuatro** códigos nuevos y previó que el registro se moviera al
    // escribir los esquemas; se movió, y en la dirección esperable: son **tres**.
    //
    // `OOS9002` —una propiedad con `is` que redeclara `type` o `labels`—
    // RETIRADO al escribir el esquema, y no por descuido: la exclusión es
    // expresable ENTERA con un `oneOf`, luego su incumplimiento ya tiene código
    // y es `OOS1004`. Es exactamente el trato que §7 le daba una fila más
    // arriba a `confidence` sin `is`, y no verlo habría sido inflar la familia
    // por simetría con una tabla.
    Oos9001 = "OOS9001", Meaning, "entidad que declara implementar una forma y no la satisface";
    Oos9003 = "OOS9003", Meaning, "confidence en un documento cuya madurez efectiva no es DRAFT";
    Oos9004 = "OOS9004", Meaning, "concepto declarado localmente al que nada referencia";
}

#[cfg(test)]
mod tests {
    use super::*;

    /// La cobertura declarada en `99-errors.md` §10 —52 códigos de v1alpha1—
    /// y los borradores, contados aparte para que ese 52 siga significando lo
    /// mismo: *una implementación de referencia pasa la especificación
    /// completa*. Un número que mezclara una versión cerrada con dos en curso
    /// ya no se sabría qué mide.
    ///
    /// `OOS2001` queda fuera de los 52 aunque sea de familia `Reference`: lo
    /// reservó v1alpha1 sin poder alcanzarlo y lo activa v1alpha3.
    #[test]
    fn el_registro_separa_v1alpha1_de_los_borradores() {
        // Códigos que viven en una familia de v1alpha1 pero los introduce una
        // versión posterior. La familia dice de qué habla el código; no dice
        // cuándo llegó.
        const POSTERIORES: &[Code] = &[Code::Oos2001, Code::Oos4015, Code::Oos5023, Code::Oos5024];
        let cerrados = Code::ALL
            .iter()
            .filter(|c| {
                !matches!(
                    c.family(),
                    Family::Effect | Family::Governance | Family::Meaning
                )
            })
            .filter(|c| !POSTERIORES.contains(c))
            .count();
        assert_eq!(cerrados, 52, "v1alpha1");
        assert_eq!(
            Code::ALL
                .iter()
                .filter(|c| c.family() == Family::Effect)
                .count(),
            10,
            "borrador de efectos"
        );
        assert_eq!(
            Code::ALL
                .iter()
                .filter(|c| c.family() == Family::Governance)
                .count(),
            5,
            "borrador de gobierno"
        );
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
