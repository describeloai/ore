//! **El guion**: un `.sql` con varias sentencias, que corren en orden en la
//! misma sesión, como en el editor de SQL de Databricks —y en nuestro
//! paradigma—.
//!
//! Un guion no es una semántica nueva: es **una lista de unidades**. Cada
//! sentencia que lee o escribe datos es la [`Unidad`] de siempre —lo que lee,
//! lo que escribe, su modo, su procedencia—, y cada una que crea algo del
//! catálogo es un verbo que ya existe (el alta de una base, `ore package
//! schema new`, el `createTable` de `/v1`). Lo que sí es del guion:
//!
//! - **partirlo** con el tokenizador —un `;` dentro de una cadena o de un
//!   comentario no corta— sin perder el sitio: cada sentencia se analiza con lo
//!   de alrededor en blanco, así que sus líneas y columnas son las del fichero;
//! - **cotejarlo en orden** ([`cotejar_guion`]): lo que crea la sentencia 1 —una
//!   base, un schema, un dataset— existe para la 2.
//!
//! # Lo que crea
//!
//! | la frase | es |
//! |---|---|
//! | `create [standard] database [if not exists] b` | una standard database vacía: sus datasets se escriben desde SQL |
//! | `create standard database b from origin o include (s.t, s.*)` | una standard database sobre un origen: se copia al lago |
//! | `create foreign database b from origin o include (…)` | una foreign database: se lee en el origen |
//! | `create schema [if not exists] b.s` | un schema declarado de la base |
//! | `create dataset [if not exists] b.s.d (col tipo, …)` | un Dataset vacío, con su esquema, en el lago |
//!
//! Y `create table` se niega: una **Table** es un puntero a un objeto de un
//! origen, nace del descubrimiento y no guarda bytes; lo que se escribe es un
//! **Dataset**. Como `sqlparser` no conoce `dataset`, la palabra se cambia por
//! `table  ` —el mismo largo, así que ninguna posición se mueve— antes de
//! analizar, y se recuerda que era un dataset.

use super::*;
use sqlparser::tokenizer::{Location, Token, Tokenizer};

/// De qué clase es una base (la standard o la foreign database de la consola).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clase {
    /// Sus datos viven en el lago.
    Standard,
    /// Se lee en su origen.
    Foreign,
}

impl Clase {
    /// Como la dice el alta de una base (`POST /paquetes {type}`).
    pub const fn como_en_el_alta(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Foreign => "foreign",
        }
    }
}

/// `from origin o include (…)`: de qué origen sale una base, y qué objetos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origen {
    pub nombre: String,
    pub pos: Option<Pos>,
    /// `s.t`, o `s.*` para un schema entero del origen.
    pub incluye: Vec<String>,
}

/// Una columna de `create dataset … (col tipo, …)`, con el tipo ya en Iceberg.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Columna {
    pub nombre: String,
    /// El tipo de Iceberg (`long`, `string`, `timestamp`, `decimal(10, 2)`…).
    pub tipo: String,
    pub pos: Option<Pos>,
}

/// Lo que una sentencia del guion es.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sentencia {
    /// Lee o escribe datos: la unidad de siempre.
    Unidad(Unidad),
    CrearBase {
        nombre: String,
        pos: Option<Pos>,
        clase: Clase,
        origen: Option<Origen>,
        si_no_existe: bool,
    },
    CrearSchema {
        base: String,
        schema: String,
        pos: Option<Pos>,
        si_no_existe: bool,
    },
    CrearDataset {
        destino: Nombre,
        columnas: Vec<Columna>,
        /// `primary key (…)`: la clave del dataset, la que usa un `insert or
        /// replace` (upsert). Vacía si no la dice.
        clave: Vec<String>,
        si_no_existe: bool,
    },
}

impl Sentencia {
    /// Lo que es, en dos palabras: lo que la consola pone junto a su resultado.
    pub fn que(&self) -> &'static str {
        match self {
            Self::Unidad(u) if u.escribe.is_none() => "select",
            Self::Unidad(u) => match u.escribe.as_ref().map(|e| e.modo) {
                Some(Modo::Sobrescribir) => "create or replace dataset",
                Some(Modo::Upsert) => "insert or replace",
                _ => "insert",
            },
            Self::CrearBase { .. } => "create database",
            Self::CrearSchema { .. } => "create schema",
            Self::CrearDataset { .. } => "create dataset",
        }
    }
}

/// Una sentencia del guion, con su texto y dónde empieza.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trozo {
    /// Tal como se escribió, sin el `;`.
    pub texto: String,
    /// Dónde empieza en el fichero (su primer token).
    pub pos: Option<Pos>,
    pub sentencia: Sentencia,
    /// Lo que se dice sin pararla (`ORE-SQL-2P`).
    pub avisos: Vec<Fallo>,
}

/// Lo que las sentencias de antes del guion ya crearon (en forma corta los
/// datasets): existe para las que vienen detrás.
#[derive(Debug, Clone, Default)]
pub struct Creado {
    pub bases: BTreeSet<String>,
    pub schemas: BTreeSet<(String, String)>,
    pub datasets: BTreeSet<String>,
}

const LA_BASE: &str =
    "`create [standard|foreign] database [if not exists] b [from origin o include (s.t, s.*)]`";

