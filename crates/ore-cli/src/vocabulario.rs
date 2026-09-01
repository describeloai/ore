//! Los conceptos que ya existen, y los retículos con los que se clasifican.
//!
//! # Por qué esto no vive en el inductor
//!
//! El inductor es puro: recibe un catálogo y devuelve documentos. Buscar
//! conceptos publicados es **mirar el disco**, así que la lectura vive aquí y lo
//! que el inductor recibe es el resultado. La frontera es la misma que la del
//! lector: quien sabe **dónde** está algo no es quien decide **qué** significa.
//!
//! # Y por qué se lee el repositorio entero y no el paquete
//!
//! Un vocabulario publicado es un paquete **sin entidades** que otros importan —
//! la forma está probada en `v1alpha4/valid/vocabulary-package-has-no-entities`—.
//! Si esto solo mirase el paquete que se está induciendo, la única respuesta
//! posible a *«¿el mismo concepto?»* sería acuñar uno nuevo, que es exactamente
//! la inflación que `02-property` §6.2 nombra: cuatro mil columnas dando cuatro
//! mil conceptos es igual que no tener vocabulario.
//!
//! # Qué se ofrece, y en qué orden
//!
//! Un candidato es un concepto **del mismo tipo** —`is` no redeclara el tipo: lo
//! toma del concepto, así que apuntar a uno de otro tipo retiparía la columna en
//! silencio— y se ordena por cómo de cerca está de lo que se pregunta: primero
//! el que se llama igual, luego el que la nombra entre sus `synonyms`, luego el
//! resto. Los sinónimos son **para esto**: son lo que alguien escribió sabiendo
//! cómo se dice ese concepto ahí fuera.

use ore_core::link::{Loaded, Package};
use std::path::Path;

/// Un concepto publicado, tal y como lo ve quien tiene que elegirlo.
pub struct Concepto {
    /// `gdpr.personalEmail`. Es lo que se escribe en un `is`.
    pub qname: String,
    pub tipo: String,
    /// Las clasificaciones que lleva puestas, ya formateadas para enseñarlas.
    /// Un concepto **sin ninguna** no gobierna nada, y eso hay que poder verlo
    /// antes de elegirlo.
    pub etiquetas: Vec<String>,
    /// Cómo se dice esto ahí fuera. Es el campo que convierte la séptima
    /// pregunta en «¿es alguno de estos?»: sin sinónimos, `email` y
    /// `correo_electronico` son dos conceptos distintos para siempre.
    pub sinonimos: Vec<String>,
}

/// Un retículo del repositorio: el eje y sus niveles.
pub struct Reticulo {
    pub qname: String,
    pub niveles: Vec<String>,
}

#[derive(Default)]
pub struct Vocabulario {
    pub conceptos: Vec<Concepto>,
    pub reticulos: Vec<Reticulo>,
}

impl Vocabulario {
    /// Lee el vocabulario de un repositorio ontológico.
    ///
    /// Se apoya en el cargador de `ore-core` en vez de analizar YAML aquí: un
    /// segundo lector de documentos OOS al lado del que ya existe sería un
    /// segundo sitio donde envejecer. Los diagnósticos se ignoran a propósito —
    /// lo inducido está en `DRAFT` y **no compila todavía**, y aun así sus
    /// conceptos ya acuñados son candidatos legítimos.
    pub fn leer(raiz: &Path) -> Vocabulario {
        let (pkg, _) = ore_core::validate::cargar_paquete(raiz);
        Vocabulario {
            conceptos: conceptos(&pkg),
            reticulos: reticulos(&pkg),
        }
    }

    pub fn de(&self, qname: &str) -> Option<&Concepto> {
        self.conceptos.iter().find(|c| c.qname == qname)
    }

    /// Los conceptos que podrían ser lo que esa columna es, mejor primero.
    pub fn candidatos(&self, columna: &str, tipo: &str) -> Vec<&Concepto> {
        let col = columna.to_ascii_lowercase();
        let mut out: Vec<(u8, &Concepto)> = self
            .conceptos
            .iter()
            // El tipo tiene que coincidir. `is` no redeclara el tipo —el esquema
            // lo prohíbe con un `oneOf`—, así que apuntar a un concepto de otro
            // tipo no es un error de estilo: retipa la columna sin decirlo.
            .filter(|c| c.tipo == tipo)
            .map(|c| {
                let corto = c.qname.rsplit('.').next().unwrap_or(&c.qname);
                let cerca = if corto.eq_ignore_ascii_case(&col) {
                    0
                } else if c.sinonimos.iter().any(|s| s.eq_ignore_ascii_case(&col)) {
                    1
                } else {
                    2
                };
                (cerca, c)
            })
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.qname.cmp(&b.1.qname)));
        out.into_iter().map(|(_, c)| c).collect()
    }

    /// Los retículos con los que se puede clasificar un concepto.
    ///
    /// `oos.maturity` no está, y no es un olvido: es el ciclo de vida de un
    /// **documento**, no la sensibilidad de un **dato**. Ofrecerlo aquí invitaría
    /// a clasificar un concepto por lo maduro que está el fichero que lo declara.
    pub fn ejes(&self) -> impl Iterator<Item = &Reticulo> {
        self.reticulos.iter().filter(|r| r.qname != "oos.maturity")
    }
}

fn conceptos(pkg: &Package) -> Vec<Concepto> {
    pkg.docs
        .iter()
        .filter(|d| d.kind == ore_core::document::Kind::Property)
        .filter_map(|d| {
            Some(Concepto {
                qname: d.qname()?,
                tipo: d.section("type")?.as_str()?.to_string(),
                etiquetas: etiquetas(d),
                sinonimos: d
                    .section("aiContext")
                    .and_then(|a| a.get("synonyms").map(|(_, v)| v))
                    .map(|v| {
                        v.items()
                            .iter()
                            .filter_map(|i| i.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
            })
        })
        .collect()
}

fn etiquetas(d: &Loaded) -> Vec<String> {
    d.section("labels")
        .map(|n| {
            n.entries()
                .iter()
                .filter_map(|(k, v)| Some(format!("{}: {}", k.as_str()?, v.as_str()?)))
                .collect()
        })
        .unwrap_or_default()
}

fn reticulos(pkg: &Package) -> Vec<Reticulo> {
    ore_core::flow::lattices(pkg)
        .into_iter()
        .map(|(qname, l)| Reticulo {
            qname,
            niveles: l.levels,
        })
        .collect()
}
