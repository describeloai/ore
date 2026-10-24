//! ⭐ **LO QUE LAS PROMESAS PIDEN CONTAR** (ADR 0063 S7c, OOS v1alpha31 `01` §4).
//!
//! La copia trae en su petición `promesas: [{id, metric, column, …}]`
//! (`ore_core::promesas::peticion`): lo que hay que contar, una entrada por
//! promesa. Aquí se cuenta **sobre los lotes que ya pasan** hacia la versión
//! que se va a publicar —la versión entera: al fundir, lo viejo fundido con lo
//! nuevo—, sin leer nada más. Lo que sale va en la respuesta, en `promesas`:
//! `{id: {cuenta: n}}`, o `{id: {error: "…"}}` si no se pudo (en inglés: lo lee
//! el dueño). Sin nada pedido, no se cuenta nada y la respuesta no cambia.
//!
//! S7e: `escribir` (el `write()` de los SDK) cuenta igual sobre los lotes que
//! escribe y lo deja además en el resumen del snapshot (`ore.promesas`); al
//! anexar sólo ve lo que llega ([`Medidor::solo_lo_que_llega`]).
//!
//! | `metric` | Lo que se cuenta |
//! |---|---|
//! | `nullValues` | filas con la columna nula |
//! | `missingValues` | nulas, o con uno de `missingValues` |
//! | `invalidValues` | no nulas fuera de `validValues`, o fuera de `between` |
//! | `duplicateValues` | no nulas cuyo valor ya apareció (exacto, con tope de memoria) |

use arrow_array::{Array, RecordBatch};
use ore_base::json::Json;
use ore_base::parse::Node;
use std::collections::{BTreeSet, HashSet};

/// El tope de memoria de los duplicados, en MiB (`ORE_PROMESAS_MEMORIA`).
const TOPE_MIB: usize = 256;

enum Que {
    Nulos,
    Ausentes(BTreeSet<String>),
    Validos(BTreeSet<String>, Vec<f64>),
    Entre(f64, f64),
    Duplicados(HashSet<String>, usize),
}

struct Pedida {
    id: String,
    columna: String,
    que: Que,
    cuenta: Result<u64, String>,
}

/// Lo que se cuenta en una pasada.
pub struct Medidor {
    pedidas: Vec<Pedida>,
    tope: usize,
}

impl Medidor {
    /// Nada que contar: la pasada de siempre.
    pub fn nada() -> Medidor {
        Medidor {
            pedidas: Vec::new(),
            tope: 0,
        }
    }