/// **Parte y analiza un guion.** Todos los fallos de todas las sentencias a la
/// vez; si el texto ni se tokeniza (una cadena sin cerrar), uno.
pub fn guion(texto: &str) -> Result<Vec<Trozo>, Vec<Fallo>> {
    let piezas = partir(texto)?;
    if piezas.is_empty() {
        return Err(vec![
            Fallo::new("el `.sql` está vacío", None).ayuda(LO_QUE_PUEDE_SER),
        ]);
    }
    let mut trozos = Vec::new();
    let mut fallos = Vec::new();
    for p in piezas {
        match sentencia(&p.texto) {
            Ok((sentencia, avisos)) => trozos.push(Trozo {
                texto: p.original,
                pos: p.pos,
                sentencia,
                avisos,
            }),
            Err(f) => fallos.extend(f),
        }
    }
    if fallos.is_empty() {
        Ok(trozos)
    } else {
        Err(fallos)
    }
}

/// Una sentencia del texto: el texto entero con lo que no es ella en blanco
/// (los saltos de línea se quedan), y la sentencia tal cual.
struct Pieza {
    texto: String,
    original: String,
    pos: Option<Pos>,
}

/// Dónde empieza cada línea, en bytes.
fn inicios_de_linea(texto: &str) -> Vec<usize> {
    std::iter::once(0)
        .chain(texto.match_indices('\n').map(|(i, _)| i + 1))
        .collect()
}

/// El byte de una posición del tokenizador (línea y columna en caracteres,
/// desde 1). Una columna más allá del final de la línea es su final.
fn byte_de(texto: &str, lineas: &[usize], l: Location) -> usize {
    let Some(&inicio) = (l.line as usize).checked_sub(1).and_then(|i| lineas.get(i)) else {
        return texto.len();
    };
    let linea = texto[inicio..].split('\n').next().unwrap_or("");
    let col = (l.column as usize).saturating_sub(1);
    inicio
        + linea
            .char_indices()
            .nth(col)
            .map_or(linea.len(), |(i, _)| i)
}

fn partir(texto: &str) -> Result<Vec<Pieza>, Vec<Fallo>> {
    let toks = Tokenizer::new(&DuckDbDialect {}, texto)
        .tokenize_with_location()
        .map_err(|e| vec![fallo_de_analisis(&e.to_string())])?;
    let lineas = inicios_de_linea(texto);
    let mut out = Vec::new();
    let mut desde = 0;
    // la posición de su primer token, si ya tiene alguno que no sea blanco
    let mut primera: Option<Option<Pos>> = None;
    let cerrar = |a: usize, b: usize, primera: Option<Option<Pos>>, out: &mut Vec<Pieza>| {
        if let Some(pos) = primera {
            out.push(Pieza {
                texto: en_blanco(texto, a, b),
                original: texto[a..b].trim().to_string(),
                pos,
            });
        }
    };
    for t in &toks {
        match &t.token {
            Token::SemiColon => {
                let b = byte_de(texto, &lineas, t.span.start);
                cerrar(desde, b, primera.take(), &mut out);
                desde = (b + 1).min(texto.len());
            }
            Token::Whitespace(_) => {}
            _ => {
                if primera.is_none() {
                    primera = Some(pos_de(t.span.start));
                }
            }
        }
    }
    cerrar(desde, texto.len(), primera, &mut out);
    Ok(out)
}

/// El texto con todo lo que no está en `a..b` en blanco, salvo los saltos de
/// línea: cada carácter, un espacio, así que líneas y columnas no se mueven.
fn en_blanco(texto: &str, a: usize, b: usize) -> String {
    texto
        .char_indices()
        .map(|(i, c)| {
            if (a..b).contains(&i) || c == '\n' || c == '\r' {
                c
            } else {
                ' '
            }
        })
        .collect()
}

/// Un token que cuenta (sin blancos ni comentarios), con su sitio.
struct Tok {
    t: Token,
    pos: Option<Pos>,
    loc: Location,
}

fn tokens(texto: &str) -> Result<Vec<Tok>, Vec<Fallo>> {
    Ok(Tokenizer::new(&DuckDbDialect {}, texto)
        .tokenize_with_location()
        .map_err(|e| vec![fallo_de_analisis(&e.to_string())])?
        .into_iter()
        .filter(|t| !matches!(t.token, Token::Whitespace(_)))
        .map(|t| Tok {
            pos: pos_de(t.span.start),
            loc: t.span.start,
            t: t.token,
        })
        .collect())
}

/// ¿Es `ts[i]` la palabra clave `k` (sin comillas)?
fn es(ts: &[Tok], i: usize, k: &str) -> bool {
    matches!(ts.get(i), Some(Tok { t: Token::Word(w), .. })
        if w.quote_style.is_none() && w.value.eq_ignore_ascii_case(k))
}

fn pos_en(ts: &[Tok], i: usize) -> Option<Pos> {
    ts.get(i).or(ts.last()).and_then(|t| t.pos)
}

/// Un nombre con puntos desde `i`: sus partes, dónde empieza y el token que
/// le sigue.
fn nombre_en(ts: &[Tok], mut i: usize) -> Option<(Vec<String>, Option<Pos>, usize)> {
    let palabra = |i: usize| match ts.get(i) {
        Some(Tok {
            t: Token::Word(w), ..
        }) => Some(w.value.clone()),
        _ => None,
    };
    let pos = pos_en(ts, i);
    let mut partes = vec![palabra(i)?];
    i += 1;
    while matches!(ts.get(i).map(|t| &t.t), Some(Token::Period)) {
        partes.push(palabra(i + 1)?);
        i += 2;
    }
    Some((partes, pos, i))
}

