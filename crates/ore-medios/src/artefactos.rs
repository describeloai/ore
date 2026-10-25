//! **Los artefactos de un registro** (ADR 0074 AR1·1): sus bytes y su
//! inmutabilidad, en la puerta de bytes de la celda.
//!
//! `ore-serve` decide —quién publica, en qué registro, si la ruta de Maven
//! cuadra— y pasa aquí el flujo; esto **guarda y sirve**, con la cuenta de
//! `ore-medios`, que en el bucket sólo crea (`objectCreator`: ni sobrescribe
//! ni borra). Sólo `ore-serve` llega a este puerto.
//!
//! ```text
//! PUT  /artefactos/<b>/<s>/<registro>/<formato>/<librería>/<versión>/<fichero>   el fichero, en flujo
//! GET  /artefactos/<b>/<s>/<registro>/<formato>/<librería>/<versión>/<fichero>   sus bytes
//! POST /artefactos/listar      {registro, formato, libreria}   versiones y ficheros, con sus sumas
//! POST /artefactos/coordenada  {formato, libreria}             qué registro la reclamó
//! POST /artefactos/librerias   {registro, formato}             las librerías de un registro (AR1·3)
//! ```
//!
//! En el bucket del inquilino, con prefijo propio (0074 §5.2):
//!
//! ```text
//! ore/artefactos/<b>/<s>/<r>/<formato>/<librería>/<versión>/<fichero>.json   el fichero: sumas, tamaño, quién, cuándo
//! ore/artefactos/coordenadas/<formato>/<librería>.json                       → {registro}: el primero que la publicó
//! ore/artefactos/sha256/<hex>                                                los bytes
//! (la librería, en una clave, con `~` donde la coordenada dice `:`: ver [`en_clave`])
//! ```
//!
//! ⭐ **La unidad es el fichero** (medido en AR0 P1 y P2): ni Maven ni PyPI
//!   dicen «terminé» una versión. Los bytes van primero, por su suma; después
//!   el `.json` del fichero, con «sube si no estaba»: si ya estaba, 409 y no se
//!   toca. La coordenada se reclama igual, en el primer fichero de la librería.
//! ⛔ **Prefijo propio, no `ore/v2/blobs/`**: la recogida de blobs (0046 E8·3b)
//!   borra lo que ningún manifiesto nombra, y un jar no lo nombra ninguno.

use crate::escritura::{borrar_temporal, temporal};
use crate::servicio::{Servicio, problema};
use ore_base::json::Json;
use ore_base::parse::Node;
use ore_entrada::http::{Bytes, Peticion, Respuesta};
use ore_store::almacen::{Blob, Cuerpo};
use ore_store::blobs::{Crc32c, EN_MEMORIA};
use sha2::Digest;
use std::io::{Read, Write};
use std::path::Path;

/// Los formatos que esta puerta guarda (v1alpha34 `01` §3).
pub const FORMATOS: &[&str] = &["maven", "pypi"];

const PREFIJO: &str = "ore/artefactos";

/// Si la petición es una subida de un artefacto: lo único que este puerto lee
/// en flujo.
pub fn es_subida(p: &Peticion) -> bool {
    p.metodo == "PUT" && p.ruta.starts_with("/artefactos/")
}

/// Dónde vive un fichero: el registro (`b.s.r`), el formato, la librería
/// (su coordenada, normalizada por quien llama), la versión y el nombre.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sitio {
    pub base: String,
    pub schema: String,
    pub registro: String,
    pub formato: String,
    pub libreria: String,
    pub version: String,
    pub fichero: String,
}

