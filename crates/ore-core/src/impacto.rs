//! Qué le pasa a **tu** árbol cuando cambia un vocabulario que importas.
//!
//! `diff` contesta *«qué cambió en el paquete»*, y es una pregunta sobre el
//! paquete. La que alguien se hace de verdad antes de aceptar una actualización
//! es otra:
//!
//! > *«El artículo 9 cambió. ¿Qué se me rompe A MÍ?»*
//!
//! Y no se responde comparando dos vocabularios, porque el efecto no está en el
//! vocabulario: está en la **clasificación efectiva** de las propiedades de uno,
//! que sale de la herencia y la propagación, y en la **cobertura de gobierno**,
//! que es una resta entre lo que se exige y lo que hay. Las dos ya existen y las
//! dos son funciones del árbol entero — no de la dependencia.
//!
//! Así que esto no computa nada nuevo. Corre lo que ya corre el compilador
//! **dos veces** —con el árbol de antes y con el de después— y resta. Que sean
//! las mismas funciones no es economía: es la única forma de que el informe
//! prometa el build que de verdad va a correr. Un informe con su propia
//! semántica sería un segundo compilador que nadie prueba.
//!
//! # Solo las propias
//!
//! Se miran las entidades que **este árbol declara**, no las que llegaron
//! vendorizadas. La distinción importa porque es toda la diferencia entre
//! informar y hacer ruido: que un vocabulario cambie por dentro es un hecho
//! sobre él, no sobre quien lo usa, y quien lee esto no puede hacer nada con eso
//! ni tiene por qué.

use crate::governance;
use crate::link::Package;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// El retrato de una propiedad: lo que la clasifica y lo que le falta.
///
/// `exige` menos `cubre` es exactamente lo que decide `OOS8001`, así que un
/// retrato con esa diferencia no vacía es una propiedad que **hoy no compila**.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Retrato {
    /// Retículo → nivel efectivo.
    pub etiquetas: BTreeMap<String, String>,
    /// Las clases de gobierno que se le exigen.
    pub exige: BTreeSet<String>,
    /// Y las que de hecho la cubren.
    pub cubre: BTreeSet<String>,
}

impl Retrato {
    /// Lo que le falta para compilar.
    pub fn descubierto(&self) -> BTreeSet<String> {
        self.exige.difference(&self.cubre).cloned().collect()
    }
}

/// Un cambio en el árbol de quien consume, no en el paquete que cambió.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cambio {
    /// La clasificación efectiva de una propiedad tuya se mueve.
    ///
    /// `antes` y `despues` son opcionales porque una etiqueta puede **aparecer**
    /// —el vocabulario nuevo clasifica algo que antes no clasificaba nadie— o
    /// **irse**, y las dos son noticia: la primera suele traer una exigencia
    /// detrás, y la segunda deja sin piso a una regla que se apoyaba en ella.
    Clasificacion {
        propiedad: String,
        reticulo: String,
        antes: Option<String>,
        despues: Option<String>,
    },
    /// Pasa a exigir gobierno que no tiene. Es `OOS8001` con fecha futura.
    SinCobertura {
        propiedad: String,
        clases: BTreeSet<String>,
        /// De dónde sale la exigencia, tal y como la nombra el compilador.
        porque: Vec<String>,
    },
    /// Deja de faltarle lo que le faltaba.
    ///
    /// Se dice, y no por simetría estética: quien actualiza para cerrar un hueco
    /// necesita ver que se cerró. Un informe que solo hablara de lo que empeora
    /// obligaría a compilar para saber si mejoró algo.
    ConCobertura {
        propiedad: String,
        clases: BTreeSet<String>,
    },
}

impl Cambio {
    /// La propiedad de la que habla. Es el criterio de orden y el de recuento.
    pub fn propiedad(&self) -> &str {
        match self {
            Cambio::Clasificacion { propiedad, .. }
            | Cambio::SinCobertura { propiedad, .. }
            | Cambio::ConCobertura { propiedad, .. } => propiedad,
        }
    }
}

/// El retrato de cada propiedad **propia** del árbol.
///
/// Las tres piezas salen de donde ya salían: `flow` clasifica, y `governance`
/// exige y cubre. Aquí no se decide nada — se junta.
pub fn retrato(pkg: &Package) -> BTreeMap<String, Retrato> {
    let lat = crate::flow::lattices(pkg);
    let etiquetas = crate::flow::efectivas(pkg, &lat);
    let exige = governance::exigencias(pkg);
    let cubre = governance::cobertura_efectiva(pkg);

    let mias = entidades_propias(pkg);
    let mut out: BTreeMap<String, Retrato> = BTreeMap::new();
    for (prop, ets) in etiquetas {
        if !es_propia(&mias, &prop) {
            continue;
        }
        let r = out.entry(prop.clone()).or_default();
        r.etiquetas = ets;
        if let Some(e) = exige.get(&prop) {
            r.exige = e.clases.iter().map(|c| (*c).to_string()).collect();
        }
        if let Some(c) = cubre.get(&prop) {
            r.cubre = c.iter().map(|c| (*c).to_string()).collect();
        }
    }
    out
}

