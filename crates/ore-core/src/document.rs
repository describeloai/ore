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
}

impl ApiVersion {
    pub const ALL: &'static [ApiVersion] = &[ApiVersion::V1Alpha1, ApiVersion::V1Alpha2];

    pub const fn as_str(self) -> &'static str {
        match self {
            ApiVersion::V1Alpha1 => "oos.dev/v1alpha1",
            ApiVersion::V1Alpha2 => "oos.dev/v1alpha2",
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
        }
    }

    /// La versión en la que este documento aparece.
    ///
    /// Un `Function` en un paquete de v1alpha1 no es un `kind` desconocido: es
    /// un documento del futuro, y el error tiene que decir eso.
    pub const fn since(self) -> ApiVersion {
        match self {
            Kind::Function | Kind::Resolution => ApiVersion::V1Alpha2,
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
            Kind::Binding
            | Kind::Lattice
            | Kind::ConduitPolicy
            | Kind::Function
            | Kind::Resolution => &["name", "namespace", "description"],
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
            ],
            Kind::Binding => &[
                "targetEntity",
                "datasourceRef",
                "profile",
                "source",
                "properties",
                "capabilities",
                "materialization",
            ],
            // `axis` es lo único que v1alpha2 añade al retículo, y de él sale
            // el combinador. `join` queda obsoleto: derivable, luego no
            // declarable (P2), y si aparece debe coincidir — `OOS7007`.
            Kind::Lattice => &["levels", "levelDescriptions", "join", "axis"],
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
        }
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
        ShapeRule {
            kind: Kind::Entity,
            path: &["spec", "uniqueKeys"],
            check: |n| no_vacio("uniqueKeys", "omite el campo en lugar de declararlo vacío")(n),
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
