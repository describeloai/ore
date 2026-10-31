//! **Las sentencias de ORE** (0081 §7.5.2, C2 y C2b). El DDL de ORE habla con sus palabras, y entra por la extensión
//! del parser que DataFusion declara (`DFParser`, como él añade `CREATE EXTERNAL TABLE` y `COPY`): se reconoce cada
//! sentencia por sus primeras palabras, y lo demás es de DataFusion. Las consultas de dentro (`as select …`) las
//! analiza DataFusion, y las columnas de un dataset vacío son su `ColumnDef`.
//!
//! | la frase | es |
//! |---|---|
//! | `create [standard] database [if not exists] b` | una standard database vacía |
//! | `create standard database b from origin o include (s.t, s.*)` | una standard database copiada de un origen |
//! | `create foreign database b from origin o include (…)` | una foreign database, que se lee en su origen |
//! | `create schema [if not exists] [b.]s` | un schema |
//! | `create dataset [if not exists] [[b.]s.]d (col tipo [not null], …)` | un dataset vacío, con su esquema |
//! | `create [or replace] dataset [if not exists] … as <consulta>` | un dataset con lo que da la consulta |
//! | `create [or replace] [materialized] view [if not exists] … [(col [comment '…'], …)] [comment '…'] [with schema evolution] as <consulta>` | una vista; materializada, con su copia |
//! | `create [or replace] media collection [if not exists] … media <tipo> formats (ext, …) [comment '…'] [as <consulta>]` | una colección de medios; con consulta, ficheros que dan ficheros |
//! | `drop (dataset \| view \| media collection) [if exists] …` | lo retira (queda en la historia) |
//! | `drop (schema \| database) [if exists] … [cascade]` | lo retira; con algo dentro, sólo con `cascade` |
//!
//! `or replace` e `if not exists` no van juntas. `create table` no existe: lo que el lago guarda es un dataset (una
//! `Table` de ORE es el puntero a un objeto de un origen, que nace del descubrimiento).

use datafusion::common::{DataFusionError, Result, not_impl_err, plan_datafusion_err, plan_err};
use datafusion::config::Dialect;
use datafusion::sql::parser::{DFParser, DFParserBuilder, Statement};
use datafusion::sql::sqlparser::ast::{self, ColumnDef, Ident, ObjectName};
use datafusion::sql::sqlparser::dialect::dialect_from_str;
use datafusion::sql::sqlparser::keywords::Keyword;
use datafusion::sql::sqlparser::parser::ParserError;
use datafusion::sql::sqlparser::tokenizer::Token;
use std::fmt;

/// La palabra de ORE para lo que el lago guarda.
pub const DATASET: &str = "dataset";

/// Una sentencia analizada.
#[derive(Debug, Clone)]
pub enum Sentencia {
    /// Lo de DataFusion: una consulta, un DML o lo que sea; la política de la sesión decide si se admite.
    DataFusion(Statement),
    /// `create [or replace] dataset <nombre> as <consulta>`: escribe datos.
    CrearDataset {
        nombre: ObjectName,
        reemplazar: bool,
        si_no_existe: bool,
        /// La consulta, como sentencia de DataFusion.
        consulta: Statement,
    },
    /// El resto del DDL de ORE: cambia el catálogo, sin datos (o con los de una consulta que es su definición).
    Ddl(Ddl),
}

/// De qué clase es una base.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clase {
    /// Sus datos viven en el lago.
    Standard,
    /// Se lee en su origen.
    Foreign,
}

/// `from origin <origen> include (<patrón>, …)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeOrigen {
    pub origen: ObjectName,
    /// `s.t` o `s.*`, como se escribieron.
    pub incluye: Vec<String>,
}

/// Una columna de una vista: su nombre y su comentario.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnaDeVista {
    pub nombre: Ident,
    pub comentario: Option<String>,
}

/// Lo que se retira.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retirable {
    Base,
    Schema,
    Dataset,
    Vista,
    Coleccion,
}

impl fmt::Display for Retirable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Retirable::Base => "database",
            Retirable::Schema => "schema",
            Retirable::Dataset => "dataset",
            Retirable::Vista => "view",
            Retirable::Coleccion => "media collection",
        })
    }
}

