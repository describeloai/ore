//! MEDIDA · W3.6a · lo que iceberg-rust escribe (0031 §10: la copia es un dataset).
//!
//! Un programa de un fichero: crea una tabla Iceberg con los tipos del contrato
//! (0032 §1), le anexa N filas en lotes (el `fast_append` que el Job de copia
//! haría), le anexa un segundo lote (snapshot 2), le añade una columna, expira
//! el primer snapshot, y escribe el puntero (`puntero.json`) como lo escribiría
//! `ore materialize`. Imprime una línea `### {json}` por hecho medido; el arnés
//! Python (`medida-w3-iceberg-rust.py`) lo compila, lo corre contra un
//! directorio y contra `gs://`, y coteja lo escrito con DuckDB.
//!
//! El catálogo es el de memoria de la librería: lo que aquí se mide es el
//! ESCRITOR (Parquet + manifiestos + metadata.json + el commit con sus
//! requisitos), no el swap del puntero, que ya está medido en Python y que en
//! Rust es implementar `Catalog::update_table` sobre el fichero.
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use arrow_array::builder::{
    BooleanBuilder, Date32Builder, Decimal128Builder, Float64Builder, Int64Builder,
    StringBuilder, Time64MicrosecondBuilder, TimestampMicrosecondBuilder,
};
use arrow_array::{ArrayRef, Float64Array, Int32Array, Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema as ArrowSchema, TimeUnit};
use iceberg::arrow::{arrow_schema_to_schema_auto_assign_ids, schema_to_arrow_schema};
use iceberg::io::GCS_TOKEN;
use iceberg::memory::{MEMORY_CATALOG_WAREHOUSE, MemoryCatalogBuilder};
use iceberg::spec::{DataFileFormat, NestedField, PrimitiveType, Type};
use iceberg::transaction::{ApplyTransactionAction, Transaction};
use iceberg::writer::base_writer::data_file_writer::DataFileWriterBuilder;
use iceberg::writer::file_writer::ParquetWriterBuilder;
use iceberg::writer::file_writer::location_generator::{
    DefaultFileNameGenerator, DefaultLocationGenerator,
};
use iceberg::writer::file_writer::rolling_writer::RollingFileWriterBuilder;
use iceberg::writer::{IcebergWriter, IcebergWriterBuilder};
use iceberg::{Catalog, CatalogBuilder, NamespaceIdent, TableCreation};
use iceberg_storage_opendal::OpenDalStorageFactory;
use parquet::file::properties::WriterProperties;

fn di(que: &str, v: serde_json::Value) {
    let mut m = serde_json::Map::new();
    m.insert("que".into(), que.into());
    if let serde_json::Value::Object(o) = v {
        m.extend(o);
    }
    println!("### {}", serde_json::Value::Object(m));
}

fn ms(t: Instant) -> u128 {
    t.elapsed().as_millis()
}

/// Las cuatro columnas de `tabla_grande` de `medida-w3-leer.py`, generadas igual.
fn lote_grande(desde: i64, n: i64) -> RecordBatch {
    let ids: Vec<i64> = (desde..desde + n).collect();
    let clientes: Vec<i32> = ids.iter().map(|i| (i % 1000) as i32).collect();
    let importes: Vec<f64> = ids.iter().map(|i| ((i * 7919) % 100000) as f64 / 100.0).collect();
    let paises: Vec<String> = ids.iter().map(|i| ((b'A' + (i % 26) as u8) as char).to_string()).collect();
    RecordBatch::try_new(
        esquema_grande(),
        vec![
            Arc::new(Int64Array::from(ids)) as ArrayRef,
            Arc::new(Int32Array::from(clientes)),
            Arc::new(Float64Array::from(importes)),
            Arc::new(StringArray::from(paises)),
        ],
    )
    .unwrap()
}

fn esquema_grande() -> Arc<ArrowSchema> {
    Arc::new(ArrowSchema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("cliente", DataType::Int32, true),
        Field::new("importe", DataType::Float64, true),
        Field::new("pais", DataType::Utf8, true),
    ]))
}

