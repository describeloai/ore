//! **El índice de una colección**: su listado, ordenado por camino, con una
//! búsqueda por huella y por blob, guardado por la transacción que lo produjo.
//!
//! El listado es una tabla Iceberg (`colecciones/<b>/<s>/<n>`) cuyas filas son
//! texto: `clave, camino, version, etag, huella, formato, tamano, modificado,
//! estado, entro, retirado, retirado_ms` y, en una mantenida, `blob` y `tipo`.
//! En una escrita por `apply()` (0049 B9), además el linaje de cada fichero
//! —`origen`, `origen_uri`, `origen_digest`, `ancla`, `derivacion`— y las
//! **marcas**: filas sin blob (`marca` = `vacio` | `error`) de un origen que no
//! dio ficheros. Una marca no es un ítem: va aparte y sólo la ve el registro.
//! Medido (B2·0): 1 M de filas se leen en 0,63 s (451 MB en Arrow) y se indexan
//! en 1,15 s; aquí se guarda sólo lo que se sirve.

use arrow_array::RecordBatch;
use arrow_array::cast::AsArray;
use ore_core::json::Json;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex};

/// Un ítem del listado: lo que se sirve de él.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub camino: String,
    pub version: String,
    /// `crc64nvme:…` (o `etag:…` si el origen no la da): validador, no identidad.
    pub huella: String,
    /// El sha256 del blob en el lago; sólo en una mantenida.
    pub blob: Option<String>,
    pub tipo: Option<String>,
    pub formato: Option<String>,
    pub tamano: Option<i64>,
    pub modificado: Option<String>,
    /// `actual`, `retirado` o `perdido`.
    pub estado: String,
    /// La transacción de la colección en que entró.
    pub entro: Option<String>,
    /// 0049 B3·1: **la clave en el origen** (`Nueva carpeta/contratos/a.pdf`)
    /// y **su ETag**, con los que la puerta de lectura fija una virtual
    /// (`versionId` + `If-Match`). El camino es relativo a la colección; la
    /// clave, no.
    pub clave: Option<String>,
    pub etag: Option<String>,
    /// 0049 B9 · el linaje de un fichero derivado.
    pub linaje: Option<Linaje>,
}

/// 0049 B9 · De qué ítem sale un fichero, o una marca, y con qué se calculó.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Linaje {
    /// La identidad del origen (v1alpha17 `01` §3.1): su `digest`, o su `uri`.
    pub origen: String,
    pub uri: String,
    pub digest: Option<String>,
    /// El ancla (JSON canónico), si el fichero es una parte del origen.
    pub ancla: Option<String>,
    /// El struct `_derivation` (JSON canónico).
    pub derivacion: Option<String>,
}

/// 0049 B9 · **Una marca**: un origen que no dio ficheros (`vacio`) o que
/// falló (`error`, con `{type, message}` en JSON). No es un ítem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Marca {
    pub clave: String,
    pub version: String,
    pub estado: String,
    pub entro: Option<String>,
    pub marca: String,
    pub error: Option<String>,
    pub linaje: Linaje,
}

/// Qué ítems se listan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Estado {
    Actual,
    Retirado,
    Perdido,
    Todos,
}

impl Estado {
    pub fn de(s: &str) -> Option<Estado> {
        Some(match s {
            "actual" => Estado::Actual,
            "retirado" => Estado::Retirado,
            "perdido" => Estado::Perdido,
            "todos" => Estado::Todos,
            _ => return None,
        })
    }

    fn admite(self, estado: &str) -> bool {
        match self {
            Estado::Actual => estado == "actual",
            Estado::Retirado => estado == "retirado",
            Estado::Perdido => estado == "perdido",
            Estado::Todos => true,
        }
    }
}

/// El índice de UNA transacción de una colección.
pub struct Indice {
    /// `b.s.n`.
    pub coleccion: String,
    pub virtual_: bool,
    /// La transacción del puntero que lo produjo: lo que un `list` dice en `as_of`.
    pub transaccion: String,
    /// Por `(camino, version)`.
    items: Vec<Item>,
    por_huella: HashMap<String, Vec<usize>>,
    por_blob: HashMap<String, Vec<usize>>,
    /// 0049 B9 · las marcas, de todos los estados.
    marcas: Vec<Marca>,
}

