//! `OOS2035` — **la identidad de un documento se declara una vez**.
//!
//! # Qué faltaba, y se midió el 2026-09-17
//!
//! `90-canonical` §5.2 dice cuál es la identidad de un documento: el par
//! `kind` + nombre cualificado —`Entity:hr.Employee`—, y **nunca su ruta**. El
//! digest del paquete se construye sobre esa lista, y una lista de identidades
//! con una repetida no es una lista de documentos: es un paquete con dos
//! verdades bajo el mismo nombre.
//!
//! Y nadie lo comprobaba. Dos ficheros que declaraban la misma `Entity`, la
//! misma `View`, la misma `Table`, el mismo `Lattice`, el mismo `Concept` o el
//! mismo `Package` compilaban limpios, los cinco medidos
//! (`pruebas-de-fuego/medida-forge-concept-e-interface.py`, C7). Lo que
//! pasaba después era peor que un error: cada referencia resolvía **la
//! primera que encontraba** —`Package::entity` es un `find`—, así que
//! `backedBy: empleados` apuntaba a una de las dos según el orden de lectura
//! del directorio, y un verbo que buscara por nombre reescribiría una y
//! dejaría la otra.
//!
//! # Por qué va antes del enlazado
//!
//! Por lo mismo que `OOS2030`: si hay dos, todo lo que las nombra resuelve mal
//! y los diagnósticos que salgan son la consecuencia, no la causa. `99-errors`
//! §2.1: gana el código específico.
//!
//! # Lo que cuenta como la misma identidad
//!
//! `kind` y nombre cualificado, exactamente como el `docId` del digest. Un
//! `View` y una `Table` que se llamen igual no chocan: son identidades
//! distintas, y `from.table` y `from.view` ya dicen cuál se nombra. Un
//! documento importado (`vendor/*.oob`) cuenta igual que uno del árbol: si el
//! árbol declara lo que también importa, hay dos, y ninguna referencia sabría
//! cuál manda.

use crate::code::Code;
use crate::diag::Diagnostic;
use crate::link::Package;
use std::collections::BTreeMap;

pub fn check(pkg: &Package) -> Vec<Diagnostic> {
    // identidad → los documentos que la declaran, en el orden de carga
    let mut por_identidad: BTreeMap<String, Vec<&crate::link::Loaded>> = BTreeMap::new();
    for d in &pkg.docs {
        let Some(qn) = d.qname() else { continue };
        por_identidad
            .entry(format!("{:?}:{qn}", d.kind))
            .or_default()
            .push(d);
    }
    let mut out = un_nombre_una_cosa(pkg);
    for (identidad, docs) in por_identidad {
        if docs.len() < 2 {
            continue;
        }
        let donde: Vec<String> = docs
            .iter()
            .map(|d| {
                d.path
                    .strip_prefix(&pkg.root)
                    .unwrap_or(&d.path)
                    .display()
                    .to_string()
                    .replace('\\', "/")
            })
            .collect();
        // Un diagnóstico por documento de más, apuntando a él: el primero es
        // el que las referencias resolvían y no tiene por qué ser el que
        // sobra, pero quien lea el error tiene que ver los dos sitios.
        for d in &docs[1..] {
            out.push(
                Diagnostic::new(
                    Code::Oos2035,
                    &d.path,
                    format!(
                        "`{identidad}` está declarado {} veces: {}",
                        docs.len(),
                        donde.join(" · ")
                    ),
                )
                .at(d.root.pos())
                .help(
                    "la identidad de un documento es su kind y su nombre cualificado, y el \
                     fichero es incidental (90-canonical §5.2): dos ficheros con la misma \
                     identidad son dos verdades, y ninguna referencia sabría cuál resolver. \
                     Retira uno, o dale otro nombre",
                ),
            );
        }
    }
    out
}

