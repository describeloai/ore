//! **De una `Peticion` a MQL** (ficha 0069 §4.5). Una función pura: sin red y
//! sin base de datos, porque la afirmación «el `find` pide solo las columnas
//! proyectadas» tiene que ser un aserto y no una promesa (0008).
//!
//! ```text
//!   Peticion { proyeccion, filtros, claves, orden, limit, timeout, rango }
//!       │   + Tipos (el tipo de cada columna, del esquema inferido)
//!       ▼
//!   Plan::Find      { filtro, proyeccion, limit, max_time }      sin orden
//!   Plan::Agregado  { tubo: [$match, $project, $sort, $limit, $project], max_time }
//! ```
//!
//! # Lo que el traductor tiene que arreglar de MongoDB
//!
//! Medido en X0 (§3 de la ficha): los operadores de consulta de Mongo **no**
//! significan lo que SQL espera, y el motor que recibe las filas es DuckDB. Cada
//! operador se escribe para que dé las filas que daría SQL sobre una columna
//! escalar:
//!
//! | SQL | MQL ingenuo | lo que hace mal | lo que se escribe |
//! |---|---|---|---|
//! | `c = v` | `{c: v}` | casa un **array** que contiene `v` | `{c: {$eq: v, $not: {$type: "array"}}}` |
//! | `c <> v` | `{c: {$ne: v}}` | casa **null y ausente** (SQL: desconocido) | `{c: {$nin: [v, null], $not: {$type: "array"}}}` |
//! | `c IS NULL` | `{c: null}` | casa un array que **contiene** `null` | `{$expr: {$in: [{$type: "$c"}, ["null", "missing"]]}}` |
//! | `c ORDER BY … ` | `sort` | los nulos van **primero** en `ASC` | marcador `__ore_nN` + `$sort` (NULLS LAST) |
//! | `c LIKE p` | `{c: /p/}` | sin anclar; `.` no cruza `\n` | `\A…\z` con la bandera `s` |
//!
//! Y se **rechaza**, no se ignora, lo que no se sabe escribir: un operador que
//! no es de los diez, un tipo que no se compara igual (mezclado, compuesto,
//! binario), una proyección vacía.
//!
//! # La proyección es cerrada
//!
//! El `find` pide solo las columnas de la petición (más `_id: 0` si no se
//! pide). Si una columna es prefijo de otra (`a` y `a.b`), se pide la corta, y
//! quien lee saca la larga de dentro: Mongo no admite las dos a la vez.

use crate::esquema::{Esquema, Tipo};
use mongodb::bson::{
    Binary, Bson, DateTime, Decimal128, Document, doc, oid::ObjectId, spec::BinarySubtype,
};
use ore_core::tipos::{Fisico, Valor as Analizado};
use ore_driver::{Fallo, Filtro, Peticion, Valor};
use std::collections::BTreeMap;
use std::str::FromStr;
use std::time::Duration;

/// Los operadores del protocolo 2 (`docs/federation.md` §1.2), los diez.
pub const OPERADORES: &[&str] = &[
    "eq",
    "neq",
    "in",
    "lt",
    "le",
    "gt",
    "ge",
    "like",
    "isNull",
    "isNotNull",
];

/// El prefijo de los campos de trabajo del pipeline de orden.
const MARCA: &str = "__ore_n";

/// El tipo de cada columna física (la ruta), para escribir los literales.
#[derive(Debug, Default, Clone)]
pub struct Tipos(BTreeMap<String, Tipo>);

impl Tipos {
    pub fn new(columnas: impl IntoIterator<Item = (String, Tipo)>) -> Tipos {
        Tipos(columnas.into_iter().collect())
    }

    pub fn de_esquema(e: &Esquema) -> Tipos {
        Tipos::new(e.columnas.iter().map(|c| (c.ruta.clone(), c.tipo.clone())))
    }

    fn de(&self, col: &str) -> Result<&Tipo, Fallo> {
        self.0.get(col).ok_or_else(|| {
            Fallo::operador(format!(
                "`{col}`: no se conoce el tipo de la columna y un literal sin tipo no se escribe (el esquema inferido no la tiene)"
            ))
        })
    }
}

/// Qué se le pide a MongoDB.
#[derive(Debug, Clone, PartialEq)]
pub enum Plan {
    /// `find` con filtro y proyección (sin orden).
    Find {
        filtro: Document,
        proyeccion: Document,
        limit: Option<i64>,
        max_time: Option<Duration>,
    },
    /// `aggregate`, cuando hay `orderBy` (los nulos al final no son nativos).
    Agregado {
        tubo: Vec<Document>,
        max_time: Option<Duration>,
    },
}

impl Plan {
    /// **La proyección que sale al cliente**: la lista cerrada de columnas.
    pub fn proyeccion(&self) -> &Document {
        match self {
            Plan::Find { proyeccion, .. } => proyeccion,
            Plan::Agregado { tubo, .. } => tubo
                .last()
                .and_then(|e| e.get_document("$project").ok())
                .expect("el último paso de un pipeline de orden es su `$project`"),
        }
    }

