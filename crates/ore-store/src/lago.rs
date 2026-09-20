//! **El lago: la copia como tabla Iceberg** (W3.6a, 0031 §10 «todo es un
//! dataset», 2026-09-20).
//!
//! Hasta aquí una copia era un sobre nuestro (`ORECOPY1`) nombrado por su
//! digest, y un recibo en el bucket decía cuál era la vigente. Desde W3.6a la
//! copia es **un dataset**: bytes en el bucket con historia —una tabla Iceberg
//! y sus snapshots—, un documento que los nombra (la `View`) y **un puntero de
//! estado en el árbol** (`copias/<p>_<v>.json`, con `metadata_location`). El
//! árbol es el catálogo: el commit del Job es el *swap* del puntero y la forja,
//! al rechazar lo que no avanza en línea recta, es el *compare-and-set* que un
//! catálogo de Iceberg hace por dentro. Lo que aquí queda es **escribir la
//! tabla y devolver el `metadata.json` nuevo**; quién lo apunta no es cosa de
//! este programa.
//!
//! # Lo que `iceberg` 0.10 trae y lo que se pone aquí
//!
//! Medido antes de entrar (`pruebas-de-fuego/medida-w3-iceberg-rust.py`): la
//! crate escribe ficheros de datos con sus estadísticas, manifiestos y listas
//! de manifiestos, y `fast_append` confirma un snapshot. **No trae**
//! sobrescribir (un snapshot que sustituye todos los ficheros), ni promover
//! tipos, ni un catálogo que sea *nuestro* árbol. Las tres cosas se hacen con
//! sus piezas públicas:
//!
//! - **sobrescribir** ([`Lago::instantanea`]): un manifiesto con los ficheros
//!   nuevos como `ADDED`, otro con los vivos del snapshot anterior como
//!   `DELETED`, la lista de manifiestos, el `Snapshot` con `Operation::
//!   Overwrite`, y `AddSnapshot` + `SetSnapshotRef` aplicados a los metadatos;
//! - **el esquema** ([`esquema_deseado`]): se construye entero a partir del
//!   lote, reusando el id de cada columna que conserva nombre y tipo y dando
//!   id nuevo a la que cambió de tipo (la forma legal de Iceberg para un cambio
//!   que no es promoción: la columna vieja se retira, la nueva nace). Como cada
//!   sobrescritura reescribe todos los datos, ningún fichero vivo lleva un id
//!   con dos tipos;
//! - **el catálogo**: no hay. Una tabla se abre por su `metadata_location`
//!   (`Table::builder`) y se confirma escribiendo el siguiente `metadata.json`
//!   ([`Lago::confirmar`]), que es exactamente lo que `MemoryCatalog::
//!   update_table` hace por dentro.
//!
//! # Lo que W3.6c añade: escribir en dos mitades, como el catálogo REST
//!
//! Un escritor de Iceberg —el nuestro o uno de fuera— escribe **los ficheros de
//! datos, los manifiestos y la lista** y manda al catálogo `requirements` +
//! `updates` (`add-snapshot`, `set-snapshot-ref`, …); **el catálogo escribe el
//! `metadata.json`**. Aquí eso son dos funciones: [`Lago::preparar`] (la
//! primera mitad: devuelve los cambios sin confirmar nada) y [`Lago::aplicar`]
//! (la segunda: valida, aplica y escribe el siguiente `metadata.json`, desde
//! una tabla o desde cero cuando el requisito es `assert-create`).
//! [`Lago::instantanea`] sigue siendo las dos seguidas, para la copia.
//!
//! # El suelo
//!
//! `iceberg` habla con el bucket a través de un `Storage`. Aquí ese `Storage`
//! **es el [`Almacen`] de siempre** (`gcs.rs`, `r2.rs`): el mismo token del
//! pod, el mismo SigV4, ningún transporte nuevo. Los ficheros de una tabla se
//! nombran por URI absoluta (`gs://<bucket>/ore/v2/…`), que es como cualquier
//! lector ajeno —DuckDB en el puesto, PyIceberg— los va a pedir.

use crate::almacen::Almacen;
use crate::carga;
use arrow_array::RecordBatch;
use arrow_schema::DataType;
use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::BoxStream;
use iceberg::io::{
    FileIO, FileIOBuilder, FileMetadata, FileRead, FileWrite, InputFile, OutputFile, Storage,
    StorageConfig, StorageFactory,
};
use iceberg::spec::{
    DataFile, DataFileFormat, MAIN_BRANCH, ManifestContentType, ManifestListWriter,
    ManifestWriterBuilder, NestedField, Operation, Schema, Snapshot, SnapshotReference,
    SnapshotRetention, SnapshotSummaryCollector, Summary, TableMetadata,
};
use iceberg::table::Table;
use iceberg::writer::base_writer::data_file_writer::DataFileWriterBuilder;
use iceberg::writer::file_writer::ParquetWriterBuilder;
use iceberg::writer::file_writer::location_generator::{
    DefaultFileNameGenerator, DefaultLocationGenerator,
};
use iceberg::writer::file_writer::rolling_writer::RollingFileWriterBuilder;
use iceberg::writer::{IcebergWriter, IcebergWriterBuilder};
use iceberg::{MetadataLocation, TableCreation, TableIdent, TableRequirement, TableUpdate};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ops::Range;
use std::str::FromStr;
use std::sync::{Arc, OnceLock};

/// El prefijo de todo dataset en el bucket. `v2` porque `ore/v1/` es el sobre
/// heredado, que `leer` sigue abriendo hasta que `recoger-huerfanas` lo retire.
pub const RAIZ: &str = "ore/v2";

