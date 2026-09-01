//! `ore-registry` — el registro de referencia, y es **un árbol de ficheros**.
//!
//! Normativo: [`spec/v1alpha6/04-registro.md`].
//!
//! ```text
//! <raíz>/
//!   index/oos.dev/regulatory/gdpr.json
//!   blobs/sha256/<64 hex>
//! ```
//!
//! Sin base de datos y sin nada que ejecutar: **cualquier servidor de ficheros
//! estáticos es un registro conforme**, y un espejo completo es un `rsync`.
//!
//! # Por qué esto llega el último y casi no decide nada
//!
//! Un registro de paquetes suele ser lo primero que se construye, y acaba siendo
//! la pieza de la que todo depende: quien lo opera decide qué existe, qué es cada
//! cosa y quién publica. Aquí llega al final, y para entonces ya se le ha quitado
//! todo eso — el digest le quitó la integridad, la firma la procedencia y el log
//! la historia. Lo que le queda es servir ficheros.
//!
//! **Y esa es la propiedad que se quiere**, no una carencia: un registro del que
//! se puede prescindir es un registro que no ata a nadie.
//!
//! # Dos órdenes
//!
//! | | Qué hace |
//! |---|---|
//! | `publish <raíz> <fichero.oob>` | escribe el blob y la entrada del índice |
//! | `verify <raíz>` | recomprueba el registro entero **sin confiar en nadie** |
//!
//! La segunda es la interesante. Un registro no dice la verdad porque lo diga
//! quien lo sirve: la dice porque cualquiera con una copia acaba de recomprobarla.

use ore_core::json::Json;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let r = match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["publish", raiz, oob] => publicar(Path::new(raiz), Path::new(oob)),
        ["verify", raiz] => verificar(Path::new(raiz)),
        _ => Err("uso: `ore-registry publish <raiz> <fichero.oob>` o \
                  `ore-registry verify <raiz>`"
            .into()),
    };
    match r {
        Ok(informe) => {
            print!("{informe}");
            ExitCode::SUCCESS
        }
        Err(m) => {
            eprintln!("ore-registry: {m}");
            ExitCode::FAILURE
        }
    }
}

// ── Publicar ────────────────────────────────────────────────────────────────

/// Publicar es **escribir dos ficheros**: el blob y la entrada del índice.
///
/// El blob se nombra por el SHA-256 de sus BYTES, y no por el digest del
/// paquete. La distinción no es cosmética: el digest identifica el paquete y no
/// cambia al firmarlo —esa es la propiedad que decidió el formato—, así que dos
/// `.oob` del mismo paquete, uno firmado y otro no, tendrían el mismo nombre y
/// bytes distintos. El almacén direcciona **bytes**; el índice, **significado**.
fn publicar(raiz: &Path, oob: &Path) -> Result<String, String> {
    let bytes = std::fs::read_to_string(oob)
        .map_err(|e| format!("no se pudo leer `{}`: {e}", oob.display()))?;
    let sobre = analizar(&bytes, &oob.display().to_string())?;

    let blob = hash(&bytes);
    let destino = raiz.join("blobs/sha256").join(&blob);
    escribir(&destino, &bytes)?;

    let idx = raiz.join("index").join(format!("{}.json", sobre.paquete));
    let mut versiones = leer_indice(&idx)?;
    // Una versión ya publicada NO se reescribe con otro digest. Corregir una
    // versión es publicar otra: quien la tenga vendorizada ya la verificó, y
    // cambiarla debajo sería mentirle sin que su lock se entere.
    if let Some(v) = versiones.iter().find(|v| v.version == sobre.version)
        && v.digest != sobre.digest
    {
        return Err(format!(
            "`{} {}` ya esta publicada con otro digest.\n  \
                 index: {}\n  nueva: {}\n  \
                 Lo que sustituye a corregir una version es publicar otra: quien la tenga \
                 vendorizada ya la verifico, y cambiarla debajo seria mentirle sin que su \
                 lock se entere",
            sobre.paquete, sobre.version, v.digest, sobre.digest
        ));
    }
    // Mismo digest y otros bytes es legal —firmar no cambia la identidad— así
    // que la entrada apunta al blob nuevo y el viejo se queda: nadie borra un
    // blob que alguien pueda tener anotado.
    versiones.retain(|v| v.version != sobre.version);
    versiones.push(Version {
        version: sobre.version.clone(),
        digest: sobre.digest.clone(),
        blob: blob.clone(),
        size: bytes.len() as u64,
    });
    versiones.sort_by_key(|v| orden(&v.version));
    escribir(&idx, &indice(&sobre.paquete, &versiones))?;

    Ok(format!(
        "  ✓ {} {}\n  · blobs/sha256/{blob}\n  · {}\n  · {}\n\n\
         \x20 El blob se nombra por el hash de sus BYTES y el indice guarda el digest del\n\
         \x20 PAQUETE: son dos preguntas distintas, y firmar cambia la primera sin tocar\n\
         \x20 la segunda.\n",
        sobre.paquete,
        sobre.version,
        relativa(raiz, &idx),
        sobre.digest,
    ))
}