/// `if not exists` en `i`: si está, y dónde sigue.
fn si_no_existe(ts: &[Tok], i: usize) -> (bool, usize) {
    if es(ts, i, "if") && es(ts, i + 1, "not") && es(ts, i + 2, "exists") {
        (true, i + 3)
    } else {
        (false, i)
    }
}

fn sobra(ts: &[Tok], i: usize, forma: &str) -> Fallo {
    Fallo::new(format!("sobra `{}`", ts[i].t), ts[i].pos).ayuda(forma.to_string())
}

/// Una identificación de una parte que puede ser un espacio de nombres.
fn nombre_de_base(n: &str) -> bool {
    crate::pertenencia::puede_ser_namespace(n)
}

/// Lo que es una sentencia (su texto, con lo de alrededor en blanco).
fn sentencia(texto: &str) -> Result<(Sentencia, Vec<Fallo>), Vec<Fallo>> {
    let ts = tokens(texto)?;
    let mut es_dataset = false;
    let mut texto = std::borrow::Cow::Borrowed(texto);
    if es(&ts, 0, "create") {
        let (clase, i) = if es(&ts, 1, "standard") {
            (Some(Clase::Standard), 2)
        } else if es(&ts, 1, "foreign") {
            (Some(Clase::Foreign), 2)
        } else {
            (None, 1)
        };
        if es(&ts, i, "database") {
            return crear_base(&ts, i + 1, clase).map(|s| (s, Vec::new()));
        }
        if clase.is_none() && es(&ts, 1, "schema") {
            return crear_schema(&ts, 2).map(|s| (s, Vec::new()));
        }
        let mut j = 1;
        if es(&ts, j, "or") && es(&ts, j + 1, "replace") {
            j += 2;
        }
        if es(&ts, j, "temp") || es(&ts, j, "temporary") {
            j += 1;
        }
        if es(&ts, j, "table") {
            return Err(vec![tabla_no(ts[j].pos)]);
        }
        if es(&ts, j, "dataset") {
            // `dataset` → `table  `: el mismo largo, nada se mueve.
            let lineas = inicios_de_linea(&texto);
            let b = byte_de(&texto, &lineas, ts[j].loc);
            let mut t = texto.into_owned();
            t.replace_range(b..b + "dataset".len(), "table  ");
            texto = std::borrow::Cow::Owned(t);
            es_dataset = true;
        }
    }
    let s = match Parser::parse_sql(&DuckDbDialect {}, &texto) {
        Ok(mut v) if !v.is_empty() => v.remove(0),
        Ok(_) => return Err(vec![Fallo::new("la sentencia está vacía", None)]),
        Err(e) => return Err(vec![fallo_de_analisis(&e.to_string())]),
    };
    match &s {
        Statement::CreateTable(c) if !es_dataset => Err(vec![tabla_no(pos_de_nombre(&c.name))]),
        Statement::CreateTable(c) if c.query.is_none() && !c.columns.is_empty() => crear_dataset(c),
        _ => unidad_de(&s, &texto).map(|u| {
            let avisos = u.avisos.clone();
            (Sentencia::Unidad(u), avisos)
        }),
    }
}

fn tabla_no(pos: Option<Pos>) -> Fallo {
    Fallo::new(
        "una Table es un puntero a un objeto de un origen: nace del descubrimiento, no de SQL, y no guarda bytes. Lo que se escribe es un Dataset",
        pos,
    )
    .ayuda("`create or replace dataset b.s.d as select …`, o vacío con sus columnas: `create dataset b.s.d (id bigint, …)`")
}