    pub fn max_time(&self) -> Option<Duration> {
        match self {
            Plan::Find { max_time, .. } | Plan::Agregado { max_time, .. } => *max_time,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Traduccion {
    pub plan: Plan,
    /// `(propiedad, ruta física)`, en el orden de la petición: lo que sale en
    /// cada fila y de dónde se saca.
    pub rutas: Vec<(String, String)>,
}

/// Traduce una petición. `Err` es siempre un [`Fallo`] del conector (`operador`):
/// quien pidió mandó lo que el conector no declaró o no sabe escribir.
pub fn traducir(p: &Peticion, tipos: &Tipos) -> Result<Traduccion, Fallo> {
    if p.fichero.is_some() || p.listado.is_some() {
        return Err(Fallo::operador(
            "`fichero` y `listado` son de los orígenes de objetos: no aplican a MongoDB",
        ));
    }
    if p.proyeccion.is_empty() {
        return Err(Fallo::operador(
            "la proyección está vacía: no se pide `SELECT *` ni nada (0008: la proyección es una lista cerrada)",
        ));
    }
    for (_, col) in &p.proyeccion {
        validar_ruta(col)?;
    }

    let mut clausulas: Vec<Document> = Vec::new();
    if let Some(c) = claves(p, tipos)? {
        clausulas.push(c);
    }
    for f in &p.filtros {
        clausulas.push(filtro(f, tipos)?);
    }
    clausulas.extend(rango(p, tipos)?);
    let filtro_doc = conjuncion(clausulas);

    let columnas: Vec<String> = p.proyeccion.iter().map(|(_, c)| c.clone()).collect();
    let proyeccion = proyeccion_de(&columnas);
    let limit = match p.limit {
        Some(n) => Some(
            i64::try_from(n)
                .map_err(|_| Fallo::operador("`limit` no cabe en un entero de 64 bits"))?,
        ),
        None => None,
    };
    let max_time = p.timeout_ms.map(Duration::from_millis);

    let plan = if p.orden.is_empty() {
        Plan::Find {
            filtro: filtro_doc,
            proyeccion,
            limit,
            max_time,
        }
    } else {
        Plan::Agregado {
            tubo: tubo(p, tipos, filtro_doc, &columnas, proyeccion, limit)?,
            max_time,
        }
    };
    Ok(Traduccion {
        plan,
        rutas: p.proyeccion.clone(),
    })
}

/// Una ruta física: `a`, `a.b`. Sin componentes vacíos ni que empiecen por `$`
/// (un nombre así se leería como un operador), ni NUL.
fn validar_ruta(col: &str) -> Result<(), Fallo> {
    if col.is_empty()
        || col.contains('\0')
        || col.split('.').any(|c| c.is_empty() || c.starts_with('$'))
    {
        return Err(Fallo::operador(format!(
            "`{col}` no es una ruta de campo que se pueda escribir (vacía, con `$` al principio de un nivel, o con NUL)"
        )));
    }
    Ok(())
}

/// Quita las columnas cubiertas por otra más corta (`a` cubre `a.b`) y los
/// duplicados.
fn colapsar(cols: &[String]) -> Vec<String> {
    let mut v: Vec<String> = cols.to_vec();
    v.sort();
    v.dedup();
    let todas = v.clone();
    v.retain(|c| {
        !todas
            .iter()
            .any(|o| o != c && c.starts_with(&format!("{o}.")))
    });
    v
}

/// `{col: 1, …}`, y `_id: 0` si `_id` no se pide.
fn proyeccion_de(cols: &[String]) -> Document {
    let mut d = Document::new();
    for c in colapsar(cols) {
        d.insert(c, 1_i32);
    }
    if !cols.iter().any(|c| c == "_id" || c.starts_with("_id.")) {
        d.insert("_id", 0_i32);
    }
    d
}

fn conjuncion(mut cs: Vec<Document>) -> Document {
    match cs.len() {
        0 => Document::new(),
        1 => cs.remove(0),
        _ => doc! {"$and": cs.into_iter().map(Bson::Document).collect::<Vec<_>>()},
    }
}

/// Lo que no casa nada.
fn nunca() -> Document {
    doc! {"$expr": false}
}

/// El campo es null **o** no está, sin contar un array que contenga null.
fn es_nulo(col: &str) -> Document {
    doc! {"$expr": {"$in": [{"$type": format!("${col}")}, ["null", "missing"]]}}
}

/// Una condición sobre un campo **escalar**: el campo no es un array (`{$eq: v}`
/// casa un array que contiene `v`: medido en X0).
fn escalar(col: &str, mut ops: Document) -> Document {
    ops.insert("$not", doc! {"$type": "array"});
    doc! {col: ops}
}

/// Lo que se escribe a la derecha de una comparación, ya con su tipo.
enum Literal {
    Un(Bson),
    /// BSON guarda milisegundos y el protocolo microsegundos: el redondeo
    /// depende del operador ([`instante`]).
    Micros(i64),
    /// Un día (`Date`): el inicio del día en milisegundos UTC. Un día no es un
    /// punto sino un tramo, y cada operador lo trata como tal ([`dia`]).
    Dia(i64),
}

fn literal(col: &str, tipo: &Tipo, texto: &str) -> Result<Literal, Fallo> {
    let mal = |que: &str| Fallo::operador(format!("`{col}`: `{texto}` no es {que}"));
    Ok(match tipo {
        Tipo::Entero => Literal::Un(Bson::Int64(texto.parse().map_err(|_| mal("un entero"))?)),
        Tipo::Real => Literal::Un(Bson::Double(texto.parse().map_err(|_| mal("un número"))?)),
        Tipo::Decimal { .. } => Literal::Un(Bson::Decimal128(
            Decimal128::from_str(texto).map_err(|_| mal("un decimal"))?,
        )),
        Tipo::Logico => Literal::Un(Bson::Boolean(match texto {
            "true" => true,
            "false" => false,
            _ => return Err(mal("`true` ni `false`")),
        })),
        Tipo::Instante => match Fisico::Instante.analizar(texto) {
            Some(Analizado::Instante(us)) => Literal::Micros(us),
            _ => return Err(mal("un instante en forma canónica (ISO 8601)")),
        },
        Tipo::FechaHora => match Fisico::FechaHora.analizar(texto) {
            Some(Analizado::FechaHora(us)) => Literal::Micros(us),
            _ => return Err(mal("una fecha y hora sin zona (ISO 8601)")),
        },
        Tipo::Fecha => match Fisico::Fecha.analizar(texto) {
            Some(Analizado::Fecha(d)) => Literal::Dia(i64::from(d) * 86_400_000),
            _ => return Err(mal("una fecha (AAAA-MM-DD)")),
        },
        Tipo::Texto => Literal::Un(Bson::String(texto.to_string())),
        Tipo::ObjectId => Literal::Un(Bson::ObjectId(
            ObjectId::parse_str(texto).map_err(|_| mal("un ObjectId (24 cifras hexadecimales)"))?,
        )),
        Tipo::Uuid => {
            let hex: String = texto.chars().filter(|c| *c != '-').collect();
            let bytes: Option<Vec<u8>> = (hex.len() == 32 && hex.is_ascii())
                .then(|| {
                    (0..16)
                        .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok())
                        .collect()
                })
                .flatten();
            Literal::Un(Bson::Binary(Binary {
                subtype: BinarySubtype::Uuid,
                bytes: bytes.ok_or_else(|| mal("un UUID"))?,
            }))
        }
        // Lo rechaza `filtro` antes de llegar aquí (`empujable`); si esa guarda falla, esto
        // es un error del operador y no un pánico del conector.
        Tipo::Binario | Tipo::Json | Tipo::Mixto => {
            return Err(Fallo::operador(format!(
                "`{col}`: la columna es {} ({}) y MongoDB no la compara como SQL; no se empuja",
                tipo.oos(),
                tipo.fisico_origen()
            )));
        }
    })
}

/// Un `i64` de milisegundos para comparar `micros` con un campo de fecha, según
/// el operador, sin perder el sentido: `ms·1000 < µs` ⟺ `ms < ⌈µs/1000⌉`, etc.
/// `None` es «ninguna fecha de BSON puede ser igual» (`eq` con µs sueltos).
fn instante(op: &str, micros: i64) -> Option<i64> {
    let suelo = micros.div_euclid(1000);
    let exacto = micros.rem_euclid(1000) == 0;
    let techo = suelo + i64::from(!exacto);
    match op {
        "eq" => exacto.then_some(suelo),
        "lt" | "ge" => Some(techo),
        "le" | "gt" => Some(suelo),
        _ => unreachable!(),
    }
}

/// Una condición sobre un `Date` de BSON para el **día** que empieza en `inicio`
/// (ms UTC): `eq` es el tramo `[inicio, inicio+1d)`, y los de orden, uno de sus
/// extremos. `None` en `neq`: lo hace [`filtro`] con el tramo complementario.
fn dia(op: &str, inicio: i64) -> Document {
    let fecha = |ms: i64| Bson::DateTime(DateTime::from_millis(ms));
    let siguiente = inicio + 86_400_000;
    match op {
        "eq" => doc! {"$gte": fecha(inicio), "$lt": fecha(siguiente)},
        "lt" => doc! {"$lt": fecha(inicio)},
        "le" => doc! {"$lt": fecha(siguiente)},
        "gt" => doc! {"$gte": fecha(siguiente)},
        "ge" => doc! {"$gte": fecha(inicio)},
        _ => unreachable!(),
    }
}

fn dos<'a>(f: &'a Filtro, tipo: &Tipo) -> Result<&'a str, Fallo> {
    match &f.valor {
        Valor::Uno(v) => Ok(v),
        _ => Err(Fallo::operador(format!(
            "`{}` sobre `{}` lleva un valor (de tipo {})",
            f.operador,
            f.columna,
            tipo.oos()
        ))),
    }
}