impl Indice {
    /// Del listado, en lotes de Arrow. Una columna que falta es nula en todas
    /// las filas (un listado de antes de que existiera); `camino`, `version`,
    /// `huella` y `estado` son obligatorias.
    pub fn de_lotes(
        coleccion: &str,
        virtual_: bool,
        transaccion: &str,
        lotes: &[RecordBatch],
    ) -> Result<Indice, String> {
        let mut items = Vec::with_capacity(lotes.iter().map(|l| l.num_rows()).sum());
        let mut marcas = Vec::new();
        for l in lotes {
            let col = |n: &str| l.column_by_name(n).and_then(|c| c.as_string_opt::<i32>());
            let obligatoria = |n: &str| {
                col(n).ok_or_else(|| format!("el listado de `{coleccion}` no tiene `{n}`"))
            };
            let (camino, version, huella, estado) = (
                obligatoria("camino")?,
                obligatoria("version")?,
                obligatoria("huella")?,
                obligatoria("estado")?,
            );
            let (blob, tipo, formato, modificado) =
                (col("blob"), col("tipo"), col("formato"), col("modificado"));
            let (clave, etag) = (col("clave"), col("etag"));
            let (origen, origen_uri, origen_digest) =
                (col("origen"), col("origen_uri"), col("origen_digest"));
            let (ancla, derivacion, marca, error) =
                (col("ancla"), col("derivacion"), col("marca"), col("error"));
            let texto = |c: Option<&arrow_array::StringArray>, i: usize| {
                c.filter(|c| !arrow_array::Array::is_null(*c, i))
                    .map(|c| c.value(i).to_string())
                    .filter(|s| !s.is_empty())
            };
            for i in 0..l.num_rows() {
                let linaje = texto(origen, i).map(|o| Linaje {
                    origen: o,
                    uri: texto(origen_uri, i).unwrap_or_default(),
                    digest: texto(origen_digest, i),
                    ancla: texto(ancla, i),
                    derivacion: texto(derivacion, i),
                });
                if let Some(m) = texto(marca, i) {
                    let Some(linaje) = linaje else {
                        return Err(format!(
                            "el listado de `{coleccion}` tiene una marca sin `origen`"
                        ));
                    };
                    marcas.push(Marca {
                        clave: texto(clave, i).unwrap_or_default(),
                        version: version.value(i).to_string(),
                        estado: estado.value(i).to_string(),
                        entro: entero(l, "entro", i).map(|e| e.to_string()),
                        marca: m,
                        error: texto(error, i),
                        linaje,
                    });
                    continue;
                }
                items.push(Item {
                    camino: camino.value(i).to_string(),
                    version: version.value(i).to_string(),
                    huella: huella.value(i).to_string(),
                    blob: texto(blob, i),
                    tipo: texto(tipo, i),
                    formato: texto(formato, i),
                    tamano: entero(l, "tamano", i),
                    modificado: texto(modificado, i),
                    estado: estado.value(i).to_string(),
                    entro: entero(l, "entro", i).map(|e| e.to_string()),
                    clave: texto(clave, i),
                    etag: texto(etag, i),
                    linaje,
                });
            }
        }
        items.sort_by(|a, b| (&a.camino, &a.version).cmp(&(&b.camino, &b.version)));
        let mut por_huella: HashMap<String, Vec<usize>> = HashMap::new();
        let mut por_blob: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, it) in items.iter().enumerate() {
            por_huella.entry(it.huella.clone()).or_default().push(i);
            if let Some(b) = &it.blob {
                por_blob.entry(b.clone()).or_default().push(i);
            }
        }
        Ok(Indice {
            coleccion: coleccion.to_string(),
            virtual_,
            transaccion: transaccion.to_string(),
            items,
            por_huella,
            por_blob,
            marcas,
        })
    }

    pub fn filas(&self) -> usize {
        self.items.len()
    }

    /// **Una página por cursor** (`list`): los ítems del `estado` pedido, bajo
    /// `prefijo`, a partir del cursor, hasta `limite`. El cursor es opaco (el
    /// camino y la versión del último dado) y la página cuesta lo mismo sea la
    /// primera o la última: una búsqueda binaria, no un desplazamiento.
    pub fn pagina(
        &self,
        prefijo: Option<&str>,
        cursor: Option<&str>,
        limite: usize,
        estado: Estado,
    ) -> Result<(Vec<&Item>, Option<String>), String> {
        let desde = match cursor {
            Some(c) => {
                let (camino, version) = de_cursor(c)?;
                self.items.partition_point(|x| {
                    (x.camino.as_str(), x.version.as_str()) <= (camino.as_str(), version.as_str())
                })
            }
            None => match prefijo {
                Some(p) => self.items.partition_point(|x| x.camino.as_str() < p),
                None => 0,
            },
        };
        let mut pagina = Vec::with_capacity(limite.min(1000));
        let mut ultimo = None;
        let mut mas = false;
        for it in &self.items[desde..] {
            if let Some(p) = prefijo
                && !it.camino.starts_with(p)
            {
                if it.camino.as_str() > p {
                    break;
                }
                continue;
            }
            if !estado.admite(&it.estado) {
                continue;
            }
            if pagina.len() == limite {
                mas = true;
                break;
            }
            ultimo = Some(it);
            pagina.push(it);
        }
        let siguiente = if mas { ultimo.map(cursor_de) } else { None };
        Ok((pagina, siguiente))
    }

    /// **Un ítem por su camino** (`stat`): de esa versión, o el actual.
    pub fn por_camino(&self, camino: &str, version: Option<&str>) -> Option<&Item> {
        let desde = self.items.partition_point(|x| x.camino.as_str() < camino);
        let mismos = self.items[desde..]
            .iter()
            .take_while(|x| x.camino == camino);
        match version {
            Some(v) => mismos.into_iter().find(|x| x.version == v),
            None => mismos.into_iter().find(|x| x.estado == "actual"),
        }
    }

    /// **Un ítem por su contenido**: `sha256:<hex>` es el blob; cualquier otra
    /// cosa, la huella. De varias filas con el mismo contenido se sirve la
    /// actual antes que la retirada y, entre iguales, la de menor camino
    /// (determinista); lo perdido no se sirve, y de una mantenida tampoco una
    /// fila sin blob (la regla de `ore collections --servir`).
    pub fn por_contenido(&self, digest: &str) -> Option<&Item> {
        let filas = match digest.strip_prefix("sha256:") {
            Some(b) => self.por_blob.get(b),
            None => self.por_huella.get(digest),
        }?;
        let rango = |e: &str| if e == "actual" { 0 } else { 1 };
        filas
            .iter()
            .map(|&i| &self.items[i])
            .filter(|x| matches!(x.estado.as_str(), "actual" | "retirado"))
            // B8·3: de una mantenida, antes lo que ya está en el lago; lo que
            // todavía no, también (se sirve de su origen mientras se copia).
            .min_by(|a, b| {
                (!self.virtual_ && a.blob.is_none())
                    .cmp(&(!self.virtual_ && b.blob.is_none()))
                    .then_with(|| rango(&a.estado).cmp(&rango(&b.estado)))
                    .then_with(|| a.camino.cmp(&b.camino))
            })
    }

    /// Cuántos ítems hay en un estado (`actual`, `retirado`, `perdido`).
    pub fn cuantos(&self, estado: &str) -> usize {
        self.items.iter().filter(|x| x.estado == estado).count()
    }

    /// Si la versión de un ítem es la actual de su camino.
    pub fn es_actual(&self, it: &Item) -> bool {
        self.por_camino(&it.camino, None)
            .is_some_and(|a| a.version == it.version)
    }

    /// **La referencia** de un ítem (OOS v1alpha17 `01` §3): la forma de
    /// Parquet `FILE` y los campos de ORE. Sin URL firmada, nunca.
    pub fn referencia(&self, it: &Item) -> Json {
        let nulo = || Json::Crudo("null".into());
        let o = |v: &Option<String>| v.as_ref().map(Json::s).unwrap_or_else(nulo);
        Json::obj([
            (
                "uri",
                Json::s(format!(
                    "ore://{}/{}?v={}",
                    self.coleccion, it.camino, it.version
                )),
            ),
            ("collection", Json::s(&self.coleccion)),
            ("path", Json::s(&it.camino)),
            ("version", Json::s(&it.version)),
            (
                "digest",
                it.blob
                    .as_ref()
                    .map(|b| Json::s(format!("sha256:{b}")))
                    .unwrap_or_else(nulo),
            ),
            ("size", it.tamano.map(Json::Int).unwrap_or_else(nulo)),
            ("content_type", o(&it.tipo)),
            ("content_type_detected", nulo()),
            ("checksum", Json::s(&it.huella)),
            ("annotations", nulo()),
            ("modified", o(&it.modificado)),
            ("state", Json::s(&it.estado)),
            // La transacción en que entró: la columna `transaction` del
            // listado (v1alpha17 `04` §1).
            ("transaction", o(&it.entro)),
        ])
        .con_linaje(it.linaje.as_ref())
    }

    /// 0049 B9 · Los ficheros actuales que salen de un origen.
    pub fn derivados_de(&self, origen: &str) -> Vec<&Item> {
        self.items
            .iter()
            .filter(|x| x.estado == "actual")
            .filter(|x| x.linaje.as_ref().is_some_and(|l| l.origen == origen))
            .collect()
    }

    /// 0049 B9 · La marca actual de un origen, si la tiene.
    pub fn marca_de(&self, origen: &str) -> Option<&Marca> {
        self.marcas
            .iter()
            .find(|m| m.estado == "actual" && m.linaje.origen == origen)
    }

    /// 0049 B9 · **El registro** (`GET …/derivations`): una entrada por origen,
    /// con sus ficheros actuales o su marca, ordenadas por origen; una página
    /// de `limite` desde el cursor (el origen del último dado, en hexadecimal).
    pub fn derivaciones(
        &self,
        cursor: Option<&str>,
        limite: usize,
    ) -> Result<(Vec<Json>, Option<String>), String> {
        let desde = match cursor {
            Some(c) => Some(de_hex(c).ok_or("`cursor` no es uno de los que da `derivations`")?),
            None => None,
        };
        let mut por_origen: BTreeMap<&str, Entrada<'_>> = BTreeMap::new();
        for it in self.items.iter().filter(|x| x.estado == "actual") {
            if let Some(l) = &it.linaje {
                por_origen
                    .entry(l.origen.as_str())
                    .or_insert_with(|| Entrada::Ficheros(l, Vec::new()))
                    .anadir(it);
            }
        }
        for m in self.marcas.iter().filter(|m| m.estado == "actual") {
            por_origen
                .entry(m.linaje.origen.as_str())
                .or_insert(Entrada::Marca(m));
        }
        let mut pagina = Vec::new();
        let mut ultimo = None;
        let mut mas = false;
        for (origen, e) in por_origen
            .iter()
            .filter(|(o, _)| desde.as_deref().is_none_or(|d| **o > d))
        {
            if pagina.len() == limite {
                mas = true;
                break;
            }
            pagina.push(e.json());
            ultimo = Some(*origen);
        }
        let siguiente = if mas { ultimo.map(hex) } else { None };
        Ok((pagina, siguiente))
    }
}

