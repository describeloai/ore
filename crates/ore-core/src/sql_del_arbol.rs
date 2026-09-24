//! **El SQL del árbol**: un `.sql` es una unidad, y dice él solo qué lee, qué
//! escribe y cómo.
//!
//! Un transform en Python declara `@transform(inputs, output)` y luego hace lo
//! que dice. En SQL la declaración **ya está en la frase**: el destino es lo que
//! sigue a `create or replace table` o a `insert into`, y lo que se lee es lo que
//! aparece tras un `from` o un `join`. Escribirlo dos veces sería darle a la
//! declaración una oportunidad de mentir. Así que aquí no hay cabecera ni
//! plantilla: se analiza la frase y se saca la declaración.
//!
//! # Lo que un `.sql` puede ser
//!
//! **Una** sentencia —un fichero, una salida—, y una de estas cuatro:
//!
//! | la frase | es | modo de `write()` |
//! |---|---|---|
//! | `select …` (o `from … select`, `with …`) | un análisis: lee y no escribe | — |
//! | `create or replace table p.t as select …` | un transform | `sobrescribir` |
//! | `insert into p.t select …` | un transform | `anexar` |
//! | `insert or replace into p.t select …` | un transform | `upsert` (con la clave de la tabla) |
//!
//! Y nada más, a propósito:
//!
//! - **`create table … as` sin `or replace`** se niega: un trabajo se vuelve a
//!   correr, y la segunda vez fallaría porque la tabla ya existe.
//! - **Un nombre se escribe `paquete.nombre`**: sin paquete no se sabe de quién
//!   es (salvo que sea un `with`), y con tres partes no es un nombre del árbol.
//! - **Se lee por nombre, nunca por función**: `read_parquet('gs://…')` o
//!   `iceberg_scan(…)` leen bytes que el árbol no nombra —sin linaje, sin
//!   conducto—. Los que generan filas sin leer nada (`range`,
//!   `generate_series`, `unnest`) sí valen.
//! - **Lo que se escribe no se lee**: un transform con la misma tabla de
//!   entrada y de salida es lo que `@transform` ya niega.
//!
//! # El dialecto
//!
//! DuckDB, que es el motor que hoy corre la frase (`sql()` en los tres
//! puestos). Medido en `pruebas-de-fuego/medida-el-sql-del-arbol.py`: con
//! `sqlparser` y dialecto DuckDB, 18 de 19 casos, y el que falta es
//! `insert … by name`. El dialecto es un dato del analizador, no de la unidad:
//! si otro motor corre la frase mañana, la regla de arriba no cambia.
//!
//! # Dos pasos, y el segundo necesita el árbol
//!
//! [`analizar`] sólo mira la frase: saca los nombres con su posición y dice si
//! es una unidad. [`cotejar`] los pone contra el árbol compilado: si lo que se
//! lee **se puede leer** y lo que se escribe **se puede escribir**. Si los
//! bytes están, lo dice el puntero al correr; no es cosa de ninguno de los dos.

use sqlparser::ast::{
    CreateTable, Ident, Insert, ObjectName, ObjectNamePart, Query, SetExpr, SqliteOnConflict,
    Statement, TableFactor, TableObject, Visit, Visitor,
};
use sqlparser::dialect::DuckDbDialect;
use sqlparser::parser::Parser;
use std::collections::BTreeSet;
use std::ops::ControlFlow;

use crate::diag::Pos;
use crate::document::Kind;
use crate::link::Package;
use crate::vistas;

/// Cómo se escribe lo que sale: los tres modos de `write()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modo {
    Sobrescribir,
    Anexar,
    Upsert,
}

impl Modo {
    /// El nombre del modo tal como lo recibe `write()` en los tres SDK.
    pub const fn como_en_write(self) -> &'static str {
        match self {
            Self::Sobrescribir => "sobrescribir",
            Self::Anexar => "anexar",
            Self::Upsert => "upsert",
        }
    }
}

/// Un nombre del árbol, `paquete.nombre`, y dónde aparece por primera vez.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nombre {
    pub paquete: String,
    pub nombre: String,
    pub pos: Option<Pos>,
}

impl Nombre {
    pub fn referencia(&self) -> String {
        format!("{}.{}", self.paquete, self.nombre)
    }
}

/// Lo que se escribe, y cómo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Escritura {
    pub destino: Nombre,
    pub modo: Modo,
    /// `insert … into p.t select …`: las columnas del `select` (por su
    /// posición, desde 0) que son una expresión SIN alias. Lo que se escribe
    /// va por nombre; éstas no lo tienen, y toman el de la columna de la
    /// tabla en su misma posición, como en SQL (decidido 2026-09-24). Las de
    /// antes de un `*` sólo: tras él, la posición ya no se sabe.
    pub por_posicion: Vec<usize>,
}

