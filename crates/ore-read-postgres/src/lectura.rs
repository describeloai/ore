//! **El conector v2** (ADR 0053 F2·2, `docs/federation.md` §1): leer en Arrow y
//! en flujo, con tiempo, cancelación, sesiones reutilizadas y errores tipados.
//!
//! # Cinco decisiones
//!
//! 1. **Por un cursor, de [`LOTE`] en [`LOTE`] filas.** `client.query` juntaba
//!    el resultado entero antes de escribir una fila: el kit midió 388 MiB y el
//!    primer byte a los 32,9 s para 10⁶ filas (F2·1). Un cursor dentro de una
//!    transacción de sólo lectura trae cada vez un lote, que sale por stdout
//!    antes de pedir el siguiente: la memoria es la de un lote. **Por el
//!    protocolo simple** (ADR 0065 PG2, D-PG6): `DECLARE … CURSOR` y `FETCH`,
//!    sin sentencias con nombre. Con ellas, detrás de un pooler en modo
//!    transacción sin *prepared statements* (Supavisor, pgbouncer < 1.21) dos
//!    lectores a la vez chocaban: `42P05 prepared statement "s0" already
//!    exists`, 14/20 en PG0. Los parámetros van como literales (`en_linea`).
//! 2. **El tipo de cada columna, del catálogo**, con la misma traducción que
//!    `catalogo` (`traducir`): es la que escribió la `Table` del árbol, así que
//!    el tipo Arrow es el de la tabla. Sólo lo exacto se tipa —enteros,
//!    reales, lógicos, `numeric(p, s)`, fechas, instantes, texto—; lo que no
//!    (`numeric` sin precisión, lo opaco, las listas) sale como texto y el
//!    almacén lo estrecha como hasta ahora. Un valor que no cabe en su tipo es
//!    un error con su columna y su fila, nunca un nulo.
//! 3. **El valor llega en su texto canónico y de ahí va a su tipo**: el
//!    protocolo simple da el texto de Postgres, con la sesión fijada
//!    (`ajustes`: UTC, ISO, `extra_float_digits`), y `ore_core::tipos` lo
//!    analiza, el mismo analizador con el que el almacén estrecha. Así cualquier
//!    tipo se lee (PG0: se negaban 27 de 38): lo que no se tipa con exactitud
//!    sale como su texto (D-PG4). Un array de un escalar exacto es una lista de
//!    Arrow. `infinity` en una fecha va a 9999-12-31 (y `-infinity` a
//!    0001-01-01), y `NaN` o infinito en un decimal, a nulo, las dos con aviso.
//! 4. **El tiempo, en el origen**: `SET LOCAL statement_timeout` y, entre lote y
//!    lote, el reloj de la petición.
//! 5. **Cancelar, en el origen**: el `CancelToken` de la sesión en curso queda
//!    a mano de quien cancela (`SIGTERM`, `{"cancelar": id}` o cerrar la
//!    entrada de `servir`), que manda la cancelación por su propia conexión.

use crate::texto;
use arrow_array::builder::{
    BooleanBuilder, Date32Builder, Decimal128Builder, Float64Builder, Int64Builder, StringBuilder,
    Time64MicrosecondBuilder, TimestampMicrosecondBuilder,
};
use arrow_array::{ArrayRef, RecordBatch};
use arrow_schema::{DataType, Field, Schema, TimeUnit};
use ore_core::json::Json;
use ore_core::tipos::{Fisico, Valor};
use ore_driver::capacidades::Capacidades;
use ore_driver::{Codigo, Fallo, Peticion};
use std::collections::BTreeMap;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Filas por lote: las que se piden al portal de cada vez y las que lleva cada
/// `RecordBatch`.
pub const LOTE: usize = 8192;

/// La zona de un instante, como la escribe el lago (`ore-store` `carga::UTC`).
const UTC: &str = "+00:00";

pub const CAPACIDADES: Capacidades = Capacidades {
    conector: "ore-read-postgres",
    version: env!("CARGO_PKG_VERSION"),
    operadores: ore_driver::OPERADORES,
    limit: true,
    order_by: true,
    estimar: true,
    servir: true,
};

/// La cancelación de la sesión en curso, a mano de quien cancela.
pub type Token = Arc<Mutex<Sesion>>;

/// Lo que se cancela: la consulta que corre en el origen (`token`) y, entre
/// un `FETCH` y el siguiente —cuando en el origen no corre nada—, la marca
/// que el bucle mira (0053 F3·2: sin ella, cancelar a mitad de un flujo no
/// paraba el bucle, que pedía lotes hasta el final).
#[derive(Default)]
pub struct Sesion {
    pub token: Option<postgres::CancelToken>,
    /// El conector con el que se abrió (ADR 0065 D-PG1): cancelar habla TLS
    /// con la misma CA.
    pub tls: Option<postgres_native_tls::MakeTlsConnector>,
    pub cancelada: bool,
}