// ── Verificar ───────────────────────────────────────────────────────────────

/// Recomprueba el registro entero, **sin hablar con nadie**.
///
/// Las tres comprobaciones son aritmética sobre ficheros, así que las hace igual
/// quien opera el registro y quien acaba de replicarlo. Es lo que hace que un
/// registro no tenga que ser de confianza: no dice la verdad porque lo diga —la
/// dice porque cualquiera acaba de recomprobarla.
///
/// Y lo que **no** puede afirmar: que un paquete sea de quien dice —eso es la
/// firma— ni que su historia sea única —eso es el log—. Un registro conforme
/// sirve paquetes sin firmar, y quien los consuma se entera por su lock.
fn verificar(raiz: &Path) -> Result<String, String> {
    let mut fallos = Vec::new();
    let mut blobs = 0usize;

    // 1 · el nombre de cada blob es el hash de sus bytes.
    let dir = raiz.join("blobs/sha256");
    for p in listar(&dir) {
        blobs += 1;
        let Ok(bytes) = std::fs::read_to_string(&p) else {
            fallos.push(format!("  no se pudo leer `{}`", relativa(raiz, &p)));
            continue;
        };
        let nombre = p.file_name().unwrap_or_default().to_string_lossy();
        let real = hash(&bytes);
        if nombre != real {
            fallos.push(format!(
                "  `blobs/sha256/{nombre}` contiene bytes que digieren `{real}`"
            ));
        }
    }

    // 2 y 3 · cada version del indice apunta a un blob que existe, y ese blob es
    // el paquete, la version y el digest que el indice declara.
    let mut versiones = 0usize;
    for idx in listar_recursivo(&raiz.join("index")) {
        let Ok(texto) = std::fs::read_to_string(&idx) else {
            continue;
        };
        let Ok(j) = ore_core::parse::parse(&texto) else {
            fallos.push(format!("  `{}` no analiza", relativa(raiz, &idx)));
            continue;
        };
        let paquete = j
            .get("package")
            .and_then(|(_, v)| v.as_str())
            .unwrap_or_default()
            .to_string();
        for v in j.get("versions").map(|(_, x)| x.items()).unwrap_or(&[]) {
            versiones += 1;
            let leer = |k: &str| v.get(k).and_then(|(_, x)| x.as_str()).unwrap_or_default();
            let (version, digest, blob) = (leer("version"), leer("digest"), leer("blob"));
            let donde = format!("{paquete} {version}");
            let ruta = raiz.join("blobs/sha256").join(blob);
            let Ok(bytes) = std::fs::read_to_string(&ruta) else {
                fallos.push(format!("  `{donde}` apunta a un blob que no esta: {blob}"));
                continue;
            };
            match analizar(&bytes, &donde) {
                Err(e) => fallos.push(format!("  {e}")),
                Ok(s) if s.paquete != paquete || s.version != version => fallos.push(format!(
                    "  `{donde}` sirve un blob que dice ser `{} {}`",
                    s.paquete, s.version
                )),
                Ok(s) if s.digest != digest => fallos.push(format!(
                    "  `{donde}`: el indice dice `{digest}` y el blob digiere `{}`",
                    s.digest
                )),
                Ok(_) => {}
            }
        }
    }

    if !fallos.is_empty() {
        return Err(format!(
            "el registro no se sostiene:\n{}\n\n  \
             Las tres comprobaciones son aritmetica sobre ficheros, asi que este fallo lo ve \
             igual quien opera el registro y quien acaba de replicarlo.",
            fallos.join("\n")
        ));
    }
    Ok(format!(
        "  ✓ {} · {blobs} blob(s) · {versiones} version(es)\n\n\
         \x20 Comprobado: cada blob digiere su nombre, cada version apunta a un blob que\n\
         \x20 esta, y cada blob es el paquete, la version y el digest que el indice dice.\n\
         \x20 Nada de esto se cree: se recomputa, y lo mismo puede hacer quien replique.\n",
        raiz.display()
    ))
}

// ── El sobre y el índice ────────────────────────────────────────────────────

struct Sobre {
    paquete: String,
    version: String,
    digest: String,
}