    /// De la petición del verbo (`promesas`). Lo que no se entiende se dice
    /// en su cuenta, y no tumba la copia: medir no es copiar.
    pub fn de(peticion: &Node) -> Medidor {
        let tope = std::env::var("ORE_PROMESAS_MEMORIA")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(TOPE_MIB)
            * 1024
            * 1024;
        let pedidas = peticion
            .get("promesas")
            .map(|(_, l)| l.items())
            .unwrap_or(&[])
            .iter()
            .filter_map(|p| {
                let s = |k: &str| p.get(k).and_then(|(_, v)| v.as_str()).map(String::from);
                let lista = |k: &str| -> BTreeSet<String> {
                    p.get(k)
                        .map(|(_, v)| {
                            v.items()
                                .iter()
                                .filter_map(|x| x.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default()
                };
                let id = s("id")?;
                let columna = s("column").unwrap_or_default();
                let mut cuenta = Ok(0);
                let que = match s("metric").as_deref() {
                    Some("nullValues") => Que::Nulos,
                    Some("missingValues") => Que::Ausentes(lista("missingValues")),
                    Some("duplicateValues") => Que::Duplicados(HashSet::new(), 0),
                    Some("invalidValues") => match p.get("between").map(|(_, v)| v.items()) {
                        Some([a, b]) => match (numero(a), numero(b)) {
                            (Some(a), Some(b)) => Que::Entre(a, b),
                            _ => {
                                cuenta = Err("the range is not two numbers".into());
                                Que::Nulos
                            }
                        },
                        _ => {
                            let v = lista("validValues");
                            let n = v.iter().filter_map(|x| x.parse::<f64>().ok()).collect();
                            Que::Validos(v, n)
                        }
                    },
                    otra => {
                        cuenta = Err(format!(
                            "`{}` is not counted by a table refresh",
                            otra.unwrap_or("?")
                        ));
                        Que::Nulos
                    }
                };
                if columna.is_empty() && cuenta.is_ok() {
                    cuenta = Err("the promise names no column".into());
                }
                Some(Pedida {
                    id,
                    columna,
                    que,
                    cuenta,
                })
            })
            .collect();
        Medidor { pedidas, tope }
    }

    pub fn vacio(&self) -> bool {
        self.pedidas.is_empty()
    }

    /// 0063 S7e: lo que pasa por aquí es sólo lo que llega (un `write()` que
    /// anexa), no la versión entera. Lo de cada fila se cuenta igual —sobre
    /// las filas añadidas—, pero un duplicado contra las filas que ya estaban
    /// no se ve: no se sabe, en vez de un «0» que nadie midió.
    pub fn solo_lo_que_llega(&mut self) {
        for p in &mut self.pedidas {
            if matches!(p.que, Que::Duplicados(..)) && p.cuenta.is_ok() {
                p.cuenta = Err(
                    "an append only sees the rows it adds: duplicates against the rows already there are not checked"
                        .into(),
                );
            }
        }
    }

    /// Un lote más de la versión que se va a publicar.
    pub fn lote(&mut self, l: &RecordBatch) {
        use arrow_cast::display::{ArrayFormatter, FormatOptions};
        let tope = self.tope;
        for p in &mut self.pedidas {
            let Ok(cuenta) = &mut p.cuenta else {
                continue;
            };
            let Some(col) = l.column_by_name(&p.columna) else {
                p.cuenta = Err(format!("column `{}` is not in this version", p.columna));
                continue;
            };
            let opciones = FormatOptions::default();
            let texto = |i: usize, f: &ArrayFormatter<'_>| f.value(i).to_string();
            match &mut p.que {
                Que::Nulos => *cuenta += col.null_count() as u64,
                Que::Entre(a, b) => {
                    // Sin esto, `cast` convierte el texto que no es número en
                    // nulos, y un rango sobre texto saldría siempre «0 fuera».
                    let numerica = col.data_type().is_numeric();
                    let n = arrow_cast::cast(col, &arrow_schema::DataType::Float64);
                    let (true, Ok(n)) = (numerica, n) else {
                        p.cuenta = Err(format!(
                            "column `{}` is not numeric: a range does not apply",
                            p.columna
                        ));
                        continue;
                    };
                    let n = n
                        .as_any()
                        .downcast_ref::<arrow_array::Float64Array>()
                        .expect("cast a Float64");
                    *cuenta += n.iter().flatten().filter(|x| *x < *a || *b < *x).count() as u64;
                }
                que => {
                    let Ok(f) = ArrayFormatter::try_new(col.as_ref(), &opciones) else {
                        p.cuenta = Err(format!("column `{}` cannot be read as text", p.columna));
                        continue;
                    };
                    for i in 0..col.len() {
                        let nulo = col.is_null(i);
                        match que {
                            Que::Ausentes(v) => {
                                if nulo || v.contains(&texto(i, &f)) {
                                    *cuenta += 1;
                                }
                            }
                            Que::Validos(v, n) => {
                                if nulo {
                                    continue;
                                }
                                let t = texto(i, &f);
                                let vale = v.contains(&t)
                                    || t.parse::<f64>().is_ok_and(|x| n.contains(&x));
                                if !vale {
                                    *cuenta += 1;
                                }
                            }
                            Que::Duplicados(vistos, bytes) => {
                                if nulo {
                                    continue;
                                }
                                let t = texto(i, &f);
                                let largo = t.len() + 32;
                                if !vistos.insert(t) {
                                    *cuenta += 1;
                                } else {
                                    *bytes += largo;
                                }
                                if *bytes > tope {
                                    break;
                                }
                            }
                            Que::Nulos | Que::Entre(..) => {}
                        }
                    }
                    if let Que::Duplicados(vistos, bytes) = que
                        && *bytes > tope
                    {
                        vistos.clear();
                        p.cuenta = Err(format!(
                            "too many distinct values in `{}` to check in memory ({} MiB)",
                            p.columna,
                            tope / 1024 / 1024
                        ));
                    }
                }
            }
        }
    }

    /// Lo contado, para la respuesta: `{id: {cuenta}}` o `{id: {error}}`.
    pub fn json(&self) -> Option<Json> {
        if self.pedidas.is_empty() {
            return None;
        }
        Some(Json::Obj(
            self.pedidas
                .iter()
                .map(|p| {
                    let v = match &p.cuenta {
                        Ok(n) => Json::obj([("cuenta", Json::Int(*n as i64))]),
                        Err(m) => Json::obj([("error", Json::s(m))]),
                    };
                    (p.id.clone(), v)
                })
                .collect(),
        ))
    }
}

fn numero(n: &Node) -> Option<f64> {
    n.as_str()?.parse::<f64>().ok().filter(|x| x.is_finite())
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use arrow_array::{Float64Array, Int64Array, StringArray};
    use std::sync::Arc;

    fn lote() -> RecordBatch {
        RecordBatch::try_from_iter([
            (
                "id",
                Arc::new(Int64Array::from(vec![
                    Some(1),
                    Some(2),
                    Some(2),
                    None,
                    Some(3),
                ])) as Arc<dyn Array>,
            ),
            (
                "cliente",
                Arc::new(StringArray::from(vec![
                    Some("ana"),
                    None,
                    Some(""),
                    Some("N/A"),
                    Some("bea"),
                ])) as Arc<dyn Array>,
            ),
            (
                "total",
                Arc::new(Float64Array::from(vec![
                    Some(10.0),
                    Some(-1.0),
                    Some(20000.0),
                    None,
                    Some(5.5),
                ])) as Arc<dyn Array>,
            ),
        ])
        .unwrap()
    }

    fn medir(peticion: &str, lotes: usize) -> String {
        let n = ore_base::parse::parse(peticion).unwrap();
        let mut m = Medidor::de(&n);
        for _ in 0..lotes {
            m.lote(&lote());
        }
        m.json().map(|j| j.jcs()).unwrap_or_default()
    }

    #[test]
    fn cuenta_lo_pedido_y_solo_eso() {
        assert_eq!(medir(r#"{"dataset":"x"}"#, 1), "");
        let j = medir(
            r#"{"promesas":[
              {"id":"nulos","metric":"nullValues","column":"cliente"},
              {"id":"ausentes","metric":"missingValues","column":"cliente","missingValues":["","N/A"]},
              {"id":"validos","metric":"invalidValues","column":"cliente","validValues":["ana","bea"]},
              {"id":"rango","metric":"invalidValues","column":"total","between":["0","10000"]},
              {"id":"enteros","metric":"invalidValues","column":"id","validValues":["1","2"]},
              {"id":"unico","metric":"duplicateValues","column":"id"},
              {"id":"falta","metric":"nullValues","column":"nada"},
              {"id":"texto","metric":"invalidValues","column":"cliente","between":["0","1"]}
            ]}"#,
            1,
        );
        assert_eq!(
            j,
            r#"{"ausentes":{"cuenta":3},"enteros":{"cuenta":1},"falta":{"error":"column `nada` is not in this version"},"nulos":{"cuenta":1},"rango":{"cuenta":2},"texto":{"error":"column `cliente` is not numeric: a range does not apply"},"unico":{"cuenta":1},"validos":{"cuenta":2}}"#
        );
        // Dos lotes: todo dobla, y los duplicados también ven lo del primero.
        let j = medir(
            r#"{"promesas":[{"id":"unico","metric":"duplicateValues","column":"id"},{"id":"nulos","metric":"nullValues","column":"cliente"}]}"#,
            2,
        );
        assert_eq!(j, r#"{"nulos":{"cuenta":2},"unico":{"cuenta":5}}"#);
    }

    #[test]
    fn los_duplicados_con_tope_son_un_error_y_no_un_aprobado() {
        let n = ore_base::parse::parse(
            r#"{"promesas":[{"id":"unico","metric":"duplicateValues","column":"cliente"}]}"#,
        )
        .unwrap();
        let mut m = Medidor::de(&n);
        m.tope = 40;
        m.lote(&lote());
        assert!(
            m.json().unwrap().jcs().contains("too many distinct values"),
            "{}",
            m.json().unwrap().jcs()
        );
    }
}
