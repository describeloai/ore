//! **La colección escrita: `put` con transacciones** (ADR 0049 B4b·1).
//!
//! Una colección sin `from` la llena el código (v1alpha16 `02` §3, v1alpha19
//! `01`). Este proceso es **el único que escribe en el lago**: el puesto le
//! manda los bytes y él los cuenta, los guarda por su contenido y, al
//! confirmar, sella el manifiesto —la misma tabla Iceberg que una mantenida,
//! con el mismo `ore_store::ciclo::sellar`—.
//!
//! | puerto | ruta | quién | qué |
//! |---|---|---|---|
//! | 8097 | `POST /escritura/abrir {coleccion, ttl_s?}` | `ore-serve` | una transacción y su **permiso de subida** |
//! | 8098 | `PUT /subida?permiso=&path=` (el cuerpo en flujo; `Content-Type` y `Repr-Digest` opcionales) | el puesto | un ítem: `201` con su referencia |
//! | 8097 | `POST /escritura/confirmar {transaccion, coleccion, metadata_location?, base?, dataset?}` | `ore-serve` | sella sobre la base que se le nombra; `ore-serve` escribe el puntero |
//! | 8097 | `POST /escritura/abortar {transaccion}` | `ore-serve` | no deja nada |
//!
//! ⭐ **El puesto no recibe una credencial del lago.** Con una prestada sobre
//!   `ore/v2/blobs/` el lago no comprobaría que el nombre de un blob es su
//!   contenido: un puesto podría plantar bytes falsos bajo el `sha256` de otro.
//!   Aquí el `sha256` y el `crc32c` se calculan **al paso**, mientras llegan, y
//!   el nombre sale de ellos; el almacén los coteja otra vez al subir
//!   (`Almacen::poner_blob`).
//!
//! ⭐ **El tipo, por los bytes** (`ore_core::medios::tipo_por_bytes`): el que el
//!   código declara se guarda como declarado, pero el que se sirve es el que
//!   dicen los bytes. Un `text/html` declarado sobre un PDF se sirve como PDF;
//!   uno que no dice nada, como lo declarado o `application/octet-stream`, y
//!   lo activo (HTML, SVG) nunca en línea (`EN_LINEA`).
//!
//! ⭐ **Subir lo que ya está es gratis**: el blob ya existe, no se sube otra vez
//!   y se **toca**, para que la recogida (gracia de 2 h) no se lo lleve antes
//!   de que una fila lo nombre. Por eso una transacción vive como mucho una
//!   hora: menos que la gracia.
//!
//! ⭐ **El ítem existe cuando la transacción se confirma.** Hasta entonces lo
//!   subido está en el lago sin nombre en ningún manifiesto; un `abort` —o una
//!   transacción que caduca— no deja ninguna fila, y la recogida se lleva sus
//!   blobs pasada la gracia.
//!
//! **El manifiesto de una escrita**: las columnas de una mantenida, con
//! `clave` = el camino y `version` = el `sha256` del contenido (dos subidas
//! del mismo contenido al mismo camino son la misma fila). Escribir un camino
//! que ya estaba con otro contenido retira su fila actual en esta transacción
//! (`retirado`, `retirado_ms`) y añade la nueva; con el mismo, no cambia nada.
//! La base la da `ore-serve` al confirmar —el puntero de ese momento—, no al
//! abrir: dos transacciones abiertas a la vez no se pisan, y quién gana la
//! escritura del puntero lo decide `ore-serve`.
//!
//! ⚠️ **En memoria**, como los permisos de leer: un reinicio olvida las
//!   transacciones abiertas (sus blobs se recogen) y el código abre otra.

use crate::servicio::{Pedido, Servicio, problema};
use ore_core::json::Json;
use ore_core::parse::Node;
use ore_entrada::http::{Peticion, Respuesta};
use ore_store::almacen::{Blob, Cuerpo};
use ore_store::blobs::{Crc32c, EN_MEMORIA, clave_de};
use ore_store::sobre::{Cabecera, Testigo};
use sha2::Digest;
use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// La vida de una transacción, en segundos: por defecto y sus límites. Menos
/// que la gracia de la recogida de blobs (2 h).
pub const VIDA_POR_DEFECTO: u64 = 3600;
pub const VIDA_MINIMA: u64 = 60;
pub const VIDA_MAXIMA: u64 = 3600;

/// Cuántas transacciones abiertas a la vez, como mucho.
pub const MAXIMAS: usize = 1000;

/// El camino más largo que se admite, en bytes.
pub const CAMINO_MAXIMO: usize = 1024;

/// Lo que se mira para decir el tipo.
const CABEZA: usize = 512;

/// Lo que se sube y no se sabe qué es.
const TIPO_POR_DEFECTO: &str = "application/octet-stream";

/// Las columnas del manifiesto, con su tipo de OOS: las de una mantenida
/// (`ore-cli/src/coleccion.rs`), blob y tipo incluidos.
const COLUMNAS: &[(&str, &str)] = &[
    ("clave", "String"),
    ("camino", "String"),
    ("version", "String"),
    ("etag", "String"),
    ("huella", "String"),
    ("formato", "String"),
    ("tamano", "Integer"),
    ("modificado", "String"),
    ("estado", "String"),
    ("entro", "Integer"),
    ("retirado", "Integer"),
    ("retirado_ms", "Integer"),
    ("blob", "String"),
    ("tipo", "String"),
];

