//! **ORE 0072 C2, fino (F2·1)**: el descriptor de una capa y sus teselas.
//!
//! - `GET /v1/layers/{layer}/descriptor` (con sujeto): resuelve la capa —hasta F2·2,
//!   la capa ES el dataset `base[.schema].nombre` (K-F2·1-a)— en la rama de `x-ore-rama`
//!   (sin ella, `main`), su columna geo y su
//!   CRS; calcula la **clave** (la huella de lo que se pinta: protocolo, capa,
//!   `metadata_location`, columna, CRS, suelo) y la guarda con su fuente; devuelve
//!   **TileJSON 3.0** con las URLs de las teselas **ya firmadas**.
//! - `GET /v1/tiles/{layer}/{clave}/WebMercatorQuad/{z}/{x}/{y}.mvt?e=…&f=…`
//!   (**sin sujeto**: la firma es la autorización, K-F2·1-b): firma y caducidad (403),
//!   clave conocida (410: el cliente pide otro descriptor), la credencial prestada
//!   del dataset y el motor (`ore-teselas`, C3). La tesela es **inmutable**: la clave
//!   cambia con la instantánea, así que se cachea para siempre (y el borde con ella).
//!
//! La tabla clave → fuente vive en memoria (K-F2·1-e): `ore-serve` es una réplica, y
//! una clave que no está es un 410 que se arregla pidiendo otro descriptor.
//!
//! **La pirámide** (F2·3·2·5, D-F2-13 a D-F2-18): por debajo del suelo de la capa, sus teselas
//! ya hechas en un PMTiles **bajo la tabla** (`<tabla>/_ore_piramides/<instantánea>-<receta>`),
//! así que la credencial prestada para el dataset la lee y muere con él. El descriptor
//! pregunta al motor si está (`POST /v1/piramide`): si está, `minzoom` 0 y la clave lleva la
//! pareja instantánea + pirámide (C4-d); si no, el suelo, y —con `ORE_PIRAMIDE_DISPARO`—
//! se pide construirla (el Job del sistema, F2·3·2·4). A quien mira con un filtro de fila (A8)
//! no se le sirve: la pirámide agrega todo (C4-f, falla cerrado). La tesela elige por zoom:
//! por debajo del suelo, la pirámide; desde él, el crudo.
//!
//! **El índice** (0072 I3·2): del suelo hacia arriba, una fuente grande se sirve de su índice
//! (`<tabla>/_ore_capa/i1/`, una tabla Iceberg del sistema ordenada por espacio, en 3857 y con
//! `_ore_id`), no de la tabla. El descriptor pregunta al motor (`POST /v1/indice`) y pinta **el
//! último momento completo**: con índice, su instantánea S —el crudo de su índice, la pirámide de
//! S—; nunca la pirámide de una instantánea y el crudo de otra. Si va por detrás, se sirve igual
//! y lo dice (`ore.indice.al_dia`); con `ORE_INDICE_DISPARO`, pide otro. Una fuente de menos de
//! `ORE_INDICE_DESDE_FILAS` filas no lo necesita y se sirve como siempre.
//!
//! **El clic** (I3·3): `GET /v1/layers/{layer}/features/{id}?clave=…` (con sujeto). Con índice,
//! cada elemento de la tesela lleva su id (`_ore_id`), que es su posición en la fuente (fichero
//! y fila, I1). La clave del descriptor dice de qué instantánea es: la fila sale de **esa**
//! instantánea de la tabla del cliente, entera, con los permisos de quien pide; una clave que ya
//! no está es un 410, como en las teselas.

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ore_core::json::Json;
use ore_entrada::http::{Bytes, Peticion, Respuesta, Salida};
use sha2::{Digest, Sha256};

use ore_entrada::identidad::Identidad;

use crate::cola;
use crate::rutas::Servidor;

pub(crate) const TMS: &str = "WebMercatorQuad";
const MAXZOOM: u32 = 16;
/// El suelo del crudo por defecto (K-F2·1-c: lo medido para los edificios en F2·0a).
const MINZOOM: u32 = 13;
/// Cuánto vale una firma: el descriptor se pide al abrir el mapa (F2·1; más corta, F5).
const VIDA_FIRMA: u64 = 12 * 3600;
/// Cuánto se usa una credencial prestada antes de pedir otra (STS da 1 h).
const VIDA_CREDENCIAL: u64 = 45 * 60;
const PROTOCOLO: u32 = 1;
/// La receta de la pirámide: entra en su nombre (D-F2-15) y sube cuando `ore-piramide` cambia
/// lo que construye (una pirámide de otra receta no se sirve: se rehace).
pub(crate) const RECETA: &str = "r1";
/// Lo que `datos` dirá cuando quien mira tenga un filtro de fila (A8, 0047): con él, sin
/// pirámide.
pub(crate) const FILTRO_DE_QUIEN_MIRA: &str = "filtro_de_quien_mira";

/// D-F2-18: ¿se pide construir la pirámide que falta? Apagado hasta que la plantilla del Job
/// (`malla/59-la-piramide.yaml`) esté en la cola de la celda: binario antes que malla.
fn disparo() -> bool {
    std::env::var("ORE_PIRAMIDE_DISPARO").is_ok_and(|v| v == "1")
}

/// I3·2: ¿se pide construir el índice que falta (o el que va por detrás)? Apagado: binario antes
/// que malla.
fn disparo_indice() -> bool {
    std::env::var("ORE_INDICE_DISPARO").is_ok_and(|v| v == "1")
}

/// Desde cuántas filas una fuente se sirve de su índice (`ORE_INDICE_DESDE_FILAS`, 5 M): por
/// debajo, la tesela del crudo cuesta poco aunque la tabla no esté ordenada (§E2·0: 20 M
/// desordenadas, ~640 ms por tesela; 30 M tumbaron el motor).
fn desde_filas() -> u64 {
    std::env::var("ORE_INDICE_DESDE_FILAS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5_000_000)
}

/// Lo que el índice de la receta `i1` es para el motor (los de `ore_teselas::indice`): su
/// geometría, ya en 3857, y su id.
const RECETA_INDICE: &str = "i1";
const INDICE_GEOMETRIA: &str = "geom";
const INDICE_CRS: &str = "EPSG:3857";
const INDICE_ID: &str = "_ore_id";

/// Cuánto se espera antes de volver a pedir la misma pirámide: lo que tarda una capa grande
/// en construirse. Pedirla es clonar la cola; sin esto, cada descriptor la clonaría.
const ESPERA_PEDIDA: Duration = Duration::from_secs(15 * 60);

/// Lo que el descriptor sabe hacer con la pirámide: preguntar al motor si está y, con el
/// disparo, pedir que se construya.
struct Piramides<'a> {
    preguntar: &'a dyn Fn(&str, &Json) -> Option<Json>,
    pedir: Option<&'a Pedir<'a>>,
}

/// `(ubicación de la pirámide, tabla, petición) → lo que se hizo`.
type Pedir<'a> = dyn Fn(&str, &str, &str) -> Result<String, String> + 'a;

/// Cómo se rinde un Job en la cola: `cola::rendir_piramide` o `cola::rendir_indice`.
type Rendir = fn(&str, &str, &str) -> Result<(String, String), String>;

/// Lo que el descriptor sabe hacer con el índice (I3·2): preguntar al motor
/// (`(tabla, metadata_location, credencial) →` lo de `POST /v1/indice`) y, con el disparo, pedir
/// que se construya. `desde_filas`: el umbral ([`desde_filas`]).
struct Indices<'a> {
    preguntar: &'a dyn Fn(&str, &str, &Json) -> Option<Json>,
    pedir: Option<&'a Pedir<'a>>,
    desde_filas: u64,
}

fn pedidas() -> &'static Mutex<HashMap<String, SystemTime>> {
    static T: OnceLock<Mutex<HashMap<String, SystemTime>>> = OnceLock::new();
    T.get_or_init(Default::default)
}

/// Encola el Job de la pirámide o del índice (`malla/59-la-piramide.yaml`, con `rendir`) en la
/// cola de la celda, una vez por `ubicacion` cada [`ESPERA_PEDIDA`]. La misma petición rinde el
/// mismo Job: si ya estaba, no se publica nada.
fn encolar(
    srv: &Servidor,
    sujeto: &Identidad,
    (ubicacion, tabla, peticion): (&str, &str, &str),
    rendir: Rendir,
    que: &str,
) -> Result<String, String> {
    {
        let mut t = pedidas().lock().unwrap();
        let ahora = SystemTime::now();
        t.retain(|_, c| ahora.duration_since(*c).unwrap_or_default() < ESPERA_PEDIDA);
        if t.contains_key(ubicacion) {
            return Ok("ya pedida".into());
        }
        t.insert(ubicacion.to_string(), ahora);
    }
    let r = (|| {
        let forja = srv
            .cola
            .as_ref()
            .ok_or("este servidor no sabe de ninguna cola (`--cola`)")?;
        let prestado = forja
            .clonar()
            .map_err(|e| format!("no se pudo leer la cola: {e}"))?;
        let dir = prestado.ruta();
        let plantilla =
            std::fs::read_to_string(dir.join(cola::PLANTILLA_PIRAMIDE)).map_err(|_| {
                format!(
                    "la cola no trae `{}`: hay que converger este inquilino",
                    cola::PLANTILLA_PIRAMIDE
                )
            })?;
        let (fichero, texto) = rendir(&plantilla, tabla, peticion)?;
        std::fs::write(dir.join(&fichero), &texto)
            .map_err(|e| format!("no se pudo escribir `{fichero}`: {e}"))?;
        if !forja.hay_cambios(dir) {
            return Ok(format!("ya encolada como `{fichero}`"));
        }
        forja
            .publicar(dir, sujeto, &format!("Construir {que} de `{tabla}`"))
            .map(|c| format!("encolada como `{fichero}` · commit {c}"))
            .map_err(|e| format!("no se pudo encolar: {e}"))
    })();
    if r.is_err() {
        // que el siguiente descriptor lo vuelva a intentar
        pedidas().lock().unwrap().remove(ubicacion);
    }
    r
}

#[derive(Clone)]
struct Fuente {
    capa: String,
    /// La rama del descriptor: la credencial se renueva en ella.
    rama: Option<String>,
    /// Lo que lee el crudo: la tabla del dataset o, con índice, el índice (I3·2).
    metadata_location: String,
    geometria: String,
    crs: String,
    /// La columna del id de cada elemento (con índice, `_ore_id`).
    id: Option<String>,
    /// De dónde sale la fila de un elemento, con índice (I3·3).
    clic: Option<Clic>,
    /// El primer zoom que se sirve: 0 con pirámide, el suelo sin ella.
    minzoom: u32,
    /// El primer zoom del crudo.
    suelo: u32,
    /// La pirámide (su ubicación), si está y quien mira puede verla.
    piramide: Option<String>,
    credencial: Json,
    credencial_hasta: u64,
}

fn tabla() -> &'static Mutex<HashMap<String, Fuente>> {
    static T: OnceLock<Mutex<HashMap<String, Fuente>>> = OnceLock::new();
    T.get_or_init(Default::default)
}

