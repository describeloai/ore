//! **Un plan, escrito en SQL**: el dialecto de DuckDB, que es el motor del
//! puesto, y el de Spark, para las Views que sirve el catálogo (`/v1`
//! `loadView`; medido en `medida-spark-por-el-catalogo.py`).
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
//!
//! # Lo que cambia de un dialecto a otro
//!
//! El nombre: `"x"` en DuckDB, `` `x` `` en Spark (en Spark `"x"` es una
//! CADENA). Y la cadena: en DuckDB la comilla de dentro se dobla; en Spark
//! **no** —`'O''Brien'` son dos literales seguidos que Spark concatena,
//! `OBrien`, sin error—, se escapa con la barra, y la barra también. El resto
//! (`in`, `is null`, `<>`, `count(*)`, `distinct`, `limit`, `union all`, un
//! decimal escrito con sus dígitos) se escribe igual en los dos.

use crate::plan::{Agregacion, Agregado, Comparador, Expr, Lectura, Nodo, Valor};

/// En qué SQL se escribe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialecto {
    DuckDb,
    Spark,
}

impl Dialecto {
    /// Como lo nombra la spec de vistas de Iceberg (`representations[].dialect`)
    /// y como lo nombra una `Opaca`.
    pub fn nombre(self) -> &'static str {
        match self {
            Dialecto::DuckDb => "duckdb",
            Dialecto::Spark => "spark",
        }
    }
}

/// El nombre, entre comillas dobles, con las de dentro dobladas (DuckDB).
pub fn ident(n: &str) -> String {
    ident_en(Dialecto::DuckDb, n)
}

/// El nombre, en el dialecto.
pub fn ident_en(d: Dialecto, n: &str) -> String {
    match d {
        Dialecto::DuckDb => format!("\"{}\"", n.replace('"', "\"\"")),
        Dialecto::Spark => format!("`{}`", n.replace('`', "``")),
    }
}

fn cadena(d: Dialecto, s: &str) -> String {
    match d {
        Dialecto::DuckDb => format!("'{}'", s.replace('\'', "''")),
        Dialecto::Spark => format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'")),
    }
}

fn valor(d: Dialecto, v: &Valor) -> String {
    match v {
        Valor::Cadena(s) => cadena(d, s),
        Valor::Entero(n) => n.to_string(),
        // Los dígitos tal cual se escribieron: DuckDB los lee como DECIMAL, que
        // es lo que son, y no como un doble.
        Valor::Decimal(d) => d.clone(),
        Valor::Booleano(b) => if *b { "true" } else { "false" }.to_string(),
    }
}

/// Una expresión del plan, en SQL de DuckDB.
pub fn expr(e: &Expr) -> Result<String, String> {
    expr_en(Dialecto::DuckDb, e)
}

/// Una expresión del plan, en el dialecto.
pub fn expr_en(d: Dialecto, e: &Expr) -> Result<String, String> {
    let expr = |e: &Expr| expr_en(d, e);
    let ident = |n: &str| ident_en(d, n);
    let valor = |v: &Valor| valor(d, v);
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
        Expr::Y(xs) => format!("({})", unir(d, xs, " and ")?),
        Expr::O(xs) => format!("({})", unir(d, xs, " or ")?),
        Expr::No(x) => format!("(not {})", expr(x)?),
        Expr::Opaca(o) if o.dialecto.eq_ignore_ascii_case(d.nombre()) => format!("({})", o.texto),
        Expr::Opaca(o) => {
            return Err(format!(
                "una expresión opaca en `{}` no se escribe en {}: `{}`",
                o.dialecto,
                d.nombre(),
                o.texto
            ));
        }
    })
}

fn unir(d: Dialecto, xs: &[Expr], sep: &str) -> Result<String, String> {
    Ok(xs
        .iter()
        .map(|e| expr_en(d, e))
        .collect::<Result<Vec<_>, _>>()?
        .join(sep))
}

fn agregacion(d: Dialecto, a: &Agregacion) -> Result<String, String> {
    let ident = |n: &str| ident_en(d, n);
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

/// El plan entero, en SQL de DuckDB. `hoja` escribe cada lectura (lo que va
/// tras `from`).
pub fn plan(n: &Nodo, hoja: &dyn Fn(&Lectura) -> Result<String, String>) -> Result<String, String> {
    plan_en(Dialecto::DuckDb, n, hoja)
}

/// El plan entero, en el dialecto. `hoja` escribe cada lectura, ya en él.
pub fn plan_en(
    d: Dialecto,
    n: &Nodo,
    hoja: &dyn Fn(&Lectura) -> Result<String, String>,
) -> Result<String, String> {
    let plan = |n: &Nodo, hoja: &dyn Fn(&Lectura) -> Result<String, String>| plan_en(d, n, hoja);
    let expr = |e: &Expr| expr_en(d, e);
    let ident = |n: &str| ident_en(d, n);
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
                cols.push(format!("{} as {}", agregacion(d, a)?, ident(nombre)));
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

    /// Spark: el nombre entre acentos graves (sus comillas dobles son una
    /// cadena), y la comilla de una cadena con la barra —doblarla la partiría
    /// en dos literales que Spark concatena sin error—.
    #[test]
    fn en_spark_el_nombre_y_la_cadena_se_escriben_a_su_manera() {
        let e = Expr::Compara {
            op: Comparador::Igual,
            izquierda: Box::new(Expr::campo("no`mbre")),
            derecha: Box::new(Expr::Literal(Valor::Cadena("O'Brien \\ 50%".into()))),
        };
        assert_eq!(
            expr_en(Dialecto::Spark, &e).unwrap(),
            "(`no``mbre` = 'O\\'Brien \\\\ 50%')"
        );
        let hoja_spark = |l: &Lectura| -> Result<String, String> {
            let (p, n) = l.objeto.split_once('.').unwrap();
            Ok(format!(
                "{}.{}",
                ident_en(Dialecto::Spark, p),
                ident_en(Dialecto::Spark, n)
            ))
        };
        let n = Nodo::Filtra {
            entrada: Box::new(lee("hr.ventas")),
            predicado: Expr::EnConjunto {
                campo: "pais".into(),
                valores: vec![Valor::Cadena("ES".into())],
            },
        };
        assert_eq!(
            plan_en(Dialecto::Spark, &n, &hoja_spark).unwrap(),
            "select * from (select * from `hr`.`ventas`) as t where (`pais` in ('ES'))"
        );
        // una opaca se escribe sólo en su dialecto
        let o = Expr::Opaca(crate::plan::Opaca {
            dialecto: "duckdb".into(),
            texto: "x::int > 1".into(),
            lee: vec!["x".into()],
            tipo: ore_core::types::parse_type("Boolean").unwrap(),
            determinista: true,
        });
        assert!(expr_en(Dialecto::DuckDb, &o).is_ok());
        assert!(expr_en(Dialecto::Spark, &o).unwrap_err().contains("spark"));
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
