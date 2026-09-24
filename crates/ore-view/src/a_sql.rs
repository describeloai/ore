//! **Un plan, escrito en SQL** (el dialecto de DuckDB, que es el motor del puesto).
//!
//! Medido en `pruebas-de-fuego/medida-la-vista-con-filtro.py`: leer una `View`
//! desde un puesto NO aplicaba su `where` ni sus `fields` —`datos` daba el
//! puntero del dataset raíz y el SDK hacía `select *` sobre él: 20 000 filas y
//! 4 columnas donde la View dice 5 000 y 2—. La View tiene que llegar al motor
//! como lo que es, una pregunta sobre su dataset, y la pregunta ya está
//! compilada: es el plan. Aquí se escribe.
//!
//! Y no es sólo corrección: medido en `medida-la-vista-a-escala.py`, con 20 M de
//! filas la View escrita así lee del bucket un 71 % menos que el `select *`
//! (la proyección baja a Parquet) y usa un 83 % menos de memoria.
//!
//! # La hoja la escribe quien llama
//!
//! Un [`Nodo::Lee`] es «esto se lee de ahí», y *dónde* es cosa de quien ejecuta:
//! para el puesto, la vista de DuckDB que el SDK crea sobre el dataset por su
//! puntero. Por eso la hoja es una función.
//!
//! # Lo que no se sabe escribir se dice
//!
//! Un [`Nodo::Referencia`] (plan sin expandir), una [`Nodo::Une`] y una
//! [`Opaca`] de otro dialecto devuelven `Err` con el motivo. Escribir algo
//! parecido sería peor que no escribir nada: es exactamente el fallo que esto
//! arregla —devolver filas que la pregunta no pide—.
//!
//! # El orden de las columnas
//!
//! El de la proyección del plan, que es un `BTreeMap`: alfabético. El plan dice
//! a propósito que el orden de columnas con nombre no significa nada (dos
//! escrituras que sólo difieran en él son el mismo plan), y el SQL lo hereda.

use crate::plan::{Agregacion, Agregado, Comparador, Expr, Lectura, Nodo, Valor};

/// El nombre, entre comillas dobles, con las de dentro dobladas.
pub fn ident(n: &str) -> String {
    format!("\"{}\"", n.replace('"', "\"\""))
}