/// Una entrada del registro, mientras se junta.
enum Entrada<'a> {
    Ficheros(&'a Linaje, Vec<&'a Item>),
    Marca(&'a Marca),
}

impl<'a> Entrada<'a> {
    fn anadir(&mut self, it: &'a Item) {
        if let Entrada::Ficheros(_, v) = self {
            v.push(it);
        }
    }

    fn json(&self) -> Json {
        match self {
            Entrada::Ficheros(l, fs) => Json::obj([
                ("source", fuente(l)),
                ("derivation", crudo(&l.derivacion)),
                ("state", Json::s("files")),
                (
                    "files",
                    Json::Arr(
                        fs.iter()
                            .map(|f| {
                                Json::obj([
                                    ("path", Json::s(&f.camino)),
                                    (
                                        "anchor",
                                        crudo(&f.linaje.as_ref().and_then(|l| l.ancla.clone())),
                                    ),
                                ])
                            })
                            .collect(),
                    ),
                ),
            ]),
            Entrada::Marca(m) => Json::obj([
                ("source", fuente(&m.linaje)),
                ("derivation", crudo(&m.linaje.derivacion)),
                (
                    "state",
                    Json::s(if m.marca == "error" { "error" } else { "empty" }),
                ),
                ("files", Json::Arr(Vec::new())),
                ("error", crudo(&m.error)),
            ]),
        }
    }
}