/// `create [standard|foreign] database …` desde `i` (tras `database`).
fn crear_base(ts: &[Tok], i: usize, clase: Option<Clase>) -> Result<Sentencia, Vec<Fallo>> {
    let (si_no_existe, i) = si_no_existe(ts, i);
    let Some((partes, pos, mut i)) = nombre_en(ts, i) else {
        return Err(vec![
            Fallo::new("falta el nombre de la base", pos_en(ts, i)).ayuda(LA_BASE),
        ]);
    };
    let mut fallos = Vec::new();
    let nombre = partes.join(".");
    if partes.len() != 1 {
        fallos.push(
            Fallo::new(
                format!("`{nombre}` no es una base: una base es un nombre de una parte"),
                pos,
            )
            .ayuda(format!("¿un schema? `create schema {nombre}`")),
        );
    } else if !nombre_de_base(&nombre) {
        fallos.push(Fallo::new(
            format!("`{nombre}` no puede ser una base: una letra y luego letras, dígitos y `_` (es el espacio de nombres de lo que contenga)"),
            pos,
        ));
    }
    let mut origen = None;
    if es(ts, i, "from") && es(ts, i + 1, "origin") {
        let Some((o, opos, j)) = nombre_en(ts, i + 2) else {
            return Err(vec![
                Fallo::new("falta el nombre del origen", pos_en(ts, i + 2)).ayuda(LA_BASE),
            ]);
        };
        i = j;
        let mut incluye = Vec::new();
        if es(ts, i, "include") {
            i += 1;
            if !matches!(ts.get(i).map(|t| &t.t), Some(Token::LParen)) {
                return Err(vec![
                    Fallo::new("`include` va seguido de `(…)`", pos_en(ts, i)).ayuda(LA_BASE),
                ]);
            }
            i += 1;
            loop {
                let objeto = match (ts.get(i), ts.get(i + 1), ts.get(i + 2)) {
                    (
                        Some(Tok {
                            t: Token::Word(s), ..
                        }),
                        Some(Tok {
                            t: Token::Period, ..
                        }),
                        Some(Tok {
                            t: Token::Word(t), ..
                        }),
                    ) => format!("{}.{}", s.value, t.value),
                    (
                        Some(Tok {
                            t: Token::Word(s), ..
                        }),
                        Some(Tok {
                            t: Token::Period, ..
                        }),
                        Some(Tok { t: Token::Mul, .. }),
                    ) => format!("{}.*", s.value),
                    _ => {
                        return Err(vec![
                            Fallo::new(
                                "un objeto del origen es `schema.objeto`, o `schema.*` para el schema entero",
                                pos_en(ts, i),
                            )
                            .ayuda(LA_BASE),
                        ]);
                    }
                };
                incluye.push(objeto);
                i += 3;
                match ts.get(i).map(|t| &t.t) {
                    Some(Token::Comma) => i += 1,
                    Some(Token::RParen) => {
                        i += 1;
                        break;
                    }
                    _ => {
                        return Err(vec![
                            Fallo::new("falta `)` o `,` en `include (…)`", pos_en(ts, i))
                                .ayuda(LA_BASE),
                        ]);
                    }
                }
            }
        }
        if incluye.is_empty() {
            fallos.push(
                Fallo::new(
                    "una base sobre un origen dice qué objetos entran: `include (s.t, s.*)`",
                    opos,
                )
                .ayuda(LA_BASE),
            );
        }
        if o.len() != 1 {
            fallos.push(Fallo::new(
                format!(
                    "`{}` no es un origen: un origen es un nombre de una parte",
                    o.join(".")
                ),
                opos,
            ));
        }
        origen = Some(Origen {
            nombre: o.join("."),
            pos: opos,
            incluye,
        });
    }
    if i < ts.len() {
        fallos.push(sobra(ts, i, LA_BASE));
    }
    let clase = match (clase, &origen) {
        (Some(c), _) => c,
        (None, None) => Clase::Standard,
        (None, Some(o)) => {
            fallos.push(
                Fallo::new(
                    format!("una base sobre el origen `{}` es `standard` (se copia al lago) o `foreign` (se lee en el origen): dilo", o.nombre),
                    pos,
                )
                .ayuda(format!("`create standard database {nombre} from origin …` o `create foreign database {nombre} from origin …`")),
            );
            Clase::Standard
        }
    };
    if clase == Clase::Foreign && origen.is_none() {
        fallos.push(
            Fallo::new(
                "una foreign database se lee en su origen: `from origin o include (…)`",
                pos,
            )
            .ayuda(LA_BASE),
        );
    }
    if !fallos.is_empty() {
        return Err(fallos);
    }
    Ok(Sentencia::CrearBase {
        nombre,
        pos,
        clase,
        origen,
        si_no_existe,
    })
}

/// `create schema [if not exists] b.s` desde `i` (tras `schema`).
fn crear_schema(ts: &[Tok], i: usize) -> Result<Sentencia, Vec<Fallo>> {
    const FORMA: &str = "`create schema [if not exists] base.schema`";
    let (si_no_existe, i) = si_no_existe(ts, i);
    let Some((partes, pos, i)) = nombre_en(ts, i) else {
        return Err(vec![
            Fallo::new("falta el nombre del schema", pos_en(ts, i)).ayuda(FORMA),
        ]);
    };
    let mut fallos = Vec::new();
    if i < ts.len() {
        fallos.push(sobra(ts, i, FORMA));
    }
    let (base, schema) = match partes.as_slice() {
        [b, s] => (b.clone(), s.clone()),
        [s] => {
            fallos.push(
                Fallo::new(format!("`{s}` no dice de qué base es"), pos)
                    .ayuda(format!("`create schema base.{s}`")),
            );
            return Err(fallos);
        }
        _ => {
            fallos.push(
                Fallo::new(
                    format!("`{}` no es un schema: es `base.schema`", partes.join(".")),
                    pos,
                )
                .ayuda(FORMA),
            );
            return Err(fallos);
        }
    };
    let forma = schema.len() <= 128
        && schema.starts_with(|c: char| c.is_ascii_alphabetic())
        && schema
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !forma {
        fallos.push(Fallo::new(
            format!("`{schema}` no puede ser un schema: una letra y luego letras, cifras o `_` (hasta 128)"),
            pos,
        ));
    } else if crate::schema::RESERVADOS.contains(&schema.to_ascii_lowercase().as_str()) {
        fallos.push(Fallo::new(
            format!("`{schema}` está reservado: `default` existe sin declararse e `information_schema` es de SQL"),
            pos,
        ));
    }
    if !fallos.is_empty() {
        return Err(fallos);
    }
    Ok(Sentencia::CrearSchema {
        base,
        schema,
        pos,
        si_no_existe,
    })
}

