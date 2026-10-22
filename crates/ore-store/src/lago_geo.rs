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
    /// Su columna de cobertura (`<col>_bbox`), si el lote la trae ([`con_cobertura`]).
    pub cobertura: Option<String>,
}

/// **El sufijo de la columna de cobertura** (ORE 0072 F1·1): `<col>_bbox`.
pub const SUFIJO_COBERTURA: &str = "_bbox";

/// La columna de cobertura de una columna geo.
pub fn nombre_cobertura(columna: &str) -> String {
    format!("{columna}{SUFIJO_COBERTURA}")
}

/// **La cobertura** (ORE 0072 F1·1, GeoParquet 1.1 *covering*): junto a cada
/// columna geo, `<col>_bbox`, una estructura `{xmin, ymin, xmax, ymax}` con la
/// caja de cada valor (nula si el valor es nulo o vacío). Iceberg no conoce la
/// geometría (`binary`) y quien lee por `iceberg_scan` (el SDK, Spark) no puede
/// podar por ella; por estas cuatro columnas sí: medido en F1·0, la tesela de
/// Madrid sobre 20 M de puntos pasa de ~660 ms a ~126 ms. La calcula siempre
/// ORE, aunque el lote la traiga: no puede quedar desalineada con la geometría.
/// Va visible, como el `bbox` de Overture, y la marca `ore.geo.<col>` la dice
/// derivada (`covering`).
pub fn con_cobertura(lote: RecordBatch) -> Result<RecordBatch, String> {
    use arrow_array::cast::AsArray;
    use arrow_array::{Array, ArrayRef, Float64Array, StructArray};
    use arrow_schema::DataType;
    let geo = columnas_geo(&lote.schema());
    if geo.is_empty() {
        return Ok(lote);
    }
    let caja_campos = || -> arrow_schema::Fields {
        ["xmin", "ymin", "xmax", "ymax"]
            .iter()
            .map(|n| Field::new(*n, DataType::Float64, true))
            .collect()
    };
    let esquema = lote.schema();
    let mut campos: Vec<Arc<Field>> = Vec::new();
    let mut columnas: Vec<ArrayRef> = Vec::new();
    let destino: Vec<String> = geo.iter().map(|g| nombre_cobertura(&g.nombre)).collect();
    // La que traiga el lote con ese nombre se recalcula: la de ORE manda.
    for (i, f) in esquema.fields().iter().enumerate() {
        if !destino.contains(f.name()) {
            campos.push(f.clone());
            columnas.push(lote.column(i).clone());
        }
    }
    // Las coberturas, al final: donde Iceberg pone una columna nueva, y en el
    // orden de una tabla que ya las tiene (los escritores casan por posición).
    for g in &geo {
        let Some(col) = lote.column_by_name(&g.nombre) else {
            continue;
        };
        let n = col.len();
        let mut v: [Vec<Option<f64>>; 4] = std::array::from_fn(|_| Vec::with_capacity(n));
        let mut validos = Vec::with_capacity(n);
        for k in 0..n {
            let b = if col.is_null(k) {
                None
            } else {
                col.as_binary_opt::<i32>()
                    .map(|a| a.value(k))
                    .or_else(|| col.as_binary_opt::<i64>().map(|a| a.value(k)))
            };
            let caja = b
                .and_then(|b| ore_core::geo::leer(b).ok())
                .and_then(|w| w.bbox);
            validos.push(caja.is_some());
            for (j, x) in v.iter_mut().enumerate() {
                x.push(caja.map(|c| c[j]));
            }
        }
        let hijos: Vec<ArrayRef> = v
            .into_iter()
            .map(|x| Arc::new(Float64Array::from(x)) as ArrayRef)
            .collect();
        let caja = StructArray::try_new(
            caja_campos(),
            hijos,
            Some(arrow_buffer::NullBuffer::from(validos)),
        )
        .map_err(|e| format!("la cobertura de `{}` no se construye: {e}", g.nombre))?;
        campos.push(Arc::new(Field::new(
            nombre_cobertura(&g.nombre),
            DataType::Struct(caja_campos()),
            true,
        )));
        columnas.push(Arc::new(caja));
    }
    RecordBatch::try_new(
        Arc::new(ArrowSchema::new_with_metadata(
            campos,
            esquema.metadata().clone(),
        )),
        columnas,
    )
    .map_err(|e| format!("el lote con su cobertura no se construye: {e}"))
}