/// Un JSON guardado en texto, tal cual; sin él, `null`.
fn crudo(v: &Option<String>) -> Json {
    Json::Crudo(v.clone().unwrap_or_else(|| "null".into()))
}

/// `{uri, digest}` de un origen.
fn fuente(l: &Linaje) -> Json {
    Json::obj([
        ("uri", Json::s(&l.uri)),
        (
            "digest",
            l.digest
                .as_ref()
                .map(Json::s)
                .unwrap_or(Json::Crudo("null".into())),
        ),
    ])
}

/// Lo que una referencia gana si el ítem es un fichero derivado (0049 B9):
/// `source` (con su `anchor`) y `derivation`.
trait ConLinaje {
    fn con_linaje(self, l: Option<&Linaje>) -> Json;
}

impl ConLinaje for Json {
    fn con_linaje(self, l: Option<&Linaje>) -> Json {
        let Some(l) = l else { return self };
        let (Json::Obj(mut m), Json::Obj(mut f)) = (self, fuente(l)) else {
            unreachable!("una referencia y una fuente son objetos")
        };
        f.insert("anchor".into(), crudo(&l.ancla));
        m.insert("source".into(), Json::Obj(f));
        m.insert("derivation".into(), crudo(&l.derivacion));
        Json::Obj(m)
    }
}

