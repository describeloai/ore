//! ORE 0067 G2·1 · **El escritor de las tablas geo.**
//!
//! El escritor de `iceberg` deriva el esquema de Arrow del de Iceberg, y como
//! `iceberg-rust` 0.10 no tiene tipos geo, la marca `geoarrow.wkb` se pierde y
//! la geometría queda como un binario sin tipo ni estadísticas (0066 T1). Para
//! las tablas que llevan una columna geo los ficheros de datos los escribe ORE
//! (la decisión B de 0067):
//!
//! - **el tipo lógico GEOMETRY/GEOGRAPHY de Parquet**, con su bbox por grupo de
//!   filas: lo pone `parquet` (*feature* `geospatial`) desde la marca del campo;
//! - **la clave GeoParquet `geo`** del fichero (1.1: columna, `encoding: WKB`,
//!   tipos de geometría, bbox, CRS en PROJJSON y `edges`), que es lo que leen
//!   GeoPandas y los lectores anteriores al tipo lógico (0066 T5);
//! - **el CRS explícito en las dos**, también cuando es el de por defecto.
//!
//! Los ficheros se registran en la tabla Iceberg (v3, G2·0) como `binary`, con
//! la propiedad `ore.geo.<columna>`. El día que `iceberg-rust` traiga el tipo,
//! la columna pasa a `geometry` solo en metadatos (G2·0): estos ficheros ya son
//! los definitivos.

use arrow_array::RecordBatch;
use arrow_schema::{Field, Schema as ArrowSchema};
use iceberg::spec::{DataContentType, DataFile, DataFileBuilder, DataFileFormat, Schema, Struct};
use parquet::file::metadata::ParquetMetaData;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, OnceLock};

/// La propiedad de tabla que guarda el tipo de una columna geo mientras
/// Iceberg no lo tenga: `ore.geo.<columna>` = `{"type": …, "crs": …}`.
pub const PROP_GEO: &str = "ore.geo.";

/// Una columna geo de un lote: lo que dice su campo.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnaGeo {
    pub nombre: String,
    /// Los metadatos de la extensión `geoarrow.wkb` (`{"crs": …}`).
    pub extension: String,
    /// El tipo de OOS, si el lote lo trae (`carga::campo` lo pone).
    pub tipo: Option<String>,
    pub crs: String,
    pub esferica: bool,
}

/// Las columnas geo de un esquema de Arrow: las que llevan la extensión
/// `geoarrow.wkb`.
pub fn columnas_geo(esquema: &ArrowSchema) -> Vec<ColumnaGeo> {
    esquema
        .fields()
        .iter()
        .filter(|f| {
            f.metadata().get("ARROW:extension:name").map(String::as_str)
                == Some(crate::carga::GEOARROW_WKB)
        })
        .map(|f| {
            let extension = f
                .metadata()
                .get("ARROW:extension:metadata")
                .cloned()
                .unwrap_or_default();
            let j: serde_json::Value = serde_json::from_str(&extension).unwrap_or_default();
            ColumnaGeo {
                nombre: f.name().clone(),
                tipo: f.metadata().get(crate::carga::META_TIPO).cloned(),
                crs: j["crs"]
                    .as_str()
                    .unwrap_or(ore_core::types::CRS_DEFECTO)
                    .to_string(),
                esferica: j["edges"].as_str() == Some("spherical"),
                extension,
            }
        })
        .collect()
}

/// El valor de la propiedad `ore.geo.<columna>`.
pub fn propiedad(c: &ColumnaGeo) -> (String, String) {
    let mut m = serde_json::Map::new();
    if let Some(t) = &c.tipo {
        m.insert("type".into(), t.clone().into());
    }
    m.insert("crs".into(), c.crs.clone().into());
    if c.esferica {
        m.insert("edges".into(), "spherical".into());
    }
    (
        format!("{PROP_GEO}{}", c.nombre),
        serde_json::Value::Object(m).to_string(),
    )
}

/// El PROJJSON de un CRS por su código, de la tabla que viaja con el binario
/// (`crs_projjson.json`, generada con pyproj; ver su `_fuente`). `None` para un
/// código que no está: la clave GeoParquet lo dice entonces como `null`
/// («desconocido») y el tipo lógico y la propiedad conservan el código.
pub fn projjson(crs: &str) -> Option<serde_json::Value> {
    static TABLA: OnceLock<serde_json::Map<String, serde_json::Value>> = OnceLock::new();
    TABLA
        .get_or_init(|| {
            serde_json::from_str(include_str!("crs_projjson.json"))
                .expect("crs_projjson.json se lee")
        })
        .get(crs)
        .cloned()
}