/// Cancela en el origen lo que corre en la sesión de `token`, si algo corre.
pub fn cancelar(token: &Token) {
    let t = token.lock().ok().and_then(|mut s| {
        s.cancelada = true;
        Some((s.token.clone()?, s.tls.clone()?))
    });
    if let Some((t, tls)) = t {
        let _ = t.cancel_query(tls);
    }
}

/// **Un error de Postgres, con su código del contrato.** Por el SQLSTATE
/// cuando lo hay; si no, es la conexión.
pub fn fallo_de(e: &postgres::Error) -> Fallo {
    if let Some(db) = e.as_db_error() {
        let estado = db.code().code();
        let codigo = match estado {
            "28P01" | "28000" => Codigo::Credencial,
            "42P01" | "42703" | "3F000" | "42704" | "42883" => Codigo::Objeto,
            "57014" => Codigo::Tiempo,
            "53300" | "57P01" | "57P03" => Codigo::Conexion,
            c if c.starts_with("08") => Codigo::Conexion,
            _ => Codigo::Origen,
        };
        // `40001` (serialización) y `40P01` (interbloqueo) salen bien a la
        // segunda; lo demás del origen, no.
        let reintentable = codigo.reintentable() || matches!(estado, "40001" | "40P01");
        return Fallo::new(codigo, format!("{estado}: {}", db.message()))
            .reintentable(reintentable);
    }
    let causa = texto::causa(e);
    if e.is_closed() || causa.contains("timed out") || causa.contains("connect") {
        return Fallo::new(Codigo::Conexion, causa);
    }
    Fallo::origen(causa)
}

/// **Abre una sesión**: TLS según D-PG1 (ADR 0065), de sólo lectura, con un
/// plazo para conectar y su nombre de aplicación (`crate::tls::abrir`).
pub fn conectar(url: &str) -> Result<postgres::Client, Fallo> {
    crate::tls::abrir(url)
}