/// Las propiedades con las que la tabla y cada snapshot dicen qué son. El sobre
/// llevaba la cabecera; el dataset la lleva aquí, y `leer` la devuelve tal cual.
pub const PROP_CABECERA: &str = "ore.cabecera";
pub const PROP_PLAN: &str = "ore.plan";
pub const PROP_TESTIGO_MODO: &str = "ore.testigo.modo";
pub const PROP_TESTIGO_VALOR: &str = "ore.testigo.valor";
pub const PROP_DATASET: &str = "ore.dataset";
/// **La clave de operación** (0031 §11 ④): quién escribió qué, como propiedad
/// del snapshot. `write()` la pone; el catálogo la coteja con la ancestría
/// antes de aplicar, y la misma operación dos veces no deja dos snapshots.
pub const PROP_OPERACION: &str = "ore.operacion";
/// **La retención, declarada en la tabla** (0031 §11 ⑥), con los nombres que
/// Iceberg usa para lo mismo: cuánto vive un snapshot superado y cuántos se
/// conservan como mínimo. Las lee `recoger`; sin ellas y sin `edad_ms`, no se
/// expira nada.
pub const PROP_RETENCION_EDAD: &str = "history.expire.max-snapshot-age-ms";
pub const PROP_RETENCION_MINIMO: &str = "history.expire.min-snapshots-to-keep";

fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("el runtime")
    })
}

fn err(e: iceberg::Error) -> String {
    e.to_string()
}

// ── El almacén como `Storage` de Iceberg ────────────────────────────────────

/// El [`Almacen`] con el traje de `Storage`. Se serializa vacío (typetag lo
/// exige) y nunca se deserializa: vive lo que dura un verbo.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Lago {
    #[serde(skip)]
    cuenta: Option<Arc<dyn Almacen>>,
    base: String,
}

impl std::fmt::Debug for Lago {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Lago({})", self.base)
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct Fabrica(Lago);

#[typetag::serde]
impl StorageFactory for Fabrica {
    fn build(&self, _config: &StorageConfig) -> iceberg::Result<Arc<dyn Storage>> {
        Ok(Arc::new(self.0.clone()))
    }
}

impl Lago {
    pub fn nuevo(cuenta: Arc<dyn Almacen>) -> Lago {
        let base = cuenta.base();
        Lago {
            cuenta: Some(cuenta),
            base,
        }
    }

    fn cuenta(&self) -> iceberg::Result<Arc<dyn Almacen>> {
        self.cuenta.clone().ok_or_else(|| {
            iceberg::Error::new(
                iceberg::ErrorKind::Unexpected,
                "el lago no tiene almacén debajo",
            )
        })
    }

    /// El almacén de debajo, para lo que no pasa por Iceberg (el sobre
    /// heredado, la enumeración de lo huérfano).
    pub fn cuenta_publica(&self) -> Result<Arc<dyn Almacen>, String> {
        self.cuenta().map_err(err)
    }

    /// La URI de una clave: `gs://<bucket>/<clave>`.
    pub fn uri(&self, clave: &str) -> String {
        format!("{}/{clave}", self.base)
    }

    /// La clave de una URI de este almacén. Una URI de otro sitio es un error
    /// dicho: los metadatos de una tabla no pueden apuntar fuera del bucket.
    pub fn clave(&self, uri: &str) -> iceberg::Result<String> {
        uri.strip_prefix(&self.base)
            .and_then(|r| r.strip_prefix('/'))
            .map(String::from)
            .ok_or_else(|| {
                iceberg::Error::new(
                    iceberg::ErrorKind::DataInvalid,
                    format!("`{uri}` no está en este almacén ({})", self.base),
                )
            })
    }

    pub fn file_io(&self) -> FileIO {
        FileIOBuilder::new(Arc::new(Fabrica(self.clone()))).build()
    }

    async fn bloqueante<T: Send + 'static>(
        &self,
        f: impl FnOnce(Arc<dyn Almacen>) -> Result<T, String> + Send + 'static,
    ) -> iceberg::Result<T> {
        let cuenta = self.cuenta()?;
        tokio::task::spawn_blocking(move || f(cuenta))
            .await
            .map_err(|e| iceberg::Error::new(iceberg::ErrorKind::Unexpected, e.to_string()))?
            .map_err(|e| iceberg::Error::new(iceberg::ErrorKind::Unexpected, e))
    }
}

#[async_trait]
#[typetag::serde]
impl Storage for Lago {
    async fn exists(&self, path: &str) -> iceberg::Result<bool> {
        let k = self.clave(path)?;
        self.bloqueante(move |c| c.existe(&k)).await
    }

    async fn metadata(&self, path: &str) -> iceberg::Result<FileMetadata> {
        let k = self.clave(path)?;
        let p = path.to_string();
        let size = self.bloqueante(move |c| c.tamano(&k)).await?;
        size.map(|size| FileMetadata { size }).ok_or_else(|| {
            iceberg::Error::new(iceberg::ErrorKind::DataInvalid, format!("no está: {p}"))
        })
    }

    async fn read(&self, path: &str) -> iceberg::Result<Bytes> {
        let k = self.clave(path)?;
        let p = path.to_string();
        let b = self.bloqueante(move |c| c.leer_bytes(&k)).await?;
        b.map(Bytes::from).ok_or_else(|| {
            iceberg::Error::new(iceberg::ErrorKind::DataInvalid, format!("no está: {p}"))
        })
    }

    /// Se baja entero y se sirve por rangos desde memoria: los ficheros de
    /// una copia se leen enteros siempre (fundir, `leer`), así que un rango
    /// nunca ahorra un byte y sí ahorraría un ida y vuelta por página.
    async fn reader(&self, path: &str) -> iceberg::Result<Box<dyn FileRead>> {
        Ok(Box::new(Lectura(self.read(path).await?)))
    }

    /// **Nunca reescribe**: los nombres de Iceberg llevan un UUID, así que un
    /// objeto que ya estaba es un error de programa y no una carrera benigna.
    async fn write(&self, path: &str, bs: Bytes) -> iceberg::Result<()> {
        let k = self.clave(path)?;
        let p = path.to_string();
        let subido = self.bloqueante(move |c| c.subir(&k, &bs)).await?;
        if subido {
            Ok(())
        } else {
            Err(iceberg::Error::new(
                iceberg::ErrorKind::Unexpected,
                format!("`{p}` ya existía y no se reescribe"),
            ))
        }
    }

