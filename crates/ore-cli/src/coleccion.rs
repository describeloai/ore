//! **La transacción de una colección** (0046 E8·1c): lo que `ore materialize`
//! hace con una `MediaCollection` mantenida, como con un `Dataset`.
//!
//! # El manifiesto es una tabla de sus ítems (E8·1a, medido)
//!
//! Una fila por **(clave, versión)** del origen, con su huella de contenido,
//! su formato, su tamaño, su **estado** —`actual`, `retirado`, `perdido`— y la
//! transacción en que entró y en la que salió. Se sella en el lago como un
//! dataset —Iceberg, `sellar` con fusión por la clave—, así que hereda el
//! puntero con CAS, la historia, `volcar`, `/v1` y SQL. Medido con 100.000
//! ítems: una transacción de tres cambios, 0,64 s; leer lo actual, 0,18 s.
//!
//! # La transacción es la diferencia de dos listados de versiones (E7)
//!
//! - lo vigente que no estaba **entra**;
//! - lo actual que ya no es vigente **se retira**: sale de la vista actual y
//!   se queda en la tabla (spec `02` §5). Sobrescribir es retirar y entrar:
//!   la clave es la misma, la versión no;
//! - lo retirado cuya versión ya no existe en el origen está **perdido**: una
//!   colección virtual no puede servirlo, y se dice en vez de fingirlo. Lo que
//!   sigue existiendo como versión vieja (una marca de borrado encima, o una
//!   versión nueva) se sigue pudiendo servir por su `VersionId`;
//! - si el testigo del listado no cambió, **no se lee ni se escribe nada**.
//!
//! # La mantenida copia los bytes (E8·2)
//!
//! La misma transacción, y cada ítem que entra lleva además su **blob**: el
//! sha256 de sus bytes en el lago del inquilino (`ore-store`, `blobs.rs`) y su
//! tipo. Para cada uno, por este orden:
//!
//! 1. **por su huella, en el propio manifiesto**: un renombrado o un duplicado
//!    no se baja (E7: la segunda transacción del experimento, ni un byte). Es
//!    seguro porque la recogida sólo se lleva blobs que ninguna fila nombra;
//! 2. **por su huella, en el índice del lago** (`blobs-hay`): lo que otra
//!    colección del inquilino ya trajo, o lo que subió un Job que se cortó
//!    antes de sellar;
//! 3. **del origen**: `ore-read-<tipo> bajar` —fijado a su versión y cotejado
//!    con su huella mientras baja— encauzado a `ore-store blobs`, que lo guarda
//!    cotejado por el servidor.
//!
//! **El manifiesto se sella con los blobs ya en el lago**: una fila nunca
//! nombra un blob que no está. Un ítem que no se pudo copiar no entra en esta
//! transacción —se dice cuál y por qué— y el testigo no avanza, así que la
//! pasada siguiente lo vuelve a intentar. En la mantenida nada se pierde: lo
//! retirado sigue en el lago aunque el origen ya no tenga su versión.
//!
//! # La retención (E8·3a)
//!
//! Lo que sale de la vista actual guarda **cuándo** (`retirado_ms`): la
//! transacción es un número y los snapshots caducan, así que la fecha va en la
//! fila. El mantenimiento (`ore collections --recoger`) quita del manifiesto lo
//! retirado o perdido más viejo que `retention`; sin `retention`, nada caduca
//! (spec `02` §3). Lo que deja de nombrarse, la recogida de blobs se lo lleva.
//! Un Job que reutiliza el blob de una fila que no es actual lo **toca** antes
//! de sellar, así que la recogida no se lo lleva por debajo.

use crate::lector;
use crate::materializar::{almacen, cabecera, campo_de, leer_puntero, programa_del_almacen};
use ore_core::document::Kind;
use ore_core::json::Json;
use ore_core::link::{Loaded, Package};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// **El techo** (E8·3c): por encima de estas filas, el manifiesto deja de
/// ser barato. La fusión del lago es *copy-on-write*: cada transacción
/// reescribe la tabla entera. Medido (E8·1a y E8·2 B, en el clúster): con
/// 100.000 filas, una transacción son ~3 s y 14 MB; crece lineal, así que con
/// un millón son ~30 s y ~140 MB por cambio, y la retención y la recogida leen
/// lo mismo cada noche. Pasado el techo toca *merge-on-read*; hasta entonces
/// se dice, en el puntero y en `ore collections`.
pub const TECHO: i64 = 500_000;

/// Lo que el puntero dice del techo, si se pasó.
pub fn techo(filas: i64) -> Option<Json> {
    (filas > TECHO).then(|| {
        Json::obj([
            ("filas", Json::Int(filas)),
            ("limite", Json::Int(TECHO)),
            (
                "aviso",
                Json::s(format!(
                    "{filas} filas en el manifiesto: cada transacción lo reescribe entero (~3 s y ~14 MB por cada 100.000); por encima de {TECHO} toca merge-on-read (0046 E8·3c)"
                )),
            ),
        ])
    })
}

fn campo_de_json(j: &Json, k: &str) -> Option<String> {
    match j {
        Json::Obj(m) => match m.get(k) {
            Some(Json::Str(v)) => Some(v.clone()),
            _ => None,
        },
        _ => None,
    }
}

/// Pone o quita el aviso del techo en un puntero.
fn con_techo(m: &mut BTreeMap<String, Json>, filas: i64) {
    match techo(filas) {
        Some(t) => {
            m.insert("techo".into(), t);
        }
        None => {
            m.remove("techo");
        }
    }
}

/// Ahora, en milisegundos desde 1970.
pub fn ahora_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Las columnas del manifiesto, con su tipo de OOS.
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
];

/// Las de la mantenida: las mismas y su blob (sha256) y su tipo.
const COLUMNAS_MANTENIDA: &[(&str, &str)] = &[("blob", "String"), ("tipo", "String")];

/// Un ítem del manifiesto: una fila.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub clave: String,
    pub camino: String,
    pub version: String,
    pub etag: String,
    pub huella: String,
    pub formato: String,
    pub tamano: i64,
    pub modificado: String,
    pub estado: String,
    pub entro: i64,
    pub retirado: Option<i64>,
    /// Cuándo salió de la vista actual (ms desde 1970): lo que la retención mide.
    pub retirado_ms: Option<i64>,
    /// El sha256 de sus bytes en el lago (la mantenida); vacío en la virtual.
    pub blob: String,
    pub tipo: String,
}

impl Item {
    fn id(&self) -> (String, String) {
        (self.clave.clone(), self.version.clone())
    }