/// Un ítem subido a una transacción abierta.
#[derive(Debug, Clone, PartialEq)]
pub struct Subido {
    /// En hexadecimal.
    pub sha256: String,
    pub tamano: u64,
    pub crc32c: u32,
    /// El que se sirve: el de los bytes; si no dicen nada, el declarado.
    pub tipo: String,
    pub detectado: Option<String>,
    pub declarado: Option<String>,
    pub modificado: String,
}

struct Transaccion {
    coleccion: String,
    permiso: String,
    items: BTreeMap<String, Subido>,
    caduca: Instant,
    /// Mientras se confirma, no entra nada más.
    confirmando: bool,
}

/// Las transacciones abiertas.
#[derive(Default)]
pub struct Escrituras {
    abiertas: Mutex<HashMap<String, Transaccion>>,
}

impl Escrituras {
    /// Una transacción nueva sobre `coleccion`: `(id, permiso, segundos)`.
    /// `None`: no caben más.
    pub fn abrir(&self, coleccion: &str, segundos: u64) -> Option<(String, String, u64)> {
        let segundos = segundos.clamp(VIDA_MINIMA, VIDA_MAXIMA);
        let ahora = Instant::now();
        let mut m = self.abiertas.lock().unwrap();
        if m.len() >= MAXIMAS {
            m.retain(|_, t| t.caduca > ahora);
            if m.len() >= MAXIMAS {
                return None;
            }
        }
        let id = format!("t-{}", uuid::Uuid::new_v4().simple());
        let permiso = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        m.insert(
            id.clone(),
            Transaccion {
                coleccion: coleccion.to_string(),
                permiso: permiso.clone(),
                items: BTreeMap::new(),
                caduca: ahora + Duration::from_secs(segundos),
                confirmando: false,
            },
        );
        Some((id, permiso, segundos))
    }

    /// La transacción de un permiso de subida, si vive y no se está
    /// confirmando: `(id, coleccion)`.
    fn de_permiso(&self, permiso: &str) -> Result<(String, String), Respuesta> {
        let ahora = Instant::now();
        let mut m = self.abiertas.lock().unwrap();
        m.retain(|_, t| t.caduca > ahora);
        let Some((id, t)) = m.iter().find(|(_, t)| t.permiso == permiso) else {
            return Err(sin_permiso());
        };
        if t.confirmando {
            return Err(problema(
                409,
                "media/transaccion",
                "la transacción se está confirmando: ya no entra nada",
            ));
        }
        Ok((id.clone(), t.coleccion.clone()))
    }

    fn anotar(&self, id: &str, camino: &str, s: Subido) -> bool {
        match self.abiertas.lock().unwrap().get_mut(id) {
            Some(t) if !t.confirmando => {
                t.items.insert(camino.to_string(), s);
                true
            }
            _ => false,
        }
    }

    /// Lo subido, para confirmar, y la transacción cerrada a más subidas.
    fn cerrar(&self, id: &str, coleccion: &str) -> Result<BTreeMap<String, Subido>, Respuesta> {
        let ahora = Instant::now();
        let mut m = self.abiertas.lock().unwrap();
        match m.get_mut(id) {
            Some(t) if t.caduca > ahora && t.coleccion == coleccion && !t.confirmando => {
                t.confirmando = true;
                Ok(t.items.clone())
            }
            Some(t) if t.coleccion != coleccion => Err(problema(
                409,
                "media/transaccion",
                format!("la transacción `{id}` es de `{}`", t.coleccion),
            )),
            Some(t) if t.confirmando => Err(problema(
                409,
                "media/transaccion",
                format!("la transacción `{id}` ya se está confirmando"),
            )),
            _ => Err(no_existe(id)),
        }
    }

    /// Si el sello falló, la transacción vuelve a admitir (y a confirmarse).
    fn reabrir(&self, id: &str) {
        if let Some(t) = self.abiertas.lock().unwrap().get_mut(id) {
            t.confirmando = false;
        }
    }

    fn quitar(&self, id: &str) -> bool {
        self.abiertas.lock().unwrap().remove(id).is_some()
    }

    pub fn cuantas(&self) -> usize {
        self.abiertas.lock().unwrap().len()
    }
}

fn sin_permiso() -> Respuesta {
    problema(
        401,
        "media/permiso",
        "el permiso de subida falta, caducó o su transacción ya se cerró: abre otra",
    )
}

fn no_existe(id: &str) -> Respuesta {
    problema(
        404,
        "media/transaccion",
        format!("no hay ninguna transacción abierta `{id}` (caducó, se cerró, o un reinicio)"),
    )
}