    async fn writer(&self, path: &str) -> iceberg::Result<Box<dyn FileWrite>> {
        Ok(Box::new(Escritura {
            lago: self.clone(),
            path: path.to_string(),
            buffer: Vec::new(),
            cerrado: false,
        }))
    }

    async fn delete(&self, path: &str) -> iceberg::Result<()> {
        let k = self.clave(path)?;
        self.bloqueante(move |c| c.borrar(&k)).await
    }

    async fn delete_prefix(&self, path: &str) -> iceberg::Result<()> {
        let k = self.clave(path)?;
        self.bloqueante(move |c| {
            for clave in c.listar(&k)? {
                c.borrar(&clave)?;
            }
            Ok(())
        })
        .await
    }

    async fn delete_stream(&self, mut paths: BoxStream<'static, String>) -> iceberg::Result<()> {
        use futures::StreamExt;
        while let Some(p) = paths.next().await {
            self.delete(&p).await?;
        }
        Ok(())
    }

    fn new_input(&self, path: &str) -> iceberg::Result<InputFile> {
        Ok(InputFile::new(Arc::new(self.clone()), path.to_string()))
    }

    fn new_output(&self, path: &str) -> iceberg::Result<OutputFile> {
        Ok(OutputFile::new(Arc::new(self.clone()), path.to_string()))
    }
}

#[derive(Debug)]
struct Lectura(Bytes);

#[async_trait]
impl FileRead for Lectura {
    async fn read(&self, range: Range<u64>) -> iceberg::Result<Bytes> {
        let (a, b) = (range.start as usize, range.end as usize);
        if b > self.0.len() || a > b {
            return Err(iceberg::Error::new(
                iceberg::ErrorKind::DataInvalid,
                format!("rango {a}..{b} fuera de {} bytes", self.0.len()),
            ));
        }
        Ok(self.0.slice(a..b))
    }
}

/// Se acumula y se sube de una vez al cerrar: una subida por fichero, con el
/// `crc32c`/`sha256` que el almacén valida.
#[derive(Debug)]
struct Escritura {
    lago: Lago,
    path: String,
    buffer: Vec<u8>,
    cerrado: bool,
}

#[async_trait]
impl FileWrite for Escritura {
    async fn write(&mut self, bs: Bytes) -> iceberg::Result<()> {
        self.buffer.extend_from_slice(&bs);
        Ok(())
    }

    async fn close(&mut self) -> iceberg::Result<()> {
        if self.cerrado {
            return Err(iceberg::Error::new(
                iceberg::ErrorKind::Unexpected,
                "el fichero ya se cerró",
            ));
        }
        self.cerrado = true;
        let cuerpo = Bytes::from(std::mem::take(&mut self.buffer));
        self.lago.write(&self.path, cuerpo).await
    }
}

// ── El esquema ──────────────────────────────────────────────────────────────

/// **El esquema de Iceberg que el lote pide**, con los ids de la tabla que ya
/// existe si la hay: una columna que conserva nombre y tipo conserva su id;
/// una que cambió de tipo, o es nueva, recibe uno nuevo. Es la única regla de
/// evolución que hace falta porque cada sobrescritura reescribe todos los
/// datos: ningún fichero vivo tiene un id con dos tipos.
pub fn esquema_deseado(
    columnas: &[(String, DataType)],
    base: Option<&Schema>,
) -> Result<Schema, String> {
    let mut siguiente = base.map(|b| b.highest_field_id()).unwrap_or(0);
    let mut campos = Vec::with_capacity(columnas.len());
    for (nombre, tipo) in columnas {
        let t = iceberg::arrow::arrow_type_to_type(tipo)
            .map_err(|e| format!("la columna `{nombre}` ({tipo}) no cabe en Iceberg: {e}"))?;
        let id = match base.and_then(|b| b.field_by_name(nombre)) {
            Some(f) if *f.field_type == t => f.id,
            _ => {
                siguiente += 1;
                siguiente
            }
        };
        campos.push(Arc::new(NestedField::optional(id, nombre, t)));
    }
    Schema::builder()
        .with_fields(campos)
        .build()
        .map_err(|e| format!("el esquema no construye: {e}"))
}

/// ¿Son el mismo esquema, columna a columna (nombre, tipo e id, en orden)?
fn mismo_esquema(a: &Schema, b: &Schema) -> bool {
    let fa = a.as_struct().fields();
    let fb = b.as_struct().fields();
    fa.len() == fb.len()
        && fa
            .iter()
            .zip(fb)
            .all(|(x, y)| x.name == y.name && x.field_type == y.field_type && x.id == y.id)
}

/// Las columnas de un lote, en su orden.
pub fn columnas_de(lote: &RecordBatch) -> Vec<(String, DataType)> {
    lote.schema()
        .fields()
        .iter()
        .map(|f| (f.name().clone(), f.data_type().clone()))
        .collect()
}

// ── La tabla ────────────────────────────────────────────────────────────────

/// Qué snapshot se produce.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Operacion {
    /// Los ficheros nuevos se suman a los que había.
    Anexar,
    /// Los ficheros nuevos sustituyen a todos los que había.
    Sobrescribir,
}

/// Lo que devuelve escribir: la tabla en su estado nuevo y las cuentas.
pub struct Escrito {
    pub tabla: Table,
    pub ficheros: usize,
    pub bytes: u64,
    pub filas: u64,
    /// Ficheros del snapshot anterior que este snapshot retira.
    pub retirados: usize,
}

/// **La primera mitad de una escritura**: los ficheros ya están en el bucket y
/// esto es lo que un catálogo tiene que aplicar para que cuenten. Es el cuerpo
/// de un `updateTable` de la spec REST, con las cuentas al lado.
pub struct Preparado {
    pub requisitos: Vec<TableRequirement>,
    pub cambios: Vec<TableUpdate>,
    pub snapshot_id: i64,
    pub ficheros: usize,
    pub bytes: u64,
    pub filas: u64,
    pub retirados: usize,
    /// Si el esquema de la tabla cambió con este lote (`add-schema` va dentro).
    pub esquema_cambiado: bool,
}