/// **El tamaño del grupo de filas de un fichero geo** (ORE 0067 G2·2, medido
/// con V4 sobre los 34 M de edificios en victor): con orden Hilbert, 100 000
/// filas descartan el 98,5–99,4 % de los grupos en una consulta por ciudad, con
/// la misma latencia que 25 000 (p95 0,6–0,7 s con un lector) y la cuarta parte
/// de metadatos; 400 000 es más lento y no cabe en memoria con lectores
/// concurrentes.
pub const GRUPO_DE_FILAS: usize = 100_000;

/// **El orden Hilbert** (ORE 0067 G2·2): las filas, ordenadas por el índice de
/// Hilbert del centro del bbox de `columna` en una rejilla de 2¹⁶ × 2¹⁶ sobre la
/// extensión de los datos. Es lo que hace que el bbox de cada grupo de filas
/// sea pequeño, y con él el descarte: medido en V4, de un 87–97 % sin orden a
/// un 98,5–99,8 % con él, y la consulta por zona de 2,3 s a 0,6 s (p95). Es
/// global: los ficheros en que luego se parte salen con rangos espaciales
/// contiguos. Las nulas y las vacías van al final.
pub fn ordenar_hilbert(lotes: Vec<RecordBatch>, columna: &str) -> Result<Vec<RecordBatch>, String> {
    use arrow_array::Array;
    use arrow_array::cast::AsArray;
    let Some(primero) = lotes.first() else {
        return Ok(lotes);
    };
    let esquema = primero.schema();
    let todo = arrow_select::concat::concat_batches(&esquema, &lotes)
        .map_err(|e| format!("los lotes no se juntan para ordenar: {e}"))?;
    let Some(col) = todo.column_by_name(columna) else {
        return Ok(vec![todo]);
    };
    let valor = |i: usize| -> Option<&[u8]> {
        if col.is_null(i) {
            return None;
        }
        col.as_binary_opt::<i32>()
            .map(|a| a.value(i))
            .or_else(|| col.as_binary_opt::<i64>().map(|a| a.value(i)))
    };
    let centros: Vec<Option<(f64, f64)>> = (0..todo.num_rows())
        .map(|i| {
            valor(i)
                .and_then(|b| ore_core::geo::leer(b).ok())
                .and_then(|w| w.bbox)
                .map(|[x0, y0, x1, y1]| ((x0 + x1) / 2.0, (y0 + y1) / 2.0))
        })
        .collect();
    let mut ext = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for (x, y) in centros.iter().flatten() {
        ext = [
            ext[0].min(*x),
            ext[1].min(*y),
            ext[2].max(*x),
            ext[3].max(*y),
        ];
    }
    let celda = |v: f64, lo: f64, hi: f64| -> u32 {
        if hi > lo {
            (((v - lo) / (hi - lo)) * 65535.0)
                .round()
                .clamp(0.0, 65535.0) as u32
        } else {
            0
        }
    };
    let claves: Vec<u64> = centros
        .iter()
        .map(|c| match c {
            Some((x, y)) => hilbert(celda(*x, ext[0], ext[2]), celda(*y, ext[1], ext[3])),
            None => u64::MAX,
        })
        .collect();
    let mut orden: Vec<u32> = (0..todo.num_rows() as u32).collect();
    orden.sort_by_key(|i| claves[*i as usize]);
    let indices = arrow_array::UInt32Array::from(orden);
    let ordenado = arrow_select::take::take_record_batch(&todo, &indices)
        .map_err(|e| format!("las filas no se reordenan: {e}"))?;
    Ok(vec![ordenado])
}