/// Un filtro `columna operador valor`.
fn filtro(f: &Filtro, tipos: &Tipos) -> Result<Document, Fallo> {
    if !OPERADORES.contains(&f.operador.as_str()) {
        return Err(Fallo::operador(format!(
            "`{}` sobre `{}`: ORE no sabe empujar ese operador a MongoDB (sabe {})",
            f.operador,
            f.columna,
            OPERADORES.join(", ")
        )));
    }
    validar_ruta(&f.columna)?;
    let tipo = tipos.de(&f.columna)?;
    let col = f.columna.as_str();

    match f.operador.as_str() {
        "isNull" | "isNotNull" => {
            if f.valor != Valor::Ninguno {
                return Err(Fallo::operador(format!(
                    "`{}` sobre `{col}` no lleva valor",
                    f.operador
                )));
            }
            let nulo = es_nulo(col);
            return Ok(if f.operador == "isNull" {
                nulo
            } else {
                doc! {"$expr": {"$not": [nulo.get_document("$expr").expect("es $expr").clone()]}}
            });
        }
        _ => {}
    }

    if !tipo.empujable() {
        return Err(Fallo::operador(format!(
            "`{}` sobre `{col}`: la columna es {} ({}) y MongoDB no la compara como SQL (un `eq` buscaría dentro de los arrays y compararía solo la clase que casa); no se empuja",
            f.operador,
            tipo.oos(),
            tipo.fisico_origen()
        )));
    }

    match f.operador.as_str() {
        "like" => {
            if *tipo != Tipo::Texto {
                return Err(Fallo::operador(format!(
                    "`like` sobre `{col}`: solo se empuja sobre texto (la columna es {})",
                    tipo.fisico_origen()
                )));
            }
            let patron = dos(f, tipo)?;
            Ok(escalar(
                col,
                doc! {"$regex": like_a_regex(patron), "$options": "s"},
            ))
        }
        "in" => {
            let Valor::Lista(vs) = &f.valor else {
                return Err(Fallo::operador(format!(
                    "`in` sobre `{col}` lleva una lista"
                )));
            };
            let mut vals: Vec<Bson> = Vec::new();
            let mut dias: Vec<Document> = Vec::new();
            for v in vs {
                match literal(col, tipo, v)? {
                    Literal::Un(b) => vals.push(b),
                    Literal::Micros(us) => {
                        // Los µs sueltos no son ninguna fecha de BSON: no casan.
                        if let Some(ms) = instante("eq", us) {
                            vals.push(Bson::DateTime(DateTime::from_millis(ms)));
                        }
                    }
                    Literal::Dia(inicio) => dias.push(escalar(col, dia("eq", inicio))),
                }
            }
            if !dias.is_empty() {
                if !vals.is_empty() {
                    dias.push(escalar(col, doc! {"$in": vals}));
                }
                return Ok(doc! {"$or": dias.into_iter().map(Bson::Document).collect::<Vec<_>>()});
            }
            if vals.is_empty() {
                return Ok(nunca());
            }
            Ok(escalar(col, doc! {"$in": vals}))
        }
        "eq" | "neq" => {
            let v = dos(f, tipo)?;
            let valor = match literal(col, tipo, v)? {
                Literal::Un(b) => Some(b),
                Literal::Micros(us) => {
                    instante("eq", us).map(|ms| Bson::DateTime(DateTime::from_millis(ms)))
                }
                // Un día es un tramo: `eq` es el tramo y `<>` los dos de fuera
                // (que no incluyen null ni ausente, como `<>` en SQL).
                Literal::Dia(inicio) if f.operador == "eq" => {
                    return Ok(escalar(col, dia("eq", inicio)));
                }
                Literal::Dia(inicio) => {
                    return Ok(doc! {"$or": [
                        escalar(col, dia("lt", inicio)),
                        escalar(col, dia("gt", inicio)),
                    ]});
                }
            };
            match (f.operador.as_str(), valor) {
                ("eq", Some(b)) => Ok(escalar(col, doc! {"$eq": b})),
                ("eq", None) => Ok(nunca()),
                // `<>` deja fuera null y ausente (en SQL el resultado es desconocido).
                ("neq", Some(b)) => Ok(escalar(col, doc! {"$nin": [b, Bson::Null]})),
                ("neq", None) => Ok(escalar(col, doc! {"$nin": [Bson::Null]})),
                _ => unreachable!(),
            }
        }
        op @ ("lt" | "le" | "gt" | "ge") => {
            if *tipo == Tipo::Uuid {
                return Err(Fallo::operador(format!(
                    "`{op}` sobre `{col}`: un UUID no se ordena"
                )));
            }
            let v = dos(f, tipo)?;
            let valor = match literal(col, tipo, v)? {
                Literal::Un(b) => b,
                Literal::Micros(us) => Bson::DateTime(DateTime::from_millis(
                    instante(op, us).expect("lt/le/gt/ge siempre dan ms"),
                )),
                Literal::Dia(inicio) => return Ok(escalar(col, dia(op, inicio))),
            };
            let mql = match op {
                "lt" => "$lt",
                "le" => "$lte",
                "gt" => "$gt",
                _ => "$gte",
            };
            Ok(escalar(col, doc! {mql: valor}))
        }
        // El vocabulario ya está comprobado arriba; si esa guarda falla, el operador es
        // lo que se rechaza, no el proceso.
        otro => Err(Fallo::operador(format!(
            "`{otro}` sobre `{col}`: ORE no sabe empujar ese operador a MongoDB (sabe {})",
            OPERADORES.join(", ")
        ))),
    }
}