    fn fila(&self) -> String {
        let mut m: BTreeMap<String, Json> = BTreeMap::new();
        for (k, v) in [
            ("clave", &self.clave),
            ("camino", &self.camino),
            ("version", &self.version),
            ("etag", &self.etag),
            ("huella", &self.huella),
            ("formato", &self.formato),
            ("modificado", &self.modificado),
            ("estado", &self.estado),
        ] {
            m.insert(k.into(), Json::s(v));
        }
        m.insert("tamano".into(), Json::s(self.tamano.to_string()));
        m.insert("entro".into(), Json::s(self.entro.to_string()));
        if let Some(r) = self.retirado {
            m.insert("retirado".into(), Json::s(r.to_string()));
        }
        if let Some(r) = self.retirado_ms {
            m.insert("retirado_ms".into(), Json::s(r.to_string()));
        }
        for (k, v) in [("blob", &self.blob), ("tipo", &self.tipo)] {
            if !v.is_empty() {
                m.insert(k.into(), Json::s(v));
            }
        }
        Json::Obj(m).jcs()
    }

    fn de_fila(n: &ore_core::parse::Node) -> Option<Item> {
        let c = |k: &str| campo_de(n, k).unwrap_or_default();
        let e = |k: &str| campo_de(n, k).and_then(|v| v.parse::<i64>().ok());
        Some(Item {
            clave: campo_de(n, "clave")?,
            camino: c("camino"),
            version: campo_de(n, "version")?,
            etag: c("etag"),
            huella: c("huella"),
            formato: c("formato"),
            tamano: e("tamano").unwrap_or(0),
            modificado: c("modificado"),
            estado: campo_de(n, "estado")?,
            entro: e("entro").unwrap_or(0),
            retirado: e("retirado"),
            retirado_ms: e("retirado_ms"),
            blob: c("blob"),
            tipo: c("tipo"),
        })
    }
}

/// Lo vigente en el origen, como lo da `ore-read-<tipo> versiones`.
#[derive(Debug, Clone)]
pub struct Vigente {
    pub clave: String,
    pub camino: String,
    pub version: String,
    pub etag: String,
    pub tamano: i64,
    pub modificado: String,
    pub huella: String,
}

/// El formato de un camino: su extensión, en minúsculas, con los dos nombres
/// de siempre juntados (`jpeg` es `jpg`, `tif` es `tiff`).
pub fn formato_de(camino: &str) -> String {
    let base = camino.rsplit('/').next().unwrap_or(camino);
    let ext = match base.rsplit_once('.') {
        Some((_, e)) => e.to_ascii_lowercase(),
        None => String::new(),
    };
    match ext.as_str() {
        "jpeg" => "jpg".into(),
        "tif" => "tiff".into(),
        _ => ext,
    }
}

/// Lo que cambia en una transacción: las filas que se escriben y las cuentas.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Cambios {
    pub filas: Vec<Item>,
    pub entran: usize,
    pub retiran: usize,
    pub pierden: usize,
    pub fuera_de_formato: usize,
}

/// **La transacción, pura**: de lo que había (`antes`, todas sus filas), lo
/// vigente, qué versiones conocidas siguen existiendo y los formatos de la
/// colección, a las filas que cambian. Determinista: mismo estado, mismas
/// filas.
pub fn transaccion(
    antes: &[Item],
    vigentes: &[Vigente],
    existen: &BTreeSet<(String, String)>,
    formatos: &[String],
    tx: i64,
) -> Cambios {
    let mut c = Cambios::default();
    let admitido = |camino: &str| {
        let f = formato_de(camino);
        formatos.is_empty() || formatos.iter().any(|x| formato_de(&format!("x.{x}")) == f)
    };
    let vivos: BTreeMap<(String, String), &Vigente> = vigentes
        .iter()
        .filter(|v| {
            let ok = admitido(&v.camino);
            if !ok {
                c.fuera_de_formato += 1;
            }
            ok
        })
        .map(|v| ((v.clave.clone(), v.version.clone()), v))
        .collect();
    let previos: BTreeMap<(String, String), &Item> = antes.iter().map(|i| (i.id(), i)).collect();

    for (id, i) in &previos {
        match (i.estado.as_str(), vivos.contains_key(id)) {
            // Sigue: nada que escribir.
            ("actual", true) => {}
            // Ya no es vigente: se retira, o se pierde si su versión no está.
            ("actual", false) => {
                let perdido = !existen.contains(id);
                if perdido {
                    c.pierden += 1;
                } else {
                    c.retiran += 1;
                }
                c.filas.push(Item {
                    estado: if perdido { "perdido" } else { "retirado" }.into(),
                    retirado: Some(tx),
                    ..(*i).clone()
                });
            }
            // Vuelve a ser vigente (se quitó la marca de borrado): entra otra vez.
            ("retirado" | "perdido", true) => {
                c.entran += 1;
                c.filas.push(Item {
                    estado: "actual".into(),
                    entro: tx,
                    retirado: None,
                    ..(*i).clone()
                });
            }
            // Retirado cuya versión ya no existe: perdido.
            ("retirado", false) if !existen.contains(id) => {
                c.pierden += 1;
                c.filas.push(Item {
                    estado: "perdido".into(),
                    ..(*i).clone()
                });
            }
            _ => {}
        }
    }
    for (id, v) in &vivos {
        if previos.contains_key(id) {
            continue;
        }
        c.entran += 1;
        c.filas.push(Item {
            clave: v.clave.clone(),
            camino: v.camino.clone(),
            version: v.version.clone(),
            etag: v.etag.clone(),
            huella: v.huella.clone(),
            formato: formato_de(&v.camino),
            tamano: v.tamano,
            modificado: v.modificado.clone(),
            estado: "actual".into(),
            entro: tx,
            retirado: None,
            retirado_ms: None,
            blob: String::new(),
            tipo: String::new(),
        });
    }
    c.filas.sort_by_key(Item::id);
    c
}

/// **La fecha de lo que cambia**: lo que sale de la vista actual guarda cuándo
/// (si no lo tenía ya: un retirado que se pierde conserva su fecha), y lo que
/// vuelve la pierde. Aparte de [`transaccion`], que es pura en el tiempo.
pub fn fechar(filas: &mut [Item], ahora_ms: i64) {
    for f in filas {
        if f.estado == "actual" {
            f.retirado_ms = None;
        } else if f.retirado_ms.is_none() {
            f.retirado_ms = Some(ahora_ms);
        }
    }
}

/// Las colecciones mantenidas del árbol: las que tienen `from`.
pub fn mantenidas(pkg: &Package) -> Vec<&Loaded> {
    pkg.docs
        .iter()
        .filter(|d| d.kind == Kind::MediaCollection && d.section("from").is_some())
        .collect()
}

pub fn es_virtual(d: &Loaded) -> bool {
    d.section("virtual").and_then(|v| v.as_str()) == Some("true")
}

/// **De qué fuente sale una colección** (0046 E9·3): `(datasource, tipo de su
/// lector, connectionEnv)`, lo que servir una virtual necesita para firmar en
/// el origen.
pub(crate) fn fuente_de(raiz: &Path, d: &Loaded) -> Result<(String, String, String), String> {
    let (pkg, _) = ore_core::validate::cargar_paquete(raiz);
    let o = objeto_de(&pkg, d)?;
    let ds = o
        .section("datasource")
        .and_then(|v| v.as_str())
        .ok_or("el `ObjectTable` no dice su `datasource`")?
        .to_string();
    let (tipo, env) =
        lector::declaracion(raiz, &ds).map_err(|f| format!("la fuente `{ds}` · {}", f.mensaje))?;
    Ok((ds, tipo, env))
}

