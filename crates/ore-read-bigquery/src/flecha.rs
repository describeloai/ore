//! **Las filas en Arrow, por la Storage Read API** (B2, ADR 0043).
//!
//! `jobs.query` entrega JSON página a página, y el texto es el camino lento:
//! medido con 2 M de filas, 157 s y 600 MB en el driver, y 2 GB en el almacén
//! que lo vuelve a tipar. La Storage Read API da la tabla en Arrow —el físico de
//! 0032 exacto: `decimal128(38, 9)`, `timestamp[us, UTC]`— por varios streams a
//! la vez, con la proyección y el filtro empujados: 21,7 s y 12 MB.
//!
//! # Cuándo se declina
//!
//! Esto es una preferencia de quien pide (`formato: arrow`), no una exigencia.
//! Se **declina** —y el driver lee por REST, en texto, como siempre— cuando lo
//! que se pide no se puede servir aquí sin cambiar el resultado:
//!
//! - el objeto no es una tabla (una vista no se lee por esta API);
//! - una columna es de un tipo cuyo Arrow no es el texto de siempre (`STRUCT`,
//!   `ARRAY`, `BIGNUMERIC`, `BYTES`, `JSON`, `GEOGRAPHY`…): por REST viajan como
//!   JSON o como texto canónico, y aquí saldrían con otra forma;
//! - un filtro no se sabe escribir como literal seguro (la API no admite
//!   parámetros);
//! - no hay token, o la cuenta no puede abrir una sesión de lectura
//!   (`bigquery.readsessions.create`).
//!
//! Declinar pasa **antes de escribir un solo byte**, así que quien lee ve o un
//! flujo Arrow entero o texto, nunca las dos cosas. Un fallo después —la red a
//! mitad de un stream— es un error del driver, y el flujo queda sin su marca de
//! fin: el almacén no sella una copia corta.
use crate::consultas;
use crate::rest::{self, Transporte};
use arrow_array::RecordBatch;
use arrow_schema::{Field, Schema, SchemaRef};
use bq_storage::google::cloud::bigquery::storage::v1 as api;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

pub enum Lectura {
    /// Se sirvió en Arrow: estas filas.
    Servida(u64),
    /// No se puede servir aquí sin cambiar el resultado, y por qué.
    Declina(String),
}

/// Los tipos que la Storage Read entrega con el mismo valor que el texto de
/// REST, una vez en el físico de 0032.
const TIPOS: &[&str] = &[
    "INT64",
    "INTEGER",
    "FLOAT64",
    "FLOAT",
    "NUMERIC",
    "BOOL",
    "BOOLEAN",
    "STRING",
    "DATE",
    "DATETIME",
    "TIME",
    "TIMESTAMP",
];

/// Cuántos streams se piden como mucho. El servidor da menos si la tabla es
/// pequeña; con más, la línea ya no da más de sí (medido desde fuera del
/// clúster).
const STREAMS: i32 = 8;

pub fn leer(
    t: &dyn Transporte,
    p: &ore_driver::Peticion,
    proyecto: &str,
    salida: &mut dyn std::io::Write,
) -> Result<Lectura, String> {
    let (dataset, tabla) = consultas::partes(&p.objeto)?;
    let info = rest::tabla(t, proyecto, dataset, tabla)?;
    let tipo = info["type"].as_str().unwrap_or("");
    if tipo != "TABLE" {
        return Ok(Lectura::Declina(format!(
            "`{}` es `{tipo}` y la Storage Read solo lee tablas",
            p.objeto
        )));
    }
    let campos: BTreeMap<&str, &Value> = info["schema"]["fields"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|c| Some((c["name"].as_str()?, c)))
                .collect()
        })
        .unwrap_or_default();

    // Las columnas, sin repetir y en su orden (dos propiedades pueden salir de
    // la misma columna).
    let mut columnas: Vec<String> = Vec::new();
    for (_, c) in &p.proyeccion {
        let campo = campos
            .get(c.as_str())
            .ok_or_else(|| format!("`{}` no tiene la columna `{c}`", p.objeto))?;
        let bq = campo["type"].as_str().unwrap_or("");
        if campo["mode"] == "REPEATED" || !TIPOS.contains(&bq) {
            return Ok(Lectura::Declina(format!(
                "la columna `{c}` es `{bq}`{}, que por REST viaja como texto o JSON y en Arrow \
                 tendría otra forma",
                if campo["mode"] == "REPEATED" {
                    " repetida"
                } else {
                    ""
                }
            )));
        }
        if !columnas.contains(c) {
            columnas.push(c.clone());
        }
    }
    let tipos: BTreeMap<String, String> = campos
        .iter()
        .filter_map(|(n, c)| {
            Some((
                n.to_string(),
                crate::valores::estandar(c["type"].as_str()?).to_string(),
            ))
        })
        .collect();
    let restriccion = match restriccion(p, proyecto, &tipos) {
        Ok(r) => r,
        Err(porque) => return Ok(Lectura::Declina(porque)),
    };
    let token = match t.token() {
        Ok(k) => k,
        Err(e) => return Ok(Lectura::Declina(e)),
    };
    let tabla_api = format!("projects/{proyecto}/datasets/{dataset}/tables/{tabla}");

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("no arranca el runtime: {e}"))?;
    rt.block_on(leer_async(
        token,
        proyecto.to_string(),
        tabla_api,
        columnas,
        restriccion,
        p,
        salida,
    ))
}

