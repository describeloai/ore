//! `OOS2030` — **un documento pertenece al paquete en el que vive**.
//!
//! # Qué faltaba, y se midió con dos experimentos
//!
//! La pertenencia la dice **el directorio** —`01-package` §3.3 lo midió y por eso
//! el manifiesto no lista sus documentos: *«eso ya lo dice el directorio, y
//! redeclararlo sería declarar lo derivable»*—. La identidad la dice
//! **`metadata.namespace`**, que es con lo que `backedBy`, `from.view` y
//! `exports` se refieren a todo.
//!
//! Y **nadie ataba las dos**. Un documento que vive en `packages/ventas` y se
//! llama a sí mismo `otro.E` validaba limpio; y ese mismo paquete declarando
//! `exports: [ventas.E]` fallaba con `OOS2027`, porque `exports` habla en nombre
//! cualificado y el documento se llama otra cosa. Las dos mitades de la pinza.
//!
//! La consecuencia práctica es que *«mover un documento a otro paquete»* no
//! tenía un significado único: eran dos cosas —el fichero y el nombre— que se
//! movían por separado sin que nada protestara. De los cinco pasos que un
//! movimiento exige, cuatro los cazaba alguien —`OOS5007` al hacer `diff`,
//! `OOS2028` al compilar, `OOS2018` al validar— y **este no lo cazaba nadie**.
//!
//! # Por qué mira el `kind`, y por qué la primera versión no lo hacía
//!
//! La medida encontró **dos poblaciones y no una mezcla**: el contenido
//! gobernado casa casi siempre —`Entity` 273/6, `View` 127/6, `Function` 25/0—
//! y el vocabulario compartido casi nunca —`Lattice` 2/197, `Ruleset` 0/37—.
//! `gdpr.sensitivity` tiene que significar lo mismo en `hr` y en `crm`, que es
//! justo la propiedad que lo hace útil.
//!
//! La primera versión intentó separarlas **por ubicación** —el vocabulario
//! compartido cuelga de la raíz del workspace, que es donde `ore init` lo pone,
//! así que no habría con qué discrepar— y así se ahorraba la lista. **No vale**,
//! y lo dijo la suite: en un árbol **plano**, donde el `package.yaml` está en la
//! raíz, *todo* está dentro del paquete —incluido `lattices/`— y no hay un
//! «fuera» al que mover un retículo compartido. La lista hace falta, y por eso
//! lleva censo: añadir un `kind` sin decir de qué población es no compila.
//!
//! # La puerta de v1alpha8, y no es por los fixtures
//!
//! Es el mismo razonamiento —y la misma puerta— que `OOS2028`: *«un documento
//! anterior se escribió cuando un paquete no tenía superficie pública, y
//! aplicárselo cambiaría lo que significa algo ya escrito»*. Aquí igual, y sobre
//! documentos de usuarios: un paquete v1alpha1 con `hr.employees` dentro de
//! `packages/hr` compila hoy, y retroactivarlo sería cambiarle el significado a
//! algo publicado.
//!
//! # Y por qué corre ANTES del enlazado
//!
//! Porque si un documento está en el espacio equivocado, **todo lo que lo nombra
//! falla** —`OOS2018`, `OOS2005`— y esos diagnósticos son la *consecuencia*.
//! `99-errors` §2.1 dice que gana el código específico, y es el mismo argumento
//! por el que `politica::check` va al final: adelantar una consecuencia manda a
//! mirar el fichero equivocado.

use std::collections::BTreeMap;
use std::path::Path;

use crate::code::Code;
use crate::diag::Diagnostic;
use crate::document::{ApiVersion, Kind};
use crate::link::Package;

/// Los `kind` cuyo nombre **es del paquete**: contenido gobernado, que alguien
/// posee y que se mueve entre paquetes.
///
/// Publica porque no es solo de esta regla: `ore package move` mueve
/// exactamente esto, y una segunda lista con los mismos nombres divergiria.
pub const DEL_PAQUETE: &[Kind] = &[
    Kind::Entity,
    Kind::View,
    Kind::Table,
    Kind::Function,
    Kind::Resolution,
    // Retirado en v1alpha8, y la puerta de version hace que no pueda llegar
    // aqui nunca. Se clasifica igual: el censo exige decirlo, y no decirlo
    // seria dejar que la ausencia signifique dos cosas.
    Kind::Binding,
];

/// Los de **vocabulario compartido**: su nombre es el del vocabulario, y tiene
/// que ser el mismo desde todos los paquetes o deja de compartirse.
pub const COMPARTIDO: &[Kind] = &[
    Kind::Lattice,
    Kind::Ruleset,
    Kind::Concept,
    Kind::Interface,
    Kind::ConduitPolicy,
    Kind::RequestPolicy,
];

