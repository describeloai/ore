//! **El conector v2** (ADR 0053 F2·2, `docs/federation.md` §1): leer en Arrow y
//! en flujo, con tiempo, cancelación, sesiones reutilizadas y errores tipados.
//!
//! # Cinco decisiones
//!
//! 1. **En flujo, de [`LOTE`] en [`LOTE`] filas.** `client.query` juntaba el
//!    resultado entero antes de escribir una fila: el kit midió 388 MiB y el
//!    primer byte a los 32,9 s para 10⁶ filas (F2·1). Las filas llegan por
//!    `query_typed_raw`, que las va dando según llegan (el canal de
//!    `tokio-postgres` es de uno: el servidor espera a que se lean), y salen
//!    por stdout de lote en lote: la memoria es la de un lote. **Con una
//!    sentencia sin nombre** (ADR 0065 PG2, D-PG6): con nombre, detrás de un
//!    pooler en modo transacción sin *prepared statements* (Supavisor,
//!    pgbouncer < 1.21) dos lectores a la vez chocaban (`42P05 prepared
//!    statement "s0" already exists`, 14/20 en PG0); la sin nombre se prepara,
//!    se ata y se ejecuta en el mismo viaje. Los parámetros van como literales
//!    (`en_linea`). Hasta ORE 0067 era un cursor por el protocolo simple, con
//!    un viaje de ida y vuelta por lote y todo en texto (`FETCH`).
//! 2. **El tipo de cada columna, del catálogo**, con la misma traducción que
//!    `catalogo` (`traducir`): es la que escribió la `Table` del árbol, así que
//!    el tipo Arrow es el de la tabla. Sólo lo exacto se tipa —enteros,
//!    reales, lógicos, `numeric(p, s)`, fechas, instantes, texto—; lo que no
//!    (`numeric` sin precisión, lo opaco, las listas) sale como texto y el
//!    almacén lo estrecha como hasta ahora. Un valor que no cabe en su tipo es
//!    un error con su columna y su fila, nunca un nulo.
//! 3. **El valor llega en su texto canónico y de ahí va a su tipo**: la
//!    consulta pide cada columna por su función de salida (`format('%s', …)`,
//!    el texto que daba el protocolo simple), con la sesión fijada
//!    (`ajustes`: UTC, ISO, `extra_float_digits`), y `ore_core::tipos` lo
//!    analiza, el mismo analizador con el que el almacén estrecha. Así cualquier
//!    tipo se lee (PG0: se negaban 27 de 38): lo que no se tipa con exactitud
//!    sale como su texto (D-PG4). Un array de un escalar exacto es una lista de
//!    Arrow. `infinity` en una fecha va a 9999-12-31 (y `-infinity` a
//!    0001-01-01), y `NaN` o infinito en un decimal, a nulo, las dos con aviso.
//!    **La geometría no** (ORE 0067): una columna `geometry`/`geography`
//!    tipada (`Geometry<…>`, `Geography<…>`) llega en binario, el EWKB de su
//!    `send`, y sale como `Binary`, que el códec del lago convierte. En hex
//!    eran el doble de bytes.
//! 4. **El tiempo, en el origen**: `SET LOCAL statement_timeout` y, entre lote y
//!    lote, el reloj de la petición.
//! 5. **Cancelar, en el origen**: el `CancelToken` de la sesión en curso queda
//!    a mano de quien cancela (`SIGTERM`, `{"cancelar": id}` o cerrar la
//!    entrada de `servir`), que manda la cancelación por su propia conexión.

