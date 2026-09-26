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
//! **Un guion**: una o varias sentencias separadas por `;`, que corren en
//! orden (ver [`guion`]). Cada una es una **unidad** —lee o escribe datos—:
//!
//! | la frase | es | modo de `write()` |
//! |---|---|---|
//! | `select …` (o `from … select`, `with …`) | un análisis: lee y no escribe | — |
//! | `create or replace dataset p.d as select …` | un transform | `sobrescribir` |
//! | `insert into p.d [(cols)] select …` o `… values (…)` | un transform | `anexar` |
//! | `insert or replace into p.d select …` | un transform | `upsert` (con la clave del dataset) |
//!
//! o **crea** algo del catálogo, en su orden —la base, su schema, un dataset
//! vacío con sus columnas—: [`guion::Sentencia`].
//!
//! Lo que se escribe es un **Dataset**, la unidad de almacenamiento del lago
//! (0033). Una **Table** es un puntero a un objeto de un origen: nace del
//! descubrimiento, no de SQL, y `create table` se niega con esa frase.
//!
//! Y nada más, a propósito:
//!
//! - **`create dataset … as` sin `or replace`** se niega: un trabajo se vuelve
//!   a correr, y la segunda vez fallaría porque el dataset ya existe.
//! - **Un nombre se escribe `base.schema.nombre`** (0038, como en Unity
//!   Catalog: la base es el paquete). Sin base no se sabe de quién es (salvo
//!   que sea un `with`). **Dos partes** (`base.nombre`, la forma de antes) se
//!   leen como `base.default.nombre` y se dicen: un aviso [`DOS_PARTES`] que no
//!   para la frase.
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

/// Un nombre del árbol, `base.schema.nombre`, y dónde aparece por primera vez.
/// `paquete` es la base (0038: la base es el paquete).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nombre {
    pub paquete: String,
    /// `default` si el nombre no lo dice (dos partes).
    pub schema: String,
    pub nombre: String,
    pub pos: Option<Pos>,
    /// Se escribió con dos partes (`base.nombre`): se lee en `default`, y se
    /// avisa ([`DOS_PARTES`]).
    pub dos_partes: bool,
}

impl Nombre {
    /// **La clave del motor**, la forma corta (`normalize::corto`): `p.n` en
    /// `default`, `p.s.n` en otro schema. Es con la que se busca en el árbol, y
    /// la que ven quienes ya hablaban con el motor: `ventas.pedidos` y
    /// `ventas.default.pedidos` son la misma.
    pub fn referencia(&self) -> String {
        crate::normalize::corto(&self.paquete, &self.schema, &self.nombre)
    }

    /// Las tres partes, `default` incluido: como se escribe en SQL.
    pub fn completo(&self) -> String {
        format!("{}.{}.{}", self.paquete, self.schema, self.nombre)
    }
}

/// **`ORE-SQL-2P`** · un nombre del árbol con dos partes (`base.nombre`): se
/// lee como `base.default.nombre`, y se dice (0038 § «Las dos partes»). Un
/// aviso: la frase corre.
pub const DOS_PARTES: &str = "ORE-SQL-2P";

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
    /// Lo que no para la frase pero se dice: hoy, los nombres de dos partes
    /// ([`DOS_PARTES`]), uno por nombre.
    pub avisos: Vec<Fallo>,
}

/// Por qué la frase no es una unidad —o, en [`Unidad::avisos`], lo que se
/// dice sin pararla—, y dónde mirar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fallo {
    pub mensaje: String,
    pub pos: Option<Pos>,
    pub ayuda: Option<String>,
    /// El código, si lo tiene (`ORE-SQL-2P`).
    pub codigo: Option<&'static str>,
}

impl Fallo {
    pub fn new(mensaje: impl Into<String>, pos: Option<Pos>) -> Self {
        Self {
            mensaje: mensaje.into(),
            pos,
            ayuda: None,
            codigo: None,
        }
    }