impl Sitio {
    pub fn de_ruta(ruta: &str) -> Result<Sitio, String> {
        let partes: Vec<&str> = ruta
            .strip_prefix("/artefactos/")
            .ok_or("not an artifact route")?
            .split('/')
            .collect();
        let [b, s, r, f, l, v, fichero] = partes.as_slice() else {
            return Err(
                "an artifact is `/artefactos/<base>/<schema>/<registry>/<format>/<library>/<version>/<file>`"
                    .into(),
            );
        };
        for (que, x) in [("base", b), ("schema", s), ("registry", r)] {
            if !identificador(x) {
                return Err(format!("`{x}` is not a {que} name"));
            }
        }
        if !FORMATOS.contains(f) {
            return Err(format!("`{f}` is not a format of a registry (maven, pypi)"));
        }
        if !segmento(l, true) {
            return Err(format!("`{l}` is not a library"));
        }
        for (que, x) in [("version", v), ("file", fichero)] {
            if !segmento(x, false) {
                return Err(format!("`{x}` is not a {que}"));
            }
        }
        Ok(Sitio {
            base: b.to_string(),
            schema: s.to_string(),
            registro: r.to_string(),
            formato: f.to_string(),
            libreria: l.to_string(),
            version: v.to_string(),
            fichero: fichero.to_string(),
        })
    }

    pub fn registro_completo(&self) -> String {
        format!("{}.{}.{}", self.base, self.schema, self.registro)
    }

    fn clave_del_fichero(&self) -> String {
        format!(
            "{PREFIJO}/{}/{}/{}/{}/{}/{}/{}.json",
            self.base,
            self.schema,
            self.registro,
            self.formato,
            en_clave(&self.libreria),
            self.version,
            self.fichero
        )
    }
}

/// `b`, `s` y `r`: lo que v1alpha34 admite como nombre.
fn identificador(x: &str) -> bool {
    !x.is_empty()
        && x.len() <= 100
        && x.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// Una librería, una versión o un fichero: el alfabeto de una coordenada de
/// Maven y de un nombre de PyPI, sin `/`, sin empezar por `.` (ni `..`).
/// `:` sólo en la librería (`grupo:artefacto`).
fn segmento(x: &str, con_dos_puntos: bool) -> bool {
    !x.is_empty()
        && x.len() <= 200
        && !x.starts_with('.')
        && x.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(c, '.' | '_' | '-' | '+')
                || (con_dos_puntos && c == ':')
        })
}

/// La librería como va en una clave del bucket: `:` como `~`. Medido en AR1
/// contra un S3 local: con `:` en la clave, la firma SigV4 no casa (403), y
/// `~` no está en el alfabeto de una coordenada (es reversible) ni lo escapa
/// ninguna URI.
fn en_clave(libreria: &str) -> String {
    libreria.replace(':', "~")
}

fn clave_de_la_coordenada(formato: &str, libreria: &str) -> String {
    format!(
        "{PREFIJO}/coordenadas/{formato}/{}.json",
        en_clave(libreria)
    )
}

fn clave_del_blob(sha256: &str) -> String {
    format!("{PREFIJO}/sha256/{sha256}")
}

/// Lo que llegó, con sus cuatro sumas: `sha256` (la identidad), `sha1` y `md5`
/// (las que Maven coteja) y `crc32c` (la que el almacén coteja al subir).
struct Recibido {
    sha256: [u8; 32],
    sha1: [u8; 20],
    md5: [u8; 16],
    crc32c: u32,
    tamano: u64,
    cuerpo: Cuerpo,
}