/// El `ObjectTable` del que sale una colección.
fn objeto_de<'a>(pkg: &'a Package, d: &Loaded) -> Result<&'a Loaded, String> {
    let r = d
        .section("from")
        .and_then(|f| f.get("objectTable"))
        .and_then(|(_, v)| v.as_str())
        .ok_or("la colección no dice de qué `objectTable` sale")?;
    let q = ore_core::normalize::a_corto(&ore_core::link::cualificar(r, d)).into_owned();
    pkg.docs
        .iter()
        .find(|o| o.kind == Kind::ObjectTable && o.qname().as_deref() == Some(q.as_str()))
        .ok_or_else(|| format!("`{r}` no es un `ObjectTable` del árbol"))
}

/// Las filas del manifiesto vigente, leídas del lago.
pub(crate) fn manifiesto(dataset: &str, ml: &str) -> Result<Vec<Item>, String> {
    let peticion = Json::obj([
        ("dataset", Json::s(dataset)),
        ("metadata_location", Json::s(ml)),
    ])
    .jcs();
    let texto = lector::ejecutar(&programa_del_almacen()?, &["leer".into()], Some(&peticion))
        .map_err(|f| format!("no se pudo leer el manifiesto: {}", f.mensaje))?;
    Ok(texto
        .lines()
        .filter_map(|l| ore_core::parse::parse(l.trim()).ok())
        .filter_map(|n| Item::de_fila(&n))
        .collect())
}

/// El `Content-Type` con el que se guarda y se sirve un ítem, por su formato.
pub fn tipo_de_formato(formato: &str) -> String {
    match formato {
        "pdf" => "application/pdf",
        "jpg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "tiff" => "image/tiff",
        "bmp" => "image/bmp",
        "heic" => "image/heic",
        "svg" => "image/svg+xml",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "txt" => "text/plain",
        "md" => "text/markdown",
        "html" | "htm" => "text/html",
        "rtf" => "application/rtf",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "odt" => "application/vnd.oasis.opendocument.text",
        _ => "application/octet-stream",
    }
    .into()
}

/// Lo que la copia de bytes de una transacción deja: el blob y el tipo de
/// cada ítem que entra, lo que no se pudo copiar, y de dónde salió cada cosa.
#[derive(Debug, Default)]
struct Copia {
    blobs: BTreeMap<(String, String), (String, String)>,
    fallos: Vec<(String, String, String)>,
    del_manifiesto: i64,
    ya_en_el_lago: i64,
    bajados: i64,
    subidos: i64,
    bytes_subidos: i64,
}

fn con_ayuda(f: lector::Fallo) -> String {
    let mut s = f.mensaje;
    for l in f.ayuda {
        s.push('\n');
        s.push_str(&l);
    }
    s
}

/// **El cauce**: `ore-read-<tipo> bajar` → `ore-store blobs`. Devuelve lo que
/// el almacén contestó (una línea por ítem y el resumen) y, si el lector
/// falló, por qué: lo que no llegó no entra.
fn bajar_y_guardar(
    tipo: &str,
    fuente: &str,
    url: &str,
    peticion: &str,
) -> Result<(String, Option<String>), String> {
    // ⭐ 0053 F9·3: con la pasarela, `bajar` va por ella (un turno de copia en
    //   la cola del origen): su flujo, al almacén, igual que el del conector.
    if let Some(p) = lector::pasarela() {
        let cuerpo = format!(
            "{{\"origen\":{},\"tipo\":{},\"url\":{},\"peticion\":{peticion}}}",
            Json::s(fuente).jcs(),
            Json::s(tipo).jcs(),
            Json::s(url).jcs()
        );
        let mut flujo = lector::bajar_por_la_pasarela(&p, &cuerpo)
            .map_err(|m| format!("`bajar` por la pasarela: {}", ore_driver::tapar(&m, url)))?;
        let mut a = lector::lanzar(&programa_del_almacen()?, &["blobs".into()], "{}\n", true)
            .map_err(con_ayuda)?;
        let Some(mut entrada) = a.stdin.take() else {
            a.matar();
            return Err("no se pudo encauzar la pasarela al almacén".into());
        };
        let cauce = std::thread::spawn(move || {
            let r = std::io::copy(&mut flujo, &mut entrada);
            drop(entrada);
            (r, flujo)
        });
        let contestado = a.esperar().map_err(con_ayuda);
        let (copiado, flujo) = cauce
            .join()
            .map_err(|_| "el cauce de la pasarela se cayó".to_string())?;
        let del_lector = match copiado {
            Err(e) => Some(format!("`bajar` por la pasarela: {e}")),
            Ok(_) => flujo
                .fin()
                .err()
                .map(|m| format!("`bajar` por la pasarela: {}", ore_driver::tapar(&m, url))),
        };
        return Ok((
            String::from_utf8_lossy(&contestado?).into_owned(),
            del_lector,
        ));
    }
    let mut de = lector::lanzar(
        &format!("ore-read-{tipo}"),
        &["bajar".into()],
        peticion,
        false,
    )
    .map_err(con_ayuda)?;
    let mut a = lector::lanzar(&programa_del_almacen()?, &["blobs".into()], "{}\n", true)
        .map_err(con_ayuda)?;
    let (Some(mut salida), Some(mut entrada)) = (de.stdout.take(), a.stdin.take()) else {
        de.matar();
        a.matar();
        return Err("no se pudo encauzar el lector al almacén".into());
    };
    let cauce = std::thread::spawn(move || std::io::copy(&mut salida, &mut entrada));
    let contestado = a.esperar().map_err(con_ayuda);
    let _ = cauce.join();
    let del_lector = de.esperar().err().map(con_ayuda);
    Ok((
        String::from_utf8_lossy(&contestado?).into_owned(),
        del_lector,
    ))
}