/// Lo que un `.oob` dice ser, y **el digest recomputado de lo que trae dentro**.
///
/// El digest no se lee: se computa de los documentos, con el mismo
/// `digest::package` del bundle. Un `.oob` no lleva el suyo escrito precisamente
/// por esto — un número que un lector no debe creerse acaba creído.
fn analizar(bytes: &str, donde: &str) -> Result<Sobre, String> {
    let j = ore_core::parse::parse(bytes).map_err(|e| format!("`{donde}` no analiza: {e:?}"))?;
    let campo = |k: &str| {
        j.get(k)
            .and_then(|(_, v)| v.as_str())
            .map(String::from)
            .ok_or_else(|| format!("`{donde}` no dice `{k}`"))
    };
    // Se carga como paquete para digerirlo, que es lo mismo que hace quien lo
    // consume. Con dos formas de computarlo, el registro y el consumidor podrian
    // discrepar sobre qué es el mismo fichero, y el registro estaria afirmando
    // algo que el compilador no confirma.
    //
    // El sitio de paso es UNO POR LLAMADA y no por contenido. Con el contenido
    // como nombre, dos procesos que digirieran el mismo blob usaban el mismo
    // directorio y el primero en terminar lo borraba mientras el otro leia: el
    // segundo cargaba la nada y digeria `e3b0c442…`, el SHA de la cadena vacia,
    // que es un digest perfectamente valido de un paquete que no existe. Solo
    // pasaba a veces, que es la peor clase de fallo.
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "ore-registry-{}-{}",
        std::process::id(),
        N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).map_err(|e| format!("no se pudo preparar la lectura: {e}"))?;
    let tmp = dir.join("p.oob");
    std::fs::write(&tmp, bytes).map_err(|e| format!("no se pudo preparar la lectura: {e}"))?;
    let (pkg, _) = ore_core::validate::cargar_paquete(&tmp);
    let digest = ore_core::digest::package(&ore_core::link::publicables(&pkg));
    let _ = std::fs::remove_dir_all(&dir);

    // Y un digest de la nada no se devuelve. Si algo salio mal al leer, decirlo
    // es mejor que publicar un indice que afirma el paquete vacio.
    if pkg.docs.is_empty() {
        return Err(format!(
            "`{donde}` no trajo ningun documento al cargarse. Un indice con el digest del              paquete vacio afirmaria algo que nadie puede usar"
        ));
    }

    Ok(Sobre {
        paquete: campo("package")?,
        version: campo("version")?,
        digest,
    })
}

struct Version {
    version: String,
    digest: String,
    blob: String,
    size: u64,
}

fn leer_indice(p: &Path) -> Result<Vec<Version>, String> {
    let Ok(texto) = std::fs::read_to_string(p) else {
        return Ok(Vec::new());
    };
    let j = ore_core::parse::parse(&texto)
        .map_err(|e| format!("`{}` no analiza: {e:?}", p.display()))?;
    Ok(j.get("versions")
        .map(|(_, x)| x.items())
        .unwrap_or(&[])
        .iter()
        .filter_map(|v| {
            let leer = |k: &str| v.get(k).and_then(|(_, x)| x.as_str()).map(String::from);
            Some(Version {
                version: leer("version")?,
                digest: leer("digest")?,
                blob: leer("blob")?,
                size: leer("size")?.parse().unwrap_or(0),
            })
        })
        .collect())
}

/// El índice, en JCS: dos publicaciones del mismo estado dan los mismos bytes,
/// igual que un `.oob`. Un espejo que difiera en el formato del índice no es un
/// espejo.
fn indice(paquete: &str, versiones: &[Version]) -> String {
    Json::obj([
        ("package", Json::s(paquete)),
        (
            "versions",
            Json::Arr(
                versiones
                    .iter()
                    .map(|v| {
                        Json::obj([
                            ("blob", Json::s(&v.blob)),
                            ("digest", Json::s(&v.digest)),
                            ("size", Json::Int(v.size as i64)),
                            ("version", Json::s(&v.version)),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
    .jcs()
}

// ── Ficheros ────────────────────────────────────────────────────────────────

fn hash(bytes: &str) -> String {
    let mut h = Sha256::new();
    h.update(bytes.as_bytes());
    ore_core::firma::a_hex(&h.finalize())
}

fn escribir(p: &Path, texto: &str) -> Result<(), String> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)
            .map_err(|e| format!("no se pudo crear `{}`: {e}", d.display()))?;
    }
    std::fs::write(p, texto).map_err(|e| format!("no se pudo escribir `{}`: {e}", p.display()))
}

fn listar(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|e| {
            e.flatten()
                .map(|x| x.path())
                .filter(|p| p.is_file())
                .collect()
        })
        .unwrap_or_default()
}

fn listar_recursivo(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut pila = vec![dir.to_path_buf()];
    while let Some(d) = pila.pop() {
        for p in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = p.path();
            if p.is_dir() {
                pila.push(p);
            } else if p.extension().is_some_and(|x| x == "json") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

fn relativa(raiz: &Path, p: &Path) -> String {
    p.strip_prefix(raiz)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Orden por número y no por texto: `0.10.0` va después de `0.9.0`.
fn orden(v: &str) -> Vec<u64> {
    v.split(['-', '+'])
        .next()
        .unwrap_or(v)
        .split('.')
        .map(|x| x.parse().unwrap_or(0))
        .collect()
}