/// **Al paso**: lo pequeño en memoria, lo grande a un temporal; la memoria no
/// crece con el fichero.
fn recibir(cuerpo: &mut dyn Read, largo: u64, dir: &Path) -> Result<Recibido, String> {
    let mut sha256 = sha2::Sha256::new();
    let mut sha1 = sha1::Sha1::new();
    let mut md5 = md5::Md5::new();
    let mut crc = Crc32c::default();
    let mut tamano = 0u64;
    let mut memoria = Vec::new();
    let mut fichero = None;
    if largo >= EN_MEMORIA {
        let ruta = dir.join(format!("ore-artefacto-{}", uuid::Uuid::new_v4().simple()));
        let f = std::fs::File::create(&ruta)
            .map_err(|e| format!("could not create `{}`: {e}", ruta.display()))?;
        fichero = Some((ruta, std::io::BufWriter::with_capacity(1 << 20, f)));
    } else {
        memoria.reserve(largo as usize);
    }
    let mut buf = vec![0u8; 1 << 16];
    let leido = loop {
        let n = match cuerpo.read(&mut buf) {
            Ok(0) => break Ok(()),
            Ok(n) => n,
            Err(e) => break Err(format!("the upload was cut short: {e}")),
        };
        let trozo = &buf[..n];
        sha256.update(trozo);
        sha1.update(trozo);
        md5.update(trozo);
        crc.sumar(trozo);
        tamano += n as u64;
        let escrito = match &mut fichero {
            Some((_, f)) => f.write_all(trozo).map_err(|e| e.to_string()),
            None => {
                memoria.extend_from_slice(trozo);
                Ok(())
            }
        };
        if let Err(e) = escrito {
            break Err(format!("could not write the temporary file: {e}"));
        }
    };
    let cuerpo = match fichero {
        Some((ruta, mut f)) => {
            let hecho = leido.and_then(|_| f.flush().map_err(|e| e.to_string()));
            drop(f);
            if let Err(e) = hecho {
                let _ = std::fs::remove_file(&ruta);
                return Err(e);
            }
            Cuerpo::Fichero(ruta)
        }
        None => {
            leido?;
            Cuerpo::Memoria(memoria)
        }
    };
    if tamano != largo {
        borrar_temporal(&cuerpo);
        return Err(format!("received {tamano} bytes of the {largo} announced"));
    }
    Ok(Recibido {
        sha256: sha256.finalize().into(),
        sha1: sha1.finalize().into(),
        md5: md5.finalize().into(),
        crc32c: crc.valor(),
        tamano,
        cuerpo,
    })
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn ahora_iso() -> String {
    let s = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (dias, resto) = (s.div_euclid(86_400), s.rem_euclid(86_400));
    // Días desde 1970 → fecha civil (Howard Hinnant, `civil_from_days`).
    let z = dias + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        resto / 3600,
        (resto % 3600) / 60,
        resto % 60
    )
}

fn texto<'a>(n: &'a Node, k: &str) -> Option<&'a str> {
    n.get(k).and_then(|(_, v)| v.as_str())
}

impl Servicio {
    /// **`PUT /artefactos/…`**: un fichero, en flujo. 201 con sus sumas; 409 si
    /// ya estaba o si la coordenada es de otro registro.
    pub fn subir_artefacto(&self, p: &Peticion, cuerpo: &mut dyn Read, largo: u64) -> Respuesta {
        let sitio = match Sitio::de_ruta(&p.ruta) {
            Ok(s) => s,
            Err(m) => return problema(400, "artifact/path", m),
        };
        let quien = p.cabeceras.get("x-ore-quien").cloned().unwrap_or_default();
        let registro = sitio.registro_completo();
        // La coordenada, primero: quien no la tiene no deja bytes.
        if let Err(r) = self.reclamar(&sitio.formato, &sitio.libreria, &registro) {
            return r;
        }
        let clave = sitio.clave_del_fichero();
        match self.cuenta.existe(&clave) {
            Ok(true) => return ya_estaba(&sitio),
            Ok(false) => {}
            Err(e) => return problema(502, "artifact/store", e),
        }
        let r = match recibir(cuerpo, largo, &temporal()) {
            Ok(r) => r,
            Err(e) => return problema(400, "artifact/upload", e),
        };
        let sha256 = hex(&r.sha256);
        let blob = Blob {
            clave: clave_del_blob(&sha256),
            tipo: "application/octet-stream".into(),
            tamano: r.tamano,
            sha256: r.sha256,
            crc32c: r.crc32c,
            cuerpo: r.cuerpo,
        };
        let subido = self.cuenta.poner_blob(&blob);
        borrar_temporal(&blob.cuerpo);
        if let Err(e) = subido {
            return problema(
                502,
                "artifact/store",
                format!("the lake did not store it: {e}"),
            );
        }
        let ficha = Json::obj([
            ("sha256", Json::s(&sha256)),
            ("sha1", Json::s(hex(&r.sha1))),
            ("md5", Json::s(hex(&r.md5))),
            ("tamano", Json::Int(r.tamano as i64)),
            ("quien", Json::s(&quien)),
            ("cuando", Json::s(ahora_iso())),
        ]);
        // ⭐ «Sube si no estaba» es la inmutabilidad: dos subidas a la vez del
        //   mismo fichero, una gana y la otra recibe 409.
        match self.cuenta.subir(&clave, ficha.jcs().as_bytes()) {
            Ok(true) => Respuesta {
                codigo: 201,
                cuerpo: ficha,
            },
            Ok(false) => ya_estaba(&sitio),
            Err(e) => problema(502, "artifact/store", e),
        }
    }

