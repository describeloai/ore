//! `OOS2014` — dos bindings del mismo objeto no pueden reclamar la misma fila.
//!
//! # El agujero que cierra
//!
//! `03-binding` §2.1 decía que una entidad puede tener varios bindings, *«cada
//! uno cubre un subconjunto de sus propiedades»* — el caso de las **columnas**.
//! El caso inverso no estaba escrito: **un objeto físico puede sostener varias
//! entidades**, que es como funciona el diseño de tabla única de DynamoDB.
//!
//! Y sin nada que lo impidiera, dos bindings sobre `app_single_table` con
//! entidades distintas **validaban limpio**. Nada decía qué filas eran de quién,
//! así que un ejecutor que resolviera `Pedido` devolvería también los clics. La
//! figura de siempre: dos bindings que se pisan tienen exactamente el mismo
//! aspecto que dos que reparten.
//!
//! # Por qué esto se puede decidir, y con SQL no se podría
//!
//! Porque la gramática del selector es cerrada: igualdad, pertenencia y
//! ausencia. Cada clave restringe una columna a un **conjunto finito de
//! valores**, así que dos selectores son disjuntos si —y solo si— existe alguna
//! columna que ambos mencionan cuyos conjuntos no se cortan. Es una comprobación
//! de intersección, no un demostrador.
//!
//! Con un `where` opaco esto no se puede ni plantear, y ese es el argumento que
//! sostiene la restricción de §3.5.1: la gramática cerrada no es elegancia, es lo
//! que hace que la afirmación *«estos dos bindings reparten el objeto»* sea
//! demostrable en lugar de creída.
//!
//! # El caso trivial no es un caso aparte
//!
//! Un binding **sin** selector reclama TODAS las filas, así que se solapa con
//! cualquier otro por construcción. No hace falta un código para «falta el
//! selector» y otro para «se solapan»: falta el selector **es** el solape total,
//! y decirlo con dos códigos sugeriría que son dos problemas.

use crate::code::Code;
use crate::diag::Diagnostic;
use crate::document::Kind;
use crate::link::{Loaded, Package};
use std::collections::BTreeMap;

/// Lo que un selector afirma: por cada columna, el conjunto de valores que
/// admite. `None` como valor es la ausencia, y se representa con el literal
/// `null` — un valor más del conjunto, no un caso especial.
type Seleccion = BTreeMap<String, Vec<String>>;

pub fn comprobar(pkg: &Package, out: &mut Vec<Diagnostic>) {
    // Agrupados por el OBJETO, que es (datasource, source). Dos bindings a la
    // misma tabla de datasources distintos no compiten por nada.
    let mut por_objeto: BTreeMap<(String, String), Vec<&Loaded>> = BTreeMap::new();
    for b in pkg.of(Kind::Binding) {
        let campo = |k: &str| b.section(k).and_then(|n| n.as_str()).map(String::from);
        if let (Some(ds), Some(src)) = (campo("datasourceRef"), campo("source")) {
            por_objeto.entry((ds, src)).or_default().push(b);
        }
    }

    for ((ds, src), bindings) in &por_objeto {
        if bindings.len() < 2 {
            continue;
        }
        let sel: Vec<Option<Seleccion>> = bindings.iter().map(|b| leer(b)).collect();

        for i in 0..bindings.len() {
            for j in (i + 1)..bindings.len() {
                if disjuntos(sel[i].as_ref(), sel[j].as_ref()) {
                    continue;
                }
                let (a, b) = (bindings[i], bindings[j]);
                let (na, nb) = (destino(a), destino(b));
                // El mismo objeto y las mismas filas para dos entidades es un
                // error distinto que para la misma entidad partida en columnas,
                // que es legítimo y frecuente.
                if na == nb {
                    continue;
                }
                let motivo = match (sel[i].is_none(), sel[j].is_none()) {
                    (true, true) => "ninguno declara `selector`, así que los dos reclaman \
                                     todas las filas"
                        .to_string(),
                    (true, false) => format!("`{}` no declara `selector`", nombre(a)),
                    (false, true) => format!("`{}` no declara `selector`", nombre(b)),
                    (false, false) => "los dos declaran `selector` y ninguna columna común \
                                       los separa"
                        .to_string(),
                };
                out.push(
                    Diagnostic::new(
                        Code::Oos2014,
                        &b.path,
                        format!(
                            "`{nb}` y `{na}` pueden reclamar la misma fila de `{src}` en `{ds}`"
                        ),
                    )
                    .at(b.section("source").map(|n| n.pos()).unwrap_or(b.root.pos()))
                    .help(format!(
                        "{motivo}. Un objeto físico puede sostener varias entidades —el \
                         diseño de tabla única lo hace siempre—, pero entonces cada binding \
                         tiene que decir qué filas son suyas y los selectores tienen que \
                         repartirlas. Si se pisan, un ejecutor devuelve unas instancias como \
                         si fueran otras, y eso no se ve en el documento"
                    )),
                );
            }
        }
    }
}