fn hex(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02x}")).collect()
}

fn de_hex(c: &str) -> Option<String> {
    if !c.len().is_multiple_of(2) {
        return None;
    }
    let bytes = (0..c.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&c[i..i + 2], 16).ok())
        .collect::<Option<Vec<u8>>>()?;
    String::from_utf8(bytes).ok()
}

/// **Un entero de una columna** (0049 B3·6). El manifiesto guarda `tamano` y
/// `entro` como `Integer` de OOS (`Int64`); leerlos como texto los dejaba
/// vacíos en TODOS los ítems —sin `size`, el SDK no verificaba la longitud ni
/// contaba un `seek` desde el final (medido en un puesto de victor)—. Un
/// listado viejo o de prueba que los traiga como texto también vale.
fn entero(l: &RecordBatch, n: &str, i: usize) -> Option<i64> {
    use arrow_array::types::{Int32Type, Int64Type};
    let c = l.column_by_name(n)?;
    if arrow_array::Array::is_null(c.as_ref(), i) {
        return None;
    }
    if let Some(a) = c.as_primitive_opt::<Int64Type>() {
        return Some(a.value(i));
    }
    if let Some(a) = c.as_primitive_opt::<Int32Type>() {
        return Some(i64::from(a.value(i)));
    }
    c.as_string_opt::<i32>()
        .and_then(|a| a.value(i).trim().parse().ok())
}

/// El cursor: el camino y la versión del último ítem dado, en hexadecimal. Es
/// opaco para quien lo recibe, y no lleva nada que no esté ya en la página.
fn cursor_de(it: &Item) -> String {
    format!("{}\u{0}{}", it.camino, it.version)
        .bytes()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn de_cursor(c: &str) -> Result<(String, String), String> {
    let malo = || "`cursor` no es uno de los que da `list`".to_string();
    if !c.len().is_multiple_of(2) {
        return Err(malo());
    }
    let bytes = (0..c.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&c[i..i + 2], 16))
        .collect::<Result<Vec<u8>, _>>()
        .map_err(|_| malo())?;
    let s = String::from_utf8(bytes).map_err(|_| malo())?;
    let (a, b) = s.split_once('\u{0}').ok_or_else(malo)?;
    Ok((a.to_string(), b.to_string()))
}

/// **Los índices vivos**, por `metadata_location`, con un tope de filas: se
/// desaloja el que más tiempo lleva sin usarse. Una transacción nueva tiene
/// otro `metadata_location`, así que nunca se sirve un índice viejo por uno
/// nuevo: el viejo se queda hasta que el tope lo saque.
pub struct Indices {
    max_filas: usize,
    vivos: Mutex<Vivos>,
}

/// Los índices por `metadata_location`, y el orden de uso (el primero, el que
/// más tiempo lleva sin usarse).
type Vivos = (HashMap<String, Arc<Indice>>, VecDeque<String>);

impl Indices {
    pub fn nuevo(max_filas: usize) -> Indices {
        Indices {
            max_filas,
            vivos: Mutex::new((HashMap::new(), VecDeque::new())),
        }
    }