/// `"esquema"."tabla"`, citado parte a parte: lo que `to_regclass` lee sin que
/// un nombre pueda decir otra cosa.
pub(crate) fn citado(objeto: &str) -> String {
    objeto
        .split('.')
        .map(|p| format!("\"{}\"", p.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(".")
}

/// El físico de una columna por su tipo de Postgres, o `None` si va como
/// texto.
fn fisico(formato: &str, familia: &str, base: Option<&str>) -> Option<Fisico> {
    // `time with time zone` lleva desfase y un `Time` de OOS no.
    if formato.starts_with("time with time zone") {
        return None;
    }
    let oos = crate::traducir(formato, familia, base)?;
    if !exacto(&oos) {
        return None;
    }
    let t = ore_core::types::parse_type(&oos).ok()?;
    Some(Fisico::de(&t))
}

/// ¿Se tipa con exactitud lo que `traducir` dice? Es la misma pregunta para
/// el catálogo y para la lectura: así el `type` de la `Table` es lo que sale.
pub(crate) fn exacto(oos: &str) -> bool {
    matches!(
        oos,
        "Integer" | "Float" | "Boolean" | "Date" | "Time" | "DateTime" | "DateTimeTz" | "String"
    ) || oos.starts_with("Decimal<")
}

/// Lo que sale de una columna: un escalar —con su físico, o como texto— o la
/// lista de un escalar exacto (`int4[]` → `List<Int64>`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Forma {
    Escalar(Option<Fisico>),
    Lista(Fisico),
}

impl Forma {
    pub(crate) fn de(formato: &str, familia: &str, base: Option<&str>) -> Forma {
        match formato.trim().strip_suffix("[]") {
            Some(dentro) => match fisico(dentro.trim(), "b", None) {
                Some(f) => Forma::Lista(f),
                None => Forma::Escalar(None),
            },
            None => Forma::Escalar(fisico(formato, familia, base)),
        }
    }

    pub(crate) fn escalar(&self) -> Option<&Fisico> {
        match self {
            Forma::Escalar(f) => f.as_ref(),
            Forma::Lista(f) => Some(f),
        }
    }
}

/// Las columnas del objeto, con su forma. `objeto` si no existe.
pub(crate) fn columnas(
    c: &mut postgres::Client,
    objeto: &str,
) -> Result<Vec<(String, Forma)>, Fallo> {
    let citado = citado(objeto);
    let filas = c
        .query_typed(
            "SELECT a.attname::text, format_type(a.atttypid, a.atttypmod), t.typtype::text, \
                    CASE WHEN t.typtype = 'd' THEN format_type(t.typbasetype, t.typtypmod) END, \
                    to_regclass($1) IS NOT NULL \
             FROM (SELECT to_regclass($1) AS r) o \
             LEFT JOIN pg_attribute a ON a.attrelid = o.r AND a.attnum > 0 AND NOT a.attisdropped \
             LEFT JOIN pg_type t ON t.oid = a.atttypid \
             ORDER BY a.attnum",
            &[(
                &citado as &(dyn postgres::types::ToSql + Sync),
                postgres::types::Type::TEXT,
            )],
        )
        .map_err(|e| fallo_de(&e))?;
    if filas.first().is_none_or(|f| !f.get::<_, bool>(4)) {
        return Err(Fallo::new(
            Codigo::Objeto,
            format!("`{objeto}` no existe en el origen (o no se puede ver)"),
        ));
    }
    Ok(filas
        .iter()
        .filter_map(|f| {
            let nombre: Option<String> = f.get(0);
            let formato: Option<String> = f.get(1);
            let familia: Option<String> = f.get(2);
            let base: Option<String> = f.get(3);
            Some((
                nombre?,
                Forma::de(&formato?, &familia.unwrap_or_default(), base.as_deref()),
            ))
        })
        .collect())
}

fn tipo_arrow(f: &Forma) -> DataType {
    match f {
        Forma::Escalar(f) => tipo_escalar(f.as_ref()),
        Forma::Lista(f) => {
            DataType::List(Arc::new(Field::new("item", tipo_escalar(Some(f)), true)))
        }
    }
}

fn tipo_escalar(f: Option<&Fisico>) -> DataType {
    match f {
        None | Some(Fisico::Texto) => DataType::Utf8,
        Some(Fisico::Entero) => DataType::Int64,
        Some(Fisico::Real) => DataType::Float64,
        Some(Fisico::Logico) => DataType::Boolean,
        Some(Fisico::Decimal { precision, escala }) => {
            DataType::Decimal128(*precision, *escala as i8)
        }
        Some(Fisico::Fecha) => DataType::Date32,
        Some(Fisico::Hora) => DataType::Time64(TimeUnit::Microsecond),
        Some(Fisico::FechaHora) => DataType::Timestamp(TimeUnit::Microsecond, None),
        Some(Fisico::Instante) => DataType::Timestamp(TimeUnit::Microsecond, Some(UTC.into())),
    }
}

/// Una columna de Arrow en construcción.
enum Col {
    Texto(StringBuilder),
    Entero(Int64Builder),
    Real(Float64Builder),
    Logico(BooleanBuilder),
    Decimal(Decimal128Builder, u8, u8),
    Fecha(Date32Builder),
    Hora(Time64MicrosecondBuilder),
    FechaHora(TimestampMicrosecondBuilder),
    Instante(TimestampMicrosecondBuilder),
    /// Una lista: los elementos en su columna, y dónde empieza cada fila.
    Lista {
        dentro: Box<Col>,
        fisico: Fisico,
        offsets: Vec<i32>,
        nulos: Vec<bool>,
    },
}

/// Lo que un valor necesitó para caber, contado por columna y dicho al final.
pub(crate) type Aviso = &'static str;

const AVISO_INFINITO_FECHA: Aviso =
    "valores `infinity`/`-infinity` llevados a 9999-12-31 / 0001-01-01";
const AVISO_DECIMAL: Aviso = "valores `NaN`/infinito en un decimal, como nulos";

/// **El texto de Postgres, hacia el analizador.** Lo que el protocolo simple
/// escribe distinto de lo que `ore_core::tipos` lee: `t`/`f`, y los valores
/// que el tipo de destino no tiene.
pub(crate) fn normalizar<'a>(
    f: &Fisico,
    t: &'a str,
) -> (Option<std::borrow::Cow<'a, str>>, Option<Aviso>) {
    use std::borrow::Cow::{Borrowed, Owned};
    let fecha = |fin: &str, ini: &str| match t {
        "infinity" => (Some(Owned(fin.to_string())), Some(AVISO_INFINITO_FECHA)),
        "-infinity" => (Some(Owned(ini.to_string())), Some(AVISO_INFINITO_FECHA)),
        _ => (Some(Borrowed(t)), None),
    };
    match f {
        Fisico::Logico => match t {
            "t" => (Some(Borrowed("true")), None),
            "f" => (Some(Borrowed("false")), None),
            _ => (Some(Borrowed(t)), None),
        },
        Fisico::Decimal { .. } if matches!(t, "NaN" | "Infinity" | "-Infinity") => {
            (None, Some(AVISO_DECIMAL))
        }
        Fisico::Fecha => fecha("9999-12-31", "0001-01-01"),
        Fisico::FechaHora => fecha("9999-12-31 23:59:59.999999", "0001-01-01 00:00:00"),
        Fisico::Instante => fecha("9999-12-31 23:59:59.999999+00", "0001-01-01 00:00:00+00"),
        _ => (Some(Borrowed(t)), None),
    }
}