/// **Los bytes de lo que entra** en una mantenida: del manifiesto, del
/// índice del lago o del origen, por este orden (ver la cabecera).
fn copiar_bytes(
    antes: &[Item],
    filas: &[Item],
    tipo: &str,
    fuente: &str,
    url: &str,
) -> Result<Copia, String> {
    let mut k = Copia::default();
    // Por huella, lo que el manifiesto ya tiene; lo de una fila actual antes
    // que lo de una retirada (esa no la puede recoger nadie).
    let mut conocidos: BTreeMap<(String, i64), (String, String, bool)> = BTreeMap::new();
    for i in antes
        .iter()
        .filter(|i| !i.blob.is_empty() && !i.huella.is_empty())
    {
        let e = conocidos.entry((i.huella.clone(), i.tamano)).or_insert((
            i.blob.clone(),
            i.tipo.clone(),
            false,
        ));
        if i.estado == "actual" {
            *e = (i.blob.clone(), i.tipo.clone(), true);
        }
    }
    // Lo que se reutiliza de una fila que no es actual se toca antes de
    // nombrarlo (la retención puede estar quitando esa fila ahora mismo); si
    // la recogida ya se lo llevó, se baja.
    let a_tocar: BTreeSet<String> = filas
        .iter()
        .filter(|f| f.estado == "actual" && f.blob.is_empty() && !f.huella.is_empty())
        .filter_map(|f| conocidos.get(&(f.huella.clone(), f.tamano)))
        .filter(|(_, _, actual)| !*actual)
        .map(|(b, _, _)| b.clone())
        .collect();
    if !a_tocar.is_empty() {
        let r = almacen(
            "blobs-tocar",
            &Json::obj([("blobs", Json::Arr(a_tocar.iter().map(Json::s).collect()))]).jcs(),
            None,
        )?;
        let faltan: BTreeSet<String> = r
            .get("faltan")
            .map(|(_, v)| v.items())
            .unwrap_or(&[])
            .iter()
            .filter_map(|x| x.as_str().map(String::from))
            .collect();
        conocidos.retain(|_, (b, _, _)| !faltan.contains(b));
    }
    // Lo que falta, junto por contenido: dos ítems con la misma huella y el
    // mismo tamaño se bajan una vez. Sin huella, cada uno es el suyo.
    let mut grupos: BTreeMap<(String, i64), Vec<&Item>> = BTreeMap::new();
    for f in filas
        .iter()
        .filter(|f| f.estado == "actual" && f.blob.is_empty())
    {
        let tipo_f = tipo_de_formato(&f.formato);
        if !f.huella.is_empty()
            && let Some((b, t, _)) = conocidos.get(&(f.huella.clone(), f.tamano))
        {
            let t = if t.is_empty() { tipo_f } else { t.clone() };
            k.blobs.insert(f.id(), (b.clone(), t));
            k.del_manifiesto += 1;
            continue;
        }
        let g = if f.huella.is_empty() {
            (format!("{}\t{}", f.clave, f.version), -1)
        } else {
            (f.huella.clone(), f.tamano)
        };
        grupos.entry(g).or_default().push(f);
    }
    if grupos.is_empty() {
        return Ok(k);
    }
    // ② Lo que el lago ya tiene, por su huella.
    let huellas: Vec<Json> = grupos
        .keys()
        .filter(|(_, t)| *t >= 0)
        .map(|(h, t)| Json::Arr(vec![Json::s(h), Json::s(t.to_string())]))
        .collect();
    if !huellas.is_empty() {
        let r = almacen(
            "blobs-hay",
            &Json::obj([("huellas", Json::Arr(huellas))]).jcs(),
            None,
        )?;
        let hay = r.get("hay").map(|(_, v)| v.items()).unwrap_or(&[]);
        for x in hay {
            let [h, t, sha] = x.items() else { continue };
            let (Some(h), Some(t), Some(sha)) = (
                h.as_str(),
                t.as_str().and_then(|t| t.parse::<i64>().ok()),
                sha.as_str(),
            ) else {
                continue;
            };
            if let Some(g) = grupos.remove(&(h.to_string(), t)) {
                for f in g {
                    k.blobs
                        .insert(f.id(), (sha.to_string(), tipo_de_formato(&f.formato)));
                    k.ya_en_el_lago += 1;
                }
            }
        }
    }
    if grupos.is_empty() {
        return Ok(k);
    }
    // ③ Del origen: el primero de cada grupo, encauzado al almacén.
    let pedidos: Vec<Json> = grupos
        .values()
        .map(|g| {
            let f = g[0];
            Json::obj([
                ("clave", Json::s(&f.clave)),
                ("version", Json::s(&f.version)),
                ("huella", Json::s(&f.huella)),
                ("tamano", Json::Int(f.tamano)),
                ("tipo", Json::s(tipo_de_formato(&f.formato))),
            ])
        })
        .collect();
    let peticion = Json::obj([("url", Json::s(url)), ("items", Json::Arr(pedidos))]).jcs();
    let (contestado, del_lector) = bajar_y_guardar(tipo, fuente, url, &peticion)?;
    let mut por_item: BTreeMap<(String, String), Result<(String, String), String>> =
        BTreeMap::new();
    for l in contestado.lines() {
        let Ok(n) = ore_core::parse::parse(l.trim()) else {
            continue;
        };
        if let Some((_, r)) = n.get("blobs") {
            let e = |c: &str| campo_de(r, c).and_then(|v| v.parse().ok()).unwrap_or(0);
            k.subidos = e("subidos");
            k.bytes_subidos = e("bytes_subidos");
            continue;
        }
        let (Some(c), Some(v)) = (campo_de(&n, "clave"), campo_de(&n, "version")) else {
            continue;
        };
        let r = match (campo_de(&n, "blob"), campo_de(&n, "error")) {
            (Some(b), _) => {
                k.bajados += 1;
                Ok((b, campo_de(&n, "tipo").unwrap_or_default()))
            }
            (None, Some(e)) => Err(e),
            _ => continue,
        };
        por_item.insert((c, v), r);
    }
    let motivo_del_lector = del_lector
        .map(|e| format!("el lector no lo entregó: {e}"))
        .unwrap_or_else(|| "el lector no lo entregó".into());
    for g in grupos.values() {
        let r = por_item
            .get(&g[0].id())
            .cloned()
            .unwrap_or_else(|| Err(motivo_del_lector.clone()));
        for f in g {
            match &r {
                Ok(bt) => {
                    k.blobs.insert(f.id(), bt.clone());
                }
                Err(m) => k
                    .fallos
                    .push((f.clave.clone(), f.version.clone(), m.clone())),
            }
        }
    }
    Ok(k)
}

