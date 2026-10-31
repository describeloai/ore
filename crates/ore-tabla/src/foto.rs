//! **La foto** (0081 §7.5.3, decisión 2): lo que una consulta ve, leído una vez y de una vez. Una sentencia ve
//! una sola foto del catálogo para todas sus tablas; la siguiente lee otra.
//!
//! Sale de los punteros (`ore_core::punteros::Punteros::todos`): sobre el catálogo de Postgres es una sentencia
//! (`vista_entera`), y Postgres garantiza que una sentencia ve una sola foto; sobre ficheros, un directorio.

use ore_core::punteros::{Punteros, partes};
use ore_runtime::Fallo;
use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::Arc;

/// `base.schema.nombre`, entero.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Nombre {
    pub base: String,
    pub schema: String,
    pub nombre: String,
}

impl Nombre {
    pub fn new(base: &str, schema: &str, nombre: &str) -> Nombre {
        Nombre {
            base: base.into(),
            schema: schema.into(),
            nombre: nombre.into(),
        }
    }

    /// De la forma corta de un puntero (`b.n` o `b.s.n`).
    pub fn de_corto(corto: &str) -> Option<Nombre> {
        partes(corto).map(|(b, s, n)| Nombre::new(b, s, n))
    }
}

impl std::fmt::Display for Nombre {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.base, self.schema, self.nombre)
    }
}

/// Lo que hay en un nombre.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Objeto {
    /// Un dataset del lago: su `metadata_location`, el snapshot que la foto fija.
    Dataset { metadata_location: String },
    /// Una vista: su SQL, en el dialecto de DataFusion (`dialect: datafusion`).
    Vista { sql: String },
}

/// **La foto**: cada nombre y lo que hay en él.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Foto {
    pub objetos: BTreeMap<Nombre, Objeto>,
}

impl Foto {
    pub fn con(mut self, nombre: Nombre, objeto: Objeto) -> Foto {
        self.objetos.insert(nombre, objeto);
        self
    }

    /// Las bases que la foto nombra.
    pub fn bases(&self) -> Vec<String> {
        let mut b: Vec<String> = self.objetos.keys().map(|n| n.base.clone()).collect();
        b.dedup();
        b
    }
}

/// **De dónde sale la foto.** Es síncrono: el catálogo de Postgres y los ficheros lo son, y quien la pide la
/// lee en un hilo de bloqueo.
pub trait Fuente: Send + Sync + Debug {
    fn foto(&self) -> Result<Foto, Fallo>;
}

/// **La foto de los punteros**: los datasets con su `metadata_location`. Lo que no la tiene (un puntero a
/// medio crear, uno de una vista vieja) no es una tabla que leer.
pub struct DePunteros<P> {
    punteros: Arc<P>,
}

impl<P> DePunteros<P> {
    pub fn new(punteros: Arc<P>) -> Self {
        DePunteros { punteros }
    }
}

impl<P> Debug for DePunteros<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DePunteros")
    }
}

impl<P: Punteros + Send + Sync> Fuente for DePunteros<P> {
    fn foto(&self) -> Result<Foto, Fallo> {
        let todos = self
            .punteros
            .todos()
            .map_err(|e| Fallo::Catalogo(format!("los punteros: {e:?}")))?;
        let mut foto = Foto::default();
        for (corto, leido) in todos {
            let Some(nombre) = Nombre::de_corto(&corto) else {
                continue;
            };
            let ml = leido
                .puntero
                .get("metadata_location")
                .and_then(|(_, v)| v.as_str())
                .filter(|s| !s.is_empty() && *s != "null");
            if let Some(ml) = ml {
                foto.objetos.insert(
                    nombre,
                    Objeto::Dataset {
                        metadata_location: ml.to_string(),
                    },
                );
            }
        }
        Ok(foto)
    }
}