impl Lago {
    fn ident(dataset: &str) -> TableIdent {
        TableIdent::from_strs(["ore", dataset]).expect("un identificador")
    }

    fn tabla(
        &self,
        meta: TableMetadata,
        ubicacion: Option<String>,
        dataset: &str,
    ) -> iceberg::Result<Table> {
        let mut b = Table::builder()
            .file_io(self.file_io())
            .metadata(meta)
            .identifier(Self::ident(dataset))
            .runtime(iceberg::Runtime::new(runtime()));
        if let Some(u) = ubicacion {
            b = b.metadata_location(u);
        }
        b.build()
    }

    /// **Abre una tabla por su `metadata_location`**: el puntero que el árbol
    /// guarda. No hay catálogo que consultar; el fichero ES el estado.
    pub fn abrir(&self, metadata_location: &str, dataset: &str) -> Result<Table, String> {
        let io = self.file_io();
        let meta = runtime()
            .block_on(TableMetadata::read_from(&io, metadata_location))
            .map_err(|e| {
                format!("`{metadata_location}` no se pudo abrir como tabla Iceberg: {e}")
            })?;
        self.tabla(meta, Some(metadata_location.to_string()), dataset)
            .map_err(err)
    }

    /// **Una tabla nueva, todavía sin escribir**: sus metadatos en memoria, en
    /// la ubicación del dataset. El primer `metadata.json` lo escribe el primer
    /// snapshot ([`Lago::instantanea`]), para que crear y poblar sea UN fichero
    /// y no dos.
    pub fn crear(
        &self,
        dataset: &str,
        esquema: Schema,
        propiedades: HashMap<String, String>,
    ) -> Result<Table, String> {
        let ubicacion = self.uri(&format!("{RAIZ}/{dataset}"));
        let mut props = propiedades;
        props.insert(PROP_DATASET.into(), dataset.into());
        let creacion = TableCreation::builder()
            .name(dataset.rsplit('/').next().unwrap_or(dataset).to_string())
            .location(ubicacion)
            .schema(esquema)
            .properties(props)
            .build();
        let meta = iceberg::spec::TableMetadataBuilder::from_table_creation(creacion)
            .and_then(|b| b.build())
            .map_err(err)?
            .metadata;
        self.tabla(meta, None, dataset).map_err(err)
    }

    /// **Aplica unos cambios y escribe el siguiente `metadata.json`.** Es lo que
    /// `MemoryCatalog::update_table` hace por dentro, sin el catálogo: los
    /// requisitos se comprueban contra los metadatos que se abrieron (el
    /// puntero del árbol), los cambios se aplican, y el fichero nuevo va a
    /// `<ubicación>/metadata/<v+1>-<uuid>.metadata.json`. Devuelve la tabla
    /// abierta sobre él. **Nadie lo apunta todavía**: eso es el commit del Job.
    pub fn confirmar(
        &self,
        tabla: &Table,
        cambios: Vec<TableUpdate>,
        requisitos: Vec<TableRequirement>,
    ) -> Result<Table, String> {
        runtime().block_on(self.confirmar_async(tabla, cambios, requisitos))
    }

    async fn confirmar_async(
        &self,
        tabla: &Table,
        cambios: Vec<TableUpdate>,
        requisitos: Vec<TableRequirement>,
    ) -> Result<Table, String> {
        // Una tabla sin `metadata_location` todavía no existe para nadie: sus
        // requisitos (`assert-create`) se comprueban contra «ninguna».
        let actual = tabla.metadata_location().map(String::from);
        for r in &requisitos {
            r.check(actual.as_ref().map(|_| tabla.metadata()))
                .map_err(err)?;
        }
        let mut b = tabla.metadata().clone().into_builder(actual.clone());
        for c in cambios {
            b = c.apply(b).map_err(err)?;
        }
        let nuevo = b.build().map_err(err)?.metadata;
        let destino = match &actual {
            Some(a) => MetadataLocation::from_str(a)
                .map_err(err)?
                .with_next_version()
                .with_new_metadata(&nuevo),
            None => MetadataLocation::new_with_metadata(nuevo.location(), &nuevo),
        };
        let io = self.file_io();
        nuevo
            .write_to(&io, &destino)
            .await
            .map_err(|e| format!("no se pudo escribir `{destino}`: {e}"))?;
        self.tabla(nuevo, Some(destino.to_string()), tabla.identifier().name())
            .map_err(err)
    }