/// Lo que la clave `geo` dice de una columna, acumulado mientras se escribe.
#[derive(Default)]
struct Resumen {
    tipos: BTreeSet<String>,
    bbox: Option<[f64; 4]>,
}

/// **Un fichero de datos geo en curso.** En memoria, como el de `iceberg`
/// (`Escritura`): se sube entero al cerrarlo.
pub struct FicheroGeo {
    escritor: parquet::arrow::ArrowWriter<Vec<u8>>,
    esquema: Arc<ArrowSchema>,
    geo: Vec<ColumnaGeo>,
    resumen: BTreeMap<String, Resumen>,
    filas: u64,
}

impl FicheroGeo {
    /// `esquema` es el de la tabla en Arrow (con sus `PARQUET:field_id`); las
    /// columnas de `geo` recuperan aquí su marca, que es lo que hace que
    /// `parquet` escriba el tipo lógico.
    pub fn nuevo(esquema: &ArrowSchema, geo: &[ColumnaGeo]) -> Result<FicheroGeo, String> {
        let campos: Vec<Field> = esquema
            .fields()
            .iter()
            .map(|f| match geo.iter().find(|g| g.nombre == *f.name()) {
                Some(g) => {
                    let mut m = f.metadata().clone();
                    m.insert(
                        "ARROW:extension:name".into(),
                        crate::carga::GEOARROW_WKB.into(),
                    );
                    m.insert("ARROW:extension:metadata".into(), g.extension.clone());
                    f.as_ref().clone().with_metadata(m)
                }
                None => f.as_ref().clone(),
            })
            .collect();
        let esquema = Arc::new(ArrowSchema::new_with_metadata(
            campos,
            esquema.metadata().clone(),
        ));
        let props = parquet::file::properties::WriterProperties::builder()
            .set_compression(parquet::basic::Compression::SNAPPY)
            .build();
        let escritor =
            parquet::arrow::ArrowWriter::try_new(Vec::new(), esquema.clone(), Some(props))
                .map_err(|e| format!("el fichero geo no se pudo abrir: {e}"))?;
        Ok(FicheroGeo {
            escritor,
            esquema,
            geo: geo.to_vec(),
            resumen: BTreeMap::new(),
            filas: 0,
        })
    }

    /// Un lote ya en el esquema de la tabla (las columnas de `geo`, binarias).
    pub fn escribir(&mut self, lote: &RecordBatch) -> Result<(), String> {
        use arrow_array::Array;
        use arrow_array::cast::AsArray;
        for g in &self.geo {
            let Some(col) = lote.column_by_name(&g.nombre) else {
                continue;
            };
            let r = self.resumen.entry(g.nombre.clone()).or_default();
            let mut ver = |b: &[u8]| {
                if let Ok(w) = ore_core::geo::leer(b) {
                    r.tipos.insert(w.subtipo());
                    if let Some([x0, y0, x1, y1]) = w.bbox {
                        let c = r.bbox.get_or_insert([x0, y0, x1, y1]);
                        c[0] = c[0].min(x0);
                        c[1] = c[1].min(y0);
                        c[2] = c[2].max(x1);
                        c[3] = c[3].max(y1);
                    }
                }
            };
            if let Some(a) = col.as_binary_opt::<i32>() {
                (0..a.len())
                    .filter(|i| !a.is_null(*i))
                    .for_each(|i| ver(a.value(i)));
            } else if let Some(a) = col.as_binary_opt::<i64>() {
                (0..a.len())
                    .filter(|i| !a.is_null(*i))
                    .for_each(|i| ver(a.value(i)));
            }
        }
        let lote = RecordBatch::try_new(self.esquema.clone(), lote.columns().to_vec())
            .map_err(|e| format!("el lote no casa con el fichero geo: {e}"))?;
        self.escritor
            .write(&lote)
            .map_err(|e| format!("el fichero geo no se pudo escribir: {e}"))?;
        self.filas += lote.num_rows() as u64;
        Ok(())
    }

    /// Lo que ocupa ya el fichero (para partirlo a su tamaño objetivo).
    pub fn tamano(&self) -> usize {
        self.escritor.bytes_written() + self.escritor.in_progress_size()
    }