    /// El índice de esa transacción; si no está, lo carga `cargar` (fuera del
    /// candado: dos peticiones a la vez pueden cargarlo las dos, y la segunda
    /// se queda con el primero que entró).
    pub fn obtener(
        &self,
        metadata_location: &str,
        cargar: impl FnOnce() -> Result<Indice, String>,
    ) -> Result<Arc<Indice>, String> {
        {
            let mut v = self.vivos.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(i) = v.0.get(metadata_location).cloned() {
                v.1.retain(|k| k != metadata_location);
                v.1.push_back(metadata_location.to_string());
                return Ok(i);
            }
        }
        let nuevo = Arc::new(cargar()?);
        let mut v = self.vivos.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(i) = v.0.get(metadata_location).cloned() {
            return Ok(i);
        }
        v.0.insert(metadata_location.to_string(), nuevo.clone());
        v.1.push_back(metadata_location.to_string());
        let mut filas: usize = v.0.values().map(|i| i.filas()).sum();
        while filas > self.max_filas && v.1.len() > 1 {
            let Some(viejo) = v.1.pop_front() else { break };
            if let Some(i) = v.0.remove(&viejo) {
                filas -= i.filas();
            }
        }
        Ok(nuevo)
    }

    pub fn cuantos(&self) -> usize {
        self.vivos.lock().unwrap_or_else(|e| e.into_inner()).0.len()
    }
}

#[cfg(test)]
pub mod pruebas {
    use super::*;
    use arrow_array::StringArray;
    use arrow_schema::{DataType, Field, Schema};