/// **Una colección, una pasada**: la línea del informe y el puntero nuevo.
#[allow(clippy::too_many_arguments)]
pub fn una(
    pkg: &Package,
    raiz: &Path,
    d: &Loaded,
    qn: &str,
    punteros: &Path,
    seco: bool,
    rehacer: bool,
    bundle: &str,
) -> Result<Option<(String, Json)>, String> {
    let virtual_ = es_virtual(d);
    let o = objeto_de(pkg, d)?;
    let datasource = o
        .section("datasource")
        .and_then(|v| v.as_str())
        .ok_or("el `ObjectTable` no dice su `datasource`")?
        .to_string();
    let prefijo = o
        .section("prefix")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let patrones: Vec<String> = [
        o.section("match").and_then(|v| v.as_str()),
        d.section("from")
            .and_then(|f| f.get("match"))
            .and_then(|(_, v)| v.as_str()),
    ]
    .into_iter()
    .flatten()
    .map(String::from)
    .collect();
    let formatos: Vec<String> = d
        .section("formats")
        .map(|f| {
            f.items()
                .iter()
                .filter_map(|x| x.as_str().map(|s| s.to_ascii_lowercase()))
                .collect()
        })
        .unwrap_or_default();

    let puntero = leer_puntero(punteros, qn);
    let ml = puntero
        .as_ref()
        .and_then(|p| campo_de(p, "metadata_location"));
    let dataset = puntero
        .as_ref()
        .and_then(|p| campo_de(p, "dataset"))
        .unwrap_or_else(|| {
            ore_core::punteros::dataset_nuevo(qn).replacen("catalogo/", "colecciones/", 1)
        });
    let antes: Vec<Item> = match &ml {
        Some(m) if !rehacer => manifiesto(&dataset, m)?,
        _ => Vec::new(),
    };
    let tx = puntero
        .as_ref()
        .and_then(|p| campo_de(p, "transaccion"))
        .and_then(|t| t.parse::<i64>().ok())
        .filter(|_| ml.is_some() && !rehacer)
        .map(|t| t + 1)
        .unwrap_or(1);

    // El listado de versiones, del lector de la fuente.
    let (tipo, env) = lector::declaracion(raiz, &datasource)
        .map_err(|f| format!("la fuente `{datasource}` · {}", f.mensaje))?;
    let url = lector::url(raiz, &env, &datasource)
        .map_err(|f| format!("la fuente `{datasource}` · {}", f.mensaje))?;
    // 0053 F9·3: por la pasarela si la hay (`lector::preguntar`, `versions`).
    let peticion = Json::obj([
        ("objeto", Json::s(&prefijo)),
        (
            "patrones",
            Json::Arr(patrones.iter().map(Json::s).collect()),
        ),
        (
            "conocidos",
            Json::Arr(
                antes
                    .iter()
                    .map(|i| {
                        Json::Arr(vec![
                            Json::s(&i.clave),
                            Json::s(&i.version),
                            Json::s(&i.huella),
                        ])
                    })
                    .collect(),
            ),
        ),
    ]);
    let salida = lector::preguntar(
        &tipo,
        &datasource,
        "versions",
        &url,
        vec![("peticion", peticion)],
    )
    .map_err(|f| format!("`ore-read-{tipo} versiones`: {}", f.mensaje))?;
    let r = ore_core::parse::parse(salida.trim())
        .map_err(|e| format!("lo que devolvió `ore-read-{tipo} versiones` no analiza: {e:?}"))?;
    let testigo = campo_de(&r, "testigo").unwrap_or_default();
    let huellas = campo_de(&r, "huellas").unwrap_or_else(|| "0".into());

    // ⭐ 0049 B8·3: una virtual que pasa a mantenida tiene ítems actuales sin
    //   blob —los de cuando era virtual—. Mantenida quiere decir sus bytes en
    //   el lago: mientras quede alguno, la pasada los copia aunque el listado
    //   no haya cambiado.
    let sin_copiar = |i: &Item| i.estado == "actual" && i.blob.is_empty();
    let por_copiar = !virtual_ && !seco && antes.iter().any(sin_copiar);

    // ④ El listado no cambió (y no queda nada por copiar): nada que leer ni escribir.
    let testigo_antes = puntero
        .as_ref()
        .and_then(|p| p.get("testigo"))
        .and_then(|(_, t)| campo_de(t, "valor"));
    if !rehacer && !por_copiar && ml.is_some() && testigo_antes.as_deref() == Some(testigo.as_str())
    {
        let mut m = match puntero.as_ref().map(Json::de_node) {
            Some(Json::Obj(m)) => m,
            _ => Default::default(),
        };
        m.insert("estado".into(), Json::s("al-dia"));
        m.insert("bundle".into(), Json::s(bundle));
        return Ok(Some((
            format!("al día · el listado no cambió (transacción {})", tx - 1),
            Json::Obj(m),
        )));
    }

    let vigentes: Vec<Vigente> = r
        .get("items")
        .map(|(_, v)| v.items())
        .unwrap_or(&[])
        .iter()
        .filter_map(|i| {
            Some(Vigente {
                clave: campo_de(i, "clave")?,
                camino: campo_de(i, "camino").unwrap_or_default(),
                version: campo_de(i, "version")?,
                etag: campo_de(i, "etag").unwrap_or_default(),
                tamano: campo_de(i, "tamano")
                    .and_then(|t| t.parse().ok())
                    .unwrap_or(0),
                modificado: campo_de(i, "modificado").unwrap_or_default(),
                huella: campo_de(i, "huella").unwrap_or_default(),
            })
        })
        .collect();
    let existen: BTreeSet<(String, String)> = r
        .get("existen")
        .map(|(_, v)| v.items())
        .unwrap_or(&[])
        .iter()
        .filter_map(|x| match x.items() {
            [c, v] => Some((c.as_str()?.to_string(), v.as_str()?.to_string())),
            _ => None,
        })
        .collect();
    // En la mantenida nada se pierde: lo retirado sigue en el lago aunque el
    // origen ya no tenga su versión.
    let existen: BTreeSet<(String, String)> = if virtual_ {
        existen
    } else {
        antes.iter().map(Item::id).collect()
    };
    let mut c = transaccion(&antes, &vigentes, &existen, &formatos, tx);
    let mut copia = Copia::default();
    // B8·3: lo que ya estaba sin sus bytes (ver `por_copiar`). No entra: se
    // reescribe su fila con su blob (el sello funde por la clave).
    let mut de_paso: BTreeSet<(String, String)> = BTreeSet::new();
    if !virtual_ && !seco {
        let ya: BTreeSet<(String, String)> = c.filas.iter().map(Item::id).collect();
        for i in antes
            .iter()
            .filter(|i| sin_copiar(i) && !ya.contains(&i.id()))
        {
            de_paso.insert(i.id());
            c.filas.push(i.clone());
        }
        copia = copiar_bytes(&antes, &c.filas, &tipo, &datasource, &url)?;
        let mut fuera = 0;
        c.filas.retain_mut(|f| {
            if f.estado != "actual" || !f.blob.is_empty() {
                return true;
            }
            match copia.blobs.get(&f.id()) {
                Some((b, t)) => {
                    f.blob = b.clone();
                    f.tipo = t.clone();
                    true
                }
                None => {
                    // Lo de paso que no se pudo copiar se queda como estaba
                    // (se sirve de su origen) y la pasada siguiente lo reintenta.
                    if !de_paso.contains(&f.id()) {
                        fuera += 1;
                    }
                    false
                }
            }
        });
        c.entran -= fuera;
    }
    let copiados_de_paso = c.filas.iter().filter(|f| de_paso.contains(&f.id())).count();
    fechar(&mut c.filas, ahora_ms());
    let cuenta = |estado: &str| -> i64 {
        let mut vivos: BTreeMap<(String, String), &str> =
            antes.iter().map(|i| (i.id(), i.estado.as_str())).collect();
        for f in &c.filas {
            vivos.insert(f.id(), f.estado.as_str());
        }
        vivos.values().filter(|e| **e == estado).count() as i64
    };
    let (actuales, retirados, perdidos) = (cuenta("actual"), cuenta("retirado"), cuenta("perdido"));
    // B8·3: los actuales que siguen sin sus bytes en el lago tras esta
    // transacción. Va en el puntero: mientras no sea 0, `ore-serve` sirve
    // esos ítems de su origen.
    let quedan_por_copiar = if virtual_ {
        0
    } else {
        let mut vivos: BTreeMap<(String, String), bool> =
            antes.iter().map(|i| (i.id(), sin_copiar(i))).collect();
        for f in &c.filas {
            vivos.insert(f.id(), sin_copiar(f));
        }
        vivos.values().filter(|s| **s).count() as i64
    };
    let mut linea = format!(
        "transacción {tx} · {} entran, {} se retiran, {} se pierden · {actuales} actuales, \
         {retirados} retirados, {perdidos} perdidos · {huellas} huellas pedidas{}",
        c.entran,
        c.retiran,
        c.pierden,
        if c.fuera_de_formato > 0 {
            format!(" · {} fuera de sus formatos", c.fuera_de_formato)
        } else {
            String::new()
        }
    );
    if !virtual_ && !seco {
        linea.push_str(&format!(
            " · bytes: {} bajados ({} KB subidos), {} ya en el lago, {} por su huella en el manifiesto",
            copia.bajados,
            copia.bytes_subidos / 1024,
            copia.ya_en_el_lago,
            copia.del_manifiesto
        ));
        if copiados_de_paso > 0 {
            linea.push_str(&format!(
                " · {copiados_de_paso} que ya estaban, con sus bytes al lago (pasa a mantenida)"
            ));
        }
        if quedan_por_copiar > 0 {
            linea.push_str(&format!(
                " · {quedan_por_copiar} siguen sin sus bytes en el lago: se sirven de su origen"
            ));
        }
        if let Some((cl, v, m)) = copia.fallos.first() {
            linea.push_str(&format!(
                " · {} sin copiar, no entran (el primero, `{cl}` versión {v}: {m})",
                copia.fallos.len()
            ));
        }
    }
    if let Some(t) = techo(actuales + retirados + perdidos) {
        linea.push_str(&format!(
            " · ⚠ {}",
            campo_de_json(&t, "aviso").unwrap_or_default()
        ));
    }
    // Lo que no se copió se vuelve a intentar: el testigo no avanza.
    let testigo_guardado = if copia.fallos.is_empty() {
        testigo.clone()
    } else {
        testigo_antes.clone().unwrap_or_default()
    };
    if seco {
        return Ok(Some((format!("haría la {linea}"), Json::obj([]))));
    }

    let esquema: BTreeMap<String, ore_core::types::Type> = COLUMNAS
        .iter()
        .chain(if virtual_ {
            &[][..]
        } else {
            COLUMNAS_MANTENIDA
        })
        .filter_map(|(c, t)| Some((c.to_string(), ore_core::types::parse_type(t).ok()?)))
        .collect();
    let plan = ore_core::digest::de_bytes(format!("coleccion:{qn}").as_bytes());
    let t = ("listing".to_string(), Some(testigo_guardado.clone()));
    let clave = vec!["clave".to_string(), "version".to_string()];
    // Sobre el manifiesto que hay, siempre (rehacer es sobrescribir: un
    // snapshot nuevo, y la historia se queda); fundir, si había filas.
    let base = ml.as_deref();
    let fundir = base.is_some() && !rehacer && !antes.is_empty();
    let mut extra = format!("{{\"dataset\":\"{dataset}\",\"fundir\":{fundir},");
    if let Some(b) = base {
        extra.push_str(&format!("\"base\":\"{b}\","));
    }
    let peticion =
        cabecera(&plan, &esquema, &t, &clave, &Default::default()).replacen('{', &extra, 1);
    let filas: String = c.filas.iter().map(|f| f.fila() + "\n").collect();
    let s = if c.filas.is_empty() && fundir {
        // El listado cambió y la colección no (lo nuevo está fuera de sus
        // formatos): el manifiesto se queda; sólo el testigo se mueve.
        None
    } else {
        Some(almacen("sellar", &peticion, Some(&filas))?)
    };
    let mut m: BTreeMap<String, Json> = match puntero.as_ref().map(Json::de_node) {
        Some(Json::Obj(m)) => m,
        _ => Default::default(),
    };
    m.remove("motivo");
    m.remove("no_copiados");
    m.insert("kind".into(), Json::s("MediaCollection"));
    m.insert("virtual".into(), Json::Bool(virtual_));
    if !virtual_ {
        let mut vivos: BTreeMap<(String, String), &str> =
            antes.iter().map(|i| (i.id(), i.blob.as_str())).collect();
        for f in &c.filas {
            vivos.insert(f.id(), f.blob.as_str());
        }
        let distintos: BTreeSet<&str> = vivos.values().copied().filter(|b| !b.is_empty()).collect();
        m.insert(
            "blobs".into(),
            Json::obj([
                ("distintos", Json::Int(distintos.len() as i64)),
                ("bajados", Json::Int(copia.bajados)),
                ("subidos", Json::Int(copia.subidos)),
                ("bytes_subidos", Json::Int(copia.bytes_subidos)),
                ("ya_en_el_lago", Json::Int(copia.ya_en_el_lago)),
                ("del_manifiesto", Json::Int(copia.del_manifiesto)),
            ]),
        );
        if !copia.fallos.is_empty() {
            m.insert(
                "no_copiados".into(),
                Json::Arr(
                    copia
                        .fallos
                        .iter()
                        .take(20)
                        .map(|(cl, v, mo)| {
                            Json::obj([
                                ("clave", Json::s(cl)),
                                ("version", Json::s(v)),
                                ("motivo", Json::s(mo)),
                            ])
                        })
                        .collect(),
                ),
            );
        }
    }
    m.insert("estado".into(), Json::s("transaccion"));
    m.insert("transaccion".into(), Json::Int(tx));
    // B8·3: lo que a esta transacción (de una mantenida) le falta en el lago.
    m.insert("por_copiar".into(), Json::Int(quedan_por_copiar));
    m.insert("bundle".into(), Json::s(bundle));
    m.insert(
        "testigo".into(),
        Json::obj([
            ("modo", Json::s("listing")),
            ("valor", Json::s(&testigo_guardado)),
        ]),
    );
    m.insert(
        "items".into(),
        Json::obj([
            ("actuales", Json::Int(actuales)),
            ("retirados", Json::Int(retirados)),
            ("perdidos", Json::Int(perdidos)),
        ]),
    );
    con_techo(&mut m, actuales + retirados + perdidos);
    m.insert(
        "cambios".into(),
        Json::obj([
            ("entran", Json::Int(c.entran as i64)),
            ("retiran", Json::Int(c.retiran as i64)),
            ("pierden", Json::Int(c.pierden as i64)),
            ("fuera_de_formato", Json::Int(c.fuera_de_formato as i64)),
        ]),
    );
    if let Some(s) = s {
        for k in ["metadata_location", "snapshot", "bytes", "ficheros"] {
            if let Some(v) = campo_de(&s, k) {
                m.insert(
                    k.into(),
                    v.parse::<i64>()
                        .ok()
                        .filter(|_| k != "snapshot")
                        .map(Json::Int)
                        .unwrap_or(Json::s(v)),
                );
            }
        }
    }
    Ok(Some((linea, Json::Obj(m))))
}

