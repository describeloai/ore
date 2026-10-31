//! **Las sentencias de ORE** (0081 §7.5.2, C2). El DDL de ORE habla con sus palabras: un dataset se crea con
//! `create [or replace] dataset <nombre> as <consulta>`. Entra por la extensión del parser que DataFusion declara
//! (`DFParser`, como él añade `CREATE EXTERNAL TABLE` y `COPY`): se reconoce la sentencia por sus primeras
//! palabras y lo demás es de DataFusion. La consulta de dentro la analiza y la planifica DataFusion.

use datafusion::common::{DataFusionError, Result, not_impl_err, plan_datafusion_err, plan_err};
use datafusion::config::Dialect;
use datafusion::sql::parser::{DFParser, DFParserBuilder, Statement};
use datafusion::sql::sqlparser::ast::{self, ObjectName};
use datafusion::sql::sqlparser::dialect::dialect_from_str;
use datafusion::sql::sqlparser::keywords::Keyword;
use datafusion::sql::sqlparser::parser::ParserError;
use datafusion::sql::sqlparser::tokenizer::Token;

/// La palabra de ORE para lo que el lago guarda.
pub const DATASET: &str = "dataset";

/// Una sentencia analizada.
#[derive(Debug, Clone)]
pub enum Sentencia {
    /// Lo de DataFusion: una consulta, un DML o lo que sea; la política de la sesión decide si se admite.
    DataFusion(Statement),
    /// `create [or replace] dataset <nombre> as <consulta>`.
    CrearDataset {
        nombre: ObjectName,
        reemplazar: bool,
        /// La consulta, como sentencia de DataFusion.
        consulta: Statement,
    },
}

/// Analiza **una** sentencia, con el dialecto y el límite de recursión de la sesión.
pub fn analizar(sql: &str, dialecto: &Dialect, recursion: usize) -> Result<Sentencia> {
    let d = dialect_from_str(dialecto)
        .ok_or_else(|| plan_datafusion_err!("el dialecto `{dialecto}` no existe"))?;
    let mut p = DFParserBuilder::new(sql)
        .with_dialect(d.as_ref())
        .with_recursion_limit(recursion)
        .build()?;
    while p.parser.consume_token(&Token::SemiColon) {}
    if p.parser.peek_token() == Token::EOF {
        return plan_err!("no hay ninguna sentencia");
    }
    let s = if es_crear_dataset(&p) {
        crear_dataset(&mut p)?
    } else {
        Sentencia::DataFusion(p.parse_statement()?)
    };
    while p.parser.consume_token(&Token::SemiColon) {}
    if p.parser.peek_token() != Token::EOF {
        return not_impl_err!("una consulta es una sola sentencia");
    }
    Ok(s)
}

fn sql(e: ParserError) -> DataFusionError {
    DataFusionError::SQL(Box::new(e), None)
}

/// `CREATE [OR REPLACE] DATASET`, mirando sin consumir.
fn es_crear_dataset(p: &DFParser) -> bool {
    let palabra = |i: usize| match p.parser.peek_nth_token(i).token {
        Token::Word(w) => Some(w),
        _ => None,
    };
    let es = |i: usize, k: Keyword| palabra(i).is_some_and(|w| w.keyword == k);
    let dataset = |i: usize| palabra(i).is_some_and(|w| w.value.eq_ignore_ascii_case(DATASET));
    es(0, Keyword::CREATE)
        && (dataset(1) || (es(1, Keyword::OR) && es(2, Keyword::REPLACE) && dataset(3)))
}

fn crear_dataset(p: &mut DFParser) -> Result<Sentencia> {
    p.parser.expect_keyword(Keyword::CREATE).map_err(sql)?;
    let reemplazar = p.parser.parse_keywords(&[Keyword::OR, Keyword::REPLACE]);
    p.parser.next_token(); // DATASET
    let nombre = p.parser.parse_object_name(false).map_err(sql)?;
    if !p.parser.parse_keyword(Keyword::AS) {
        if p.parser.peek_token() == Token::LParen {
            return not_impl_err!(
                "`create dataset` con columnas todavía no está en ORE SQL; con `as <consulta>`, sí"
            );
        }
        return p
            .parser
            .expected("AS <consulta>", p.parser.peek_token())
            .map_err(sql);
    }
    let consulta = p.parser.parse_query().map_err(sql)?;
    Ok(Sentencia::CrearDataset {
        nombre,
        reemplazar,
        consulta: Statement::Statement(Box::new(ast::Statement::Query(consulta))),
    })
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn analiza(sql: &str) -> Result<Sentencia> {
        analizar(sql, &Dialect::Generic, 50)
    }

    #[test]
    fn crear_dataset_con_y_sin_reemplazar() {
        for (sql, nombre, r) in [
            ("create dataset b.s.d as select 1", "b.s.d", false),
            ("CREATE OR REPLACE DATASET s.d AS SELECT 1;", "s.d", true),
            (
                "create or replace Dataset d as with t as (select 1) select * from t",
                "d",
                true,
            ),
        ] {
            match analiza(sql).unwrap() {
                Sentencia::CrearDataset {
                    nombre: n,
                    reemplazar,
                    consulta: Statement::Statement(q),
                } => {
                    assert_eq!(n.to_string(), nombre, "{sql}");
                    assert_eq!(reemplazar, r, "{sql}");
                    assert!(matches!(*q, ast::Statement::Query(_)), "{sql}");
                }
                otra => panic!("{sql}: {otra:?}"),
            }
        }
    }

    #[test]
    fn lo_demas_es_de_datafusion() {
        for sql in [
            "select 1",
            "create table t (v int)",
            "create view dataset as select 1",
            "insert into t values (1)",
            "copy t to 'x.parquet'",
        ] {
            assert!(
                matches!(analiza(sql).unwrap(), Sentencia::DataFusion(_)),
                "{sql}"
            );
        }
    }

    #[test]
    fn lo_que_no_es_una_sentencia_de_ore() {
        for (sql, dice) in [
            ("create dataset d (v int)", "con columnas"),
            ("create dataset d select 1", "AS <consulta>"),
            ("create dataset as select 1", "Expected"),
            (
                "create dataset d as select 1; select 2",
                "una sola sentencia",
            ),
            ("select 1; select 2", "una sola sentencia"),
            (" ; ", "ninguna sentencia"),
        ] {
            let e = analiza(sql).unwrap_err().to_string();
            assert!(e.contains(dice), "{sql}: {e}");
        }
    }
}