/// Los físicos del contrato (0032 §1) que salen de `carga.rs`, con nulos.
fn lote_tipos() -> RecordBatch {
    let esquema = Arc::new(ArrowSchema::new(vec![
        Field::new("entero", DataType::Int64, true),
        Field::new("real", DataType::Float64, true),
        Field::new("logico", DataType::Boolean, true),
        Field::new("texto", DataType::Utf8, true),
        Field::new("decimal", DataType::Decimal128(38, 18), true),
        Field::new("dinero", DataType::Decimal128(10, 2), true),
        Field::new("fecha", DataType::Date32, true),
        Field::new("hora", DataType::Time64(TimeUnit::Microsecond), true),
        Field::new("fecha_hora", DataType::Timestamp(TimeUnit::Microsecond, None), true),
        Field::new("instante", DataType::Timestamp(TimeUnit::Microsecond, Some("+00:00".into())), true),
    ]));
    let mut entero = Int64Builder::new();
    entero.append_value(9007199254740993);
    entero.append_null();
    entero.append_value(-1);
    let mut real = Float64Builder::new();
    real.append_value(1.5);
    real.append_null();
    real.append_value(f64::NAN);
    let mut logico = BooleanBuilder::new();
    logico.append_value(true);
    logico.append_null();
    logico.append_value(false);
    let mut texto = StringBuilder::new();
    texto.append_value("ñandú 🐍");
    texto.append_null();
    texto.append_value("");
    let mut decimal = Decimal128Builder::new().with_data_type(DataType::Decimal128(38, 18));
    decimal.append_value(12345678901234567890123456789i128); // 12345678901.234567890123456789
    decimal.append_null();
    decimal.append_value(-1);
    let mut dinero = Decimal128Builder::new().with_data_type(DataType::Decimal128(10, 2));
    dinero.append_value(150);
    dinero.append_null();
    dinero.append_value(-5);
    let mut fecha = Date32Builder::new();
    fecha.append_value(20715); // 2026-09-19
    fecha.append_null();
    fecha.append_value(-1); // 1969-12-31
    let mut hora = Time64MicrosecondBuilder::new();
    hora.append_value(14 * 3_600_000_000 + 30 * 60_000_000 + 123456);
    hora.append_null();
    hora.append_value(0);
    let mut fecha_hora = TimestampMicrosecondBuilder::new();
    fecha_hora.append_value(1_789_828_200_123_456); // 2026-09-19 14:30:00.123456
    fecha_hora.append_null();
    fecha_hora.append_value(-1_000_000); // 1969-12-31 23:59:59
    // Iceberg nombra la zona "+00:00", no "UTC": misma física, otro nombre; el escritor lo tiene que casar (0032: timestamp[us, tz=UTC]).
    let mut instante = TimestampMicrosecondBuilder::new().with_timezone("+00:00");
    instante.append_value(1_789_828_200_123_456);
    instante.append_null();
    instante.append_value(-1_000_000);
    RecordBatch::try_new(
        esquema,
        vec![
            Arc::new(entero.finish()) as ArrayRef,
            Arc::new(real.finish()),
            Arc::new(logico.finish()),
            Arc::new(texto.finish()),
            Arc::new(decimal.finish()),
            Arc::new(dinero.finish()),
            Arc::new(fecha.finish()),
            Arc::new(hora.finish()),
            Arc::new(fecha_hora.finish()),
            Arc::new(instante.finish()),
        ],
    )
    .unwrap()
}