/// Lo que la retención dejó de una colección: su línea, su puntero si se
/// movió, y las filas que quedan (las que nombran blobs vivos).
pub struct Caducado {
    pub linea: String,
    pub puntero: Option<Json>,
    pub filas: Vec<Item>,
}

/// **La retención de una colección** (E8·3a): del manifiesto, lo retirado o
/// perdido más viejo que su `retention` se va; lo retirado sin fecha (de antes
/// de E8·3) la gana hoy. Si algo cambia, un snapshot nuevo con lo que queda
/// (la historia se queda en los anteriores hasta que caduquen) y el puntero se
/// mueve. Sin `retention`, o sin documento, no caduca nada.
pub fn caducar(
    d: Option<&Loaded>,
    qn: &str,
    puntero: &ore_core::parse::Node,
    ahora_ms: i64,
    seco: bool,
) -> Result<Caducado, String> {
    let (Some(ml), Some(dataset)) = (
        campo_de(puntero, "metadata_location"),
        campo_de(puntero, "dataset"),
    ) else {
        return Ok(Caducado {
            linea: "sin transacción todavía".into(),
            puntero: None,
            filas: Vec::new(),
        });
    };
    let filas = manifiesto(&dataset, &ml)?;
    let declarada = d
        .and_then(|d| d.section("retention"))
        .and_then(|v| v.as_str())
        .map(String::from);
    let retencion = match declarada.as_deref() {
        Some(r) => Some(
            crate::datasets::edad_ms(r)
                .map_err(|e| format!("la `retention` de `{qn}` no se entiende: {e}"))?,
        ),
        None => None,
    };
    let mut sin_fecha = 0usize;
    let mut caducados = 0usize;
    let mut quedan = Vec::with_capacity(filas.len());
    for mut f in filas {
        if f.estado != "actual" {
            match (f.retirado_ms, retencion) {
                (None, _) => {
                    f.retirado_ms = Some(ahora_ms);
                    sin_fecha += 1;
                }
                (Some(t), Some(r)) if t.saturating_add(r) <= ahora_ms => {
                    caducados += 1;
                    continue;
                }
                _ => {}
            }
        }
        quedan.push(f);
    }
    let ret = declarada.map_or("sin `retention`: nada caduca".to_string(), |r| {
        format!("retención {r}")
    });
    let mut linea = format!(
        "{ret} · {} filas, {caducados} caducadas{}",
        quedan.len() + caducados,
        if sin_fecha > 0 {
            format!(", {sin_fecha} retiradas sin fecha la ganan hoy")
        } else {
            String::new()
        }
    );
    if (caducados == 0 && sin_fecha == 0) || seco {
        if seco && (caducados > 0 || sin_fecha > 0) {
            linea = format!("en seco · {linea}");
        }
        return Ok(Caducado {
            linea,
            puntero: None,
            filas: quedan,
        });
    }
    if quedan.is_empty() {
        // Una tabla sin filas no se sella: la colección se queda con lo que
        // tenía hasta que entre algo.
        return Ok(Caducado {
            linea: format!("{linea} · se deja: no quedaría ninguna fila"),
            puntero: None,
            filas: quedan,
        });
    }
    let virtual_ = campo_de(puntero, "virtual").as_deref() != Some("false");
    let esquema: BTreeMap<String, ore_core::types::Type> = COLUMNAS
        .iter()
        .chain(if virtual_ {
            &[][..]
        } else {
            COLUMNAS_MANTENIDA
        })
        .filter_map(|(c, t)| Some((c.to_string(), ore_core::types::parse_type(t).ok()?)))
        .collect();
    let plan = ore_core::digest::de_bytes(format!("coleccion:{qn}").as_bytes());
    let testigo = puntero
        .get("testigo")
        .and_then(|(_, t)| campo_de(t, "valor"))
        .unwrap_or_default();
    let t = ("listing".to_string(), Some(testigo));
    let clave = vec!["clave".to_string(), "version".to_string()];
    let extra = format!("{{\"dataset\":\"{dataset}\",\"fundir\":false,\"base\":\"{ml}\",");
    let peticion =
        cabecera(&plan, &esquema, &t, &clave, &Default::default()).replacen('{', &extra, 1);
    let texto: String = quedan.iter().map(|f| f.fila() + "\n").collect();
    let s = almacen("sellar", &peticion, Some(&texto))?;
    let mut m: BTreeMap<String, Json> = match Json::de_node(puntero) {
        Json::Obj(m) => m,
        _ => Default::default(),
    };
    for k in ["metadata_location", "snapshot"] {
        if let Some(v) = campo_de(&s, k) {
            m.insert(k.into(), Json::s(v));
        }
    }
    let n = |e: &str| quedan.iter().filter(|f| f.estado == e).count() as i64;
    m.insert(
        "items".into(),
        Json::obj([
            ("actuales", Json::Int(n("actual"))),
            ("retirados", Json::Int(n("retirado"))),
            ("perdidos", Json::Int(n("perdido"))),
        ]),
    );
    con_techo(&mut m, quedan.len() as i64);
    m.insert(
        "retencion".into(),
        Json::obj([
            ("caducados", Json::Int(caducados as i64)),
            ("ms", Json::Int(ahora_ms)),
        ]),
    );
    Ok(Caducado {
        linea,
        puntero: Some(Json::Obj(m)),
        filas: quedan,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vig(clave: &str, version: &str, huella: &str) -> Vigente {
        Vigente {
            clave: clave.into(),
            camino: clave.into(),
            version: version.into(),
            etag: String::new(),
            tamano: 10,
            modificado: String::new(),
            huella: huella.into(),
        }
    }

    fn fijar(antes: &[Item], c: &Cambios) -> Vec<Item> {
        let mut m: BTreeMap<(String, String), Item> =
            antes.iter().map(|i| (i.id(), i.clone())).collect();
        for f in &c.filas {
            m.insert(f.id(), f.clone());
        }
        m.into_values().collect()
    }

    fn estado<'a>(items: &'a [Item], clave: &str, version: &str) -> &'a str {
        items
            .iter()
            .find(|i| i.clave == clave && i.version == version)
            .map(|i| i.estado.as_str())
            .unwrap_or("-")
    }

    /// **El experimento de E7, en dos transacciones.** Antes: `a.pdf` (a1),
    /// `b.pdf` y `c.jpg`. Después: `a.pdf` sobrescrito (a2), `b.pdf` borrado
    /// (su versión sigue), `c.jpg` renombrado a `c2.jpg` (su versión sigue),
    /// `d.pdf` nuevo con el contenido de otro; y un `.txt` que no es de la
    /// colección.
    #[test]
    fn el_experimento_de_e7_en_dos_transacciones() {
        let formatos = vec!["pdf".to_string(), "jpg".to_string()];
        let t1 = transaccion(
            &[],
            &[
                vig("a.pdf", "a1", "F"),
                vig("b.pdf", "b1", "R"),
                vig("c.jpg", "c1", "P"),
            ],
            &BTreeSet::new(),
            &formatos,
            1,
        );
        assert_eq!((t1.entran, t1.retiran, t1.pierden), (3, 0, 0));
        let antes = fijar(&[], &t1);

        let existen: BTreeSet<(String, String)> =
            [("a.pdf", "a1"), ("b.pdf", "b1"), ("c.jpg", "c1")]
                .iter()
                .map(|(c, v)| (c.to_string(), v.to_string()))
                .collect();
        let t2 = transaccion(
            &antes,
            &[
                vig("a.pdf", "a2", "R"),
                vig("c2.jpg", "c2", "P"),
                vig("d.pdf", "d1", "R"),
                vig("nota.txt", "t1", "T"),
            ],
            &existen,
            &formatos,
            2,
        );
        assert_eq!((t2.entran, t2.retiran, t2.pierden), (3, 3, 0));
        assert_eq!(t2.fuera_de_formato, 1);
        let despues = fijar(&antes, &t2);
        assert_eq!(
            estado(&despues, "a.pdf", "a1"),
            "retirado",
            "sobrescrito: la vieja se retira"
        );
        assert_eq!(estado(&despues, "a.pdf", "a2"), "actual");
        assert_eq!(
            estado(&despues, "b.pdf", "b1"),
            "retirado",
            "borrado: su versión sigue"
        );
        assert_eq!(estado(&despues, "c.jpg", "c1"), "retirado");
        assert_eq!(estado(&despues, "c2.jpg", "c2"), "actual");
        assert_eq!(estado(&despues, "d.pdf", "d1"), "actual");
        let a1 = despues.iter().find(|i| i.version == "a1").unwrap();
        assert_eq!((a1.entro, a1.retirado), (1, Some(2)));
        // El contenido repetido se ve por la huella: tres ítems, un contenido.
        let r: Vec<&str> = despues
            .iter()
            .filter(|i| i.huella == "R" && i.estado == "actual")
            .map(|i| i.clave.as_str())
            .collect();
        assert_eq!(r, ["a.pdf", "d.pdf"]);

        // Sin cambios: ninguna fila.
        let vivos: Vec<Vigente> = despues
            .iter()
            .filter(|i| i.estado == "actual")
            .map(|i| vig(&i.clave, &i.version, &i.huella))
            .collect();
        let t3 = transaccion(&despues, &vivos, &existen, &formatos, 3);
        assert!(t3.filas.is_empty(), "{:?}", t3.filas);
    }

    /// **Perdido**: lo que deja de ser vigente y cuya versión ya no existe (un
    /// origen sin versionado, o una versión purgada por el ciclo de vida del
    /// cliente). Y un retirado que se pierde después, también.
    #[test]
    fn lo_que_ya_no_existe_se_pierde_y_se_dice() {
        let t1 = transaccion(
            &[],
            &[vig("x.pdf", "null", "H"), vig("y.pdf", "null", "I")],
            &BTreeSet::new(),
            &[],
            1,
        );
        let antes = fijar(&[], &t1);
        let solo_y: BTreeSet<(String, String)> = [("y.pdf".to_string(), "null".to_string())].into();
        let t2 = transaccion(&antes, &[], &solo_y, &[], 2);
        assert_eq!((t2.retiran, t2.pierden), (1, 1));
        let d = fijar(&antes, &t2);
        assert_eq!(estado(&d, "x.pdf", "null"), "perdido");
        assert_eq!(estado(&d, "y.pdf", "null"), "retirado");
        let t3 = transaccion(&d, &[], &BTreeSet::new(), &[], 3);
        assert_eq!(t3.pierden, 1, "y.pdf ya no existe como versión");
        assert_eq!(estado(&fijar(&d, &t3), "y.pdf", "null"), "perdido");
    }

    /// Lo que sale de la vista actual gana su fecha; lo perdido que ya la
    /// tenía la conserva; lo que vuelve la pierde.
    #[test]
    fn lo_que_sale_guarda_cuando() {
        let t1 = transaccion(
            &[],
            &[vig("a.pdf", "a1", "A"), vig("b.pdf", "b1", "B")],
            &BTreeSet::new(),
            &[],
            1,
        );
        let mut antes = fijar(&[], &t1);
        fechar(&mut antes, 100);
        assert!(antes.iter().all(|i| i.retirado_ms.is_none()));
        let existen: BTreeSet<(String, String)> = [("a.pdf".to_string(), "a1".to_string())].into();
        let mut t2 = transaccion(&antes, &[vig("b.pdf", "b1", "B")], &existen, &[], 2);
        fechar(&mut t2.filas, 200);
        let d = fijar(&antes, &t2);
        let a = d.iter().find(|i| i.clave == "a.pdf").unwrap();
        assert_eq!((a.estado.as_str(), a.retirado_ms), ("retirado", Some(200)));
        let mut t3 = transaccion(&d, &[vig("b.pdf", "b1", "B")], &BTreeSet::new(), &[], 3);
        fechar(&mut t3.filas, 300);
        let d = fijar(&d, &t3);
        let a = d.iter().find(|i| i.clave == "a.pdf").unwrap();
        assert_eq!(
            (a.estado.as_str(), a.retirado_ms),
            ("perdido", Some(200)),
            "se pierde después, y conserva cuándo salió"
        );
        let mut t4 = transaccion(
            &d,
            &[vig("a.pdf", "a1", "A"), vig("b.pdf", "b1", "B")],
            &BTreeSet::new(),
            &[],
            4,
        );
        fechar(&mut t4.filas, 400);
        let a = t4.filas.iter().find(|i| i.clave == "a.pdf").unwrap();
        assert_eq!((a.estado.as_str(), a.retirado_ms), ("actual", None));
    }

    /// El techo: hasta 500.000 filas no se dice nada; pasado, el puntero
    /// lo lleva; y si la retención baja de nuevo, se quita.
    #[test]
    fn el_techo_se_dice_y_se_quita() {
        assert!(techo(TECHO).is_none());
        let t = techo(TECHO + 1).expect("pasado el techo");
        assert!(t.jcs().contains("merge-on-read"), "{}", t.jcs());
        let mut m = BTreeMap::new();
        con_techo(&mut m, 900_000);
        assert!(m.contains_key("techo"));
        con_techo(&mut m, 10);
        assert!(!m.contains_key("techo"));
    }

    #[test]
    fn el_formato_por_la_extension() {
        assert_eq!(formato_de("Nueva carpeta/Foto Portada 2026.JPG"), "jpg");
        assert_eq!(formato_de("x/y.jpeg"), "jpg");
        assert_eq!(formato_de("sin-extension"), "");
    }
}