fn cadena(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn valor(v: &Valor) -> String {
    match v {
        Valor::Cadena(s) => cadena(s),
        Valor::Entero(n) => n.to_string(),
        // Los dígitos tal cual se escribieron: DuckDB los lee como DECIMAL, que
        // es lo que son, y no como un doble.
        Valor::Decimal(d) => d.clone(),
        Valor::Booleano(b) => if *b { "true" } else { "false" }.to_string(),
    }
}

/// Una expresión del plan, en SQL.
pub fn expr(e: &Expr) -> Result<String, String> {
    Ok(match e {
        Expr::Campo(c) => ident(c),
        Expr::Literal(v) => valor(v),
        Expr::Compara {
            op,
            izquierda,
            derecha,
        } => {
            let o = match op {
                Comparador::Igual => "=",
                Comparador::Distinto => "<>",
                Comparador::Menor => "<",
                Comparador::MenorIgual => "<=",
                Comparador::Mayor => ">",
                Comparador::MayorIgual => ">=",
            };
            format!("({} {o} {})", expr(izquierda)?, expr(derecha)?)
        }
        Expr::EnConjunto { campo, valores } if valores.is_empty() => {
            // `x IN ()` no es SQL, y un conjunto vacío no contiene nada.
            let _ = campo;
            "false".to_string()
        }
        Expr::EnConjunto { campo, valores } => format!(
            "({} in ({}))",
            ident(campo),
            valores.iter().map(valor).collect::<Vec<_>>().join(", ")
        ),
        Expr::EsNulo(x) => format!("({} is null)", expr(x)?),
        Expr::Y(xs) if xs.is_empty() => "true".to_string(),
        Expr::O(xs) if xs.is_empty() => "false".to_string(),
        Expr::Y(xs) => format!("({})", unir(xs, " and ")?),
        Expr::O(xs) => format!("({})", unir(xs, " or ")?),
        Expr::No(x) => format!("(not {})", expr(x)?),
        Expr::Opaca(o) if o.dialecto.eq_ignore_ascii_case("duckdb") => format!("({})", o.texto),
        Expr::Opaca(o) => {
            return Err(format!(
                "una expresión opaca en `{}` no se escribe en DuckDB: `{}`",
                o.dialecto, o.texto
            ));
        }
    })
}

fn unir(xs: &[Expr], sep: &str) -> Result<String, String> {
    Ok(xs
        .iter()
        .map(expr)
        .collect::<Result<Vec<_>, _>>()?
        .join(sep))
}

fn agregacion(a: &Agregacion) -> Result<String, String> {
    let f = match a.funcion {
        Agregado::Cuenta => "count",
        Agregado::Suma => "sum",
        Agregado::Minimo => "min",
        Agregado::Maximo => "max",
        Agregado::Promedio => "avg",
    };
    Ok(match (&a.funcion, &a.sobre) {
        (Agregado::Cuenta, None) => "count(*)".to_string(),
        (_, Some(c)) => format!("{f}({})", ident(c)),
        (_, None) => return Err(format!("`{f}` sin columna sobre la que agregar")),
    })
}

/// El plan entero, en SQL. `hoja` escribe cada lectura (lo que va tras `from`).
pub fn plan(n: &Nodo, hoja: &dyn Fn(&Lectura) -> Result<String, String>) -> Result<String, String> {
    Ok(match n {
        Nodo::Lee(l) => format!("select * from {}", hoja(l)?),
        Nodo::Referencia(r) => {
            return Err(format!(
                "el plan nombra `{r}` sin haberlo expandido: no se sabe qué se lee"
            ));
        }
        Nodo::Proyecta { entrada, campos } => {
            let cols = campos
                .iter()
                .map(|(nombre, e)| Ok(format!("{} as {}", expr(e)?, ident(nombre))))
                .collect::<Result<Vec<_>, String>>()?;
            format!(
                "select {} from ({}) as t",
                if cols.is_empty() {
                    "*".to_string()
                } else {
                    cols.join(", ")
                },
                plan(entrada, hoja)?
            )
        }
        Nodo::Filtra { entrada, predicado } => format!(
            "select * from ({}) as t where {}",
            plan(entrada, hoja)?,
            expr(predicado)?
        ),
        Nodo::Agrupa {
            entrada,
            por,
            agregados,
        } => {
            let mut cols: Vec<String> = por.iter().map(|c| ident(c)).collect();
            for (nombre, a) in agregados {
                cols.push(format!("{} as {}", agregacion(a)?, ident(nombre)));
            }
            let grupo = if por.is_empty() {
                String::new()
            } else {
                format!(
                    " group by {}",
                    por.iter().map(|c| ident(c)).collect::<Vec<_>>().join(", ")
                )
            };
            format!(
                "select {} from ({}) as t{grupo}",
                cols.join(", "),
                plan(entrada, hoja)?
            )
        }
        Nodo::Unifica(ramas) => ramas
            .iter()
            .map(|r| Ok(format!("({})", plan(r, hoja)?)))
            .collect::<Result<Vec<_>, String>>()?
            .join(" union all "),
        Nodo::Distingue(e) => format!("select distinct * from ({}) as t", plan(e, hoja)?),
        Nodo::Limita { entrada, n } => {
            format!("select * from ({}) as t limit {n}", plan(entrada, hoja)?)
        }
        Nodo::Une { .. } => {
            return Err(
                "una unión de dos entradas todavía no se escribe en SQL: esa View no se lee desde un puesto"
                    .to_string(),
            );
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    fn lee(objeto: &str) -> Nodo {
        Nodo::Lee(Lectura {
            datasource: "dataset".into(),
            objeto: objeto.into(),
            campos: BTreeMap::new(),
        })
    }

    fn hoja(l: &Lectura) -> Result<String, String> {
        Ok(format!("{}.{}", ident("__ore_dataset"), ident(&l.objeto)))
    }

    /// La View de la medida: `where {pais: ES}`, `fields {id, pais}`.
    #[test]
    fn la_vista_con_filtro_y_proyeccion() {
        let n = Nodo::Proyecta {
            entrada: Box::new(Nodo::Filtra {
                entrada: Box::new(lee("hr.ventas")),
                predicado: Expr::EnConjunto {
                    campo: "pais".into(),
                    valores: vec![Valor::Cadena("ES".into())],
                },
            }),
            campos: [
                ("id".to_string(), Expr::campo("id")),
                ("pais".to_string(), Expr::campo("pais")),
            ]
            .into_iter()
            .collect(),
        };
        assert_eq!(
            plan(&n, &hoja).unwrap(),
            "select \"id\" as \"id\", \"pais\" as \"pais\" from (select * from (select * from \"__ore_dataset\".\"hr.ventas\") as t where (\"pais\" in ('ES'))) as t"
        );
    }

    #[test]
    fn agrupar_contar_y_lo_que_no_se_escribe() {
        let n = Nodo::Agrupa {
            entrada: Box::new(lee("hr.ventas")),
            por: BTreeSet::from(["pais".to_string()]),
            agregados: [
                (
                    "n".to_string(),
                    Agregacion {
                        funcion: Agregado::Cuenta,
                        sobre: None,
                    },
                ),
                (
                    "suma".to_string(),
                    Agregacion {
                        funcion: Agregado::Suma,
                        sobre: Some("total".into()),
                    },
                ),
            ]
            .into_iter()
            .collect(),
        };
        let s = plan(&n, &hoja).unwrap();
        assert!(
            s.starts_with("select \"pais\", count(*) as \"n\", sum(\"total\") as \"suma\" from (")
                && s.ends_with(") as t group by \"pais\""),
            "{s}"
        );
        assert!(plan(&Nodo::Referencia("hr.x".into()), &hoja).is_err());
        let une = Nodo::Une {
            izquierda: Box::new(lee("a.b")),
            derecha: Box::new(lee("a.c")),
            tipo: crate::plan::Junta::Interna,
            sobre: vec![("id".into(), "id".into())],
        };
        assert!(plan(&une, &hoja).unwrap_err().contains("unión"));
    }

    #[test]
    fn los_literales_no_se_escapan_del_sql() {
        let e = Expr::Compara {
            op: Comparador::Igual,
            izquierda: Box::new(Expr::campo("no\"mbre")),
            derecha: Box::new(Expr::Literal(Valor::Cadena("O'Brien".into()))),
        };
        assert_eq!(expr(&e).unwrap(), "(\"no\"\"mbre\" = 'O''Brien')");
        assert_eq!(
            expr(&Expr::Literal(Valor::Decimal("0.10".into()))).unwrap(),
            "0.10"
        );
        assert_eq!(expr(&Expr::Y(vec![])).unwrap(), "true");
        assert_eq!(
            expr(&Expr::EnConjunto {
                campo: "x".into(),
                valores: vec![]
            })
            .unwrap(),
            "false"
        );
    }
}
