//! G2·0 (ADR 0067): un fichero de datos como lo escribirá G2·1 —GEOMETRY nativo de Parquet, WKB,
//! CRS explícito— con los `field_id` de Iceberg (1 id, 2 geom), para registrarlo en una tabla Iceberg.
use arrow_array::{BinaryArray, Int64Array, RecordBatch};
use arrow_schema::{DataType, Field, Schema};
use std::collections::HashMap;
use std::sync::Arc;

fn main() {
    let n = 1000usize;
    let geoms: Vec<Vec<u8>> = (0..n)
        .map(|i| {
            let mut v = vec![1u8, 1, 0, 0, 0];
            v.extend_from_slice(&(-3.7 + i as f64 * 0.001).to_le_bytes());
            v.extend_from_slice(&(40.4 + i as f64 * 0.001).to_le_bytes());
            v
        })
        .collect();
    let id = |n: i32| HashMap::from([("PARQUET:field_id".to_string(), n.to_string())]);
    let md = parquet_geospatial::WkbMetadata::new(Some("OGC:CRS84"), None);
    let geom = Field::new("geom", DataType::Binary, true)
        .with_extension_type(parquet_geospatial::WkbType::new(Some(md)));
    let mut gm = geom.metadata().clone();
    gm.extend(id(2));
    let esquema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, true).with_metadata(id(1)),
        geom.with_metadata(gm),
    ]));
    let lote = RecordBatch::try_new(
        esquema.clone(),
        vec![
            Arc::new(Int64Array::from_iter_values(0..n as i64)),
            Arc::new(BinaryArray::from_iter_values(geoms.iter())),
        ],
    )
    .unwrap();
    for (ruta, geo) in [("/salida/geo-nativo.parquet", true), ("/salida/geo-plano.parquet", false)] {
        // `geo-plano`: el mismo WKB como BYTE_ARRAY sin tipo lógico (lo que escribe hoy iceberg-rust).
        let (s, l) = if geo {
            (esquema.clone(), lote.clone())
        } else {
            let s = Arc::new(Schema::new(vec![
                esquema.field(0).clone(),
                Field::new("geom", DataType::Binary, true).with_metadata(id(2)),
            ]));
            (s.clone(), RecordBatch::try_new(s, lote.columns().to_vec()).unwrap())
        };
        let f = std::fs::File::create(ruta).unwrap();
        let mut w = parquet::arrow::ArrowWriter::try_new(f, s, None).unwrap();
        w.write(&l).unwrap();
        w.close().unwrap();
        let r = parquet::file::reader::SerializedFileReader::new(std::fs::File::open(ruta).unwrap()).unwrap();
        use parquet::file::reader::FileReader;
        let c = r.metadata().file_metadata().schema_descr().column(1).clone();
        println!("{ruta}: tipo lógico {:?} · {} bytes", c.logical_type_ref(), std::fs::metadata(ruta).unwrap().len());
    }
}
