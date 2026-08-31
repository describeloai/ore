//! Despacho de documentos y comprobaciones de forma.
//!
//! # Por qué no se usa un validador de JSON Schema
//!
//! Los esquemas publicados en `schemas/v1alpha1/` son **la mitad sintáctica de
//! L0**, y su destinatario son los consumidores externos: editores, linters en
//! otros lenguajes, acciones de CI. Ese es su trabajo y lo hacen bien.
//!
//! Para ORE serían maquinaria desproporcionada — los validadores 2020-12
//! disponibles pesan entre 73 y 192 crates, arrastran FFI de plataforma y
//! producen mensajes como `/spec/primaryKey: minItems`. La tesis del proyecto es
//! que **el error es el producto**, y ese es exactamente el error que criticamos.
//!
//! Aquí las siete formas se comprueban de forma nativa y tipada, con posición y
//! con un mensaje que dice qué hacer. La deriva contra los esquemas la atrapa la
//! suite de conformidad, que es el árbitro de ambos.
//!
//! Registro: `docs/decisions/0002-sin-validador-de-json-schema.md`

use crate::parse::Node;

/// Qué falla una regla de forma: el mensaje y, si la hay, qué hacer al respecto.
pub type ShapeFailure = (String, Option<String>);

pub const API_VERSION: &str = "oos.dev/v1alpha1";

/// Las versiones de la especificación que esta implementación entiende.
///
/// Deja de ser una constante en el momento en que hay dos, y el cambio no es
/// cosmético: **el documento elige su conjunto de reglas**. Un `kind` que no
/// existía en v1alpha1 no es un error de escritura, es un documento de otra
/// versión, y decirlo así es la diferencia entre «no reconozco esto» y «esto
/// existe, en otra versión».
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ApiVersion {
    V1Alpha1,
    V1Alpha2,
    V1Alpha3,
    V1Alpha4,
}

impl ApiVersion {
    pub const ALL: &'static [ApiVersion] = &[
        ApiVersion::V1Alpha1,
        ApiVersion::V1Alpha2,
        ApiVersion::V1Alpha3,
        ApiVersion::V1Alpha4,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            ApiVersion::V1Alpha1 => "oos.dev/v1alpha1",
            ApiVersion::V1Alpha2 => "oos.dev/v1alpha2",
            ApiVersion::V1Alpha3 => "oos.dev/v1alpha3",
            ApiVersion::V1Alpha4 => "oos.dev/v1alpha4",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|v| v.as_str() == s)
    }
}

/// Los cinco documentos de v1alpha1, más el manifiesto raíz.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    OntologyConfig,
    Package,
    Entity,
    Binding,
    Lattice,
    ConduitPolicy,
    /// v1alpha2. La superficie de efecto.
    Function,
    /// v1alpha2. El efecto sobre la identidad.
    Resolution,
    /// v1alpha3. La regla que apunta.
    Ruleset,
    /// v1alpha4. El concepto: qué ES un dato.
    Property,
    /// v1alpha4. La forma: un conjunto de entidades nombrado por lo que tienen.
    Interface,
}