use crate::texto;
use arrow_array::RecordBatch;
use arrow_schema::{DataType, Field, Schema};
use ore_arrow::{Columna, RecordBatchBuilder};
use ore_base::json::Json;
use ore_base::tipos::Fisico;
use ore_driver::capacidades::Capacidades;
use ore_driver::{Codigo, Fallo, Peticion};
use std::collections::BTreeMap;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Filas por lote: las que se piden al portal de cada vez y las que lleva cada
/// `RecordBatch`.
pub const LOTE: usize = 8192;

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
/// un lote y el siguiente, la marca que el bucle mira (0053 F3·2: sin ella, cancelar a mitad de un flujo no
/// paraba el bucle, que pedía lotes hasta el final).
#[derive(Default)]
pub struct Sesion {
    pub token: Option<postgres::CancelToken>,
    /// El conector con el que se abrió (ADR 0065 D-PG1): cancelar habla TLS
    /// con la misma CA.
    pub tls: Option<postgres_native_tls::MakeTlsConnector>,
    pub cancelada: bool,
    /// Las sesiones de los tramos de una lectura repartida (ORE 0067).
    pub tramos: Vec<(postgres::CancelToken, postgres_native_tls::MakeTlsConnector)>,
}

/// Cancela en el origen lo que corre en la sesión de `token`, si algo corre.
pub fn cancelar(token: &Token) {
    let Ok(mut s) = token.lock() else { return };
    s.cancelada = true;
    if let (Some(t), Some(tls)) = (&s.token, &s.tls) {
        let _ = t.cancel_query(tls.clone());
    }
    for (t, tls) in &s.tramos {
        let _ = t.cancel_query(tls.clone());
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
    let t = ore_base::types::parse_type(&oos).ok()?;
    Some(Fisico::de(&t))
}

/// ¿Se tipa con exactitud lo que `traducir` dice? Es la misma pregunta para
/// el catálogo y para la lectura: así el `type` de la `Table` es lo que sale.
pub(crate) fn exacto(oos: &str) -> bool {
    matches!(
        oos,
        "Integer"
            | "Float"
            | "Boolean"
            | "Date"
            | "Time"
            | "DateTime"
            | "DateTimeTz"
            | "String"
            | "Binary"
    ) || oos.starts_with("Decimal<")
}

/// Lo que sale de una columna: un escalar —con su físico, o como texto— o la
/// lista de un escalar exacto (`int4[]` → `List<Int64>`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Forma {
    Escalar(Option<Fisico>),
    Lista(Fisico),
    /// ORE 0067: `geometry`/`geography` con su tipo de OOS, en binario (EWKB).
    Geo,
}

impl Forma {
    pub(crate) fn de(formato: &str, familia: &str, base: Option<&str>) -> Forma {
        if crate::traducir(formato, familia, base)
            .and_then(|t| ore_base::types::parse_type(&t).ok())
            .is_some_and(|t| t.es_geo())
        {
            return Forma::Geo;
        }
        match formato.trim().strip_suffix("[]") {
            Some(dentro) => match fisico(dentro.trim(), "b", None) {
                // Una lista de binarios sale como texto, como dice el catálogo (`Opaque`).
                Some(f) if f != Fisico::Binario => Forma::Lista(f),
                _ => Forma::Escalar(None),
            },
            None => Forma::Escalar(fisico(formato, familia, base)),
        }
    }

    pub(crate) fn escalar(&self) -> Option<&Fisico> {
        match self {
            Forma::Escalar(f) => f.as_ref(),
            Forma::Lista(f) => Some(f),
            Forma::Geo => None,
        }
    }

    /// Cómo la pide la consulta: la geometría tal cual (binario) y lo demás
    /// por su función de salida, que es el texto canónico (un `::text` no lo
    /// es: `true` por `t`, la máscara de un `inet`, sin los blancos de un
    /// `char(n)`).
    fn en_la_consulta(&self, col: &str) -> String {
        let c = format!("ore_q.\"{}\"", col.replace('"', "\"\""));
        match self {
            // `::bytea`, el EWKB de su `send`, con un tipo que el protocolo
            // conoce: un tipo de extensión en el resultado hace que
            // `tokio-postgres` pregunte por él (otra consulta) con las filas a
            // medio llegar, y las dos se esperan para siempre en cuanto no
            // caben en el búfer (pasaba a partir de unos cientos de puntos).
            Forma::Geo => format!("{c}::bytea"),
            _ => format!("CASE WHEN {c} IS NULL THEN NULL ELSE format('%s', {c}) END"),
        }
    }
}