/// El índice de Hilbert de la celda `(x, y)` en una rejilla de 2¹⁶ × 2¹⁶ (el
/// algoritmo clásico de rotar y acumular por cuadrantes).
fn hilbert(mut x: u32, mut y: u32) -> u64 {
    const N: u32 = 1 << 16;
    let mut d: u64 = 0;
    let mut s = N / 2;
    while s > 0 {
        let rx = u32::from(x & s > 0);
        let ry = u32::from(y & s > 0);
        d += u64::from(s) * u64::from(s) * u64::from((3 * rx) ^ ry);
        if ry == 0 {
            if rx == 1 {
                x = N - 1 - x;
                y = N - 1 - y;
            }
            std::mem::swap(&mut x, &mut y);
        }
        s /= 2;
    }
    d
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
                cobertura: esquema
                    .field_with_name(&nombre_cobertura(f.name()))
                    .ok()
                    .filter(|c| matches!(c.data_type(), arrow_schema::DataType::Struct(_)))
                    .map(|c| c.name().clone()),
                extension,
            }
        })
        .collect()
}

/// El valor de la propiedad `ore.geo.<columna>`.
pub fn propiedad(c: &ColumnaGeo) -> (String, String) {
    // ⛔ Las claves en orden alfabético AL INSERTAR: con `preserve_order` de
    //   serde_json (lo enciende `bson`, de 0069, al compilar el workspace junto)
    //   el mapa guarda el orden de inserción; sin él, los ordena. Así sale el
    //   mismo texto de las dos maneras (el CI lo cazó el 2026-10-20).
    let mut m = serde_json::Map::new();
    // F1·1: la columna de cobertura, derivada de ésta (la escribe ORE).
    if let Some(b) = &c.cobertura {
        m.insert("covering".into(), b.clone().into());
    }
    m.insert("crs".into(), c.crs.clone().into());
    if c.esferica {
        m.insert("edges".into(), "spherical".into());
    }
    if let Some(t) = &c.tipo {
        m.insert("type".into(), t.clone().into());
    }
    (
        format!("{PROP_GEO}{}", c.nombre),
        ordenado(serde_json::Value::Object(m)),
    )
}

/// **El JSON con las claves en orden**, sea cual sea la feature de `serde_json`.
///
/// Sin `preserve_order` el mapa ya va ordenado; con ella —y `bson`, que trae el
/// cliente de MongoDB, la enciende para todo el espacio de trabajo— sale en el
/// orden en que se insertó. Estos bytes se sellan en el lago y en el fichero
/// (`ore.geo.<col>`, la clave `geo`): que dependan de qué crate se compiló al lado
/// es lo que no puede pasar.
fn ordenado(v: serde_json::Value) -> String {
    fn rec(v: serde_json::Value) -> serde_json::Value {
        match v {
            serde_json::Value::Object(m) => {
                let mut e: Vec<_> = m.into_iter().map(|(k, v)| (k, rec(v))).collect();
                e.sort_by(|a, b| a.0.cmp(&b.0));
                serde_json::Value::Object(e.into_iter().collect())
            }
            serde_json::Value::Array(a) => {
                serde_json::Value::Array(a.into_iter().map(rec).collect())
            }
            otro => otro,
        }
    }
    rec(v).to_string()
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
            .set_max_row_group_row_count(Some(GRUPO_DE_FILAS))
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
        // GeoParquet 1.1: la columna de cobertura, para quien pode por ella.
        if let Some(b) = &g.cobertura {
            c.insert(
                "covering".into(),
                serde_json::json!({"bbox": {
                    "xmax": [b, "xmax"], "xmin": [b, "xmin"],
                    "ymax": [b, "ymax"], "ymin": [b, "ymin"],
                }}),
            );
        }
        columnas.insert(g.nombre.clone(), c.into());
    }
    ordenado(serde_json::json!({
        "version": "1.1.0",
        "primary_column": geo.first().map(|g| g.nombre.clone()).unwrap_or_default(),
        "columns": columnas,
    }))
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

/// La propiedad que deja la lente: la tabla se leyó con sus `geometry` /
/// `geography` vistas como `binary`, y no se escribe encima (ver [`lente`]).
pub const PROP_LENTE: &str = "ore.geo.lente";

