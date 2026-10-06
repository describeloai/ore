//! **La muestra de un dataset: una página, sin bajar la tabla** (el preview
//! de un activo del catálogo: lo que `select * from x limit N offset D` daría
//! en el editor, servido sin puesto).
//!
//! `leer` y `pagina` bajan cada fichero de datos entero y cortan después:
//! medido en victor (P0 del preview, 2026-10-06), 100 filas de un dataset de
//! 2·10⁶ (54 MB, un fichero) eran 10 s y 1,8 GB de memoria — en `ore-serve`,
//! con 512 MiB, un OOM. Aquí se lee **lo que la página toca**:
//!
//! 1. **Los ficheros que quedan antes de `desde` no se abren**: el manifiesto
//!    dice cuántas filas tiene cada uno (`record_count`).
//! 2. **Del fichero, por rangos**: el pie y el índice de páginas (ORE escribe
//!    con parquet-rs, que deja `offset index` y `column index`), y de cada
//!    columna sólo las páginas que la selección toca. Un *row group* de ORE
//!    son 10⁶ filas (~28 MB): cortar por grupo no bastaba.
//! 3. **El orden es el del snapshot** (sus manifiestos, en orden), y la página
//!    dice qué snapshot leyó: la siguiente se pide sobre el mismo y no se
//!    mueve aunque alguien escriba entre medias.
//!
//! Un fichero con posiciones borradas (lo que deja DuckDB o Spark con
//! merge-on-read) se lee entero, como siempre: saber qué fila es la `desde`
//! exige saber cuáles no están. Lo de ORE es copy-on-write y no los deja.

use crate::almacen::Almacen;
use crate::carga::{self, Fila};
use crate::lago::Lago;
use bytes::{Buf, Bytes};
use iceberg::spec::{DataContentType, Snapshot};
use iceberg::table::Table;
use parquet::arrow::arrow_reader::{
    ArrowReaderOptions, ParquetRecordBatchReaderBuilder, RowSelection, RowSelector,
};
use parquet::errors::ParquetError;
use parquet::file::metadata::PageIndexPolicy;
use parquet::file::reader::{ChunkReader, Length};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Las columnas de una página, en orden: nombre y escalar OOS.
pub type Columnas = Vec<(String, String)>;

/// Lo que cabe en una página: es una respuesta para mirar, no una copia.
pub const LIMITE_MAXIMO: usize = 1_000;

/// Cuánto del final de un fichero se trae de una vez: el pie, sus metadatos y
/// el índice de páginas suelen caber, y así son una petición y no tres.
const COLA: u64 = 64 << 10;

/// Una página de un dataset.
#[derive(Debug)]
pub struct Muestra {
    /// Las columnas en el orden del esquema de la tabla, con su escalar OOS.
    pub columnas: Columnas,
    pub filas: Vec<Fila>,
    /// Las filas del snapshot (por sus manifiestos, como `historia`).
    pub total: u64,
    /// El snapshot leído; `None` si la tabla está vacía.
    pub snapshot: Option<i64>,
    /// Bytes pedidos al almacén para esta página (lo que la prueba mira).
    pub bytes_leidos: u64,
}

/// Un fichero del lago leído **por rangos**, para el lector de Parquet.
struct Rangos {
    cuenta: Arc<dyn Almacen>,
    clave: String,
    largo: u64,
    /// `(desde, bytes)`: el final del fichero, traído de una vez.
    cola: (u64, Bytes),
    leidos: Arc<AtomicU64>,
}

impl Rangos {
    fn abrir(
        cuenta: Arc<dyn Almacen>,
        clave: String,
        largo: u64,
        leidos: Arc<AtomicU64>,
    ) -> Result<Rangos, String> {
        let desde = largo.saturating_sub(COLA);
        let cola = if largo == 0 {
            Bytes::new()
        } else {
            let b = cuenta
                .leer_rango(&clave, Some((desde, largo - 1)))?
                .ok_or_else(|| format!("el fichero de datos `{clave}` no está en el almacén"))?;
            leidos.fetch_add(b.len() as u64, Ordering::Relaxed);
            Bytes::from(b)
        };
        Ok(Rangos {
            cuenta,
            clave,
            largo,
            cola: (desde, cola),
            leidos,
        })
    }
}

impl Length for Rangos {
    fn len(&self) -> u64 {
        self.largo
    }
}

impl ChunkReader for Rangos {
    type T = bytes::buf::Reader<Bytes>;

    fn get_read(&self, start: u64) -> parquet::errors::Result<Self::T> {
        let largo = self.largo.saturating_sub(start) as usize;
        Ok(self.get_bytes(start, largo)?.reader())
    }