/// Escribe lotes en una tabla y confirma un `fast_append`. Devuelve la tabla nueva.
async fn anexar(
    catalogo: &dyn Catalog,
    tabla: &iceberg::table::Table,
    lotes: impl IntoIterator<Item = RecordBatch>,
    prefijo: &str,
) -> iceberg::Result<(iceberg::table::Table, usize, u64, u128, u128)> {
    let t0 = Instant::now();
    let ubicacion = DefaultLocationGenerator::new(tabla.metadata())?;
    let nombres = DefaultFileNameGenerator::new(prefijo.to_string(), None, DataFileFormat::Parquet);
    // SNAPPY, como la copia de hoy (0015): mismos bytes para la misma entrada.
    let props = WriterProperties::builder()
        .set_compression(parquet::basic::Compression::SNAPPY)
        .build();
    let parquet = ParquetWriterBuilder::new(props, tabla.metadata().current_schema().clone());
    let rodante = RollingFileWriterBuilder::new_with_default_file_size(
        parquet,
        tabla.file_io().clone(),
        ubicacion,
        nombres,
    );
    let mut escritor = DataFileWriterBuilder::new(rodante).build(None).await?;
    // El escritor exige que cada lote lleve los ids de campo de Iceberg en los
    // metadatos del esquema de Arrow (`PARQUET:field_id`): se reenvuelven las
    // mismas columnas con el esquema que sale de la tabla.
    let esquema_arrow = Arc::new(schema_to_arrow_schema(tabla.metadata().current_schema())?);
    for lote in lotes {
        let lote = RecordBatch::try_new(esquema_arrow.clone(), lote.columns().to_vec())
            .map_err(|e| iceberg::Error::new(iceberg::ErrorKind::DataInvalid, e.to_string()))?;
        escritor.write(lote).await?;
    }
    let ficheros = escritor.close().await?;
    let escritos_ms = ms(t0);
    let n = ficheros.len();
    let bytes: u64 = ficheros.iter().map(|f| f.file_size_in_bytes()).sum();
    let t1 = Instant::now();
    let tx = Transaction::new(tabla);
    let tx = tx.fast_append().add_data_files(ficheros).apply(tx)?;
    let tabla = tx.commit(catalogo).await?;
    Ok((tabla, n, bytes, escritos_ms, ms(t1)))
}

fn puntero(dir: &str, nombre: &str, tabla: &iceberg::table::Table) {
    let p = serde_json::json!({
        "metadata_location": tabla.metadata_location(),
        "snapshot": tabla.metadata().current_snapshot().map(|s| s.snapshot_id()),
    });
    if let Ok(_) = std::fs::create_dir_all(dir) {
        let _ = std::fs::write(format!("{dir}/{nombre}.json"), serde_json::to_string_pretty(&p).unwrap());
    }
}