/// La declaración que la frase lleva dentro.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unidad {
    /// Lo que se lee, sin repetir, en el orden en que aparece.
    pub lee: Vec<Nombre>,
    /// `None` si es un análisis.
    pub escribe: Option<Escritura>,
    /// **La consulta que produce lo que sale**, tal como está escrita: la
    /// frase entera si es un análisis, y lo que sigue a `… as` o a `insert
    /// into p.t` si escribe. Es lo que se le pasa a `sql()`; el destino y el
    /// modo van a `write()`. Se corta del texto —no se reimprime desde el
    /// árbol sintáctico— para que corra exactamente lo que se escribió.
    pub consulta: String,
}

/// Por qué la frase no es una unidad, y dónde mirar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fallo {
    pub mensaje: String,
    pub pos: Option<Pos>,
    pub ayuda: Option<String>,
}

impl Fallo {
    fn new(mensaje: impl Into<String>, pos: Option<Pos>) -> Self {
        Self {
            mensaje: mensaje.into(),
            pos,
            ayuda: None,
        }
    }

    fn ayuda(mut self, a: impl Into<String>) -> Self {
        self.ayuda = Some(a.into());
        self
    }
}

/// Las funciones que generan filas sin leer nada: no rompen el linaje.
const GENERADORAS: [&str; 3] = ["range", "generate_series", "unnest"];

const LO_QUE_PUEDE_SER: &str = "un `.sql` del árbol es un `select` (lee), `create or replace table p.t as select …` (sobrescribe), `insert into p.t select …` (anexa) o `insert or replace into p.t select …` (upsert)";

/// Analiza el texto de un `.sql` y devuelve lo que declara. Todos los fallos a
/// la vez cuando se puede —un nombre mal escrito no esconde al siguiente—; uno
/// solo si la frase no analiza.
pub fn analizar(texto: &str) -> Result<Unidad, Vec<Fallo>> {
    let sentencias = match Parser::parse_sql(&DuckDbDialect {}, texto) {
        Ok(s) => s,
        Err(e) => return Err(vec![fallo_de_analisis(&e.to_string())]),
    };
    let mut sentencias = sentencias.into_iter();
    let Some(s) = sentencias.next() else {
        return Err(vec![
            Fallo::new("el `.sql` está vacío", None).ayuda(LO_QUE_PUEDE_SER),
        ]);
    };
    if let Some(otra) = sentencias.next() {
        return Err(vec![
            Fallo::new(
                "un `.sql` del árbol es UNA sentencia: un fichero, una salida",
                pos_de_sentencia(&otra),
            )
            .ayuda("parte el fichero en dos, o junta lo que calculas en un `with`"),
        ]);
    }

    let mut fallos = Vec::new();
    let escribe = match &s {
        Statement::Query(_) => None,
        Statement::CreateTable(c) => escritura_de_create(c, &mut fallos),
        Statement::Insert(i) => escritura_de_insert(i, &mut fallos),
        otra => {
            fallos.push(
                Fallo::new(
                    format!(
                        "`{}` no es una frase que el árbol sepa correr",
                        primera_palabra(otra)
                    ),
                    pos_de_sentencia(otra),
                )
                .ayuda(LO_QUE_PUEDE_SER),
            );
            None
        }
    };

    let mut lectura = Lectura::default();
    let _ = s.visit(&mut lectura);
    fallos.extend(lectura.fallos);

    let mut vistos = BTreeSet::new();
    let mut lee = Vec::new();
    for n in lectura.nombres {
        if vistos.insert(n.referencia()) {
            lee.push(n);
        }
    }
    if let Some(e) = &escribe
        && let Some(n) = lee
            .iter()
            .find(|n| n.referencia() == e.destino.referencia())
    {
        fallos.push(
            Fallo::new(
                format!(
                    "`{}` se escribe y se lee a la vez: un transform no lee lo que escribe",
                    n.referencia()
                ),
                n.pos,
            )
            .ayuda("lee de la tabla de la que sale, no de la que produce"),
        );
    }

    let consulta = match &s {
        Statement::Query(_) => Some(sin_punto_y_coma(texto)),
        Statement::CreateTable(c) => c.query.as_deref().and_then(|q| desde(texto, q)),
        Statement::Insert(i) => i.source.as_deref().and_then(|q| desde(texto, q)),
        _ => None,
    };
    if fallos.is_empty() {
        let Some(consulta) = consulta else {
            return Err(vec![Fallo::new(
                "no se encuentra dónde empieza la consulta de la frase",
                pos_de_sentencia(&s),
            )]);
        };
        Ok(Unidad {
            lee,
            escribe,
            consulta,
        })
    } else {
        Err(fallos)
    }
}

