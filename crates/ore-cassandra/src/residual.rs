//! **Lo que el servidor no sabe filtrar, lo filtra el conector** (ficha 0070 §4.4).
//!
//! La spec (`v1alpha24` §3) manda que lo que no se empuja lo evalúe el motor y
//! que una lectura no devuelva más filas de las pedidas sin decirlo. Con Cassandra
//! casi todo filtro cae aquí (`!=`, `IS NULL`, `LIKE`, una columna sin índice…),
//! y el conector, que ya tiene la fila delante, lo evalúa **sin `ALLOW FILTERING`**:
//! el servidor no hace trabajo de más, y lo que llega es lo pedido.
//!
//! La semántica es la de SQL, que es la del motor que recibe las filas:
//!
//! - **Un nulo no casa con nada**, ni con `<>` ni con `NOT IN`: la comparación es
//!   desconocida y la fila se descarta. Solo `isNull` lo ve.
//! - Se compara lo que el motor compara: el **tipo de OOS** de la columna. Un `uuid`,
//!   un `varint` o un `inet` son `String` y se comparan como texto.
//! - `NaN` es mayor que todo y es igual a sí mismo, como en DuckDB.
//! - `like`: `%` es cualquier cosa y `_` un carácter; no hay escape.
//!
//! Función pura sobre [`ore_core::tipos::Valor`]: quien lee las filas del driver
//! convierte cada celda a su `Valor` (`None` es nulo) y llama a [`Residual::cumple`].

use crate::esquema::Tabla;
use crate::traductor::{Literal, Plan, literal};
use ore_core::tipos::Valor;
use ore_driver::{Fallo, Filtro};
use std::cmp::Ordering;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Eq,
    Neq,
    Lt,
    Le,
    Gt,
    Ge,
    En,
    Como,
    EsNulo,
    NoEsNulo,
}

#[derive(Debug, Clone)]
enum Lit {
    Uno(Valor),
    Lista(Vec<Valor>),
    Patron(Vec<char>),
    Ninguno,
}

#[derive(Debug, Clone)]
struct Cond {
    columna: String,
    op: Op,
    lit: Lit,
}

/// Los filtros de un [`Plan`] que evalúa el conector, ya analizados una vez.
#[derive(Debug, Clone, Default)]
pub struct Residual {
    condiciones: Vec<Cond>,
    /// Un recorte por clave que no es la de partición: `(columnas, claves)`.
    claves: Option<(Vec<String>, Vec<Vec<Valor>>)>,
}

impl Residual {
    /// Analiza los literales del plan. `Err` es `operador`: el mismo que el
    /// traductor habría dado, pero a estas alturas ya no puede ocurrir salvo que
    /// el plan y la tabla no sean de la misma lectura.
    pub fn de(plan: &Plan, t: &Tabla) -> Result<Residual, Fallo> {
        let mut condiciones = Vec::with_capacity(plan.residual.len());
        for f in &plan.residual {
            condiciones.push(cond(f, t)?);
        }
        let claves = match &plan.claves {
            None => None,
            Some((cols, filas)) => {
                let mut v = Vec::with_capacity(filas.len());
                for fila in filas {
                    let mut k = Vec::with_capacity(cols.len());
                    for (c, texto) in cols.iter().zip(fila) {
                        let col = t.columna(c).ok_or_else(|| {
                            Fallo::new(
                                ore_driver::Codigo::Esquema,
                                format!("la columna de clave `{c}` ya no está"),
                            )
                        })?;
                        k.push(col.tipo.fisico().analizar(texto).ok_or_else(|| {
                            Fallo::operador(format!(
                                "la clave `{c}`: `{texto}` no es un valor de {}",
                                col.tipo.oos()
                            ))
                        })?);
                    }
                    v.push(k);
                }
                Some((cols.clone(), v))
            }
        };
        Ok(Residual {
            condiciones,
            claves,
        })
    }

    /// Si no hay nada que evaluar (la lectura va directa).
    pub fn vacio(&self) -> bool {
        self.condiciones.is_empty() && self.claves.is_none()
    }