impl Kind {
    pub const ALL: &'static [Kind] = &[
        Kind::OntologyConfig,
        Kind::Package,
        Kind::Entity,
        Kind::Binding,
        Kind::Lattice,
        Kind::ConduitPolicy,
        Kind::Function,
        Kind::Resolution,
        Kind::Ruleset,
        Kind::Property,
        Kind::Interface,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Kind::OntologyConfig => "OntologyConfig",
            Kind::Package => "Package",
            Kind::Entity => "Entity",
            Kind::Binding => "Binding",
            Kind::Lattice => "Lattice",
            Kind::ConduitPolicy => "ConduitPolicy",
            Kind::Function => "Function",
            Kind::Resolution => "Resolution",
            Kind::Ruleset => "Ruleset",
            Kind::Property => "Property",
            Kind::Interface => "Interface",
        }
    }

    /// La versión en la que este documento aparece.
    ///
    /// Un `Function` en un paquete de v1alpha1 no es un `kind` desconocido: es
    /// un documento del futuro, y el error tiene que decir eso.
    pub const fn since(self) -> ApiVersion {
        match self {
            Kind::Function | Kind::Resolution => ApiVersion::V1Alpha2,
            Kind::Ruleset => ApiVersion::V1Alpha3,
            Kind::Property | Kind::Interface => ApiVersion::V1Alpha4,
            _ => ApiVersion::V1Alpha1,
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|k| k.as_str() == s)
    }

    /// Claves admitidas en la raíz del documento.
    pub const fn root_keys(self) -> &'static [&'static str] {
        &["apiVersion", "kind", "metadata", "spec"]
    }

    /// Claves admitidas bajo `metadata`.
    pub const fn metadata_keys(self) -> &'static [&'static str] {
        match self {
            Kind::OntologyConfig => &["name", "version", "description"],
            Kind::Package => &[
                "name",
                "version",
                "status",
                "domain",
                "id",
                "tenant",
                "tags",
                "description",
            ],
            Kind::Entity => &["name", "namespace", "labels", "description", "aiContext"],
            // `Function` no admite `labels`, y la ausencia es normativa: su
            // integridad SE COMPUTA de sus endosos (`02-function` §6). Admitir
            // una etiqueta dejaría que una función escribiera `attested` sobre
            // sí misma sin que exista atestación, y una afirmación sobre uno
            // mismo no es una garantía. Al no existir el campo, el error es
            // estructural — `OOS1005` — en vez de necesitar un código propio.
            // `Resolution` tampoco admite `labels`, y por lo mismo: la
            // integridad que puede producir se deriva de sus estrategias.
            // `Ruleset` tampoco, y por una razón distinta: **no porta datos**,
            // luego no tiene clasificación. Un campo del que nada se computa
            // acaba adquiriendo un significado que nadie escribió.
            Kind::Binding
            | Kind::Lattice
            | Kind::ConduitPolicy
            | Kind::Function
            | Kind::Resolution
            | Kind::Ruleset
            | Kind::Interface => &["name", "namespace", "description"],
            // `Property` es el único documento con `labels` en LOS DOS SITIOS,
            // y la primera versión de este `match` se lo negó por miedo a la
            // duplicación. Era un error, y lo destapó `confidence`: un concepto
            // acuñado por inferencia tiene que poder declararse `DRAFT`, y la
            // madurez de un documento se declara donde se declara siempre.
            //
            // No son dos superficies para lo mismo porque **el sujeto es
            // distinto**, y así hay que leerlas:
            //
            // - `metadata.labels` clasifica ESTE DOCUMENTO — su madurez.
            // - `spec.labels` clasifica EL DATO que lleve este concepto, y es
            //   lo que hereda una propiedad que declare `is`.
            //
            // Es la misma distinción que en `Entity`, que lleva `labels` en
            // `metadata` y otras dentro de cada propiedad sin que nadie las
            // confunda.
            Kind::Property => &["name", "namespace", "labels", "description"],
        }
    }

    /// Claves admitidas bajo `spec`.
    pub const fn spec_keys(self) -> &'static [&'static str] {
        match self {
            Kind::OntologyConfig => &["workspace", "dependencies", "datasources"],
            Kind::Package => &[
                "owner",
                "team",
                "roles",
                "support",
                "sla",
                "authoritativeDefinitions",
                "dependencies",
            ],
            Kind::Entity => &[
                "nature",
                "primaryKey",
                "timeKey",
                "uniqueKeys",
                "temporal",
                "properties",
                "relations",
                "moved",
                "reserved",
                // v1alpha4. La forma que la entidad declara satisfacer.
                "implements",
            ],
            Kind::Binding => &[
                "targetEntity",
                "datasourceRef",
                "profile",
                "source",
                "selector",
                "properties",
                "capabilities",
                "materialization",
            ],
            // `axis` es lo único que v1alpha2 añade al retículo, y de él sale
            // el combinador. `join` queda obsoleto: derivable, luego no
            // declarable (P2), y si aparece debe coincidir — `OOS7007`.
            // `requiresGovernance` es lo que v1alpha3 añade, y va aquí y no en
            // el `Ruleset` a propósito: importar el paquete de clasificación
            // importa **su exigencia**, y eso es lo que hace que «GDPR como
            // dependencia» deje de ser una metáfora.
            Kind::Lattice => &[
                "levels",
                "levelDescriptions",
                "join",
                "axis",
                "requiresGovernance",
            ],
            Kind::ConduitPolicy => &["conduits"],
            Kind::Function => &[
                "runtime",
                "entrypoint",
                "source",
                "limits",
                "input",
                "output",
                "preconditions",
                "effects",
                "endorsements",
                "authorization",
                "idempotency",
            ],
            Kind::Resolution => &["entity", "sources", "strategies", "endorsements"],
            Kind::Ruleset => &["owner", "targets", "assertions", "masks", "duties"],
            // La línea que decide qué cabe en un concepto: **declara lo que es
            // cierto de él en todas partes**. `required`, `unique` y `temporal`
            // no están porque dependen de la tabla, no del significado; `enum`
            // y `aiContext` sí, porque un código de moneda es ISO 4217 en los
            // quince sistemas y los sinónimos de un concepto son los mismos en
            // todos. `derivedFrom` y `expression` tampoco: un correo personal
            // significa lo mismo se calcule como se calcule.
            Kind::Property => &[
                "type",
                "labels",
                "description",
                "enum",
                "aiContext",
                // v1alpha4 · la exigencia CATEGÓRICA. Un retículo exige por
                // nivel; un concepto, por ser lo que es. Mismo nombre que en el
                // retículo a propósito: es la misma noción sobre otro sujeto, y
                // otro nombre habría sido el error de los dos nombres para un
                // concepto.
                "requiresGovernance",
                "confidence",
            ],
            Kind::Interface => &["requires", "description"],
        }
    }

    /// Claves admitidas dentro de una propiedad de `Entity`.
    ///
    /// Se comprueban por lo mismo que las de `spec`: con `additionalProperties:
    /// true` una errata como `qualtiy:` se aceptaría en silencio y la propiedad
    /// quedaría sin gobernar. Aquí eso no es una molestia — es un hueco de
    /// gobierno que no produce ningún síntoma.
    ///
    /// `expression` no es nueva de v1alpha2: existe desde v1alpha1 como prosa
    /// documental. Lo que v1alpha2 cambia es su ESTATUTO —pasa a ser CEL y a
    /// comprobarse—, no su nombre. Un `expr` al lado habrían sido dos nombres
    /// para un concepto.
    ///
    /// Y `quality` NO está: el cuerpo de una aserción es `quality` de ODCS y su
    /// destino de emisión también, pero escribirla aquí sería una segunda
    /// superficie de autoría **sin dueño propio**. Vive en un `Ruleset`, que
    /// admite objetivos por nombre además de por predicado.
    pub const fn property_keys(self) -> &'static [&'static str] {
        &[
            "type",
            "labels",
            "description",
            "required",
            "unique",
            "temporal",
            "enum",
            "derivedFrom",
            "expression",
            "examples",
            "aiContext",
            // v1alpha4. El nombre es mío, el significado es del concepto.
            "is",
            "confidence",
        ]
    }

    /// `OntologyConfig` no lleva `spec`: sus secciones cuelgan de la raíz.
    pub const fn sections_at_root(self) -> bool {
        matches!(self, Kind::OntologyConfig)
    }
}