    pub fn filas(&self) -> u64 {
        self.filas
    }

    /// Cierra el fichero con su clave `geo`: los bytes y los metadatos de Parquet.
    pub fn cerrar(mut self) -> Result<(Vec<u8>, ParquetMetaData), String> {
        let clave = clave_geo(&self.geo, &self.resumen);
        self.escritor
            .append_key_value_metadata(parquet::file::metadata::KeyValue::new(
                "geo".to_string(),
                clave,
            ));
        let meta = self
            .escritor
            .finish()
            .map_err(|e| format!("el fichero geo no se pudo cerrar: {e}"))?;
        // `finish` ya cerró el fichero: los bytes están en el búfer.
        Ok((std::mem::take(self.escritor.inner_mut()), meta))
    }
}

/// **La clave GeoParquet `geo`** (1.1.0) de un fichero.
fn clave_geo(geo: &[ColumnaGeo], resumen: &BTreeMap<String, Resumen>) -> String {
    let mut columnas = serde_json::Map::new();
    for g in geo {
        let r = resumen.get(&g.nombre);
        let mut c = serde_json::Map::new();
        c.insert("encoding".into(), "WKB".into());
        let tipos: Vec<serde_json::Value> = r
            .map(|r| r.tipos.iter().map(|t| t.clone().into()).collect())
            .unwrap_or_default();
        c.insert("geometry_types".into(), tipos.into());
        // El CRS siempre explícito (0066 T5), en PROJJSON; `null` si no está en la tabla.
        c.insert(
            "crs".into(),
            projjson(&g.crs).unwrap_or(serde_json::Value::Null),
        );
        if g.esferica {
            c.insert("edges".into(), "spherical".into());
        }
        if let Some(b) = r.and_then(|r| r.bbox) {
            c.insert(
                "bbox".into(),
                b.iter()
                    .map(|x| serde_json::json!(x))
                    .collect::<Vec<_>>()
                    .into(),
            );
        }
        columnas.insert(g.nombre.clone(), c.into());
    }
    serde_json::json!({
        "version": "1.1.0",
        "primary_column": geo.first().map(|g| g.nombre.clone()).unwrap_or_default(),
        "columns": columnas,
    })
    .to_string()
}

/// **El `DataFile` de un fichero escrito aquí**, con las métricas que se
/// sacan de sus metadatos de Parquet: filas, y tamaño, valores y nulos por
/// columna (por el id de campo de la tabla). Sin cotas inferior y superior:
/// las del manifiesto son opcionales, y las estadísticas por grupo de filas
/// siguen en el fichero (para un escalar) o son el bbox (para la geometría).
pub fn data_file(
    meta: &ParquetMetaData,
    ruta: String,
    bytes: u64,
    esquema: &Schema,
    spec_id: i32,
) -> Result<DataFile, String> {
    let mut tamanos: HashMap<i32, u64> = HashMap::new();
    let mut valores: HashMap<i32, u64> = HashMap::new();
    let mut nulos: HashMap<i32, u64> = HashMap::new();
    for g in meta.row_groups() {
        for c in g.columns() {
            let ruta_col = c.column_descr().path().parts();
            let Some(campo) = ruta_col.first().and_then(|n| esquema.field_by_name(n)) else {
                continue;
            };
            // Solo las columnas de primer nivel sin anidar: un hijo no es su columna.
            if ruta_col.len() != 1 {
                continue;
            }
            *tamanos.entry(campo.id).or_default() += c.compressed_size() as u64;
            *valores.entry(campo.id).or_default() += c.num_values() as u64;
            if let Some(n) = c.statistics().and_then(|s| s.null_count_opt()) {
                *nulos.entry(campo.id).or_default() += n;
            }
        }
    }
    DataFileBuilder::default()
        .content(DataContentType::Data)
        .file_path(ruta)
        .file_format(DataFileFormat::Parquet)
        .partition(Struct::empty())
        .partition_spec_id(spec_id)
        .record_count(meta.file_metadata().num_rows() as u64)
        .file_size_in_bytes(bytes)
        .column_sizes(tamanos)
        .value_counts(valores)
        .null_value_counts(nulos)
        .split_offsets(Some(
            meta.row_groups()
                .iter()
                .filter_map(|g| g.file_offset())
                .collect(),
        ))
        .build()
        .map_err(|e| format!("el DataFile no se construye: {e}"))
}