/// El secreto de la firma: el de la celda (`ORE_TESELAS_SECRETO`) o, sin él, uno al
/// azar por proceso (las claves también viven en memoria: mueren juntos).
fn secreto() -> &'static [u8; 32] {
    static S: OnceLock<[u8; 32]> = OnceLock::new();
    S.get_or_init(|| {
        if let Ok(s) = std::env::var("ORE_TESELAS_SECRETO")
            && !s.is_empty()
        {
            return sha256(s.as_bytes());
        }
        let mut b = [0u8; 32];
        if std::fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut b))
            .is_ok()
        {
            return b;
        }
        use std::hash::{BuildHasher, Hasher};
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(ahora_ns());
        h.write_u32(std::process::id());
        sha256(&h.finish().to_le_bytes())
    })
}

fn sha256(b: &[u8]) -> [u8; 32] {
    Sha256::digest(b).into()
}

pub(crate) fn hmac(clave: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    k[..clave.len().min(64)].copy_from_slice(&clave[..clave.len().min(64)]);
    let mut i = Sha256::new();
    i.update(k.map(|b| b ^ 0x36));
    i.update(msg);
    let mut o = Sha256::new();
    o.update(k.map(|b| b ^ 0x5c));
    o.update(i.finalize());
    o.finalize().into()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn ahora_ns() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

fn ahora() -> u64 {
    (ahora_ns() / 1_000_000_000) as u64
}

/// La firma de una clave hasta `e` (32 hex: 128 bits).
pub(crate) fn firma(clave: &str, e: u64) -> String {
    hex(&hmac(secreto(), format!("{clave}|{e}").as_bytes()))[..32].to_string()
}

/// Comparar sin dar pistas por el tiempo.
fn iguales(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

fn consulta(cruda: &str) -> HashMap<&str, &str> {
    cruda
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .collect()
}

type Datos = (String, String, String, Json, bool);

/// Lo que el dataset dice de sí en `rama` (`datos_en`, con su vuelta a `main`):
/// `(metadata_location, columna, crs, credencial, ¿filtro de quien mira?)`.
fn resolver(srv: &Servidor, capa: &str, rama: Option<&str>) -> Result<Datos, Respuesta> {
    de_los_datos(srv.datos_en(rama.map(String::from), capa), capa)
}

fn de_los_datos(r: Respuesta, capa: &str) -> Result<Datos, Respuesta> {
    if r.codigo != 200 {
        return Err(r);
    }
    let Json::Obj(m) = &r.cuerpo else {
        return Err(Respuesta::error(500, "`datos` no devolvió un objeto"));
    };
    let ml = match m.get("metadata_location") {
        Some(Json::Str(s)) if !s.is_empty() => s.clone(),
        _ => {
            return Err(Respuesta::error(
                409,
                format!(
                    "`{capa}` no tiene una copia Iceberg que pintar (un sobre heredado no se tesela)"
                ),
            ));
        }
    };
    let Some(Json::Obj(geo)) = m.get("geo") else {
        return Err(Respuesta::error(
            422,
            format!("`{capa}` no tiene columnas geo"),
        ));
    };
    // La primera columna geo (por nombre): una capa pinta una geometría (F5: elegirla).
    let Some((col, g)) = geo.iter().next() else {
        return Err(Respuesta::error(
            422,
            format!("`{capa}` no tiene columnas geo"),
        ));
    };
    let crs = match g {
        Json::Obj(g) => match g.get("crs") {
            Some(Json::Str(c)) => c.clone(),
            _ => "OGC:CRS84".to_string(),
        },
        _ => "OGC:CRS84".to_string(),
    };
    let credencial = m
        .get("credencial")
        .cloned()
        .unwrap_or(Json::Obj(Default::default()));
    let filtro = match m.get(FILTRO_DE_QUIEN_MIRA) {
        None | Some(Json::Bool(false)) => false,
        Some(Json::Str(s)) => !s.is_empty(),
        Some(Json::Obj(o)) => !o.is_empty(),
        Some(Json::Arr(a)) => !a.is_empty(),
        Some(_) => true,
    };
    Ok((ml, col.clone(), crs, credencial, filtro))
}

/// Dónde vive la pirámide de una instantánea (D-F2-15): bajo la tabla, junto a `metadata/` y
/// `data/` (la recogida de la tabla sólo mira esas dos; la de los datasets, lo que cuelga de un
/// dataset vivo), con el nombre del `metadata.json` y la receta.
pub(crate) fn ubicacion_de_la_piramide(ml: &str) -> Option<String> {
    let (tabla, fichero) = ml.rsplit_once("/metadata/")?;
    let instantanea = fichero.strip_suffix(".metadata.json").unwrap_or(fichero);
    (!tabla.is_empty() && !instantanea.is_empty())
        .then(|| format!("{tabla}/_ore_piramides/{instantanea}-{RECETA}.pmtiles"))
}

/// Lo que se sabe de la pirámide al hacer el descriptor: `(ubicación servible, estado)`.
fn estado_de_la_piramide(
    ml: &str,
    credencial: &Json,
    filtro: bool,
    pir: &Piramides,
    peticion_de: &dyn Fn(&str) -> String,
) -> (Option<String>, Json) {
    let Some(u) = ubicacion_de_la_piramide(ml) else {
        return (None, Json::obj([("estado", Json::s("ausente"))]));
    };
    if filtro {
        return (
            None,
            Json::obj([
                ("estado", Json::s("no")),
                (
                    "motivo",
                    Json::s("quien mira tiene un filtro de fila: la pirámide agrega todo (C4-f)"),
                ),
            ]),
        );
    }
    match (pir.preguntar)(&u, credencial) {
        // F2·3·2·6: no está y el Job que la construía falló; no se vuelve a pedir (el mismo Job
        // es el mismo fichero en la cola): se dice por qué
        Some(Json::Obj(r)) if r.contains_key("fallo") => {
            let motivo = match r.get("fallo") {
                Some(Json::Obj(f)) => match f.get("error") {
                    Some(Json::Str(e)) => e.clone(),
                    _ => "el Job de la pirámide falló".into(),
                },
                _ => "el Job de la pirámide falló".into(),
            };
            (
                None,
                Json::obj([("estado", Json::s("fallida")), ("motivo", Json::s(motivo))]),
            )
        }
        Some(Json::Obj(r)) => {
            let mut e = Json::obj([("estado", Json::s("lista"))]);
            if let Json::Obj(m) = &mut e {
                for k in ["minzoom", "maxzoom"] {
                    if let Some(v) = r.get(k) {
                        m.insert(k.into(), v.clone());
                    }
                }
                if let Some(Json::Obj(man)) = r.get("manifiesto")
                    && let Some(t) = man.get("tipo")
                {
                    m.insert("tipo".into(), t.clone());
                }
            }
            (Some(u), e)
        }
        _ => {
            let Some(pedir) = pir.pedir else {
                return (None, Json::obj([("estado", Json::s("ausente"))]));
            };
            let tabla = ml.rsplit_once("/metadata/").map_or(ml, |(t, _)| t);
            match pedir(&u, tabla, &peticion_de(&u)) {
                Ok(dicho) => {
                    eprintln!("pirámide · {u} · {dicho}");
                    (None, Json::obj([("estado", Json::s("construyendo"))]))
                }
                Err(e) => {
                    eprintln!("pirámide · {u} · NO pedida: {e}");
                    (
                        None,
                        Json::obj([("estado", Json::s("ausente")), ("motivo", Json::s(e))]),
                    )
                }
            }
        }
    }
}

/// Lo que lee `ore-piramide` (`malla/59-la-piramide.yaml`): la instantánea, la capa y el destino;
/// sin credenciales (el Job usa la identidad del driver) y en una línea. Sin suelo: lo decide el
/// tipo, que sólo se sabe al leer (13, o 14 para las líneas), y el descriptor lo toma de la pirámide.
fn peticion_de_la_piramide(
    ml: &str,
    geometria: &str,
    crs: &str,
    capa: &str,
    destino: &str,
) -> String {
    Json::obj([
        (
            "fuente",
            Json::obj([
                ("metadata_location", Json::s(ml)),
                ("geometria", Json::s(geometria)),
                ("crs", Json::s(crs)),
            ]),
        ),
        ("capa", Json::s(capa)),
        ("destino", Json::obj([("ubicacion", Json::s(destino))])),
        ("limites", Json::obj([("segundos", Json::Int(3000))])),
        ("trabajo", Json::s("/trabajo")),
    ])
    .jcs()
}

/// `GET /v1/layers/{layer}/descriptor`: TileJSON 3.0, con las teselas firmadas.
/// La clave no lleva la rama: la lleva su `metadata_location` (otra instantánea, otra
/// clave; la misma, las mismas teselas).
pub(crate) fn descriptor(
    srv: &Servidor,
    p: &Peticion,
    capa: &str,
    rama: Option<&str>,
    sujeto: &Identidad,
) -> Respuesta {
    let pedir_piramide = |u: &str, tabla: &str, peticion: &str| {
        encolar(
            srv,
            sujeto,
            (u, tabla, peticion),
            cola::rendir_piramide,
            "la pirámide",
        )
    };
    let pedir_indice = |u: &str, tabla: &str, peticion: &str| {
        encolar(
            srv,
            sujeto,
            (u, tabla, peticion),
            cola::rendir_indice,
            "el índice",
        )
    };
    let pir = Piramides {
        preguntar: &piramide_al_motor,
        pedir: disparo().then_some(&pedir_piramide as &Pedir<'_>),
    };
    let idx = Indices {
        preguntar: &indice_al_motor,
        pedir: disparo_indice().then_some(&pedir_indice as &Pedir<'_>),
        desde_filas: desde_filas(),
    };
    descriptor_con(resolver(srv, capa, rama), p, capa, rama, &pir, &idx)
}

/// `POST /v1/indice` al motor (I3·2): `{indice?, instantanea_ms?, fuente{filas, secuencia}}`, o
/// `None` si no se pudo saber (entonces se sirve sin índice, como antes).
fn indice_al_motor(tabla: &str, ml: &str, credencial: &Json) -> Option<Json> {
    let cuerpo = Json::obj([
        ("ubicacion", Json::s(tabla)),
        ("metadata_location", Json::s(ml)),
        ("credencial", credencial.clone()),
    ])
    .jcs();
    let motor = std::env::var("ORE_TESELAS").unwrap_or_else(|_| "ore-teselas:8097".into());
    match al_motor_en(&motor, "/v1/indice", &cuerpo) {
        Ok((200, _, b)) => ore_core::parse::parse(&String::from_utf8_lossy(&b))
            .ok()
            .map(|n| Json::de_node(&n)),
        Ok((c, _, b)) => {
            eprintln!(
                "índice · {tabla} · el motor contestó {c}: {}",
                String::from_utf8_lossy(&b)
            );
            None
        }
        Err(e) => {
            eprintln!("índice · {tabla} · {e}");
            None
        }
    }
}

/// Lo que lee el crudo (del suelo hacia arriba): la tabla del dataset o su índice.
struct Crudo {
    metadata_location: String,
    geometria: String,
    crs: String,
    id: Option<String>,
    clic: Option<Clic>,
}

/// De dónde sale la fila de un elemento (I3·3): la instantánea que pinta el índice y su tabla de
/// ordinales de ficheros (la clave del objeto, en el cubo de la tabla). Sólo con identidad por
/// posición; con otra, el clic aún no (I6).
#[derive(Clone)]
struct Clic {
    instantanea: String,
    ficheros: String,
}

/// Lo que el índice manda (`ore-piramide` con `"tarea": "indice"`): la instantánea, su geometría,
/// la capa, el suelo (para partir lo largo) y el destino, la tabla; sin credenciales ni
/// columnas (las teselas aún no llevan propiedades), en una línea.
fn peticion_del_indice(
    ml: &str,
    geometria: &str,
    crs: &str,
    capa: &str,
    suelo: u32,
    tabla: &str,
) -> String {
    Json::obj([
        ("tarea", Json::s("indice")),
        (
            "fuente",
            Json::obj([
                ("metadata_location", Json::s(ml)),
                ("geometria", Json::s(geometria)),
                ("crs", Json::s(crs)),
                ("columnas", Json::Arr(vec![])),
            ]),
        ),
        ("capa", Json::s(capa)),
        ("suelo", Json::Int(suelo as i64)),
        ("destino", Json::obj([("ubicacion", Json::s(tabla))])),
        ("trabajo", Json::s("/trabajo")),
    ])
    .jcs()
}

/// Un momento que el índice puede pintar (I4·5): lo que lee el crudo, la instantánea del índice,
/// su secuencia y lo que el descriptor dice de él.
struct Momento {
    crudo: Crudo,
    instantanea: String,
    secuencia: u64,
    campos: Vec<(&'static str, Json)>,
}

/// **El índice al hacer el descriptor** (I3·2): `(los momentos que puede pintar, del más nuevo al
/// más viejo —vacío si no hay índice que servir—; su estado)`. Se sirve si el vigente es de esta
/// instantánea o de una anterior (si va por detrás, se dice y, con el disparo, se pide otro); nunca
/// uno de una instantánea posterior (una rama que lee algo más viejo). Sin él: por debajo del
/// umbral, no hace falta; por encima, se pide. Con un filtro de fila de quien mira, no: el motor
/// aún no lo aplica (A8). Cuál de los momentos se pinta lo decide la pirámide (I4·5).
#[allow(clippy::too_many_arguments)]
fn estado_del_indice(
    ml: &str,
    geometria: &str,
    crs: &str,
    capa: &str,
    suelo: u32,
    credencial: &Json,
    filtro: bool,
    idx: &Indices,
) -> (Vec<Momento>, Json) {
    let Some((tabla, fichero)) = ml.rsplit_once("/metadata/") else {
        return (vec![], Json::obj([("estado", Json::s("ausente"))]));
    };
    if filtro {
        return (
            vec![],
            Json::obj([
                ("estado", Json::s("no")),
                (
                    "motivo",
                    Json::s("quien mira tiene un filtro de fila y el motor aún no lo aplica (A8)"),
                ),
            ]),
        );
    }
    let Some(Json::Obj(r)) = (idx.preguntar)(tabla, ml, credencial) else {
        return (
            vec![],
            Json::obj([
                ("estado", Json::s("ausente")),
                ("motivo", Json::s("el motor no contestó")),
            ]),
        );
    };
    let de = |o: Option<&Json>, k: &str| match o {
        Some(Json::Obj(m)) => m.get(k).cloned(),
        _ => None,
    };
    let num = |o: Option<&Json>, k: &str| match de(o, k) {
        Some(Json::Str(v)) => v.parse::<u64>().ok(),
        Some(Json::Int(v)) => u64::try_from(v).ok(),
        _ => None,
    };
    let texto = |o: Option<&Json>, k: &str| match de(o, k) {
        Some(Json::Str(v)) if !v.is_empty() => Some(v),
        _ => None,
    };
    let fuente = r.get("fuente");
    let (filas, secuencia) = (num(fuente, "filas"), num(fuente, "secuencia"));
    // pedirlo: una vez por instantánea (otra instantánea, otro Job)
    let pedir = || {
        let instantanea = fichero.strip_suffix(".metadata.json").unwrap_or(fichero);
        idx.pedir.map(|p| {
            let u = format!("{tabla}/_ore_capa/{RECETA_INDICE}/{instantanea}");
            let r = p(
                &u,
                tabla,
                &peticion_del_indice(ml, geometria, crs, capa, suelo, tabla),
            );
            match &r {
                Ok(d) => eprintln!("índice · {u} · {d}"),
                Err(e) => eprintln!("índice · {u} · NO pedido: {e}"),
            }
            r
        })
    };
    // un índice que se puede pintar: `{indice, instantanea_ms?}` del motor
    let momento_de = |r: &BTreeMap<String, Json>| -> Option<Momento> {
        let man = r.get("indice");
        let (s, ml_i, sec_i) = (
            texto(man, "snapshot_fuente")?,
            texto(man, "indice")?,
            num(man, "secuencia_fuente")?,
        );
        let mut campos = vec![("instantanea", Json::s(&s))];
        for k in ["generacion", "filas", "profundidad"] {
            if let Some(v) = de(man, k) {
                campos.push((k, v));
            }
        }
        if let Some(v) = r.get("instantanea_ms") {
            campos.push(("instantanea_ms", v.clone()));
        }
        let posicion = matches!(
            de(man, "identidad"),
            Some(Json::Obj(i)) if matches!(i.get("tipo"), Some(Json::Str(t)) if t == "posicion")
        );
        Some(Momento {
            crudo: Crudo {
                metadata_location: ml_i,
                geometria: INDICE_GEOMETRIA.into(),
                crs: INDICE_CRS.into(),
                id: Some(INDICE_ID.into()),
                clic: texto(man, "ficheros")
                    .filter(|_| posicion)
                    .map(|ficheros| Clic {
                        instantanea: s.clone(),
                        ficheros,
                    }),
            },
            instantanea: s,
            secuencia: sec_i,
            campos,
        })
    };
    if let Some(ultimo) = momento_de(&r) {
        if secuencia.is_none_or(|sec| ultimo.secuencia > sec) {
            return (
                vec![],
                Json::obj([
                    ("estado", Json::s("no")),
                    (
                        "motivo",
                        Json::s("el índice es de una instantánea posterior a la que se pinta"),
                    ),
                ]),
            );
        }
        let mut e = vec![("estado", Json::s("lista"))];
        if ultimo.instantanea != ml {
            match pedir() {
                Some(Ok(_)) => e.push(("poniendose_al_dia", Json::Bool(true))),
                Some(Err(m)) => e.push(("motivo", Json::s(m))),
                None => {}
            }
        }
        // I4·5: los de antes, por si la pirámide del último aún no está
        let anteriores: Vec<Momento> = match r.get("anteriores") {
            Some(Json::Arr(a)) => a
                .iter()
                .filter_map(|j| match j {
                    Json::Obj(o) => momento_de(o),
                    _ => None,
                })
                .filter(|m| m.secuencia < ultimo.secuencia)
                .collect(),
            _ => vec![],
        };
        let mut momentos = vec![ultimo];
        momentos.extend(anteriores);
        return (momentos, Json::obj(e));
    }
    if filas.is_some_and(|f| f < idx.desde_filas) {
        return (
            vec![],
            Json::obj([
                ("estado", Json::s("no_hace_falta")),
                ("filas", Json::s(filas.unwrap_or(0).to_string())),
            ]),
        );
    }
    let e = match pedir() {
        Some(Ok(_)) => Json::obj([("estado", Json::s("construyendo"))]),
        Some(Err(m)) => Json::obj([("estado", Json::s("ausente")), ("motivo", Json::s(m))]),
        None => Json::obj([("estado", Json::s("ausente"))]),
    };
    (vec![], e)
}

/// `POST /v1/piramide` al motor: su resumen, o `None` (no está, o no se pudo saber: entonces
/// se sirve sin pirámide, como antes).
fn piramide_al_motor(ubicacion: &str, credencial: &Json) -> Option<Json> {
    let cuerpo = Json::obj([
        ("ubicacion", Json::s(ubicacion)),
        ("credencial", credencial.clone()),
    ])
    .jcs();
    let motor = std::env::var("ORE_TESELAS").unwrap_or_else(|_| "ore-teselas:8097".into());
    match al_motor_en(&motor, "/v1/piramide", &cuerpo) {
        Ok((200, _, b)) => ore_core::parse::parse(&String::from_utf8_lossy(&b))
            .ok()
            .map(|n| Json::de_node(&n)),
        // el motor dice `fallo` si el Job falló (F2·3·2·6); sin él, que no está
        Ok((404, _, b)) => ore_core::parse::parse(&String::from_utf8_lossy(&b))
            .ok()
            .map(|n| Json::de_node(&n))
            .filter(|j| matches!(j, Json::Obj(m) if m.contains_key("fallo"))),
        Ok((c, _, b)) => {
            eprintln!(
                "pirámide · {ubicacion} · el motor contestó {c}: {}",
                String::from_utf8_lossy(&b)
            );
            None
        }
        Err(e) => {
            eprintln!("pirámide · {ubicacion} · {e}");
            None
        }
    }
}

fn descriptor_con(
    datos: Result<Datos, Respuesta>,
    p: &Peticion,
    capa: &str,
    rama: Option<&str>,
    pir: &Piramides,
    idx: &Indices,
) -> Respuesta {
    let (ml, geometria, crs, credencial, filtro) = match datos {
        Ok(v) => v,
        Err(r) => return r,
    };
    let q = consulta(&p.consulta_cruda);
    // `minzoom` en la consulta es el suelo del crudo (K-F2·1-c); con pirámide se ve desde z0
    let suelo = q
        .get("minzoom")
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|z| *z <= MAXZOOM)
        .unwrap_or(MINZOOM);
    // el crudo, de la tabla o de su índice (I3·2)
    let (mut momentos, mut estado_indice) =
        estado_del_indice(&ml, &geometria, &crs, capa, suelo, &credencial, filtro, idx);
    // la pirámide de un momento: con el suelo en z0 todo es crudo y no hay pirámide que pedir; y la
    // de un momento que ya pasó se sirve si está, pero no se pide (se pide la del nuevo cuando
    // llegue su índice)
    let piramide_de = |momento: &str, pedir: bool| {
        let pir = Piramides {
            preguntar: pir.preguntar,
            pedir: pir.pedir.filter(|_| pedir && suelo > 0 && momento == ml),
        };
        let peticion_de = |u: &str| peticion_de_la_piramide(momento, &geometria, &crs, capa, u);
        estado_de_la_piramide(momento, &credencial, filtro, &pir, &peticion_de)
    };
    // **el último momento completo** (I4·5): el más nuevo de los del índice cuya pirámide esté; si
    // no está la de ninguno, el más nuevo, del suelo hacia arriba (y se pide su pirámide)
    let mut esperando = None;
    let mut elegido = None;
    for (i, m) in momentos.iter().enumerate() {
        let (p, e) = piramide_de(&m.instantanea, i == 0);
        if p.is_some() {
            elegido = Some((i, p, e));
            break;
        }
        if i == 0 {
            esperando = Some(e);
        }
    }
    let (crudo, piramide, estado) = if momentos.is_empty() {
        let (p, e) = piramide_de(&ml, true);
        let crudo = Crudo {
            metadata_location: ml.clone(),
            geometria: geometria.clone(),
            crs: crs.clone(),
            id: None,
            clic: None,
        };
        (crudo, p, e)
    } else {
        let ultimo = momentos[0].instantanea.clone();
        let (i, p, e) = match elegido {
            Some(v) => v,
            None => (
                0,
                None,
                esperando
                    .take()
                    .unwrap_or_else(|| Json::obj([("estado", Json::s("ausente"))])),
            ),
        };
        let m = momentos.swap_remove(i);
        if let Json::Obj(o) = &mut estado_indice {
            o.insert("al_dia".into(), Json::Bool(m.instantanea == ml));
            for (k, v) in m.campos {
                o.insert(k.into(), v);
            }
            // se pinta uno de antes porque la pirámide del último aún no está: se dice cuál espera
            // y cómo va su pirámide
            if i > 0
                && let Some(e) = esperando
            {
                o.insert(
                    "esperando".into(),
                    Json::obj([("instantanea", Json::s(ultimo)), ("piramide", e)]),
                );
            }
        }
        (m.crudo, p, e)
    };
    // con pirámide, el suelo es el suyo: donde acaba (su último zoom + 1), lo decidiera quien la
    // construyó por el tipo o no
    let suelo = match (&piramide, &estado) {
        (Some(_), Json::Obj(m)) => match m.get("maxzoom") {
            Some(Json::Int(z)) if (0..MAXZOOM as i64).contains(z) => *z as u32 + 1,
            _ => suelo,
        },
        _ => suelo,
    };
    let minzoom = if piramide.is_some() { 0 } else { suelo };
    // la pareja (C4-d): otra pirámide —o ninguna— es otra clave, y otras teselas; con índice, el
    // crudo es el índice (otro índice, otra clave) y lleva su id
    let clave = hex(&sha256(
        format!(
            "v{PROTOCOLO}|{capa}|{}|{}|{}|{suelo}|{}{}",
            crudo.metadata_location,
            crudo.geometria,
            crudo.crs,
            piramide.as_deref().unwrap_or(""),
            crudo
                .id
                .as_deref()
                .map(|i| format!("|{i}"))
                .unwrap_or_default()
        )
        .as_bytes(),
    ))[..32]
        .to_string();
    tabla().lock().unwrap().insert(
        clave.clone(),
        Fuente {
            capa: capa.to_string(),
            rama: rama.map(String::from),
            metadata_location: crudo.metadata_location,
            geometria: crudo.geometria,
            crs: crudo.crs,
            id: crudo.id,
            clic: crudo.clic,
            minzoom,
            suelo,
            piramide,
            credencial,
            credencial_hasta: ahora() + VIDA_CREDENCIAL,
        },
    );
    let e = ahora() + VIDA_FIRMA;
    let f = firma(&clave, e);
    // Las teselas, por la entrada por la que llegó el descriptor (la de la celda).
    let host = p
        .cabeceras
        .get("x-forwarded-host")
        .or_else(|| p.cabeceras.get("host"))
        .cloned()
        .unwrap_or_default();
    let esquema = p
        .cabeceras
        .get("x-forwarded-proto")
        .cloned()
        .unwrap_or_else(|| {
            if host.starts_with("localhost") || host.starts_with("127.") {
                "http".into()
            } else {
                "https".into()
            }
        });
    let url = format!(
        "{esquema}://{host}/v1/tiles/{capa}/{clave}/{TMS}/{{z}}/{{x}}/{{y}}.mvt?e={e}&f={f}"
    );
    let z = |v: u32| Json::Int(v as i64);
    Respuesta::ok(Json::obj([
        ("tilejson", Json::s("3.0.0")),
        ("name", Json::s(capa)),
        ("scheme", Json::s("xyz")),
        ("tiles", Json::Arr(vec![Json::s(url)])),
        ("minzoom", z(minzoom)),
        ("maxzoom", z(MAXZOOM)),
        (
            "bounds",
            Json::Crudo("[-180,-85.0511287798066,180,85.0511287798066]".into()),
        ),
        (
            "vector_layers",
            Json::Arr(vec![Json::obj([
                ("id", Json::s(capa)),
                ("fields", Json::Obj(Default::default())),
                ("minzoom", z(minzoom)),
                ("maxzoom", z(MAXZOOM)),
            ])]),
        ),
        ("attribution", Json::s(format!("ORE · {capa}"))),
        (
            "ore",
            Json::obj([
                ("clave", Json::s(&clave)),
                ("metadata_location", Json::s(ml)),
                ("geometria", Json::s(geometria)),
                ("expira", Json::Int(e as i64)),
                ("suelo", z(suelo)),
                ("piramide", estado),
                ("indice", estado_indice),
            ]),
        ),
    ]))
}

/// **El clic** (I3·3): `GET /v1/layers/{capa}/features/{id}?clave=…`.
pub(crate) fn elemento(srv: &Servidor, p: &Peticion, capa: &str, id: &str) -> Respuesta {
    elemento_con(
        p,
        capa,
        id,
        &|rama| resolver(srv, capa, rama),
        &fila_al_motor,
    )
}

/// Una llamada al motor: `cuerpo → (código, respuesta)`.
type AlMotor<'a> = dyn Fn(&str) -> Result<(u16, Json), String> + 'a;

/// Los bits bajos del id: la fila dentro de su fichero (`ordinal << 40 | fila`, I1).
const BITS_DE_FILA: u32 = 40;

fn elemento_con(
    p: &Peticion,
    capa: &str,
    id: &str,
    resolver: &dyn Fn(Option<&str>) -> Result<Datos, Respuesta>,
    motor: &AlMotor<'_>,
) -> Respuesta {
    let q = consulta(&p.consulta_cruda);
    let Some(clave) = q.get("clave") else {
        return Respuesta::error(422, "falta `clave`: la del descriptor de la capa");
    };
    let Some(f) = tabla().lock().unwrap().get(*clave).cloned() else {
        return Respuesta::error(410, "esa clave ya no está: pide otro descriptor");
    };
    if f.capa != capa {
        return Respuesta::error(404, "esa clave no es de esta capa");
    }
    let Ok(id) = id.parse::<u64>() else {
        return Respuesta::error(422, "el id de un elemento es un entero");
    };
    let Some(clic) = &f.clic else {
        return Respuesta::error(
            409,
            "esta capa no se sirve de un índice: sus elementos no llevan id",
        );
    };
    // quien pide puede ver la capa ahora, en la rama del descriptor; y sin filtro de fila (A8)
    let (_, _, _, credencial, filtro) = match resolver(f.rama.as_deref()) {
        Ok(v) => v,
        Err(r) => return r,
    };
    if filtro {
        return Respuesta::error(
            403,
            "quien mira tiene un filtro de fila y el clic aún no lo aplica (A8)",
        );
    }
    let cuerpo = Json::obj([
        ("metadata_location", Json::s(&clic.instantanea)),
        ("ficheros", Json::s(&clic.ficheros)),
        ("ordinal", Json::s((id >> BITS_DE_FILA).to_string())),
        (
            "fila",
            Json::s((id & ((1u64 << BITS_DE_FILA) - 1)).to_string()),
        ),
        ("credencial", credencial),
    ])
    .jcs();
    match motor(&cuerpo) {
        Ok((200, Json::Obj(m))) => Respuesta::ok(Json::obj([
            ("capa", Json::s(capa)),
            ("id", Json::s(id.to_string())),
            ("instantanea", Json::s(&clic.instantanea)),
            (
                "propiedades",
                m.get("propiedades")
                    .cloned()
                    .unwrap_or(Json::Obj(Default::default())),
            ),
        ])),
        Ok((422, _)) => Respuesta::error(
            404,
            "ese elemento no está en la instantánea que pinta la capa",
        ),
        Ok((c, j)) => {
            eprintln!("clic · {capa}/{id} · el motor contestó {c}: {}", j.jcs());
            Respuesta::error(502, format!("el motor contestó {c}"))
        }
        Err(e) => {
            eprintln!("clic · {capa}/{id} · {e}");
            Respuesta::error(502, e)
        }
    }
}

/// `POST /v1/fila` al motor: `(código, cuerpo)`.
fn fila_al_motor(cuerpo: &str) -> Result<(u16, Json), String> {
    let motor = std::env::var("ORE_TESELAS").unwrap_or_else(|_| "ore-teselas:8097".into());
    let (c, _, b) = al_motor_en(&motor, "/v1/fila", cuerpo)?;
    let j = ore_core::parse::parse(&String::from_utf8_lossy(&b))
        .map(|n| Json::de_node(&n))
        .unwrap_or(Json::Obj(Default::default()));
    Ok((c, j))
}

fn error(codigo: u16, m: impl Into<String>) -> Salida {
    let m = m.into();
    Salida::Bytes(Bytes {
        codigo,
        cabeceras: vec![
            ("content-type".into(), "application/json".into()),
            ("access-control-allow-origin".into(), "*".into()),
        ],
        largo: None,
        lector: Box::new(std::io::Cursor::new(
            Json::obj([("error", Json::s(m))]).jcs().into_bytes(),
        )),
        finales: None,
    })
}

/// Lo que se decide sin el motor: la forma, la firma, la clave y la pirámide.
fn comprobar(p: &Peticion, seg: &[&str]) -> Result<(Fuente, u32, u32, u32), Salida> {
    let ["v1", "tiles", capa, clave, tms, z, x, y] = seg else {
        return Err(error(
            404,
            "una tesela es /v1/tiles/{layer}/{clave}/WebMercatorQuad/{z}/{x}/{y}.mvt",
        ));
    };
    if *tms != TMS {
        return Err(error(404, format!("`{tms}`: sólo {TMS} (F2)")));
    }
    let (Ok(z), Ok(x), Some(Ok(y))) = (
        z.parse::<u32>(),
        x.parse::<u32>(),
        y.strip_suffix(".mvt").map(|y| y.parse::<u32>()),
    ) else {
        return Err(error(404, "z, x e y son enteros y la tesela acaba en .mvt"));
    };
    let q = consulta(&p.consulta_cruda);
    let (Some(e), Some(f)) = (q.get("e").and_then(|e| e.parse::<u64>().ok()), q.get("f")) else {
        return Err(error(
            403,
            "una tesela va firmada (`e` y `f`): pide el descriptor",
        ));
    };
    if e < ahora() {
        return Err(error(403, "la firma caducó: pide otro descriptor"));
    }
    if !iguales(&firma(clave, e), f) {
        return Err(error(403, "la firma no es de esta clave"));
    }
    let Some(fuente) = tabla().lock().unwrap().get(*clave).cloned() else {
        return Err(error(410, "esa clave ya no está: pide otro descriptor"));
    };
    if fuente.capa != *capa {
        return Err(error(404, "la clave es de otra capa"));
    }
    if z < fuente.minzoom || z > MAXZOOM || x >= 1 << z || y >= 1 << z {
        return Err(error(
            404,
            format!("fuera de la pirámide: z{} a z{MAXZOOM}", fuente.minzoom),
        ));
    }
    Ok((fuente, z, x, y))
}

/// `GET /v1/tiles/{layer}/{clave}/{tms}/{z}/{x}/{y}.mvt?e=…&f=…`
pub(crate) fn tesela(srv: &Servidor, p: &Peticion, seg: &[&str]) -> Salida {
    let (mut fuente, z, x, y) = match comprobar(p, seg) {
        Ok(v) => v,
        Err(s) => return s,
    };
    let (capa, clave) = (seg[2], seg[3]);
    // La credencial prestada caduca (STS, 1 h): se pide otra, para el mismo dataset.
    if ahora() >= fuente.credencial_hasta {
        match resolver(srv, capa, fuente.rama.as_deref()) {
            Ok((_, _, _, c, _)) => {
                fuente.credencial = c;
                fuente.credencial_hasta = ahora() + VIDA_CREDENCIAL;
                tabla()
                    .lock()
                    .unwrap()
                    .insert(clave.to_string(), fuente.clone());
            }
            Err(r) => return Salida::Una(r),
        }
    }
    let s = |v: &str| Json::s(v);
    let n = |v: u64| Json::s(v.to_string());
    // por debajo del suelo, la pirámide (con la credencial del dataset: vive bajo la tabla)
    let mut de_la_fuente = vec![
        ("metadata_location", s(&fuente.metadata_location)),
        ("geometria", s(&fuente.geometria)),
        ("crs", s(&fuente.crs)),
    ];
    if let Some(id) = &fuente.id {
        de_la_fuente.push(("id", s(id)));
    }
    if let Some(u) = fuente.piramide.as_deref().filter(|_| z < fuente.suelo) {
        de_la_fuente.push(("piramide", Json::obj([("ubicacion", s(u))])));
    }
    let cuerpo = Json::obj([
        ("protocolo", n(PROTOCOLO as u64)),
        ("tms", s(TMS)),
        ("z", n(z as u64)),
        ("x", n(x as u64)),
        ("y", n(y as u64)),
        ("formato", s("mvt")),
        ("capa", s(capa)),
        ("fuente", Json::obj(de_la_fuente)),
        (
            "limites",
            Json::obj([("plazo_ms", n(10_000)), ("max_bytes", n(2_000_000))]),
        ),
        ("credencial", fuente.credencial.clone()),
    ])
    .jcs();
    let motor = std::env::var("ORE_TESELAS").unwrap_or_else(|_| "ore-teselas:8097".into());
    let (codigo, cabeceras, bytes) = match al_motor_en(&motor, "/v1/teselas", &cuerpo) {
        Ok(v) => v,
        Err(m) => return error(502, m),
    };
    let de = |k: &str| {
        cabeceras
            .iter()
            .find(|(c, _)| c.eq_ignore_ascii_case(k))
            .map(|(_, v)| v.clone())
    };
    match codigo {
        200 | 204 => {
            let mut c = vec![
                (
                    "cache-control".to_string(),
                    "public, max-age=31536000, immutable".to_string(),
                ),
                ("access-control-allow-origin".into(), "*".into()),
            ];
            if codigo == 200 {
                c.push((
                    "content-type".into(),
                    "application/vnd.mapbox-vector-tile".into(),
                ));
            }
            for k in ["etag", "x-ore-cuenta"] {
                if let Some(v) = de(k) {
                    c.push((k.into(), v));
                }
            }
            Salida::Bytes(Bytes {
                codigo,
                cabeceras: c,
                largo: Some(bytes.len() as u64),
                lector: Box::new(std::io::Cursor::new(bytes)),
                finales: None,
            })
        }
        422 | 503 => error(codigo, String::from_utf8_lossy(&bytes).to_string()),
        otro => error(
            502,
            format!(
                "`ore-teselas` contestó {otro}: {}",
                String::from_utf8_lossy(&bytes)
            ),
        ),
    }
}

/// `POST /v1/teselas` al motor, por HTTP plano dentro de la celda (como `al_motor` de
/// `preview.rs`, con el cuerpo en bytes): `(código, cabeceras, cuerpo)`.
/// `(código, cabeceras, cuerpo)` de lo que contestó el motor.
type DelMotor = (u16, Vec<(String, String)>, Vec<u8>);

fn al_motor_en(motor: &str, ruta: &str, cuerpo: &str) -> Result<DelMotor, String> {
    use std::net::ToSocketAddrs as _;
    let dir = motor
        .to_socket_addrs()
        .ok()
        .and_then(|mut d| d.next())
        .ok_or_else(|| format!("`ore-teselas` (`{motor}`) no tiene dirección"))?;
    let mut s = std::net::TcpStream::connect_timeout(&dir, Duration::from_secs(5))
        .map_err(|e| format!("`ore-teselas` no contesta: {e}"))?;
    s.set_read_timeout(Some(Duration::from_secs(30))).ok();
    let req = format!(
        "POST {ruta} HTTP/1.1\r\nhost: teselas\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{cuerpo}",
        cuerpo.len()
    );
    s.write_all(req.as_bytes())
        .map_err(|e| format!("`ore-teselas`: {e}"))?;
    let mut todo = Vec::new();
    s.read_to_end(&mut todo)
        .map_err(|e| format!("`ore-teselas`: {e}"))?;
    let fin = todo
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("`ore-teselas` contestó algo que no es HTTP")?;
    let cabeza = String::from_utf8_lossy(&todo[..fin]).to_string();
    let mut lineas = cabeza.lines();
    let codigo = lineas
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse().ok())
        .unwrap_or(502);
    let cabeceras = lineas
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    let mut cuerpo = todo[fin + 4..].to_vec();
    // `ore-entrada` trocea lo que no sabe medir: el error JSON de un 4xx/5xx
    if cabeza
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        cuerpo = destrocear(&cuerpo);
    }
    Ok((codigo, cabeceras, cuerpo))
}

fn destrocear(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let Some(fin) = b[i..].windows(2).position(|w| w == b"\r\n") else {
            break;
        };
        let n =
            usize::from_str_radix(String::from_utf8_lossy(&b[i..i + fin]).trim(), 16).unwrap_or(0);
        i += fin + 2;
        if n == 0 || i + n > b.len() {
            break;
        }
        out.extend_from_slice(&b[i..i + n]);
        i += n + 2;
    }
    out
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn hmac_es_el_de_la_rfc_4231() {
        // RFC 4231, caso 2
        let h = hmac(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(
            hex(&h),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn la_firma_es_de_una_clave_y_un_momento() {
        let f = firma("abc", 100);
        assert_eq!(f.len(), 32);
        assert_eq!(f, firma("abc", 100));
        assert_ne!(f, firma("abd", 100));
        assert_ne!(f, firma("abc", 101));
        assert!(iguales(&f, &firma("abc", 100)));
        assert!(!iguales(&f, &f[..31]));
    }

    fn pet(ruta: &str, consulta: &str) -> Peticion {
        Peticion {
            metodo: "GET".into(),
            ruta: ruta.into(),
            cabeceras: [("host".to_string(), "victor.ore.paladio.io".to_string())].into(),
            cuerpo: String::new(),
            consulta: Default::default(),
            consulta_cruda: consulta.into(),
        }
    }

    fn datos(geo: &str, ml: &str) -> Result<Datos, Respuesta> {
        let j = format!(
            r#"{{"dataset":"geo.edificios","metadata_location":"{ml}","geo":{geo},"credencial":{{"gcs.oauth2.token":"t"}}}}"#
        );
        let n = ore_core::parse::parse(&j).unwrap();
        de_los_datos(Respuesta::ok(Json::de_node(&n)), "geo.edificios")
    }

    fn sin_piramide(_: &str, _: &Json) -> Option<Json> {
        None
    }

    fn con_piramide(u: &str, _: &Json) -> Option<Json> {
        assert!(u.ends_with("/_ore_piramides/1-r1.pmtiles"), "{u}");
        let n = ore_core::parse::parse(
            r#"{"minzoom":0,"maxzoom":12,"manifiesto":{"tipo":"puntos","suelo":13}}"#,
        )
        .unwrap();
        Some(Json::de_node(&n))
    }

    /// Sólo preguntar, sin disparo.
    fn solo<'a>(preguntar: &'a dyn Fn(&str, &Json) -> Option<Json>) -> Piramides<'a> {
        Piramides {
            preguntar,
            pedir: None,
        }
    }

    /// El descriptor y su URL: `(json, ruta con {z}/{x}/{y}, consulta)`.
    fn descriptor_y_url(
        d: Result<Datos, Respuesta>,
        preguntar: &dyn Fn(&str, &Json) -> Option<Json>,
    ) -> (String, String, String) {
        let r = descriptor_de(
            d,
            &pet("/v1/layers/geo.edificios/descriptor", ""),
            "geo.edificios",
            None,
            &solo(preguntar),
        );
        assert_eq!(r.codigo, 200, "{}", r.cuerpo.jcs());
        let j = r.cuerpo.jcs();
        let url = j
            .split("https://victor.ore.paladio.io")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap()
            .to_string();
        let (ruta, q) = url.split_once('?').unwrap();
        (j.clone(), ruta.to_string(), q.to_string())
    }

    #[test]
    fn la_piramide_vive_bajo_la_tabla_con_su_instantanea_y_su_receta() {
        assert_eq!(
            ubicacion_de_la_piramide(
                "gs://b/ore/v2/catalogo/query/default/t/metadata/00001-ab.metadata.json"
            )
            .as_deref(),
            Some("gs://b/ore/v2/catalogo/query/default/t/_ore_piramides/00001-ab-r1.pmtiles")
        );
        assert_eq!(ubicacion_de_la_piramide("gs://b/t/sin-metadata.json"), None);
    }

    /// Con pirámide: `minzoom` 0, su estado y su tipo en el descriptor; la clave cambia (la
    /// pareja); por debajo del suelo la tesela pasa y va a la pirámide; desde él, al crudo.
    #[test]
    fn con_piramide_el_descriptor_baja_a_cero_y_la_clave_lleva_la_pareja() {
        let ml = "gs://b/ore/v2/datasets/geo_edificios/metadata/1.metadata.json";
        let d = || datos(r#"{"geom":{"crs":"EPSG:4326"}}"#, ml);
        let (sin_j, sin_ruta, _) = descriptor_y_url(d(), &sin_piramide);
        let (con_j, con_ruta, q) = descriptor_y_url(d(), &con_piramide);
        assert!(
            sin_j.contains(r#""minzoom":13"#) && sin_j.contains(r#""estado":"ausente""#),
            "{sin_j}"
        );
        assert!(con_j.contains(r#""minzoom":0"#), "{con_j}");
        assert!(
            con_j.contains(r#""estado":"lista""#) && con_j.contains(r#""tipo":"puntos""#),
            "{con_j}"
        );
        assert_ne!(sin_ruta, con_ruta, "otra pirámide (o ninguna), otra clave");
        // z5, por debajo del suelo: pasa, y la fuente sabe su pirámide
        let baja = con_ruta.replace("{z}/{x}/{y}", "5/15/12");
        let seg: Vec<&str> = baja.trim_start_matches('/').split('/').collect();
        let Ok((f, z, ..)) = comprobar(&pet(&baja, &q), &seg) else {
            panic!("z5 con pirámide")
        };
        assert!(
            z < f.suelo
                && f.piramide
                    .as_deref()
                    .is_some_and(|u| u.contains("_ore_piramides"))
        );
    }

    /// El suelo, con pirámide, es el suyo: una de líneas acaba en z13, así que el crudo empieza en
    /// z14 aunque el descriptor se pida con el suelo de siempre; z13 va a la pirámide.
    #[test]
    fn con_piramide_el_suelo_es_el_de_la_piramide() {
        let ml = "gs://b/ore/v2/datasets/geo_edificios/metadata/1.metadata.json";
        let de_lineas = |_: &str, _: &Json| {
            let n = ore_core::parse::parse(
                r#"{"minzoom":0,"maxzoom":13,"manifiesto":{"tipo":"agregado","suelo":14}}"#,
            )
            .unwrap();
            Some(Json::de_node(&n))
        };
        let (j, ruta, q) = descriptor_y_url(datos(r#"{"geom":{}}"#, ml), &de_lineas);
        assert!(
            j.contains(r#""suelo":14"#) && j.contains(r#""minzoom":0"#),
            "{j}"
        );
        let z13 = ruta.replace("{z}/{x}/{y}", "13/2411/3080");
        let seg: Vec<&str> = z13.trim_start_matches('/').split('/').collect();
        let Ok((f, z, ..)) = comprobar(&pet(&z13, &q), &seg) else {
            panic!("z13 con pirámide de líneas")
        };
        assert!(z < f.suelo && f.piramide.is_some(), "z13 va a la pirámide");
    }

    /// Si el Job de la pirámide falló, el descriptor lo dice (`fallida`, con el motivo), se sirve
    /// desde el suelo y no se vuelve a pedir.
    #[test]
    fn si_el_job_fallo_el_descriptor_lo_dice_y_no_la_vuelve_a_pedir() {
        let ml = "gs://b/ore/v2/datasets/geo_edificios/metadata/1.metadata.json";
        let fallida = |_: &str, _: &Json| {
            let n = ore_core::parse::parse(
                r#"{"error":"no hay pirámide ahí","fallo":{"error":"tippecanoe: sin memoria","cuando":1}}"#,
            )
            .unwrap();
            Some(Json::de_node(&n))
        };
        let pedidas = std::cell::Cell::new(0);
        let pedir = |_: &str, _: &str, _: &str| {
            pedidas.set(pedidas.get() + 1);
            Ok::<_, String>("encolada".into())
        };
        let r = descriptor_de(
            datos(r#"{"geom":{}}"#, ml),
            &pet("/", ""),
            "geo.edificios",
            None,
            &Piramides {
                preguntar: &fallida,
                pedir: Some(&pedir),
            },
        );
        let j = r.cuerpo.jcs();
        assert!(
            j.contains(r#""estado":"fallida""#)
                && j.contains("sin memoria")
                && j.contains(r#""minzoom":13"#),
            "{j}"
        );
        assert_eq!(pedidas.get(), 0);
    }

    /// A quien mira con un filtro de fila no se le sirve la pirámide: agrega todo (C4-f).
    #[test]
    fn con_filtro_de_quien_mira_no_hay_piramide() {
        let j = format!(
            r#"{{"dataset":"geo.e","metadata_location":"gs://b/t/metadata/1.metadata.json","geo":{{"geom":{{}}}},"credencial":{{}},"{FILTRO_DE_QUIEN_MIRA}":"region = 'norte'"}}"#
        );
        let n = ore_core::parse::parse(&j).unwrap();
        let d = de_los_datos(Respuesta::ok(Json::de_node(&n)), "geo.e");
        let (j, ..) = descriptor_y_url(d, &con_piramide);
        assert!(
            j.contains(r#""estado":"no""#) && j.contains(r#""minzoom":13"#),
            "{j}"
        );
    }

    /// Con el disparo (D-F2-18): sin pirámide, el descriptor la pide —con la instantánea, la
    /// capa, el suelo y el destino, sin credenciales— y dice `construyendo`; si pedirla falla,
    /// `ausente` con el motivo; con pirámide o con filtro de fila, no pide nada.
    #[test]
    fn sin_piramide_el_disparo_la_pide_una_vez_y_dice_construyendo() {
        let ml = "gs://b/ore/v2/datasets/geo_edificios/metadata/1.metadata.json";
        let d = || datos(r#"{"geom":{"crs":"EPSG:4326"}}"#, ml);
        let pedidas = std::cell::RefCell::new(Vec::new());
        let pedir = |u: &str, tabla: &str, pet: &str| {
            pedidas
                .borrow_mut()
                .push((u.to_string(), tabla.to_string(), pet.to_string()));
            Ok::<_, String>("encolada".into())
        };
        let r = descriptor_de(
            d(),
            &pet("/v1/layers/geo.edificios/descriptor", "minzoom=14"),
            "geo.edificios",
            None,
            &Piramides {
                preguntar: &sin_piramide,
                pedir: Some(&pedir),
            },
        );
        let j = r.cuerpo.jcs();
        assert!(
            j.contains(r#""estado":"construyendo""#) && j.contains(r#""minzoom":14"#),
            "{j}"
        );
        {
            let p = pedidas.borrow();
            assert_eq!(p.len(), 1);
            let (u, tabla, pet) = &p[0];
            assert_eq!(u, &ubicacion_de_la_piramide(ml).unwrap());
            assert_eq!(tabla, "gs://b/ore/v2/datasets/geo_edificios");
            let n = ore_core::parse::parse(pet).unwrap();
            let s = |o: &str, k: &str| {
                n.get(o)
                    .and_then(|(_, v)| v.get(k))
                    .and_then(|(_, v)| v.as_str().map(String::from))
            };
            assert_eq!(s("fuente", "metadata_location").as_deref(), Some(ml));
            assert_eq!(s("fuente", "geometria").as_deref(), Some("geom"));
            assert_eq!(s("destino", "ubicacion"), Some(u.clone()));
            assert!(
                !pet.contains("suelo") && !pet.contains("credencial"),
                "{pet}"
            );
            assert!(!pet.contains('\n') && !pet.contains('\''));
        }
        // pedirla falla: ausente, con el motivo
        let falla = |_: &str, _: &str, _: &str| Err::<String, _>("la cola no trae".to_string());
        let r = descriptor_de(
            d(),
            &pet("/", ""),
            "geo.edificios",
            None,
            &Piramides {
                preguntar: &sin_piramide,
                pedir: Some(&falla),
            },
        );
        let j = r.cuerpo.jcs();
        assert!(
            j.contains(r#""estado":"ausente""#) && j.contains("la cola no trae"),
            "{j}"
        );
        // con pirámide no se pide
        descriptor_de(
            d(),
            &pet("/", ""),
            "geo.edificios",
            None,
            &Piramides {
                preguntar: &con_piramide,
                pedir: Some(&pedir),
            },
        );
        assert_eq!(pedidas.borrow().len(), 1);
    }

    fn codigo(s: Result<(Fuente, u32, u32, u32), Salida>) -> u16 {
        match s {
            Ok(_) => 200,
            Err(Salida::Bytes(b)) => b.codigo,
            Err(Salida::Una(r)) => r.codigo,
            Err(_) => 0,
        }
    }

    /// El descriptor da una URL firmada; con ella la tesela pasa, y con cualquier
    /// cosa cambiada no: firma (403), caducidad (403), clave (410), pirámide (404).
    #[test]
    fn el_descriptor_firma_y_la_tesela_lo_comprueba() {
        let d = datos(
            r#"{"geom":{"crs":"EPSG:4326"}}"#,
            "gs://b/ore/v2/datasets/geo_edificios/metadata/1.metadata.json",
        );
        let r = descriptor_de(
            d,
            &pet("/v1/layers/geo.edificios/descriptor", "minzoom=12"),
            "geo.edificios",
            Some("ana/geo"),
            &solo(&sin_piramide),
        );
        assert_eq!(r.codigo, 200, "{}", r.cuerpo.jcs());
        let j = r.cuerpo.jcs();
        assert!(
            j.contains(r#""tilejson":"3.0.0""#) && j.contains(r#""minzoom":12"#),
            "{j}"
        );
        let url = j
            .split("https://victor.ore.paladio.io")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        let (ruta, q) = url.split_once('?').unwrap();
        let ruta = ruta.replace("{z}/{x}/{y}", "14/8023/6177");
        let seg: Vec<&str> = ruta.trim_start_matches('/').split('/').collect();
        assert_eq!(codigo(comprobar(&pet(&ruta, q), &seg)), 200);
        // la fuente recuerda la rama: la credencial se renueva en ella
        let Ok((fuente, ..)) = comprobar(&pet(&ruta, q), &seg) else {
            unreachable!()
        };
        assert_eq!(fuente.rama.as_deref(), Some("ana/geo"));
        // la firma de otra cosa
        // (otro último carácter: si ya era `0`, `0` la dejaba igual una vez de cada 16)
        let otro = if q.ends_with('0') { '1' } else { '0' };
        let q_mala = format!("{}{otro}", &q[..q.len() - 1]);
        assert_eq!(codigo(comprobar(&pet(&ruta, &q_mala), &seg)), 403);
        // sin firma
        assert_eq!(codigo(comprobar(&pet(&ruta, ""), &seg)), 403);
        // caducada, aunque bien firmada
        let clave = seg[3];
        let vieja = format!("e=100&f={}", firma(clave, 100));
        assert_eq!(codigo(comprobar(&pet(&ruta, &vieja), &seg)), 403);
        // una clave que no está, bien firmada: 410 (pide otro descriptor)
        let e = ahora() + 60;
        let otra = ruta.replace(clave, "0000000000000000000000000000beef");
        let seg_otra: Vec<&str> = otra.trim_start_matches('/').split('/').collect();
        let q_otra = format!("e={e}&f={}", firma("0000000000000000000000000000beef", e));
        assert_eq!(codigo(comprobar(&pet(&otra, &q_otra), &seg_otra)), 410);
        // por debajo del suelo: 404
        let baja = ruta.replace("14/8023/6177", "11/1002/772");
        let seg_baja: Vec<&str> = baja.trim_start_matches('/').split('/').collect();
        assert_eq!(codigo(comprobar(&pet(&baja, q), &seg_baja)), 404);
    }

    #[test]
    fn sin_geo_o_sin_copia_iceberg_no_hay_descriptor() {
        let sin_geo = descriptor_de(
            datos("{}", "gs://b/x/metadata/1.json"),
            &pet("/", ""),
            "geo.x",
            None,
            &solo(&sin_piramide),
        );
        assert_eq!(sin_geo.codigo, 422);
        let sin_copia = descriptor_de(
            datos(r#"{"geom":{}}"#, ""),
            &pet("/", ""),
            "geo.x",
            None,
            &solo(&sin_piramide),
        );
        assert_eq!(sin_copia.codigo, 409);
        let no_esta = descriptor_de(
            Err(Respuesta::error(404, "no hay")),
            &pet("/", ""),
            "geo.x",
            None,
            &solo(&sin_piramide),
        );
        assert_eq!(no_esta.codigo, 404);
    }

    fn json(t: &str) -> Json {
        Json::de_node(&ore_core::parse::parse(t).unwrap())
    }

    /// Una fuente pequeña: el índice no hace falta.
    fn sin_indice(_: &str, _: &str, _: &Json) -> Option<Json> {
        Some(json(r#"{"fuente":{"filas":"10","secuencia":"1"}}"#))
    }

    /// Las pruebas de la pirámide, con una fuente que no necesita índice (I3·2).
    fn descriptor_de(
        d: Result<Datos, Respuesta>,
        p: &Peticion,
        capa: &str,
        rama: Option<&str>,
        pir: &Piramides,
    ) -> Respuesta {
        let idx = Indices {
            preguntar: &sin_indice,
            pedir: None,
            desde_filas: 5_000_000,
        };
        descriptor_con(d, p, capa, rama, pir, &idx)
    }

    const ML: &str = "gs://b/ore/v2/datasets/geo_edificios/metadata/2-b.metadata.json";
    const TABLA: &str = "gs://b/ore/v2/datasets/geo_edificios";
    const INDICE: &str =
        "gs://b/ore/v2/datasets/geo_edificios/_ore_capa/i1/1-a/metadata/00000-x.metadata.json";
    const FICHEROS: &str = "ore/v2/datasets/geo_edificios/_ore_capa/i1/1-a/ficheros.json";

    /// El motor: la instantánea de ahora (`ML`) tiene 30 M filas y secuencia 2; el índice vigente
    /// es de `s`, con secuencia `sec`.
    fn con_indice(s: &'static str, sec: u64) -> impl Fn(&str, &str, &Json) -> Option<Json> {
        move |tabla, ml, _| {
            assert_eq!((tabla, ml), (TABLA, ML));
            Some(json(&format!(
                r#"{{"instantanea_ms":"1793000000000","indice":{{"generacion":"3","snapshot_fuente":"{s}","secuencia_fuente":"{sec}","identidad":{{"tipo":"posicion"}},"indice":"{INDICE}","ficheros":"{FICHEROS}","filas":"30000000","profundidad":"1.5"}},"fuente":{{"filas":"30000000","secuencia":"2"}}}}"#
            )))
        }
    }

    /// La fuente que la tesela `z/x/y` de un descriptor manda al motor.
    fn fuente_de(r: &Respuesta, zxy: &str) -> (Fuente, u32) {
        assert_eq!(r.codigo, 200, "{}", r.cuerpo.jcs());
        let j = r.cuerpo.jcs();
        let url = j
            .split("https://victor.ore.paladio.io")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap()
            .to_string();
        let (ruta, q) = url.split_once('?').unwrap();
        let ruta = ruta.replace("{z}/{x}/{y}", zxy);
        let seg: Vec<&str> = ruta.trim_start_matches('/').split('/').collect();
        let Ok((f, z, ..)) = comprobar(&pet(&ruta, q), &seg) else {
            panic!("{zxy}: {j}")
        };
        (f, z)
    }

    fn pide<'a>(
        v: &'a std::cell::RefCell<Vec<(String, String, String)>>,
    ) -> impl Fn(&str, &str, &str) -> Result<String, String> + 'a {
        move |u, t, p| {
            v.borrow_mut()
                .push((u.to_string(), t.to_string(), p.to_string()));
            Ok("encolado".into())
        }
    }

    /// I3·2 · Con un índice de esta instantánea, el crudo es el índice —su geometría en 3857 y su
    /// id— y la pirámide, la de esta instantánea; la clave no es la de la tabla.
    #[test]
    fn con_indice_al_dia_la_tesela_lee_el_indice_con_su_id() {
        let d = || datos(r#"{"geom":{"crs":"EPSG:4326"}}"#, ML);
        let preguntadas = std::cell::RefCell::new(vec![]);
        let pir_preguntar = |u: &str, _: &Json| {
            preguntadas.borrow_mut().push(u.to_string());
            None
        };
        let al_dia = con_indice(ML, 2);
        let r = descriptor_con(
            d(),
            &pet("/v1/layers/geo.edificios/descriptor", ""),
            "geo.edificios",
            None,
            &solo(&pir_preguntar),
            &Indices {
                preguntar: &al_dia,
                pedir: None,
                desde_filas: 5_000_000,
            },
        );
        let j = r.cuerpo.jcs();
        assert!(
            j.contains(r#""indice":{"al_dia":true"#) && j.contains(r#""estado":"lista""#),
            "{j}"
        );
        assert!(j.contains(r#""instantanea_ms":"1793000000000""#), "{j}");
        assert_eq!(
            preguntadas.borrow().as_slice(),
            [ubicacion_de_la_piramide(ML).unwrap()]
        );
        let (f, _) = fuente_de(&r, "14/8023/6177");
        assert_eq!(
            (
                f.metadata_location.as_str(),
                f.geometria.as_str(),
                f.crs.as_str(),
                f.id.as_deref()
            ),
            (INDICE, "geom", "EPSG:3857", Some("_ore_id"))
        );
        // sin índice, la tabla tal cual y otra clave
        let r_sin = descriptor_de(
            d(),
            &pet("/", ""),
            "geo.edificios",
            None,
            &solo(&sin_piramide),
        );
        let (f_sin, _) = fuente_de(&r_sin, "14/8023/6177");
        assert_eq!((f_sin.metadata_location.as_str(), f_sin.id), (ML, None));
        assert!(r_sin.cuerpo.jcs().contains(r#""estado":"no_hace_falta""#));
    }

    /// I3·2 · Con un índice que va por detrás se pinta su momento entero: el crudo de su índice y
    /// la pirámide de su instantánea (si está; no se pide), nunca la de ahora. Se dice, y con el
    /// disparo se pide otro índice, el de ahora.
    #[test]
    fn con_indice_por_detras_se_pinta_su_momento_y_se_pide_otro() {
        let viejo = "gs://b/ore/v2/datasets/geo_edificios/metadata/1-a.metadata.json";
        let preguntadas = std::cell::RefCell::new(vec![]);
        let pir_preguntar = |u: &str, _: &Json| {
            preguntadas.borrow_mut().push(u.to_string());
            None
        };
        let (piramides, indices) = (
            std::cell::RefCell::new(vec![]),
            std::cell::RefCell::new(vec![]),
        );
        let (pedir_p, pedir_i) = (pide(&piramides), pide(&indices));
        let por_detras = con_indice(viejo, 1);
        let r = descriptor_con(
            datos(r#"{"geom":{"crs":"EPSG:4326"}}"#, ML),
            &pet("/", ""),
            "geo.edificios",
            None,
            &Piramides {
                preguntar: &pir_preguntar,
                pedir: Some(&pedir_p),
            },
            &Indices {
                preguntar: &por_detras,
                pedir: Some(&pedir_i),
                desde_filas: 5_000_000,
            },
        );
        let j = r.cuerpo.jcs();
        assert!(
            j.contains(r#""al_dia":false"#) && j.contains(r#""poniendose_al_dia":true"#),
            "{j}"
        );
        assert!(j.contains(&format!(r#""instantanea":"{viejo}""#)), "{j}");
        // la pirámide, la de su momento; y no se pide
        assert_eq!(
            preguntadas.borrow().as_slice(),
            [ubicacion_de_la_piramide(viejo).unwrap()]
        );
        assert!(piramides.borrow().is_empty());
        // el crudo, del índice
        assert_eq!(fuente_de(&r, "14/8023/6177").0.metadata_location, INDICE);
        // y se pide el índice de ahora: la instantánea, su geometría, el suelo y el destino
        let p = indices.borrow();
        assert_eq!(p.len(), 1);
        let (u, tabla, pet) = &p[0];
        assert_eq!(u, &format!("{TABLA}/_ore_capa/i1/2-b"));
        assert_eq!(tabla, TABLA);
        let n = ore_core::parse::parse(pet).unwrap();
        let s = |o: &str, k: &str| {
            n.get(o)
                .and_then(|(_, v)| v.get(k))
                .and_then(|(_, v)| v.as_str().map(String::from))
        };
        assert_eq!(n.get("tarea").and_then(|(_, v)| v.as_str()), Some("indice"));
        assert_eq!(s("fuente", "metadata_location").as_deref(), Some(ML));
        assert_eq!(s("fuente", "geometria").as_deref(), Some("geom"));
        assert_eq!(s("destino", "ubicacion").as_deref(), Some(TABLA));
        assert!(pet.contains(r#""suelo":13"#), "{pet}");
        assert!(!pet.contains("credencial") && !pet.contains('\n') && !pet.contains('\''));
    }

    /// I3·2 · Un índice de una instantánea posterior a la que se pinta (una rama que lee algo más
    /// viejo) no se sirve: pintaría el futuro.
    #[test]
    fn un_indice_posterior_no_se_sirve() {
        let nuevo = "gs://b/ore/v2/datasets/geo_edificios/metadata/3-c.metadata.json";
        let posterior = con_indice(nuevo, 3);
        let r = descriptor_con(
            datos(r#"{"geom":{}}"#, ML),
            &pet("/", ""),
            "geo.edificios",
            None,
            &solo(&sin_piramide),
            &Indices {
                preguntar: &posterior,
                pedir: None,
                desde_filas: 5_000_000,
            },
        );
        assert!(r.cuerpo.jcs().contains("posterior"), "{}", r.cuerpo.jcs());
        let (f, _) = fuente_de(&r, "14/8023/6177");
        assert_eq!((f.metadata_location.as_str(), f.id), (ML, None));
    }

    /// I4·5 · **El último momento completo**: el índice ya siguió a la instantánea de ahora pero su
    /// pirámide aún se construye; se pinta el momento de antes entero —su índice y su pirámide, desde
    /// z0—, se dice cuál espera y se pide la pirámide de ahora. Cuando está, se pinta el de ahora.
    #[test]
    fn sin_la_piramide_del_ultimo_se_pinta_el_momento_completo_de_antes() {
        let viejo = "gs://b/ore/v2/datasets/geo_edificios/metadata/1-a.metadata.json";
        let indice_viejo = INDICE.replace("00000-x", "00000-w");
        let motor = |_: &str, _: &str, _: &Json| {
            Some(json(&format!(
                r#"{{"instantanea_ms":"1793000600000","indice":{{"generacion":"4","snapshot_fuente":"{ML}","secuencia_fuente":"2","identidad":{{"tipo":"posicion"}},"indice":"{INDICE}","ficheros":"{FICHEROS}"}},"anteriores":[{{"instantanea_ms":"1793000000000","indice":{{"generacion":"3","snapshot_fuente":"{viejo}","secuencia_fuente":"1","identidad":{{"tipo":"posicion"}},"indice":"{indice_viejo}","ficheros":"{FICHEROS}"}}}}],"fuente":{{"filas":"30000000","secuencia":"2"}}}}"#
            )))
        };
        let lista = std::cell::Cell::new(false);
        let pir_preguntar = |u: &str, _: &Json| {
            (u == ubicacion_de_la_piramide(viejo).unwrap()
                || (lista.get() && u == ubicacion_de_la_piramide(ML).unwrap()))
            .then(|| json(r#"{"minzoom":0,"maxzoom":12,"manifiesto":{"tipo":"puntos"}}"#))
        };
        let piramides = std::cell::RefCell::new(vec![]);
        let pedir_p = pide(&piramides);
        let abrir = || {
            descriptor_con(
                datos(r#"{"geom":{"crs":"EPSG:4326"}}"#, ML),
                &pet("/", ""),
                "geo.edificios",
                None,
                &Piramides {
                    preguntar: &pir_preguntar,
                    pedir: Some(&pedir_p),
                },
                &Indices {
                    preguntar: &motor,
                    pedir: None,
                    desde_filas: 5_000_000,
                },
            )
        };
        let r = abrir();
        let j = r.cuerpo.jcs();
        assert!(j.contains(r#""minzoom":0"#), "{j}");
        assert!(
            j.contains(&format!(r#""instantanea":"{viejo}""#))
                && j.contains(r#""al_dia":false"#)
                && j.contains(r#""instantanea_ms":"1793000000000""#),
            "{j}"
        );
        assert!(
            j.contains(&format!(
                r#""esperando":{{"instantanea":"{ML}","piramide":{{"estado":"construyendo"}}}}"#
            )),
            "{j}"
        );
        // el crudo y la pirámide, los del momento de antes; la pirámide pedida, la de ahora
        let (f, _) = fuente_de(&r, "14/8023/6177");
        assert_eq!(f.metadata_location, indice_viejo);
        assert_eq!(f.clic.unwrap().instantanea, viejo);
        assert_eq!(f.piramide, ubicacion_de_la_piramide(viejo));
        assert_eq!(
            piramides
                .borrow()
                .iter()
                .map(|(u, ..)| u.clone())
                .collect::<Vec<_>>(),
            [ubicacion_de_la_piramide(ML).unwrap()]
        );
        // la pirámide de ahora acaba: se pinta el momento de ahora entero
        lista.set(true);
        let j = abrir().cuerpo.jcs();
        assert!(
            j.contains(r#""al_dia":true"#) && !j.contains("esperando"),
            "{j}"
        );
        let (f, _) = fuente_de(&abrir(), "14/8023/6177");
        assert_eq!(
            (f.metadata_location.as_str(), f.piramide),
            (INDICE, ubicacion_de_la_piramide(ML))
        );
    }

    /// I3·2 · Sin índice: una fuente grande lo pide (con el disparo) y dice `construyendo`, y sin
    /// el disparo `ausente`; una pequeña no lo necesita y no pide nada. El crudo, la tabla.
    #[test]
    fn sin_indice_la_grande_lo_pide_y_la_pequena_no() {
        let grande = |_: &str, _: &str, _: &Json| {
            Some(json(r#"{"fuente":{"filas":"30000000","secuencia":"2"}}"#))
        };
        let pedidos = std::cell::RefCell::new(vec![]);
        let pedir = pide(&pedidos);
        let con = |preguntar: &dyn Fn(&str, &str, &Json) -> Option<Json>, disparo: bool| {
            descriptor_con(
                datos(r#"{"geom":{}}"#, ML),
                &pet("/", ""),
                "geo.edificios",
                None,
                &solo(&sin_piramide),
                &Indices {
                    preguntar,
                    pedir: disparo.then_some(&pedir as &Pedir<'_>),
                    desde_filas: 5_000_000,
                },
            )
        };
        let r = con(&grande, true);
        assert!(
            r.cuerpo
                .jcs()
                .contains(r#""indice":{"estado":"construyendo"}"#),
            "{}",
            r.cuerpo.jcs()
        );
        assert_eq!(pedidos.borrow().len(), 1);
        assert_eq!(fuente_de(&r, "14/8023/6177").0.metadata_location, ML);
        let r = con(&grande, false);
        assert!(
            r.cuerpo.jcs().contains(r#""indice":{"estado":"ausente"}"#),
            "{}",
            r.cuerpo.jcs()
        );
        let r = con(&sin_indice, true);
        assert!(
            r.cuerpo.jcs().contains(r#""estado":"no_hace_falta""#),
            "{}",
            r.cuerpo.jcs()
        );
        assert_eq!(pedidos.borrow().len(), 1, "la pequeña no pide");
    }

    /// I3·2 · Con un filtro de fila de quien mira, ni índice ni pregunta: el motor aún no aplica
    /// el filtro (A8), y el índice no abre nada que hoy no esté abierto.
    #[test]
    fn con_filtro_de_quien_mira_no_hay_indice() {
        let j = format!(
            r#"{{"dataset":"geo.e","metadata_location":"{ML}","geo":{{"geom":{{}}}},"credencial":{{}},"{FILTRO_DE_QUIEN_MIRA}":"region = 'norte'"}}"#
        );
        let d = de_los_datos(Respuesta::ok(json(&j)), "geo.e");
        let preguntado = std::cell::Cell::new(false);
        let preguntar = |_: &str, _: &str, _: &Json| {
            preguntado.set(true);
            None
        };
        let r = descriptor_con(
            d,
            &pet("/", ""),
            "geo.e",
            None,
            &solo(&sin_piramide),
            &Indices {
                preguntar: &preguntar,
                pedir: None,
                desde_filas: 5_000_000,
            },
        );
        assert!(r.cuerpo.jcs().contains("A8"), "{}", r.cuerpo.jcs());
        assert!(!preguntado.get());
        assert_eq!(fuente_de(&r, "14/8023/6177").0.metadata_location, ML);
    }

    /// La clave del descriptor (`ore.clave`).
    fn clave_de(r: &Respuesta) -> String {
        let j = r.cuerpo.jcs();
        j.split(r#""clave":""#)
            .nth(1)
            .and_then(|c| c.split('"').next())
            .unwrap_or_else(|| panic!("{j}"))
            .to_string()
    }

    /// I3·3 · El clic: el id es la posición (ordinal y fila); la fila sale de la instantánea que
    /// pinta la capa —aunque vaya por detrás—, con la credencial de quien pide ahora.
    #[test]
    fn el_clic_pide_la_fila_de_la_instantanea_que_se_pinta() {
        let viejo = "gs://b/ore/v2/datasets/geo_edificios/metadata/1-a.metadata.json";
        let por_detras = con_indice(viejo, 1);
        let d = || datos(r#"{"geom":{"crs":"EPSG:4326"}}"#, ML);
        let r = descriptor_con(
            d(),
            &pet("/", ""),
            "geo.edificios",
            None,
            &solo(&sin_piramide),
            &Indices {
                preguntar: &por_detras,
                pedir: None,
                desde_filas: 5_000_000,
            },
        );
        let clave = clave_de(&r);
        let pedido = std::cell::RefCell::new(String::new());
        let motor = |c: &str| {
            *pedido.borrow_mut() = c.to_string();
            Ok((
                200,
                json(
                    r#"{"fichero":"gs://b/x.parquet","fila":"7","propiedades":{"t_fare":"12.5"}}"#,
                ),
            ))
        };
        let resolver = |_: Option<&str>| d();
        let id = (2u64 << 40) | 7;
        let q = format!("clave={clave}");
        let r = elemento_con(
            &pet("/", &q),
            "geo.edificios",
            &id.to_string(),
            &resolver,
            &motor,
        );
        let j = r.cuerpo.jcs();
        assert_eq!(r.codigo, 200, "{j}");
        assert!(
            j.contains(r#""t_fare":"12.5""#) && j.contains(&format!(r#""instantanea":"{viejo}""#)),
            "{j}"
        );
        let n = ore_core::parse::parse(&pedido.borrow()).unwrap();
        let s = |k: &str| n.get(k).and_then(|(_, v)| v.as_str()).map(String::from);
        assert_eq!(s("metadata_location").as_deref(), Some(viejo));
        assert_eq!(s("ficheros").as_deref(), Some(FICHEROS));
        assert_eq!(
            (s("ordinal").as_deref(), s("fila").as_deref()),
            (Some("2"), Some("7"))
        );
        // lo que no: el motor no la encuentra (404), un id que no es número (422), una clave que
        // ya no está (410), otra capa (404), sin clave (422)
        let no_esta = |_: &str| Ok((422, json("{}")));
        let e = |q: &str, capa: &str, id: &str, m: &AlMotor<'_>| {
            elemento_con(&pet("/", q), capa, id, &resolver, m).codigo
        };
        assert_eq!(e(&q, "geo.edificios", "7", &no_esta), 404);
        assert_eq!(e(&q, "geo.edificios", "siete", &motor), 422);
        assert_eq!(e("clave=0000beef", "geo.edificios", "7", &motor), 410);
        assert_eq!(e(&q, "geo.otra", "7", &motor), 404);
        assert_eq!(e("", "geo.edificios", "7", &motor), 422);
        // quien pide ya no puede verla: lo que diga `datos`
        let sin_permiso = |_: Option<&str>| Err(Respuesta::error(403, "no"));
        assert_eq!(
            elemento_con(&pet("/", &q), "geo.edificios", "7", &sin_permiso, &motor).codigo,
            403
        );
    }

    /// I3·3 · Sin índice los elementos no llevan id: el clic no tiene qué abrir (409). Con un
    /// filtro de fila, tampoco (403, A8).
    #[test]
    fn sin_indice_o_con_filtro_no_hay_clic() {
        let d = || datos(r#"{"geom":{}}"#, ML);
        let r = descriptor_de(
            d(),
            &pet("/", ""),
            "geo.edificios",
            None,
            &solo(&sin_piramide),
        );
        let q = format!("clave={}", clave_de(&r));
        let motor = |_: &str| Ok((200, json("{}")));
        let resolver = |_: Option<&str>| d();
        assert_eq!(
            elemento_con(&pet("/", &q), "geo.edificios", "7", &resolver, &motor).codigo,
            409
        );
        let al_dia = con_indice(ML, 2);
        let r = descriptor_con(
            d(),
            &pet("/", ""),
            "geo.edificios",
            None,
            &solo(&sin_piramide),
            &Indices {
                preguntar: &al_dia,
                pedir: None,
                desde_filas: 5_000_000,
            },
        );
        let q = format!("clave={}", clave_de(&r));
        let con_filtro = |_: Option<&str>| {
            let j = format!(
                r#"{{"metadata_location":"{ML}","geo":{{"geom":{{}}}},"credencial":{{}},"{FILTRO_DE_QUIEN_MIRA}":"region = 'norte'"}}"#
            );
            de_los_datos(Respuesta::ok(json(&j)), "geo.edificios")
        };
        assert_eq!(
            elemento_con(&pet("/", &q), "geo.edificios", "7", &con_filtro, &motor).codigo,
            403
        );
    }

    #[test]
    fn destrocear_junta_los_trozos() {
        assert_eq!(
            destrocear(b"4\r\nhola\r\n3\r\n ok\r\n0\r\n\r\n"),
            b"hola ok"
        );
    }
}
