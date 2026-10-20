//! **La tabla**, tal como `system_schema` la cuenta: cada columna con su rol
//! (clave de partición, de clustering, estática o normal), su tipo y el índice
//! que la ayuda a filtrar. Es lo que el traductor necesita para saber qué puede
//! pedirle el servidor sin `ALLOW FILTERING` (ficha 0070 §3.2).
//!
//! Función pura sobre las filas que el transporte lee de `system_schema.columns`
//! y `system_schema.indexes`: sin servidor.

use crate::tipos::{Cql, Tipo};

/// Qué papel juega una columna en la clave.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rol {
    /// Clave de partición, en su posición (0, 1…).
    Particion(usize),
    /// Clave de clustering, en su posición y su orden.
    Clustering {
        posicion: usize,
        descendente: bool,
    },
    /// Una columna `STATIC`: una por partición, no por fila.
    Estatica,
    Normal,
}

/// El índice que una columna tiene, el más capaz si hay varios.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Indice {
    Ninguno,
    /// Un índice clásico (`COMPOSITES`): solo igualdad (medido en X0).
    Clasico,
    /// `StorageAttachedIndex`: igualdad y rangos, sin `ALLOW FILTERING`.
    Sai,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Columna {
    pub nombre: String,
    pub cql: Cql,
    pub tipo: Tipo,
    pub rol: Rol,
    pub indice: Indice,
}

/// Una fila de `system_schema.columns`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilaColumna {
    pub nombre: String,
    /// `partition_key`, `clustering`, `static` o `regular`.
    pub kind: String,
    pub posicion: i32,
    /// `asc`, `desc` o `none`.
    pub orden: String,
    /// El tipo CQL, en texto.
    pub tipo: String,
}

/// Una fila de `system_schema.indexes`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilaIndice {
    pub nombre: String,
    /// `COMPOSITES`, `KEYS` o `CUSTOM`.
    pub kind: String,
    /// `options['class_name']`, si es `CUSTOM`.
    pub clase: Option<String>,
    /// `options['target']`: la columna, o `values(c)`, `keys(c)`, `entries(c)`.
    pub objetivo: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tabla {
    pub keyspace: String,
    pub nombre: String,
    pub columnas: Vec<Columna>,
    /// Lo que el catálogo dice al lector: una columna que no se dejó tipar, un
    /// índice que no se usa.
    pub avisos: Vec<String>,
}

