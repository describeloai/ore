//! **El conector v2** (ADR 0053 F2·3, `docs/federation.md` §1): siempre
//! Arrow, los diez operadores, `limit`, `orderBy`, tiempo, cancelar, `estimar`
//! sin facturar y errores tipados.
//!
//! # Cuatro decisiones
//!
//! 1. **El tipo de cada columna, de `tables.get`** —con la precisión de sus
//!    `NUMERIC(p, s)`— y la misma traducción que `catalogo`: el tipo Arrow es
//!    el de la `Table` del árbol. Sólo lo exacto se tipa; un `BIGNUMERIC`, un
//!    `BYTES`, un `JSON`, lo repetido o lo anidado salen como texto, como
//!    siempre, y el almacén decide.
//! 2. **Dos caminos, una salida.** La Storage Read cuando se puede (tabla,
//!    tipos que trae con su valor, sin `orderBy`): rápida, y `limit` corta los
//!    streams al llegar a n. Si no, la consulta por REST, ahora también en
//!    Arrow: cada página, del texto canónico de `valores` a su tipo.
//! 3. **El tiempo, en BigQuery**: `jobTimeoutMs` en el job y el reloj de quien
//!    pide entre sondeo y página; agotado, `jobs.cancel`.
//! 4. **`estimar` es un *dry run***: BigQuery dice los bytes que procesaría y
//!    no factura nada.

use crate::{consultas, flecha, rest, valores};
use arrow_array::builder::{
    BooleanBuilder, Date32Builder, Decimal128Builder, Float64Builder, Int64Builder, StringBuilder,
    Time64MicrosecondBuilder, TimestampMicrosecondBuilder,
};
use arrow_array::{ArrayRef, RecordBatch};
use arrow_schema::{DataType, Field, Schema, SchemaRef, TimeUnit};
use ore_core::json::Json;
use ore_core::tipos::{Fisico, Valor};
use ore_driver::capacidades::Capacidades;
use ore_driver::{Codigo, Fallo, Peticion};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const CAPACIDADES: Capacidades = Capacidades {
    conector: "ore-read-bigquery",
    version: env!("CARGO_PKG_VERSION"),
    operadores: ore_driver::OPERADORES,
    limit: true,
    order_by: true,
    estimar: true,
    servir: true,
};

/// La zona de un instante, como la escribe el lago (`ore-store` `carga::UTC`).
const UTC: &str = "+00:00";

/// **Un error de BigQuery, con su código del contrato.** Los de dentro son
/// texto (`BigQuery contestó 404: …`); se clasifican por lo que dicen.
pub fn fallo(m: String) -> Fallo {
    let tiene = |xs: &[&str]| xs.iter().any(|x| m.contains(x));
    let (codigo, reintentable) =
        if m.starts_with(rest::AGOTADO) || tiene(&["Job timed out", "jobTimeoutMs", "Timeout"]) {
            (Codigo::Tiempo, true)
        } else if tiene(&[
            "contestó 401",
            "contestó 403",
            "token",
            "Permission",
            "Access Denied",
        ]) {
            (Codigo::Credencial, false)
        } else if tiene(&[
            "contestó 404",
            "Not found",
            "Unrecognized name",
            "no tiene la columna",
        ]) {
            (Codigo::Objeto, false)
        } else if tiene(&["no se pudo hablar con BigQuery"]) {
            (Codigo::Conexion, true)
        } else if tiene(&[
            "contestó 429",
            "contestó 500",
            "contestó 503",
            "rateLimitExceeded",
        ]) {
            (Codigo::Origen, true)
        } else {
            (Codigo::Origen, false)
        };
    Fallo::new(codigo, m).reintentable(reintentable)
}