#[tokio::main]
async fn main() -> iceberg::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mut bodega = String::new();
    let mut punteros = String::from(".");
    let mut filas: i64 = 10_000_000;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--bodega" => { bodega = args[i + 1].clone(); i += 1; }
            "--punteros" => { punteros = args[i + 1].clone(); i += 1; }
            "--filas" => { filas = args[i + 1].parse().unwrap(); i += 1; }
            _ => {}
        }
        i += 1;
    }
    if bodega.is_empty() {
        eprintln!("--bodega file:///dir | gs://bucket/prefijo  [--punteros dir] [--filas n]");
        std::process::exit(2);
    }
    let en_gcs = bodega.starts_with("gs://");
    let mut props = HashMap::from([(MEMORY_CATALOG_WAREHOUSE.to_string(), bodega.clone())]);
    if en_gcs {
        // El token de quien lo corre (gcloud) llega por el entorno y no se imprime.
        // En el pod no haría falta: opendal pide el token al servidor de metadatos.
        if let Ok(tok) = std::env::var("GCS_TOKEN") {
            props.insert(GCS_TOKEN.to_string(), tok);
        }
    }
    let fabrica: Arc<dyn iceberg::io::StorageFactory> = Arc::new(if en_gcs { OpenDalStorageFactory::Gcs } else { OpenDalStorageFactory::Fs });
    let t0 = Instant::now();
    let catalogo = MemoryCatalogBuilder::default()
        .with_storage_factory(fabrica)
        .load("medida", props)
        .await?;
    let ns = NamespaceIdent::new("lago".into());
    catalogo.create_namespace(&ns, HashMap::new()).await?;
    di("catalogo", serde_json::json!({"ms": ms(t0), "bodega": bodega, "iceberg": "0.10.1"}));

    // §1 · la tabla grande: crear + anexar N filas en lotes de 1 M
    let t0 = Instant::now();
    let esquema = arrow_schema_to_schema_auto_assign_ids(&esquema_grande())?;
    let creacion = TableCreation::builder().name("grande".to_string()).schema(esquema).build();
    let tabla = catalogo.create_table(&ns, creacion).await?;
    let crear_ms = ms(t0);
    let lote = 1_000_000.min(filas);
    let lotes = (0..filas).step_by(lote as usize).map(move |d| lote_grande(d, lote.min(filas - d)));
    let t0 = Instant::now();
    let gen_ms = { let _ = lote_grande(0, 1); ms(t0) };
    let (tabla, n, bytes, escritos_ms, commit_ms) = anexar(&catalogo, &tabla, lotes, "grande").await?;
    puntero(&punteros, "grande", &tabla);
    di("grande", serde_json::json!({"filas": filas, "crear_ms": crear_ms, "generar_ms": gen_ms, "escribir_ms": escritos_ms, "commit_ms": commit_ms, "ficheros": n, "bytes": bytes, "metadata_location": tabla.metadata_location(), "snapshot": tabla.metadata().current_snapshot().map(|s| s.snapshot_id())}));

    // §2 · anexar 1 M más (snapshot 2)
    let (tabla, n2, bytes2, e2, c2) = anexar(&catalogo, &tabla, [lote_grande(filas, 1_000_000)], "delta").await?;
    puntero(&punteros, "grande", &tabla);
    di("append", serde_json::json!({"filas": 1_000_000, "escribir_ms": e2, "commit_ms": c2, "ficheros": n2, "bytes": bytes2, "snapshots": tabla.metadata().snapshots().count(), "metadata_location": tabla.metadata_location()}));

    // §3 · evolucionar: una columna nueva (lo que 0.10 sabe hacer; promover int → long, no)
    let t0 = Instant::now();
    let tx = Transaction::new(&tabla);
    let accion = tx.update_schema().add_column(iceberg::transaction::AddColumn::optional(
        "canal",
        Type::Primitive(PrimitiveType::String),
    ));
    let resultado = match accion.apply(tx) {
        Ok(tx) => tx.commit(&catalogo).await,
        Err(e) => Err(e),
    };
    match resultado {
        Ok(t) => {
            let columnas: Vec<String> = t.metadata().current_schema().as_struct().fields().iter().map(|f: &Arc<NestedField>| format!("{}:{}", f.name, f.field_type)).collect();
            puntero(&punteros, "grande", &t);
            di("esquema", serde_json::json!({"ok": true, "ms": ms(t0), "columnas": columnas, "metadata_location": t.metadata_location()}));
            // §4 · expirar el snapshot 1 (mantenimiento)
            let t1 = Instant::now();
            let tx = Transaction::new(&t);
            let r = tx.expire_snapshots().retain_last(1).apply(tx).and_then(|tx| Ok(tx));
            match r {
                Ok(tx) => match tx.commit(&catalogo).await {
                    Ok(t) => { puntero(&punteros, "grande", &t); di("expirar", serde_json::json!({"ok": true, "ms": ms(t1), "snapshots": t.metadata().snapshots().count(), "metadata_location": t.metadata_location()})); }
                    Err(e) => di("expirar", serde_json::json!({"ok": false, "porque": e.to_string()})),
                },
                Err(e) => di("expirar", serde_json::json!({"ok": false, "porque": e.to_string()})),
            }
        }
        Err(e) => di("esquema", serde_json::json!({"ok": false, "ms": ms(t0), "porque": e.to_string()})),
    }

    // §5 · los tipos del contrato
    let t0 = Instant::now();
    let lote = lote_tipos();
    let esquema = arrow_schema_to_schema_auto_assign_ids(lote.schema().as_ref())?;
    let creacion = TableCreation::builder().name("tipos".to_string()).schema(esquema).build();
    let r = catalogo.create_table(&ns, creacion).await;
    match r {
        Ok(tabla) => match anexar(&catalogo, &tabla, [lote], "tipos").await {
            Ok((t, _, _, _, _)) => {
                puntero(&punteros, "tipos", &t);
                di("tipos", serde_json::json!({"ok": true, "ms": ms(t0), "metadata_location": t.metadata_location(), "iceberg": t.metadata().current_schema().as_struct().fields().iter().map(|f| format!("{}:{}", f.name, f.field_type)).collect::<Vec<_>>()}));
            }
            Err(e) => di("tipos", serde_json::json!({"ok": false, "porque": e.to_string()})),
        },
        Err(e) => di("tipos", serde_json::json!({"ok": false, "porque": e.to_string()})),
    }
    Ok(())
}