impl Tabla {
    /// Arma la tabla. `Err` es un `esquema`: `system_schema` dijo algo que no se
    /// entiende, y no se inventa un tipo.
    pub fn de_system_schema(
        keyspace: &str,
        nombre: &str,
        columnas: &[FilaColumna],
        indices: &[FilaIndice],
    ) -> Result<Tabla, String> {
        let mut avisos = Vec::new();
        let mut v = Vec::with_capacity(columnas.len());
        for c in columnas {
            let cql = Cql::parse(&c.tipo)
                .map_err(|e| format!("`{keyspace}.{nombre}.{}`: {e}", c.nombre))?;
            let pos = usize::try_from(c.posicion.max(0)).unwrap_or(0);
            let rol = match c.kind.as_str() {
                "partition_key" => Rol::Particion(pos),
                "clustering" => Rol::Clustering {
                    posicion: pos,
                    descendente: c.orden.eq_ignore_ascii_case("desc"),
                },
                "static" => Rol::Estatica,
                "regular" => Rol::Normal,
                otro => {
                    return Err(format!(
                        "`{keyspace}.{nombre}.{}`: un `kind` que no se conoce (`{otro}`)",
                        c.nombre
                    ));
                }
            };
            v.push(Columna {
                tipo: Tipo::de(&cql),
                nombre: c.nombre.clone(),
                cql,
                rol,
                indice: Indice::Ninguno,
            });
        }
        if !v.iter().any(|c| matches!(c.rol, Rol::Particion(_))) {
            return Err(format!("`{keyspace}.{nombre}` no tiene clave de partición"));
        }
        for i in indices {
            let Some(obj) = i.objetivo.as_deref() else {
                avisos.push(format!(
                    "el índice `{}` no dice su columna: no se usa",
                    i.nombre
                ));
                continue;
            };
            let nivel = match (i.kind.as_str(), i.clase.as_deref()) {
                ("CUSTOM", Some(c)) if c.ends_with("StorageAttachedIndex") => Indice::Sai,
                ("CUSTOM", otra) => {
                    avisos.push(format!(
                        "el índice `{}` es `{}`: no se usa para filtrar",
                        i.nombre,
                        otra.unwrap_or("custom")
                    ));
                    continue;
                }
                ("COMPOSITES" | "KEYS", _) => Indice::Clasico,
                (otro, _) => {
                    avisos.push(format!("el índice `{}` es `{otro}`: no se usa", i.nombre));
                    continue;
                }
            };
            // `values(c)`, `keys(c)`, `entries(c)` indexan el interior de una
            // colección: no son un filtro sobre la columna.
            match v.iter_mut().find(|c| c.nombre == obj) {
                Some(c) => c.indice = c.indice.max(nivel),
                None => avisos.push(format!(
                    "el índice `{}` es sobre `{obj}`, que no es una columna entera: no se usa",
                    i.nombre
                )),
            }
        }
        Ok(Tabla {
            keyspace: keyspace.to_string(),
            nombre: nombre.to_string(),
            columnas: v,
            avisos,
        })
    }

    pub fn columna(&self, nombre: &str) -> Option<&Columna> {
        self.columnas.iter().find(|c| c.nombre == nombre)
    }

    /// Las columnas de la clave de partición, en orden.
    pub fn particion(&self) -> Vec<&Columna> {
        let mut v: Vec<(usize, &Columna)> = self
            .columnas
            .iter()
            .filter_map(|c| match c.rol {
                Rol::Particion(p) => Some((p, c)),
                _ => None,
            })
            .collect();
        v.sort_by_key(|(p, _)| *p);
        v.into_iter().map(|(_, c)| c).collect()
    }

    /// Las columnas de clustering, en orden.
    pub fn clustering(&self) -> Vec<&Columna> {
        let mut v: Vec<(usize, &Columna)> = self
            .columnas
            .iter()
            .filter_map(|c| match c.rol {
                Rol::Clustering { posicion, .. } => Some((posicion, c)),
                _ => None,
            })
            .collect();
        v.sort_by_key(|(p, _)| *p);
        v.into_iter().map(|(_, c)| c).collect()
    }

    /// `primaryKey` del catálogo: partición y luego clustering.
    pub fn clave_primaria(&self) -> Vec<String> {
        self.particion()
            .into_iter()
            .chain(self.clustering())
            .map(|c| c.nombre.clone())
            .collect()
    }

    /// `keyspace.tabla`, citado: el único sitio donde un nombre entra en un CQL.
    pub fn objeto_citado(&self) -> String {
        format!("{}.{}", citar(&self.keyspace), citar(&self.nombre))
    }
}

/// Un identificador de CQL, siempre entre comillas (con las comillas internas
/// duplicadas): un nombre del usuario nunca se lee como sintaxis.
pub fn citar(nombre: &str) -> String {
    format!("\"{}\"", nombre.replace('"', "\"\""))
}

#[cfg(test)]
pub(crate) mod pruebas {
    use super::*;

    pub fn col(n: &str, kind: &str, pos: i32, orden: &str, tipo: &str) -> FilaColumna {
        FilaColumna {
            nombre: n.into(),
            kind: kind.into(),
            posicion: pos,
            orden: orden.into(),
            tipo: tipo.into(),
        }
    }