/// **El físico de una columna de BigQuery**, de su campo de `tables.get`;
/// `None` si sale como texto.
pub fn fisico(campo: &Value) -> Option<Fisico> {
    let t = campo["type"].as_str()?;
    if campo["mode"] == "REPEATED" || matches!(t, "RECORD" | "STRUCT") {
        return None;
    }
    let bq = match (t, campo["precision"].as_str()) {
        ("NUMERIC" | "BIGNUMERIC", Some(p)) => {
            format!("{t}({p}, {})", campo["scale"].as_str().unwrap_or("0"))
        }
        _ => t.to_string(),
    };
    let oos = crate::catalogo::traducir(&bq)?;
    let exacto = matches!(
        oos.as_str(),
        "Integer" | "Float" | "Boolean" | "Date" | "Time" | "DateTime" | "DateTimeTz" | "String"
    ) || oos.starts_with("Decimal<");
    if !exacto {
        return None;
    }
    Some(Fisico::de(&ore_core::types::parse_type(&oos).ok()?))
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

    /// Añade un valor en su texto canónico; `Err` si no cabe en su tipo.
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

/// Lo que una petición necesita saber de su tabla.
struct Plan {
    proyecto: String,
    info: Value,
    /// Por propiedad: su columna y su físico.
    salida: Vec<(String, String, Option<Fisico>)>,
    esquema: SchemaRef,
    /// Columna → tipo de GoogleSQL, para los parámetros.
    tipos: BTreeMap<String, String>,
}

fn planear(t: &dyn rest::Transporte, p: &Peticion) -> Result<Plan, Fallo> {
    if let Some(porque) = ore_driver::rango_servible(p, true, false) {
        return Err(Fallo::operador(porque));
    }
    CAPACIDADES.admite(p)?;
    let proyecto = crate::proyecto(&p.url).map_err(Fallo::operador)?;
    let (dataset, tabla) =
        consultas::partes(&p.objeto).map_err(|e| Fallo::new(Codigo::Objeto, e))?;
    let info = rest::tabla(t, &proyecto, dataset, tabla).map_err(fallo)?;
    let campos: BTreeMap<String, Value> = info["schema"]["fields"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|c| Some((c["name"].as_str()?.to_string(), c.clone())))
                .collect()
        })
        .unwrap_or_default();
    let falta = |col: &str| {
        Fallo::new(
            Codigo::Objeto,
            format!("`{}` no tiene la columna `{col}`", p.objeto),
        )
    };
    let mut salida = Vec::new();
    for (prop, col) in &p.proyeccion {
        let campo = campos.get(col).ok_or_else(|| falta(col))?;
        salida.push((prop.clone(), col.clone(), fisico(campo)));
    }
    for col in p
        .filtros
        .iter()
        .map(|f| &f.columna)
        .chain(p.orden.iter().map(|o| &o.columna))
    {
        if !campos.contains_key(col) {
            return Err(falta(col));
        }
    }
    let esquema = Arc::new(Schema::new(
        salida
            .iter()
            .map(|(prop, _, f)| Field::new(prop, tipo_arrow(f.as_ref()), true))
            .collect::<Vec<_>>(),
    ));
    let tipos = campos
        .iter()
        .filter_map(|(n, c)| {
            Some((
                n.clone(),
                valores::estandar(c["type"].as_str()?).to_string(),
            ))
        })
        .collect();
    Ok(Plan {
        proyecto,
        info,
        salida,
        esquema,
        tipos,
    })
}

fn consulta(p: &Peticion, plan: &Plan) -> Result<ore_sql::Consulta, Fallo> {
    ore_sql::consulta(
        p,
        &ore_sql::dialectos::BIGQUERY,
        &consultas::cualificado(&plan.proyecto, &p.objeto),
        &plan.tipos,
    )
    .map_err(Fallo::operador)
}