/// Las dos cabeceras de cada llamada. Un tipo con nombre y no un cierre: el
/// cliente se clona por stream, y un cierre en caja no se clona.
#[derive(Clone)]
struct Cabeceras {
    auth: tonic::metadata::MetadataValue<tonic::metadata::Ascii>,
    ruta: tonic::metadata::MetadataValue<tonic::metadata::Ascii>,
}

impl tonic::service::Interceptor for Cabeceras {
    fn call(&mut self, mut r: tonic::Request<()>) -> Result<tonic::Request<()>, tonic::Status> {
        r.metadata_mut().insert("authorization", self.auth.clone());
        r.metadata_mut()
            .insert("x-goog-request-params", self.ruta.clone());
        Ok(r)
    }
}

type Cliente = api::big_query_read_client::BigQueryReadClient<
    tonic::service::interceptor::InterceptedService<tonic::transport::Channel, Cabeceras>,
>;

async fn cliente(token: &str, tabla: &str) -> Result<Cliente, String> {
    let mut http = hyper_util::client::legacy::connect::HttpConnector::new();
    http.enforce_http(false);
    let tls = native_tls::TlsConnector::builder()
        .request_alpns(&["h2"])
        .build()
        .map_err(|e| format!("TLS: {e}"))?;
    let https = hyper_tls::HttpsConnector::from((http, tls.into()));
    let canal = tonic::transport::Endpoint::from_static("https://bigquerystorage.googleapis.com")
        .connect_with_connector(https)
        .await
        .map_err(|e| format!("no se pudo conectar con la Storage Read API: {e}"))?;
    let cabeceras = Cabeceras {
        auth: format!("Bearer {token}")
            .parse()
            .map_err(|_| "el token no cabe en una cabecera".to_string())?,
        // Sin ella, el servidor no sabe a qué región encaminar y contesta con
        // un error que no lo dice (medido).
        ruta: format!("read_session.table={tabla}")
            .parse()
            .map_err(|_| "el nombre de la tabla no cabe en una cabecera".to_string())?,
    };
    Ok(
        api::big_query_read_client::BigQueryReadClient::with_interceptor(canal, cabeceras)
            .max_decoding_message_size(256 << 20),
    )
}

enum Mensaje {
    Lote(Vec<u8>, i64),
    Fallo(String),
}

