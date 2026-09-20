//! MEDIDA · W3.6c · Node sin escritor de Iceberg: la tabla Arrow viaja por IPC
//! (stdin) al agente y el agente escribe la tabla con iceberg-rust (lo que
//! `ore-store` hace). Lee el flujo IPC, crea la tabla con el esquema del primer
//! lote, anexa todos los lotes, confirma, e imprime una línea `### {json}` con
//! filas, ficheros, bytes y los tiempos de leer, escribir y confirmar.
//!
//!   node genera.mjs | ipc --bodega file:///dir
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use arrow_array::RecordBatch;
use arrow_ipc::reader::StreamReader;
use iceberg::arrow::{arrow_schema_to_schema_auto_assign_ids, schema_to_arrow_schema};
use iceberg::memory::{MEMORY_CATALOG_WAREHOUSE, MemoryCatalogBuilder};
use iceberg::spec::DataFileFormat;
use iceberg::transaction::{ApplyTransactionAction, Transaction};
use iceberg::writer::base_writer::data_file_writer::DataFileWriterBuilder;
use iceberg::writer::file_writer::ParquetWriterBuilder;
use iceberg::writer::file_writer::location_generator::{DefaultFileNameGenerator, DefaultLocationGenerator};
use iceberg::writer::file_writer::rolling_writer::RollingFileWriterBuilder;
use iceberg::writer::{IcebergWriter, IcebergWriterBuilder};
use iceberg::{Catalog, CatalogBuilder, NamespaceIdent, TableCreation};
use iceberg_storage_opendal::OpenDalStorageFactory;
use parquet::file::properties::WriterProperties;

fn ms(t: Instant) -> u128 {
    t.elapsed().as_millis()
}

#[tokio::main]
async fn main() -> iceberg::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let bodega = args.iter().position(|a| a == "--bodega").map(|i| args[i + 1].clone()).unwrap_or_default();
    if bodega.is_empty() {
        eprintln!("--bodega file:///dir");
        std::process::exit(2);
    }
    let fabrica: Arc<dyn iceberg::io::StorageFactory> = Arc::new(OpenDalStorageFactory::Fs);
    let catalogo = MemoryCatalogBuilder::default()
        .with_storage_factory(fabrica)
        .load("medida", HashMap::from([(MEMORY_CATALOG_WAREHOUSE.to_string(), bodega.clone())]))
        .await?;
    let ns = NamespaceIdent::new("lago".into());
    catalogo.create_namespace(&ns, HashMap::new()).await?;

    // Leer el flujo IPC de stdin, lote a lote, escribiendo según llega.
    let t0 = Instant::now();
    let stdin = std::io::stdin();
    let lector = StreamReader::try_new(std::io::BufReader::with_capacity(1 << 20, stdin.lock()), None)
        .map_err(|e| iceberg::Error::new(iceberg::ErrorKind::DataInvalid, e.to_string()))?;
    let esquema_ipc = lector.schema();
    let esquema = arrow_schema_to_schema_auto_assign_ids(esquema_ipc.as_ref())?;
    let creacion = TableCreation::builder().name("ipc".to_string()).schema(esquema).build();
    let tabla = catalogo.create_table(&ns, creacion).await?;
    let ubicacion = DefaultLocationGenerator::new(tabla.metadata())?;
    let nombres = DefaultFileNameGenerator::new("ipc".to_string(), None, DataFileFormat::Parquet);
    let props = WriterProperties::builder().set_compression(parquet::basic::Compression::SNAPPY).build();
    let parquet = ParquetWriterBuilder::new(props, tabla.metadata().current_schema().clone());
    let rodante = RollingFileWriterBuilder::new_with_default_file_size(parquet, tabla.file_io().clone(), ubicacion, nombres);
    let mut escritor = DataFileWriterBuilder::new(rodante).build(None).await?;
    let esquema_arrow = Arc::new(schema_to_arrow_schema(tabla.metadata().current_schema())?);
    let mut filas = 0usize;
    let mut lotes = 0usize;
    let mut leer_ms = 0u128;
    let mut escribir_ms = 0u128;
    let mut t = Instant::now();
    for lote in lector {
        let lote = lote.map_err(|e| iceberg::Error::new(iceberg::ErrorKind::DataInvalid, e.to_string()))?;
        leer_ms += ms(t);
        let t1 = Instant::now();
        filas += lote.num_rows();
        lotes += 1;
        let lote = RecordBatch::try_new(esquema_arrow.clone(), lote.columns().to_vec())
            .map_err(|e| iceberg::Error::new(iceberg::ErrorKind::DataInvalid, e.to_string()))?;
        escritor.write(lote).await?;
        escribir_ms += ms(t1);
        t = Instant::now();
    }
    let t1 = Instant::now();
    let ficheros = escritor.close().await?;
    escribir_ms += ms(t1);
    let n = ficheros.len();
    let bytes: u64 = ficheros.iter().map(|f| f.file_size_in_bytes()).sum();
    let t2 = Instant::now();
    let tx = Transaction::new(&tabla);
    let tx = tx.fast_append().add_data_files(ficheros).apply(tx)?;
    let tabla = tx.commit(&catalogo).await?;
    println!(
        "### {}",
        serde_json::json!({
            "filas": filas, "lotes": lotes, "ficheros": n, "bytes": bytes,
            "leer_ms": leer_ms, "escribir_ms": escribir_ms, "commit_ms": ms(t2), "total_ms": ms(t0),
            "columnas": esquema_ipc.fields().iter().map(|f| format!("{}:{}", f.name(), f.data_type())).collect::<Vec<_>>(),
            "metadata_location": tabla.metadata_location(),
        })
    );
    Ok(())
}