    /// Las columnas que mira: tienen que estar en el `SELECT`.
    pub fn columnas(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self
            .condiciones
            .iter()
            .map(|c| c.columna.as_str())
            .collect();
        if let Some((cols, _)) = &self.claves {
            v.extend(cols.iter().map(String::as_str));
        }
        v.sort_unstable();
        v.dedup();
        v
    }

    /// Si la fila cumple **todo**. `celda(columna)` es el valor de esa columna en
    /// la fila, o `None` si es nulo.
    pub fn cumple<'a>(&self, celda: impl Fn(&str) -> Option<&'a Valor>) -> bool {
        if let Some((cols, claves)) = &self.claves {
            let fila: Option<Vec<&Valor>> = cols.iter().map(|c| celda(c)).collect();
            let Some(fila) = fila else { return false };
            if !claves
                .iter()
                .any(|k| k.iter().zip(&fila).all(|(a, b)| a == *b))
            {
                return false;
            }
        }
        self.condiciones
            .iter()
            .all(|c| evalua(c, celda(&c.columna)))
    }
}

fn cond(f: &Filtro, t: &Tabla) -> Result<Cond, Fallo> {
    let op = match f.operador.as_str() {
        "eq" => Op::Eq,
        "neq" => Op::Neq,
        "lt" => Op::Lt,
        "le" => Op::Le,
        "gt" => Op::Gt,
        "ge" => Op::Ge,
        "in" => Op::En,
        "like" => Op::Como,
        "isNull" => Op::EsNulo,
        "isNotNull" => Op::NoEsNulo,
        otro => return Err(Fallo::operador(format!("`{otro}` no es un operador"))),
    };
    let lit = match literal(f, t)? {
        Literal::Ninguno => Lit::Ninguno,
        Literal::Lista(l) => Lit::Lista(l),
        Literal::Uno(Valor::Texto(p)) if op == Op::Como => Lit::Patron(p.chars().collect()),
        Literal::Uno(v) => Lit::Uno(v),
    };
    Ok(Cond {
        columna: f.columna.clone(),
        op,
        lit,
    })
}

fn evalua(c: &Cond, v: Option<&Valor>) -> bool {
    // Un nulo solo lo ven `isNull` e `isNotNull`.
    match c.op {
        Op::EsNulo => return v.is_none(),
        Op::NoEsNulo => return v.is_some(),
        _ => {}
    }
    let Some(v) = v else { return false };
    match (&c.op, &c.lit) {
        (Op::En, Lit::Lista(l)) => l.iter().any(|x| comparar(v, x) == Some(Ordering::Equal)),
        (Op::Como, Lit::Patron(p)) => match v {
            Valor::Texto(s) => como(&s.chars().collect::<Vec<_>>(), p),
            _ => false,
        },
        (op, Lit::Uno(x)) => match (op, comparar(v, x)) {
            (_, None) => false,
            (Op::Eq, Some(o)) => o == Ordering::Equal,
            (Op::Neq, Some(o)) => o != Ordering::Equal,
            (Op::Lt, Some(o)) => o == Ordering::Less,
            (Op::Le, Some(o)) => o != Ordering::Greater,
            (Op::Gt, Some(o)) => o == Ordering::Greater,
            (Op::Ge, Some(o)) => o != Ordering::Less,
            _ => false,
        },
        _ => false,
    }
}

/// Compara dos valores del mismo tipo; `None` si no son comparables.
fn comparar(a: &Valor, b: &Valor) -> Option<Ordering> {
    Some(match (a, b) {
        (Valor::Texto(x), Valor::Texto(y)) => x.as_bytes().cmp(y.as_bytes()),
        (Valor::Entero(x), Valor::Entero(y)) => x.cmp(y),
        (Valor::Real(x), Valor::Real(y)) => real(*x, *y),
        // Un entero que se compara con un real, como hace SQL.
        (Valor::Entero(x), Valor::Real(y)) => real(*x as f64, *y),
        (Valor::Real(x), Valor::Entero(y)) => real(*x, *y as f64),
        (Valor::Logico(x), Valor::Logico(y)) => x.cmp(y),
        (Valor::Decimal(x), Valor::Decimal(y)) => x.cmp(y),
        (Valor::Fecha(x), Valor::Fecha(y)) => x.cmp(y),
        (Valor::Hora(x), Valor::Hora(y)) => x.cmp(y),
        (Valor::FechaHora(x), Valor::FechaHora(y)) => x.cmp(y),
        (Valor::Instante(x), Valor::Instante(y)) => x.cmp(y),
        _ => return None,
    })
}