/// Una clave desconocida es un error salvo que declare ser una extensión de
/// proveedor: `x-<proveedor>-<lo que sea>`.
///
/// La estrictez es deliberada: con `additionalProperties: true` una errata como
/// `propertis:` se aceptaría en silencio y el campo real quedaría sin declarar.
pub fn is_extension(key: &str) -> bool {
    let Some(rest) = key.strip_prefix("x-") else {
        return false;
    };
    let Some((vendor, _)) = rest.split_once('-') else {
        return false;
    };
    !vendor.is_empty()
        && vendor
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

/// Una regla de forma que no se puede expresar como «esta clave existe».
pub struct ShapeRule {
    pub kind: Kind,
    /// Ruta desde la raíz, p. ej. `["spec", "primaryKey"]`.
    pub path: &'static [&'static str],
    pub check: fn(&Node) -> Option<ShapeFailure>,
}

fn no_vacio(nombre: &'static str, ayuda: &'static str) -> impl Fn(&Node) -> Option<ShapeFailure> {
    move |n: &Node| {
        matches!(n, Node::Sequence { items, .. } if items.is_empty())
            .then(|| (format!("`{nombre}` está vacío"), Some(ayuda.to_string())))
    }
}

/// Reglas de forma comprobadas hoy. Crece con las fases; la suite de
/// conformidad dice cuáles faltan.
const AYUDA_NATURALEZA: &str = "el vocabulario es cerrado: `constraint`, `authorization`, \
     `obligation` y `transformation`. `derivation` no está, y no por olvido — produce \
     contenido, no lo gobierna. Y una naturaleza que no se reconoce no exige nada EN \
     SILENCIO, que es el peor de los dos fallos posibles";

fn naturaleza_desconocida(n: &crate::parse::Node) -> Option<String> {
    n.items()
        .iter()
        .filter_map(|i| i.as_str())
        .find(|s| !crate::governance::NATURALEZAS.contains(s))
        .map(String::from)
}