/// `create dataset [if not exists] b.s.d (col tipo, …)`.
fn crear_dataset(c: &CreateTable) -> Result<(Sentencia, Vec<Fallo>), Vec<Fallo>> {
    let pos = pos_de_nombre(&c.name);
    let mut fallos = Vec::new();
    if c.or_replace {
        fallos.push(
            Fallo::new(
                "`create or replace dataset … (columnas)` vaciaría el dataset: todavía no",
                pos,
            )
            .ayuda(
                "`create dataset if not exists … (…)`, o `create or replace dataset … as select …`",
            ),
        );
    }
    if c.temporary {
        fallos.push(Fallo::new(
            "un dataset temporal no sale de la sesión: usa un `with`",
            pos,
        ));
    }
    // La clave: `primary key (a, b)` de la tabla, o `a tipo primary key`.
    let mut clave: Vec<String> = Vec::new();
    for r in &c.constraints {
        match r {
            sqlparser::ast::TableConstraint::PrimaryKey(pk) if clave.is_empty() => {
                clave = pk.columns.iter().map(|x| x.to_string()).collect();
            }
            _ => fallos.push(Fallo::new(
                format!(
                    "`{r}`: de las restricciones, un dataset sólo sabe `primary key (…)`, su clave"
                ),
                pos,
            )),
        }
    }
    let mut columnas = Vec::new();
    for col in &c.columns {
        let cpos = pos_de(col.name.span.start);
        for o in &col.options {
            match &o.option {
                sqlparser::ast::ColumnOption::PrimaryKey(_) if clave.is_empty() => {
                    clave.push(col.name.value.clone())
                }
                otra => fallos.push(Fallo::new(
                    format!(
                        "`{}`: `{otra}` todavía no; una columna es su nombre y su tipo, y la clave, `primary key`",
                        col.name.value
                    ),
                    cpos,
                )),
            }
        }
        match tipo_iceberg(&col.data_type) {
            Some(tipo) => columnas.push(Columna {
                nombre: col.name.value.clone(),
                tipo,
                pos: cpos,
            }),
            None => fallos.push(
                Fallo::new(
                    format!(
                        "`{}`: `{}` no es un tipo que el lago sepa guardar",
                        col.name.value, col.data_type
                    ),
                    cpos,
                )
                .ayuda("bigint, int, string (varchar), boolean, double, float, decimal(p, s), date, time, timestamp, timestamptz, binary, uuid"),
            ),
        }
    }
    for k in &clave {
        if !columnas.iter().any(|c| &c.nombre == k) && !c.columns.iter().any(|x| &x.name.value == k)
        {
            fallos.push(Fallo::new(
                format!("la clave nombra `{k}`, que no es una columna del dataset"),
                pos,
            ));
        }
    }
    let destino = nombre_del_arbol(&c.name, &mut fallos);
    if !fallos.is_empty() {
        return Err(fallos);
    }
    let destino = destino.expect("sin fallos hay nombre");
    let avisos = if destino.dos_partes {
        vec![Fallo::dos_partes(&destino)]
    } else {
        Vec::new()
    };
    Ok((
        Sentencia::CrearDataset {
            destino,
            columnas,
            clave,
            si_no_existe: c.if_not_exists,
        },
        avisos,
    ))
}

/// **El tipo de Iceberg de un tipo de SQL**, en el dialecto de DuckDB (el que
/// corre la frase; el de Spark no se traduce). Lo que el lago no guarda
/// (listas, structs, intervalos, enteros sin signo) es `None`.
pub fn tipo_iceberg(t: &sqlparser::ast::DataType) -> Option<String> {
    let texto = t.to_string().to_ascii_uppercase();
    let (base, args) = match texto.split_once('(') {
        Some((b, r)) => (
            b.trim().to_string(),
            Some(r.trim_end_matches(')').to_string()),
        ),
        None => (texto.trim().to_string(), None),
    };
    let tipo = match base.as_str() {
        "BIGINT" | "INT8" | "LONG" | "INT64" => "long",
        "INT" | "INTEGER" | "INT4" | "INT32" | "SMALLINT" | "INT2" | "INT16" | "TINYINT"
        | "INT1" | "SIGNED" => "int",
        "STRING" | "VARCHAR" | "TEXT" | "CHAR" | "CHARACTER" | "CHARACTER VARYING" | "BPCHAR"
        | "NVARCHAR" => "string",
        "BOOLEAN" | "BOOL" | "LOGICAL" => "boolean",
        "DOUBLE" | "FLOAT8" | "DOUBLE PRECISION" | "FLOAT64" => "double",
        "FLOAT" | "FLOAT4" | "REAL" | "FLOAT32" => "float",
        "DATE" => "date",
        "TIME" => "time",
        "TIMESTAMP" | "DATETIME" | "TIMESTAMP WITHOUT TIME ZONE" => "timestamp",
        "TIMESTAMPTZ" | "TIMESTAMP WITH TIME ZONE" => "timestamptz",
        "BLOB" | "BYTEA" | "BINARY" | "VARBINARY" => "binary",
        "UUID" => "uuid",
        "DECIMAL" | "NUMERIC" | "DEC" => {
            // sin precisión, la de DuckDB: decimal(18, 3)
            let (p, s) = match args.as_deref().map(|a| {
                a.split(',')
                    .map(|x| x.trim().parse::<u32>().ok())
                    .collect::<Vec<_>>()
            }) {
                None => (18, 3),
                Some(v) => match v.as_slice() {
                    [Some(p)] => (*p, 0),
                    [Some(p), Some(s)] => (*p, *s),
                    _ => return None,
                },
            };
            if p == 0 || p > 38 || s > p {
                return None;
            }
            return Some(format!("decimal({p}, {s})"));
        }
        _ => return None,
    };
    Some(tipo.to_string())
}