    /// **La segunda mitad de una escritura, para lo que venga de fuera**: los
    /// `requirements` y `updates` de un `updateTable` (los de PyIceberg, los de
    /// DuckDB, los de [`Lago::preparar`]) aplicados a la tabla del puntero —o a
    /// **ninguna**, cuando el requisito es `assert-create`: entonces la tabla
    /// nace de los propios cambios (`add-schema`, `add-spec`, `set-location`,
    /// …), que es lo que un `stage-create` seguido de su commit significa—.
    /// Escribe el siguiente `metadata.json` y devuelve la tabla sobre él. El
    /// puntero sigue sin moverse: eso es de `ore`.
    pub fn aplicar(
        &self,
        base: Option<&Table>,
        dataset: &str,
        requisitos: Vec<TableRequirement>,
        cambios: Vec<TableUpdate>,
    ) -> Result<Table, String> {
        if let Some(t) = base {
            return self.confirmar(t, cambios, requisitos);
        }
        for r in &requisitos {
            r.check(None).map_err(err)?;
        }
        // Desde cero: lo que los cambios traen decide la tabla; lo que no
        // traen lo pone el dataset (la ubicación) o la spec (v2, sin partición
        // ni orden).
        let mut esquema = None;
        let mut spec = None;
        let mut orden = None;
        let mut ubicacion = None;
        let mut version = iceberg::spec::FormatVersion::V2;
        let mut props = HashMap::new();
        for c in &cambios {
            match c {
                TableUpdate::AddSchema { schema, .. } if esquema.is_none() => {
                    esquema = Some(schema.clone())
                }
                TableUpdate::AddSpec { spec: s } if spec.is_none() => spec = Some(s.clone()),
                TableUpdate::AddSortOrder { sort_order } if orden.is_none() => {
                    orden = Some(sort_order.clone())
                }
                TableUpdate::SetLocation { location } => ubicacion = Some(location.clone()),
                TableUpdate::UpgradeFormatVersion { format_version } => version = *format_version,
                TableUpdate::SetProperties { updates } => props.extend(updates.clone()),
                _ => {}
            }
        }
        let esquema = esquema.ok_or("una tabla nueva necesita `add-schema` entre sus cambios")?;
        let ubicacion = ubicacion.unwrap_or_else(|| self.uri(&format!("{RAIZ}/{dataset}")));
        props
            .entry(PROP_DATASET.into())
            .or_insert_with(|| dataset.into());
        let mut b = iceberg::spec::TableMetadataBuilder::new(
            esquema,
            spec.unwrap_or_else(iceberg::spec::UnboundPartitionSpec::default),
            orden.unwrap_or_else(iceberg::spec::SortOrder::unsorted_order),
            ubicacion,
            version,
            props,
        )
        .map_err(err)?;
        for c in cambios {
            // Lo que ya puso el arranque —el esquema, la partición, el orden,
            // la ubicación, la versión, las propiedades— vuelve a aplicarse
            // sin efecto: el constructor reconoce lo igual.
            b = c.apply(b).map_err(err)?;
        }
        let nuevo = b.build().map_err(err)?.metadata;
        let destino = MetadataLocation::new_with_metadata(nuevo.location(), &nuevo);
        let io = self.file_io();
        runtime()
            .block_on(nuevo.write_to(&io, &destino))
            .map_err(|e| format!("no se pudo escribir `{destino}`: {e}"))?;
        self.tabla(nuevo, Some(destino.to_string()), dataset)
            .map_err(err)
    }

    /// **El esquema de la tabla pasa a ser el del lote**, si no lo era ya. Un
    /// `metadata.json` más sólo cuando cambia algo.
    pub fn esquema(&self, tabla: &Table, deseado: Schema) -> Result<(Table, bool), String> {
        if mismo_esquema(tabla.metadata().current_schema(), &deseado) {
            return Ok((tabla.clone(), false));
        }
        let t = self.confirmar(
            tabla,
            vec![
                TableUpdate::AddSchema { schema: deseado },
                TableUpdate::SetCurrentSchema { schema_id: -1 },
            ],
            vec![TableRequirement::UuidMatch {
                uuid: tabla.metadata().uuid(),
            }],
        )?;
        Ok((t, true))
    }

    /// **Escribe los ficheros de datos** (Parquet, SNAPPY, con las estadísticas
    /// que Iceberg pide) y **confirma el snapshot**. El lote llega con las
    /// columnas en el orden del esquema de la tabla; se reenvuelve con el
    /// esquema de Arrow que sale de ella para que lleve los ids de campo.
    pub fn instantanea(
        &self,
        tabla: &Table,
        lote: RecordBatch,
        operacion: Operacion,
        propiedades: HashMap<String, String>,
    ) -> Result<Escrito, String> {
        let esquema = tabla.metadata().current_schema().as_ref().clone();
        let p = self.preparar(tabla, esquema, vec![lote], operacion, propiedades)?;
        let nueva = self.confirmar(tabla, p.cambios, p.requisitos)?;
        Ok(Escrito {
            tabla: nueva,
            ficheros: p.ficheros,
            bytes: p.bytes,
            filas: p.filas,
            retirados: p.retirados,
        })
    }

    /// **La primera mitad de escribir**: los ficheros de datos (Parquet, SNAPPY,
    /// con las estadísticas que Iceberg pide), los manifiestos y la lista, para
    /// los lotes que lleguen, con `esquema` —el de la tabla, o el que el lote
    /// pide: si no es el vigente, `add-schema` + `set-current-schema` van
    /// delante del snapshot y los ficheros ya llevan los ids nuevos—. Devuelve
    /// los `requirements` y `updates` que un catálogo aplica; nada se confirma.
    pub fn preparar(
        &self,
        tabla: &Table,
        esquema: Schema,
        lotes: Vec<RecordBatch>,
        operacion: Operacion,
        propiedades: HashMap<String, String>,
    ) -> Result<Preparado, String> {
        runtime().block_on(self.preparar_async(tabla, esquema, lotes, operacion, propiedades))
    }

