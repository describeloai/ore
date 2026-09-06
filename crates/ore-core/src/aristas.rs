//! **Las aristas que un paquete declara**, y de qué columnas salen.
//!
//! > Por cada relación con `via` de una entidad con clave simple: una
//! > proyección de **dos columnas** sobre la fuente física de esa entidad —la
//! > clave, y la columna del enlace.
//!
//! # Por qué esto vive aquí y no en quien lo usa
//!
//! Porque lo usaban dos y lo derivaban por separado: el ejecutor del paradigma
//! de bindings, para construir el índice de topología, y el registro de copias
//! de `ore-cli`. Dos derivaciones de la misma cosa divergen en la que ninguna
//! prueba ejerce — que es exactamente lo que le pasó a esta: el índice de
//! topología **es** una vista materializada, escrita a mano en el paradigma
//! anterior, y nadie la reconocía como copia porque cada lado la nombraba a su
//! manera.
//!
//! **De los dos consumidores queda uno**: `ore-exec` se retiró. Esto se queda
//! aquí igualmente —una lectura de la gramática es del núcleo, la usen dos o
//! uno— y se dice que el motivo original ya no aplica, que es distinto de que la
//! decisión haya dejado de ser correcta.
//!
//! Y vive en el núcleo porque **es una lectura de la gramática**, no álgebra:
//! `relations`, `via`, `primaryKey` y de dónde sale físicamente una entidad. Lo
//! mismo que ya hacen [`crate::vistas::respaldo`] y
//! [`crate::vistas::datasources_de`], que también van de una entidad a lo
//! físico. Cada consumidor construye encima su propia representación —una
//! `Lectura` del ejecutor, un plan del motor de vistas— y **ninguno de los dos
//! necesita al otro**.
//!
//! # Lo que descarta, y por qué se descarta y no se inventa
//!
//! Una clave o una `via` **compuesta** es una tupla, y aplanarla aquí
//! inventaría una codificación que nadie declaró. Se salta, igual que se
//! saltaba antes.

use std::collections::BTreeMap;

use crate::link::{Loaded, Package};
use crate::vistas;

/// Una arista declarada, ya bajada a columnas físicas.
///
/// `desde` y `hasta` se llaman así y no `clave`/`via` porque es lo que sale por
/// el protocolo del driver: la proyección se pide con esos dos nombres y lo que
/// vuelve **ya es una arista**, sin que el driver se entere de que esto es un
/// índice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arista {
    /// `<entidad>.<relación>`, cualificado. Es `entidad` y `relacion` unidos, y
    /// se guarda igualmente porque es **la identidad de la copia** en el
    /// registro: `oretopo·hr.Employee.manager`.
    pub nombre: String,
    /// Quién declaró la fuente física: un binding, o la vista que respalda.
    pub declara: String,
    pub datasource: String,
    pub objeto: String,
    /// La columna de la clave primaria de la entidad.
    pub desde: String,
    /// La columna que sostiene el enlace.
    pub hasta: String,

    // ── Lo que el SELLO necesita, y el driver no ────────────────────────────
    //
    // Arriba todo son nombres físicos, porque el driver no conoce el modelo.
    // El sello es al revés: las etiquetas las pone la ENTIDAD sobre sus
    // propiedades, así que para preguntar «¿qué lleva puesto lo que se copia?»
    // hacen falta los nombres de arriba.
    //
    // Podrían derivarse fuera —partir `nombre`, releer `primaryKey`— y eso
    // sería la segunda derivación de lo mismo, que es exactamente lo que este
    // módulo existe para impedir.
    /// La entidad, cualificada.
    pub entidad: String,
    /// La relación, sin cualificar: lo que el diagnóstico tiene que nombrar.
    pub relacion: String,
    /// La **propiedad** de la clave primaria — `desde` es su columna.
    pub clave: String,
    /// La **propiedad** del enlace — `hasta` es su columna.
    pub via: String,
    /// Si la fuente la declara una **vista**, y no un binding.
    ///
    /// Decide de quién es la copia. Un binding **declara** su
    /// `materialization.topology` y el sello corre sobre la declaración; una
    /// vista no declara nada —lo derivable no se declara (P2)— y por eso el
    /// sello tiene que correr sobre esta derivación. Los dos caminos llegan al
    /// mismo conducto por sitios distintos, y confundirlos sellaría dos veces
    /// lo mismo o ninguna.
    pub derivada: bool,
}

/// La fuente física de una entidad: la raíz de la vista que la respalda.
///
/// Devolvía una lista porque había dos caminos —los bindings de la entidad y su
/// vista—. Con `Binding` retirado hay uno, y una entidad sale de una vista o de
/// ninguna: la lista era la forma de decir «puede haber varios», y ya no puede.
type Fisica = (String, String, String, BTreeMap<String, String>);

fn fisicas(pkg: &Package, e: &Loaded) -> Option<Fisica> {
    let v = vistas::respaldo(pkg, e)?;
    let r = vistas::raiz(pkg, v).ok()?;
    Some((v.qname().unwrap_or_default(), r.datasource, r.objeto, r.columnas))
}