    /// El aviso de un nombre de dos partes.
    fn dos_partes(n: &Nombre) -> Self {
        Self {
            mensaje: format!(
                "`{}.{}` tiene dos partes: se lee como `{}`",
                n.paquete,
                n.nombre,
                n.completo()
            ),
            pos: n.pos,
            ayuda: Some(format!(
                "escribe las tres, `base.schema.nombre`: `{}`",
                n.completo()
            )),
            codigo: Some(DOS_PARTES),
        }
    }

    pub fn ayuda(mut self, a: impl Into<String>) -> Self {
        self.ayuda = Some(a.into());
        self
    }
}

/// Las funciones que generan filas sin leer nada: no rompen el linaje.
const GENERADORAS: [&str; 3] = ["range", "generate_series", "unnest"];

const LO_QUE_PUEDE_SER: &str = "una sentencia del árbol es un `select` (lee), `create or replace dataset b.s.d as select …` (sobrescribe), `insert into b.s.d select …` o `… values (…)` (anexa), `insert or replace into b.s.d select …` (upsert), o crea: `create standard|foreign database b`, `create schema b.s`, `create dataset b.s.d (columnas)`, `create [or replace] view b.s.v as select …` (y `drop view b.s.v`)";

pub mod guion;

/// Analiza el texto de un `.sql` que es **una** unidad —un trabajo, una celda
/// que escribe— y devuelve lo que declara. Todos los fallos a la vez cuando se
/// puede —un nombre mal escrito no esconde al siguiente—; uno solo si la frase
/// no analiza. Un guion de varias sentencias es [`guion::guion`].
pub fn analizar(texto: &str) -> Result<Unidad, Vec<Fallo>> {
    let mut trozos = guion::guion(texto)?;
    if let Some(otra) = trozos.get(1) {
        return Err(vec![
            Fallo::new(
                "un `.sql` que corre como trabajo es UNA sentencia: un fichero, una salida",
                otra.pos,
            )
            .ayuda("parte el fichero en dos, junta lo que calculas en un `with`, o córrelo en la sesión: allí un guion corre sentencia a sentencia"),
        ]);
    }
    match trozos.pop() {
        Some(guion::Trozo {
            sentencia: guion::Sentencia::Unidad(u),
            ..
        }) => Ok(u),
        Some(t) => Err(vec![
            Fallo::new(
                format!(
                    "`{}` crea algo del catálogo y no lee ni escribe datos: corre en la sesión, como una sentencia de un guion",
                    t.sentencia.que()
                ),
                t.pos,
            )
            .ayuda(LO_QUE_PUEDE_SER),
        ]),
        None => Err(vec![
            Fallo::new("el `.sql` está vacío", None).ayuda(LO_QUE_PUEDE_SER),
        ]),
    }
}