/// ORE 0067 G2·4·1 · **La lente**: `iceberg` 0.10 no abre un `metadata.json`
/// v3 con `geometry` o `geography` (no deserializa: ni el esquema), sea de una
/// tabla ajena o de una de ORE ya promovida. Antes de dárselo, cada columna geo
/// de cada esquema pasa a `binary` con su mismo id, y la tabla gana la
/// propiedad `ore.geo.<col>` (tipo de OOS, CRS y `edges`) de las de primer
/// nivel del esquema actual, que es como ORE escribe las suyas: lo demás de ORE
/// ya la lee como geometría. Y [`PROP_LENTE`], que la deja **de solo lectura**
/// para ORE: escribir encima reescribiría un esquema ajeno con `binary`. Se
/// quita con `iceberg` 0.11 (PR 2933). Devuelve si cambió algo.
pub fn lente(meta: &mut serde_json::Value) -> bool {
    let actual = meta.get("current-schema-id").and_then(|v| v.as_i64());
    let mut vistas: Vec<(String, String)> = Vec::new();
    let mut cambio = false;
    if let Some(esquemas) = meta.get_mut("schemas").and_then(|v| v.as_array_mut()) {
        for e in esquemas {
            let es_actual = e.get("schema-id").and_then(|v| v.as_i64()) == actual;
            if let Some(campos) = e.get_mut("fields").and_then(|v| v.as_array_mut()) {
                for c in campos {
                    let nombre = c
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let Some(t) = c.get_mut("type") else { continue };
                    let (cambiado, prop) = binario_de(t);
                    cambio |= cambiado;
                    if let (Some(prop), true) = (prop, es_actual) {
                        vistas.push((nombre, prop));
                    }
                }
            }
        }
    }
    if !cambio {
        return false;
    }
    let props = meta
        .as_object_mut()
        .expect("un metadata.json es un objeto")
        .entry("properties")
        .or_insert_with(|| serde_json::json!({}));
    if let Some(p) = props.as_object_mut() {
        for (nombre, prop) in vistas {
            p.entry(format!("{PROP_GEO}{nombre}"))
                .or_insert(prop.into());
        }
        p.insert(
            PROP_LENTE.into(),
            "iceberg 0.10: geometry y geography leídas como binary".into(),
        );
    }
    true
}

/// Un tipo de Iceberg (también anidado: `struct`, `list`, `map`) con sus geo a
/// `binary`: si cambió algo y, de uno geo de primer nivel, el valor de
/// `ore.geo.<col>` (una geo dentro de una lista o un `struct` no tiene propiedad).
fn binario_de(t: &mut serde_json::Value) -> (bool, Option<String>) {
    match t {
        serde_json::Value::String(s) => match propiedad_de_iceberg(s) {
            Some(prop) => {
                *t = "binary".into();
                (true, Some(prop))
            }
            None => (false, None),
        },
        serde_json::Value::Object(o) => {
            let mut cambio = false;
            for k in ["element", "key", "value"] {
                if let Some(x) = o.get_mut(k) {
                    cambio |= binario_de(x).0;
                }
            }
            if let Some(cs) = o.get_mut("fields").and_then(|v| v.as_array_mut()) {
                for c in cs {
                    if let Some(x) = c.get_mut("type") {
                        cambio |= binario_de(x).0;
                    }
                }
            }
            (cambio, None)
        }
        _ => (false, None),
    }
}