fn escritura_de_create(c: &CreateTable, fallos: &mut Vec<Fallo>) -> Option<Escritura> {
    let pos = pos_de_nombre(&c.name);
    if c.query.is_none() {
        fallos.push(
            Fallo::new(
                "una tabla del árbol nace de lo que se escribe en ella, no de una lista de columnas",
                pos,
            )
            .ayuda("`create or replace table p.t as select …`"),
        );
        return None;
    }
    if c.temporary {
        fallos.push(Fallo::new(
            "una tabla temporal no sale de la sesión: usa un `with`",
            pos,
        ));
        return None;
    }
    if !c.or_replace {
        fallos.push(
            Fallo::new(
                "`create table … as` falla la segunda vez que corre: la tabla ya existe",
                pos,
            )
            .ayuda("`create or replace table … as`: sobrescribe, que es lo que un trabajo repetido tiene que hacer"),
        );
        return None;
    }
    let destino = nombre_del_arbol(&c.name, fallos)?;
    Some(Escritura {
        destino,
        modo: Modo::Sobrescribir,
        por_posicion: Vec::new(),
    })
}

fn escritura_de_insert(i: &Insert, fallos: &mut Vec<Fallo>) -> Option<Escritura> {
    let TableObject::TableName(nombre) = &i.table else {
        fallos.push(Fallo::new(
            "se escribe en una tabla por su nombre, no en una función",
            None,
        ));
        return None;
    };
    let pos = pos_de_nombre(nombre);
    let modo = match i.or {
        None if !i.replace_into && !i.ignore => Modo::Anexar,
        Some(SqliteOnConflict::Replace) => Modo::Upsert,
        _ => {
            fallos.push(
                Fallo::new("de los `insert or …`, el árbol sólo sabe `or replace`", pos).ayuda(
                    "`insert or replace into p.t select …`: upsert con la clave de la tabla",
                ),
            );
            return None;
        }
    };
    if i.returning.is_some() || i.on.is_some() {
        fallos.push(
            Fallo::new(
                "un `insert` del árbol termina en su `select`: sin `returning` ni `on conflict`",
                pos,
            )
            .ayuda("`insert or replace into p.t select …` para el upsert"),
        );
        return None;
    }
    if !i.columns.is_empty() {
        fallos.push(
            Fallo::new(
                "lo que se escribe es lo que el `select` devuelve, con sus nombres: sin lista de columnas",
                pos,
            )
            .ayuda("pon los nombres como alias en el `select`"),
        );
        return None;
    }
    match i.source.as_deref() {
        Some(q) if !matches!(*q.body, SetExpr::Values(_)) => {}
        _ => {
            fallos.push(
                Fallo::new(
                    "un `insert` del árbol escribe lo que sale de un `select`",
                    pos,
                )
                .ayuda("`insert into p.t select …`"),
            );
            return None;
        }
    }
    let destino = nombre_del_arbol(nombre, fallos)?;
    let por_posicion = i.source.as_deref().map(sin_nombre).unwrap_or_default();
    Some(Escritura {
        destino,
        modo,
        por_posicion,
    })
}

/// Las columnas del `select` de un `insert` que son una expresión sin alias
/// (`0.5`, `sum(x)`), por su posición: una columna, un `t.col` o un `… as n`
/// tienen nombre. De un `union`, el primer `select` (el que da los nombres).
fn sin_nombre(q: &Query) -> Vec<usize> {
    use sqlparser::ast::{Expr, SelectItem};
    let mut cuerpo = q.body.as_ref();
    let proyeccion = loop {
        match cuerpo {
            SetExpr::Select(s) => break &s.projection,
            SetExpr::SetOperation { left, .. } => cuerpo = left.as_ref(),
            SetExpr::Query(q) => cuerpo = q.body.as_ref(),
            _ => return Vec::new(),
        }
    };
    let mut out = Vec::new();
    for (i, item) in proyeccion.iter().enumerate() {
        match item {
            SelectItem::UnnamedExpr(Expr::Identifier(_) | Expr::CompoundIdentifier(_))
            | SelectItem::ExprWithAlias { .. } => {}
            SelectItem::UnnamedExpr(_) => out.push(i),
            _ => break,
        }
    }
    out
}

/// Lo que se lee: cada tabla tras un `from` o un `join`, con los `with` de
/// cada consulta fuera.
#[derive(Default)]
struct Lectura {
    ctes: Vec<BTreeSet<String>>,
    nombres: Vec<Nombre>,
    fallos: Vec<Fallo>,
}

impl Lectura {
    fn es_cte(&self, n: &str) -> bool {
        self.ctes.iter().any(|s| s.contains(n))
    }
}

impl Visitor for Lectura {
    type Break = ();

    fn pre_visit_query(&mut self, q: &Query) -> ControlFlow<()> {
        let nombres = q
            .with
            .as_ref()
            .map(|w| {
                w.cte_tables
                    .iter()
                    .map(|c| c.alias.name.value.clone())
                    .collect()
            })
            .unwrap_or_default();
        self.ctes.push(nombres);
        ControlFlow::Continue(())
    }

    fn post_visit_query(&mut self, _q: &Query) -> ControlFlow<()> {
        self.ctes.pop();
        ControlFlow::Continue(())
    }