    fn get_bytes(&self, start: u64, length: usize) -> parquet::errors::Result<Bytes> {
        if length == 0 {
            return Ok(Bytes::new());
        }
        let (desde, cola) = &self.cola;
        let fin = start + length as u64;
        if start >= *desde && fin <= desde + cola.len() as u64 {
            let a = (start - desde) as usize;
            return Ok(cola.slice(a..a + length));
        }
        let b = self
            .cuenta
            .leer_rango(&self.clave, Some((start, fin - 1)))
            .map_err(ParquetError::General)?
            .ok_or_else(|| {
                ParquetError::General(format!("`{}` ya no está en el almacén", self.clave))
            })?;
        if b.len() != length {
            return Err(ParquetError::General(format!(
                "`{}`: se pidieron {length} bytes desde {start} y llegaron {}",
                self.clave,
                b.len()
            )));
        }
        self.leidos.fetch_add(b.len() as u64, Ordering::Relaxed);
        Ok(Bytes::from(b))
    }
}

/// Un fichero de datos vivo del snapshot.
struct Fichero {
    ruta: String,
    filas: u64,
    bytes: u64,
}

/// Los ficheros de datos del snapshot **en su orden**, y las posiciones
/// borradas por fichero.
async fn ficheros(tabla: &Table, s: &Arc<Snapshot>) -> Result<(Vec<Fichero>, Vec<String>), String> {
    let io = tabla.file_io();
    let lista = tabla
        .manifest_list_reader(s)
        .load()
        .await
        .map_err(|e| e.to_string())?;
    let (mut datos, mut posiciones) = (Vec::new(), Vec::new());
    for mf in lista.entries() {
        let m = mf.load_manifest(io).await.map_err(|e| e.to_string())?;
        for e in m.entries().iter().filter(|e| e.is_alive()) {
            let d = e.data_file();
            match d.content_type() {
                DataContentType::Data => datos.push(Fichero {
                    ruta: e.file_path().to_string(),
                    filas: d.record_count(),
                    bytes: d.file_size_in_bytes(),
                }),
                DataContentType::PositionDeletes => posiciones.push(e.file_path().to_string()),
                DataContentType::EqualityDeletes => {
                    return Err(format!(
                        "`{}` es un equality delete y `ore-store` no lo aplica todavía: la tabla la \
                         escribió otro motor con merge-on-read; léela con DuckDB o reescríbela",
                        e.file_path()
                    ));
                }
            }
        }
    }
    Ok((datos, posiciones))
}

/// **Una página**: `limite` filas a partir de la `desde`-ésima, del snapshot
/// `snapshot` (o del vigente).
pub fn muestra(
    lago: &Lago,
    tabla: &Table,
    desde: u64,
    limite: usize,
    snapshot: Option<i64>,
) -> Result<Muestra, String> {
    let limite = limite.min(LIMITE_MAXIMO);
    let meta = tabla.metadata();
    let s = match snapshot {
        Some(id) => Some(meta.snapshot_by_id(id).cloned().ok_or_else(|| {
            format!("el snapshot `{id}` ya no está en la tabla: vuelve a la primera página")
        })?),
        None => meta.current_snapshot().cloned(),
    };
    let esquema = match &s {
        Some(s) => s
            .schema(meta)
            .map(|e| e.as_ref().clone())
            .unwrap_or_else(|_| meta.current_schema().as_ref().clone()),
        None => meta.current_schema().as_ref().clone(),
    };
    let columnas: Vec<(String, String)> = esquema
        .as_struct()
        .fields()
        .iter()
        .map(|f| (f.name.clone(), crate::lago::oos_de_iceberg(&f.field_type)))
        .collect();
    let Some(s) = s else {
        return Ok(Muestra {
            columnas,
            filas: Vec::new(),
            total: 0,
            snapshot: None,
            bytes_leidos: 0,
        });
    };
    let total = Lago::filas_de(tabla, &s);
    let (datos, posiciones) = crate::lago::runtime().block_on(ficheros(tabla, &s))?;
    let cuenta = lago.cuenta_publica()?;
    let leidos = Arc::new(AtomicU64::new(0));

    // Las posiciones borradas, por fichero de datos (como `Lago::filas`).
    let mut borradas: BTreeMap<String, BTreeSet<i64>> = BTreeMap::new();
    for ruta in posiciones {
        let k = lago.clave(&ruta).map_err(|e| e.to_string())?;
        let bytes = cuenta
            .leer_bytes(&k)?
            .ok_or_else(|| format!("el fichero de posiciones `{ruta}` no está en el almacén"))?;
        leidos.fetch_add(bytes.len() as u64, Ordering::Relaxed);
        for f in carga::leer(&bytes)? {
            if let (Some(fichero), Some(Ok(pos))) =
                (f.get("file_path"), f.get("pos").map(|p| p.parse::<i64>()))
            {
                borradas.entry(fichero.clone()).or_default().insert(pos);
            }
        }
    }

    let mut saltar = desde;
    let mut filas: Vec<Fila> = Vec::with_capacity(limite);
    for f in &datos {
        if filas.len() >= limite {
            break;
        }
        let quitadas = borradas.get(&f.ruta);
        let vivas = f.filas - quitadas.map_or(0, |q| q.len() as u64);
        // ① Lo que queda entero antes de `desde` no se abre.
        if saltar >= vivas {
            saltar -= vivas;
            continue;
        }
        let tomar = limite - filas.len();
        let clave = lago.clave(&f.ruta).map_err(|e| e.to_string())?;
        match quitadas {
            // Con posiciones borradas, entero: hay que saber cuáles faltan.
            Some(q) => {
                let bytes = cuenta.leer_bytes(&clave)?.ok_or_else(|| {
                    format!("el fichero de datos `{}` no está en el almacén", f.ruta)
                })?;
                leidos.fetch_add(bytes.len() as u64, Ordering::Relaxed);
                let mut todas = Vec::new();
                for l in carga::lotes_de_parquet(&bytes)? {
                    todas.extend(carga::filas_para_ver(&l));
                }
                filas.extend(
                    todas
                        .into_iter()
                        .enumerate()
                        .filter(|(i, _)| !q.contains(&(*i as i64)))
                        .map(|(_, f)| f)
                        .skip(saltar as usize)
                        .take(tomar),
                );
            }
            // ② Por rangos: sólo las páginas que la selección toca.
            None => {
                let r = Rangos::abrir(cuenta.clone(), clave, f.bytes, leidos.clone())?;
                filas.extend(pagina_del_fichero(r, saltar as usize, tomar, &f.ruta)?);
            }
        }
        saltar = 0;
    }
    Ok(Muestra {
        columnas,
        filas,
        total,
        snapshot: Some(s.snapshot_id()),
        bytes_leidos: leidos.load(Ordering::Relaxed),
    })
}