/// **Coteja un guion con el árbol, en orden**: cada unidad como [`cotejar`], y
/// cada sentencia que crea, contra lo que ya hay; lo que crea una sentencia
/// existe para las siguientes. Lo que no es cosa del árbol —que el origen
/// tenga esos objetos, que el nombre del schema no sea una carpeta de kind—
/// lo dice el verbo al correr.
pub fn cotejar_guion(pkg: &Package, trozos: &[Trozo]) -> Vec<Fallo> {
    let mut creado = Creado::default();
    let mut fallos = Vec::new();
    let hay_base_o_creada = |c: &Creado, b: &str| hay_base(pkg, b) || c.bases.contains(b);
    let sin_base = |b: &str, pos: Option<Pos>| {
        Fallo::new(format!("no hay ninguna base `{b}` en el árbol"), pos).ayuda(format!(
            "créala antes en el guion: `create standard database {b}`"
        ))
    };
    for t in trozos {
        match &t.sentencia {
            Sentencia::Unidad(u) => {
                fallos.extend(cotejar_con(pkg, u, &creado));
                if let Some(e) = &u.escribe {
                    creado.datasets.insert(e.destino.referencia());
                }
            }
            Sentencia::CrearBase {
                nombre,
                pos,
                origen,
                si_no_existe,
                ..
            } => {
                if hay_base_o_creada(&creado, nombre) && !si_no_existe {
                    fallos.push(
                        Fallo::new(format!("ya hay una base `{nombre}`"), *pos).ayuda(format!(
                            "`… database if not exists {nombre}`, si da igual que ya esté"
                        )),
                    );
                }
                if let Some(o) = origen
                    && !hay_base(pkg, &o.nombre)
                {
                    fallos.push(Fallo::new(
                        format!("no hay ningún origen `{}` en el árbol", o.nombre),
                        o.pos,
                    ));
                }
                creado.bases.insert(nombre.clone());
            }
            Sentencia::CrearSchema {
                base,
                schema,
                pos,
                si_no_existe,
            } => {
                if !hay_base_o_creada(&creado, base) {
                    fallos.push(sin_base(base, *pos));
                } else if (hay_schema_declarado(pkg, base, schema)
                    || creado.schemas.contains(&(base.clone(), schema.clone())))
                    && !si_no_existe
                {
                    fallos.push(
                        Fallo::new(format!("ya hay un schema `{base}.{schema}`"), *pos).ayuda(
                            format!("`create schema if not exists {base}.{schema}`, si da igual que ya esté"),
                        ),
                    );
                }
                creado.schemas.insert((base.clone(), schema.clone()));
            }
            Sentencia::CrearDataset {
                destino,
                si_no_existe,
                ..
            } => {
                let r = destino.referencia();
                let ya = || {
                    Fallo::new(format!("ya hay un dataset `{r}`"), destino.pos).ayuda(format!(
                        "`create dataset if not exists {}`, si da igual que ya esté",
                        destino.completo()
                    ))
                };
                if !hay_base_o_creada(&creado, &destino.paquete) {
                    fallos.push(sin_base(&destino.paquete, destino.pos));
                } else if !(hay_schema_declarado(pkg, &destino.paquete, &destino.schema)
                    || creado
                        .schemas
                        .contains(&(destino.paquete.clone(), destino.schema.clone())))
                {
                    fallos.push(
                        Fallo::new(
                            format!(
                                "no hay ningún schema `{}` en la base `{}`",
                                destino.schema, destino.paquete
                            ),
                            destino.pos,
                        )
                        .ayuda(format!(
                            "créalo antes en el guion: `create schema {}.{}`",
                            destino.paquete, destino.schema
                        )),
                    );
                } else {
                    match doc_de(pkg, &r) {
                        Some(d) if d.kind == Kind::Dataset && d.section("from").is_none() => {
                            if !si_no_existe {
                                fallos.push(ya());
                            }
                        }
                        Some(d) if d.kind == Kind::Dataset => fallos.push(Fallo::new(
                            format!("`{r}` es un `Dataset` mantenido: sale de su `from`"),
                            destino.pos,
                        )),
                        Some(d) => fallos.push(Fallo::new(
                            format!("`{r}` ya es una `{:?}`", d.kind),
                            destino.pos,
                        )),
                        None if creado.datasets.contains(&r) && !si_no_existe => fallos.push(ya()),
                        None => {}
                    }
                }
                creado.datasets.insert(r);
            }
        }
    }
    fallos
}

#[cfg(test)]
mod tests {
    use super::*;