/// `NaN` es mayor que todo y es igual a sí mismo.
fn real(a: f64, b: f64) -> Ordering {
    match (a.is_nan(), b.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        _ => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
    }
}

/// `LIKE` sin escape: `%` es cualquier secuencia (también vacía) y `_` un carácter.
fn como(texto: &[char], patron: &[char]) -> bool {
    let (mut t, mut p) = (0, 0);
    let (mut vuelta_p, mut vuelta_t) = (usize::MAX, 0);
    while t < texto.len() {
        if p < patron.len() && patron[p] == '%' {
            vuelta_p = p;
            vuelta_t = t;
            p += 1;
        } else if p < patron.len() && (patron[p] == '_' || patron[p] == texto[t]) {
            p += 1;
            t += 1;
        } else if vuelta_p != usize::MAX {
            vuelta_t += 1;
            t = vuelta_t;
            p = vuelta_p + 1;
        } else {
            return false;
        }
    }
    patron[p..].iter().all(|c| *c == '%')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::esquema::pruebas::pedidos;
    use crate::traductor::{Opciones, traducir};
    use ore_driver::{Peticion, Valor as V};
    use std::collections::HashMap;

    /// Compila los filtros por el camino de verdad: traductor → plan → residual.
    fn res(filtros: Vec<Filtro>) -> Residual {
        let t = pedidos(&[]);
        let p = Peticion {
            objeto: "ore_prueba.pedidos".into(),
            proyeccion: vec![("total".into(), "total".into())],
            filtros,
            ..Default::default()
        };
        let plan = traducir(&p, &t, &Opciones::default()).unwrap().plan;
        Residual::de(&plan, &t).unwrap()
    }

    fn fila(celdas: &[(&str, Option<Valor>)]) -> HashMap<String, Option<Valor>> {
        celdas
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    fn pasa(r: &Residual, f: &HashMap<String, Option<Valor>>) -> bool {
        r.cumple(|c| f.get(c).and_then(|v| v.as_ref()))
    }

    fn texto(s: &str) -> Option<Valor> {
        Some(Valor::Texto(s.into()))
    }

    #[test]
    fn un_nulo_no_casa_con_nada_salvo_is_null() {
        let nulo = fila(&[("estado", None)]);
        let hay = fila(&[("estado", texto("a"))]);
        for (op, v) in [("eq", "a"), ("neq", "a"), ("lt", "z"), ("ge", "a")] {
            let r = res(vec![Filtro::uno("estado", op, v)]);
            assert!(!pasa(&r, &nulo), "{op} no puede casar un nulo");
        }
        let es_nulo = res(vec![Filtro {
            columna: "estado".into(),
            operador: "isNull".into(),
            valor: V::Ninguno,
        }]);
        let no_es_nulo = res(vec![Filtro {
            columna: "estado".into(),
            operador: "isNotNull".into(),
            valor: V::Ninguno,
        }]);
        assert!(pasa(&es_nulo, &nulo) && !pasa(&es_nulo, &hay));
        assert!(!pasa(&no_es_nulo, &nulo) && pasa(&no_es_nulo, &hay));
    }

    #[test]
    fn neq_deja_fuera_el_nulo_y_el_igual() {
        let r = res(vec![Filtro::uno("estado", "neq", "a")]);
        assert!(pasa(&r, &fila(&[("estado", texto("b"))])));
        assert!(!pasa(&r, &fila(&[("estado", texto("a"))])));
        assert!(!pasa(&r, &fila(&[("estado", None)])));
    }

    #[test]
    fn in_y_los_rangos_de_texto_van_por_bytes() {
        let r = res(vec![Filtro {
            columna: "estado".into(),
            operador: "in".into(),
            valor: V::Lista(vec!["a".into(), "c".into()]),
        }]);
        assert!(pasa(&r, &fila(&[("estado", texto("c"))])));
        assert!(!pasa(&r, &fila(&[("estado", texto("b"))])));
        // `Z` (0x5A) < `a` (0x61): como el motor, sin mayúsculas ni locale.
        let r = res(vec![Filtro::uno("estado", "lt", "a")]);
        assert!(pasa(&r, &fila(&[("estado", texto("Z"))])));
        assert!(!pasa(&r, &fila(&[("estado", texto("a"))])));
    }

    #[test]
    fn like_sin_escape() {
        for (patron, texto_, casa) in [
            ("a%", "abc", true),
            ("a%", "ba", false),
            ("%c", "abc", true),
            ("a_c", "abc", true),
            ("a_c", "ac", false),
            ("%b%", "abc", true),
            ("%", "", true),
            ("_", "", false),
            ("a%b%c", "aXXbYYc", true),
            ("a%b%c", "aXXbYY", false),
            ("100%", "100", true),
            ("a.c", "abc", false), // el punto no es un comodín
            ("%ñ_", "añb", true),
        ] {
            let r = res(vec![Filtro::uno("nota", "like", patron)]);
            assert_eq!(
                pasa(&r, &fila(&[("nota", texto(texto_))])),
                casa,
                "`{texto_}` LIKE `{patron}`"
            );
        }
    }

    #[test]
    fn los_decimales_son_texto_porque_en_oos_son_string() {
        // `total` es un `decimal` de CQL → `String` de OOS: el motor lo compara como texto.
        let r = res(vec![Filtro::uno("total", "eq", "10.50")]);
        assert!(pasa(&r, &fila(&[("total", texto("10.50"))])));
        assert!(!pasa(&r, &fila(&[("total", texto("10.5"))])));
    }

    #[test]
    fn varias_condiciones_son_una_conjuncion() {
        let r = res(vec![
            Filtro::uno("estado", "neq", "a"),
            Filtro::uno("nota", "like", "x%"),
        ]);
        assert!(pasa(
            &r,
            &fila(&[("estado", texto("b")), ("nota", texto("xy"))])
        ));
        assert!(!pasa(
            &r,
            &fila(&[("estado", texto("a")), ("nota", texto("xy"))])
        ));
        assert!(!pasa(
            &r,
            &fila(&[("estado", texto("b")), ("nota", texto("y"))])
        ));
        assert_eq!(r.columnas(), ["estado", "nota"]);
        assert!(!r.vacio());
    }

    #[test]
    fn las_claves_de_fuera_de_la_particion_se_evaluan_aqui() {
        let t = pedidos(&[]);
        let p = Peticion {
            objeto: "ore_prueba.pedidos".into(),
            proyeccion: vec![("total".into(), "total".into())],
            clave_columnas: vec!["estado".into(), "nota".into()],
            claves: vec![vec!["a".into(), "x".into()], vec!["b".into(), "y".into()]],
            ..Default::default()
        };
        let plan = traducir(&p, &t, &Opciones::default()).unwrap().plan;
        let r = Residual::de(&plan, &t).unwrap();
        assert!(pasa(
            &r,
            &fila(&[("estado", texto("a")), ("nota", texto("x"))])
        ));
        assert!(pasa(
            &r,
            &fila(&[("estado", texto("b")), ("nota", texto("y"))])
        ));
        assert!(
            !pasa(&r, &fila(&[("estado", texto("a")), ("nota", texto("y"))])),
            "la pareja, no cada una"
        );
        assert!(!pasa(&r, &fila(&[("estado", texto("a")), ("nota", None)])));
    }

    #[test]
    fn nan_es_mayor_que_todo_y_igual_a_si_mismo() {
        assert_eq!(real(f64::NAN, f64::NAN), Ordering::Equal);
        assert_eq!(real(f64::NAN, f64::INFINITY), Ordering::Greater);
        assert_eq!(real(1.0, f64::NAN), Ordering::Less);
        assert_eq!(
            comparar(&Valor::Entero(2), &Valor::Real(2.0)),
            Some(Ordering::Equal)
        );
        assert_eq!(comparar(&Valor::Entero(2), &Valor::Texto("2".into())), None);
    }

    #[test]
    fn sin_residual_no_hay_nada_que_evaluar() {
        let r = res(vec![]);
        assert!(r.vacio() && r.columnas().is_empty());
        assert!(pasa(&r, &fila(&[])));
    }
}