    /// La coordenada es de `registro`, o lo pasa a ser si nadie la tenía.
    fn reclamar(&self, formato: &str, libreria: &str, registro: &str) -> Result<(), Respuesta> {
        let clave = clave_de_la_coordenada(formato, libreria);
        let ficha = Json::obj([("registro", Json::s(registro))]).jcs();
        match self.cuenta.subir(&clave, ficha.as_bytes()) {
            Ok(true) => Ok(()),
            Ok(false) => match self.duena(formato, libreria) {
                Ok(Some(r)) if r == registro => Ok(()),
                Ok(Some(otro)) => Err(problema(
                    409,
                    "artifact/coordinate-taken",
                    format!(
                        "`{libreria}` ({formato}) is published in the registry `{otro}`: a library lives in one registry of the cell"
                    ),
                )),
                Ok(None) => Err(problema(
                    502,
                    "artifact/store",
                    format!("the claim of `{libreria}` is unreadable"),
                )),
                Err(e) => Err(problema(502, "artifact/store", e)),
            },
            Err(e) => Err(problema(502, "artifact/store", e)),
        }
    }

    /// El registro (`b.s.r`) que reclamó una coordenada, si alguno.
    pub(crate) fn duena(&self, formato: &str, libreria: &str) -> Result<Option<String>, String> {
        let Some(t) = self
            .cuenta
            .leer(&clave_de_la_coordenada(formato, libreria))?
        else {
            return Ok(None);
        };
        let n = ore_base::parse::parse(&t).map_err(|_| "the claim is not JSON".to_string())?;
        Ok(texto(&n, "registro").map(str::to_string))
    }

    /// **`GET /artefactos/…`**: los bytes de un fichero.
    pub fn bajar_artefacto(&self, p: &Peticion) -> Result<Bytes, Respuesta> {
        let sitio = Sitio::de_ruta(&p.ruta).map_err(|m| problema(400, "artifact/path", m))?;
        let ficha = self
            .cuenta
            .leer(&sitio.clave_del_fichero())
            .map_err(|e| problema(502, "artifact/store", e))?
            .ok_or_else(|| no_esta(&sitio))?;
        let n = ore_base::parse::parse(&ficha)
            .map_err(|_| problema(502, "artifact/store", "the file record is not JSON"))?;
        let sha256 = texto(&n, "sha256").unwrap_or_default().to_string();
        // ⚠️ Entero en memoria: un jar de librería son kilobytes o pocos MB, y
        //   el túnel corta en 100 MiB (AR1·0). Servirlo en flujo desde el lago
        //   es deuda (como `contenido::del_lago`).
        let bytes = self
            .cuenta
            .leer_bytes(&clave_del_blob(&sha256))
            .map_err(|e| problema(502, "artifact/store", e))?
            .ok_or_else(|| {
                problema(
                    502,
                    "artifact/store",
                    format!(
                        "the bytes of `{}` are missing (sha256:{sha256})",
                        sitio.fichero
                    ),
                )
            })?;
        Ok(Bytes {
            codigo: 200,
            cabeceras: vec![
                ("content-type".into(), "application/octet-stream".into()),
                ("x-ore-sha256".into(), sha256),
            ],
            largo: Some(bytes.len() as u64),
            lector: Box::new(std::io::Cursor::new(bytes)),
            finales: None,
        })
    }