/// **Los elementos de un array de una dimensión**, de su texto
/// (`{1,2,NULL,"a,b"}`). Uno de más dimensiones no cabe en una lista.
pub(crate) fn elementos(t: &str) -> Result<Vec<Option<String>>, String> {
    // `[2:3]={…}`: los límites, si no empiezan en 1.
    let t = if t.starts_with('[') {
        t.split_once('=').map(|(_, r)| r).unwrap_or(t)
    } else {
        t
    };
    let dentro = t
        .strip_prefix('{')
        .and_then(|r| r.strip_suffix('}'))
        .ok_or_else(|| format!("`{t}` no es un array"))?;
    if dentro.starts_with('{') {
        return Err("un array de más de una dimensión no cabe en una lista".into());
    }
    let mut out = Vec::new();
    if dentro.is_empty() {
        return Ok(out);
    }
    let mut cs = dentro.chars().peekable();
    loop {
        if cs.peek() == Some(&'"') {
            cs.next();
            let mut v = String::new();
            loop {
                match cs.next() {
                    Some('\\') => v.extend(cs.next()),
                    Some('"') => break,
                    Some(c) => v.push(c),
                    None => return Err(format!("`{t}`: una comilla sin cerrar")),
                }
            }
            out.push(Some(v));
        } else {
            let mut v = String::new();
            while let Some(&c) = cs.peek() {
                if c == ',' {
                    break;
                }
                v.push(c);
                cs.next();
            }
            out.push(if v.eq_ignore_ascii_case("NULL") {
                None
            } else {
                Some(v)
            });
        }
        match cs.next() {
            Some(',') => continue,
            None => break,
            Some(c) => return Err(format!("`{t}`: `{c}` donde iba una coma")),
        }
    }
    Ok(out)
}

impl Col {
    fn de(f: &Forma) -> Col {
        match f {
            Forma::Escalar(f) => Col::nueva(f.as_ref()),
            Forma::Lista(f) => Col::Lista {
                dentro: Box::new(Col::nueva(Some(f))),
                fisico: *f,
                offsets: vec![0],
                nulos: Vec::new(),
            },
        }
    }

    fn nueva(f: Option<&Fisico>) -> Col {
        match f {
            None | Some(Fisico::Texto) => Col::Texto(StringBuilder::new()),
            Some(Fisico::Entero) => Col::Entero(Int64Builder::new()),
            Some(Fisico::Real) => Col::Real(Float64Builder::new()),
            Some(Fisico::Logico) => Col::Logico(BooleanBuilder::new()),
            Some(Fisico::Decimal { precision, escala }) => {
                Col::Decimal(Decimal128Builder::new(), *precision, *escala)
            }
            Some(Fisico::Fecha) => Col::Fecha(Date32Builder::new()),
            Some(Fisico::Hora) => Col::Hora(Time64MicrosecondBuilder::new()),
            Some(Fisico::FechaHora) => Col::FechaHora(TimestampMicrosecondBuilder::new()),
            Some(Fisico::Instante) => Col::Instante(TimestampMicrosecondBuilder::new()),
        }
    }

    /// Añade un valor en su texto de Postgres: lo normaliza, lo analiza y
    /// devuelve el aviso, si lo hubo. `Err` con el tipo si no cabe.
    fn poner(&mut self, f: Option<&Fisico>, v: Option<&str>) -> Result<Option<Aviso>, String> {
        if let Col::Lista {
            dentro,
            fisico,
            offsets,
            nulos,
        } = self
        {
            let mut aviso = None;
            match v {
                None => nulos.push(false),
                Some(t) => {
                    for e in elementos(t)? {
                        aviso = dentro.poner(Some(fisico), e.as_deref())?.or(aviso);
                    }
                    nulos.push(true);
                }
            }
            offsets.push(dentro.largo() as i32);
            return Ok(aviso);
        }
        let (v, aviso) = match (f, v) {
            (Some(f), Some(t)) if !matches!(self, Col::Texto(_)) => normalizar(f, t),
            (_, v) => (v.map(std::borrow::Cow::Borrowed), None),
        };
        self.poner_normal(f, v.as_deref())?;
        Ok(aviso)
    }

    /// Cuántos valores lleva (para los `offsets` de una lista).
    fn largo(&self) -> usize {
        use arrow_array::builder::ArrayBuilder;
        match self {
            Col::Texto(b) => b.len(),
            Col::Entero(b) => b.len(),
            Col::Real(b) => b.len(),
            Col::Logico(b) => b.len(),
            Col::Decimal(b, ..) => b.len(),
            Col::Fecha(b) => b.len(),
            Col::Hora(b) => b.len(),
            Col::FechaHora(b) => b.len(),
            Col::Instante(b) => b.len(),
            Col::Lista { nulos, .. } => nulos.len(),
        }
    }