/// Las columnas del objeto, con su forma. `objeto` si no existe.
pub(crate) fn columnas(
    c: &mut postgres::Client,
    objeto: &str,
) -> Result<Vec<(String, Forma)>, Fallo> {
    columnas_y_paginas(c, objeto).map(|(cols, _)| cols)
}

/// Las columnas de un objeto, por nombre, con su forma.
type Columnas = Vec<(String, Forma)>;

/// Las columnas y, en el mismo viaje, las páginas del objeto si se puede leer
/// en tramos (`paginas_a_repartir`): una tabla o vista materializada de un
/// primario con Postgres 14 o más.
fn columnas_y_paginas(
    c: &mut postgres::Client,
    objeto: &str,
) -> Result<(Columnas, Option<i64>), Fallo> {
    let citado = citado(objeto);
    let filas = c
        .query_typed(
            "SELECT a.attname::text, format_type(a.atttypid, a.atttypmod), t.typtype::text, \
                    CASE WHEN t.typtype = 'd' THEN format_type(t.typbasetype, t.typtypmod) END, \
                    to_regclass($1) IS NOT NULL, \
                    CASE WHEN current_setting('server_version_num')::int >= 140000 \
                          AND NOT pg_is_in_recovery() \
                          AND (SELECT relkind IN ('r', 'm') FROM pg_class WHERE oid = o.r) \
                         THEN pg_relation_size(o.r) / current_setting('block_size')::int8 END \
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
    let paginas: Option<i64> = filas.first().and_then(|f| f.get(5));
    let cols = filas
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
        .collect();
    Ok((cols, paginas))
}

/// El tipo de Arrow de una forma: el del contrato, que es el de `ore-arrow`. Una geometría llega en EWKB y va como
/// `Binary` sin la marca de GeoArrow (que es de WKB).
fn tipo_arrow(f: &Forma) -> DataType {
    ore_arrow::tipo_arrow(&ore(f))
}

/// La forma de `ore-base` de una columna de Postgres. Sin tipo exacto, texto; una geometría, sus bytes.
fn ore(f: &Forma) -> ore_base::tipos::Forma {
    use ore_base::tipos::Forma as O;
    match f {
        Forma::Escalar(f) => O::Escalar(f.unwrap_or(Fisico::Texto)),
        Forma::Geo => O::Escalar(Fisico::Binario),
        Forma::Lista(f) => O::Lista(Box::new(O::Escalar(*f))),
    }
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
        // `bytea` sale en hexadecimal (`\xDEAD`, el `bytea_output` por defecto); el texto canónico de `Binary` es base64.
        Fisico::Binario => match t.strip_prefix("\\x").and_then(|h| {
            if h.is_empty() {
                Some(Vec::new())
            } else {
                ore_base::firma::hex(h)
            }
        }) {
            Some(b) => (Some(Owned(ore_base::tipos::base64_codificar(&b))), None),
            None => (Some(Borrowed(t)), None),
        },
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

/// Escribe un valor en su texto de Postgres en la columna: lo normaliza, lo analiza y devuelve el aviso, si lo hubo.
/// `Err` con el tipo si no cabe; entonces no escribe nada de esa celda.
fn poner(
    w: &mut ore_arrow::Escritor<'_>,
    forma: &Forma,
    v: Option<&str>,
) -> Result<Option<Aviso>, String> {
    if let Forma::Lista(f) = forma {
        let Some(t) = v else {
            w.nulo();
            return Ok(None);
        };
        let mut aviso = None;
        for e in elementos(t)? {
            let mut h = w.elemento().map_err(|e| e.0)?;
            aviso = poner_escalar(&mut h, Some(f), e.as_deref())?.or(aviso);
        }
        w.cerrar_lista().map_err(|e| e.0)?;
        return Ok(aviso);
    }
    poner_escalar(w, forma.escalar(), v)
}

fn poner_escalar(
    w: &mut ore_arrow::Escritor<'_>,
    f: Option<&Fisico>,
    v: Option<&str>,
) -> Result<Option<Aviso>, String> {
    let Some(t) = v else {
        w.nulo();
        return Ok(None);
    };
    let f = match f {
        None | Some(Fisico::Texto) => return w.texto(t).map(|_| None).map_err(|e| e.0),
        Some(f) => f,
    };
    let (t, aviso) = normalizar(f, t);
    match t {
        None => w.nulo(),
        Some(t) => {
            let valor = f
                .analizar(&t)
                .ok_or_else(|| format!("`{t}` no es un {}", f.arrow()))?;
            w.valor(&valor)
                .map_err(|_| format!("`{t}` no es un {}", f.arrow()))?;
        }
    }
    Ok(aviso)
}

/// La consulta de la petición, ya traducida, y lo que hace falta para leerla.
struct Plan {
    consulta: ore_sql::Consulta,
    /// Por propiedad: su posición en el resultado, su forma y su columna.
    salida: Vec<(String, usize, Forma, String)>,
    esquema: Arc<Schema>,
    /// Las páginas del objeto, si se puede leer en tramos (ORE 0067).
    paginas: Option<i64>,
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
    let (cols, paginas) = columnas_y_paginas(c, &p.objeto)?;
    let consulta = binarios_en_los_parametros(consulta, &cols)?;
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
        paginas,
    })
}