/// Lo que cambia entre dos estados del **mismo** árbol.
///
/// `antes` y `despues` no son dos versiones de la dependencia: son el árbol de
/// quien consume con una y con la otra. Es la distinción entera de este módulo.
pub fn impacto(antes: &Package, despues: &Package) -> Vec<Cambio> {
    let a = retrato(antes);
    let b = retrato(despues);
    // El porqué se lee una vez. Dentro del bucle era el mismo cómputo del árbol
    // entero por cada propiedad.
    let porques = governance::exigencias(despues);
    let mut out = Vec::new();

    // Las propiedades de los dos lados, para que una que aparece o desaparece
    // no se caiga del informe por no estar en el mapa que se recorre.
    let todas: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
    for prop in todas {
        let vacio = Retrato::default();
        let (x, y) = (a.get(prop).unwrap_or(&vacio), b.get(prop).unwrap_or(&vacio));

        let reticulos: BTreeSet<&String> = x.etiquetas.keys().chain(y.etiquetas.keys()).collect();
        for ret in reticulos {
            let (antes, despues) = (x.etiquetas.get(ret), y.etiquetas.get(ret));
            if antes == despues {
                continue;
            }
            out.push(Cambio::Clasificacion {
                propiedad: prop.clone(),
                reticulo: ret.clone(),
                antes: antes.cloned(),
                despues: despues.cloned(),
            });
        }

        // La cobertura se compara por lo que **falta**, no por lo que se exige:
        // que una propiedad pase a exigir `authorization` y ya la tuviera no le
        // cambia nada a nadie, y decirlo sería una línea que hay que leer para
        // descubrir que no había que hacer nada.
        let (falta_antes, falta_despues) = (x.descubierto(), y.descubierto());
        let nuevas: BTreeSet<String> = falta_despues.difference(&falta_antes).cloned().collect();
        if !nuevas.is_empty() {
            out.push(Cambio::SinCobertura {
                propiedad: prop.clone(),
                clases: nuevas,
                porque: porques.get(prop).map(|e| e.porque.clone()).unwrap_or_default(),
            });
        }
        let cerradas: BTreeSet<String> = falta_antes.difference(&falta_despues).cloned().collect();
        if !cerradas.is_empty() {
            out.push(Cambio::ConCobertura {
                propiedad: prop.clone(),
                clases: cerradas,
            });
        }
    }
    out
}

/// Los nombres cualificados de las entidades que **este árbol declara**.
fn entidades_propias(pkg: &Package) -> BTreeSet<String> {
    pkg.entities()
        .filter(|e| !viene_de_un_oob(&e.path))
        .filter_map(|e| e.qname())
        .collect()
}

/// Una propiedad es propia si lo es la entidad que la declara.
///
/// La clave es `entidad.propiedad` y la entidad lleva su espacio de nombres
/// dentro, así que se parte por el **último** punto: `hr.Employee.nationalId` es
/// `nationalId` de `hr.Employee`, no `Employee.nationalId` de `hr`.
fn es_propia(mias: &BTreeSet<String>, prop: &str) -> bool {
    prop.rsplit_once('.')
        .is_some_and(|(entidad, _)| mias.contains(entidad))
}

/// Si un documento llegó dentro de un `.oob`.
///
/// Se mira la ruta y no una lista de miembros porque el cargador ya dejó la
/// respuesta escrita ahí: un documento importado cuelga de `<el .oob>/…`, que es
/// la misma forma sintética que hace que un paquete importado sea su propio
/// miembro del workspace.
fn viene_de_un_oob(p: &Path) -> bool {
    p.ancestors()
        .any(|a| a.extension().is_some_and(|x| x == "oob"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn una_propiedad_de_un_oob_no_es_tuya() {
        assert!(viene_de_un_oob(Path::new(
            "vendor/gdpr-0.1.0.oob/Property:gdpr%2fdateOfBirth"
        )));
        assert!(!viene_de_un_oob(Path::new("entities/Employee.yaml")));
    }

    /// El espacio de nombres lleva puntos, así que partir por el primero
    /// atribuiría `hr.Employee.nationalId` a una entidad llamada `hr`.
    #[test]
    fn la_entidad_es_todo_menos_el_ultimo_segmento() {
        let mias = BTreeSet::from(["hr.Employee".to_string()]);
        assert!(es_propia(&mias, "hr.Employee.nationalId"));
        assert!(!es_propia(&mias, "hr.Contractor.nationalId"));
    }
}