/// **Una sentencia que lee o escribe datos**, ya analizada. `texto` es el de
/// la sentencia con lo de alrededor en blanco (líneas y columnas, las del
/// fichero): de él se corta la consulta.
fn unidad_de(s: &Statement, texto: &str) -> Result<Unidad, Vec<Fallo>> {
    let mut fallos = Vec::new();
    let escribe = match s {
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
    // Un aviso por nombre de dos partes, donde aparece por primera vez: el
    // destino y lo que se lee, en el orden del texto.
    let avisos: Vec<Fallo> = escribe
        .iter()
        .map(|e| &e.destino)
        .chain(lee.iter())
        .filter(|n| n.dos_partes)
        .map(Fallo::dos_partes)
        .collect();
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

    let consulta = match s {
        Statement::Query(_) => Some(sin_punto_y_coma(texto)),
        Statement::CreateTable(c) => c.query.as_deref().and_then(|q| desde(texto, q)),
        Statement::Insert(i) => i
            .source
            .as_deref()
            .and_then(|q| consulta_de_insert(texto, i, q)),
        _ => None,
    };
    if fallos.is_empty() {
        let Some(consulta) = consulta else {
            return Err(vec![Fallo::new(
                "no se encuentra dónde empieza la consulta de la frase",
                pos_de_sentencia(s),
            )]);
        };
        Ok(Unidad {
            lee,
            escribe,
            consulta,
            avisos,
        })
    } else {
        Err(fallos)
    }
}

fn escritura_de_create(c: &CreateTable, fallos: &mut Vec<Fallo>) -> Option<Escritura> {
    let pos = pos_de_nombre(&c.name);
    if c.query.is_none() {
        // Un dataset con columnas es [`guion::Sentencia::CrearDataset`]: aquí
        // sólo llega lo que no es ni eso.
        fallos.push(
            Fallo::new(
                "un dataset nace vacío con sus columnas (`create dataset b.s.d (…)`) o de lo que se escribe en él",
                pos,
            )
            .ayuda("`create or replace dataset b.s.d as select …`"),
        );
        return None;
    }
    if c.temporary {
        fallos.push(Fallo::new(
            "un dataset temporal no sale de la sesión: usa un `with`",
            pos,
        ));
        return None;
    }
    if !c.or_replace {
        fallos.push(
            Fallo::new(
                "`create dataset … as` falla la segunda vez que corre: el dataset ya existe",
                pos,
            )
            .ayuda("`create or replace dataset … as`: sobrescribe, que es lo que un trabajo repetido tiene que hacer"),
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
    let Some(q) = i.source.as_deref() else {
        fallos.push(
            Fallo::new(
                "un `insert` del árbol escribe lo que sale de un `select` o de un `values`",
                pos,
            )
            .ayuda("`insert into b.s.d select …` o `insert into b.s.d (cols) values (…)`"),
        );
        return None;
    };
    let destino = nombre_del_arbol(nombre, fallos)?;
    // Con lista de columnas, lo escrito va por ESOS nombres (la consulta se
    // envuelve con ellos: [`consulta_de_insert`]). Sin ella, un `values` no
    // tiene nombres —todas sus columnas van por posición— y un `select`, los
    // suyos salvo las expresiones sin alias.
    let por_posicion = match (&*q.body, i.columns.is_empty()) {
        (_, false) => Vec::new(),
        (SetExpr::Values(v), true) => (0..v.rows.first().map_or(0, |r| r.content.len())).collect(),
        (_, true) => sin_nombre(q),
    };
    Some(Escritura {
        destino,
        modo,
        por_posicion,
    })
}

/// **Lo que produce un `insert`**, como consulta de `sql()`: lo que sigue al
/// destino, tal como se escribió; con lista de columnas, envuelto para que
/// salga con ESOS nombres (`select * from (…) as v(a, b)`), y un `values`
/// suelto, envuelto también: lo que escribe `write()` es una tabla con nombre.
fn consulta_de_insert(texto: &str, i: &Insert, q: &Query) -> Option<String> {
    let base = desde(texto, q)?;
    // El nodo de un `values` empieza en su primera fila, no en la palabra.
    let es_values = matches!(*q.body, SetExpr::Values(_));
    let base = if es_values
        && !base
            .get(..6)
            .is_some_and(|p| p.eq_ignore_ascii_case("values"))
    {
        format!("VALUES {base}")
    } else {
        base
    };
    let columnas: Vec<String> = i.columns.iter().map(ToString::to_string).collect();
    Some(match (columnas.is_empty(), es_values) {
        (true, false) => base,
        (true, true) => format!("SELECT * FROM ({base}) AS v"),
        (false, _) => format!("SELECT * FROM ({base}) AS v({})", columnas.join(", ")),
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

/// `base.schema.nombre` —o `base.nombre`, en `default` y con aviso—, o el fallo
/// que diga por qué no lo es.
fn nombre_del_arbol(n: &ObjectName, fallos: &mut Vec<Fallo>) -> Option<Nombre> {
    let pos = pos_de_nombre(n);
    let p = partes(n);
    if p.len() != n.0.len() {
        fallos.push(Fallo::new(format!("`{n}` no es un nombre del árbol"), pos));
        return None;
    }
    match p.as_slice() {
        [paquete, schema, nombre] => Some(Nombre {
            paquete: paquete.value.clone(),
            schema: schema.value.clone(),
            nombre: nombre.value.clone(),
            pos,
            dos_partes: false,
        }),
        [paquete, nombre] => Some(Nombre {
            paquete: paquete.value.clone(),
            schema: crate::normalize::SCHEMA_POR_DEFECTO.to_string(),
            nombre: nombre.value.clone(),
            pos,
            dos_partes: true,
        }),
        [solo] => {
            fallos.push(
                Fallo::new(format!("`{}` no dice de qué base es", solo.value), pos)
                    .ayuda(format!("`base.schema.{}`", solo.value)),
            );
            None
        }
        _ => {
            fallos.push(Fallo::new(
                format!(
                    "`{n}` tiene {} partes: un nombre del árbol es `base.schema.nombre`",
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
    cotejar_con(pkg, u, &guion::Creado::default())
}

/// El documento del árbol de un nombre (en su forma corta), si lo hay.
fn doc_de<'a>(pkg: &'a Package, r: &str) -> Option<&'a crate::link::Loaded> {
    pkg.docs
        .iter()
        .find(|d| d.kind != Kind::Package && d.qname().as_deref() == Some(r))
}

fn hay_base(pkg: &Package, p: &str) -> bool {
    pkg.docs
        .iter()
        .any(|d| d.kind == Kind::Package && d.meta("name").and_then(|n| n.as_str()) == Some(p))
}

/// Un schema que no es `default` existe si se declara (v1alpha13, 01 §2).
fn hay_schema_declarado(pkg: &Package, base: &str, schema: &str) -> bool {
    schema == crate::normalize::SCHEMA_POR_DEFECTO
        || pkg.docs.iter().any(|d| {
            d.kind == Kind::Schema
                && d.meta("namespace").and_then(|x| x.as_str()) == Some(base)
                && d.meta("name").and_then(|x| x.as_str()) == Some(schema)
        })
}

/// [`cotejar`] con lo que las sentencias de antes del guion ya crearon: una
/// base, un schema o un dataset de la sentencia 1 existen para la 2.
fn cotejar_con(pkg: &Package, u: &Unidad, creado: &guion::Creado) -> Vec<Fallo> {
    let doc = |n: &Nombre| doc_de(pkg, &n.referencia());
    let hay_paquete = |p: &str| hay_base(pkg, p) || creado.bases.contains(p);
    let sin_paquete = |n: &Nombre| {
        Fallo::new(
            format!("no hay ningún paquete `{}` en el árbol", n.paquete),
            n.pos,
        )
    };
    let hay_schema = |n: &Nombre| {
        hay_schema_declarado(pkg, &n.paquete, &n.schema)
            || creado
                .schemas
                .contains(&(n.paquete.clone(), n.schema.clone()))
    };
    let sin_schema = |n: &Nombre| {
        Fallo::new(
            format!(
                "no hay ningún schema `{}` en la base `{}`",
                n.schema, n.paquete
            ),
            n.pos,
        )
        .ayuda(format!(
            "un `kind: Schema` en `packages/{}/{}/schema.yaml`",
            n.paquete, n.schema
        ))
    };
    let mut fallos = Vec::new();
    for n in &u.lee {
        let r = n.referencia();
        match doc(n) {
            Some(d) if d.kind == Kind::Dataset => {}
            Some(d) if d.kind == Kind::View => {
                if !vistas::se_lee_de_datasets(pkg, d) {
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
            None if !hay_schema(n) => fallos.push(sin_schema(n)),
            // lo creó una sentencia de antes del guion
            None if creado.datasets.contains(&r) || creado.vistas.contains(&r) => {}
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
            None if !hay_schema(n) => fallos.push(sin_schema(n)),
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
/// Un `a.b` o un `a.b.c` —con o sin comillas, fuera de comentarios y cadenas,
/// que no sea parte de uno más largo— se resuelve, **en su forma corta**
/// (`a.default.b` es `a.b`; 0038), si:
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
    let mut out: Vec<String> = Vec::new();
    for n in nombres_de_celda(texto, pkg) {
        if n.se_resuelve && !out.contains(&n.qn) {
            out.push(n.qn);
        }
    }
    out.sort();
    out
}

/// **Los avisos de una celda `sql`** (0038): un `ORE-SQL-2P` por cada nombre
/// del árbol escrito con dos partes —lo que se lee, como en
/// [`nombres_a_resolver`], y el destino de un `create … table` o un `insert …
/// into`—, donde aparece por primera vez. Con el tokenizador, como aquél: lo
/// que el parser no analiza también se ve.
pub fn avisos_de_celda(texto: &str, pkg: &Package) -> Vec<Fallo> {
    let mut vistos = BTreeSet::new();
    nombres_de_celda(texto, pkg)
        .into_iter()
        .filter(|n| n.dos_partes && (n.se_resuelve || n.se_escribe))
        .filter(|n| vistos.insert(n.qn.clone()))
        .map(|n| {
            let (p, nombre) = n.qn.split_once('.').unwrap_or((&n.qn, ""));
            Fallo::dos_partes(&Nombre {
                paquete: p.to_string(),
                schema: crate::normalize::SCHEMA_POR_DEFECTO.to_string(),
                nombre: nombre.to_string(),
                pos: n.pos,
                dos_partes: true,
            })
        })
        .collect()
}

/// Un `a.b` o `a.b.c` del texto de una celda, visto por el tokenizador.
struct NombreDeCelda {
    /// En su forma corta.
    qn: String,
    dos_partes: bool,
    pos: Option<Pos>,
    /// Es un nombre del árbol que se lee (la regla de [`nombres_a_resolver`]).
    se_resuelve: bool,
    /// Va tras `table` o `into`, y su primer trozo es un paquete.
    se_escribe: bool,
}

fn nombres_de_celda(texto: &str, pkg: &Package) -> Vec<NombreDeCelda> {
    use sqlparser::tokenizer::{Token, Tokenizer};
    let Ok(toks) = Tokenizer::new(&DuckDbDialect {}, texto).tokenize_with_location() else {
        // Un texto que ni se tokeniza (una cadena sin cerrar) no llega a
        // ninguna parte: que lo diga el motor, con su posición.
        return Vec::new();
    };
    let toks: Vec<(Token, Option<Pos>)> = toks
        .into_iter()
        .filter(|t| !matches!(t.token, Token::Whitespace(_)))
        .map(|t| (t.token, pos_de(t.span.start)))
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
    let palabra = |t: Option<&(Token, Option<Pos>)>| match t {
        Some((Token::Word(w), _)) => Some(w.value.clone()),
        _ => None,
    };
    let punto = |t: Option<&(Token, Option<Pos>)>| matches!(t, Some((Token::Period, _)));
    let tras = |i: usize, ks: &[&str]| {
        i > 0
            && matches!(&toks[i - 1].0, Token::Word(w)
                if ks.iter().any(|k| w.value.eq_ignore_ascii_case(k)))
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i + 2 < toks.len() {
        if let (Some(a), true, Some(b)) = (
            palabra(toks.get(i)),
            punto(toks.get(i + 1)),
            palabra(toks.get(i + 2)),
        ) {
            let antes = i > 0 && punto(toks.get(i - 1));
            // `a.b.c`: tres partes, si no sigue otra
            let c = if punto(toks.get(i + 3)) {
                palabra(toks.get(i + 4))
            } else {
                None
            };
            let largo = if c.is_some() { 5 } else { 3 };
            let despues = punto(toks.get(i + largo));
            let mal_formado = c.is_none() && punto(toks.get(i + 3));
            if !antes && !despues && !mal_formado {
                let qn = match &c {
                    Some(c) => crate::normalize::a_corto(&format!("{a}.{b}.{c}")).into_owned(),
                    None => format!("{a}.{b}"),
                };
                let es_paquete = paquete(&a);
                let se_resuelve = del_arbol(&qn) || (tras(i, &TRAS_LAS_QUE_SE_LEE) && es_paquete);
                let se_escribe = tras(i, &["table", "dataset", "into"]) && es_paquete;
                out.push(NombreDeCelda {
                    qn,
                    dos_partes: c.is_none(),
                    pos: toks[i].1,
                    se_resuelve,
                    se_escribe,
                });
            }
            i += largo;
            continue;
        }
        i += 1;
    }
    out
}

/// Qué escribe en el árbol una celda de la sesión, si escribe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EscribeEnElArbol {
    /// `create … table` o `insert … into` un `base.schema.nombre` (o
    /// `base.nombre`): una tabla del lago (y la celda se corre como un `.sql`
    /// del árbol). En su forma corta.
    Tabla(String),
    /// `create [or replace] view` o `drop view` de un `base.schema.nombre`
    /// (ADR 0040 paso 5): una View del árbol, que corre como una sentencia del
    /// guion.
    Vista(String),
    /// Crea algo del catálogo (0039): `create [standard|foreign] database b`
    /// —DuckDB no tiene bases, así que siempre es del árbol— o `create schema
    /// b.s` de una base del árbol (`create schema tmp` sigue siendo de DuckDB).
    /// Lo que dice: `database b`, `schema b.s`.
    Crea(String),
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
/// `create [or replace] [temp|temporary] table|dataset|view [if not exists] a.b` e
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
    // `a.b` o `a.b.c` en `i` (y no uno más largo), con `a` un paquete del
    // árbol; en su forma corta (0038)
    let nombre = |i: usize| -> Option<String> {
        let palabra = |k: usize| match toks.get(k) {
            Some(Token::Word(w)) => Some(w.value.clone()),
            _ => None,
        };
        let punto = |k: usize| matches!(toks.get(k), Some(Token::Period));
        let a = palabra(i).filter(|a| paquete(a))?;
        if !punto(i + 1) {
            return None;
        }
        let b = palabra(i + 2)?;
        if !punto(i + 3) {
            return Some(format!("{a}.{b}"));
        }
        let c = palabra(i + 4)?;
        if punto(i + 5) {
            return None;
        }
        Some(crate::normalize::a_corto(&format!("{a}.{b}.{c}")).into_owned())
    };
    for i in 0..toks.len() {
        if es(i, "create") {
            // 0039: lo que crea en el catálogo
            let k = if es(i + 1, "standard") || es(i + 1, "foreign") {
                i + 2
            } else {
                i + 1
            };
            let tras_si_no_existe = |m: usize| {
                if es(m, "if") && es(m + 1, "not") && es(m + 2, "exists") {
                    m + 3
                } else {
                    m
                }
            };
            if es(k, "database") {
                let m = tras_si_no_existe(k + 1);
                if let Some(Token::Word(w)) = toks.get(m) {
                    return Some(EscribeEnElArbol::Crea(format!("database {}", w.value)));
                }
            }
            if es(i + 1, "schema")
                && let Some(n) = nombre(tras_si_no_existe(i + 2))
            {
                return Some(EscribeEnElArbol::Crea(format!("schema {n}")));
            }
            let mut j = i + 1;
            if es(j, "or") && es(j + 1, "replace") {
                j += 2;
            }
            if es(j, "temp") || es(j, "temporary") {
                j += 1;
            }
            let vista = es(j, "view");
            if !(vista || es(j, "table") || es(j, "dataset")) {
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
        } else if es(i, "drop") && es(i + 1, "view") {
            // ADR 0040 paso 5: quitar una View del árbol también es del guion
            let j = if es(i + 2, "if") && es(i + 3, "exists") {
                i + 4
            } else {
                i + 2
            };
            if let Some(n) = nombre(j) {
                return Some(EscribeEnElArbol::Vista(n));
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
            p("create or replace dataset hr.x as select 0.5 from hr.a"),
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
        assert!(f[0].mensaje.contains("`t` no dice de qué base es"), "{f:?}");
    }

    #[test]
    fn los_tres_modos_de_write() {
        assert_eq!(
            escribe("create or replace dataset hr.salida as select * from hr.a"),
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
    fn tres_partes_y_dos_con_su_aviso() {
        // tres partes: la base, el schema y el nombre; la clave es la corta
        let u = analizar("create or replace dataset ventas.espana.r as select * from ventas.default.pedidos join ventas.espana.clientes using (id)").unwrap();
        assert_eq!(
            u.lee.iter().map(Nombre::referencia).collect::<Vec<_>>(),
            ["ventas.pedidos", "ventas.espana.clientes"]
        );
        let d = &u.escribe.as_ref().unwrap().destino;
        assert_eq!(
            (d.referencia(), d.completo()),
            ("ventas.espana.r".into(), "ventas.espana.r".into())
        );
        assert!(u.avisos.is_empty(), "{:?}", u.avisos);

        // dos partes: `default`, y un aviso por nombre, en su sitio
        let u =
            analizar("insert into hr.x\nselect * from hr.a join hr.default.b using (id)").unwrap();
        assert_eq!(
            u.avisos
                .iter()
                .map(|a| (a.codigo, a.pos.map(|p| (p.line, p.col))))
                .collect::<Vec<_>>(),
            [
                (Some(DOS_PARTES), Some((1, 13))),
                (Some(DOS_PARTES), Some((2, 15)))
            ]
        );
        assert!(
            u.avisos[0].mensaje.contains("`hr.default.x`"),
            "{:?}",
            u.avisos
        );
        assert!(
            u.avisos[1]
                .ayuda
                .as_deref()
                .unwrap()
                .contains("`hr.default.a`")
        );

        // `p.n` y `p.default.n` son el mismo: se lee una vez, y no se escribe lo que se lee
        assert_eq!(
            lee("select * from hr.a join hr.default.a using (id)"),
            ["hr.a"]
        );
        let f = falla("insert into hr.default.x select * from hr.x");
        assert!(f[0].mensaje.contains("se escribe y se lee"), "{f:?}");
    }

    #[test]
    fn lo_que_no_es_una_unidad_se_dice_con_su_sitio() {
        let f = falla("create dataset hr.salida as select * from hr.a");
        assert!(f[0].mensaje.contains("la segunda vez"), "{f:?}");
        assert_eq!(f[0].pos, Some(Pos { line: 1, col: 16 }));

        let f = falla("select * from hr.a;\nselect * from hr.b");
        assert!(f[0].mensaje.contains("UNA sentencia"), "{f:?}");
        assert_eq!(f[0].pos.map(|p| p.line), Some(2));

        let f = falla("select *\nfrom ore.lago.hr.espanoles");
        assert!(f[0].mensaje.contains("4 partes"), "{f:?}");
        assert!(f[0].mensaje.contains("base.schema.nombre"), "{f:?}");
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
        // una Table no se crea desde SQL; un dataset con columnas, sí, pero
        // en un guion de la sesión, no como trabajo
        assert!(
            falla("create or replace table hr.x as select 1")[0]
                .mensaje
                .contains("una Table es un puntero")
        );
        assert!(
            falla("create dataset hr.x (a int)")[0]
                .mensaje
                .contains("corre en la sesión")
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
                "create or replace dataset hr.s as\n-- lo de España\nselect * exclude (x)\nfrom hr.a\nqualify row_number() over () = 1;"
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