    /// **`POST /artefactos/listar {registro, formato, libreria}`**: las
    /// versiones de una librería de un registro, cada una con sus ficheros.
    pub fn listar_artefacto(&self, n: &Node) -> Respuesta {
        let (Some(registro), Some(formato), Some(libreria)) = (
            texto(n, "registro"),
            texto(n, "formato"),
            texto(n, "libreria"),
        ) else {
            return problema(
                400,
                "artifact/request",
                "`registro`, `formato` and `libreria`",
            );
        };
        let partes: Vec<&str> = registro.split('.').collect();
        let [b, s, r] = partes.as_slice() else {
            return problema(
                400,
                "artifact/request",
                "`registro` is `<base>.<schema>.<name>`",
            );
        };
        if !(identificador(b) && identificador(s) && identificador(r))
            || !FORMATOS.contains(&formato)
            || !segmento(libreria, true)
        {
            return problema(400, "artifact/request", "not a registry, format or library");
        }
        let prefijo = format!("{PREFIJO}/{b}/{s}/{r}/{formato}/{}/", en_clave(libreria));
        let claves = match self.cuenta.listar(&prefijo) {
            Ok(c) => c,
            Err(e) => return problema(502, "artifact/store", e),
        };
        let mut versiones: std::collections::BTreeMap<String, Vec<Json>> = Default::default();
        for clave in claves {
            let Some(resto) = clave.strip_prefix(&prefijo) else {
                continue;
            };
            let Some((version, fichero)) =
                resto.strip_suffix(".json").and_then(|r| r.split_once('/'))
            else {
                continue;
            };
            let Ok(Some(t)) = self.cuenta.leer(&clave) else {
                continue;
            };
            let Ok(ficha) = ore_base::parse::parse(&t) else {
                continue;
            };
            let campo = |k: &str| Json::s(texto(&ficha, k).unwrap_or_default());
            let tamano = ficha
                .get("tamano")
                .and_then(|(_, v)| v.as_str().and_then(|t| t.parse::<i64>().ok()))
                .unwrap_or(0);
            versiones
                .entry(version.to_string())
                .or_default()
                .push(Json::obj([
                    ("nombre", Json::s(fichero)),
                    ("sha256", campo("sha256")),
                    ("sha1", campo("sha1")),
                    ("md5", campo("md5")),
                    ("tamano", Json::Int(tamano)),
                    ("quien", campo("quien")),
                    ("cuando", campo("cuando")),
                ]));
        }
        Respuesta::ok(Json::obj([(
            "versiones",
            Json::Arr(
                versiones
                    .into_iter()
                    .map(|(v, f)| Json::obj([("version", Json::s(v)), ("ficheros", Json::Arr(f))]))
                    .collect(),
            ),
        )]))
    }

    /// **`POST /artefactos/librerias {registro, formato}`** (AR1·3): las
    /// librerías que un registro tiene de un formato. Lo pregunta `ore-serve`
    /// antes de quitarle un formato a un registro o de retirarlo.
    pub fn librerias_artefacto(&self, n: &Node) -> Respuesta {
        let (Some(registro), Some(formato)) = (texto(n, "registro"), texto(n, "formato")) else {
            return problema(400, "artifact/request", "`registro` and `formato`");
        };
        let partes: Vec<&str> = registro.split('.').collect();
        let [b, s, r] = partes.as_slice() else {
            return problema(
                400,
                "artifact/request",
                "`registro` is `<base>.<schema>.<name>`",
            );
        };
        if !(identificador(b) && identificador(s) && identificador(r))
            || !FORMATOS.contains(&formato)
        {
            return problema(400, "artifact/request", "not a registry or format");
        }
        let prefijo = format!("{PREFIJO}/{b}/{s}/{r}/{formato}/");
        let claves = match self.cuenta.listar(&prefijo) {
            Ok(c) => c,
            Err(e) => return problema(502, "artifact/store", e),
        };
        let mut librerias: Vec<String> = claves
            .iter()
            .filter_map(|c| c.strip_prefix(&prefijo)?.split('/').next())
            .map(|l| l.replace('~', ":"))
            .collect();
        librerias.sort();
        librerias.dedup();
        Respuesta::ok(Json::obj([(
            "librerias",
            Json::Arr(librerias.into_iter().map(Json::s).collect()),
        )]))
    }