/// `LIKE` de SQL, sin carácter de escape (`%` cualquier secuencia, `_` un
/// carácter: como el kit y como DuckDB), a una expresión regular anclada.
pub fn like_a_regex(patron: &str) -> String {
    let mut r = String::from("\\A");
    for c in patron.chars() {
        match c {
            '%' => r.push_str(".*"),
            '_' => r.push('.'),
            c if "\\.^$|?*+()[]{}".contains(c) => {
                r.push('\\');
                r.push(c);
            }
            c => r.push(c),
        }
    }
    r.push_str("\\z");
    r
}

/// El recorte por clave: `claveColumnas` con sus `claves`.
fn claves(p: &Peticion, tipos: &Tipos) -> Result<Option<Document>, Fallo> {
    if p.claves.is_empty() {
        return Ok(None);
    }
    if p.clave_columnas.is_empty() {
        return Err(Fallo::operador("hay `claves` y no `claveColumnas`"));
    }
    for c in &p.clave_columnas {
        validar_ruta(c)?;
    }
    for k in &p.claves {
        if k.len() != p.clave_columnas.len() {
            return Err(Fallo::operador(
                "una clave no tiene tantos valores como `claveColumnas`",
            ));
        }
    }
    // Una columna: un `in`. Varias: un `or` de conjunciones de `eq`.
    if p.clave_columnas.len() == 1 {
        let valores: Vec<String> = p.claves.iter().map(|k| k[0].clone()).collect();
        return filtro(
            &Filtro {
                columna: p.clave_columnas[0].clone(),
                operador: "in".into(),
                valor: Valor::Lista(valores),
            },
            tipos,
        )
        .map(Some);
    }
    let mut alternativas = Vec::new();
    for k in &p.claves {
        let mut y = Vec::new();
        for (c, v) in p.clave_columnas.iter().zip(k) {
            y.push(filtro(&Filtro::uno(c, "eq", v), tipos)?);
        }
        alternativas.push(Bson::Document(conjuncion(y)));
    }
    Ok(Some(doc! {"$or": alternativas}))
}

/// El rango del cursor: `start` exclusivo y `end` inclusivo (como `ore-sql`).
fn rango(p: &Peticion, tipos: &Tipos) -> Result<Vec<Document>, Fallo> {
    if p.start.is_none() && p.end.is_none() {
        return Ok(Vec::new());
    }
    let Some(cursor) = p.cursor.as_deref() else {
        return Err(Fallo::operador(
            "un rango por la posición del propio origen (change streams) todavía no está: ficha 0069 M4",
        ));
    };
    let mut v = Vec::new();
    if let Some(s) = &p.start {
        v.push(filtro(&Filtro::uno(cursor, "gt", s), tipos)?);
    }
    if let Some(e) = &p.end {
        v.push(filtro(&Filtro::uno(cursor, "le", e), tipos)?);
    }
    Ok(v)
}