/// `tomar` filas desde la `saltar`-ésima de un Parquet, leyendo sólo los
/// grupos y las páginas que tocan.
fn pagina_del_fichero(
    r: Rangos,
    saltar: usize,
    tomar: usize,
    ruta: &str,
) -> Result<Vec<Fila>, String> {
    let malo = |e: ParquetError| format!("`{ruta}` no es un Parquet legible: {e}");
    // Sólo el `offset index`: dónde empieza cada página, que es lo que deja
    // saltarlas. El `column index` (mínimos y máximos) no hace falta aquí.
    let opciones = ArrowReaderOptions::new().with_offset_index_policy(PageIndexPolicy::Optional);
    let b = ParquetRecordBatchReaderBuilder::try_new_with_options(r, opciones).map_err(malo)?;
    let mut grupos = Vec::new();
    let mut seleccion = Vec::new();
    let (mut saltar, mut tomar) = (saltar, tomar);
    for (g, rg) in b.metadata().row_groups().iter().enumerate() {
        if tomar == 0 {
            break;
        }
        let n = rg.num_rows() as usize;
        if saltar >= n {
            saltar -= n;
            continue;
        }
        let t = (n - saltar).min(tomar);
        grupos.push(g);
        if saltar > 0 {
            seleccion.push(RowSelector::skip(saltar));
        }
        seleccion.push(RowSelector::select(t));
        if saltar + t < n {
            seleccion.push(RowSelector::skip(n - saltar - t));
        }
        saltar = 0;
        tomar -= t;
    }
    if grupos.is_empty() {
        return Ok(Vec::new());
    }
    let lector = b
        .with_row_groups(grupos)
        .with_row_selection(RowSelection::from(seleccion))
        .with_batch_size(LIMITE_MAXIMO)
        .build()
        .map_err(malo)?;
    let mut out = Vec::new();
    for l in lector {
        let l = l.map_err(|e| format!("un lote de `{ruta}` no se lee: {e}"))?;
        out.extend(carga::filas_para_ver(&l));
    }
    Ok(out)
}

/// **Las filas de un flujo Arrow** (lo que la pasarela devuelve al leer una
/// `Table` en vivo), saltando `desde` y hasta `limite`: la pasarela no sabe
/// `offset` y se le pide `desde + limite` (como `limit … offset` en SQL).
pub fn de_un_flujo(
    lector: impl std::io::Read,
    desde: u64,
    limite: usize,
) -> Result<(Columnas, Vec<Fila>, u64), String> {
    let limite = limite.min(LIMITE_MAXIMO);
    let flujo = arrow_ipc::reader::StreamReader::try_new(lector, None)
        .map_err(|e| format!("lo que llegó no es un flujo Arrow: {e}"))?;
    let columnas: Vec<(String, String)> = flujo
        .schema()
        .fields()
        .iter()
        .map(|c| (c.name().clone(), carga::oos_de_arrow(c.data_type())))
        .collect();
    let (mut saltar, mut vistas, mut filas) = (desde as usize, 0u64, Vec::new());
    for l in flujo {
        let l = l.map_err(|e| format!("un lote del flujo no se lee: {e}"))?;
        vistas += l.num_rows() as u64;
        if filas.len() >= limite {
            continue;
        }
        if saltar >= l.num_rows() {
            saltar -= l.num_rows();
            continue;
        }
        let quedan = limite - filas.len();
        let corte = l.slice(saltar, (l.num_rows() - saltar).min(quedan));
        saltar = 0;
        filas.extend(carga::filas_para_ver(&corte));
    }
    Ok((columnas, filas, vistas))
}