/// v1alpha35 · **Un filtro por un `bytea`** llega con el texto de `Binary` (base64) y Postgres espera el suyo
/// (`ÞADBEEF`): se pasa a hexadecimal antes de escribirlo como literal. Un valor que no es base64 es un defecto
/// de quien pidió (`operador`).
fn binarios_en_los_parametros(
    mut c: ore_sql::Consulta,
    cols: &[(String, Forma)],
) -> Result<ore_sql::Consulta, Fallo> {
    for (i, col) in c.columnas_de_parametros.iter().enumerate() {
        let binaria = cols
            .iter()
            .any(|(n, f)| n == col && matches!(f, Forma::Escalar(Some(Fisico::Binario))));
        if !binaria {
            continue;
        }
        let Some(v) = c.parametros.get_mut(i) else {
            continue;
        };
        let bytes = ore_base::tipos::base64_decodificar(v).ok_or_else(|| {
            Fallo::operador(format!(
                "`{v}` no es base64 (los bytes de un `Binary`), y es lo que se compara con `{col}`"
            ))
        })?;
        *v = format!(
            "\\x{}",
            bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
        );
    }
    Ok(c)
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
        s.tramos.clear();
    }
    let paginas = paginas_a_repartir(&plan, p);
    let mut escritor = arrow_ipc::writer::StreamWriter::try_new(&mut *salida, &plan.esquema)
        .map_err(|e| Fallo::origen(format!("no se pudo empezar el flujo: {e}")))?;
    let mut escribir = |lote: &RecordBatch| -> Result<(), Fallo> {
        escritor
            .write(lote)
            .and_then(|_| escritor.flush())
            .map_err(|e| Fallo::new(Codigo::Conexion, format!("no se pudo escribir: {e}")))
    };
    let mut avisos: BTreeMap<(String, Aviso), u64> = BTreeMap::new();
    let n = match paginas {
        None => {
            // Abrir la transacción y fijar la sesión, en un solo viaje.
            let abrir = format!("START TRANSACTION READ ONLY; {}", ajustes(p.timeout_ms));
            en_transaccion(c, &abrir, |c, _| {
                let mut filas = c
                    .query_typed_raw(&sql_de(&plan, &plan.consulta), sin_parametros())
                    .map_err(|e| fallo_de(&e))?;
                por_lotes(&mut filas, &plan, p, token, inicio, &mut avisos, |l| {
                    escribir(&l)
                })
            })?
        }
        Some(paginas) => en_tramos(c, &plan, p, token, inicio, paginas, &mut avisos, escribir)?,
    };
    escritor
        .finish()
        .map_err(|e| Fallo::new(Codigo::Conexion, format!("no se pudo cerrar el flujo: {e}")))?;
    for ((columna, aviso), k) in &avisos {
        eprintln!("ore-read-postgres: aviso · `{columna}`: {k} {aviso}");
    }
    if let Ok(mut s) = token.lock() {
        s.token = None;
        s.tramos.clear();
    }
    Ok(n)
}