/// **Leer**: la petición, siempre en Arrow, por la Storage Read o por REST.
pub fn leer(t: &dyn rest::Transporte, p: &Peticion, salida: &mut dyn Write) -> Result<u64, Fallo> {
    let inicio = Instant::now();
    let plan = planear(t, p)?;
    let hasta = p.timeout_ms.map(|ms| inicio + Duration::from_millis(ms));
    let freno = flecha::Freno {
        limite: p.limit,
        hasta,
        ms: p.timeout_ms,
    };
    match flecha::leer(
        t,
        p,
        &plan.proyecto,
        &plan.info,
        &plan.esquema,
        freno,
        &mut *salida,
    )
    .map_err(fallo)?
    {
        flecha::Lectura::Servida(n) => {
            eprintln!("ore-read-bigquery: {n} filas en Arrow (Storage Read)");
            return Ok(n);
        }
        flecha::Lectura::Declina(porque) => {
            eprintln!("ore-read-bigquery: aviso · por consulta: {porque}");
        }
    }
    let c = consulta(p, &plan)?;
    let mut escritor = arrow_ipc::writer::StreamWriter::try_new(&mut *salida, &plan.esquema)
        .map_err(|e| Fallo::origen(format!("no se pudo empezar el flujo: {e}")))?;
    let mut n: u64 = 0;
    rest::consultar_con(
        t,
        &plan.proyecto,
        &rest::Consulta {
            texto: &c.texto,
            parametros: &c.parametros,
            sin_job: false,
        },
        &rest::Opciones {
            tiempo_ms: p.timeout_ms,
            hasta,
        },
        |campos, pagina| {
            // Por nombre y no por posición: la proyección se de-duplica.
            let indice: BTreeMap<&str, usize> = campos
                .iter()
                .enumerate()
                .filter_map(|(i, c)| c["name"].as_str().map(|n| (n, i)))
                .collect();
            let mut cols: Vec<Col> = plan
                .salida
                .iter()
                .map(|(_, _, f)| Col::nueva(f.as_ref()))
                .collect();
            for (k, fila) in pagina.iter().enumerate() {
                let celdas = fila["f"].as_array().map(Vec::as_slice).unwrap_or(&[]);
                for ((prop, col, f), c) in plan.salida.iter().zip(cols.iter_mut()) {
                    let i = *indice
                        .get(col.as_str())
                        .ok_or_else(|| format!("BigQuery no devolvió la columna `{col}`"))?;
                    let v = valores::texto(&campos[i], &celdas[i]["v"])?;
                    c.poner(f.as_ref(), v.as_deref())
                        .map_err(|e| format!("`{prop}`, fila {}: {e}", n + k as u64 + 1))?;
                }
            }
            if pagina.is_empty() {
                return Ok(());
            }
            let arrays = cols
                .iter_mut()
                .map(Col::fin)
                .collect::<Result<Vec<_>, _>>()?;
            let lote = RecordBatch::try_new(plan.esquema.clone(), arrays)
                .map_err(|e| format!("el lote no casa: {e}"))?;
            escritor
                .write(&lote)
                .and_then(|_| escritor.flush())
                .map_err(|e| format!("no se pudo escribir el flujo: {e}"))?;
            n += pagina.len() as u64;
            Ok(())
        },
    )
    .map_err(fallo)?;
    escritor
        .finish()
        .map_err(|e| Fallo::new(Codigo::Conexion, format!("no se pudo cerrar el flujo: {e}")))?;
    eprintln!("ore-read-bigquery: {n} filas en Arrow (por consulta)");
    Ok(n)
}