    const EJEMPLO: &str = "CREATE SCHEMA IF NOT EXISTS ventas.demo_uc;

CREATE DATASET IF NOT EXISTS ventas.demo_uc.clientes (
  id BIGINT,
  nombre STRING,
  email STRING,
  created_at TIMESTAMP
);

INSERT INTO ventas.demo_uc.clientes (id, nombre, email, created_at) VALUES
  (1, 'Ana López; la primera', 'ana@example.com', current_timestamp),
  (2, 'Bruno Díaz', 'bruno@example.com', current_timestamp);

-- y lo que queda; un comentario con punto y coma
SELECT * FROM ventas.demo_uc.clientes;
";

    fn trozos(q: &str) -> Vec<Trozo> {
        guion(q).unwrap_or_else(|f| panic!("{q}: {f:?}"))
    }

    fn falla(q: &str) -> Vec<Fallo> {
        guion(q).expect_err(q)
    }

    /// El guion del ejemplo: cuatro sentencias, cada una en su sitio, y un `;`
    /// dentro de una cadena o de un comentario no corta.
    #[test]
    fn el_ejemplo_son_cuatro_sentencias_en_su_sitio() {
        let t = trozos(EJEMPLO);
        assert_eq!(
            t.iter()
                .map(|t| (t.sentencia.que(), t.pos.map(|p| (p.line, p.col))))
                .collect::<Vec<_>>(),
            [
                ("create schema", Some((1, 1))),
                ("create dataset", Some((3, 1))),
                ("insert", Some((10, 1))),
                ("select", Some((15, 1)))
            ]
        );
        assert!(
            t[3].texto.starts_with("-- y lo que queda"),
            "{:?}",
            t[3].texto
        );
        match &t[0].sentencia {
            Sentencia::CrearSchema {
                base,
                schema,
                si_no_existe,
                ..
            } => assert_eq!(
                (base.as_str(), schema.as_str(), *si_no_existe),
                ("ventas", "demo_uc", true)
            ),
            s => panic!("{s:?}"),
        }
        match &t[1].sentencia {
            Sentencia::CrearDataset {
                destino,
                columnas,
                clave,
                si_no_existe,
            } => {
                assert!(clave.is_empty());
                assert_eq!(destino.referencia(), "ventas.demo_uc.clientes");
                assert_eq!(destino.pos.map(|p| (p.line, p.col)), Some((3, 30)));
                assert!(*si_no_existe);
                assert_eq!(
                    columnas
                        .iter()
                        .map(|c| (c.nombre.as_str(), c.tipo.as_str()))
                        .collect::<Vec<_>>(),
                    [
                        ("id", "long"),
                        ("nombre", "string"),
                        ("email", "string"),
                        ("created_at", "timestamp")
                    ]
                );
            }
            s => panic!("{s:?}"),
        }
        match &t[2].sentencia {
            Sentencia::Unidad(u) => {
                let e = u.escribe.as_ref().unwrap();
                assert_eq!(
                    (e.destino.referencia(), e.modo),
                    ("ventas.demo_uc.clientes".into(), Modo::Anexar)
                );
                assert!(e.por_posicion.is_empty());
                assert!(
                    u.consulta.starts_with("SELECT * FROM (VALUES")
                        && u.consulta
                            .ends_with(") AS v(id, nombre, email, created_at)"),
                    "{}",
                    u.consulta
                );
                assert!(u.consulta.contains("'Ana López; la primera'"));
                assert!(u.lee.is_empty());
            }
            s => panic!("{s:?}"),
        }
    }