    /// `pedidos` de la semilla de X0: `((cliente, dia), hora DESC, id)`.
    pub fn pedidos(indices: &[FilaIndice]) -> Tabla {
        Tabla::de_system_schema(
            "ore_prueba",
            "pedidos",
            &[
                col("cliente", "partition_key", 0, "none", "int"),
                col("dia", "partition_key", 1, "none", "date"),
                col("hora", "clustering", 0, "desc", "timestamp"),
                col("id", "clustering", 1, "asc", "timeuuid"),
                col("total", "regular", -1, "none", "decimal"),
                col("estado", "regular", -1, "none", "text"),
                col("nota", "regular", -1, "none", "text"),
            ],
            indices,
        )
        .unwrap()
    }

    pub fn sai(nombre: &str, columna: &str) -> FilaIndice {
        FilaIndice {
            nombre: nombre.into(),
            kind: "CUSTOM".into(),
            clase: Some("StorageAttachedIndex".into()),
            objetivo: Some(columna.into()),
        }
    }

    pub fn clasico(nombre: &str, columna: &str) -> FilaIndice {
        FilaIndice {
            nombre: nombre.into(),
            kind: "COMPOSITES".into(),
            clase: None,
            objetivo: Some(columna.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::pruebas::*;
    use super::*;

    #[test]
    fn la_clave_son_la_particion_y_el_clustering_en_orden() {
        let t = pedidos(&[]);
        assert_eq!(t.clave_primaria(), ["cliente", "dia", "hora", "id"]);
        assert_eq!(t.particion().len(), 2);
        assert_eq!(
            t.columna("hora").unwrap().rol,
            Rol::Clustering {
                posicion: 0,
                descendente: true
            }
        );
        assert_eq!(t.columna("total").unwrap().tipo, Tipo::DecimalLibre);
    }

    #[test]
    fn los_indices_se_leen_y_el_mejor_gana() {
        let t = pedidos(&[
            clasico("a", "estado"),
            sai("b", "estado"),
            clasico("c", "nota"),
        ]);
        assert_eq!(t.columna("estado").unwrap().indice, Indice::Sai);
        assert_eq!(t.columna("nota").unwrap().indice, Indice::Clasico);
        assert_eq!(t.columna("total").unwrap().indice, Indice::Ninguno);
        assert!(t.avisos.is_empty());
    }

    #[test]
    fn un_indice_que_no_filtra_la_columna_entera_se_dice_y_no_se_usa() {
        let sasi = FilaIndice {
            nombre: "viejo".into(),
            kind: "CUSTOM".into(),
            clase: Some("org.apache.cassandra.index.sasi.SASIIndex".into()),
            objetivo: Some("estado".into()),
        };
        let interior = clasico("dentro", "values(estado)");
        let t = pedidos(&[sasi, interior]);
        assert_eq!(t.columna("estado").unwrap().indice, Indice::Ninguno);
        assert_eq!(t.avisos.len(), 2, "{:?}", t.avisos);
    }

    #[test]
    fn el_nombre_nunca_entra_como_sintaxis() {
        assert_eq!(citar("a\"b"), "\"a\"\"b\"");
        assert_eq!(pedidos(&[]).objeto_citado(), "\"ore_prueba\".\"pedidos\"");
    }

    #[test]
    fn lo_que_no_se_entiende_es_un_error() {
        let sin_clave =
            Tabla::de_system_schema("k", "t", &[col("a", "regular", -1, "none", "int")], &[]);
        assert!(sin_clave.unwrap_err().contains("clave de partición"));
        let kind_raro =
            Tabla::de_system_schema("k", "t", &[col("a", "rara", 0, "none", "int")], &[]);
        assert!(kind_raro.unwrap_err().contains("rara"));
        let tipo_roto = Tabla::de_system_schema(
            "k",
            "t",
            &[col("a", "partition_key", 0, "none", "list<")],
            &[],
        );
        assert!(tipo_roto.unwrap_err().contains("k.t.a"));
    }
}