    /// `(camino, version, huella, estado, blob)`.
    pub type Fila<'a> = (&'a str, &'a str, &'a str, &'a str, Option<&'a str>);

    /// Un listado como el de `ore collections`.
    pub fn listado(filas: &[Fila<'_>]) -> RecordBatch {
        let cols = [
            "camino", "version", "huella", "estado", "blob", "tipo", "tamano",
        ];
        let esquema = Arc::new(Schema::new(
            cols.iter()
                .map(|c| Field::new(*c, DataType::Utf8, true))
                .collect::<Vec<_>>(),
        ));
        let col = |f: &dyn Fn(&Fila<'_>) -> Option<String>| {
            Arc::new(StringArray::from(filas.iter().map(f).collect::<Vec<_>>()))
                as arrow_array::ArrayRef
        };
        RecordBatch::try_new(
            esquema,
            vec![
                col(&|r| Some(r.0.to_string())),
                col(&|r| Some(r.1.to_string())),
                col(&|r| Some(r.2.to_string())),
                col(&|r| Some(r.3.to_string())),
                col(&|r| r.4.map(String::from)),
                col(&|_| Some("application/pdf".to_string())),
                col(&|_| Some("77".to_string())),
            ],
        )
        .unwrap()
    }

    /// Como lo escribe `ore collections`: `tamano` y `entro` son `Int64`.
    #[test]
    fn el_tamano_y_la_entrada_enteros_se_leen() {
        use arrow_array::Int64Array;
        let esquema = Arc::new(Schema::new(vec![
            Field::new("camino", DataType::Utf8, false),
            Field::new("version", DataType::Utf8, false),
            Field::new("huella", DataType::Utf8, false),
            Field::new("estado", DataType::Utf8, false),
            Field::new("tamano", DataType::Int64, true),
            Field::new("entro", DataType::Int64, true),
        ]));
        let s = |v: &[&str]| Arc::new(StringArray::from(v.to_vec())) as arrow_array::ArrayRef;
        let l = RecordBatch::try_new(
            esquema,
            vec![
                s(&["a.pdf", "b.pdf"]),
                s(&["null", "v1"]),
                s(&["etag:a", "etag:b"]),
                s(&["actual", "actual"]),
                Arc::new(Int64Array::from(vec![Some(717), None])),
                Arc::new(Int64Array::from(vec![Some(3), Some(1)])),
            ],
        )
        .unwrap();
        let ix = Indice::de_lotes("legal.archivo.contratos", true, "3", &[l]).unwrap();
        let a = ix.por_camino("a.pdf", None).unwrap();
        assert_eq!((a.tamano, a.entro.as_deref()), (Some(717), Some("3")));
        assert!(ix.referencia(a).jcs().contains("\"size\":717"));
        assert_eq!(ix.por_camino("b.pdf", None).unwrap().tamano, None);
    }

    fn indice() -> Indice {
        let l = listado(&[
            ("docs/c.pdf", "v2", "crc64nvme:C2", "actual", Some("bb")),
            ("docs/a.pdf", "v1", "crc64nvme:A", "actual", Some("aa")),
            ("docs/c.pdf", "v1", "crc64nvme:C1", "retirado", Some("cc")),
            ("img/b.png", "v1", "crc64nvme:B", "actual", Some("dd")),
            ("docs/d.pdf", "v1", "crc64nvme:A", "actual", Some("aa")),
            ("docs/x.pdf", "v1", "crc64nvme:X", "perdido", None),
        ]);
        Indice::de_lotes("legal.archivo.contratos", false, "7", &[l]).unwrap()
    }

    #[test]
    fn una_pagina_por_cursor_recorre_todo_una_vez() {
        let i = indice();
        let (p1, c1) = i.pagina(None, None, 2, Estado::Actual).unwrap();
        assert_eq!(
            p1.iter().map(|x| x.camino.as_str()).collect::<Vec<_>>(),
            ["docs/a.pdf", "docs/c.pdf"]
        );
        let (p2, c2) = i.pagina(None, c1.as_deref(), 2, Estado::Actual).unwrap();
        assert_eq!(
            p2.iter().map(|x| x.camino.as_str()).collect::<Vec<_>>(),
            ["docs/d.pdf", "img/b.png"]
        );
        assert_eq!(c2, None, "la última página no da cursor");
        let (todos, _) = i.pagina(None, None, 100, Estado::Todos).unwrap();
        assert_eq!(todos.len(), 6);
    }

    #[test]
    fn el_prefijo_acota_y_el_cursor_malo_se_dice() {
        let i = indice();
        let (p, _) = i.pagina(Some("img/"), None, 10, Estado::Actual).unwrap();
        assert_eq!(p.len(), 1);
        assert!(i.pagina(None, Some("zz"), 10, Estado::Actual).is_err());
    }

    #[test]
    fn stat_por_camino_y_la_version_actual() {
        let i = indice();
        let c = i.por_camino("docs/c.pdf", None).unwrap();
        assert_eq!(c.version, "v2");
        let viejo = i.por_camino("docs/c.pdf", Some("v1")).unwrap();
        assert!(!i.es_actual(viejo));
        assert!(i.es_actual(c));
        assert!(i.por_camino("docs/no.pdf", None).is_none());
    }

    #[test]
    fn por_contenido_la_actual_de_menor_camino_y_nunca_lo_perdido() {
        let i = indice();
        assert_eq!(i.por_contenido("crc64nvme:A").unwrap().camino, "docs/a.pdf");
        assert_eq!(i.por_contenido("sha256:aa").unwrap().camino, "docs/a.pdf");
        assert!(i.por_contenido("crc64nvme:X").is_none(), "perdido");
        assert_eq!(i.por_contenido("crc64nvme:C1").unwrap().estado, "retirado");
    }

    #[test]
    fn la_referencia_no_lleva_url() {
        let i = indice();
        let r = i
            .referencia(i.por_camino("docs/a.pdf", None).unwrap())
            .jcs();
        assert!(
            r.contains("\"uri\":\"ore://legal.archivo.contratos/docs/a.pdf?v=v1\""),
            "{r}"
        );
        assert!(r.contains("\"digest\":\"sha256:aa\""), "{r}");
        assert!(r.contains("\"size\":77"), "{r}");
        assert!(
            !r.to_lowercase().contains("signature") && !r.contains("https://"),
            "{r}"
        );
    }

    #[test]
    fn el_tope_desaloja_el_menos_usado() {
        let ix = Indices::nuevo(10);
        let carga = |n: usize| {
            let filas: Vec<(String, String)> =
                (0..n).map(|k| (format!("p{k}"), "v".to_string())).collect();
            move || {
                let f: Vec<(&str, &str, &str, &str, Option<&str>)> = filas
                    .iter()
                    .map(|(c, v)| (c.as_str(), v.as_str(), "h", "actual", None))
                    .collect();
                Indice::de_lotes("c", true, "1", &[listado(&f)])
            }
        };
        ix.obtener("m1", carga(6)).unwrap();
        ix.obtener("m2", carga(3)).unwrap();
        ix.obtener("m1", || unreachable!("está vivo")).unwrap();
        ix.obtener("m3", carga(4)).unwrap();
        assert_eq!(ix.cuantos(), 2, "13 filas > 10: sale m2, el menos usado");
        assert!(ix.obtener("m2", || Err("cargado otra vez".into())).is_err());
    }
}