/// El pipeline de un `orderBy`: los nulos van **al final** en los dos sentidos,
/// que Mongo no tiene (medido: nulos primero en `ASC`, últimos en `DESC`). Se
/// calcula un marcador por criterio, se ordena por él y por la columna, y al
/// final se proyecta lo pedido y nada más.
fn tubo(
    p: &Peticion,
    tipos: &Tipos,
    filtro_doc: Document,
    columnas: &[String],
    proyeccion: Document,
    limit: Option<i64>,
) -> Result<Vec<Document>, Fallo> {
    let mut sort = Document::new();
    let mut intermedia: Vec<String> = columnas.to_vec();
    let mut marcas = Document::new();
    for (i, o) in p.orden.iter().enumerate() {
        validar_ruta(&o.columna)?;
        let tipo = tipos.de(&o.columna)?;
        if !tipo.empujable() || *tipo == Tipo::Uuid {
            return Err(Fallo::operador(format!(
                "`orderBy` sobre `{}`: la columna es {} ({}) y no se ordena como SQL; no se empuja",
                o.columna,
                tipo.oos(),
                tipo.fisico_origen()
            )));
        }
        let marca = format!("{MARCA}{i}");
        marcas.insert(
            marca.clone(),
            doc! {"$cond": [{"$in": [{"$type": format!("${}", o.columna)}, ["null", "missing"]]}, 1, 0]},
        );
        sort.insert(marca, 1_i32);
        sort.insert(
            o.columna.clone(),
            if o.descendente { -1_i32 } else { 1_i32 },
        );
        intermedia.push(o.columna.clone());
    }
    // Desempate por `_id`: dos lecturas del mismo `limit` dan las mismas filas.
    sort.insert("_id", 1_i32);

    // Primero se recorta a lo necesario (columnas pedidas, de orden y marcas):
    // ordenar documentos enteros agota los 32 MB de un M0 (medido en X0).
    let mut etapa1 = Document::new();
    for c in colapsar(&intermedia) {
        etapa1.insert(c, 1_i32);
    }
    for (k, v) in marcas {
        etapa1.insert(k, v);
    }

    let mut t = Vec::new();
    if !filtro_doc.is_empty() {
        t.push(doc! {"$match": filtro_doc});
    }
    t.push(doc! {"$project": etapa1});
    t.push(doc! {"$sort": sort});
    if let Some(n) = limit {
        t.push(doc! {"$limit": n});
    }
    t.push(doc! {"$project": proyeccion});
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ore_driver::Orden;

    fn tipos() -> Tipos {
        Tipos::new([
            ("_id".to_string(), Tipo::Entero),
            ("edad".to_string(), Tipo::Entero),
            ("nombre".to_string(), Tipo::Texto),
            (
                "precio".to_string(),
                Tipo::Decimal {
                    precision: 38,
                    escala: 2,
                },
            ),
            ("alta".to_string(), Tipo::Instante),
            ("oid".to_string(), Tipo::ObjectId),
            ("uid".to_string(), Tipo::Uuid),
            ("ok".to_string(), Tipo::Logico),
            ("x".to_string(), Tipo::Real),
            ("awards.wins".to_string(), Tipo::Entero),
            ("awards".to_string(), Tipo::Json),
            ("raro".to_string(), Tipo::Mixto),
        ])
    }

    fn pet(proy: &[(&str, &str)]) -> Peticion {
        Peticion {
            objeto: "ore_prueba.polimorfica".into(),
            proyeccion: proy
                .iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect(),
            ..Default::default()
        }
    }

    fn f(col: &str, op: &str, v: &str) -> Filtro {
        Filtro::uno(col, op, v)
    }

    fn solo_filtro(fi: Filtro) -> Result<Document, Fallo> {
        let mut p = pet(&[("n", "nombre")]);
        p.filtros = vec![fi];
        match traducir(&p, &tipos())?.plan {
            Plan::Find { filtro, .. } => Ok(filtro),
            _ => unreachable!(),
        }
    }

    fn array() -> Document {
        doc! {"$type": "array"}
    }

    /// La proyección es la lista cerrada, con `_id: 0` si no se pide.
    #[test]
    fn la_proyeccion_pide_solo_lo_proyectado() {
        let t = traducir(&pet(&[("n", "nombre"), ("e", "edad")]), &tipos()).unwrap();
        assert_eq!(
            t.plan.proyeccion(),
            &doc! {"edad": 1, "nombre": 1, "_id": 0}
        );
        assert_eq!(
            t.rutas,
            vec![("n".into(), "nombre".into()), ("e".into(), "edad".into())]
        );
        // Con `_id` pedido no se excluye.
        let t = traducir(&pet(&[("id", "_id")]), &tipos()).unwrap();
        assert_eq!(t.plan.proyeccion(), &doc! {"_id": 1});
        // Una columna que filtra pero no se pide NO entra en la proyección.
        let mut p = pet(&[("n", "nombre")]);
        p.filtros = vec![f("edad", "gt", "5")];
        let Plan::Find { proyeccion, .. } = traducir(&p, &tipos()).unwrap().plan else {
            panic!()
        };
        assert!(!proyeccion.contains_key("edad"), "{proyeccion:?}");
    }

    #[test]
    fn una_proyeccion_vacia_se_rechaza() {
        let e = traducir(&pet(&[]), &tipos()).unwrap_err();
        assert_eq!(e.codigo, ore_driver::Codigo::Operador);
        assert!(e.mensaje.contains("lista cerrada"), "{e}");
    }

    /// `a` y `a.b` a la vez no se admiten en Mongo: se pide la corta.
    #[test]
    fn una_columna_que_es_prefijo_de_otra_se_pide_una_vez() {
        let t = traducir(&pet(&[("a", "awards"), ("w", "awards.wins")]), &tipos()).unwrap();
        assert_eq!(t.plan.proyeccion(), &doc! {"awards": 1, "_id": 0});
        // Pero la petición conserva las dos: el lector saca `wins` de dentro.
        assert_eq!(t.rutas.len(), 2);
        // `awards2` NO está dentro de `awards`: el punto cuenta, y se piden las dos.
        let t = traducir(&pet(&[("a", "awards"), ("b", "awards2")]), &tipos()).unwrap();
        assert_eq!(
            t.plan.proyeccion(),
            &doc! {"awards": 1, "awards2": 1, "_id": 0}
        );
    }

    /// `eq`: el campo no es un array (medido: `{v: 1}` casa `[1, 2]`).
    #[test]
    fn eq_no_casa_dentro_de_un_array() {
        assert_eq!(
            solo_filtro(f("edad", "eq", "5")).unwrap(),
            doc! {"edad": {"$eq": 5_i64, "$not": array()}}
        );
        assert_eq!(
            solo_filtro(f("nombre", "eq", "n1")).unwrap(),
            doc! {"nombre": {"$eq": "n1", "$not": array()}}
        );
    }

    /// `neq`: SQL no devuelve las filas con null (el resultado es desconocido).
    #[test]
    fn neq_deja_fuera_null_y_ausente() {
        assert_eq!(
            solo_filtro(f("edad", "neq", "5")).unwrap(),
            doc! {"edad": {"$nin": [5_i64, Bson::Null], "$not": array()}}
        );
    }

    #[test]
    fn la_comparacion_de_orden_y_el_in() {
        for (op, mql) in [("lt", "$lt"), ("le", "$lte"), ("gt", "$gt"), ("ge", "$gte")] {
            assert_eq!(
                solo_filtro(f("edad", op, "50")).unwrap(),
                doc! {"edad": {mql: 50_i64, "$not": array()}},
                "{op}"
            );
        }
        let mut fi = f("edad", "in", "");
        fi.valor = Valor::Lista(vec!["1".into(), "2".into()]);
        assert_eq!(
            solo_filtro(fi).unwrap(),
            doc! {"edad": {"$in": [1_i64, 2_i64], "$not": array()}}
        );
        // `in` con la lista vacía no deja pasar ninguna fila.
        let mut vacio = f("edad", "in", "");
        vacio.valor = Valor::Lista(vec![]);
        assert_eq!(solo_filtro(vacio).unwrap(), doc! {"$expr": false});
    }

    /// `isNull` por `$type` agregado: `{c: null}` casaría un array con null.
    #[test]
    fn isnull_no_confunde_un_array_con_un_null() {
        let nulo = doc! {"$expr": {"$in": [{"$type": "$edad"}, ["null", "missing"]]}};
        let mut fi = f("edad", "isNull", "");
        fi.valor = Valor::Ninguno;
        assert_eq!(solo_filtro(fi).unwrap(), nulo);
        let mut fi = f("edad", "isNotNull", "");
        fi.valor = Valor::Ninguno;
        assert_eq!(
            solo_filtro(fi).unwrap(),
            doc! {"$expr": {"$not": [{"$in": [{"$type": "$edad"}, ["null", "missing"]]}]}}
        );
    }

    #[test]
    fn like_es_el_de_sql_sin_escape_y_anclado() {
        assert_eq!(like_a_regex("a_b"), "\\Aa.b\\z");
        assert_eq!(like_a_regex("%n%"), "\\A.*n.*\\z");
        assert_eq!(like_a_regex("A%"), "\\AA.*\\z");
        // Lo que es regex en el patrón de SQL es literal: `.`, `(`, `\`.
        assert_eq!(like_a_regex("1.5(x)\\"), "\\A1\\.5\\(x\\)\\\\\\z");
        assert_eq!(
            solo_filtro(f("nombre", "like", "n_%")).unwrap(),
            doc! {"nombre": {"$regex": "\\An..*\\z", "$options": "s", "$not": array()}}
        );
        // Solo sobre texto.
        let e = solo_filtro(f("edad", "like", "1%")).unwrap_err();
        assert!(e.mensaje.contains("solo se empuja sobre texto"), "{e}");
    }

    /// Un decimal, un ObjectId y un UUID se escriben con su tipo, no como texto.
    #[test]
    fn los_literales_llevan_su_tipo() {
        let Bson::Decimal128(d) = solo_filtro(f("precio", "ge", "10.50"))
            .unwrap()
            .get_document("precio")
            .unwrap()
            .get("$gte")
            .unwrap()
            .clone()
        else {
            panic!()
        };
        assert_eq!(d, Decimal128::from_str("10.50").unwrap());
        assert_eq!(
            solo_filtro(f("oid", "eq", "65a1b2c3d4e5f60718293a4b")).unwrap(),
            doc! {"oid": {"$eq": ObjectId::parse_str("65a1b2c3d4e5f60718293a4b").unwrap(), "$not": array()}}
        );
        let u = solo_filtro(f("uid", "eq", "12345678-1234-5678-1234-567812345678")).unwrap();
        let Bson::Binary(b) = u.get_document("uid").unwrap().get("$eq").unwrap().clone() else {
            panic!()
        };
        assert_eq!((b.subtype, b.bytes.len()), (BinarySubtype::Uuid, 16));
        assert_eq!(
            solo_filtro(f("ok", "eq", "true")).unwrap(),
            doc! {"ok": {"$eq": true, "$not": array()}}
        );
        // Un literal que no es del tipo se rechaza diciendo cuál.
        let e = solo_filtro(f("edad", "eq", "cinco")).unwrap_err();
        assert!(e.mensaje.contains("`edad`: `cinco` no es un entero"), "{e}");
        assert!(solo_filtro(f("oid", "eq", "zz")).is_err());
        assert!(solo_filtro(f("ok", "eq", "1")).is_err());
    }

    /// BSON guarda ms y el protocolo µs: el redondeo respeta el sentido.
    #[test]
    fn los_instantes_se_redondean_segun_el_operador() {
        // 2026-01-01T00:00:00.0015Z = ...000 ms + 1,5 ms
        let t = "2026-01-01T00:00:00.001500+00:00";
        let ms = |d: &Document, op: &str| match d.get_document("alta").unwrap().get(op).unwrap() {
            Bson::DateTime(d) => d.timestamp_millis(),
            otro => panic!("{otro:?}"),
        };
        let base = 1_767_225_600_000_i64; // 2026-01-01T00:00:00Z
        // lt/ge: el techo (2 ms); le/gt: el suelo (1 ms).
        assert_eq!(
            ms(&solo_filtro(f("alta", "lt", t)).unwrap(), "$lt"),
            base + 2
        );
        assert_eq!(
            ms(&solo_filtro(f("alta", "ge", t)).unwrap(), "$gte"),
            base + 2
        );
        assert_eq!(
            ms(&solo_filtro(f("alta", "le", t)).unwrap(), "$lte"),
            base + 1
        );
        assert_eq!(
            ms(&solo_filtro(f("alta", "gt", t)).unwrap(), "$gt"),
            base + 1
        );
        // Una hora exacta no se redondea.
        let exacto = "2026-01-01T00:00:00+00:00";
        assert_eq!(
            ms(&solo_filtro(f("alta", "lt", exacto)).unwrap(), "$lt"),
            base
        );
        assert_eq!(
            ms(&solo_filtro(f("alta", "eq", exacto)).unwrap(), "$eq"),
            base
        );
        // `eq` con µs sueltos no casa nada (ninguna fecha de BSON los tiene).
        assert_eq!(
            solo_filtro(f("alta", "eq", t)).unwrap(),
            doc! {"$expr": false}
        );
        // y `neq` casa todo lo que no es null.
        assert_eq!(
            solo_filtro(f("alta", "neq", t)).unwrap(),
            doc! {"alta": {"$nin": [Bson::Null], "$not": array()}}
        );
    }

    #[test]
    fn lo_que_no_se_sabe_escribir_se_rechaza_no_se_ignora() {
        // Un operador fuera de los diez.
        let e = solo_filtro(f("edad", "between", "1")).unwrap_err();
        assert_eq!(e.codigo, ore_driver::Codigo::Operador);
        assert!(e.mensaje.contains("no sabe empujar"), "{e}");
        // Una columna sin tipo.
        assert!(
            solo_filtro(f("sin_tipo", "eq", "1"))
                .unwrap_err()
                .mensaje
                .contains("no se conoce el tipo")
        );
        // Un tipo mezclado o compuesto: eq/orden no, isNull sí.
        let e = solo_filtro(f("raro", "eq", "1")).unwrap_err();
        assert!(e.mensaje.contains("no se empuja"), "{e}");
        assert!(solo_filtro(f("awards", "eq", "1")).is_err());
        let mut n = f("raro", "isNull", "");
        n.valor = Valor::Ninguno;
        assert!(solo_filtro(n).is_ok());
        // Un UUID no se ordena.
        assert!(solo_filtro(f("uid", "lt", "12345678-1234-5678-1234-567812345678")).is_err());
        // Una ruta que parecería un operador.
        assert!(solo_filtro(f("$where", "eq", "1")).is_err());
        assert!(solo_filtro(f("a..b", "eq", "1")).is_err());
        // Un valor donde va una lista y al revés.
        assert!(solo_filtro(f("edad", "in", "1")).is_err());
        let mut l = f("edad", "eq", "");
        l.valor = Valor::Lista(vec![]);
        assert!(solo_filtro(l).is_err());
    }

    #[test]
    fn varios_filtros_son_una_conjuncion() {
        let mut p = pet(&[("n", "nombre")]);
        p.filtros = vec![f("edad", "ge", "18"), f("edad", "lt", "65")];
        let Plan::Find { filtro, .. } = traducir(&p, &tipos()).unwrap().plan else {
            panic!()
        };
        assert_eq!(
            filtro,
            doc! {"$and": [
                {"edad": {"$gte": 18_i64, "$not": array()}},
                {"edad": {"$lt": 65_i64, "$not": array()}},
            ]}
        );
        // Sin filtros, el filtro es vacío.
        let Plan::Find { filtro, .. } = traducir(&pet(&[("n", "nombre")]), &tipos()).unwrap().plan
        else {
            panic!()
        };
        assert!(filtro.is_empty());
    }

    #[test]
    fn el_recorte_por_clave() {
        let mut p = pet(&[("n", "nombre")]);
        p.clave_columnas = vec!["_id".into()];
        p.claves = vec![vec!["7".into()], vec!["9".into()]];
        let Plan::Find { filtro, .. } = traducir(&p, &tipos()).unwrap().plan else {
            panic!()
        };
        assert_eq!(
            filtro,
            doc! {"_id": {"$in": [7_i64, 9_i64], "$not": array()}}
        );
        // Clave compuesta: un `or` de conjunciones.
        p.clave_columnas = vec!["_id".into(), "nombre".into()];
        p.claves = vec![vec!["7".into(), "a".into()]];
        let Plan::Find { filtro, .. } = traducir(&p, &tipos()).unwrap().plan else {
            panic!()
        };
        assert_eq!(
            filtro,
            doc! {"$or": [{"$and": [
                {"_id": {"$eq": 7_i64, "$not": array()}},
                {"nombre": {"$eq": "a", "$not": array()}},
            ]}]}
        );
        // Una clave con menos valores que columnas.
        p.claves = vec![vec!["7".into()]];
        assert!(traducir(&p, &tipos()).is_err());
    }

    /// El rango: `start` exclusivo, `end` inclusivo (como `ore-sql`).
    #[test]
    fn el_rango_sobre_una_columna() {
        let mut p = pet(&[("n", "nombre")]);
        p.cursor = Some("edad".into());
        p.start = Some("10".into());
        p.end = Some("20".into());
        let Plan::Find { filtro, .. } = traducir(&p, &tipos()).unwrap().plan else {
            panic!()
        };
        assert_eq!(
            filtro,
            doc! {"$and": [
                {"edad": {"$gt": 10_i64, "$not": array()}},
                {"edad": {"$lte": 20_i64, "$not": array()}},
            ]}
        );
        // Un rango por la posición del origen es de M4.
        p.cursor = None;
        let e = traducir(&p, &tipos()).unwrap_err();
        assert!(e.mensaje.contains("M4"), "{e}");
    }

    #[test]
    fn limit_y_timeout_viajan() {
        let mut p = pet(&[("n", "nombre")]);
        p.limit = Some(100);
        p.timeout_ms = Some(30_000);
        let t = traducir(&p, &tipos()).unwrap();
        assert!(
            matches!(&t.plan, Plan::Find { limit: Some(100), max_time: Some(d), .. } if *d == Duration::from_secs(30))
        );
        assert_eq!(t.plan.max_time(), Some(Duration::from_secs(30)));
    }

    /// El orden con nulos al final: un marcador por criterio, y el `$project`
    /// final deja pasar solo lo pedido.
    #[test]
    fn el_orden_con_los_nulos_al_final() {
        let mut p = pet(&[("n", "nombre")]);
        p.filtros = vec![f("edad", "gt", "5")];
        p.orden = vec![
            Orden {
                columna: "edad".into(),
                descendente: false,
            },
            Orden {
                columna: "nombre".into(),
                descendente: true,
            },
        ];
        p.limit = Some(5);
        let t = traducir(&p, &tipos()).unwrap();
        let Plan::Agregado { tubo, .. } = &t.plan else {
            panic!()
        };
        assert_eq!(tubo.len(), 5, "{tubo:?}");
        assert_eq!(
            tubo[0],
            doc! {"$match": {"edad": {"$gt": 5_i64, "$not": array()}}}
        );
        // Etapa 1: columnas pedidas y de orden, y los marcadores.
        assert_eq!(
            tubo[1],
            doc! {"$project": {
                "edad": 1, "nombre": 1,
                "__ore_n0": {"$cond": [{"$in": [{"$type": "$edad"}, ["null", "missing"]]}, 1, 0]},
                "__ore_n1": {"$cond": [{"$in": [{"$type": "$nombre"}, ["null", "missing"]]}, 1, 0]},
            }}
        );
        // Orden: marcador (nulos al final), columna, y `_id` de desempate.
        assert_eq!(
            tubo[2],
            doc! {"$sort": {"__ore_n0": 1, "edad": 1, "__ore_n1": 1, "nombre": -1, "_id": 1}}
        );
        assert_eq!(tubo[3], doc! {"$limit": 5_i64});
        // La proyección final es la cerrada: sin `edad` (solo ordenó) y sin marcadores.
        assert_eq!(tubo[4], doc! {"$project": {"nombre": 1, "_id": 0}});
        assert_eq!(t.plan.proyeccion(), &doc! {"nombre": 1, "_id": 0});
    }

    #[test]
    fn el_orden_sobre_un_tipo_que_no_se_ordena_como_sql_se_rechaza() {
        let mut p = pet(&[("n", "nombre")]);
        p.orden = vec![Orden {
            columna: "raro".into(),
            descendente: false,
        }];
        assert!(
            traducir(&p, &tipos())
                .unwrap_err()
                .mensaje
                .contains("no se empuja")
        );
        p.orden = vec![Orden {
            columna: "uid".into(),
            descendente: false,
        }];
        assert!(traducir(&p, &tipos()).is_err());
    }

    #[test]
    fn lo_de_los_ficheros_no_aplica() {
        let mut p = pet(&[("n", "nombre")]);
        p.listado = Some(String::new());
        assert!(
            traducir(&p, &tipos())
                .unwrap_err()
                .mensaje
                .contains("orígenes de objetos")
        );
    }

    fn con_fecha() -> Tipos {
        Tipos::new([
            ("dia".to_string(), Tipo::Fecha),
            ("hora".to_string(), Tipo::FechaHora),
        ])
    }

    fn filtro_de_fecha(fi: Filtro) -> Result<Document, Fallo> {
        let mut p = pet(&[("d", "dia")]);
        p.filtros = vec![fi];
        match traducir(&p, &con_fecha())?.plan {
            Plan::Find { filtro, .. } => Ok(filtro),
            _ => unreachable!(),
        }
    }

    /// 2000-02-29 es el día 11016 desde 1970: empieza en 951_782_400_000 ms.
    const DIA: i64 = 951_782_400_000;

    fn ms(n: i64) -> Bson {
        Bson::DateTime(DateTime::from_millis(n))
    }

    /// Un `Date` es un tramo: `eq` es `[inicio, inicio + 1 día)`.
    #[test]
    fn una_fecha_declarada_es_un_dia_entero() {
        assert_eq!(
            filtro_de_fecha(f("dia", "eq", "2000-02-29")).unwrap(),
            doc! {"dia": {"$gte": ms(DIA), "$lt": ms(DIA + 86_400_000), "$not": array()}}
        );
        assert_eq!(
            filtro_de_fecha(f("dia", "lt", "2000-02-29")).unwrap(),
            doc! {"dia": {"$lt": ms(DIA), "$not": array()}}
        );
        // `<=` incluye el día entero: es `< el día siguiente`.
        assert_eq!(
            filtro_de_fecha(f("dia", "le", "2000-02-29")).unwrap(),
            doc! {"dia": {"$lt": ms(DIA + 86_400_000), "$not": array()}}
        );
        assert_eq!(
            filtro_de_fecha(f("dia", "gt", "2000-02-29")).unwrap(),
            doc! {"dia": {"$gte": ms(DIA + 86_400_000), "$not": array()}}
        );
        assert_eq!(
            filtro_de_fecha(f("dia", "ge", "2000-02-29")).unwrap(),
            doc! {"dia": {"$gte": ms(DIA), "$not": array()}}
        );
    }

    /// `<>` son los dos tramos de fuera, y deja fuera null y ausente.
    #[test]
    fn distinto_de_un_dia_son_los_dos_tramos_de_fuera() {
        assert_eq!(
            filtro_de_fecha(f("dia", "neq", "2000-02-29")).unwrap(),
            doc! {"$or": [
                {"dia": {"$lt": ms(DIA), "$not": array()}},
                {"dia": {"$gte": ms(DIA + 86_400_000), "$not": array()}},
            ]}
        );
    }

    #[test]
    fn in_sobre_fechas_es_una_alternativa_de_dias() {
        let fi = Filtro {
            columna: "dia".into(),
            operador: "in".into(),
            valor: Valor::Lista(vec!["2000-02-29".into(), "1970-01-01".into()]),
        };
        let d = filtro_de_fecha(fi).unwrap();
        let o = d.get_array("$or").unwrap();
        assert_eq!(o.len(), 2, "{d:?}");
        // Sin valores no casa nada.
        let vacio = Filtro {
            columna: "dia".into(),
            operador: "in".into(),
            valor: Valor::Lista(vec![]),
        };
        assert_eq!(filtro_de_fecha(vacio).unwrap(), nunca());
    }

    /// Una fecha mal escrita es del operador, y nombra la columna.
    #[test]
    fn una_fecha_mal_escrita_se_rechaza() {
        let e = filtro_de_fecha(f("dia", "eq", "29/02/2000")).unwrap_err();
        assert_eq!(e.codigo, ore_driver::Codigo::Operador);
        assert!(e.mensaje.contains("`dia`"), "{}", e.mensaje);
    }

    /// Una hora sin zona se compara como el instante UTC que es en BSON.
    #[test]
    fn una_fecha_y_hora_sin_zona_es_el_instante_en_utc() {
        let mut p = pet(&[("h", "hora")]);
        p.filtros = vec![f("hora", "ge", "2000-02-29T12:30:00")];
        let Plan::Find { filtro, .. } = traducir(&p, &con_fecha()).unwrap().plan else {
            panic!()
        };
        assert_eq!(
            filtro,
            doc! {"hora": {"$gte": ms(DIA + 45_000_000), "$not": array()}}
        );
    }
}