async fn leer_async(
    token: String,
    proyecto: String,
    tabla: String,
    columnas: Vec<String>,
    restriccion: String,
    p: &ore_driver::Peticion,
    salida: &mut dyn std::io::Write,
) -> Result<Lectura, String> {
    let mut c = cliente(&token, &tabla).await?;
    let sesion = c
        .create_read_session(api::CreateReadSessionRequest {
            parent: format!("projects/{proyecto}"),
            read_session: Some(api::ReadSession {
                table: tabla.clone(),
                data_format: api::DataFormat::Arrow as i32,
                read_options: Some(api::read_session::TableReadOptions {
                    selected_fields: columnas.clone(),
                    row_restriction: restriccion,
                    output_format_serialization_options: Some(
                        api::read_session::table_read_options::OutputFormatSerializationOptions::ArrowSerializationOptions(
                            api::ArrowSerializationOptions {
                                buffer_compression: api::arrow_serialization_options::CompressionCodec::Lz4Frame as i32,
                                ..Default::default()
                            },
                        ),
                    ),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            max_stream_count: STREAMS,
            ..Default::default()
        })
        .await;
    let sesion = match sesion {
        Ok(s) => s.into_inner(),
        Err(s) if s.code() == tonic::Code::PermissionDenied => {
            return Ok(Lectura::Declina(format!(
                "la cuenta no puede abrir una sesión de lectura (hace falta \
                 `bigquery.readsessions.create`, p. ej. `roles/bigquery.readSessionUser`): {}",
                s.message()
            )));
        }
        Err(s) => {
            return Err(format!(
                "la Storage Read API no abrió la sesión: {} ({:?})",
                s.message(),
                s.code()
            ));
        }
    };
    let esquema_bq = match sesion.schema {
        Some(api::read_session::Schema::ArrowSchema(a)) => a.serialized_schema,
        _ => return Err("la sesión de lectura no trae su esquema Arrow".into()),
    };
    let origen = arrow_ipc::reader::StreamReader::try_new(std::io::Cursor::new(&esquema_bq), None)
        .map_err(|e| format!("el esquema de la sesión no se lee: {e}"))?
        .schema();
    let esquema = salida_de(&origen, p)?;

    let mut escritor = arrow_ipc::writer::StreamWriter::try_new(salida, &esquema)
        .map_err(|e| format!("no se pudo empezar el flujo: {e}"))?;

    // Un stream por tarea, y los lotes a un canal acotado: el que escribe
    // marca el paso y la memoria no crece con la tabla.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Mensaje>(8);
    for st in &sesion.streams {
        let (mut c, tx, nombre) = (c.clone(), tx.clone(), st.name.clone());
        tokio::spawn(async move {
            if let Err(e) = leer_stream(&mut c, &nombre, &tx).await {
                let _ = tx.send(Mensaje::Fallo(e)).await;
            }
        });
    }
    drop(tx);

    let (mut anunciadas, mut filas) = (0i64, 0u64);
    while let Some(m) = rx.recv().await {
        let (bytes, n) = match m {
            Mensaje::Lote(b, n) => (b, n),
            Mensaje::Fallo(e) => return Err(e),
        };
        anunciadas += n;
        let mut buf = esquema_bq.clone();
        buf.extend_from_slice(&bytes);
        let lector = arrow_ipc::reader::StreamReader::try_new(std::io::Cursor::new(buf), None)
            .map_err(|e| format!("un lote de la Storage Read no se lee: {e}"))?;
        for lote in lector {
            let lote = lote.map_err(|e| format!("un lote de la Storage Read no se lee: {e}"))?;
            filas += lote.num_rows() as u64;
            escritor
                .write(&proyectar(&lote, &esquema, p)?)
                .map_err(|e| format!("no se pudo escribir el flujo: {e}"))?;
        }
    }
    if anunciadas as u64 != filas {
        return Err(format!(
            "la Storage Read anunció {anunciadas} filas y se leyeron {filas}: no se entrega \
             una copia que no cuadra"
        ));
    }
    // La marca de fin: sin ella el almacén no sella (ADR 0043).
    escritor
        .finish()
        .map_err(|e| format!("no se pudo cerrar el flujo: {e}"))?;
    Ok(Lectura::Servida(filas))
}

/// Un stream entero, reanudando por `offset` si se corta: la API lo admite y
/// una lectura larga se corta de verdad.
async fn leer_stream(
    c: &mut Cliente,
    nombre: &str,
    tx: &tokio::sync::mpsc::Sender<Mensaje>,
) -> Result<(), String> {
    let mut offset = 0i64;
    let mut intentos = 0;
    loop {
        let r = c
            .read_rows(api::ReadRowsRequest {
                read_stream: nombre.to_string(),
                offset,
            })
            .await;
        let mut flujo = match r {
            Ok(f) => f.into_inner(),
            Err(s) if intentos < 3 && reintentable(&s) => {
                intentos += 1;
                continue;
            }
            Err(s) => return Err(format!("ReadRows falló: {} ({:?})", s.message(), s.code())),
        };
        loop {
            match flujo.message().await {
                Ok(Some(m)) => {
                    if let Some(api::read_rows_response::Rows::ArrowRecordBatch(b)) = m.rows {
                        offset += m.row_count;
                        if tx
                            .send(Mensaje::Lote(b.serialized_record_batch, m.row_count))
                            .await
                            .is_err()
                        {
                            return Ok(());
                        }
                    }
                }
                Ok(None) => return Ok(()),
                Err(s) if intentos < 3 && reintentable(&s) => {
                    intentos += 1;
                    break;
                }
                Err(s) => {
                    return Err(format!(
                        "un stream se cortó en la fila {offset}: {} ({:?})",
                        s.message(),
                        s.code()
                    ));
                }
            }
        }
    }
}

fn reintentable(s: &tonic::Status) -> bool {
    matches!(
        s.code(),
        tonic::Code::Unavailable | tonic::Code::Internal | tonic::Code::DeadlineExceeded
    )
}

/// El esquema que sale: una columna por propiedad, con el nombre de la
/// propiedad (el de la columna es del origen y no sale del driver).
fn salida_de(origen: &Schema, p: &ore_driver::Peticion) -> Result<SchemaRef, String> {
    let campos = p
        .proyeccion
        .iter()
        .map(|(campo, col)| {
            let f = origen
                .field_with_name(col)
                .map_err(|_| format!("la sesión no trae la columna `{col}`"))?;
            Ok(Field::new(campo, f.data_type().clone(), true))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Arc::new(Schema::new(campos)))
}

fn proyectar(
    lote: &RecordBatch,
    esquema: &SchemaRef,
    p: &ore_driver::Peticion,
) -> Result<RecordBatch, String> {
    let columnas = p
        .proyeccion
        .iter()
        .map(|(_, col)| {
            lote.column_by_name(col)
                .cloned()
                .ok_or_else(|| format!("un lote no trae la columna `{col}`"))
        })
        .collect::<Result<Vec<_>, String>>()?;
    RecordBatch::try_new(esquema.clone(), columnas).map_err(|e| format!("el lote no casa: {e}"))
}

/// **El `WHERE` de `ore-sql`, con los parámetros escritos como literales.**
///
/// La Storage Read no admite parámetros: `row_restriction` es texto. Se
/// reutiliza la traducción de la forma —la misma que va por REST— y cada
/// `@pN` se sustituye por un literal **validado por su tipo**. Lo que no se
/// sabe escribir con seguridad declina: nunca se concatena un valor sin
/// comprobar.
pub fn restriccion(
    p: &ore_driver::Peticion,
    proyecto: &str,
    tipos: &BTreeMap<String, String>,
) -> Result<String, String> {
    use ore_sql::dialectos::BIGQUERY;
    let objeto = consultas::cualificado(proyecto, &p.objeto);
    let entera = ore_sql::consulta(p, &BIGQUERY, &objeto, tipos)?;
    let base = ore_sql::consulta(
        &ore_driver::Peticion {
            proyeccion: p.proyeccion.clone(),
            ..Default::default()
        },
        &BIGQUERY,
        &objeto,
        tipos,
    )?;
    let Some(resto) = entera.texto.strip_prefix(&base.texto) else {
        return Err("la consulta no empieza por su SELECT: no se sabe separar el WHERE".into());
    };
    let resto = resto.trim();
    if resto.is_empty() {
        return Ok(String::new());
    }
    let mut cond = resto
        .strip_prefix("WHERE ")
        .ok_or_else(|| format!("lo que sigue al SELECT no es un WHERE: `{resto}`"))?
        .to_string();
    // De atrás adelante: `@p1` es prefijo de `@p10`.
    for (i, par) in entera.parametros.iter().enumerate().rev() {
        let mut partes = par.splitn(3, ':');
        let (_, tipo, valor) = (
            partes.next().unwrap_or(""),
            partes.next().unwrap_or(""),
            partes.next().unwrap_or(""),
        );
        cond = cond.replace(&format!("@p{i}"), &literal(tipo, valor)?);
    }
    Ok(cond)
}

/// Un valor como literal de GoogleSQL, o por qué no.
pub fn literal(tipo: &str, v: &str) -> Result<String, String> {
    let decimal = |v: &str| {
        let d = v.strip_prefix('-').unwrap_or(v);
        let (e, f) = d.split_once('.').unwrap_or((d, "0"));
        !e.is_empty()
            && !f.is_empty()
            && e.bytes().all(|b| b.is_ascii_digit())
            && f.bytes().all(|b| b.is_ascii_digit())
    };
    let fechable = |v: &str| {
        !v.is_empty()
            && v.bytes()
                .all(|b| b.is_ascii_digit() || b" -:.+TZ".contains(&b))
    };
    let no = || {
        Err(format!(
            "`{v}` no es un literal `{tipo}` que se sepa escribir sin riesgo"
        ))
    };
    Ok(match tipo {
        "INT64" => match v.parse::<i64>() {
            Ok(n) => n.to_string(),
            Err(_) => return no(),
        },
        "FLOAT64" => match v.parse::<f64>() {
            Ok(f) if f.is_finite() => format!("{f:?}"),
            _ => return no(),
        },
        "NUMERIC" if decimal(v) => format!("NUMERIC '{v}'"),
        "BOOL" if v == "true" || v == "false" => v.to_string(),
        "DATE" | "DATETIME" | "TIME" | "TIMESTAMP" if fechable(v) => format!("{tipo} '{v}'"),
        "STRING" => {
            if v.chars().any(|c| c.is_control()) {
                return no();
            }
            format!("'{}'", v.replace('\\', "\\\\").replace('\'', "\\'"))
        }
        _ => return no(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tipos() -> BTreeMap<String, String> {
        [
            ("id", "STRING"),
            ("n", "INT64"),
            ("total", "NUMERIC"),
            ("ts", "TIMESTAMP"),
        ]
        .into_iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
    }

    fn peticion(filtros: &[(&str, &str, &str)]) -> ore_driver::Peticion {
        ore_driver::Peticion {
            url: "bigquery://p/d".into(),
            objeto: "d.t".into(),
            proyeccion: vec![("clave".into(), "id".into()), ("n".into(), "n".into())],
            filtros: filtros
                .iter()
                .map(|(a, b, c)| (a.to_string(), b.to_string(), c.to_string()))
                .collect(),
            ..Default::default()
        }
    }

    /// Sin filtros no hay restricción: se lee la tabla entera.
    #[test]
    fn sin_filtros_no_hay_restriccion() {
        assert_eq!(restriccion(&peticion(&[]), "p", &tipos()).unwrap(), "");
    }

    /// El filtro de la forma, con el literal de su tipo.
    #[test]
    fn el_filtro_va_con_el_literal_de_su_tipo() {
        let r = restriccion(
            &peticion(&[("id", "eq", "ore-e2e-c1"), ("n", "gt", "7")]),
            "p",
            &tipos(),
        )
        .unwrap();
        assert!(r.contains("`id` = 'ore-e2e-c1'"), "{r}");
        assert!(r.contains("`n` > 7"), "{r}");
        assert!(!r.contains('@'), "{r}");
    }

    /// Una comilla se escapa, y un entero que no es un entero no se escribe.
    #[test]
    fn lo_que_no_es_seguro_no_se_escribe() {
        assert_eq!(literal("STRING", "o'neil").unwrap(), r"'o\'neil'");
        assert_eq!(literal("STRING", r"a\'b").unwrap(), r"'a\\\'b'");
        assert!(literal("INT64", "1 OR 1=1").is_err());
        assert!(literal("NUMERIC", "1e5").is_err());
        assert!(literal("TIMESTAMP", "2026' OR '1").is_err());
        assert!(literal("STRING", "a\nb").is_err());
        assert!(literal("BYTES", "x").is_err());
        assert_eq!(literal("NUMERIC", "-2.50").unwrap(), "NUMERIC '-2.50'");
    }

    /// `@p1` no se come el principio de `@p10`.
    #[test]
    fn once_parametros_no_se_pisan() {
        let fs: Vec<(String, String, String)> = (0..11)
            .map(|i| ("n".to_string(), "eq".to_string(), i.to_string()))
            .collect();
        let mut p = peticion(&[]);
        p.filtros = fs;
        let r = restriccion(&p, "p", &tipos()).unwrap();
        assert!(r.contains("`n` = 10"), "{r}");
        assert!(r.contains("`n` = 1 ") || r.ends_with("`n` = 1"), "{r}");
    }
}