    fn poner_normal(&mut self, f: Option<&Fisico>, v: Option<&str>) -> Result<(), String> {
        let Some(t) = v else {
            match self {
                Col::Texto(b) => b.append_null(),
                Col::Entero(b) => b.append_null(),
                Col::Real(b) => b.append_null(),
                Col::Logico(b) => b.append_null(),
                Col::Decimal(b, ..) => b.append_null(),
                Col::Fecha(b) => b.append_null(),
                Col::Hora(b) => b.append_null(),
                Col::FechaHora(b) => b.append_null(),
                Col::Instante(b) => b.append_null(),
                Col::Lista { .. } => unreachable!("una lista se pone por `poner`"),
            }
            return Ok(());
        };
        if let Col::Texto(b) = self {
            b.append_value(t);
            return Ok(());
        }
        let fisico = f.expect("una columna tipada tiene físico");
        let valor = fisico
            .analizar(t)
            .ok_or_else(|| format!("`{t}` no es un {}", fisico.arrow()))?;
        match (self, valor) {
            (Col::Entero(b), Valor::Entero(n)) => b.append_value(n),
            (Col::Real(b), Valor::Real(n)) => b.append_value(n),
            (Col::Logico(b), Valor::Logico(n)) => b.append_value(n),
            (Col::Decimal(b, ..), Valor::Decimal(n)) => b.append_value(n),
            (Col::Fecha(b), Valor::Fecha(n)) => b.append_value(n),
            (Col::Hora(b), Valor::Hora(n)) => b.append_value(n),
            (Col::FechaHora(b), Valor::FechaHora(n)) => b.append_value(n),
            (Col::Instante(b), Valor::Instante(n)) => b.append_value(n),
            _ => return Err(format!("`{t}` no es un {}", fisico.arrow())),
        }
        Ok(())
    }

    fn fin(&mut self) -> Result<ArrayRef, String> {
        Ok(match self {
            Col::Texto(b) => Arc::new(b.finish()),
            Col::Entero(b) => Arc::new(b.finish()),
            Col::Real(b) => Arc::new(b.finish()),
            Col::Logico(b) => Arc::new(b.finish()),
            Col::Decimal(b, p, s) => Arc::new(
                b.finish()
                    .with_precision_and_scale(*p, *s as i8)
                    .map_err(|e| e.to_string())?,
            ),
            Col::Fecha(b) => Arc::new(b.finish()),
            Col::Hora(b) => Arc::new(b.finish()),
            Col::FechaHora(b) => Arc::new(b.finish()),
            Col::Instante(b) => Arc::new(b.finish().with_timezone(UTC)),
            Col::Lista {
                dentro,
                offsets,
                nulos,
                ..
            } => {
                let valores = dentro.fin()?;
                let campo = Arc::new(Field::new("item", valores.data_type().clone(), true));
                let off = std::mem::replace(offsets, vec![0]);
                let nul = std::mem::take(nulos);
                Arc::new(
                    arrow_array::ListArray::try_new(
                        campo,
                        arrow_buffer::OffsetBuffer::new(arrow_buffer::ScalarBuffer::from(off)),
                        valores,
                        Some(arrow_buffer::NullBuffer::from(nul)),
                    )
                    .map_err(|e| e.to_string())?,
                )
            }
        })
    }
}

/// La consulta de la petición, ya traducida, y lo que hace falta para leerla.
struct Plan {
    consulta: ore_sql::Consulta,
    /// Por propiedad: su posición en el resultado, su forma y su columna.
    salida: Vec<(String, usize, Forma, String)>,
    esquema: Arc<Schema>,
}

fn planear(c: &mut postgres::Client, p: &Peticion) -> Result<Plan, Fallo> {
    if let Some(porque) = ore_driver::rango_servible(p, true, false) {
        return Err(Fallo::operador(porque));
    }
    CAPACIDADES.admite(p)?;
    let consulta = ore_sql::preparar(p, &ore_sql::dialectos::POSTGRES, &p.objeto, || {
        unreachable!("`POSTGRES` no exige tipos")
    })
    .map_err(Fallo::operador)?;
    let cols = columnas(c, &p.objeto)?;
    let mut salida = Vec::new();
    for (prop, col) in &p.proyeccion {
        let f = cols
            .iter()
            .find(|(n, _)| n == col)
            .ok_or_else(|| deriva(&p.objeto, col, &cols))?
            .1
            .clone();
        let i = consulta
            .columnas
            .iter()
            .position(|x| x == col)
            .ok_or_else(|| Fallo::origen(format!("la consulta no pide `{col}`")))?;
        salida.push((prop.clone(), i, f, col.clone()));
    }
    // Las de los filtros y el orden también tienen que existir: un nombre que
    // no está es `objeto`, no un error del servidor a mitad.
    for col in p
        .filtros
        .iter()
        .map(|f| &f.columna)
        .chain(p.orden.iter().map(|o| &o.columna))
    {
        if !cols.iter().any(|(n, _)| n == col) {
            return Err(deriva(&p.objeto, col, &cols));
        }
    }
    let esquema = Arc::new(Schema::new(
        salida
            .iter()
            .map(|(prop, _, f, _)| Field::new(prop, tipo_arrow(f), true))
            .collect::<Vec<_>>(),
    ));
    Ok(Plan {
        consulta,
        salida,
        esquema,
    })
}