    /// **`POST /artefactos/coordenada {formato, libreria}`**: qué registro la
    /// tiene. 404 si ninguno.
    pub fn coordenada_artefacto(&self, n: &Node) -> Respuesta {
        let (Some(formato), Some(libreria)) = (texto(n, "formato"), texto(n, "libreria")) else {
            return problema(400, "artifact/request", "`formato` and `libreria`");
        };
        if !FORMATOS.contains(&formato) || !segmento(libreria, true) {
            return problema(400, "artifact/request", "not a format or library");
        }
        match self.duena(formato, libreria) {
            Ok(Some(r)) => Respuesta::ok(Json::obj([("registro", Json::s(r))])),
            Ok(None) => problema(
                404,
                "artifact/not-found",
                format!("no registry of this cell has `{libreria}` ({formato})"),
            ),
            Err(e) => problema(502, "artifact/store", e),
        }
    }
}

fn ya_estaba(s: &Sitio) -> Respuesta {
    problema(
        409,
        "artifact/immutable",
        format!(
            "`{}` {} is already published in `{}`: a published file is never replaced; publish a new version",
            s.fichero,
            s.version,
            s.registro_completo()
        ),
    )
}

fn no_esta(s: &Sitio) -> Respuesta {
    problema(
        404,
        "artifact/not-found",
        format!(
            "`{}` {} is not in `{}`",
            s.fichero,
            s.version,
            s.registro_completo()
        ),
    )
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::escritura::pruebas::Memoria;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    fn servicio() -> (Servicio, Arc<Memoria>) {
        let m = Arc::new(Memoria::default());
        (
            Servicio {
                listados: Box::new(ore_store::lago::Lago::nuevo(m.clone())),
                cuenta: m.clone(),
                indices: crate::indice::Indices::nuevo(1_000),
                vistos: Default::default(),
                permisos: Default::default(),
                escrituras: Default::default(),
            },
            m,
        )
    }

    fn peticion(metodo: &str, ruta: &str, cuerpo: &str) -> Peticion {
        let mut cabeceras = BTreeMap::new();
        cabeceras.insert("x-ore-quien".to_string(), "persona:ana".to_string());
        Peticion {
            metodo: metodo.into(),
            ruta: ruta.into(),
            cabeceras,
            cuerpo: cuerpo.into(),
            consulta: BTreeMap::new(),
            consulta_cruda: String::new(),
        }
    }

    const JAR: &str =
        "/artefactos/banco/riesgo/libs/maven/com.banco:motor-riesgo/1.0.0/motor-riesgo-1.0.0.jar";

    fn subir(s: &Servicio, ruta: &str, bytes: &[u8]) -> Respuesta {
        s.subir_artefacto(
            &peticion("PUT", ruta, ""),
            &mut &bytes[..],
            bytes.len() as u64,
        )
    }

    fn json(r: &Respuesta) -> Node {
        ore_base::parse::parse(&r.cuerpo.jcs()).unwrap()
    }

    #[test]
    fn un_fichero_se_guarda_con_sus_sumas_y_se_baja_igual() {
        let (s, m) = servicio();
        let r = subir(&s, JAR, b"hola");
        assert_eq!(r.codigo, 201, "{}", r.cuerpo.jcs());
        let n = json(&r);
        // Las de `printf hola | sha256sum`, `sha1sum` y `md5sum`.
        assert_eq!(
            texto(&n, "sha256"),
            Some("b221d9dbb083a7f33428d7c2a3c3198ae925614d70210e28716ccaa7cd4ddb79")
        );
        assert_eq!(
            texto(&n, "sha1"),
            Some("99800b85d3383e3a2fb45eb7d0066a4879a9dad0")
        );
        assert_eq!(texto(&n, "md5"), Some("4d186321c1a7f0f354b297e8914ab240"));
        assert!(
            m.0.lock()
                .unwrap()
                .keys()
                .any(|k| k.starts_with("ore/artefactos/sha256/"))
        );
        let mut b = s.bajar_artefacto(&peticion("GET", JAR, "")).ok().unwrap();
        let mut leido = Vec::new();
        b.lector.read_to_end(&mut leido).unwrap();
        assert_eq!(leido, b"hola");
    }

    #[test]
    fn un_fichero_publicado_no_se_reemplaza() {
        let (s, _) = servicio();
        assert_eq!(subir(&s, JAR, b"hola").codigo, 201);
        let r = subir(&s, JAR, b"otra cosa");
        assert_eq!(r.codigo, 409, "{}", r.cuerpo.jcs());
        assert!(r.cuerpo.jcs().contains("artifact/immutable"));
        // Y lo de dentro sigue siendo lo primero.
        let mut b = s.bajar_artefacto(&peticion("GET", JAR, "")).ok().unwrap();
        let mut leido = Vec::new();
        b.lector.read_to_end(&mut leido).unwrap();
        assert_eq!(leido, b"hola");
    }

    #[test]
    fn una_coordenada_es_del_primer_registro_que_la_publica() {
        let (s, _) = servicio();
        assert_eq!(subir(&s, JAR, b"hola").codigo, 201);
        // Otra versión en el mismo registro: vale.
        let v2 = JAR.replace("1.0.0", "1.1.0");
        assert_eq!(subir(&s, &v2, b"hola2").codigo, 201);
        // La misma coordenada en otro registro: no.
        let otro = JAR.replace("/libs/", "/otras/");
        let r = subir(&s, &otro, b"hola");
        assert_eq!(r.codigo, 409, "{}", r.cuerpo.jcs());
        assert!(r.cuerpo.jcs().contains("banco.riesgo.libs"));
        let c = s.coordenada_artefacto(
            &ore_base::parse::parse(r#"{"formato":"maven","libreria":"com.banco:motor-riesgo"}"#)
                .unwrap(),
        );
        assert_eq!(texto(&json(&c), "registro"), Some("banco.riesgo.libs"));
    }

    #[test]
    fn el_listado_da_las_versiones_con_sus_ficheros() {
        let (s, _) = servicio();
        subir(&s, JAR, b"hola");
        subir(&s, &JAR.replace(".jar", ".pom"), b"<project/>");
        subir(&s, &JAR.replace("1.0.0", "1.1.0"), b"hola2");
        let r = s.listar_artefacto(
            &ore_base::parse::parse(
                r#"{"registro":"banco.riesgo.libs","formato":"maven","libreria":"com.banco:motor-riesgo"}"#,
            )
            .unwrap(),
        );
        assert_eq!(r.codigo, 200);
        let t = r.cuerpo.jcs();
        assert!(
            t.contains(r#""version":"1.0.0""#) && t.contains(r#""version":"1.1.0""#),
            "{t}"
        );
        assert!(t.contains("motor-riesgo-1.0.0.pom"), "{t}");
        assert!(t.contains(r#""tamano":4"#), "{t}");
        let l = s.librerias_artefacto(
            &ore_base::parse::parse(r#"{"registro":"banco.riesgo.libs","formato":"maven"}"#)
                .unwrap(),
        );
        assert_eq!(
            l.cuerpo.jcs(),
            r#"{"librerias":["com.banco:motor-riesgo"]}"#
        );
        let vacio = s.librerias_artefacto(
            &ore_base::parse::parse(r#"{"registro":"banco.riesgo.libs","formato":"pypi"}"#)
                .unwrap(),
        );
        assert_eq!(vacio.cuerpo.jcs(), r#"{"librerias":[]}"#);
    }

    #[test]
    fn una_ruta_rara_no_entra() {
        let (s, _) = servicio();
        for ruta in [
            "/artefactos/banco/riesgo/libs/maven/com.banco:x/1.0.0/../x.jar",
            "/artefactos/banco/riesgo/libs/npm/x/1.0.0/x.tgz",
            "/artefactos/Banco/riesgo/libs/maven/x/1.0.0/x.jar",
            "/artefactos/banco/riesgo/libs/maven/x/1.0.0",
            "/artefactos/banco/riesgo/libs/maven/x/1.0.0/a:b.jar",
        ] {
            assert_eq!(subir(&s, ruta, b"x").codigo, 400, "{ruta}");
        }
    }

    #[test]
    fn lo_que_llega_corto_no_se_guarda() {
        let (s, m) = servicio();
        let r = s.subir_artefacto(&peticion("PUT", JAR, ""), &mut &b"hol"[..], 4);
        assert_eq!(r.codigo, 400);
        assert!(!m.0.lock().unwrap().keys().any(|k| k.ends_with(".jar.json")));
    }
}