/// **Estimar**: un *dry run* de la consulta que leería, sin facturar.
pub fn estimar(t: &dyn rest::Transporte, p: &Peticion) -> Result<String, Fallo> {
    let plan = planear(t, p)?;
    let c = consulta(p, &plan)?;
    let bytes = rest::estimar(
        t,
        &plan.proyecto,
        &rest::Consulta {
            texto: &c.texto,
            parametros: &c.parametros,
            sin_job: false,
        },
    )
    .map_err(fallo)?;
    Ok(Json::obj([
        ("bytes", Json::Int(bytes as i64)),
        ("fuente", Json::s("dryRun")),
    ])
    .jcs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rest::pruebas::Guion;
    use serde_json::json;

    fn tabla() -> Value {
        json!({
            "type": "TABLE",
            "schema": {"fields": [
                {"name": "id", "type": "INTEGER", "mode": "REQUIRED"},
                {"name": "importe", "type": "NUMERIC", "precision": "12", "scale": "2"},
                {"name": "cuando", "type": "TIMESTAMP"},
                {"name": "nota", "type": "STRING"},
                {"name": "datos", "type": "RECORD", "fields": [{"name": "a", "type": "STRING"}]}
            ]}
        })
    }

    fn peticion(extra: &str) -> Peticion {
        ore_driver::leer_peticion(&format!(
            r#"{{"url":"bigquery://p/d","objeto":"d.t","formato":"arrow",
                "proyeccion":{{"clave":"id","importe":"importe","cuando":"cuando","datos":"datos"}}{extra}}}"#
        ))
        .unwrap()
    }

    #[test]
    fn el_tipo_sale_de_la_tabla_con_su_precision() {
        let t = tabla();
        let f = |i: usize| fisico(&t["schema"]["fields"][i]);
        assert_eq!(f(0), Some(Fisico::Entero));
        assert_eq!(
            f(1),
            Some(Fisico::Decimal {
                precision: 12,
                escala: 2
            })
        );
        assert_eq!(f(2), Some(Fisico::Instante));
        assert_eq!(f(4), None, "lo anidado sale como texto");
        assert_eq!(
            fisico(&json!({"name": "x", "type": "NUMERIC"})),
            Some(Fisico::Decimal {
                precision: 38,
                escala: 9
            })
        );
        // 76 cifras no caben en decimal128: texto, como dice el catálogo.
        assert_eq!(
            fisico(&json!({"name": "x", "type": "BIGNUMERIC"})),
            Some(Fisico::Texto)
        );
    }

    /// **Por REST, también en Arrow**: sin token la Storage Read declina, y la
    /// consulta lleva el `ORDER BY … NULLS LAST`, el `LIMIT` y el
    /// `jobTimeoutMs`; lo que vuelve sale con el tipo de la tabla.
    #[test]
    fn por_consulta_sale_en_arrow_con_el_tipo_de_la_tabla() {
        let respuesta = json!({
            "jobComplete": true,
            "jobReference": {"jobId": "j1", "location": "EU"},
            "totalRows": "2",
            "schema": {"fields": [
                {"name": "id", "type": "INTEGER"},
                {"name": "importe", "type": "NUMERIC"},
                {"name": "cuando", "type": "TIMESTAMP"},
                {"name": "datos", "type": "RECORD", "fields": [{"name": "a", "type": "STRING"}]}
            ]},
            "rows": [
                {"f": [{"v": "1"}, {"v": "9999999999.99"}, {"v": "1759572000000001"}, {"v": {"f": [{"v": "x"}]}}]},
                {"f": [{"v": "2"}, {"v": null}, {"v": null}, {"v": null}]}
            ]
        });
        let g = Guion::new(vec![Ok(tabla()), Ok(respuesta)]);
        let p = peticion(
            r#","orderBy":[{"columna":"id","direccion":"desc"}],"limit":2,"timeoutMs":5000"#,
        );
        let mut buf = Vec::new();
        assert_eq!(leer(&g, &p, &mut buf).expect("lee"), 2);
        let pedidas = g.pedidas.borrow();
        let post = &pedidas[1];
        assert!(
            post.contains("ORDER BY `id` DESC NULLS LAST LIMIT 2"),
            "{post}"
        );
        assert!(post.contains(r#""jobTimeoutMs":"5000""#), "{post}");
        let r = arrow_ipc::reader::StreamReader::try_new(std::io::Cursor::new(buf), None).unwrap();
        let esquema = r.schema();
        assert_eq!(esquema.field(1).data_type(), &DataType::Decimal128(12, 2));
        assert_eq!(
            esquema.field(2).data_type(),
            &DataType::Timestamp(TimeUnit::Microsecond, Some(UTC.into()))
        );
        assert_eq!(esquema.field(3).data_type(), &DataType::Utf8);
        let lotes: Vec<RecordBatch> = r.map(|l| l.unwrap()).collect();
        assert_eq!(lotes.iter().map(|l| l.num_rows()).sum::<usize>(), 2);
    }

    /// Una columna que no está es `objeto`, antes de lanzar nada.
    #[test]
    fn una_columna_que_no_esta_es_objeto() {
        let g = Guion::new(vec![Ok(tabla())]);
        let p = peticion(r#","filtros":[{"columna":"no_esta","operador":"eq","valor":"1"}]"#);
        let e = leer(&g, &p, &mut Vec::new()).expect_err("no está");
        assert_eq!(e.codigo, Codigo::Objeto);
        assert_eq!(g.pedidas.borrow().len(), 1, "sólo `tables.get`");
    }

    /// **Estimar es un dry run** y dice los bytes.
    #[test]
    fn estimar_es_un_dry_run() {
        let g = Guion::new(vec![
            Ok(tabla()),
            Ok(json!({"jobComplete": true, "totalBytesProcessed": "4096"})),
        ]);
        let j = estimar(&g, &peticion("")).expect("estima");
        assert_eq!(j, r#"{"bytes":4096,"fuente":"dryRun"}"#);
        assert!(g.pedidas.borrow()[1].contains(r#""dryRun":true"#));
    }

    #[test]
    fn los_errores_de_bigquery_con_su_codigo() {
        assert_eq!(
            fallo("BigQuery contestó 404: Not found: Table p:d.t".into()).codigo,
            Codigo::Objeto
        );
        assert_eq!(
            fallo("BigQuery contestó 403: Access Denied".into()).codigo,
            Codigo::Credencial
        );
        let t = fallo(format!("{}: 1000 ms", rest::AGOTADO));
        assert_eq!((t.codigo, t.reintentable), (Codigo::Tiempo, true));
        assert!(fallo("BigQuery contestó 503: backendError".into()).reintentable);
        assert_eq!(
            fallo("no se pudo hablar con BigQuery: dns".into()).codigo,
            Codigo::Conexion
        );
    }
}