    async fn preparar_async(
        &self,
        tabla: &Table,
        esquema: Schema,
        lotes: Vec<RecordBatch>,
        operacion: Operacion,
        propiedades: HashMap<String, String>,
    ) -> Result<Preparado, String> {
        let meta = tabla.metadata();
        let io = tabla.file_io().clone();
        let filas: u64 = lotes.iter().map(|l| l.num_rows() as u64).sum();
        let esquema = Arc::new(esquema);

        // ── el esquema: el de la tabla, o el nuevo con el id que le tocará ──
        let esquema_cambiado = !mismo_esquema(meta.current_schema(), &esquema);
        let mut cambios = Vec::new();
        let schema_id = if esquema_cambiado {
            // Lo mismo que el constructor de Iceberg hace al aplicar
            // `add-schema`: si ya hay un esquema igual, su id; si no, el
            // siguiente al mayor.
            let ya = meta
                .schemas_iter()
                .find(|s| s.as_struct() == esquema.as_struct())
                .map(|s| s.schema_id());
            cambios.push(TableUpdate::AddSchema {
                schema: esquema.as_ref().clone(),
            });
            cambios.push(TableUpdate::SetCurrentSchema { schema_id: -1 });
            ya.unwrap_or_else(|| {
                meta.schemas_iter()
                    .map(|s| s.schema_id())
                    .max()
                    .unwrap_or(-1)
                    + 1
            })
        } else {
            meta.current_schema_id()
        };

        // ── los ficheros de datos ───────────────────────────────────────────
        let ficheros: Vec<DataFile> = if filas == 0 {
            Vec::new()
        } else {
            let ubicacion = DefaultLocationGenerator::new(meta).map_err(err)?;
            let nombres = DefaultFileNameGenerator::new(
                uuid::Uuid::new_v4().to_string(),
                None,
                DataFileFormat::Parquet,
            );
            let props = parquet::file::properties::WriterProperties::builder()
                .set_compression(parquet::basic::Compression::SNAPPY)
                .build();
            let parquet = ParquetWriterBuilder::new(props, esquema.clone());
            let rodante = RollingFileWriterBuilder::new_with_default_file_size(
                parquet,
                io.clone(),
                ubicacion,
                nombres,
            );
            let mut escritor = DataFileWriterBuilder::new(rodante)
                .build(None)
                .await
                .map_err(err)?;
            let arrow = Arc::new(iceberg::arrow::schema_to_arrow_schema(&esquema).map_err(err)?);
            for lote in lotes {
                let lote = RecordBatch::try_new(arrow.clone(), lote.columns().to_vec())
                    .map_err(|e| format!("el lote no casa con el esquema de la tabla: {e}"))?;
                escritor.write(lote).await.map_err(err)?;
            }
            escritor.close().await.map_err(err)?
        };
        let bytes: u64 = ficheros.iter().map(|f| f.file_size_in_bytes()).sum();

        // ── los manifiestos ─────────────────────────────────────────────────
        let snapshot_id = {
            let mut id = id_aleatorio();
            while meta.snapshots().any(|s| s.snapshot_id() == id) {
                id = id_aleatorio();
            }
            id
        };
        let commit = uuid::Uuid::new_v4();
        let secuencia = meta.next_sequence_number();
        let mut contador = 0u32;
        let mut nuevo_manifiesto = |io: &FileIO| -> iceberg::Result<iceberg::spec::ManifestWriter> {
            let ruta = format!("{}/metadata/{commit}-m{contador}.avro", meta.location());
            contador += 1;
            Ok(ManifestWriterBuilder::new(
                io.new_output(ruta)?,
                Some(snapshot_id),
                esquema.clone(),
                meta.default_partition_spec().as_ref().clone(),
            )
            .build_v2_data())
        };
        let mut manifiestos = Vec::new();
        if !ficheros.is_empty() {
            let mut w = nuevo_manifiesto(&io).map_err(err)?;
            for f in &ficheros {
                w.add_file(f.clone(), secuencia).map_err(err)?;
            }
            manifiestos.push(w.write_manifest_file().await.map_err(err)?);
        }
        let mut retirados = 0usize;
        let mut filas_retiradas = 0u64;
        let mut bytes_retirados = 0u64;
        match (operacion, meta.current_snapshot()) {
            (Operacion::Sobrescribir, Some(actual)) => {
                let lista = tabla
                    .manifest_list_reader(actual)
                    .load()
                    .await
                    .map_err(err)?;
                for mf in lista.entries() {
                    if mf.content != ManifestContentType::Data {
                        continue;
                    }
                    let m = mf.load_manifest(&io).await.map_err(err)?;
                    let vivos: Vec<_> = m.entries().iter().filter(|e| e.is_alive()).collect();
                    if vivos.is_empty() {
                        continue;
                    }
                    let mut w = nuevo_manifiesto(&io).map_err(err)?;
                    for e in vivos {
                        w.add_delete_file(
                            e.data_file().clone(),
                            e.sequence_number().unwrap_or(secuencia),
                            e.file_sequence_number,
                        )
                        .map_err(err)?;
                        retirados += 1;
                        filas_retiradas += e.data_file().record_count();
                        bytes_retirados += e.data_file().file_size_in_bytes();
                    }
                    manifiestos.push(w.write_manifest_file().await.map_err(err)?);
                }
            }
            (Operacion::Anexar, Some(actual)) => {
                // Los manifiestos que había siguen: es lo que `fast_append` hace.
                let lista = tabla
                    .manifest_list_reader(actual)
                    .load()
                    .await
                    .map_err(err)?;
                manifiestos.extend(lista.consume_entries());
            }
            (_, None) => {}
        }
        if manifiestos.is_empty() {
            return Err("nada que escribir: ni ficheros nuevos ni tabla que sobrescribir".into());
        }

        // ── la lista de manifiestos y el snapshot ──────────────────────────
        let ruta_lista = format!(
            "{}/metadata/snap-{snapshot_id}-0-{commit}.avro",
            meta.location()
        );
        let salida = io.new_output(&ruta_lista).map_err(err)?;
        let mut lista = ManifestListWriter::v2(
            salida.writer().await.map_err(err)?,
            snapshot_id,
            meta.current_snapshot_id(),
            secuencia,
        );
        lista.add_manifests(manifiestos.into_iter()).map_err(err)?;
        lista.close().await.map_err(err)?;

        let mut resumen = SnapshotSummaryCollector::default();
        for f in &ficheros {
            resumen.add_file(f, esquema.clone(), meta.default_partition_spec().clone());
        }
        let mut props = propiedades;
        props.extend(resumen.build());
        let anterior = meta.current_snapshot().map(|s| s.summary());
        let previo = |k: &str| -> u64 {
            anterior
                .and_then(|s| s.additional_properties.get(k))
                .and_then(|v| v.parse().ok())
                .unwrap_or(0)
        };
        let (total_filas, total_ficheros, total_bytes) = match operacion {
            Operacion::Sobrescribir => (filas, ficheros.len() as u64, bytes),
            Operacion::Anexar => (
                previo("total-records") + filas,
                previo("total-data-files") + ficheros.len() as u64,
                previo("total-files-size") + bytes,
            ),
        };
        props.insert("total-records".into(), total_filas.to_string());
        props.insert("total-data-files".into(), total_ficheros.to_string());
        props.insert("total-files-size".into(), total_bytes.to_string());
        props.insert("total-delete-files".into(), "0".into());
        props.insert("total-position-deletes".into(), "0".into());
        props.insert("total-equality-deletes".into(), "0".into());
        if retirados > 0 {
            props.insert("deleted-data-files".into(), retirados.to_string());
            props.insert("deleted-records".into(), filas_retiradas.to_string());
            props.insert("removed-files-size".into(), bytes_retirados.to_string());
        }
        let snapshot = Snapshot::builder()
            .with_snapshot_id(snapshot_id)
            .with_parent_snapshot_id(meta.current_snapshot_id())
            .with_sequence_number(secuencia)
            .with_timestamp_ms(ahora_ms())
            .with_manifest_list(ruta_lista)
            .with_summary(Summary {
                operation: match operacion {
                    Operacion::Anexar => Operation::Append,
                    Operacion::Sobrescribir => Operation::Overwrite,
                },
                additional_properties: props,
            })
            .with_schema_id(schema_id)
            .build();
        // Los requisitos: la tabla que se abrió sigue siendo esa (uuid) y
        // `main` sigue donde estaba. Una tabla que todavía no tiene
        // `metadata.json` es una que nace en este commit: `assert-create`, y
        // con ella viajan todos sus cimientos, como manda la spec REST.
        let requisitos = if tabla.metadata_location().is_some() {
            vec![
                TableRequirement::UuidMatch { uuid: meta.uuid() },
                TableRequirement::RefSnapshotIdMatch {
                    r#ref: MAIN_BRANCH.to_string(),
                    snapshot_id: meta.current_snapshot_id(),
                },
            ]
        } else {
            let mut cimientos = vec![
                TableUpdate::AssignUuid { uuid: meta.uuid() },
                TableUpdate::UpgradeFormatVersion {
                    format_version: meta.format_version(),
                },
            ];
            if !esquema_cambiado {
                cimientos.push(TableUpdate::AddSchema {
                    schema: esquema.as_ref().clone(),
                });
                cimientos.push(TableUpdate::SetCurrentSchema { schema_id: -1 });
            }
            cimientos.extend([
                TableUpdate::AddSpec {
                    spec: meta
                        .default_partition_spec()
                        .as_ref()
                        .clone()
                        .into_unbound(),
                },
                TableUpdate::SetDefaultSpec { spec_id: -1 },
                TableUpdate::AddSortOrder {
                    sort_order: meta.default_sort_order().as_ref().clone(),
                },
                TableUpdate::SetDefaultSortOrder { sort_order_id: -1 },
                TableUpdate::SetLocation {
                    location: meta.location().to_string(),
                },
                TableUpdate::SetProperties {
                    updates: meta.properties().clone(),
                },
            ]);
            cimientos.append(&mut cambios);
            cambios = cimientos;
            vec![TableRequirement::NotExist]
        };
        cambios.push(TableUpdate::AddSnapshot { snapshot });
        cambios.push(TableUpdate::SetSnapshotRef {
            ref_name: MAIN_BRANCH.to_string(),
            reference: SnapshotReference::new(
                snapshot_id,
                SnapshotRetention::branch(None, None, None),
            ),
        });
        Ok(Preparado {
            requisitos,
            cambios,
            snapshot_id,
            ficheros: ficheros.len(),
            bytes,
            filas: total_filas,
            retirados,
            esquema_cambiado,
        })
    }