fn nombre(b: &Loaded) -> String {
    b.qname().unwrap_or_default()
}

fn destino(b: &Loaded) -> String {
    b.section("targetEntity")
        .and_then(|n| n.as_str())
        .unwrap_or_default()
        .to_string()
}

/// `None` significa **sin selector**: reclama todo.
fn leer(b: &Loaded) -> Option<Seleccion> {
    let s = b.section("selector")?;
    let mut out = Seleccion::new();
    for (k, v) in s.entries() {
        let Some(col) = k.as_str() else { continue };
        let items = v.items();
        let valores: Vec<String> = if items.is_empty() {
            vec![v.as_str().unwrap_or("null").to_string()]
        } else {
            items
                .iter()
                .filter_map(|i| i.as_str().map(String::from))
                .collect()
        };
        if !valores.is_empty() {
            out.insert(col.to_string(), valores);
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Dos selecciones son disjuntas si **alguna** columna que ambas mencionan tiene
/// conjuntos de valores que no se cortan. Basta una: la conjunción hace que una
/// sola discrepancia excluya la fila entera.
///
/// Sin selector no hay ninguna columna que mirar, así que nunca es disjunto — y
/// eso es correcto, no una laguna: quien no dice qué filas son suyas las reclama
/// todas.
fn disjuntos(a: Option<&Seleccion>, b: Option<&Seleccion>) -> bool {
    let (Some(a), Some(b)) = (a, b) else {
        return false;
    };
    a.iter().any(|(col, va)| {
        b.get(col)
            .is_some_and(|vb| !va.iter().any(|x| vb.contains(x)))
    })
}

// ── Comprobaciones ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sel(pares: &[(&str, &[&str])]) -> Option<Seleccion> {
        Some(
            pares
                .iter()
                .map(|(k, v)| (k.to_string(), v.iter().map(|s| s.to_string()).collect()))
                .collect(),
        )
    }

    #[test]
    fn una_columna_que_discrepa_basta_para_repartir() {
        let a = sel(&[("tipo", &["PEDIDO"]), ("region", &["eu"])]);
        let b = sel(&[("tipo", &["CLIC"]), ("region", &["eu"])]);
        assert!(disjuntos(a.as_ref(), b.as_ref()));
    }

    /// La pertenencia es un conjunto, y dos conjuntos que comparten un valor no
    /// reparten nada: la fila `enviado` cae en los dos.
    #[test]
    fn dos_conjuntos_que_se_cortan_no_reparten() {
        let a = sel(&[("estado", &["nuevo", "enviado"])]);
        let b = sel(&[("estado", &["enviado", "anulado"])]);
        assert!(!disjuntos(a.as_ref(), b.as_ref()));
    }

    /// Y si no comparten ninguna columna, no hay nada que los separe. Es el caso
    /// que más se parece a estar bien y no lo está.
    #[test]
    fn sin_columna_comun_no_hay_reparto() {
        let a = sel(&[("tipo", &["PEDIDO"])]);
        let b = sel(&[("clase", &["CLIC"])]);
        assert!(!disjuntos(a.as_ref(), b.as_ref()));
    }

    #[test]
    fn quien_no_declara_selector_lo_reclama_todo() {
        let a = sel(&[("tipo", &["PEDIDO"])]);
        assert!(!disjuntos(a.as_ref(), None));
        assert!(!disjuntos(None, None));
    }
}
