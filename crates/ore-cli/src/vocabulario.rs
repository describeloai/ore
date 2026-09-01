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
//! silencio— y **tiene que parecerse a lo que se pregunta**: se llama igual, la
//! nombra entre sus `synonyms`, o uno contiene al otro. Los sinónimos son **para
//! esto**: son lo que alguien escribió sabiendo cómo se dice ese concepto ahí
//! fuera.
//!
//! **No hay un cuarto grupo con el resto**, y esa ausencia se ganó midiendo
//! contra un dataset de verdad: ofrecer once conceptos irrelevantes a una columna
//! que no se parece a ninguno invita a una respuesta equivocada justo donde
//! clasifica mal un dato para siempre. Está desarrollado en `candidatos`.

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
    ///
    /// # Tres escalones de parecido, y **ninguno es «el resto»**
    ///
    /// Hubo un cuarto grupo —*todo lo demás del mismo tipo*— y se retiró al
    /// medirlo contra un dataset de verdad: a la columna `cod_pais` le ofrecía
    /// `gdpr.healthCondition`, `gdpr.nationalId` y nueve más, porque todos son
    /// `String` y ninguno casaba, así que caían al resto y se mostraban enteros.
    ///
    /// > **Ofrecer once conceptos irrelevantes es peor que no ofrecer ninguno.**
    ///
    /// No es ruido inofensivo: invita a una respuesta equivocada justo donde una
    /// respuesta equivocada clasifica mal un dato para siempre. Y la lista vacía
    /// no deja a nadie sin salida — es lo que convierte la pregunta en *«¿lo
    /// acuñamos?»*, que es la que de verdad tocaba.
    ///
    /// El tercer escalón —**contención**— entró porque quitar el resto se llevó
    /// por delante un parecido de los buenos: `email` contra `gdpr.personalEmail`
    /// no casa por nombre ni figura como sinónimo, y cualquiera diría que es. La
    /// longitud mínima es lo que impide que vuelva el ruido por esta puerta: sin
    /// ella `n` estaría contenida en medio vocabulario.
    pub fn candidatos(&self, columna: &str, tipo: &str) -> Vec<&Concepto> {
        /// Sin `_` y en minúsculas: `cod_pais` y `codPais` son el mismo nombre
        /// escrito por dos personas.
        fn plano(s: &str) -> String {
            s.to_ascii_lowercase().replace('_', "")
        }
        /// Por debajo de esto, una contención no es un parecido: es una letra
        /// que aparece en todas partes.
        const MINIMO: usize = 4;

        let col = plano(columna);
        let mut out: Vec<(u8, &Concepto)> = self
            .conceptos
            .iter()
            // El tipo tiene que coincidir. `is` no redeclara el tipo —el esquema
            // lo prohíbe con un `oneOf`—, así que apuntar a un concepto de otro
            // tipo no es un error de estilo: retipa la columna sin decirlo.
            .filter(|c| c.tipo == tipo)
            .filter_map(|c| {
                let corto = plano(c.qname.rsplit('.').next().unwrap_or(&c.qname));
                if corto == col {
                    Some((0, c))
                } else if c.sinonimos.iter().any(|s| plano(s) == col) {
                    Some((1, c))
                } else if col.len() >= MINIMO && (corto.contains(&col) || col.contains(&corto)) {
                    Some((2, c))
                } else {
                    None
                }
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
        .filter(|d| d.kind == ore_core::document::Kind::Concept)
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

// ── Comprobaciones ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// El vocabulario del RGPD, con los sinónimos que trae de verdad.
    fn gdpr() -> Vocabulario {
        let c = |q: &str, t: &str, sin: &[&str]| Concepto {
            qname: q.into(),
            tipo: t.into(),
            etiquetas: vec!["gdpr.sensitivity: high".into()],
            sinonimos: sin.iter().map(|s| (*s).to_string()).collect(),
        };
        Vocabulario {
            conceptos: vec![
                c("gdpr.personalEmail", "String", &["correo", "mail"]),
                c("gdpr.fullName", "String", &["nombre", "nom", "name"]),
                c("gdpr.nationalId", "String", &["dni", "nif", "nie"]),
                c("gdpr.healthCondition", "String", &["diagnostico"]),
                c("gdpr.postalAddress", "String", &["direccion", "calle"]),
            ],
            reticulos: Vec::new(),
        }
    }

    fn nombres(v: &Vocabulario, columna: &str) -> Vec<String> {
        v.candidatos(columna, "String")
            .iter()
            .map(|c| c.qname.clone())
            .collect()
    }

    /// **Lo que se midió contra un dataset de verdad, y es la razón de esta
    /// función.**
    ///
    /// `cod_pais` no tiene nada que ver con una condición de salud. La versión
    /// anterior le ofrecía los cinco, porque todos son `String` y ninguno casaba:
    /// caían al «resto» y se mostraban enteros. Ofrecer irrelevantes es peor que
    /// no ofrecer nada — invita a una respuesta equivocada justo donde clasifica
    /// mal un dato para siempre.
    #[test]
    fn una_columna_sin_parecido_no_ofrece_nada() {
        assert!(
            nombres(&gdpr(), "cod_pais").is_empty(),
            "{:?}",
            nombres(&gdpr(), "cod_pais")
        );
        assert!(nombres(&gdpr(), "total").is_empty());
    }

    /// Y el sinónimo es lo que hace útil al vocabulario: `nif` y `nom` no se
    /// parecen a nada de lo que el concepto se llama.
    #[test]
    fn el_sinonimo_encuentra_lo_que_el_nombre_no() {
        assert_eq!(nombres(&gdpr(), "nif"), ["gdpr.nationalId"]);
        assert_eq!(nombres(&gdpr(), "nom"), ["gdpr.fullName"]);
    }

    /// La contención existe porque quitar el «resto» se llevó por delante un
    /// parecido de los buenos: `email` no es el nombre corto de
    /// `gdpr.personalEmail` ni figura entre sus sinónimos, y cualquiera diría
    /// que es.
    #[test]
    fn la_contencion_recupera_el_parecido_evidente() {
        assert_eq!(nombres(&gdpr(), "email"), ["gdpr.personalEmail"]);
    }

    /// Y la longitud mínima es lo que impide que el ruido vuelva por esa puerta.
    /// Sin ella, `nom` estaría contenida en medio vocabulario por accidente.
    #[test]
    fn una_columna_demasiado_corta_no_contiene_a_nadie() {
        let v = Vocabulario {
            conceptos: vec![Concepto {
                qname: "gdpr.nationalId".into(),
                tipo: "String".into(),
                etiquetas: Vec::new(),
                sinonimos: Vec::new(),
            }],
            reticulos: Vec::new(),
        };
        // `nal` está dentro de `nationalId` y no significa nada.
        assert!(nombres(&v, "nal").is_empty());
    }

    /// El tipo manda por encima del parecido: `is` no redeclara el tipo, así que
    /// apuntar a un concepto de otro retipa la columna sin decirlo.
    #[test]
    fn el_parecido_no_salta_por_encima_del_tipo() {
        assert!(gdpr().candidatos("email", "Integer").is_empty());
    }
}