    fn pre_visit_table_factor(&mut self, t: &TableFactor) -> ControlFlow<()> {
        match t {
            TableFactor::Table {
                name, args: None, ..
            } => {
                let partes = partes(name);
                if partes.len() == 1 && self.es_cte(&partes[0].value) {
                    return ControlFlow::Continue(());
                }
                if let Some(n) = nombre_del_arbol(name, &mut self.fallos) {
                    self.nombres.push(n);
                }
            }
            TableFactor::Table {
                name,
                args: Some(_),
                ..
            }
            | TableFactor::Function { name, .. } => {
                let f = name.to_string().to_lowercase();
                if !GENERADORAS.contains(&f.as_str()) {
                    self.fallos.push(
                        Fallo::new(
                            format!(
                                "`{f}(…)` lee por función, y lo que lee no lo nombra el árbol: ni linaje ni conducto"
                            ),
                            pos_de_nombre(name),
                        )
                        .ayuda("lee por su nombre: `from paquete.dataset`"),
                    );
                }
            }
            TableFactor::TableFunction { .. } => {
                self.fallos.push(Fallo::new(
                    "`table(…)` lee por función, y lo que lee no lo nombra el árbol",
                    None,
                ));
            }
            _ => {}
        }
        ControlFlow::Continue(())
    }
}

fn partes(n: &ObjectName) -> Vec<&Ident> {
    n.0.iter()
        .filter_map(|p| match p {
            ObjectNamePart::Identifier(i) => Some(i),
            ObjectNamePart::Function(_) => None,
        })
        .collect()
}

/// `paquete.nombre`, o el fallo que diga por qué no lo es.
fn nombre_del_arbol(n: &ObjectName, fallos: &mut Vec<Fallo>) -> Option<Nombre> {
    let pos = pos_de_nombre(n);
    let p = partes(n);
    if p.len() != n.0.len() {
        fallos.push(Fallo::new(format!("`{n}` no es un nombre del árbol"), pos));
        return None;
    }
    match p.as_slice() {
        [paquete, nombre] => Some(Nombre {
            paquete: paquete.value.clone(),
            nombre: nombre.value.clone(),
            pos,
        }),
        [solo] => {
            fallos.push(
                Fallo::new(format!("`{}` no dice de qué paquete es", solo.value), pos)
                    .ayuda(format!("`paquete.{}`", solo.value)),
            );
            None
        }
        _ => {
            fallos.push(Fallo::new(
                format!(
                    "`{n}` tiene {} partes: un nombre del árbol es `paquete.nombre`",
                    p.len()
                ),
                pos,
            ));
            None
        }
    }
}

/// La posición de un nombre: la de su primera parte (línea y columna en base 1,
/// que es como las cuenta `sqlparser` y como las pinta el editor).
fn pos_de_nombre(n: &ObjectName) -> Option<Pos> {
    partes(n).first().and_then(|i| pos_de(i.span.start))
}

fn pos_de(l: sqlparser::tokenizer::Location) -> Option<Pos> {
    (l.line > 0 && l.column > 0).then_some(Pos {
        line: l.line as usize,
        col: l.column as usize,
    })
}

/// Lo que va desde donde empieza `q` hasta el final de la frase. En las dos
/// formas que escriben (`create or replace table p.t as <q>` e `insert into
/// p.t <q>`) la consulta es lo último, así que basta con saber dónde EMPIEZA:
/// el final que `sqlparser` da a algunos nodos es aproximado, el comienzo no.
fn desde(texto: &str, q: &Query) -> Option<String> {
    use sqlparser::ast::Spanned;
    let l = q.span().start;
    let linea = texto
        .split_inclusive('\n')
        .nth((l.line as usize).checked_sub(1)?)?;
    let antes: usize = texto
        .split_inclusive('\n')
        .take(l.line as usize - 1)
        .map(str::len)
        .sum();
    let col = linea
        .char_indices()
        .nth((l.column as usize).checked_sub(1)?)?
        .0;
    // `insert into p.t (select …)`: el nodo empieza DENTRO del paréntesis, y
    // cortar ahí deja un `)` suelto al final. Se retrocede sobre los que abren.
    let mut inicio = antes + col;
    loop {
        let previo = texto[..inicio].trim_end();
        match previo.strip_suffix('(') {
            Some(resto) => inicio = resto.len(),
            None => break,
        }
    }
    Some(sin_punto_y_coma(&texto[inicio..]))
}

fn sin_punto_y_coma(t: &str) -> String {
    t.trim().trim_end_matches(';').trim_end().to_string()
}

fn pos_de_sentencia(s: &Statement) -> Option<Pos> {
    use sqlparser::ast::Spanned;
    pos_de(s.span().start)
}

fn primera_palabra(s: &Statement) -> String {
    s.to_string()
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_lowercase()
}

/// `sqlparser` pone la posición al final del mensaje: «… at Line: 1, Column: 15».
fn fallo_de_analisis(m: &str) -> Fallo {
    let m = m.strip_prefix("sql parser error: ").unwrap_or(m);
    let (texto, pos) = match m.rsplit_once(" at Line: ") {
        Some((t, resto)) => {
            let mut nums = resto
                .split(|c: char| !c.is_ascii_digit())
                .filter(|s| !s.is_empty())
                .filter_map(|s| s.parse::<usize>().ok());
            let pos = match (nums.next(), nums.next()) {
                (Some(line), Some(col)) => Some(Pos { line, col }),
                _ => None,
            };
            (t.to_string(), pos)
        }
        None => (m.to_string(), None),
    };
    Fallo::new(format!("la frase no analiza: {texto}"), pos)
}