/// La consulta que se manda: sólo lo que sale, en el orden de `salida`, la
/// geometría en binario y lo demás por su texto (`Forma::en_la_consulta`).
fn sql_de(plan: &Plan, consulta: &ore_sql::Consulta) -> String {
    format!(
        "SELECT {} FROM ({}) AS ore_q",
        plan.salida
            .iter()
            .map(|(_, _, f, col)| f.en_la_consulta(col))
            .collect::<Vec<_>>()
            .join(", "),
        en_linea(consulta)
    )
}

/// `abrir` (que empieza una transacción; sus filas, a `hacer`), `hacer` y
/// `COMMIT`. Si algo falla, `ROLLBACK`: la sesión queda como estaba y puede
/// volver a usarse (`servir`).
fn en_transaccion<T>(
    c: &mut postgres::Client,
    abrir: &str,
    hacer: impl FnOnce(&mut postgres::Client, Vec<postgres::SimpleQueryRow>) -> Result<T, Fallo>,
) -> Result<T, Fallo> {
    let hecho = c
        .simple_query(abrir)
        .map_err(|e| fallo_de(&e))
        .and_then(|m| hacer(c, filas_de(m)));
    match hecho {
        Ok(t) => {
            c.batch_execute("COMMIT").map_err(|e| fallo_de(&e))?;
            Ok(t)
        }
        Err(f) => {
            let _ = c.batch_execute("ROLLBACK");
            Err(f)
        }
    }
}

fn sin_parametros() -> std::iter::Empty<(
    &'static (dyn postgres::types::ToSql + Sync),
    postgres::types::Type,
)> {
    std::iter::empty()
}

/// **Las filas, de [`LOTE`] en [`LOTE`]**, en Arrow, a `dar`. Entre lote y
/// lote mira el reloj de la petición y la marca de cancelar. Devuelve las
/// filas.
fn por_lotes(
    filas: &mut postgres::RowIter<'_>,
    plan: &Plan,
    p: &Peticion,
    token: &Token,
    inicio: Instant,
    avisos: &mut BTreeMap<(String, Aviso), u64>,
    mut dar: impl FnMut(RecordBatch) -> Result<(), Fallo>,
) -> Result<u64, Fallo> {
    use postgres::fallible_iterator::FallibleIterator;
    let mut n: u64 = 0;
    let columnas: Vec<Columna> = plan
        .salida
        .iter()
        .map(|(prop, _, f, _)| Columna::de_forma(prop, ore(f), ""))
        .collect();
    let mut lote = RecordBatchBuilder::con_capacidad(&columnas, LOTE)
        .map_err(|e| Fallo::origen(e.to_string()))?;
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
        let mut k: usize = 0;
        let mut fin = false;
        while k < LOTE {
            let Some(fila) = filas.next().map_err(|e| fallo_de(&e))? else {
                fin = true;
                break;
            };
            for (i, (prop, _, f, columna)) in plan.salida.iter().enumerate() {
                let v = fila
                    .try_get::<_, Option<Crudo>>(i)
                    .map_err(|e| {
                        Fallo::origen(format!(
                            "la columna `{columna}` no se pudo leer: {}",
                            texto::causa(&e)
                        ))
                    })?
                    .map(|c| c.0);
                let mut w = lote.col(i);
                let puesto = match (f, v) {
                    (Forma::Geo, Some(v)) => w.bytes(v).map(|_| None).map_err(|e| e.0),
                    (f, v) => match v.map(std::str::from_utf8).transpose() {
                        Ok(t) => poner(&mut w, f, t),
                        Err(e) => Err(format!("no es texto: {e}")),
                    },
                };
                if let Some(a) = puesto.map_err(|e| {
                    Fallo::origen(format!(
                        "`{prop}` (la columna `{columna}`), fila {}: {e}",
                        n + k as u64 + 1
                    ))
                })? {
                    *avisos.entry((columna.clone(), a)).or_default() += 1;
                }
            }
            lote.fila_escrita();
            k += 1;
        }
        if k > 0 {
            let arrays = lote
                .finish()
                .map_err(|e| Fallo::origen(e.to_string()))?
                .columns()
                .to_vec();
            dar(RecordBatch::try_new(plan.esquema.clone(), arrays)
                .map_err(|e| Fallo::origen(format!("el lote no casa: {e}")))?)?;
            n += k as u64;
        }
        if fin {
            return Ok(n);
        }
    }
}