    /// **Las filas de la tabla, en su snapshot vigente**, como texto canónico
    /// (lo que [`carga::leer`] devuelve): para fundir un incremento y para
    /// `leer`. Cada fichero de datos se baja entero, que es lo que se necesita.
    pub fn filas(&self, tabla: &Table) -> Result<Vec<carga::Fila>, String> {
        let rutas = runtime().block_on(self.ficheros_vivos(tabla))?;
        let cuenta = self.cuenta().map_err(err)?;
        let mut out = Vec::new();
        for ruta in rutas {
            let k = self.clave(&ruta).map_err(err)?;
            let bytes = cuenta
                .leer_bytes(&k)?
                .ok_or_else(|| format!("el fichero de datos `{ruta}` no está en el almacén"))?;
            out.extend(carga::leer(&bytes)?);
        }
        Ok(out)
    }

    /// Los ficheros de datos vivos del snapshot vigente.
    async fn ficheros_vivos(&self, tabla: &Table) -> Result<Vec<String>, String> {
        let Some(actual) = tabla.metadata().current_snapshot() else {
            return Ok(Vec::new());
        };
        let io = tabla.file_io();
        let lista = tabla
            .manifest_list_reader(actual)
            .load()
            .await
            .map_err(err)?;
        let mut out = Vec::new();
        for mf in lista.entries() {
            if mf.content != ManifestContentType::Data {
                continue;
            }
            let m = mf.load_manifest(io).await.map_err(err)?;
            out.extend(
                m.entries()
                    .iter()
                    .filter(|e| e.is_alive())
                    .map(|e| e.file_path().to_string()),
            );
        }
        Ok(out)
    }

    /// **Expira los snapshots que no son el vigente y son más viejos que
    /// `edad_ms`**. Sólo los metadatos: los ficheros que se quedan sin
    /// snapshot los retira [`Lago::huerfanos`]. Un `metadata.json` más sólo si
    /// expiró alguno.
    pub fn expirar(
        &self,
        tabla: &Table,
        edad_ms: i64,
        minimo: usize,
    ) -> Result<(Table, Vec<i64>), String> {
        let ids = Self::expirables(tabla, edad_ms, minimo);
        if ids.is_empty() {
            return Ok((tabla.clone(), ids));
        }
        let meta = tabla.metadata();
        let t = self.confirmar(
            tabla,
            vec![TableUpdate::RemoveSnapshots {
                snapshot_ids: ids.clone(),
            }],
            vec![TableRequirement::UuidMatch { uuid: meta.uuid() }],
        )?;
        Ok((t, ids))
    }

