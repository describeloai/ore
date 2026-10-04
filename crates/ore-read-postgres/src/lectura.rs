//! **El conector v2** (ADR 0053 F2·2, `docs/federation.md` §1): leer en Arrow y
//! en flujo, con tiempo, cancelación, sesiones reutilizadas y errores tipados.
//!
//! # Cinco decisiones
//!
//! 1. **Por un portal, de [`LOTE`] en [`LOTE`] filas.** `client.query` juntaba
//!    el resultado entero antes de escribir una fila: el kit midió 388 MiB y el
//!    primer byte a los 32,9 s para 10⁶ filas (F2·1). Un portal dentro de una
//!    transacción de sólo lectura trae cada vez un lote, que sale por stdout
//!    antes de pedir el siguiente: la memoria es la de un lote.
//! 2. **El tipo de cada columna, del catálogo**, con la misma traducción que
//!    `catalogo` (`traducir`): es la que escribió la `Table` del árbol, así que
//!    el tipo Arrow es el de la tabla. Sólo lo exacto se tipa —enteros,
//!    reales, lógicos, `numeric(p, s)`, fechas, instantes, texto—; lo que no
//!    (`numeric` sin precisión, lo opaco, las listas) sale como texto y el
//!    almacén lo estrecha como hasta ahora. Un valor que no cabe en su tipo es
//!    un error con su columna y su fila, nunca un nulo.
//! 3. **El valor, del cable a su texto canónico y de ahí a su tipo**: `texto.rs`
//!    decodifica y `ore_core::tipos` analiza, el mismo analizador con el que el
//!    almacén estrecha. No hay una segunda conversión que pueda divergir.
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
pub type Token = Arc<Mutex<Option<postgres::CancelToken>>>;

fn tls() -> Result<postgres_native_tls::MakeTlsConnector, Fallo> {
    native_tls::TlsConnector::new()
        .map(postgres_native_tls::MakeTlsConnector::new)
        .map_err(|e| Fallo::origen(format!("no se pudo preparar TLS: {e}")))
}

/// Cancela en el origen lo que corre en la sesión de `token`, si algo corre.
pub fn cancelar(token: &Token) {
    let t = token.lock().ok().and_then(|t| t.clone());
    if let (Some(t), Ok(tls)) = (t, tls()) {
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

/// **Abre una sesión**, de sólo lectura, con un plazo para conectar y su
/// nombre de aplicación (para que quien mire `pg_stat_activity` la vea).
pub fn conectar(url: &str) -> Result<postgres::Client, Fallo> {
    let mut config: postgres::Config = url
        .parse()
        .map_err(|e| Fallo::new(Codigo::Conexion, format!("la URL no es de Postgres: {e}")))?;
    config.connect_timeout(Duration::from_secs(10));
    config.application_name("ore-read-postgres");
    let mut c = config.connect(tls()?).map_err(|e| fallo_de(&e))?;
    c.batch_execute("SET SESSION CHARACTERISTICS AS TRANSACTION READ ONLY")
        .map_err(|e| fallo_de(&e))?;
    Ok(c)
}

/// `"esquema"."tabla"`, citado parte a parte: lo que `to_regclass` lee sin que
/// un nombre pueda decir otra cosa.
fn citado(objeto: &str) -> String {
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
    let exacto = matches!(
        oos.as_str(),
        "Integer" | "Float" | "Boolean" | "Date" | "Time" | "DateTime" | "DateTimeTz" | "String"
    ) || oos.starts_with("Decimal<");
    if !exacto {
        return None;
    }
    let t = ore_core::types::parse_type(&oos).ok()?;
    Some(Fisico::de(&t))
}

/// Las columnas del objeto, con su físico. `objeto` si no existe.
fn columnas(
    c: &mut postgres::Client,
    objeto: &str,
) -> Result<Vec<(String, Option<Fisico>)>, Fallo> {
    let filas = c
        .query(
            "SELECT a.attname::text, format_type(a.atttypid, a.atttypmod), t.typtype::text, \
                    CASE WHEN t.typtype = 'd' THEN format_type(t.typbasetype, t.typtypmod) END, \
                    to_regclass($1) IS NOT NULL \
             FROM (SELECT to_regclass($1) AS r) o \
             LEFT JOIN pg_attribute a ON a.attrelid = o.r AND a.attnum > 0 AND NOT a.attisdropped \
             LEFT JOIN pg_type t ON t.oid = a.atttypid \
             ORDER BY a.attnum",
            &[&citado(objeto)],
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
                fisico(&formato?, &familia.unwrap_or_default(), base.as_deref()),
            ))
        })
        .collect())
}

fn tipo_arrow(f: Option<&Fisico>) -> DataType {
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
}

impl Col {
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

