//! **Las aristas que un paquete declara**, y de qué columnas salen.
//!
//! > Por cada relación con `via` de una entidad con clave simple: una
//! > proyección de **dos columnas** sobre la fuente física de esa entidad —la
//! > clave, y la columna del enlace.
//!
//! # Por qué esto vive aquí y no en quien lo usa
//!
//! Porque lo usan dos, y hasta ahora lo derivaban por separado: `ore-exec` para
//! construir el índice de topología, y el registro de copias de `ore-cli` para
//! saber qué copias tiene un paquete. Dos derivaciones de la misma cosa divergen
//! en la que ninguna prueba ejerce — que es exactamente lo que le pasó a esta:
//! el índice de topología **es** una vista materializada, escrita a mano en el
//! paradigma anterior, y nadie la reconocía como copia porque cada lado la
//! nombraba a su manera.
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

use crate::document::Kind;
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
    /// `<entidad>.<relación>`, cualificado.
    pub nombre: String,
    /// Quién declaró la fuente física: un binding, o la vista que respalda.
    pub declara: String,
    pub datasource: String,
    pub objeto: String,
    /// La columna de la clave primaria de la entidad.
    pub desde: String,
    /// La columna que sostiene el enlace.
    pub hasta: String,
}

/// Las fuentes físicas de una entidad: sus bindings, y la raíz de la vista que
/// la respalda.
///
/// Los dos caminos, y no solo el segundo: un documento v1alpha7 sigue
/// compilando mientras v1alpha1 sea normativo, así que un paquete con bindings
/// tiene que seguir dando sus aristas. Es la misma pareja que
/// [`crate::vistas::datasources_de`] ya recorre.
fn fisicas(pkg: &Package, e: &Loaded) -> Vec<(String, String, String, BTreeMap<String, String>)> {
    let qn = e.qname().unwrap_or_default();
    let mut out = Vec::new();
    for b in pkg.of(Kind::Binding) {
        if b.section("targetEntity").and_then(|t| t.as_str()) != Some(qn.as_str()) {
            continue;
        }
        let Some(ds) = b.section("datasourceRef").and_then(|v| v.as_str()) else {
            continue;
        };
        out.push((
            b.qname().unwrap_or_default(),
            ds.to_string(),
            b.section("source")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            columnas_de_binding(b),
        ));
    }
    if let Some(v) = vistas::respaldo(pkg, e)
        && let Ok(r) = vistas::raiz(pkg, v)
    {
        out.push((
            v.qname().unwrap_or_default(),
            r.datasource,
            r.objeto,
            r.columnas,
        ));
    }
    out
}

/// Propiedad → columna de un binding. Admite la forma breve y la expandida.
fn columnas_de_binding(b: &Loaded) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some(ps) = b.section("properties") else {
        return out;
    };
    for (k, v) in ps.entries() {
        let Some(nombre) = k.as_str() else { continue };
        let col = v.as_str().map(str::to_string).or_else(|| {
            v.get("column")
                .and_then(|(_, c)| c.as_str())
                .map(str::to_string)
        });
        if let Some(col) = col {
            out.insert(nombre.to_string(), col);
        }
    }
    out
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
        let fuentes = fisicas(pkg, e);
        for (rk, rv) in rels.entries() {
            let Some(rel) = rk.as_str() else { continue };
            let via = lista(rv.get("via").map(|(_, v)| v));
            if via.len() != 1 {
                continue;
            }
            for (declara, datasource, objeto, columnas) in &fuentes {
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
                });
            }
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

    /// **El camino del binding, que no ejerce ningún otro fichero del árbol.**
    ///
    /// Ninguno de los casos de `ore-exec` tiene bindings *y* relaciones a la vez,
    /// y `acme-retail` ya no tiene bindings. Así que si esta rama se cayera al
    /// mover la derivación aquí, nadie se enteraría — y un paquete v1alpha7 con
    /// bindings dejaría de tener topología en silencio. Sigue siendo legal
    /// mientras v1alpha1 lo sea.
    #[test]
    fn un_binding_da_sus_aristas_igual_que_una_vista() {
        let b = doc(
            Kind::Binding,
            "apiVersion: oos.dev/v1alpha1\nkind: Binding\n\
             metadata: { name: workday, namespace: hr }\nspec:\n  \
             targetEntity: hr.Employee\n  datasourceRef: erp\n  source: public.workers\n  \
             properties:\n    employeeId: worker_id\n    managerId: { column: mgr_ref }\n",
        );
        let a = aristas(&paquete(vec![entidad(RELACION), b]));
        assert_eq!(a.len(), 1, "{a:?}");
        assert_eq!(a[0].nombre, "hr.Employee.manager");
        assert_eq!(a[0].declara, "hr.workday");
        assert_eq!(
            (a[0].datasource.as_str(), a[0].objeto.as_str()),
            ("erp", "public.workers")
        );
        // La forma breve y la expandida, las dos.
        assert_eq!(
            (a[0].desde.as_str(), a[0].hasta.as_str()),
            ("worker_id", "mgr_ref")
        );
    }

    /// Sin fuente física no hay columnas contra las que proyectar, y una arista
    /// sin columnas no es media arista: no es ninguna. Es el caso de cinco de
    /// las siete entidades de `acme-retail`, que no declaran `backedBy`.
    #[test]
    fn una_entidad_sin_fuente_fisica_no_da_aristas() {
        assert!(aristas(&paquete(vec![entidad(RELACION)])).is_empty());
    }

    /// Una `via` compuesta es una clave de destino en tupla. Aplanarla aquí
    /// inventaría una codificación que nadie declaró, así que se descarta — y
    /// se descarta **entera**, no a medias.
    #[test]
    fn una_via_compuesta_se_descarta_en_vez_de_aplanarse() {
        let b = doc(
            Kind::Binding,
            "apiVersion: oos.dev/v1alpha1\nkind: Binding\n\
             metadata: { name: workday, namespace: hr }\nspec:\n  \
             targetEntity: hr.Employee\n  datasourceRef: erp\n  source: public.workers\n  \
             properties:\n    employeeId: worker_id\n    managerId: mgr_ref\n    pais: country\n",
        );
        let compuesta = "  relations:\n    manager:\n      target: hr.Employee\n      \
             cardinality: many_to_one\n      via: [managerId, pais]\n";
        assert!(aristas(&paquete(vec![entidad(compuesta), b])).is_empty());
    }
}