/// **Un camino de ítem que se puede escribir**: relativo, sin `.` ni `..` ni
/// segmentos vacíos, sin `\` ni caracteres de control, y no más largo que
/// [`CAMINO_MAXIMO`]. Es lo que después sale en una URL y en un nombre de
/// descarga: lo que no cumple, no entra.
pub fn camino_valido(c: &str) -> Result<(), String> {
    if c.is_empty() || c.len() > CAMINO_MAXIMO {
        return Err(format!("un camino tiene entre 1 y {CAMINO_MAXIMO} bytes"));
    }
    if c.chars().any(|x| x.is_control() || x == '\\') {
        return Err("un camino no lleva caracteres de control ni `\\`".into());
    }
    if c.split('/').any(|s| s.is_empty() || s == "." || s == "..") {
        return Err(format!(
            "`{c}` no es un camino relativo limpio: sin `/` al principio ni al final, ni `//`, `.` o `..`"
        ));
    }
    Ok(())
}

/// El `sha-256` de un `Repr-Digest` (RFC 9530), en base64, si lo trae.
fn digest_pedido(cabecera: &str) -> Option<String> {
    cabecera.split(',').find_map(|parte| {
        let (alg, valor) = parte.trim().split_once('=')?;
        if !alg.trim().eq_ignore_ascii_case("sha-256") {
            return None;
        }
        Some(valor.trim().trim_matches(':').to_string())
    })
}

/// Lo que llegó de una subida, contado al paso.
struct Recibido {
    sha256: [u8; 32],
    crc32c: u32,
    tamano: u64,
    cabeza: Vec<u8>,
    cuerpo: Cuerpo,
}

/// **Recibe los bytes al paso**: el `sha256`, el `crc32c` y los primeros
/// bytes mientras llegan; lo pequeño en memoria y lo grande a un temporal
/// (la memoria no crece con el fichero, como en `ore-store blobs`).
fn recibir(cuerpo: &mut dyn Read, largo: u64, temporal: &Path) -> Result<Recibido, String> {
    let mut sha = sha2::Sha256::new();
    let mut crc = Crc32c::default();
    let mut cabeza = Vec::with_capacity(CABEZA);
    let mut tamano = 0u64;
    let en_memoria = largo < EN_MEMORIA;
    let mut memoria = Vec::new();
    let mut fichero = None;
    if !en_memoria {
        let ruta = temporal.join(format!("ore-medios-{}", uuid::Uuid::new_v4().simple()));
        let f = std::fs::File::create(&ruta)
            .map_err(|e| format!("el temporal `{}` no se pudo crear: {e}", ruta.display()))?;
        fichero = Some((ruta, std::io::BufWriter::with_capacity(1 << 20, f)));
    } else {
        memoria.reserve(largo as usize);
    }
    let mut buf = vec![0u8; 1 << 16];
    let resultado = loop {
        let n = match cuerpo.read(&mut buf) {
            Ok(0) => break Ok(()),
            Ok(n) => n,
            Err(e) => break Err(format!("la subida se cortó: {e}")),
        };
        let trozo = &buf[..n];
        sha.update(trozo);
        crc.sumar(trozo);
        if cabeza.len() < CABEZA {
            let falta = CABEZA - cabeza.len();
            cabeza.extend_from_slice(&trozo[..n.min(falta)]);
        }
        tamano += n as u64;
        match &mut fichero {
            Some((_, f)) => {
                if let Err(e) = f.write_all(trozo) {
                    break Err(format!("el temporal no se pudo escribir: {e}"));
                }
            }
            None => memoria.extend_from_slice(trozo),
        }
    };
    let cuerpo = match fichero {
        Some((ruta, mut f)) => {
            let hecho = resultado.and_then(|_| {
                f.flush()
                    .map_err(|e| format!("el temporal no se pudo escribir: {e}"))
            });
            if let Err(e) = hecho {
                drop(f);
                let _ = std::fs::remove_file(&ruta);
                return Err(e);
            }
            Cuerpo::Fichero(ruta)
        }
        None => {
            resultado?;
            Cuerpo::Memoria(memoria)
        }
    };
    if tamano != largo {
        borrar_temporal(&cuerpo);
        return Err(format!("llegaron {tamano} bytes de los {largo} anunciados"));
    }
    Ok(Recibido {
        sha256: sha.finalize().into(),
        crc32c: crc.valor(),
        tamano,
        cabeza,
        cuerpo,
    })
}

fn borrar_temporal(c: &Cuerpo) {
    if let Cuerpo::Fichero(p) = c {
        let _ = std::fs::remove_file(p);
    }
}