/// **La consulta con sus parámetros como literales**, para el protocolo
/// simple. Un literal sin tipo (`'7'`) lo coacciona el servidor igual que un
/// parámetro en texto sin tipo, que es lo que se mandaba: un `$1` sobre un
/// `int4` sigue siendo un `int4`. Las comillas se doblan, y `ajustes` fija
/// `standard_conforming_strings`: una barra no escapa nada. Los `$n` dentro de
/// un identificador o de un literal no se tocan.
pub(crate) fn en_linea(c: &ore_sql::Consulta) -> String {
    let mut out = String::with_capacity(c.texto.len());
    let mut cs = c.texto.chars().peekable();
    let mut dentro: Option<char> = None;
    while let Some(ch) = cs.next() {
        match dentro {
            Some(q) => {
                out.push(ch);
                if ch == q {
                    dentro = None;
                }
            }
            None if ch == '"' || ch == '\'' => {
                dentro = Some(ch);
                out.push(ch);
            }
            None if ch == '$' && cs.peek().is_some_and(|d| d.is_ascii_digit()) => {
                let mut n = String::new();
                while let Some(d) = cs.peek().filter(|d| d.is_ascii_digit()) {
                    n.push(*d);
                    cs.next();
                }
                match n
                    .parse::<usize>()
                    .ok()
                    .and_then(|k| c.parametros.get(k.wrapping_sub(1)))
                {
                    Some(v) => {
                        out.push('\'');
                        out.push_str(&v.replace('\'', "''"));
                        out.push('\'');
                    }
                    None => {
                        out.push('$');
                        out.push_str(&n);
                    }
                }
            }
            None => out.push(ch),
        }
    }
    out
}

/// **La sesión, fijada en cada transacción** (`SET LOCAL`: detrás de un pooler
/// en modo transacción, un `SET` de sesión se pierde). Es lo que hace que el
/// texto sea el canónico: instantes en UTC, fechas ISO, el real más corto que
/// vuelve al mismo número y `bytea` en hexadecimal.
pub(crate) fn ajustes(timeout_ms: Option<u64>) -> String {
    let mut s = String::from(
        "SET LOCAL standard_conforming_strings = on; SET LOCAL TimeZone = 'UTC'; \
         SET LOCAL DateStyle = 'ISO, MDY'; SET LOCAL IntervalStyle = 'postgres'; \
         SET LOCAL extra_float_digits = 1; SET LOCAL bytea_output = 'hex'",
    );
    if let Some(ms) = timeout_ms {
        s.push_str(&format!("; SET LOCAL statement_timeout = {ms}"));
    }
    s
}

/// Las filas de lo que devolvió el protocolo simple.
pub(crate) fn filas_de(m: Vec<postgres::SimpleQueryMessage>) -> Vec<postgres::SimpleQueryRow> {
    m.into_iter()
        .filter_map(|x| match x {
            postgres::SimpleQueryMessage::Row(r) => Some(r),
            _ => None,
        })
        .collect()
}