/// **El DDL de ORE**, tal como se escribió (los nombres sin resolver; las consultas, como sentencias).
#[derive(Debug, Clone)]
pub enum Ddl {
    CrearBase {
        nombre: ObjectName,
        clase: Clase,
        si_no_existe: bool,
        origen: Option<DeOrigen>,
    },
    CrearSchema {
        nombre: ObjectName,
        si_no_existe: bool,
    },
    CrearDatasetVacio {
        nombre: ObjectName,
        si_no_existe: bool,
        columnas: Vec<ColumnDef>,
    },
    CrearVista {
        nombre: ObjectName,
        reemplazar: bool,
        si_no_existe: bool,
        materializada: bool,
        columnas: Vec<ColumnaDeVista>,
        comentario: Option<String>,
        evolucion: bool,
        consulta: Statement,
    },
    CrearColeccion {
        nombre: ObjectName,
        reemplazar: bool,
        si_no_existe: bool,
        medio: String,
        formatos: Vec<String>,
        comentario: Option<String>,
        consulta: Option<Statement>,
    },
    Retirar {
        que: Retirable,
        nombre: ObjectName,
        si_existe: bool,
        cascada: bool,
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
    let s = match de_ore(&p) {
        Some(Verbo::Crear) => crear(&mut p)?,
        Some(Verbo::Retirar) => retirar(&mut p)?,
        None => Sentencia::DataFusion(p.parse_statement()?),
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

enum Verbo {
    Crear,
    Retirar,
}

/// La palabra `i` por delante, sin consumir.
fn palabra(p: &DFParser, i: usize) -> Option<String> {
    match p.parser.peek_nth_token(i).token {
        Token::Word(w) => Some(w.value.to_ascii_lowercase()),
        _ => None,
    }
}

/// Si la sentencia es de ORE: `create` o `drop` seguido de un objeto de ORE (o de `table`, que ORE dice).
fn de_ore(p: &DFParser) -> Option<Verbo> {
    let w = |i| palabra(p, i);
    match w(0)?.as_str() {
        "create" => {
            let i = if w(1).as_deref() == Some("or") && w(2).as_deref() == Some("replace") {
                3
            } else {
                1
            };
            matches!(
                w(i)?.as_str(),
                "standard"
                    | "foreign"
                    | "database"
                    | "schema"
                    | "dataset"
                    | "view"
                    | "materialized"
                    | "media"
                    | "table"
            )
            .then_some(Verbo::Crear)
        }
        "drop" => matches!(
            w(1)?.as_str(),
            "database" | "schema" | "dataset" | "view" | "media" | "table"
        )
        .then_some(Verbo::Retirar),
        _ => None,
    }
}

/// Consume `palabra` si es la siguiente (por su texto: las de ORE no son palabras clave de SQL).
fn tomar(p: &mut DFParser, palabra: &str) -> bool {
    match &p.parser.peek_token().token {
        Token::Word(w) if w.value.eq_ignore_ascii_case(palabra) => {
            p.parser.next_token();
            true
        }
        _ => false,
    }
}

fn exigir(p: &mut DFParser, palabra: &str) -> Result<()> {
    if tomar(p, palabra) {
        Ok(())
    } else {
        p.parser
            .expected(&palabra.to_uppercase(), p.parser.peek_token())
            .map_err(sql)
    }
}

fn si_no_existe(p: &mut DFParser) -> bool {
    p.parser
        .parse_keywords(&[Keyword::IF, Keyword::NOT, Keyword::EXISTS])
}

fn comentario(p: &mut DFParser) -> Result<Option<String>> {
    if p.parser.parse_keyword(Keyword::COMMENT) {
        Ok(Some(p.parser.parse_literal_string().map_err(sql)?))
    } else {
        Ok(None)
    }
}

fn consulta(p: &mut DFParser) -> Result<Statement> {
    let q = p.parser.parse_query().map_err(sql)?;
    Ok(Statement::Statement(Box::new(ast::Statement::Query(q))))
}

fn crear(p: &mut DFParser) -> Result<Sentencia> {
    p.parser.expect_keyword(Keyword::CREATE).map_err(sql)?;
    let reemplazar = p.parser.parse_keywords(&[Keyword::OR, Keyword::REPLACE]);
    let que = palabra(p, 0).unwrap_or_default();
    let sentencia = match que.as_str() {
        "table" => {
            return plan_err!(
                "en ORE un dataset se crea con `create [or replace] dataset <nombre> as <consulta>` o con sus \
                 columnas; `CREATE TABLE` no existe"
            );
        }
        "standard" | "foreign" | "database" => {
            let clase = if tomar(p, "foreign") {
                Clase::Foreign
            } else {
                tomar(p, "standard");
                Clase::Standard
            };
            exigir(p, "database")?;
            if reemplazar {
                return plan_err!("una base no se reemplaza: se retira con `drop database`");
            }
            let si_no_existe = si_no_existe(p);
            let nombre = p.parser.parse_object_name(false).map_err(sql)?;
            let origen = if p.parser.parse_keyword(Keyword::FROM) {
                exigir(p, "origin")?;
                let origen = p.parser.parse_object_name(false).map_err(sql)?;
                p.parser.expect_keyword(Keyword::INCLUDE).map_err(sql)?;
                p.parser.expect_token(&Token::LParen).map_err(sql)?;
                let incluye = p.parser.parse_comma_separated(patron).map_err(sql)?;
                p.parser.expect_token(&Token::RParen).map_err(sql)?;
                Some(DeOrigen { origen, incluye })
            } else {
                None
            };
            if clase == Clase::Foreign && origen.is_none() {
                return plan_err!(
                    "una foreign database se lee en su origen: `from origin <origen> include (…)`"
                );
            }
            Ddl::CrearBase {
                nombre,
                clase,
                si_no_existe,
                origen,
            }
        }
        "schema" => {
            exigir(p, "schema")?;
            if reemplazar {
                return plan_err!("un schema no se reemplaza: se retira con `drop schema`");
            }
            let si_no_existe = si_no_existe(p);
            let nombre = p.parser.parse_object_name(false).map_err(sql)?;
            Ddl::CrearSchema {
                nombre,
                si_no_existe,
            }
        }
        "dataset" => {
            exigir(p, DATASET)?;
            let si_no_existe = si_no_existe(p);
            excluyentes(reemplazar, si_no_existe)?;
            let nombre = p.parser.parse_object_name(false).map_err(sql)?;
            if p.parser.parse_keyword(Keyword::AS) {
                return Ok(Sentencia::CrearDataset {
                    nombre,
                    reemplazar,
                    si_no_existe,
                    consulta: consulta(p)?,
                });
            }
            if p.parser.peek_token() != Token::LParen {
                return p
                    .parser
                    .expected("AS <consulta> o (<columnas>)", p.parser.peek_token())
                    .map_err(sql);
            }
            if reemplazar {
                return plan_err!(
                    "un dataset vacío no se reemplaza: `create or replace dataset … as <consulta>`"
                );
            }
            let (columnas, restricciones) = p.parser.parse_columns().map_err(sql)?;
            if !restricciones.is_empty() {
                return not_impl_err!(
                    "un dataset de ORE no lleva restricciones de tabla; `not null`, en la columna"
                );
            }
            Ddl::CrearDatasetVacio {
                nombre,
                si_no_existe,
                columnas,
            }
        }
        "view" | "materialized" => {
            let materializada = tomar(p, "materialized");
            exigir(p, "view")?;
            let si_no_existe = si_no_existe(p);
            excluyentes(reemplazar, si_no_existe)?;
            let nombre = p.parser.parse_object_name(false).map_err(sql)?;
            let columnas = if p.parser.consume_token(&Token::LParen) {
                let c = p
                    .parser
                    .parse_comma_separated(|q| {
                        let nombre = q.parse_identifier()?;
                        let comentario = if q.parse_keyword(Keyword::COMMENT) {
                            Some(q.parse_literal_string()?)
                        } else {
                            None
                        };
                        Ok(ColumnaDeVista { nombre, comentario })
                    })
                    .map_err(sql)?;
                p.parser.expect_token(&Token::RParen).map_err(sql)?;
                c
            } else {
                Vec::new()
            };
            let comentario = comentario(p)?;
            let evolucion = if p.parser.parse_keyword(Keyword::WITH) {
                p.parser.expect_keyword(Keyword::SCHEMA).map_err(sql)?;
                exigir(p, "evolution")?;
                true
            } else {
                false
            };
            p.parser.expect_keyword(Keyword::AS).map_err(sql)?;
            Ddl::CrearVista {
                nombre,
                reemplazar,
                si_no_existe,
                materializada,
                columnas,
                comentario,
                evolucion,
                consulta: consulta(p)?,
            }
        }
        "media" => {
            exigir(p, "media")?;
            exigir(p, "collection")?;
            let si_no_existe = si_no_existe(p);
            excluyentes(reemplazar, si_no_existe)?;
            let nombre = p.parser.parse_object_name(false).map_err(sql)?;
            exigir(p, "media")?;
            let medio = p.parser.parse_identifier().map_err(sql)?.value;
            exigir(p, "formats")?;
            p.parser.expect_token(&Token::LParen).map_err(sql)?;
            let formatos = p
                .parser
                .parse_comma_separated(|q| q.parse_identifier().map(|i| i.value))
                .map_err(sql)?;
            p.parser.expect_token(&Token::RParen).map_err(sql)?;
            let comentario = comentario(p)?;
            let consulta = if p.parser.parse_keyword(Keyword::AS) {
                Some(consulta(p)?)
            } else {
                None
            };
            Ddl::CrearColeccion {
                nombre,
                reemplazar,
                si_no_existe,
                medio,
                formatos,
                comentario,
                consulta,
            }
        }
        otra => return plan_err!("`create {otra}` no es una sentencia de ORE"),
    };
    Ok(Sentencia::Ddl(sentencia))
}

fn excluyentes(reemplazar: bool, si_no_existe: bool) -> Result<()> {
    if reemplazar && si_no_existe {
        return plan_err!("`or replace` e `if not exists` no van juntas");
    }
    Ok(())
}

/// `s.t` o `s.*`.
fn patron(q: &mut datafusion::sql::sqlparser::parser::Parser) -> Result<String, ParserError> {
    let schema = q.parse_identifier()?.value;
    q.expect_token(&Token::Period)?;
    if q.consume_token(&Token::Mul) {
        Ok(format!("{schema}.*"))
    } else {
        Ok(format!("{schema}.{}", q.parse_identifier()?.value))
    }
}

fn retirar(p: &mut DFParser) -> Result<Sentencia> {
    p.parser.expect_keyword(Keyword::DROP).map_err(sql)?;
    let que = match palabra(p, 0).unwrap_or_default().as_str() {
        "table" => {
            return plan_err!(
                "en ORE se retira un dataset con `drop dataset`; `DROP TABLE` no existe"
            );
        }
        "database" => Retirable::Base,
        "schema" => Retirable::Schema,
        "dataset" => Retirable::Dataset,
        "view" => Retirable::Vista,
        _ => {
            p.parser.next_token();
            exigir(p, "collection")?;
            Retirable::Coleccion
        }
    };
    if que != Retirable::Coleccion {
        p.parser.next_token();
    }
    let si_existe = p.parser.parse_keywords(&[Keyword::IF, Keyword::EXISTS]);
    let nombre = p.parser.parse_object_name(false).map_err(sql)?;
    let cascada = p.parser.parse_keyword(Keyword::CASCADE);
    if cascada && !matches!(que, Retirable::Base | Retirable::Schema) {
        return plan_err!("`cascade` es de `drop schema` y `drop database`");
    }
    Ok(Sentencia::Ddl(Ddl::Retirar {
        que,
        nombre,
        si_existe,
        cascada,
    }))
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn analiza(sql: &str) -> Result<Sentencia> {
        analizar(sql, &Dialect::Generic, 50)
    }

    fn ddl(sql: &str) -> Ddl {
        match analiza(sql) {
            Ok(Sentencia::Ddl(d)) => d,
            otra => panic!("{sql}: {otra:?}"),
        }
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
                    ..
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
    fn las_bases() {
        match ddl("create database if not exists ventas") {
            Ddl::CrearBase {
                nombre,
                clase: Clase::Standard,
                si_no_existe: true,
                origen: None,
            } => assert_eq!(nombre.to_string(), "ventas"),
            otra => panic!("{otra:?}"),
        }
        match ddl(
            "create foreign database crm from origin pg.prod include (public.*, ventas.pedidos)",
        ) {
            Ddl::CrearBase {
                clase: Clase::Foreign,
                origen: Some(o),
                ..
            } => {
                assert_eq!(o.origen.to_string(), "pg.prod");
                assert_eq!(o.incluye, ["public.*", "ventas.pedidos"]);
            }
            otra => panic!("{otra:?}"),
        }
        assert!(matches!(
            ddl("create standard database copia from origin o include (s.t)"),
            Ddl::CrearBase {
                clase: Clase::Standard,
                origen: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn schemas_y_datasets_vacios() {
        assert!(matches!(
            ddl("create schema ventas.eu"),
            Ddl::CrearSchema {
                si_no_existe: false,
                ..
            }
        ));
        match ddl("create dataset if not exists eu.d (id bigint not null, nombre varchar)") {
            Ddl::CrearDatasetVacio {
                nombre,
                si_no_existe: true,
                columnas,
            } => {
                assert_eq!(nombre.to_string(), "eu.d");
                assert_eq!(columnas.len(), 2);
                assert_eq!(columnas[0].to_string(), "id BIGINT NOT NULL");
            }
            otra => panic!("{otra:?}"),
        }
    }

    #[test]
    fn las_vistas() {
        match ddl(
            "create or replace materialized view v.s.w (a comment 'la a', b) comment 'una vista' \
             with schema evolution as select 1 as a, 2 as b",
        ) {
            Ddl::CrearVista {
                nombre,
                reemplazar: true,
                si_no_existe: false,
                materializada: true,
                columnas,
                comentario,
                evolucion: true,
                ..
            } => {
                assert_eq!(nombre.to_string(), "v.s.w");
                assert_eq!(columnas.len(), 2);
                assert_eq!(columnas[0].comentario.as_deref(), Some("la a"));
                assert_eq!(columnas[1].comentario, None);
                assert_eq!(comentario.as_deref(), Some("una vista"));
            }
            otra => panic!("{otra:?}"),
        }
        assert!(matches!(
            ddl("create view if not exists v as select 1"),
            Ddl::CrearVista {
                si_no_existe: true,
                materializada: false,
                evolucion: false,
                ..
            }
        ));
    }

    #[test]
    fn las_colecciones() {
        match ddl(
            "create media collection fotos media image formats (jpg, png) comment 'las fotos'",
        ) {
            Ddl::CrearColeccion {
                medio,
                formatos,
                comentario,
                consulta: None,
                ..
            } => {
                assert_eq!(medio, "image");
                assert_eq!(formatos, ["jpg", "png"]);
                assert_eq!(comentario.as_deref(), Some("las fotos"));
            }
            otra => panic!("{otra:?}"),
        }
        assert!(matches!(
            ddl(
                "create or replace media collection miniaturas media image formats (png) as select 1"
            ),
            Ddl::CrearColeccion {
                reemplazar: true,
                consulta: Some(_),
                ..
            }
        ));
    }

    #[test]
    fn retirar() {
        for (sql, que, si, cascada) in [
            (
                "drop dataset if exists b.s.d",
                Retirable::Dataset,
                true,
                false,
            ),
            ("drop view v", Retirable::Vista, false, false),
            (
                "drop media collection fotos",
                Retirable::Coleccion,
                false,
                false,
            ),
            (
                "drop schema if exists b.s cascade",
                Retirable::Schema,
                true,
                true,
            ),
            ("drop database b", Retirable::Base, false, false),
        ] {
            match ddl(sql) {
                Ddl::Retirar {
                    que: q,
                    si_existe,
                    cascada: c,
                    ..
                } => assert_eq!((q, si_existe, c), (que, si, cascada), "{sql}"),
                otra => panic!("{sql}: {otra:?}"),
            }
        }
    }

    #[test]
    fn lo_demas_es_de_datafusion() {
        for sql in [
            "select 1",
            "insert into t values (1)",
            "create external table t stored as parquet location 'x'",
            "create function f as 'x'",
            "copy t to 'x.parquet'",
            "drop function f",
        ] {
            assert!(
                matches!(analiza(sql), Ok(Sentencia::DataFusion(_)) | Err(_)),
                "{sql}"
            );
        }
        for sql in [
            "select 1",
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
    fn lo_que_no_es_ore_sql() {
        for (sql, dice) in [
            ("create table t (v int)", "`CREATE TABLE` no existe"),
            ("create table t as select 1", "`CREATE TABLE` no existe"),
            ("drop table t", "`DROP TABLE` no existe"),
            ("create dataset d select 1", "AS <consulta> o (<columnas>)"),
            (
                "create or replace dataset d (v int)",
                "un dataset vacío no se reemplaza",
            ),
            ("create dataset d (v int, primary key (v))", "restricciones"),
            (
                "create or replace dataset if not exists d as select 1",
                "no van juntas",
            ),
            (
                "create or replace view if not exists v as select 1",
                "no van juntas",
            ),
            ("create or replace schema s", "un schema no se reemplaza"),
            ("create or replace database b", "una base no se reemplaza"),
            ("create foreign database b", "se lee en su origen"),
            ("create view v select 1", "Expected"),
            ("create view v with schema as select 1", "EVOLUTION"),
            (
                "create media collection c media image as select 1",
                "FORMATS",
            ),
            ("drop view v cascade", "`cascade` es de"),
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