/// Coteja la declaración con el árbol compilado: que lo que se lee **se pueda
/// leer** y que lo que se escribe **se pueda escribir**. Las reglas y las
/// frases son las de `datos_de` en ore-serve (0031 §10, 0033: un lector, un
/// camino), dichas antes de correr en vez de en la celda:
///
/// - se lee un `Dataset`, o una `View` cuya raíz de lectura es uno;
/// - una `View` virtual no tiene nada debajo que leer, y una `Table` de una
///   fuente se lee por un `Dataset` que la copie, nunca del origen;
/// - se escribe un `Dataset` **escrito** (sin `from`), exista o no todavía; uno
///   mantenido sale de su `from` y no se escribe a mano, y una `View`, una
///   `Table` o una `Entity` no se escriben.
///
/// Si **existe** el bytes que se lee lo dice el puntero en el momento de
/// correr, no el árbol: eso no se coteja aquí.
pub fn cotejar(pkg: &Package, u: &Unidad) -> Vec<Fallo> {
    let doc = |n: &Nombre| {
        let r = n.referencia();
        pkg.docs
            .iter()
            .find(|d| d.kind != Kind::Package && d.qname().as_deref() == Some(r.as_str()))
    };
    let hay_paquete = |p: &str| {
        pkg.docs
            .iter()
            .any(|d| d.kind == Kind::Package && d.meta("name").and_then(|n| n.as_str()) == Some(p))
    };
    let sin_paquete = |n: &Nombre| {
        Fallo::new(
            format!("no hay ningún paquete `{}` en el árbol", n.paquete),
            n.pos,
        )
    };
    let mut fallos = Vec::new();
    for n in &u.lee {
        let r = n.referencia();
        match doc(n) {
            Some(d) if d.kind == Kind::Dataset => {}
            Some(d) if d.kind == Kind::View => {
                if vistas::raiz_de_lectura(pkg, d).is_none() {
                    fallos.push(
                        Fallo::new(
                            format!("`{r}` es una `View` virtual: no tiene ningún dataset debajo del que leer"),
                            n.pos,
                        )
                        .ayuda("declara un `Dataset` con `from` sobre ella, o lee el dataset del que sale"),
                    );
                }
            }
            Some(d) if d.kind == Kind::Table => fallos.push(
                Fallo::new(
                    format!("`{r}` es una `Table` de una fuente, no un dataset: se lee por un `Dataset` que la copie, nunca del origen"),
                    n.pos,
                )
                .ayuda(format!("un `Dataset` con `from: {{ table: {r} }}`")),
            ),
            Some(d) => fallos.push(Fallo::new(
                format!("`{r}` es una `{:?}`: en SQL se lee un `Dataset` o una `View`", d.kind),
                n.pos,
            )),
            None if !hay_paquete(&n.paquete) => fallos.push(sin_paquete(n)),
            None => fallos.push(Fallo::new(
                format!("no hay ningún `Dataset` ni `View` `{r}` en el árbol"),
                n.pos,
            )),
        }
    }
    if let Some(e) = &u.escribe {
        let n = &e.destino;
        let r = n.referencia();
        match doc(n) {
            Some(d) if d.kind == Kind::Dataset && d.section("from").is_none() => {}
            Some(d) if d.kind == Kind::Dataset => fallos.push(
                Fallo::new(
                    format!(
                        "`{r}` es un `Dataset` mantenido: sale de su `from` y no se escribe a mano"
                    ),
                    n.pos,
                )
                .ayuda("escribe en un dataset tuyo, con otro nombre"),
            ),
            Some(d) => fallos.push(Fallo::new(
                format!(
                    "`{r}` es una `{:?}`: lo que un `.sql` escribe es un `Dataset`",
                    d.kind
                ),
                n.pos,
            )),
            None if !hay_paquete(&n.paquete) => fallos.push(sin_paquete(n)),
            None => {}
        }
    }
    fallos
}

/// Las palabras tras las que un `paquete.nombre` es algo que se LEE —y no una
/// columna con el alias de una tabla—.
const TRAS_LAS_QUE_SE_LEE: [&str; 6] =
    ["FROM", "JOIN", "PIVOT", "UNPIVOT", "SUMMARIZE", "DESCRIBE"];