/// **Leer**: la petición, en Arrow y en flujo, por `salida`. Devuelve las filas.
pub fn leer(
    c: &mut postgres::Client,
    token: &Token,
    p: &Peticion,
    salida: &mut dyn Write,
) -> Result<u64, Fallo> {
    let inicio = Instant::now();
    let plan = planear(c, p)?;
    if let Ok(mut s) = token.lock() {
        s.token = Some(c.cancel_token());
        s.tls = crate::tls::conector_de(&p.url);
        s.cancelada = false;
    }
    let mut tx = c
        .build_transaction()
        .read_only(true)
        .start()
        .map_err(|e| fallo_de(&e))?;
    tx.batch_execute(&ajustes(p.timeout_ms))
        .map_err(|e| fallo_de(&e))?;
    tx.batch_execute(&format!(
        "DECLARE ore_lectura NO SCROLL CURSOR FOR {}",
        en_linea(&plan.consulta)
    ))
    .map_err(|e| fallo_de(&e))?;
    let mut avisos: BTreeMap<(String, Aviso), u64> = BTreeMap::new();
    let mut escritor = arrow_ipc::writer::StreamWriter::try_new(&mut *salida, &plan.esquema)
        .map_err(|e| Fallo::origen(format!("no se pudo empezar el flujo: {e}")))?;
    let mut n: u64 = 0;
    loop {
        if let Some(ms) = p.timeout_ms
            && inicio.elapsed() > Duration::from_millis(ms)
        {
            return Err(Fallo::new(
                Codigo::Tiempo,
                format!("se agotaron los {ms} ms con {n} filas leídas"),
            ));
        }
        if token.lock().map(|s| s.cancelada).unwrap_or(false) {
            return Err(Fallo::origen(format!("cancelada con {n} filas leídas")));
        }
        let filas = filas_de(
            tx.simple_query(&format!("FETCH {LOTE} FROM ore_lectura"))
                .map_err(|e| fallo_de(&e))?,
        );
        if filas.is_empty() {
            break;
        }
        let mut cols: Vec<Col> = plan.salida.iter().map(|(_, _, f, _)| Col::de(f)).collect();
        for (k, fila) in filas.iter().enumerate() {
            for ((prop, i, f, columna), col) in plan.salida.iter().zip(cols.iter_mut()) {
                let v = fila.try_get(*i).map_err(|e| {
                    Fallo::origen(format!(
                        "la columna `{columna}` no se pudo leer: {}",
                        texto::causa(&e)
                    ))
                })?;
                if let Some(a) = col.poner(f.escalar(), v).map_err(|e| {
                    Fallo::origen(format!(
                        "`{prop}` (la columna `{columna}`), fila {}: {e}",
                        n + k as u64 + 1
                    ))
                })? {
                    *avisos.entry((columna.clone(), a)).or_default() += 1;
                }
            }
        }
        let arrays = cols
            .iter_mut()
            .map(Col::fin)
            .collect::<Result<Vec<_>, _>>()
            .map_err(Fallo::origen)?;
        let lote = RecordBatch::try_new(plan.esquema.clone(), arrays)
            .map_err(|e| Fallo::origen(format!("el lote no casa: {e}")))?;
        escritor
            .write(&lote)
            .map_err(|e| Fallo::new(Codigo::Conexion, format!("no se pudo escribir: {e}")))?;
        escritor
            .flush()
            .map_err(|e| Fallo::new(Codigo::Conexion, format!("no se pudo escribir: {e}")))?;
        n += filas.len() as u64;
        if filas.len() < LOTE {
            break;
        }
    }
    escritor
        .finish()
        .map_err(|e| Fallo::new(Codigo::Conexion, format!("no se pudo cerrar el flujo: {e}")))?;
    tx.batch_execute("CLOSE ore_lectura")
        .map_err(|e| fallo_de(&e))?;
    tx.commit().map_err(|e| fallo_de(&e))?;
    for ((columna, aviso), k) in &avisos {
        eprintln!("ore-read-postgres: aviso · `{columna}`: {k} {aviso}");
    }
    if let Ok(mut s) = token.lock() {
        s.token = None;
    }
    Ok(n)
}

/// **Estimar**: lo que el planificador de Postgres cree que devolvería la
/// petición, sin leerla (`EXPLAIN`, sin `ANALYZE`).
pub fn estimar(c: &mut postgres::Client, p: &Peticion) -> Result<String, Fallo> {
    let plan = planear(c, p)?;
    let filas = filas_de(
        c.simple_query(&format!(
            "EXPLAIN (FORMAT JSON) {}",
            en_linea(&plan.consulta)
        ))
        .map_err(|e| fallo_de(&e))?,
    );
    let json = filas
        .first()
        .ok_or_else(|| Fallo::origen("EXPLAIN no devolvió nada"))?
        .try_get(0)
        .map_err(|e| Fallo::origen(texto::causa(&e)))?
        .unwrap_or_default()
        .to_string();
    let raiz = ore_core::parse::parse(&json)
        .map_err(|e| Fallo::origen(format!("el plan no es JSON: {e:?}")))?;
    let plan_raiz = raiz
        .items()
        .first()
        .and_then(|x| x.get("Plan"))
        .map(|(_, v)| v)
        .ok_or_else(|| Fallo::origen("el plan no trae `Plan`"))?;
    let numero = |k: &str| {
        plan_raiz
            .get(k)
            .and_then(|(_, v)| v.as_str())
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0)
    };
    let filas = numero("Plan Rows").max(0.0) as i64;
    let ancho = numero("Plan Width").max(0.0) as i64;
    Ok(Json::obj([
        ("bytes", Json::Int(filas.saturating_mul(ancho))),
        ("filas", Json::Int(filas)),
        ("fuente", Json::s("EXPLAIN")),
    ])
    .jcs())
}