/// Y los estructurales: el manifiesto **es** el nombre, asi que no puede
/// discrepar de si mismo, y `OntologyConfig` es del workspace y no lleva
/// espacio de nombres — sus secciones cuelgan de la raiz.
pub const ESTRUCTURAL: &[Kind] = &[Kind::Package, Kind::OntologyConfig];

pub fn check(pkg: &Package) -> Vec<Diagnostic> {
    let miembros = crate::link::miembros(pkg);
    if miembros.is_empty() {
        return Vec::new();
    }
    // Directorio del miembro -> el nombre que su manifiesto declara.
    let nombres: BTreeMap<&Path, &str> = pkg
        .docs
        .iter()
        .filter(|d| d.kind == Kind::Package)
        .filter_map(|d| Some((d.path.parent()?, d.meta("name")?.as_str()?)))
        .collect();

    let mut out = Vec::new();
    for d in &pkg.docs {
        // El manifiesto **es** el nombre, así que no puede discrepar de sí
        // mismo; y `OntologyConfig` es del workspace y no lleva espacio de
        // nombres — sus secciones cuelgan de la raíz.
        if !DEL_PAQUETE.contains(&d.kind) {
            continue;
        }
        if d.version().is_none_or(|v| v < ApiVersion::V1Alpha8) {
            continue;
        }
        let Some(m) = crate::link::miembro_de(&miembros, &d.path) else {
            continue; // en la raíz: no hay paquete con el que discrepar
        };
        let Some(suyo) = nombres.get(m) else {
            continue;
        };
        let declarado = d.meta("namespace").and_then(|n| n.as_str());
        if declarado == Some(*suyo) {
            continue;
        }
        let pos = d
            .root
            .get("metadata")
            .and_then(|(_, m)| m.get("namespace"))
            .map(|(k, _)| k.pos())
            .unwrap_or_else(|| d.root.pos());
        let dicho = match declarado {
            Some(n) => format!("dice `{n}`"),
            None => "no lo dice".to_string(),
        };
        out.push(
            Diagnostic::new(
                Code::Oos2030,
                &d.path,
                format!("este documento vive en el paquete `{suyo}` y {dicho}"),
            )
            .at(pos)
            .help(if declarado.is_some() {
                "un documento pertenece al paquete en el que vive, y su `namespace` es el \
                 nombre de ese paquete. O se corrige el `namespace`, o el documento está en \
                 el directorio equivocado — y `ore package move` hace las dos cosas a la vez"
            } else {
                "sin `namespace` su nombre cualificado no lleva el del paquete, así que \
                 `exports` no puede nombrarlo y una referencia de fuera no lo alcanza. El \
                 vocabulario compartido —retículos, reglas, políticas— vive en la raíz del \
                 workspace, no dentro de un paquete"
            }),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn doc(ruta: &str, kind: Kind, texto: &str) -> crate::link::Loaded {
        crate::link::Loaded {
            path: PathBuf::from(ruta),
            kind,
            root: crate::parse::parse(texto).expect("analiza"),
        }
    }

    fn paquete(docs: Vec<crate::link::Loaded>) -> Package {
        Package {
            root: PathBuf::from("."),
            docs,
            cedar: Vec::new(),
            generated: Vec::new(),
            sobres: Vec::new(),
        }
    }

    const MANIFIESTO: &str = "apiVersion: oos.dev/v1alpha1\nkind: Package\n\
         metadata: { name: ventas, version: 0.1.0 }\nspec: { owner: team:datos }\n";

    fn vista(ns: Option<&str>, version: &str) -> String {
        let meta = match ns {
            Some(n) => format!("{{ name: v, namespace: {n} }}"),
            None => "{ name: v }".to_string(),
        };
        format!(
            "apiVersion: oos.dev/{version}\nkind: View\nmetadata: {meta}\n\
             spec:\n  owner: team:datos\n  from: {{ table: t }}\n  fields: {{ a: a }}\n"
        )
    }

    /// **El censo.** Sin el, la lista se puede ampliar sin mirar: alguien anade
    /// un `kind` y la regla deja de cobrarlo en silencio, que es el defecto
    /// peligroso —conceder— y no el ruidoso.
    #[test]
    fn cada_kind_dice_de_que_poblacion_es() {
        let sin: Vec<&str> = Kind::ALL
            .iter()
            .filter(|k| {
                !DEL_PAQUETE.contains(k) && !COMPARTIDO.contains(k) && !ESTRUCTURAL.contains(k)
            })
            .map(|k| k.as_str())
            .collect();
        assert!(
            sin.is_empty(),
            "estos `kind` no dicen si su nombre es del paquete o de un              vocabulario compartido: {sin:?}"
        );
        // Y en una sola: clasificar dos veces es no clasificar.
        for k in Kind::ALL {
            let n = [DEL_PAQUETE, COMPARTIDO, ESTRUCTURAL]
                .iter()
                .filter(|l| l.contains(k))
                .count();
            assert_eq!(n, 1, "`{}` esta en {n} poblaciones", k.as_str());
        }
    }

    /// **La regla, en sus dos direcciones.**
    #[test]
    fn un_documento_pertenece_al_paquete_en_el_que_vive() {
        let bien = paquete(vec![
            doc("packages/ventas/package.yaml", Kind::Package, MANIFIESTO),
            doc(
                "packages/ventas/views/v.yaml",
                Kind::View,
                &vista(Some("ventas"), "v1alpha8"),
            ),
        ]);
        assert!(check(&bien).is_empty(), "el que casa no dice nada");

        let mal = paquete(vec![
            doc("packages/ventas/package.yaml", Kind::Package, MANIFIESTO),
            doc(
                "packages/ventas/views/v.yaml",
                Kind::View,
                &vista(Some("otro"), "v1alpha8"),
            ),
        ]);
        let d = check(&mal);
        assert_eq!(d.len(), 1, "{d:?}");
        assert_eq!(d[0].code, Code::Oos2030);
        assert!(d[0].message.contains("`ventas`") && d[0].message.contains("`otro`"));
    }

    /// Sin `namespace` tampoco: su nombre cualificado no lleva el del paquete,
    /// así que `exports` no puede nombrarlo. Se midió antes de decidirlo —**tres**
    /// documentos v1alpha8 del submódulo están dentro de un paquete sin
    /// `namespace`, y los tres son `OntologyConfig`, que está exento.
    #[test]
    fn omitirlo_no_es_una_forma_de_estar_de_acuerdo() {
        let p = paquete(vec![
            doc("packages/ventas/package.yaml", Kind::Package, MANIFIESTO),
            doc(
                "packages/ventas/views/v.yaml",
                Kind::View,
                &vista(None, "v1alpha8"),
            ),
        ]);
        let d = check(&p);
        assert_eq!(d.len(), 1);
        assert!(d[0].message.contains("no lo dice"), "{:?}", d[0].message);
    }

    /// **La puerta.** Un documento anterior se escribió cuando el `namespace` no
    /// significaba pertenencia; aplicárselo cambiaría lo que dice.
    #[test]
    fn un_documento_anterior_a_v1alpha8_no_la_cobra() {
        let p = paquete(vec![
            doc("packages/ventas/package.yaml", Kind::Package, MANIFIESTO),
            doc(
                "packages/ventas/views/v.yaml",
                Kind::View,
                &vista(Some("otro"), "v1alpha7"),
            ),
        ]);
        assert!(check(&p).is_empty());
    }

    /// El vocabulario compartido **dentro** de un paquete tampoco se cobra, y
    /// este es el caso que tumbó la version por ubicacion: un arbol plano, donde
    /// el `package.yaml` esta en la raiz y no hay un «fuera» al que mover un
    /// reticulo.
    #[test]
    fn el_vocabulario_compartido_no_se_cobra_ni_dentro_de_un_paquete() {
        let p = paquete(vec![
            doc("packages/ventas/package.yaml", Kind::Package, MANIFIESTO),
            doc(
                "packages/ventas/lattices/gdpr.yaml",
                Kind::Lattice,
                "apiVersion: oos.dev/v1alpha8\nkind: Lattice\n\
                 metadata: { name: sensitivity, namespace: gdpr }\n\
                 spec: { levels: [none, high], join: max }\n",
            ),
        ]);
        assert!(check(&p).is_empty());
    }

    /// Y un paquete dueño de su vocabulario tampoco se cobra —el `kind` decide,
    /// no la ubicación—, que es lo correcto: quien lo comparte es quien lo
    /// nombra, y el compilador no tiene que opinar.
    #[test]
    fn un_paquete_dueno_de_su_vocabulario_tampoco_se_cobra() {
        let p = paquete(vec![
            doc(
                "packages/gdpr/package.yaml",
                Kind::Package,
                "apiVersion: oos.dev/v1alpha1\nkind: Package\n\
                 metadata: { name: gdpr, version: 0.1.0 }\nspec: { owner: team:legal }\n",
            ),
            doc(
                "packages/gdpr/lattices/sensitivity.yaml",
                Kind::Lattice,
                "apiVersion: oos.dev/v1alpha8\nkind: Lattice\n\
                 metadata: { name: sensitivity, namespace: gdpr }\n\
                 spec: { levels: [none, high], join: max }\n",
            ),
        ]);
        assert!(check(&p).is_empty());
    }
}