/// Dónde van las subidas grandes mientras llegan.
fn temporal() -> PathBuf {
    std::env::var("ORE_MEDIOS_TEMPORAL")
        .ok()
        .filter(|t| !t.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

/// `2026-10-01T12:00:00Z`, ahora.
fn ahora_iso() -> String {
    let (h, _) = ore_sigv4::firma::ahora();
    // `YYYYMMDDTHHMMSSZ` → ISO 8601 extendido.
    format!(
        "{}-{}-{}T{}:{}:{}Z",
        &h[0..4],
        &h[4..6],
        &h[6..8],
        &h[9..11],
        &h[11..13],
        &h[13..15]
    )
}

fn ahora_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// La extensión de un camino, en minúsculas: el `formato` de su fila.
fn formato_de(camino: &str) -> String {
    let nombre = camino.rsplit('/').next().unwrap_or(camino);
    match nombre.rsplit_once('.') {
        Some((a, e)) if !a.is_empty() => e.to_ascii_lowercase(),
        _ => String::new(),
    }
}

/// Una fila del manifiesto: un objeto plano de textos (lo que `sellar` lee).
fn fila(campos: &[(&str, String)]) -> String {
    Json::Obj(
        campos
            .iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(k, v)| (k.to_string(), Json::s(v)))
            .collect(),
    )
    .jcs()
}

fn fila_nueva(camino: &str, s: &Subido, tx: i64) -> String {
    fila(&[
        ("clave", camino.to_string()),
        ("camino", camino.to_string()),
        ("version", s.sha256.clone()),
        ("huella", format!("sha256:{}", s.sha256)),
        ("formato", formato_de(camino)),
        ("tamano", s.tamano.to_string()),
        ("modificado", s.modificado.clone()),
        ("estado", "actual".into()),
        ("entro", tx.to_string()),
        ("blob", s.sha256.clone()),
        ("tipo", s.tipo.clone()),
    ])
}

/// La fila actual de un camino, retirada en la transacción `tx`.
fn fila_retirada(a: &crate::indice::Item, tx: i64, ms: i64) -> String {
    let o = |v: &Option<String>| v.clone().unwrap_or_default();
    fila(&[
        ("clave", a.clave.clone().unwrap_or_else(|| a.camino.clone())),
        ("camino", a.camino.clone()),
        ("version", a.version.clone()),
        ("etag", o(&a.etag)),
        ("huella", a.huella.clone()),
        ("formato", o(&a.formato)),
        (
            "tamano",
            a.tamano.map(|t| t.to_string()).unwrap_or_default(),
        ),
        ("modificado", o(&a.modificado)),
        ("estado", "retirado".into()),
        ("entro", o(&a.entro)),
        ("retirado", tx.to_string()),
        ("retirado_ms", ms.to_string()),
        ("blob", o(&a.blob)),
        ("tipo", o(&a.tipo)),
    ])
}

/// La cabecera del manifiesto de una escrita: la de una mantenida (plan,
/// esquema, clave), con la transacción por testigo.
pub fn cabecera(coleccion: &str, tx: i64) -> Cabecera {
    Cabecera {
        plan: ore_core::digest::de_bytes(format!("coleccion:{coleccion}").as_bytes()),
        esquema: COLUMNAS
            .iter()
            .map(|(c, t)| (c.to_string(), t.to_string()))
            .collect(),
        testigo: Testigo {
            modo: "transaccion".into(),
            valor: Some(tx.to_string()),
        },
        clave: vec!["clave".into(), "version".into()],
        // No la autoriza un conducto: la escribe código, con el permiso que
        // `ore-serve` dio a esa transacción.
        conducto: "media.escrita".into(),
        obligatorias: Default::default(),
    }
}

fn texto<'a>(n: &'a Node, k: &str) -> Option<&'a str> {
    n.get(k)
        .and_then(|(_, v)| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

impl Servicio {
    /// `POST /escritura/abrir {coleccion, ttl_s?}`.
    pub(crate) fn escritura_abrir(&self, n: &Node) -> Respuesta {
        let Some(coleccion) = texto(n, "coleccion").filter(|c| c.split('.').count() == 3) else {
            return problema(400, "media/peticion", "falta `coleccion` (`b.s.n`)");
        };
        let ttl = n
            .get("ttl_s")
            .and_then(|(_, v)| v.as_str())
            .and_then(|t| t.trim().parse().ok())
            .unwrap_or(VIDA_POR_DEFECTO);
        let Some((id, permiso, segundos)) = self.escrituras.abrir(coleccion, ttl) else {
            return problema(
                429,
                "media/limite",
                format!("hay {MAXIMAS} transacciones abiertas: confirma o aborta alguna"),
            );
        };
        Respuesta {
            codigo: 201,
            cuerpo: Json::obj([
                ("transaccion", Json::s(id)),
                ("coleccion", Json::s(coleccion)),
                ("permiso", Json::s(permiso)),
                ("ttl_s", Json::Int(segundos as i64)),
                (
                    "expires_ms",
                    Json::Int(ahora_ms() + (segundos as i64) * 1000),
                ),
            ]),
        }
    }

    /// `POST /escritura/abortar {transaccion}`: no deja nada.
    pub(crate) fn escritura_abortar(&self, n: &Node) -> Respuesta {
        let Some(id) = texto(n, "transaccion") else {
            return problema(400, "media/peticion", "falta `transaccion`");
        };
        if self.escrituras.quitar(id) {
            Respuesta::sin_contenido()
        } else {
            no_existe(id)
        }
    }

    /// **`PUT /subida?permiso=&path=`** (el puerto del puesto): un ítem a una
    /// transacción abierta. `201` con su referencia.
    pub fn subir(&self, p: &Peticion, cuerpo: &mut dyn Read, largo: u64) -> Respuesta {
        let Some(permiso) = p.consulta.get("permiso") else {
            return sin_permiso();
        };
        let (id, coleccion) = match self.escrituras.de_permiso(permiso) {
            Ok(t) => t,
            Err(r) => return r,
        };
        let Some(camino) = p.consulta.get("path") else {
            return problema(422, "media/peticion", "falta `path`: dónde va el ítem");
        };
        if let Err(m) = camino_valido(camino) {
            return problema(422, "media/peticion", m);
        }
        let r = match recibir(cuerpo, largo, &temporal()) {
            Ok(r) => r,
            Err(e) => return problema(400, "media/peticion", e),
        };
        let hex = ore_sigv4::hex(&r.sha256);
        if let Some(pedido) = p
            .cabeceras
            .get("repr-digest")
            .and_then(|c| digest_pedido(c))
            && pedido != ore_sigv4::base64(&r.sha256)
        {
            borrar_temporal(&r.cuerpo);
            return problema(
                422,
                "media/digest-no-casa",
                format!("el `Repr-Digest` no es el de los bytes que llegaron (sha256:{hex})"),
            );
        }
        let detectado = ore_core::medios::tipo_por_bytes(&r.cabeza).map(str::to_string);
        let declarado = p
            .cabeceras
            .get("content-type")
            .and_then(|t| t.split(';').next())
            .map(|t| t.trim().to_ascii_lowercase())
            .filter(|t| !t.is_empty() && t.len() <= 255 && t.contains('/'));
        let tipo = detectado
            .clone()
            .or_else(|| declarado.clone())
            .unwrap_or_else(|| TIPO_POR_DEFECTO.into());
        let blob = Blob {
            clave: clave_de(&hex),
            tipo: tipo.clone(),
            tamano: r.tamano,
            sha256: r.sha256,
            crc32c: r.crc32c,
            cuerpo: r.cuerpo,
        };
        let subido = self.cuenta.poner_blob(&blob);
        borrar_temporal(&blob.cuerpo);
        let subido = match subido {
            Ok(s) => s,
            Err(e) => return problema(502, "media/origen", format!("el lago no lo guardó: {e}")),
        };
        // Ya estaba: que la recogida no se lo lleve antes de que una fila lo
        // nombre (E8·3b).
        if !subido {
            let _ = self.cuenta.tocar(&blob.clave);
        }
        let s = Subido {
            sha256: hex.clone(),
            tamano: r.tamano,
            crc32c: r.crc32c,
            tipo: tipo.clone(),
            detectado: detectado.clone(),
            declarado,
            modificado: ahora_iso(),
        };
        if !self.escrituras.anotar(&id, camino, s.clone()) {
            return sin_permiso();
        }
        let nulo = || Json::Crudo("null".into());
        Respuesta {
            codigo: 201,
            cuerpo: Json::obj([
                (
                    "uri",
                    Json::s(format!("ore://{coleccion}/{camino}?v={hex}")),
                ),
                ("collection", Json::s(&coleccion)),
                ("path", Json::s(camino)),
                ("version", Json::s(&hex)),
                ("digest", Json::s(format!("sha256:{hex}"))),
                ("size", Json::Int(r.tamano as i64)),
                ("content_type", Json::s(&tipo)),
                (
                    "content_type_detected",
                    detectado.map(Json::s).unwrap_or_else(nulo),
                ),
                ("checksum", Json::s(format!("crc32c:{:08x}", r.crc32c))),
                ("transaction", Json::s(&id)),
                ("stored", Json::Bool(subido)),
            ]),
        }
    }

    /// **`POST /escritura/confirmar`**: sella lo subido sobre la base que
    /// `ore-serve` nombra (`metadata_location` y `base`, la transacción de su
    /// puntero; sin ellas, la colección nace). Devuelve lo que `sellar` dice
    /// —`metadata_location`, `snapshot`…— y la transacción nueva, para el
    /// puntero.
    pub(crate) fn escritura_confirmar(&self, n: &Node) -> Respuesta {
        let (Some(id), Some(coleccion)) = (texto(n, "transaccion"), texto(n, "coleccion")) else {
            return problema(400, "media/peticion", "faltan `transaccion` y `coleccion`");
        };
        let base_ml = texto(n, "metadata_location");
        let base_tx: i64 = match texto(n, "base").map(str::parse) {
            None => 0,
            Some(Ok(b)) => b,
            Some(Err(_)) => {
                return problema(
                    400,
                    "media/peticion",
                    "`base` es la transacción del puntero",
                );
            }
        };
        let items = match self.escrituras.cerrar(id, coleccion) {
            Ok(i) => i,
            Err(r) => return r,
        };
        let r = self.sellar_escrita(coleccion, n, base_ml, base_tx, &items);
        // `cerrar: false` (B4b·2): quien escribe el puntero cierra después
        // (`abortar`) —si pierde la carrera de la forja, la transacción sigue
        // abierta y se vuelve a confirmar sobre la base nueva—.
        if r.codigo == 200 && texto(n, "cerrar") != Some("false") {
            self.escrituras.quitar(id);
        } else {
            self.escrituras.reabrir(id);
        }
        r
    }

    fn sellar_escrita(
        &self,
        coleccion: &str,
        n: &Node,
        base_ml: Option<&str>,
        base_tx: i64,
        items: &BTreeMap<String, Subido>,
    ) -> Respuesta {
        let tx = base_tx + 1;
        let base_ix = match base_ml {
            Some(ml) => {
                let b = base_tx.to_string();
                let pedido = Pedido {
                    coleccion,
                    virtual_: false,
                    metadata_location: ml,
                    transaccion: &b,
                };
                match self.indice(&pedido) {
                    Ok(ix) => Some(ix),
                    Err(e) => {
                        return problema(502, "media/origen", format!("la base no se lee: {e}"));
                    }
                }
            }
            None => None,
        };
        let ms = ahora_ms();
        let (mut entran, mut cambian, mut iguales) = (0, 0, 0);
        let mut filas = Vec::new();
        for (camino, s) in items {
            match base_ix.as_ref().and_then(|ix| ix.por_camino(camino, None)) {
                Some(a) if a.blob.as_deref() == Some(s.sha256.as_str()) => {
                    iguales += 1;
                    continue;
                }
                Some(a) => {
                    filas.push(fila_retirada(a, tx, ms));
                    cambian += 1;
                }
                None => entran += 1,
            }
            filas.push(fila_nueva(camino, s, tx));
        }
        // Lo que el puntero cuenta (`items`, como el de una mantenida).
        let antes = |e: &str| base_ix.as_ref().map_or(0, |ix| ix.cuantos(e) as i64);
        let (actuales, retirados, perdidos) = (
            antes("actual") + entran,
            antes("retirado") + cambian,
            antes("perdido"),
        );
        let resumen = |m: &mut BTreeMap<String, Json>| {
            m.insert(
                "items".into(),
                Json::obj([
                    ("actuales", Json::Int(actuales)),
                    ("retirados", Json::Int(retirados)),
                    ("perdidos", Json::Int(perdidos)),
                ]),
            );
            m.insert(
                "cambios".into(),
                Json::obj([
                    ("entran", Json::Int(entran)),
                    ("cambian", Json::Int(cambian)),
                    ("iguales", Json::Int(iguales)),
                ]),
            );
        };
        // Nada que cambiar sobre una base: el manifiesto se queda.
        if filas.is_empty()
            && let Some(ml) = base_ml
        {
            let mut m = BTreeMap::new();
            m.insert("metadata_location".into(), Json::s(ml));
            m.insert("transaccion".into(), Json::Int(base_tx));
            m.insert("sin_cambios".into(), Json::Bool(true));
            resumen(&mut m);
            return Respuesta::ok(Json::Obj(m));
        }
        let dataset = texto(n, "dataset")
            .map(str::to_string)
            .unwrap_or_else(|| format!("colecciones/{}", coleccion.replace('.', "/")));
        let cab = cabecera(coleccion, tx);
        let dicho = match self.listados.sellar(&cab, &dataset, base_ml, &filas) {
            Ok(d) => d,
            Err(e) => return problema(502, "media/origen", format!("el sello falló: {e}")),
        };
        let mut m = match ore_core::parse::parse(dicho.trim()).map(|n| Json::de_node(&n)) {
            Ok(Json::Obj(m)) => m,
            _ => return problema(502, "media/origen", "el sello no contestó JSON"),
        };
        m.insert("transaccion".into(), Json::Int(tx));
        m.insert("dataset".into(), Json::s(&dataset));
        resumen(&mut m);
        Respuesta::ok(Json::Obj(m))
    }
}

#[cfg(test)]
pub(crate) mod pruebas {
    use super::*;

    #[test]
    fn un_camino_es_relativo_y_limpio() {
        for bien in ["a.pdf", "docs/2026/a b.pdf", "ñandú/x.png"] {
            assert!(camino_valido(bien).is_ok(), "{bien}");
        }
        for mal in ["", "/a", "a/", "a//b", "../a", "a/./b", "a\\b", "a\nb"] {
            assert!(camino_valido(mal).is_err(), "{mal:?}");
        }
        assert!(camino_valido(&"x".repeat(CAMINO_MAXIMO + 1)).is_err());
    }

    #[test]
    fn el_repr_digest_se_lee_entre_otros() {
        assert_eq!(
            digest_pedido("sha-512=:AAA=:, sha-256=:q1w2:").as_deref(),
            Some("q1w2")
        );
        assert_eq!(digest_pedido("sha-512=:AAA=:"), None);
    }

    #[test]
    fn lo_grande_va_a_disco_y_se_cuenta_igual() {
        let dir = std::env::temp_dir();
        let grande: Vec<u8> = (0..EN_MEMORIA + 10).map(|i| (i % 253) as u8).collect();
        let r = recibir(&mut &grande[..], grande.len() as u64, &dir).unwrap();
        assert!(matches!(r.cuerpo, Cuerpo::Fichero(_)));
        assert_eq!(r.sha256, <[u8; 32]>::from(sha2::Sha256::digest(&grande)));
        assert_eq!(r.cabeza.len(), CABEZA);
        assert_eq!(r.cuerpo.bytes().unwrap(), grande);
        borrar_temporal(&r.cuerpo);
        let corto = recibir(&mut &b"abc"[..], 5, &dir);
        assert!(corto.is_err(), "faltan bytes: no se da por bueno");
    }

    #[test]
    fn el_formato_es_la_extension() {
        assert_eq!(formato_de("docs/A.PDF"), "pdf");
        assert_eq!(formato_de("docs/.oculto"), "");
        assert_eq!(formato_de("sin"), "");
    }

    /// Un almacén en memoria, como el de las pruebas de `ore-store`: lo justo
    /// para sellar y leer un manifiesto de verdad sin red.
    #[derive(Default)]
    pub struct Memoria(pub Mutex<BTreeMap<String, Vec<u8>>>);

    impl ore_store::almacen::Almacen for Memoria {
        fn base(&self) -> String {
            "memory://pruebas".into()
        }
        fn leer(&self, clave: &str) -> Result<Option<String>, String> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .get(clave)
                .map(|b| String::from_utf8_lossy(b).into_owned()))
        }
        fn existe(&self, clave: &str) -> Result<bool, String> {
            Ok(self.0.lock().unwrap().contains_key(clave))
        }
        fn subir(&self, clave: &str, cuerpo: &[u8]) -> Result<bool, String> {
            let mut m = self.0.lock().unwrap();
            if m.contains_key(clave) {
                return Ok(false);
            }
            m.insert(clave.to_string(), cuerpo.to_vec());
            Ok(true)
        }
        fn listar(&self, prefijo: &str) -> Result<Vec<String>, String> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .keys()
                .filter(|k| k.starts_with(prefijo))
                .cloned()
                .collect())
        }
        fn borrar(&self, clave: &str) -> Result<(), String> {
            self.0.lock().unwrap().remove(clave);
            Ok(())
        }
        fn leer_bytes(&self, clave: &str) -> Result<Option<Vec<u8>>, String> {
            Ok(self.0.lock().unwrap().get(clave).cloned())
        }
    }

    fn servicio() -> (Servicio, std::sync::Arc<Memoria>) {
        let m = std::sync::Arc::new(Memoria::default());
        (
            Servicio {
                listados: Box::new(ore_store::lago::Lago::nuevo(m.clone())),
                cuenta: m.clone(),
                indices: crate::indice::Indices::nuevo(1_000_000),
                vistos: Default::default(),
                permisos: Default::default(),
                escrituras: Default::default(),
            },
            m,
        )
    }

    fn pedir(s: &Servicio, ruta: &str, cuerpo: String) -> (u16, Node) {
        let r = s.atender(&Peticion {
            metodo: "POST".into(),
            ruta: ruta.into(),
            cabeceras: BTreeMap::new(),
            cuerpo,
            consulta: BTreeMap::new(),
        });
        let t = if r.codigo == 204 {
            "{}".to_string()
        } else {
            r.cuerpo.jcs()
        };
        (r.codigo, ore_core::parse::parse(&t).unwrap())
    }

    fn campo(n: &Node, k: &str) -> String {
        n.get(k)
            .and_then(|(_, v)| v.as_str())
            .unwrap_or("")
            .to_string()
    }

    fn subir_a(
        s: &Servicio,
        permiso: &str,
        camino: &str,
        bytes: &[u8],
        cabeceras: &[(&str, &str)],
    ) -> (u16, Node) {
        let r = s.subir(
            &Peticion {
                metodo: "PUT".into(),
                ruta: "/subida".into(),
                cabeceras: cabeceras
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
                cuerpo: String::new(),
                consulta: [
                    ("permiso".to_string(), permiso.to_string()),
                    ("path".to_string(), camino.to_string()),
                ]
                .into(),
            },
            &mut &bytes[..],
            bytes.len() as u64,
        );
        (r.codigo, ore_core::parse::parse(&r.cuerpo.jcs()).unwrap())
    }

    fn abrir(s: &Servicio) -> (String, String) {
        let (c, n) = pedir(
            s,
            "/escritura/abrir",
            r#"{"coleccion":"legal.archivo.paginas"}"#.into(),
        );
        assert_eq!(c, 201);
        (campo(&n, "transaccion"), campo(&n, "permiso"))
    }

    fn confirmar(
        s: &Servicio,
        tx: &str,
        coleccion: &str,
        base: Option<(&str, &str)>,
    ) -> (u16, Node) {
        let base = match base {
            Some((ml, b)) => format!(r#","metadata_location":"{ml}","base":"{b}""#),
            None => String::new(),
        };
        pedir(
            s,
            "/escritura/confirmar",
            format!(r#"{{"transaccion":"{tx}","coleccion":"{coleccion}"{base}}}"#),
        )
    }

    fn listado(s: &Servicio, ml: &str, tx: &str, estado: &str) -> String {
        let (c, n) = pedir(
            s,
            "/indice/items",
            format!(
                r#"{{"coleccion":"legal.archivo.paginas","metadata_location":"{ml}","transaccion":"{tx}","estado":"{estado}"}}"#
            ),
        );
        assert_eq!(c, 200);
        Json::de_node(&n).jcs()
    }

    fn sha(b: &[u8]) -> String {
        ore_sigv4::hex(&sha2::Sha256::digest(b))
    }

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n-una-pagina-";
    const PDF: &[u8] = b"%PDF-1.7\n-un-contrato-";
    const JPG: &[u8] = b"\xff\xd8\xff\xe0-otra-pagina-";
    const PAGINAS: &str = "legal.archivo.paginas";

    /// 0049 B4b·1, de punta a punta con un lago en memoria: abrir, subir
    /// (al paso, el tipo por los bytes, el digest cotejado), confirmar —el
    /// manifiesto sellado y leído por el índice de siempre—, reescribir un
    /// camino sobre la base, lo igual no cambia, y abortar no deja nada.
    #[test]
    fn una_coleccion_escrita_se_llena_por_transacciones() {
        let (s, lago) = servicio();
        let (tx, permiso) = abrir(&s);
        let (png, jpg) = (sha(PNG), sha(JPG));

        // Subir: el tipo es el de los bytes, no el declarado.
        let (c, n) = subir_a(
            &s,
            &permiso,
            "p/1.png",
            PNG,
            &[("content-type", "text/html")],
        );
        assert_eq!(c, 201, "{n:?}");
        assert_eq!(campo(&n, "content_type"), "image/png");
        assert_eq!(campo(&n, "digest"), format!("sha256:{png}"));
        assert!(lago.0.lock().unwrap().contains_key(&clave_de(&png)));

        // Con su Repr-Digest, cotejado; uno que no casa no deja nada.
        let bueno = format!(
            "sha-256=:{}:",
            ore_sigv4::base64(&sha2::Sha256::digest(PDF))
        );
        let (c, _) = subir_a(&s, &permiso, "p/2.pdf", PDF, &[("repr-digest", &bueno)]);
        assert_eq!(c, 201);
        let (c, n) = subir_a(&s, &permiso, "p/3.jpg", JPG, &[("repr-digest", &bueno)]);
        assert_eq!(
            (c, campo(&n, "type").as_str()),
            (422, "media/digest-no-casa")
        );
        assert!(!lago.0.lock().unwrap().contains_key(&clave_de(&jpg)));

        // Un camino sucio, un permiso que no es.
        assert_eq!(subir_a(&s, &permiso, "../x.png", PNG, &[]).0, 422);
        assert_eq!(subir_a(&s, "otro", "p/x.png", PNG, &[]).0, 401);

        // Confirmar sin base: la colección nace, transacción 1.
        let (c, n) = confirmar(&s, &tx, PAGINAS, None);
        assert_eq!(c, 200, "{}", Json::de_node(&n).jcs());
        assert_eq!(campo(&n, "transaccion"), "1");
        let ml1 = campo(&n, "metadata_location");
        assert!(ml1.starts_with("memory://pruebas/"), "{ml1}");
        let l = listado(&s, &ml1, "1", "actual");
        assert!(
            l.contains("\"path\":\"p/1.png\"") && l.contains("\"path\":\"p/2.pdf\""),
            "{l}"
        );
        assert!(l.contains(&format!("\"digest\":\"sha256:{png}\"")), "{l}");
        assert!(l.contains("\"content_type\":\"image/png\""), "{l}");
        // Confirmada, la transacción ya no admite.
        assert_eq!(subir_a(&s, &permiso, "p/4.png", PNG, &[]).0, 401);

        // Otra: p/1.png cambia, p/2.pdf es lo mismo (y no se sube otra vez).
        let (tx2, permiso2) = abrir(&s);
        assert_eq!(subir_a(&s, &permiso2, "p/1.png", JPG, &[]).0, 201);
        let (_, n) = subir_a(&s, &permiso2, "p/2.pdf", PDF, &[]);
        assert_eq!(campo(&n, "stored"), "false", "ya estaba en el lago");
        let (c, n) = confirmar(&s, &tx2, PAGINAS, Some((&ml1, "1")));
        assert_eq!(c, 200, "{}", Json::de_node(&n).jcs());
        assert_eq!(campo(&n, "transaccion"), "2");
        let cambios = Json::de_node(n.get("cambios").unwrap().1).jcs();
        assert_eq!(cambios, r#"{"cambian":1,"entran":0,"iguales":1}"#);
        let items = Json::de_node(n.get("items").unwrap().1).jcs();
        assert_eq!(items, r#"{"actuales":2,"perdidos":0,"retirados":1}"#);
        let ml2 = campo(&n, "metadata_location");
        let actual = listado(&s, &ml2, "2", "actual");
        assert!(
            actual.contains(&format!("\"digest\":\"sha256:{jpg}\"")),
            "{actual}"
        );
        assert!(
            actual.contains("\"content_type\":\"image/jpeg\""),
            "{actual}"
        );
        let retirado = listado(&s, &ml2, "2", "retirado");
        assert!(
            retirado.contains(&format!("\"digest\":\"sha256:{png}\"")),
            "{retirado}"
        );

        // Nada nuevo sobre la base: el manifiesto se queda.
        let (tx3, permiso3) = abrir(&s);
        subir_a(&s, &permiso3, "p/2.pdf", PDF, &[]);
        let (c, n) = confirmar(&s, &tx3, PAGINAS, Some((&ml2, "2")));
        assert_eq!((c, campo(&n, "sin_cambios").as_str()), (200, "true"));
        assert_eq!(campo(&n, "metadata_location"), ml2);

        // Otra colección, 409; abortar no deja nada y su permiso ya no sube.
        let (tx4, permiso4) = abrir(&s);
        let (c, n) = confirmar(&s, &tx4, "legal.archivo.otra", None);
        assert_eq!((c, campo(&n, "type").as_str()), (409, "media/transaccion"));
        let (c, _) = pedir(
            &s,
            "/escritura/abortar",
            format!(r#"{{"transaccion":"{tx4}"}}"#),
        );
        assert_eq!(c, 204);
        assert_eq!(subir_a(&s, &permiso4, "p/5.png", PNG, &[]).0, 401);
        assert_eq!(s.escrituras.cuantas(), 0);
    }
}