/// **Qué nombres del árbol lee el texto de una celda** (`sql()` en un puesto).
///
/// No es [`analizar`]: una celda no es una unidad —puede tener varias
/// sentencias, leer por función, escribir en un esquema de la sesión— y el
/// motor es DuckDB, cuya sintaxis el parser no alcanza entera. Medido en
/// `pruebas-de-fuego/medida-el-terreno-de-la-regex.py`: de 52 frases, el
/// parser de `sqlparser` no analiza 7 que DuckDB acepta (PIVOT, UNPIVOT,
/// SUMMARIZE, ASOF, USING SAMPLE, listas por comprensión, INSERT … BY NAME) y
/// la regex de hoy acierta 43. Aquí se usa su **tokenizador**, que no falla
/// con lo que no conoce, y **el árbol como filtro**: 52 de 52.
///
/// Un `a.b` —con o sin comillas, fuera de comentarios y cadenas, que no sea
/// parte de un nombre de tres— se resuelve si:
///
/// - es un `Dataset`, una `View` o una `Table` del árbol (la Table, para que
///   su 409 diga «se lee por un Dataset que la copie»); o
/// - su primer trozo es un paquete y va justo tras FROM, JOIN, PIVOT, UNPIVOT,
///   SUMMARIZE o DESCRIBE: lo que no existe se dice (`LookupError`, el 404 de
///   siempre) en vez de dejárselo a DuckDB; y un alias con el nombre de un
///   paquete en la lista de columnas no se toca.
///
/// Lo demás —un esquema de la sesión (`tmp.t`), un alias, un campo de un
/// struct— es del motor. Hoy la regex mandaba `tmp.t` a ore-serve: 404, y la
/// celda moría.
pub fn nombres_a_resolver(texto: &str, pkg: &Package) -> Vec<String> {
    use sqlparser::tokenizer::{Token, Tokenizer};
    let Ok(toks) = Tokenizer::new(&DuckDbDialect {}, texto).tokenize() else {
        // Un texto que ni se tokeniza (una cadena sin cerrar) no llega a
        // ninguna parte: que lo diga el motor, con su posición.
        return Vec::new();
    };
    let toks: Vec<Token> = toks
        .into_iter()
        .filter(|t| !matches!(t, Token::Whitespace(_)))
        .collect();
    let del_arbol = |qn: &str| {
        pkg.docs.iter().any(|d| {
            matches!(d.kind, Kind::Dataset | Kind::View | Kind::Table)
                && d.qname().as_deref() == Some(qn)
        })
    };
    let paquete = |p: &str| {
        pkg.docs
            .iter()
            .any(|d| d.kind == Kind::Package && d.meta("name").and_then(|n| n.as_str()) == Some(p))
    };
    let palabra = |t: &Token| match t {
        Token::Word(w) => Some(w.value.clone()),
        _ => None,
    };
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i + 2 < toks.len() {
        if let (Some(a), Token::Period, Some(b)) =
            (palabra(&toks[i]), &toks[i + 1], palabra(&toks[i + 2]))
        {
            let antes = i > 0 && matches!(toks[i - 1], Token::Period);
            let despues = matches!(toks.get(i + 3), Some(Token::Period));
            if !antes && !despues {
                let qn = format!("{a}.{b}");
                let tras_lectura = i > 0
                    && matches!(&toks[i - 1], Token::Word(w)
                        if TRAS_LAS_QUE_SE_LEE.iter().any(|k| w.value.eq_ignore_ascii_case(k)));
                if (del_arbol(&qn) || (tras_lectura && paquete(&a))) && !out.contains(&qn) {
                    out.push(qn);
                }
            }
            i += 3;
            continue;
        }
        i += 1;
    }
    out.sort();
    out
}

/// Qué escribe en el árbol una celda de la sesión, si escribe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EscribeEnElArbol {
    /// `create … table` o `insert … into` un `paquete.nombre`: una tabla del
    /// lago (y la celda se corre como un `.sql` del árbol).
    Tabla(String),
    /// `create … view paquete.nombre`: una View del árbol no nace de una
    /// celda, se declara.
    Vista(String),
}