/// **La derivación.** Determinista: recorre en el orden en que el paquete lo
/// declara, que es el mismo que la forma canónica fija.
pub fn aristas(pkg: &Package) -> Vec<Arista> {
    let mut out = Vec::new();
    for e in pkg.entities() {
        let (Some(qn), Some(rels)) = (e.qname(), e.section("relations")) else {
            continue;
        };
        let clave = lista(e.section("primaryKey"));
        if clave.len() != 1 {
            continue;
        }
        let Some((declara, datasource, objeto, columnas)) = fisicas(pkg, e) else {
            continue;
        };
        for (rk, rv) in rels.entries() {
            let Some(rel) = rk.as_str() else { continue };
            let via = lista(rv.get("via").map(|(_, v)| v));
            if via.len() != 1 {
                continue;
            }
            let (Some(desde), Some(hasta)) = (columnas.get(&clave[0]), columnas.get(&via[0]))
            else {
                continue;
            };
            out.push(Arista {
                nombre: format!("{qn}.{rel}"),
                declara: declara.clone(),
                datasource: datasource.clone(),
                objeto: objeto.clone(),
                desde: desde.clone(),
                hasta: hasta.clone(),
                entidad: qn.clone(),
                relacion: rel.to_string(),
                clave: clave[0].clone(),
                via: via[0].clone(),
                derivada: true,
            });
        }
    }
    out
}

fn lista(n: Option<&crate::parse::Node>) -> Vec<String> {
    n.map(|x| {
        x.items()
            .iter()
            .filter_map(|i| i.as_str().map(String::from))
            .collect()
    })
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Kind;
    use crate::parse::parse;
    use std::path::PathBuf;

    fn doc(kind: Kind, texto: &str) -> Loaded {
        Loaded {
            path: PathBuf::from(format!("{}.yaml", kind.as_str())),
            kind,
            root: parse(texto).expect("yaml"),
        }
    }

    fn paquete(docs: Vec<Loaded>) -> Package {
        Package {
            root: PathBuf::from("."),
            docs,
            cedar: Vec::new(),
            generated: Vec::new(),
            sobres: Vec::new(),
        }
    }

    fn entidad(extra: &str) -> Loaded {
        doc(
            Kind::Entity,
            &format!(
                "apiVersion: oos.dev/v1alpha8\nkind: Entity\n\
                 metadata: {{ name: Employee, namespace: hr }}\nspec:\n  nature: entity\n  \
                 primaryKey: [employeeId]\n  properties:\n    \
                 employeeId: {{ type: String }}\n    managerId: {{ type: String }}\n{extra}"
            ),
        )
    }

    const RELACION: &str = "  relations:\n    manager:\n      target: hr.Employee\n      \
         cardinality: many_to_one\n      via: [managerId]\n";

    /// La tabla y la vista que dan las columnas.
    ///
    /// Se escriben aquí porque la arista sale de la **raíz** de la vista y no
    /// de la vista: si esa resolución se rompiera, una entidad con `backedBy`
    /// dejaría de tener topología en silencio.
    fn sustrato() -> Vec<Loaded> {
        vec![
            doc(
                Kind::Table,
                "apiVersion: oos.dev/v1alpha8\nkind: Table\n\
                 metadata: { name: workers, namespace: erp }\nspec:\n  datasource: erp\n  \
                 object: public.workers\n  columns:\n    worker_id: {}\n    mgr_ref: {}\n    \
                 country: {}\n  reads: { fullScan: cheap }\n  changes: { mode: none }\n",
            ),
            doc(
                Kind::View,
                "apiVersion: oos.dev/v1alpha8\nkind: View\n\
                 metadata: { name: empleados, namespace: hr }\nspec:\n  owner: team:hr\n  \
                 from: { table: erp.workers }\n  fields:\n    employeeId: worker_id\n    \
                 managerId: mgr_ref\n    pais: country\n",
            ),
        ]
    }

    /// La arista sale de la raíz de la vista, con sus **columnas físicas** y no
    /// con los nombres de los campos.
    ///
    /// Sustituye a una pareja de pruebas que afirmaba lo mismo por los dos
    /// caminos —binding y vista—. Con `Binding` retirado queda uno, y la
    /// afirmación no pierde nada: lo que se comprobaba era que la proyección
    /// baja hasta la columna, y eso es de la vista.
    #[test]
    fn la_arista_sale_de_las_columnas_de_la_raiz() {
        let mut docs = sustrato();
        docs.push(entidad(&format!("  backedBy: empleados\n{RELACION}")));
        let a = aristas(&paquete(docs));
        assert_eq!(a.len(), 1, "{a:?}");
        assert_eq!(a[0].nombre, "hr.Employee.manager");
        assert_eq!(a[0].declara, "hr.empleados");
        assert_eq!(
            (a[0].datasource.as_str(), a[0].objeto.as_str()),
            ("erp", "public.workers")
        );
        assert_eq!(
            (a[0].desde.as_str(), a[0].hasta.as_str()),
            ("worker_id", "mgr_ref")
        );
    }

    /// Sin fuente física no hay columnas contra las que proyectar, y una arista
    /// sin columnas no es media arista: no es ninguna.
    #[test]
    fn una_entidad_sin_fuente_fisica_no_da_aristas() {
        assert!(aristas(&paquete(vec![entidad(RELACION)])).is_empty());
    }

    /// Una `via` compuesta es una clave de destino en tupla. Aplanarla aquí
    /// inventaría una codificación que nadie declaró, así que se descarta — y
    /// se descarta **entera**, no a medias.
    #[test]
    fn una_via_compuesta_se_descarta_en_vez_de_aplanarse() {
        let compuesta = "  backedBy: empleados\n  relations:\n    manager:\n      \
             target: hr.Employee\n      cardinality: many_to_one\n      \
             via: [managerId, pais]\n";
        let mut docs = sustrato();
        docs.push(entidad(compuesta));
        assert!(aristas(&paquete(docs)).is_empty());
    }
}