/// Cuántas conexiones leen a la vez una tabla grande (ORE 0067).
const TRAMOS: i64 = 4;

/// Desde cuántas páginas de 8 KiB (48 MiB) se reparte: por debajo, abrir las
/// conexiones cuesta más viajes de los que ahorra. Medido a 100 ms del origen:
/// 18 MiB (214 000 puntos) van a 90 000 filas/s con una y a 76 000 con cuatro;
/// 57 MiB, a 85 000 y 94 000; 179 MiB (2,1 M), a 283 000 y 496 000.
const PAGINAS_PARA_REPARTIR: i64 = 6144;

/// **¿Se lee en tramos?** ORE 0067: a 100 ms del origen, una conexión no pasa
/// de unas 90 000 filas/s, por los viajes y por la ventana de TCP; varias, cada
/// una con su trozo de la tabla por `ctid`, se suman. Sólo cuando el orden no
/// importa y se lee entera (sin `limit`, sin orden, sin claves: una copia), de
/// una tabla o vista materializada con [`PAGINAS_PARA_REPARTIR`] páginas o
/// más, en un primario (exportar la instantánea no se puede en una réplica)
/// de Postgres 14 o más (antes no hay *TID Range Scan*, y cada tramo
/// recorrería la tabla entera): eso lo pregunta `columnas_y_paginas`, en el
/// mismo viaje. Devuelve las páginas.
fn paginas_a_repartir(plan: &Plan, p: &Peticion) -> Option<i64> {
    if p.limit.is_some() || !p.orden.is_empty() || !p.claves.is_empty() {
        return None;
    }
    plan.paginas.filter(|n| *n >= PAGINAS_PARA_REPARTIR)
}

/// **Leer en tramos** desde una misma instantánea: la conexión de la
/// petición abre una transacción `REPEATABLE READ` y la exporta; cada tramo
/// abre la suya con esa instantánea (`SET TRANSACTION SNAPSHOT`) y lee su
/// rango de páginas (`ctid`), el último sin cota de arriba. Lo leído es lo de
/// un instante, como con una conexión. Los lotes llegan por un canal acotado y
/// se escriben en el orden en que llegan. Si un tramo falla, se cancelan los
/// demás y el error es el suyo.
#[allow(clippy::too_many_arguments)]
fn en_tramos(
    c: &mut postgres::Client,
    plan: &Plan,
    p: &Peticion,
    token: &Token,
    inicio: Instant,
    paginas: i64,
    avisos: &mut BTreeMap<(String, Aviso), u64>,
    escribir: impl FnMut(&RecordBatch) -> Result<(), Fallo>,
) -> Result<u64, Fallo> {
    // Abrirla y exportar su instantánea, en un viaje.
    en_transaccion(
        c,
        "START TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY; \
         SELECT pg_export_snapshot()",
        |_, filas| {
            let instantanea = filas
                .first()
                .and_then(|f| f.get(0))
                .ok_or_else(|| Fallo::origen("pg_export_snapshot no devolvió nada"))?
                .to_string();
            tramos_desde(
                &instantanea,
                plan,
                p,
                token,
                inicio,
                paginas,
                avisos,
                escribir,
            )
        },
    )
}