/// **¿Escribe esta celda en el árbol?** (una celda `sql` de la sesión).
///
/// Decidido (2026-09-24): en la sesión, una frase que crea o inserta en un
/// `paquete.nombre` de un paquete del árbol ESCRIBE de verdad —como `write()`
/// desde Python—; lo que crea en otra parte (`tmp.t`, `x`, una temporal) es de
/// DuckDB, como siempre. Medido en `medida-el-sql-que-escribe.sh`: hoy esa
/// frase daba `Count` y no dejaba nada.
///
/// Con el **tokenizador**, no con el parser: lo que `sqlparser` no analiza
/// (`insert … by name`, `pivot`, `using sample`) también se ve, y en vez de
/// perderse en la memoria de DuckDB se dice por qué no se corre. Mira
/// `create [or replace] [temp|temporary] table|view [if not exists] a.b` e
/// `insert [or replace|ignore] into a.b`, en cualquier sentencia del texto.
pub fn escribe_en_el_arbol(texto: &str, pkg: &Package) -> Option<EscribeEnElArbol> {
    use sqlparser::tokenizer::{Token, Tokenizer};
    let toks: Vec<Token> = Tokenizer::new(&DuckDbDialect {}, texto)
        .tokenize()
        .ok()?
        .into_iter()
        .filter(|t| !matches!(t, Token::Whitespace(_)))
        .collect();
    let es = |i: usize, k: &str| matches!(toks.get(i), Some(Token::Word(w)) if w.quote_style.is_none() && w.value.eq_ignore_ascii_case(k));
    let paquete = |p: &str| {
        pkg.docs
            .iter()
            .any(|d| d.kind == Kind::Package && d.meta("name").and_then(|n| n.as_str()) == Some(p))
    };
    // `a.b` en `i` (y no `a.b.c`), con `a` un paquete del árbol
    let nombre = |i: usize| -> Option<String> {
        match (
            toks.get(i),
            toks.get(i + 1),
            toks.get(i + 2),
            toks.get(i + 3),
        ) {
            (Some(Token::Word(a)), Some(Token::Period), Some(Token::Word(b)), siguiente)
                if !matches!(siguiente, Some(Token::Period)) && paquete(&a.value) =>
            {
                Some(format!("{}.{}", a.value, b.value))
            }
            _ => None,
        }
    };
    for i in 0..toks.len() {
        if es(i, "create") {
            let mut j = i + 1;
            if es(j, "or") && es(j + 1, "replace") {
                j += 2;
            }
            if es(j, "temp") || es(j, "temporary") {
                j += 1;
            }
            let vista = es(j, "view");
            if !(vista || es(j, "table")) {
                continue;
            }
            j += 1;
            if es(j, "if") && es(j + 1, "not") && es(j + 2, "exists") {
                j += 3;
            }
            if let Some(n) = nombre(j) {
                return Some(if vista {
                    EscribeEnElArbol::Vista(n)
                } else {
                    EscribeEnElArbol::Tabla(n)
                });
            }
        } else if es(i, "insert") {
            let mut j = i + 1;
            if es(j, "or") && (es(j + 1, "replace") || es(j + 1, "ignore")) {
                j += 2;
            }
            if es(j, "into")
                && let Some(n) = nombre(j + 1)
            {
                return Some(EscribeEnElArbol::Tabla(n));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lee(q: &str) -> Vec<String> {
        analizar(q)
            .unwrap_or_else(|f| panic!("{q}: {f:?}"))
            .lee
            .iter()
            .map(Nombre::referencia)
            .collect()
    }

    fn escribe(q: &str) -> (String, Modo) {
        let e = analizar(q).unwrap().escribe.unwrap();
        (e.destino.referencia(), e.modo)
    }

    /// Las columnas de un `insert` sin nombre: una expresión sin alias toma el
    /// de la tabla en su posición (decidido 2026-09-24); las demás van por el
    /// suyo. Tras un `*` la posición ya no se sabe.
    #[test]
    fn un_insert_dice_que_columnas_van_por_posicion() {
        let p = |q: &str| analizar(q).unwrap().escribe.unwrap().por_posicion;
        assert_eq!(p("insert into hr.x select letra, 0.5 from hr.a"), [1]);
        assert_eq!(
            p(
                "insert into hr.x select a, t.b, c as d, sum(e), e + 1 from hr.a as t group by 1, 2, 3"
            ),
            [3, 4]
        );
        assert_eq!(
            p("insert or replace into hr.x select id, 'b' from hr.a"),
            [1]
        );
        assert_eq!(p("insert into hr.x select 1, * from hr.a"), [0]);
        assert_eq!(
            p("insert into hr.x select *, 1 from hr.a"),
            Vec::<usize>::new()
        );
        assert_eq!(
            p("insert into hr.x select 1 as a from hr.a union all select 2 from hr.b"),
            Vec::<usize>::new()
        );
        assert_eq!(
            p("insert into hr.x with w as (select 1 as a) select a, 2 from w"),
            [1]
        );
        assert_eq!(
            p("create or replace table hr.x as select 0.5 from hr.a"),
            Vec::<usize>::new()
        );
    }

    fn falla(q: &str) -> Vec<Fallo> {
        analizar(q).expect_err(q)
    }

    /// Los casos de `medida-el-sql-del-arbol.py` §2: los que la regex del SDK
    /// fallaba, y los que no.
    #[test]
    fn lee_lo_que_la_regex_no_veia() {
        assert_eq!(
            lee("select pais, count(*) as n\nfrom mi_paquete.mi_dataset\ngroup by pais"),
            ["mi_paquete.mi_dataset"]
        );
        assert_eq!(
            lee("select * from hr.a, hr.b where a.id = b.id"),
            ["hr.a", "hr.b"]
        );
        assert_eq!(
            lee("-- antes: from viejo.tabla\nselect * from hr.a"),
            ["hr.a"]
        );
        assert_eq!(lee("select 'from x.y' as s from hr.a"), ["hr.a"]);
        assert_eq!(lee("select * from \"hr\".\"espanoles\""), ["hr.espanoles"]);
        assert_eq!(lee("from hr.espanoles select nombre"), ["hr.espanoles"]);
        assert_eq!(
            lee("select * from hr.a where id in (select id from hr.b)"),
            ["hr.a", "hr.b"]
        );
        assert_eq!(lee("select * from hr.a, unnest(a.xs) as u(x)"), ["hr.a"]);
        assert_eq!(lee("select * exclude (x) from hr.a"), ["hr.a"]);
        assert_eq!(
            lee(
                "select pais, count(*) as n from hr.a group by all qualify row_number() over (order by n desc) <= 3"
            ),
            ["hr.a"]
        );
        assert_eq!(lee("select * from range(10)"), Vec::<String>::new());
    }

    #[test]
    fn un_with_no_es_un_nombre_del_arbol_y_se_repite_una_vez() {
        assert_eq!(
            lee(
                "with t as (select * from hr.a) select * from t join hr.b using (id) join hr.a using (id)"
            ),
            ["hr.a", "hr.b"]
        );
        // el `with` sólo tapa dentro de su consulta
        let f = falla("select * from (with t as (select 1) select * from t), t");
        assert!(
            f[0].mensaje.contains("`t` no dice de qué paquete es"),
            "{f:?}"
        );
    }

    #[test]
    fn los_tres_modos_de_write() {
        assert_eq!(
            escribe("create or replace table hr.salida as select * from hr.a"),
            ("hr.salida".into(), Modo::Sobrescribir)
        );
        assert_eq!(
            escribe("insert into hr.salida select * from hr.a"),
            ("hr.salida".into(), Modo::Anexar)
        );
        assert_eq!(
            escribe("insert or replace into hr.salida select * from hr.a"),
            ("hr.salida".into(), Modo::Upsert)
        );
        let u = analizar("insert into hr.salida select * from hr.a").unwrap();
        assert_eq!(
            u.lee.iter().map(Nombre::referencia).collect::<Vec<_>>(),
            ["hr.a"]
        );
        assert!(analizar("select 1").unwrap().escribe.is_none());
    }

    #[test]
    fn lo_que_no_es_una_unidad_se_dice_con_su_sitio() {
        let f = falla("create table hr.salida as select * from hr.a");
        assert!(f[0].mensaje.contains("la segunda vez"), "{f:?}");
        assert_eq!(f[0].pos, Some(Pos { line: 1, col: 14 }));

        let f = falla("select * from hr.a;\nselect * from hr.b");
        assert!(f[0].mensaje.contains("UNA sentencia"), "{f:?}");
        assert_eq!(f[0].pos.map(|p| p.line), Some(2));

        let f = falla("select *\nfrom lago.hr.espanoles");
        assert!(f[0].mensaje.contains("3 partes"), "{f:?}");
        assert_eq!(f[0].pos, Some(Pos { line: 2, col: 6 }));

        let f = falla("select * from read_parquet('gs://b/x.parquet')");
        assert!(f[0].mensaje.contains("read_parquet"), "{f:?}");

        let f = falla("insert into hr.salida select * from hr.salida");
        assert!(f[0].mensaje.contains("se escribe y se lee"), "{f:?}");

        assert!(falla("delete from hr.a")[0].mensaje.contains("`delete`"));
        assert!(
            falla("insert or ignore into hr.x select 1")[0]
                .mensaje
                .contains("or replace")
        );
        assert!(
            falla("insert into hr.x (a) select 1")[0]
                .mensaje
                .contains("lista de columnas")
        );
        assert!(
            falla("insert into hr.x values (1)")[0]
                .mensaje
                .contains("`select`")
        );
        assert!(
            falla("create or replace table hr.x (a int)")[0]
                .mensaje
                .contains("lista de columnas")
        );
        assert!(falla("   ")[0].mensaje.contains("vacío"));
    }

    /// Lo que se le pasa a `sql()`: el `select` tal como se escribió, con sus
    /// comentarios y su dialecto, sin el envoltorio que escribe.
    #[test]
    fn la_consulta_se_corta_del_texto() {
        let c = |q: &str| analizar(q).unwrap().consulta;
        assert_eq!(c("select 1;\n"), "select 1");
        assert_eq!(
            c(
                "create or replace table hr.s as\n-- lo de España\nselect * exclude (x)\nfrom hr.a\nqualify row_number() over () = 1;"
            ),
            "select * exclude (x)\nfrom hr.a\nqualify row_number() over () = 1"
        );
        assert_eq!(
            c("insert or replace into hr.s with t as (select * from hr.a) from t select *"),
            "with t as (select * from hr.a) from t select *"
        );
        assert_eq!(
            c("insert into hr.s (select ñ from hr.a)"),
            "(select ñ from hr.a)"
        );
        assert!(
            falla("insert into hr.s select 1 returning *")[0]
                .mensaje
                .contains("returning")
        );
    }

    #[test]
    fn todos_los_nombres_malos_a_la_vez() {
        let f = falla("select * from a join b using (id)");
        assert_eq!(f.len(), 2, "{f:?}");
    }

    #[test]
    fn la_frase_que_no_analiza_dice_donde() {
        let f = falla("select from where hr.a");
        assert!(f[0].mensaje.starts_with("la frase no analiza"), "{f:?}");
        assert!(f[0].pos.is_some(), "{f:?}");
    }
}