/// v1alpha14 · **un nombre, una cosa**. En SQL un nombre no dice su `kind`:
/// `FROM ventas.clientes` no puede elegir entre una tabla y una vista que se
/// llamen así. Desde v1alpha14 una `Table`, una `View` y un `Dataset` comparten
/// el espacio de nombres de su schema, como en Unity Catalog, y dos con el
/// mismo nombre son la misma identidad (ADR 0040). Hasta v1alpha13 podían
/// convivir —`from.table` y `from.view` decían cuál— y siguen pudiendo: la
/// regla alcanza a la pareja en cuanto uno de los dos es de v1alpha14.
fn un_nombre_una_cosa(pkg: &Package) -> Vec<Diagnostic> {
    use crate::document::{ApiVersion, Kind};
    let mut por_nombre: BTreeMap<String, Vec<&crate::link::Loaded>> = BTreeMap::new();
    for d in &pkg.docs {
        if !matches!(d.kind, Kind::Table | Kind::View | Kind::Dataset) {
            continue;
        }
        let Some(qn) = d.qname() else { continue };
        por_nombre.entry(qn).or_default().push(d);
    }
    let mut out = Vec::new();
    for (qn, docs) in por_nombre {
        let kinds: std::collections::BTreeSet<&str> =
            docs.iter().map(|d| d.kind.as_str()).collect();
        if kinds.len() < 2
            || !docs
                .iter()
                .any(|d| d.version().is_some_and(|v| v >= ApiVersion::V1Alpha14))
        {
            continue;
        }
        let que: Vec<&str> = kinds.into_iter().collect();
        for d in docs
            .iter()
            .filter(|d| d.version().is_some_and(|v| v >= ApiVersion::V1Alpha14))
        {
            out.push(
                Diagnostic::new(
                    Code::Oos2035,
                    &d.path,
                    format!(
                        "`{qn}` es a la vez {}: en v1alpha14 un nombre es una cosa",
                        que.join(" y ")
                    ),
                )
                .at(d.root.pos())
                .help(
                    "una tabla, una vista y un dataset comparten el espacio de nombres de su \
                     schema, como en Unity Catalog: una consulta SQL nombra por nombre y no puede \
                     decir cuál lee. Dale otro nombre a uno —una vista sobre la tabla del mismo \
                     nombre se llama como lo que pregunta—",
                ),
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Kind;
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

    const EMPLEADO: &str = "apiVersion: oos.dev/v1alpha1\nkind: Entity\n\
        metadata: { name: Employee, namespace: hr }\n\
        spec:\n  nature: entity\n  primaryKey: [id]\n  properties:\n    id: { type: String }\n";

    #[test]
    fn dos_ficheros_con_la_misma_entidad_son_oos2035() {
        let pkg = paquete(vec![
            doc("packages/hr/entities/Employee.yaml", Kind::Entity, EMPLEADO),
            doc(
                "packages/hr/entities/Employee2.yaml",
                Kind::Entity,
                EMPLEADO,
            ),
        ]);
        let out = check(&pkg);
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(out[0].code, Code::Oos2035);
        assert!(
            out[0]
                .message
                .contains("`Entity:hr.Employee` está declarado 2 veces")
        );
        assert!(
            out[0]
                .message
                .contains("Employee.yaml · packages/hr/entities/Employee2.yaml")
        );
    }

    /// El mismo nombre en dos kinds no es la misma identidad: `from.table` y
    /// `from.view` ya dicen cuál se nombra.
    #[test]
    fn el_mismo_nombre_en_dos_kinds_no_choca() {
        let vista = "apiVersion: oos.dev/v1alpha8\nkind: View\nmetadata: { name: empleados, namespace: hr }\n\
            spec:\n  owner: team:hr\n  from: { table: t }\n  fields: { id: id }\n";
        let tabla = "apiVersion: oos.dev/v1alpha8\nkind: Table\nmetadata: { name: empleados, namespace: hr }\n\
            spec:\n  datasource: erp\n  object: t\n  columns: { id: {} }\n  reads: {}\n  changes: { mode: none, witness: none }\n";
        let pkg = paquete(vec![
            doc("packages/hr/views/empleados.yaml", Kind::View, vista),
            doc("packages/hr/tables/empleados.yaml", Kind::Table, tabla),
        ]);
        assert!(check(&pkg).is_empty());
    }

    /// Y un `Package` se identifica por su nombre a secas: dos manifiestos
    /// `hr` son dos paquetes que dicen ser el mismo.
    #[test]
    fn dos_manifiestos_del_mismo_paquete_son_oos2035() {
        let m = "apiVersion: oos.dev/v1alpha1\nkind: Package\nmetadata: { name: hr, version: 1.0.0 }\nspec: { owner: team:hr }\n";
        let pkg = paquete(vec![
            doc("packages/hr/package.yaml", Kind::Package, m),
            doc("packages/hr/package2.yaml", Kind::Package, m),
        ]);
        let out = check(&pkg);
        assert_eq!(out.len(), 1);
        assert!(out[0].message.contains("`Package:hr`"));
    }

    #[test]
    fn tres_copias_son_dos_diagnosticos() {
        let pkg = paquete(vec![
            doc("a.yaml", Kind::Entity, EMPLEADO),
            doc("b.yaml", Kind::Entity, EMPLEADO),
            doc("c.yaml", Kind::Entity, EMPLEADO),
        ]);
        assert_eq!(check(&pkg).len(), 2);
    }
}