/// Los tramos, cada uno en su conexión con la instantánea `instantanea`.
#[allow(clippy::too_many_arguments)]
fn tramos_desde(
    instantanea: &str,
    plan: &Plan,
    p: &Peticion,
    token: &Token,
    inicio: Instant,
    paginas: i64,
    avisos: &mut BTreeMap<(String, Aviso), u64>,
    mut escribir: impl FnMut(&RecordBatch) -> Result<(), Fallo>,
) -> Result<u64, Fallo> {
    let paso = (paginas + TRAMOS - 1) / TRAMOS;
    let tramos: Vec<String> = (0..TRAMOS)
        .map(|i| {
            let mut q = p.clone();
            let cota = |op: &str, pagina: i64| ore_driver::Filtro {
                columna: "ctid".into(),
                operador: op.into(),
                valor: ore_driver::Valor::Uno(format!("({pagina},0)")),
            };
            if i > 0 {
                q.filtros.push(cota("ge", i * paso));
            }
            if i < TRAMOS - 1 {
                q.filtros.push(cota("lt", (i + 1) * paso));
            }
            ore_sql::preparar(&q, &ore_sql::dialectos::POSTGRES, &q.objeto, || {
                unreachable!("`POSTGRES` no exige tipos")
            })
            .map(|consulta| sql_de(plan, &consulta))
            .map_err(Fallo::operador)
        })
        .collect::<Result<_, _>>()?;
    let abrir = format!(
        "START TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY; \
         SET TRANSACTION SNAPSHOT '{instantanea}'; {}",
        ajustes(p.timeout_ms)
    );
    let parar = || {
        if let Ok(mut s) = token.lock() {
            s.cancelada = true;
            for (t, tls) in &s.tramos {
                let _ = t.cancel_query(tls.clone());
            }
        }
    };
    let (dar, lotes) =
        std::sync::mpsc::sync_channel::<Result<RecordBatch, Fallo>>(2 * TRAMOS as usize);
    std::thread::scope(|s| {
        let hilos: Vec<_> = tramos
            .iter()
            .map(|sql| {
                let dar = dar.clone();
                let abrir = &abrir;
                s.spawn(move || {
                    let mut suyos = BTreeMap::new();
                    let leido = (|| {
                        let mut c = conectar(&p.url)?;
                        if let (Ok(mut s), Some(tls)) =
                            (token.lock(), crate::tls::conector_de(&p.url))
                        {
                            s.tramos.push((c.cancel_token(), tls));
                        }
                        en_transaccion(&mut c, abrir, |c, _| {
                            let mut filas = c
                                .query_typed_raw(sql, sin_parametros())
                                .map_err(|e| fallo_de(&e))?;
                            por_lotes(&mut filas, plan, p, token, inicio, &mut suyos, |l| {
                                dar.send(Ok(l))
                                    .map_err(|_| Fallo::origen("la lectura ya terminó"))
                            })
                        })
                    })();
                    if let Err(f) = leido {
                        let _ = dar.send(Err(f));
                    }
                    suyos
                })
            })
            .collect();
        drop(dar);
        let mut n: u64 = 0;
        let mut fallo: Option<Fallo> = None;
        for lote in lotes {
            match lote.and_then(|l| {
                escribir(&l)?;
                Ok(l.num_rows() as u64)
            }) {
                Ok(k) => n += k,
                Err(f) => {
                    // El primero es la causa; los de después, los tramos
                    // que se cancelaron por él.
                    if fallo.is_none() {
                        fallo = Some(f);
                        parar();
                    }
                }
            }
        }
        for h in hilos {
            for (clave, k) in h.join().unwrap_or_default() {
                *avisos.entry(clave).or_default() += k;
            }
        }
        match fallo {
            Some(f) => Err(f),
            None => Ok(n),
        }
    })
}

/// **El valor tal como llega**, en el formato binario del protocolo: para la
/// geometría, su EWKB; para lo demás, que la consulta pide como texto, sus
/// bytes en UTF-8.
struct Crudo<'a>(&'a [u8]);