/// **La tabla está y le falta una columna declarada** (ADR 0063 S5b): el
/// esquema del origen cambió. `esquema` y no `objeto`, con lo que tiene ahora.
fn deriva<T>(objeto: &str, col: &str, cols: &[(String, T)]) -> Fallo {
    Fallo::new(
        Codigo::Esquema,
        format!(
            "the source schema changed: `{objeto}` no longer has the column `{col}` (its columns now: {})",
            cols.iter()
                .map(|(n, _)| n.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_tipo_sale_del_catalogo_y_solo_lo_exacto_se_tipa() {
        assert_eq!(fisico("bigint", "b", None), Some(Fisico::Entero));
        assert_eq!(
            fisico("numeric(12,2)", "b", None),
            Some(Fisico::Decimal {
                precision: 12,
                escala: 2
            })
        );
        assert_eq!(fisico("numeric", "b", None), None, "sin precisión, texto");
        assert_eq!(fisico("jsonb", "b", None), None);
        assert_eq!(fisico("integer[]", "b", None), None);
        assert_eq!(fisico("time with time zone", "b", None), None);
        assert_eq!(
            fisico("timestamp with time zone", "b", None),
            Some(Fisico::Instante)
        );
        assert_eq!(fisico("nif", "d", Some("text")), Some(Fisico::Texto));
    }

    #[test]
    fn un_nombre_se_cita_parte_a_parte() {
        assert_eq!(citado("kit.tipos"), "\"kit\".\"tipos\"");
        assert_eq!(citado("a\"; drop"), "\"a\"\"; drop\"");
    }

    #[test]
    fn un_valor_que_no_cabe_no_es_un_nulo() {
        let f = Fisico::Decimal {
            precision: 4,
            escala: 2,
        };
        let mut c = Col::nueva(Some(&f));
        assert!(c.poner(Some(&f), Some("12.50")).is_ok());
        assert!(c.poner(Some(&f), Some("doce")).is_err());
        assert!(c.poner(Some(&f), None).is_ok());
        // `NaN` no tiene sitio en un decimal: nulo, y se dice (D-PG4).
        assert_eq!(c.poner(Some(&f), Some("NaN")).unwrap(), Some(AVISO_DECIMAL));
        let a = c.fin().unwrap();
        assert_eq!((a.len(), a.null_count()), (3, 2));
    }

    #[test]
    fn el_texto_de_postgres_se_normaliza() {
        assert_eq!(normalizar(&Fisico::Logico, "t").0.as_deref(), Some("true"));
        assert_eq!(normalizar(&Fisico::Logico, "f").0.as_deref(), Some("false"));
        let (v, a) = normalizar(&Fisico::Fecha, "infinity");
        assert_eq!(
            (v.as_deref(), a),
            (Some("9999-12-31"), Some(AVISO_INFINITO_FECHA))
        );
        for (f, t) in [
            (Fisico::Fecha, "-infinity"),
            (Fisico::FechaHora, "infinity"),
            (Fisico::Instante, "-infinity"),
        ] {
            let (v, _) = normalizar(&f, t);
            let mut c = Col::nueva(Some(&f));
            c.poner_normal(Some(&f), v.as_deref())
                .unwrap_or_else(|e| panic!("{t} en {f:?}: {e}"));
        }
        assert_eq!(normalizar(&Fisico::Entero, "7").1, None);
    }

    #[test]
    fn los_elementos_de_un_array() {
        let e = |t: &str| elementos(t).unwrap();
        assert_eq!(e("{}"), Vec::<Option<String>>::new());
        assert_eq!(
            e("{1,2,NULL}"),
            vec![Some("1".into()), Some("2".into()), None]
        );
        assert_eq!(
            e(r#"{"a,b","x\"y",NULL,"NULL"}"#),
            vec![
                Some("a,b".into()),
                Some("x\"y".into()),
                None,
                Some("NULL".into())
            ]
        );
        assert_eq!(e("[2:3]={7,8}"), vec![Some("7".into()), Some("8".into())]);
        assert!(
            elementos("{{1,2},{3,4}}")
                .unwrap_err()
                .contains("dimensión")
        );
        assert!(elementos("nada").is_err());
    }

    #[test]
    fn una_lista_es_una_lista_de_arrow() {
        let f = Forma::Lista(Fisico::Entero);
        let mut c = Col::de(&f);
        c.poner(f.escalar(), Some("{1,2,NULL}")).unwrap();
        c.poner(f.escalar(), None).unwrap();
        c.poner(f.escalar(), Some("{}")).unwrap();
        let a = c.fin().unwrap();
        assert_eq!(a.data_type(), &tipo_arrow(&f));
        assert_eq!((a.len(), a.null_count()), (3, 1));
        let l = a.as_any().downcast_ref::<arrow_array::ListArray>().unwrap();
        assert_eq!(l.value(0).len(), 3);
        assert_eq!(l.value(2).len(), 0);
        assert!(c.poner(f.escalar(), Some("{x}")).is_err());
    }

    #[test]
    fn los_parametros_van_como_literales() {
        let c = ore_sql::Consulta {
            texto: r#"SELECT "a$1" FROM "t" WHERE "id" IN ($1, $2) AND "x" = '$1' AND "y" = $10"#
                .into(),
            parametros: vec!["7".into(), "o'brien".into()],
            columnas: vec![],
        };
        assert_eq!(
            en_linea(&c),
            r#"SELECT "a$1" FROM "t" WHERE "id" IN ('7', 'o''brien') AND "x" = '$1' AND "y" = $10"#
        );
    }
}