pub fn shape_rules() -> Vec<ShapeRule> {
    vec![
        ShapeRule {
            kind: Kind::Entity,
            path: &["spec", "primaryKey"],
            check: |n| {
                no_vacio(
                    "primaryKey",
                    "una clave vacía no identifica nada; declara al menos una propiedad, \
                     o usa `nature: event` con `timeKey` si los registros no tienen \
                     identidad estable",
                )(n)
            },
        },
        // v1alpha4 · el guardarraíl, y va aquí y no en una familia nueva
        // porque **el esquema lo expresa entero**: es un `oneOf` entre declarar
        // localmente y referenciar un concepto. El borrador de v1alpha4 reservó
        // `OOS9002` para esto y sobra — un incumplimiento de forma ya tiene
        // código, y es este. Inflar una familia por simetría con una tabla es
        // lo contrario de lo que P7 pide.
        ShapeRule {
            kind: Kind::Entity,
            path: &["spec", "properties"],
            check: |n| {
                for (k, v) in n.entries() {
                    let Some(nombre) = k.as_str() else { continue };
                    let referencia = v.get("is").is_some();
                    // El guardarraíl alcanza a `type` y **no** a `labels**, y
                    // la asimetría no es un descuido: la primera versión
                    // prohibió las dos y se contradecía con `OOS4012`, que
                    // permite elevar la clasificación heredada. Elevarla exige
                    // escribirla, luego prohibir `labels` dejaba la elevación
                    // sin sintaxis.
                    //
                    // Los dos campos no son la misma clase de cosa:
                    //
                    // - `type` es una IGUALDAD. Redeclararlo solo puede coincidir
                    //   o contradecir, y en el segundo caso no hay nada a lo que
                    //   apelar para decidir quién gana. Se prohíbe.
                    // - `labels` es un ORDEN. Redeclararlas tiene un significado
                    //   definido —elevar— y un error definido —rebajar—, y la
                    //   regla que los separa existe desde v1alpha1.
                    //
                    // Por eso `OOS4012` «sube un nivel sin cambiar una letra»:
                    // porque aquí no hace falta ninguna.
                    if referencia && v.get("type").is_some() {
                        return Some((
                            format!(
                                "`{nombre}` declara `is` y también `type`: una propiedad declara                                  localmente o referencia un concepto, nunca las dos"
                            ),
                            Some(
                                "el tipo lo pone el concepto, y no hay orden al que apelar si la                                  copia deja de coincidir. La clasificación es otra cosa: esa se                                  puede escribir para ELEVARLA —`OOS4012`— y nunca para rebajarla"
                                    .into(),
                            ),
                        ));
                    }
                    if !referencia && v.get("confidence").is_some() {
                        return Some((
                            format!("`{nombre}` declara `confidence` sin `is`"),
                            Some(
                                "`confidence` es la confianza de UNA INFERENCIA, y sin mapeo no                                  hay nada de lo que dudar. Una propiedad escrita a mano no es una                                  inferencia: es una decisión, y nadie declara cuánta confianza                                  tiene en algo que acaba de decidir"
                                    .into(),
                            ),
                        ));
                    }
                }
                None
            },
        },
        // El vocabulario de naturalezas **no lo validaba nadie**, y es un
        // agujero que v1alpha3 ya tenía: `cobertura` filtra por la lista
        // cerrada, así que un `autorization` mal escrito no exigía nada **en
        // silencio**. Es `OOS8002` un piso más arriba, otra vez — una exigencia
        // que no exige nada tiene el mismo aspecto que una que sí.
        //
        // Se arregla en los dos sitios a la vez, que es lo que obliga a añadir
        // el segundo: si solo se validara el nuevo, el viejo quedaría peor por
        // comparación.
        ShapeRule {
            kind: Kind::Lattice,
            path: &["spec", "requiresGovernance"],
            check: |n| {
                for (nivel, v) in n.entries() {
                    let nivel = nivel.as_str().unwrap_or("?");
                    if let Some(mala) = naturaleza_desconocida(v) {
                        return Some((
                            format!("`{mala}` no es una naturaleza de regla, en `{nivel}`"),
                            Some(AYUDA_NATURALEZA.into()),
                        ));
                    }
                }
                None
            },
        },
        ShapeRule {
            kind: Kind::Property,
            path: &["spec", "requiresGovernance"],
            check: |n| {
                if n.items().is_empty() {
                    return Some((
                        "`requiresGovernance` vacío".into(),
                        Some("omite el campo en lugar de exigir la nada".into()),
                    ));
                }
                naturaleza_desconocida(n).map(|mala| {
                    (
                        format!("`{mala}` no es una naturaleza de regla"),
                        Some(AYUDA_NATURALEZA.into()),
                    )
                })
            },
        },
        ShapeRule {
            kind: Kind::Property,
            path: &["spec"],
            check: |n| {
                n.get("type").is_none().then(|| {
                    (
                        "un `Property` DEBE declarar `type`".into(),
                        Some(
                            "es la mitad de lo que el concepto declara —la otra es `labels`— y es                              lo que hereda toda propiedad que lo referencie"
                                .into(),
                        ),
                    )
                })
            },
        },
        ShapeRule {
            kind: Kind::Interface,
            path: &["spec"],
            check: |n| {
                n.get("requires").is_none().then(|| {
                    (
                        "un `Interface` DEBE declarar `requires`".into(),
                        Some(
                            "una forma sin exigencias la satisface cualquier cosa, y entonces no                              nombra ningún conjunto. Es `OOS8002` visto desde el otro lado"
                                .into(),
                        ),
                    )
                })
            },
        },
        ShapeRule {
            kind: Kind::Interface,
            path: &["spec", "requires"],
            check: |n| {
                no_vacio(
                    "requires",
                    "omite el documento en lugar de declarar una forma que no exige nada",
                )(n)
            },
        },
        ShapeRule {
            kind: Kind::Entity,
            path: &["spec", "uniqueKeys"],
            check: |n| no_vacio("uniqueKeys", "omite el campo en lugar de declararlo vacío")(n),
        },
        // Las claves obligatorias de un `Ruleset`, y la disyunción que el
        // esquema expresa con `anyOf`. Van sobre `spec` entero y no sobre cada
        // clave porque una regla sobre una clave ausente no llega a correr:
        // una clave que falta no tiene nodo donde mirarla.
        ShapeRule {
            kind: Kind::Ruleset,
            path: &["spec"],
            check: |n| {
                if n.get("owner").is_none() {
                    return Some((
                        "un `Ruleset` DEBE declarar `owner`".into(),
                        Some(
                            "y es independiente del dueño de los paquetes a los que apunta: ahí \
                             está la razón de que esto sea un documento y no un bloque dentro de \
                             `Entity`. En un entorno regulado, quien responde del cumplimiento \
                             tiene que poder restringir la ontología sin poder editarla"
                                .into(),
                        ),
                    ));
                }
                if n.get("targets").is_none() {
                    return Some((
                        "un `Ruleset` DEBE declarar `targets`".into(),
                        Some(
                            "una regla sin objetivo es una regla que enumera, y para eso ya está \
                             `quality` de ODCS colgando de la propiedad"
                                .into(),
                        ),
                    ));
                }
                if ["assertions", "masks", "duties"]
                    .iter()
                    .all(|k| n.get(k).is_none())
                {
                    return Some((
                        "este `Ruleset` no declara ninguna regla".into(),
                        Some(
                            "necesita al menos `assertions`, `masks` o `duties`: un objetivo sin \
                             nada que sostener selecciona propiedades y no las gobierna"
                                .into(),
                        ),
                    ));
                }
                None
            },
        },
        ShapeRule {
            kind: Kind::Lattice,
            path: &["spec", "levels"],
            check: |n| match n {
                Node::Sequence { items, .. } if items.len() < 2 => Some((
                    "`levels` necesita al menos dos niveles".into(),
                    Some("un retículo con un solo nivel no ordena nada".into()),
                )),
                _ => None,
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconoce_extensiones_de_proveedor() {
        assert!(is_extension("x-acme-owner"));
        assert!(is_extension("x-oos-dependencies"));
        // Sin proveedor, sin sufijo, o mayúsculas: no es una extensión válida.
        assert!(!is_extension("x-owner"));
        assert!(!is_extension("acmeOwner"));
        assert!(!is_extension("x-ACME-owner"));
    }

    #[test]
    fn los_kinds_se_resuelven() {
        for k in Kind::ALL {
            assert_eq!(Kind::parse(k.as_str()), Some(*k));
        }
        assert_eq!(Kind::parse("Ontology"), None);
    }

    #[test]
    fn function_es_de_v1alpha2_y_no_admite_etiquetas() {
        assert_eq!(Kind::Function.since(), ApiVersion::V1Alpha2);
        assert_eq!(Kind::Entity.since(), ApiVersion::V1Alpha1);
        // La ausencia que impide que una función se atestigüe a sí misma.
        assert!(!Kind::Function.metadata_keys().contains(&"labels"));
        assert!(Kind::Entity.metadata_keys().contains(&"labels"));
    }
}