impl<'a> postgres::types::FromSql<'a> for Crudo<'a> {
    fn from_sql(
        _: &postgres::types::Type,
        raw: &'a [u8],
    ) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(Crudo(raw))
    }

    fn accepts(_: &postgres::types::Type) -> bool {
        true
    }
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
    let raiz = ore_base::parse::parse(&json)
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
mod dorado;

#[cfg(test)]
mod tests {
    use super::dorado::Una;
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
        let mut c = Una::de(&Forma::Escalar(Some(f)));
        assert!(c.poner(Some("12.50")).is_ok());
        assert!(c.poner(Some("doce")).is_err());
        assert!(c.poner(None).is_ok());
        // `NaN` no tiene sitio en un decimal: nulo, y se dice (D-PG4).
        assert_eq!(c.poner(Some("NaN")).unwrap(), Some(AVISO_DECIMAL));
        let a = c.fin();
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
            let mut c = Una::de(&Forma::Escalar(Some(f)));
            c.poner(v.as_deref())
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
        let mut c = Una::de(&f);
        c.poner(Some("{1,2,NULL}")).unwrap();
        c.poner(None).unwrap();
        c.poner(Some("{}")).unwrap();
        let a = c.fin();
        assert_eq!(a.data_type(), &tipo_arrow(&f));
        assert_eq!((a.len(), a.null_count()), (3, 1));
        let l = a.as_any().downcast_ref::<arrow_array::ListArray>().unwrap();
        assert_eq!(l.value(0).len(), 3);
        assert_eq!(l.value(2).len(), 0);
        assert!(c.poner(Some("{x}")).is_err());
    }

    /// v1alpha35 · Un `bytea` es `Binary`; su filtro (base64) se pasa al literal hexadecimal de Postgres, y un
    /// `bytea[]` sigue siendo texto.
    #[test]
    fn un_bytea_es_binary_y_su_filtro_va_en_hexadecimal() {
        assert_eq!(
            Forma::de("bytea", "b", None),
            Forma::Escalar(Some(Fisico::Binario))
        );
        assert_eq!(Forma::de("bytea[]", "b", None), Forma::Escalar(None));
        let cols = vec![
            ("id".to_string(), Forma::Escalar(Some(Fisico::Entero))),
            ("guid".to_string(), Forma::Escalar(Some(Fisico::Binario))),
        ];
        let c = |v: &str, col: &str| ore_sql::Consulta {
            texto: "SELECT 1".into(),
            parametros: vec![v.into()],
            columnas: vec![],
            columnas_de_parametros: vec![col.into()],
        };
        let ok = binarios_en_los_parametros(c("3q2+7w==", "guid"), &cols).unwrap();
        assert_eq!(ok.parametros, vec!["\\xdeadbeef"]);
        let vacio = binarios_en_los_parametros(c("", "guid"), &cols).unwrap();
        assert_eq!(vacio.parametros, vec!["\\x"]);
        // Lo que no es un binario no se toca.
        let otro = binarios_en_los_parametros(c("7", "id"), &cols).unwrap();
        assert_eq!(otro.parametros, vec!["7"]);
        let e = binarios_en_los_parametros(c("no es base64!", "guid"), &cols).unwrap_err();
        assert_eq!(e.codigo, Codigo::Operador);
    }

    #[test]
    fn los_parametros_van_como_literales() {
        let c = ore_sql::Consulta {
            texto: r#"SELECT "a$1" FROM "t" WHERE "id" IN ($1, $2) AND "x" = '$1' AND "y" = $10"#
                .into(),
            parametros: vec!["7".into(), "o'brien".into()],
            columnas: vec![],
            columnas_de_parametros: vec![],
        };
        assert_eq!(
            en_linea(&c),
            r#"SELECT "a$1" FROM "t" WHERE "id" IN ('7', 'o''brien') AND "x" = '$1' AND "y" = $10"#
        );
    }
}