    /// **Qué snapshots expirarían**: los que no son el vigente, son más viejos
    /// que `edad_ms`, y no están entre los `minimo` más recientes (el vigente
    /// cuenta entre ellos, como en Iceberg).
    pub fn expirables(tabla: &Table, edad_ms: i64, minimo: usize) -> Vec<i64> {
        let meta = tabla.metadata();
        let corte = ahora_ms() - edad_ms;
        let mut todos: Vec<_> = meta.snapshots().collect();
        todos.sort_by_key(|s| -s.timestamp_ms());
        todos
            .iter()
            .skip(minimo.max(1))
            .filter(|s| Some(s.snapshot_id()) != meta.current_snapshot_id())
            .filter(|s| s.timestamp_ms() <= corte)
            .map(|s| s.snapshot_id())
            .collect()
    }

    /// **La retención que rige**: la de la tabla (`history.expire.*`) y, para
    /// lo que la tabla no diga, el defecto que traiga quien llama. `None` de
    /// edad es «no expirar».
    pub fn retencion(tabla: &Table, edad_defecto_ms: Option<i64>) -> (Option<i64>, usize) {
        let p = tabla.metadata().properties();
        let edad = p
            .get(PROP_RETENCION_EDAD)
            .and_then(|v| v.parse::<i64>().ok())
            .or(edad_defecto_ms);
        let minimo = p
            .get(PROP_RETENCION_MINIMO)
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(1);
        (edad, minimo)
    }

    /// **Lo que hay bajo la ubicación de la tabla y ningún snapshot suyo
    /// nombra**: ficheros de datos y manifiestos de snapshots expirados, lo
    /// que dejó una pasada que no llegó a apuntarse en el árbol, y los
    /// `metadata.json` que el registro de metadatos ya no lista. Devuelve
    /// cuántos se fueron (o se irían, en seco).
    pub fn huerfanos(&self, tabla: &Table, seco: bool) -> Result<usize, String> {
        let vivos = runtime().block_on(self.alcanzables(tabla))?;
        let cuenta = self.cuenta().map_err(err)?;
        let prefijo = format!("{}/", self.clave(tabla.metadata().location()).map_err(err)?);
        let mut n = 0usize;
        for k in cuenta.listar(&prefijo)? {
            if vivos.contains(&self.uri(&k)) {
                continue;
            }
            if !seco {
                cuenta.borrar(&k)?;
            }
            n += 1;
        }
        Ok(n)
    }

    /// Todo lo que la tabla nombra: el `metadata.json` vigente y los del
    /// registro, y por cada snapshot su lista, sus manifiestos y los ficheros
    /// de datos vivos en él.
    async fn alcanzables(&self, tabla: &Table) -> Result<BTreeSet<String>, String> {
        let meta = tabla.metadata();
        let io = tabla.file_io();
        let mut vivos = BTreeSet::new();
        if let Some(m) = tabla.metadata_location() {
            vivos.insert(m.to_string());
        }
        for l in meta.metadata_log() {
            vivos.insert(l.metadata_file.clone());
        }
        for s in meta.snapshots() {
            vivos.insert(s.manifest_list().to_string());
            let lista = tabla.manifest_list_reader(s).load().await.map_err(err)?;
            for mf in lista.entries() {
                vivos.insert(mf.manifest_path.clone());
                let m = mf.load_manifest(io).await.map_err(err)?;
                vivos.extend(
                    m.entries()
                        .iter()
                        .filter(|e| e.is_alive())
                        .map(|e| e.file_path().to_string()),
                );
            }
        }
        Ok(vivos)
    }

    /// Cuántas filas dice el snapshot vigente (`total-records`).
    pub fn filas_del_snapshot(tabla: &Table) -> u64 {
        tabla
            .metadata()
            .current_snapshot()
            .and_then(|s| s.summary().additional_properties.get("total-records"))
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    }

    /// Una propiedad del snapshot vigente, o de la tabla.
    pub fn propiedad(tabla: &Table, k: &str) -> Option<String> {
        tabla
            .metadata()
            .current_snapshot()
            .and_then(|s| s.summary().additional_properties.get(k).cloned())
            .or_else(|| tabla.metadata().properties().get(k).cloned())
    }
}

fn id_aleatorio() -> i64 {
    let (a, b) = uuid::Uuid::new_v4().as_u64_pair();
    ((a ^ b) as i64).wrapping_abs().max(1)
}

pub fn ahora_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// **Del tipo de Iceberg al escalar de OOS** (0032, la vuelta): lo que la
/// `Table` del lago declara cuando nace de una escritura. Lo que el contrato
/// no tiene (anidados, binario, uuid) va como `String` y se dice en la ficha
/// por su tipo de Iceberg.
pub fn oos_de_iceberg(t: &iceberg::spec::Type) -> &'static str {
    use iceberg::spec::{PrimitiveType, Type};
    match t {
        Type::Primitive(p) => match p {
            PrimitiveType::Int | PrimitiveType::Long => "Integer",
            PrimitiveType::Float | PrimitiveType::Double => "Float",
            PrimitiveType::Boolean => "Boolean",
            PrimitiveType::Decimal { .. } => "Decimal",
            PrimitiveType::Date => "Date",
            PrimitiveType::Time => "Time",
            PrimitiveType::Timestamp | PrimitiveType::TimestampNs => "DateTime",
            PrimitiveType::Timestamptz | PrimitiveType::TimestamptzNs => "DateTimeTz",
            _ => "String",
        },
        _ => "String",
    }
}

/// Las columnas de un esquema de Iceberg como escalares de OOS.
pub fn columnas_oos(tabla: &Table) -> BTreeMap<String, String> {
    tabla
        .metadata()
        .current_schema()
        .as_struct()
        .fields()
        .iter()
        .map(|f| (f.name.clone(), oos_de_iceberg(&f.field_type).to_string()))
        .collect()
}

/// Las columnas de un esquema de Iceberg, por nombre y tipo, para el informe.
pub fn columnas_iceberg(tabla: &Table) -> BTreeMap<String, String> {
    tabla
        .metadata()
        .current_schema()
        .as_struct()
        .fields()
        .iter()
        .map(|f| (f.name.clone(), f.field_type.to_string()))
        .collect()
}