    /// Añade un valor en su texto canónico; `Err` con el tipo si no cabe.
    fn poner(&mut self, f: Option<&Fisico>, v: Option<&str>) -> Result<(), String> {
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
        })
    }
}

/// La consulta de la petición, ya traducida, y lo que hace falta para leerla.
struct Plan {
    consulta: ore_sql::Consulta,
    /// Por propiedad: su posición en el resultado y su físico.
    salida: Vec<(String, usize, Option<Fisico>)>,
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
            .ok_or_else(|| {
                Fallo::new(
                    Codigo::Objeto,
                    format!("`{}` no tiene la columna `{col}`", p.objeto),
                )
            })?
            .1;
        let i = consulta
            .columnas
            .iter()
            .position(|x| x == col)
            .ok_or_else(|| Fallo::origen(format!("la consulta no pide `{col}`")))?;
        salida.push((prop.clone(), i, f));
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
            return Err(Fallo::new(
                Codigo::Objeto,
                format!("`{}` no tiene la columna `{col}`", p.objeto),
            ));
        }
    }
    let esquema = Arc::new(Schema::new(
        salida
            .iter()
            .map(|(prop, _, f)| Field::new(prop, tipo_arrow(f.as_ref()), true))
            .collect::<Vec<_>>(),
    ));
    Ok(Plan {
        consulta,
        salida,
        esquema,
    })
}

fn parametros(c: &ore_sql::Consulta) -> Vec<texto::Parametro> {
    c.parametros
        .iter()
        .map(|v| texto::Parametro(v.clone()))
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
    if let Ok(mut t) = token.lock() {
        *t = Some(c.cancel_token());
    }
    let ps = parametros(&plan.consulta);
    let refs: Vec<&(dyn postgres::types::ToSql + Sync)> = ps
        .iter()
        .map(|v| v as &(dyn postgres::types::ToSql + Sync))
        .collect();
    let mut tx = c
        .build_transaction()
        .read_only(true)
        .start()
        .map_err(|e| fallo_de(&e))?;
    if let Some(ms) = p.timeout_ms {
        tx.batch_execute(&format!("SET LOCAL statement_timeout = {ms}"))
            .map_err(|e| fallo_de(&e))?;
    }
    let portal = tx
        .bind(plan.consulta.texto.as_str(), &refs)
        .map_err(|e| fallo_de(&e))?;
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
        let filas = tx
            .query_portal(&portal, LOTE as i32)
            .map_err(|e| fallo_de(&e))?;
        if filas.is_empty() {
            break;
        }
        let mut cols: Vec<Col> = plan
            .salida
            .iter()
            .map(|(_, _, f)| Col::nueva(f.as_ref()))
            .collect();
        for (k, fila) in filas.iter().enumerate() {
            for ((prop, i, f), col) in plan.salida.iter().zip(cols.iter_mut()) {
                let v = fila.try_get::<_, Option<texto::Texto>>(*i).map_err(|e| {
                    Fallo::origen(format!(
                        "la columna de `{prop}` no se pudo leer: {}",
                        texto::causa(&e)
                    ))
                })?;
                col.poner(f.as_ref(), v.as_ref().map(|t| t.0.as_str()))
                    .map_err(|e| {
                        Fallo::origen(format!("`{prop}`, fila {}: {e}", n + k as u64 + 1))
                    })?;
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
    drop(portal);
    tx.commit().map_err(|e| fallo_de(&e))?;
    if let Ok(mut t) = token.lock() {
        *t = None;
    }
    Ok(n)
}

/// **Estimar**: lo que el planificador de Postgres cree que devolvería la
/// petición, sin leerla (`EXPLAIN`, sin `ANALYZE`).
pub fn estimar(c: &mut postgres::Client, p: &Peticion) -> Result<String, Fallo> {
    let plan = planear(c, p)?;
    let ps = parametros(&plan.consulta);
    let refs: Vec<&(dyn postgres::types::ToSql + Sync)> = ps
        .iter()
        .map(|v| v as &(dyn postgres::types::ToSql + Sync))
        .collect();
    let filas = c
        .query(
            format!("EXPLAIN (FORMAT JSON) {}", plan.consulta.texto).as_str(),
            &refs,
        )
        .map_err(|e| fallo_de(&e))?;
    let json: Option<texto::Texto> = filas
        .first()
        .ok_or_else(|| Fallo::origen("EXPLAIN no devolvió nada"))?
        .try_get(0)
        .map_err(|e| Fallo::origen(texto::causa(&e)))?;
    let json = json.map(|t| t.0).unwrap_or_default();
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
        assert!(c.poner(Some(&f), Some("NaN")).is_err());
        assert!(c.poner(Some(&f), None).is_ok());
        let a = c.fin().unwrap();
        assert_eq!((a.len(), a.null_count()), (2, 1));
    }
}