    #[test]
    fn los_fallos_dicen_su_sitio_en_el_fichero() {
        let f = falla("select 1;\n\nselect * from\n  ventas");
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].pos.map(|p| p.line), Some(4), "{f:?}");

        // todas las sentencias a la vez
        let f = falla("select * from a;\nselect * from b");
        assert_eq!(
            f.iter().map(|f| f.pos.map(|p| p.line)).collect::<Vec<_>>(),
            [Some(1), Some(2)]
        );
        assert!(falla("  ;\n -- nada\n;")[0].mensaje.contains("vacío"));
    }

    #[test]
    fn una_table_no_se_crea_desde_sql() {
        for q in [
            "create table hr.x (a int)",
            "create or replace table hr.x as select 1",
            "select 1;\ncreate table if not exists hr.x as select 1",
        ] {
            let f = falla(q);
            assert!(
                f[0].mensaje.contains("una Table es un puntero"),
                "{q}: {f:?}"
            );
            assert!(f[0].ayuda.as_deref().unwrap().contains("dataset"));
        }
        // y en su sitio: la palabra `table`
        assert_eq!(
            falla("select 1;\n  create table hr.x (a int)")[0].pos,
            Some(Pos { line: 2, col: 10 })
        );
    }

    #[test]
    fn un_dataset_se_crea_o_se_escribe() {
        // `dataset` → `table  `: la posición del nombre es la del fichero
        let t = trozos("create or replace dataset hr.s as\nselect * from hr.a");
        match &t[0].sentencia {
            Sentencia::Unidad(u) => {
                assert_eq!(u.consulta, "select * from hr.a");
                assert_eq!(
                    u.escribe.as_ref().unwrap().destino.pos,
                    Some(Pos { line: 1, col: 27 })
                );
            }
            s => panic!("{s:?}"),
        }
        let f = falla("create dataset hr.s as select 1");
        assert!(f[0].mensaje.contains("la segunda vez"), "{f:?}");
        let f = falla("create or replace dataset hr.s (a int)");
        assert!(f[0].mensaje.contains("vaciaría"), "{f:?}");
        let f = falla("create dataset hr.s (a int not null, b list, c decimal(10,2))");
        assert_eq!(f.len(), 2, "{f:?}");
        assert!(f[0].mensaje.contains("`NOT NULL` todavía no"), "{f:?}");
        assert!(f[1].mensaje.contains("`b`"), "{f:?}");
        // la clave: de la tabla o de la columna
        for q in [
            "create dataset hr.s (id bigint, n string, primary key (id))",
            "create dataset hr.s (id bigint primary key, n string)",
        ] {
            match trozos(q).remove(0).sentencia {
                Sentencia::CrearDataset { clave, .. } => assert_eq!(clave, ["id"], "{q}"),
                s => panic!("{s:?}"),
            }
        }
        let f = falla("create dataset hr.s (id bigint, primary key (otra))");
        assert!(f[0].mensaje.contains("`otra`"), "{f:?}");
        // dos partes: `default`, con su aviso
        let t = trozos("create dataset hr.s (a decimal(10, 2), b timestamptz, c varchar(3))");
        assert_eq!(t[0].avisos.len(), 1);
        match &t[0].sentencia {
            Sentencia::CrearDataset { columnas, .. } => assert_eq!(
                columnas.iter().map(|c| c.tipo.as_str()).collect::<Vec<_>>(),
                ["decimal(10, 2)", "timestamptz", "string"]
            ),
            s => panic!("{s:?}"),
        }
    }

    #[test]
    fn un_insert_con_columnas_o_con_values() {
        let u = |q: &str| match trozos(q).remove(0).sentencia {
            Sentencia::Unidad(u) => u,
            s => panic!("{s:?}"),
        };
        let x = u("insert into hr.x values (1, 'a'), (2, 'b')");
        assert_eq!(x.consulta, "SELECT * FROM (VALUES (1, 'a'), (2, 'b')) AS v");
        assert_eq!(x.escribe.unwrap().por_posicion, [0, 1]);
        let x = u("insert into hr.x (b, a) select a, b from hr.y");
        assert_eq!(
            x.consulta,
            "SELECT * FROM (select a, b from hr.y) AS v(b, a)"
        );
        assert!(x.escribe.unwrap().por_posicion.is_empty());
        assert_eq!(
            x.lee.iter().map(Nombre::referencia).collect::<Vec<_>>(),
            ["hr.y"]
        );
    }

    #[test]
    fn la_base_y_el_schema() {
        let s = |q: &str| trozos(q).remove(0).sentencia;
        assert_eq!(
            s("create database if not exists mi_base"),
            Sentencia::CrearBase {
                nombre: "mi_base".into(),
                pos: Some(Pos { line: 1, col: 31 }),
                clase: Clase::Standard,
                origen: None,
                si_no_existe: true
            }
        );
        match s(
            "CREATE FOREIGN DATABASE espejo FROM ORIGIN erp INCLUDE (ventas.pedidos, \"RRHH\".*)",
        ) {
            Sentencia::CrearBase {
                clase,
                origen: Some(o),
                ..
            } => {
                assert_eq!(clase, Clase::Foreign);
                assert_eq!(o.nombre, "erp");
                assert_eq!(o.incluye, ["ventas.pedidos", "RRHH.*"]);
            }
            x => panic!("{x:?}"),
        }
        assert!(matches!(
            s("create standard database copia from origin erp include (ventas.*)"),
            Sentencia::CrearBase {
                clase: Clase::Standard,
                origen: Some(_),
                ..
            }
        ));
        for (q, dice) in [
            ("create foreign database espejo", "se lee en su origen"),
            (
                "create database b from origin erp include (s.t)",
                "`standard`",
            ),
            ("create standard database b from origin erp", "include"),
            ("create database ventas.espana", "una parte"),
            ("create database mi-base", "sobra"),
            (
                "create database b from origin erp include (t)",
                "schema.objeto",
            ),
            ("create schema espana", "no dice de qué base"),
            ("create schema ventas.default", "reservado"),
            ("create schema ventas.espana otra", "sobra"),
        ] {
            let f = falla(q);
            assert!(f.iter().any(|f| f.mensaje.contains(dice)), "{q}: {f:?}");
        }
        assert!(matches!(
            s("create schema if not exists ventas.espana"),
            Sentencia::CrearSchema {
                si_no_existe: true,
                ..
            }
        ));
    }

    #[test]
    fn los_tipos_del_lago() {
        let t = |q: &str| match trozos(&format!("create dataset b.s.d (c {q})"))
            .remove(0)
            .sentencia
        {
            Sentencia::CrearDataset { columnas, .. } => columnas[0].tipo.clone(),
            s => panic!("{s:?}"),
        };
        assert_eq!(t("bigint"), "long");
        assert_eq!(t("INTEGER"), "int");
        assert_eq!(t("text"), "string");
        assert_eq!(t("double"), "double");
        assert_eq!(t("date"), "date");
        assert_eq!(t("timestamp with time zone"), "timestamptz");
        assert_eq!(t("decimal"), "decimal(18, 3)");
        assert_eq!(t("numeric(9)"), "decimal(9, 0)");
        assert_eq!(t("blob"), "binary");
        assert_eq!(t("uuid"), "uuid");
    }
}