/// `geometry`, `geometry(EPSG:25830)`, `geography`, `geography(C, vincenty)`:
/// la propiedad `ore.geo.<col>` que ORE escribiría para esa columna.
fn propiedad_de_iceberg(s: &str) -> Option<String> {
    let s = s.trim();
    let (ctor, resto) = if let Some(r) = s.strip_prefix("geometry") {
        ("Geometry", r)
    } else {
        ("Geography", s.strip_prefix("geography")?)
    };
    let args: Vec<String> = match resto.trim() {
        "" => vec![],
        r => r
            .strip_prefix('(')?
            .strip_suffix(')')?
            .split(',')
            .map(|a| a.trim().to_string())
            .collect(),
    };
    let crs = args
        .first()
        .filter(|c| !c.is_empty())
        .cloned()
        .unwrap_or_else(|| ore_core::types::CRS_DEFECTO.to_string());
    let mut m = serde_json::Map::new();
    let tipo = if ctor == "Geometry" && crs != ore_core::types::CRS_DEFECTO {
        format!("Geometry<Any, {crs}>")
    } else {
        ctor.to_string()
    };
    m.insert("type".into(), tipo.into());
    m.insert("crs".into(), crs.into());
    if ctor == "Geography" {
        // El algoritmo de las aristas (`spherical` por defecto en la spec).
        let a = args.get(1).map(String::as_str).unwrap_or("spherical");
        m.insert("edges".into(), a.into());
    }
    Some(serde_json::Value::Object(m).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// La curva visita las cuatro esquinas de la rejilla en su orden, y dos
    /// celdas vecinas tienen índices cercanos (lo que hace pequeño el bbox de
    /// un grupo de filas).
    #[test]
    fn la_curva_de_hilbert() {
        let m = 65535;
        assert_eq!(hilbert(0, 0), 0);
        assert!(hilbert(0, m) < hilbert(m, m) && hilbert(m, m) < hilbert(m, 0));
        assert_eq!(hilbert(1, 0).abs_diff(hilbert(0, 0)), 1);
    }

    /// El orden junta lo cercano: tres puntos de Madrid y tres de Barcelona,
    /// mezclados, salen agrupados por ciudad; una nula va al final.
    #[test]
    fn el_orden_hilbert_junta_lo_cercano() {
        use arrow_array::{Array, BinaryArray, Int64Array};
        use arrow_schema::{DataType, Field};
        let p = |x: f64, y: f64| {
            let mut w = vec![1u8, 1, 0, 0, 0];
            w.extend_from_slice(&x.to_le_bytes());
            w.extend_from_slice(&y.to_le_bytes());
            w
        };
        let geoms = [
            Some(p(-3.70, 40.41)),
            Some(p(2.17, 41.38)),
            None,
            Some(p(-3.71, 40.42)),
            Some(p(2.16, 41.39)),
            Some(p(-3.69, 40.40)),
            Some(p(2.18, 41.40)),
        ];
        let esquema = Arc::new(ArrowSchema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("geom", DataType::Binary, true),
        ]));
        let lote = RecordBatch::try_new(
            esquema,
            vec![
                Arc::new(Int64Array::from_iter_values(0..7)),
                Arc::new(BinaryArray::from_iter(geoms.iter().map(|g| g.as_deref()))),
            ],
        )
        .unwrap();
        let o = ordenar_hilbert(vec![lote.slice(0, 4), lote.slice(4, 3)], "geom").unwrap();
        let ids: Vec<i64> = o[0]
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap()
            .values()
            .to_vec();
        let madrid = [0i64, 3, 5];
        let ciudad: Vec<bool> = ids[..6].iter().map(|i| madrid.contains(i)).collect();
        // Seis con geometría: tres de una ciudad seguidos de tres de la otra.
        assert!(
            ciudad == [true, true, true, false, false, false]
                || ciudad == [false, false, false, true, true, true],
            "{ids:?}"
        );
        assert_eq!(ids[6], 2, "la nula, al final");
        assert!(o[0].column(1).is_null(6));
    }

    /// Un lote con una columna geo marcada (`geoarrow.wkb`): un punto, una
    /// línea, una nula, y `nombre` para ver que lo demás no se toca.
    fn lote_geo(con_cobertura_ajena: bool) -> RecordBatch {
        use arrow_array::{ArrayRef, BinaryArray, Float64Array, StringArray, StructArray};
        use arrow_schema::DataType;
        let punto = |x: f64, y: f64| {
            let mut w = vec![1u8, 1, 0, 0, 0];
            w.extend_from_slice(&x.to_le_bytes());
            w.extend_from_slice(&y.to_le_bytes());
            w
        };
        let mut linea = vec![1u8, 2, 0, 0, 0, 2, 0, 0, 0];
        for v in [1.0f64, 5.0, 3.0, 2.0] {
            linea.extend_from_slice(&v.to_le_bytes());
        }
        let geoms = [Some(punto(-3.7, 40.4)), Some(linea), None];
        let marca = std::collections::HashMap::from([
            (
                "ARROW:extension:name".to_string(),
                crate::carga::GEOARROW_WKB.to_string(),
            ),
            (
                "ARROW:extension:metadata".to_string(),
                r#"{"crs":"OGC:CRS84"}"#.to_string(),
            ),
        ]);
        let mut campos = vec![
            Field::new("geom", DataType::Binary, true).with_metadata(marca),
            Field::new("nombre", DataType::Utf8, true),
        ];
        let mut columnas: Vec<ArrayRef> = vec![
            Arc::new(BinaryArray::from_iter(geoms.iter().map(|g| g.as_deref()))),
            Arc::new(StringArray::from(vec!["a", "b", "c"])),
        ];
        if con_cobertura_ajena {
            // Una `geom_bbox` que trae quien escribe, y mal: la de ORE manda.
            let f: arrow_schema::Fields = ["xmin", "ymin", "xmax", "ymax"]
                .iter()
                .map(|n| Field::new(*n, DataType::Float64, true))
                .collect();
            let hijos: Vec<ArrayRef> = (0..4)
                .map(|_| Arc::new(Float64Array::from(vec![9.0, 9.0, 9.0])) as ArrayRef)
                .collect();
            campos.push(Field::new("geom_bbox", DataType::Struct(f.clone()), true));
            columnas.push(Arc::new(StructArray::new(f, hijos, None)));
        }
        RecordBatch::try_new(Arc::new(ArrowSchema::new(campos)), columnas).unwrap()
    }

    fn caja(l: &RecordBatch, fila: usize) -> Option<[f64; 4]> {
        use arrow_array::cast::AsArray;
        use arrow_array::{Array, Float64Array};
        let s = l.column_by_name("geom_bbox").unwrap().as_struct();
        if s.is_null(fila) {
            return None;
        }
        let v = |n: &str| {
            s.column_by_name(n)
                .unwrap()
                .as_any()
                .downcast_ref::<Float64Array>()
                .unwrap()
                .value(fila)
        };
        Some([v("xmin"), v("ymin"), v("xmax"), v("ymax")])
    }

    /// ORE 0072 F1·1 · La cobertura va al final del lote, con la caja
    /// de cada valor (la de la línea, su extensión; la de la nula, nula), y lo
    /// demás no se toca.
    #[test]
    fn la_cobertura_es_la_caja_de_cada_valor() {
        let l = con_cobertura(lote_geo(false)).unwrap();
        let nombres: Vec<String> = l
            .schema()
            .fields()
            .iter()
            .map(|f| f.name().clone())
            .collect();
        assert_eq!(nombres, ["geom", "nombre", "geom_bbox"]);
        assert_eq!(caja(&l, 0), Some([-3.7, 40.4, -3.7, 40.4]));
        assert_eq!(caja(&l, 1), Some([1.0, 2.0, 3.0, 5.0]));
        assert_eq!(caja(&l, 2), None);
        // La marca: `columnas_geo` la ve, y la propiedad la dice derivada.
        let g = columnas_geo(&l.schema());
        assert_eq!(g[0].cobertura.as_deref(), Some("geom_bbox"));
        assert!(propiedad(&g[0]).1.contains(r#""covering":"geom_bbox""#));
    }

    /// La que trae quien escribe no vale: ORE la recalcula y no se duplica.
    #[test]
    fn la_cobertura_de_fuera_se_recalcula() {
        let l = con_cobertura(lote_geo(true)).unwrap();
        let nombres: Vec<String> = l
            .schema()
            .fields()
            .iter()
            .map(|f| f.name().clone())
            .collect();
        assert_eq!(nombres, ["geom", "nombre", "geom_bbox"]);
        assert_eq!(caja(&l, 0), Some([-3.7, 40.4, -3.7, 40.4]));
    }

    /// Sin columna geo, el lote sale tal cual; y la clave GeoParquet dice la
    /// cobertura (1.1 `covering`).
    #[test]
    fn sin_geo_no_hay_cobertura_y_geoparquet_la_dice() {
        use arrow_array::Int64Array;
        use arrow_schema::DataType;
        let l = RecordBatch::try_new(
            Arc::new(ArrowSchema::new(vec![Field::new(
                "id",
                DataType::Int64,
                false,
            )])),
            vec![Arc::new(Int64Array::from(vec![1]))],
        )
        .unwrap();
        assert_eq!(con_cobertura(l.clone()).unwrap(), l);
        let g = columnas_geo(&con_cobertura(lote_geo(false)).unwrap().schema());
        let clave: serde_json::Value =
            serde_json::from_str(&clave_geo(&g, &BTreeMap::new())).unwrap();
        assert_eq!(
            clave["columns"]["geom"]["covering"]["bbox"]["xmin"],
            serde_json::json!(["geom_bbox", "xmin"])
        );
    }
}
