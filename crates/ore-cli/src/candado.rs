//! `ore lock` — el resolutor que se podía escribir sin registro.
//!
//! # Lo que faltaba, y de qué mitad se ocupa esto
//!
//! `ontology.lock` existe, `dependencies` existe, `OOS2013` comprueba que estén
//! sincronizados y **nada podía escribirlo**: la ayuda de ese diagnóstico decía
//! literalmente *«el resolutor que escribe el lock todavía no existe: hoy la
//! entrada se añade a mano»*. La cabecera del lock del ejemplo de referencia
//! anuncia desde el principio `ore lock` y `ore lock --check`, dos comandos que
//! no existían.
//!
//! El alcance que `v1alpha2/00-scope` §4 le pone a la resolución completa son
//! tres cosas —**un protocolo de registro, el formato del paquete publicable y
//! el resolutor**— y dos de las tres no son un comando. Pero hay una mitad que
//! no necesita ninguna: **resolver contra lo que ya está en el árbol.**
//!
//! Es el caso que se usa hoy y el único que se puede usar hoy. Un vocabulario se
//! consume copiándolo como un miembro más del workspace, y hasta ahora eso no se
//! podía **declarar**: `dependencies` quedaba escrito sin que nada lo
//! comprobara, o directamente sin escribir, y el árbol no registraba de dónde
//! venía su clasificación.
//!
//! # Por qué una coordenada casa con un paquete del árbol
//!
//! Porque el nombre de un paquete **es** la coordenada con la que otro lo
//! importa. Estaba en la descripción de `packageName` desde el principio —*«los
//! paquetes publicados llevan espacio de nombres, p. ej.
//! `oos.dev/regulatory/gdpr`»*— y el patrón del esquema la rechazaba, así que
//! había dos ideas de qué es un nombre de paquete y ninguna implementada.
//! Arreglado eso, casar `{package: oos.dev/regulatory/gdpr}` con el paquete que
//! se llama así **no es una convención de este motor**: es leer el nombre.
//!
//! # Y lo que este resolutor NO hace, que es la otra mitad
//!
//! **No trae nada.** Si la coordenada no está en el árbol, falla diciéndolo — no
//! la busca, no la descarga y no inventa una entrada. `ore` no sabe hablar por la
//! red y esa es una propiedad comprobada, no una promesa: el día que exista un
//! registro, traer un paquete se delegará como se delega leer una fuente.
//!
//! **No firma nada.** El `digest` que escribe es el de los documentos que hay en
//! el árbol, computado con el mismo `digest::package` que usa el bundle. No es el
//! digest de un `.oob` publicado, porque ese formato no existe todavía — y decir
//! que lo es sería exactamente la clase de afirmación que este proyecto desmonta.

use ore_core::document::Kind;
use ore_core::impacto::Cambio;
use ore_core::link::Package;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const CONFIG: &str = "ontology.config.yaml";
const LOCK: &str = "ontology.lock";

#[derive(Debug)]
pub struct Fallo {
    pub codigo: u8,
    pub mensaje: String,
    pub ayuda: Vec<String>,
}

fn fallo(codigo: u8, mensaje: impl Into<String>, ayuda: &[&str]) -> Fallo {
    Fallo {
        codigo,
        mensaje: mensaje.into(),
        ayuda: ayuda.iter().map(|s| (*s).to_string()).collect(),
    }
}

pub fn lock(raiz: &Path, comprobar: bool) -> ExitCode {
    match intentar(raiz, comprobar) {
        Ok(informe) => {
            print!("{informe}");
            ExitCode::SUCCESS
        }
        Err(f) => {
            eprintln!("error: {}", f.mensaje);
            for l in &f.ayuda {
                eprintln!("{l}");
            }
            ExitCode::from(f.codigo)
        }
    }
}

fn intentar(raiz: &Path, comprobar: bool) -> Result<String, Fallo> {
    if !raiz.join(CONFIG).is_file() {
        return Err(fallo(
            66, // EX_NOINPUT
            format!("no hay `{CONFIG}` en `{}`", raiz.display()),
            &["  Un lock es del workspace, y el manifiesto es lo que lo delimita."],
        ));
    }
    let (pkg, _) = ore_core::validate::cargar_paquete(raiz);

    let declaradas = declaradas(&pkg);
    if declaradas.is_empty() {
        // No se escribe un lock vacío: un artefacto generado que no resuelve
        // nada es un fichero que hay que mantener sin que diga nada.
        return Ok(format!(
            "  Sin dependencias declaradas en `{CONFIG}`. No hay nada que resolver.\n"
        ));
    }

    // Lo que falta se trae, y traerlo se DELEGA: `ore` no sabe hablar por la
    // red. Lo que llega se vendoriza en el árbol —no en una caché— para que a
    // partir de aquí compilar no necesite a nadie: un clon recién hecho tiene
    // todo lo que el lock nombra, y su CI también.
    let (nuevo, traidas) = match traer_lo_que_falte(raiz, &pkg, &declaradas, comprobar)? {
        Some((nuevo, cuantas)) => (Some(nuevo), cuantas),
        None => (None, Vec::new()),
    };
    // El impacto, y este es el momento: el `.oob` nuevo ya está en el árbol pero
    // el lock todavía no se ha escrito, así que se puede decir qué cambia
    // **antes** de que cambie. Después es una compilación rota explicando algo
    // que ya pasó.
    //
    // Se computa entre los dos estados del MISMO árbol, no entre las dos
    // versiones del vocabulario: lo que le importa a quien lee esto no es que el
    // artículo 9 se moviera, es cuáles de sus propiedades se movieron con él.
    let cambios = nuevo
        .as_ref()
        .map(|n| ore_core::impacto::impacto(&pkg, n))
        .unwrap_or_default();
    let pkg = nuevo.unwrap_or(pkg);

    let disponibles = miembros(&pkg);
    let mut entradas = Vec::new();
    for (coordenada, raiz_de_quien) in &declaradas {
        let Some((dir, version)) = disponibles.get(coordenada) else {
            return Err(no_encontrada(coordenada, &disponibles));
        };
        let rango = rango_de(&pkg, coordenada)?;
        if !satisface(version, &rango)? {
            return Err(fallo(
                65, // EX_DATAERR
                format!("`{coordenada}` está en el árbol como `{version}`, y se pidió `{rango}`"),
                &[
                    "  Un lock que resolviera un rango con una versión que no lo satisface",
                    "  diría que se cumple algo que no se cumple. O se cambia el rango, o se",
                    "  vendoriza la versión que pide.",
                ],
            ));
        }
        entradas.push(Entrada {
            nombre: coordenada.clone(),
            version: version.clone(),
            ruta: relativa(raiz, dir),
            digest: digest_de(&pkg, dir),
            rango,
            raiz: *raiz_de_quien,
            provides: provides_de(&pkg, dir),
            firmantes: firmantes_de(&pkg, dir, coordenada, version),
            logs: logs_de(&pkg, dir, coordenada, version),
            sitio: dir.clone(),
        });
    }
    entradas.sort_by(|a, b| a.nombre.cmp(&b.nombre));

    // Las cabezas de log, avanzadas CON PRUEBA. Es la mitad de la transparencia
    // que no cabe en un paquete: quien publica no sabe que cabeza viste tu la
    // ultima vez, asi que la prueba de que el log extiende lo que ya viste se
    // pide igual que se pide un paquete.
    let observadas = observaciones(&pkg, &entradas, &raices_previas(&pkg))?;

    let texto = escribir(&nombre_del_workspace(&pkg), &entradas, &observadas);
    let ruta = raiz.join(LOCK);

    if comprobar {
        let actual = std::fs::read_to_string(&ruta).unwrap_or_default();
        if actual == texto {
            return Ok(format!(
                "  ✓ `{LOCK}` está al día · {} resueltas\n",
                entradas.len()
            ));
        }
        return Err(fallo(
            65,
            format!("`{LOCK}` no corresponde a `{CONFIG}`"),
            &[
                "  Es un artefacto generado y este quedó atrás. `ore lock` lo reescribe.",
                "  Se comprueba aparte de regenerarlo porque en CI hace falta saberlo sin",
                "  tocar el árbol: un artefacto obsoleto que se arregla solo al mirarlo no",
                "  se distingue de uno al día.",
            ],
        ));
    }

    std::fs::write(&ruta, &texto).map_err(|e| {
        fallo(
            73,
            format!("no se pudo escribir `{}`: {e}", ruta.display()),
            &[],
        )
    })?;
    Ok(informe(&entradas, &ruta, &traidas, &cambios))
}

// ── Lo que se declara y lo que hay ──────────────────────────────────────────

struct Entrada {
    nombre: String,
    version: String,
    ruta: String,
    digest: String,
    rango: String,
    raiz: bool,
    provides: BTreeMap<String, Vec<String>>,
    /// Donde vive el miembro. Se guarda en vez de reconstruirlo de `ruta`, que
    /// esta normalizada con barras hacia delante para que el lock sea el mismo
    /// en cualquier sistema: rehacerla daria una ruta que no casa con ninguna.
    sitio: PathBuf,
    /// Quién lo firmó, de lo que **se pudo comprobar aquí**.
    ///
    /// No se copia lo que el `.oob` dice de sí mismo: se verifica contra una
    /// clave que el consumidor declaró, y solo entra lo que verifica. Un lock
    /// que anotara firmantes sin comprobarlos sería un rumor con formato de
    /// artefacto — y el lock es justo el documento del que todo lo demás tira.
    firmantes: Vec<String>,
    /// En qué logs de transparencia está probado, tambien comprobado aquí.
    logs: Vec<String>,
}

/// Las dependencias declaradas, y si las pide el manifiesto o un paquete.
///
/// `requestedBy: root` es la del manifiesto; una declarada por un miembro es
/// transitiva desde el punto de vista del workspace, y el lock lo dice porque es
/// lo que permite saber **por qué** está ahí lo que está.
fn declaradas(pkg: &Package) -> Vec<(String, bool)> {
    let mut out: BTreeMap<String, bool> = BTreeMap::new();
    for d in &pkg.docs {
        let raiz = d.kind == Kind::OntologyConfig;
        if !raiz && d.kind != Kind::Package {
            continue;
        }
        for it in d.section("dependencies").map(|s| s.items()).unwrap_or(&[]) {
            if let Some(n) = it.get("package").and_then(|(_, v)| v.as_str()) {
                // Si la pide la raíz, la raíz manda: es la declaración que
                // alguien firma en el manifiesto publicable.
                let e = out.entry(n.to_string()).or_insert(raiz);
                *e = *e || raiz;
            }
        }
    }
    out.into_iter().collect()
}

/// El rango que se pidió para una coordenada. La primera declaración gana, y son
/// la misma porque `OOS2003` ya rechaza declararla dos veces.
fn rango_de(pkg: &Package, coordenada: &str) -> Result<String, Fallo> {
    for d in &pkg.docs {
        for it in d.section("dependencies").map(|s| s.items()).unwrap_or(&[]) {
            if it.get("package").and_then(|(_, v)| v.as_str()) == Some(coordenada)
                && let Some(r) = it.get("version").and_then(|(_, v)| v.as_str())
            {
                return Ok(r.to_string());
            }
        }
    }
    Err(fallo(
        65,
        format!("`{coordenada}` se declara sin `version`"),
        &["  Un rango es obligatorio: es lo único que dice a qué enunciado te acoges."],
    ))
}

/// Los paquetes que hay en el árbol, por su nombre — que es su coordenada.
fn miembros(pkg: &Package) -> BTreeMap<String, (PathBuf, String)> {
    pkg.docs
        .iter()
        .filter(|d| d.kind == Kind::Package)
        .filter_map(|d| {
            let nombre = d.meta("name")?.as_str()?.to_string();
            let version = d.meta("version")?.as_str()?.to_string();
            Some((nombre, (d.path.parent()?.to_path_buf(), version)))
        })
        .collect()
}

fn no_encontrada(coordenada: &str, disponibles: &BTreeMap<String, (PathBuf, String)>) -> Fallo {
    let mut ayuda = vec![
        "  `ore` no sabe hablar por la red, así que traerla se delega: pon un `ore-fetch`"
            .to_string(),
        "  en el PATH que lea la petición por stdin y escriba el `.oob` por stdout, o".to_string(),
        "  vendoriza el paquete en el árbol como un miembro más.".to_string(),
        "  Lo que llegue se comprueba igual —el paquete, la versión y el digest—, que es"
            .to_string(),
        "  lo que permite que su origen no tenga que ser de confianza.".to_string(),
    ];
    if !disponibles.is_empty() {
        // Con la versión: en un fallo de rango, saber que `gdpr` está pero en
        // `0.1.0` es justo el dato que convierte el error en una acción.
        ayuda.push(format!(
            "  En el árbol hay: {}",
            disponibles
                .iter()
                .map(|(n, (_, v))| format!("{n} {v}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Fallo {
        codigo: 69, // EX_UNAVAILABLE
        mensaje: format!("`{coordenada}` no está en el árbol"),
        ayuda,
    }
}

// ── Traer lo que no está ────────────────────────────────────────────────────

const OBTENEDOR: &str = "ore-fetch";
const VENDOR: &str = "vendor";

/// Trae las coordenadas que no estén en el árbol, y devuelve el árbol recargado
/// si trajo alguna.
///
/// `--check` **no trae nada**: comprobar es comprobar, y un lock que se arregla
/// solo al mirarlo no se distingue de uno al día.
fn traer_lo_que_falte(
    raiz: &Path,
    pkg: &Package,
    declaradas: &[(String, bool)],
    comprobar: bool,
) -> Result<Option<(Package, Vec<String>)>, Fallo> {
    let hay = miembros(pkg);
    // Falta lo que no está **y también lo que está y se quedó corto**: subir el
    // rango de `^0.1` a `^0.2` con la vieja vendorizada fallaba en vez de traer
    // la nueva, así que actualizar era borrar un fichero a mano. Un ciclo de
    // vida que solo cubre la primera vez no es un ciclo de vida.
    let faltan: Vec<&(String, bool)> = declaradas
        .iter()
        .filter(|(c, _)| match hay.get(c) {
            None => true,
            Some((_, v)) => rango_de(pkg, c)
                .and_then(|r| satisface(v, &r))
                .is_ok_and(|ok| !ok),
        })
        .collect();
    if faltan.is_empty() || comprobar {
        return Ok(None);
    }
    if crate::lector::resolver(OBTENEDOR).is_none() {
        return Ok(None); // sin obtenedor, el error de siempre y su ayuda
    }

    let mut traidas = Vec::new();
    for (coordenada, _) in faltan {
        let rango = rango_de(pkg, coordenada)?;
        let bytes = obtener(coordenada, &rango)?;
        let (nombre, version) = sobre_de(&bytes, coordenada, &rango)?;
        let ruta = raiz.join(VENDOR).join(format!(
            "{}-{version}.oob",
            nombre.rsplit('/').next().unwrap_or(&nombre)
        ));
        std::fs::create_dir_all(raiz.join(VENDOR))
            .map_err(|e| fallo(73, format!("no se pudo crear `{VENDOR}/`: {e}"), &[]))?;
        std::fs::write(&ruta, &bytes).map_err(|e| {
            fallo(
                73,
                format!("no se pudo escribir `{}`: {e}", ruta.display()),
                &[],
            )
        })?;
        // Y se retira la que había. Dos `.oob` del mismo paquete en el árbol son
        // dos verdades sobre lo mismo, y el cargador las metería las dos: el
        // concepto quedaría declarado dos veces y ganaría la que ordenara antes.
        retirar_anteriores(raiz, &nombre, &ruta)?;
        traidas.push(nombre);
    }
    Ok(Some((ore_core::validate::cargar_paquete(raiz).0, traidas)))
}

/// Quita los `.oob` del mismo paquete que no sean el que se acaba de traer.
///
/// No se borra por nombre de fichero —que es una convención— sino por lo que el
/// sobre **dice ser**: un `.oob` renombrado seguiría siendo el mismo paquete, y
/// dejarlo ahí dejaría dos versiones del mismo concepto compitiendo en silencio.
fn retirar_anteriores(raiz: &Path, nombre: &str, salvo: &Path) -> Result<(), Fallo> {
    let Ok(entradas) = std::fs::read_dir(raiz.join(VENDOR)) else {
        return Ok(());
    };
    for e in entradas.flatten() {
        let p = e.path();
        if p == salvo || p.extension().is_none_or(|x| x != "oob") {
            continue;
        }
        let Ok(t) = std::fs::read_to_string(&p) else {
            continue;
        };
        let suyo = ore_core::parse::parse(&t).ok().and_then(|j| {
            j.get("package")
                .and_then(|(_, v)| v.as_str())
                .map(String::from)
        });
        if suyo.as_deref() == Some(nombre) {
            std::fs::remove_file(&p).map_err(|e| {
                fallo(
                    73,
                    format!("no se pudo retirar `{}`: {e}", p.display()),
                    &[],
                )
            })?;
        }
    }
    Ok(())
}

/// Ejecuta el obtenedor. La petición va por **stdin**, nunca por `argv`: lo lee
/// cualquier proceso de la máquina, y una coordenada privada dice de qué depende
/// una organización.
fn obtener(coordenada: &str, rango: &str) -> Result<String, Fallo> {
    let peticion = ore_core::json::Json::obj([
        ("package", ore_core::json::Json::s(coordenada)),
        ("range", ore_core::json::Json::s(rango)),
    ])
    .jcs();
    crate::lector::ejecutar(OBTENEDOR, &[], Some(&peticion)).map_err(|f| Fallo {
        codigo: f.codigo,
        mensaje: f.mensaje,
        ayuda: f.ayuda,
    })
}

/// Lo que dice el sobre, comprobado contra lo que se pidió.
///
/// **Nada de lo que llega se cree.** El obtenedor puede equivocarse o mentir, y
/// da igual cuál de las dos: un `.oob` que diga otro paquete o una versión fuera
/// del rango no se escribe. Es la misma postura que hace que el origen no tenga
/// que ser de confianza — y por eso el obtenedor de referencia ni siquiera
/// interpreta el rango.
fn sobre_de(bytes: &str, coordenada: &str, rango: &str) -> Result<(String, String), Fallo> {
    let j = ore_core::parse::parse(bytes).map_err(|e| {
        fallo(
            65,
            format!("lo que devolvió `{OBTENEDOR}` no analiza: {e:?}"),
            &[],
        )
    })?;
    let campo = |k: &str| j.get(k).and_then(|(_, v)| v.as_str()).map(String::from);
    let (Some(nombre), Some(version)) = (campo("package"), campo("version")) else {
        return Err(fallo(
            65,
            format!("lo que devolvió `{OBTENEDOR}` no dice qué paquete es"),
            &["  Un `.oob` lleva su identidad dentro: uno renombrado es uno que miente."],
        ));
    };
    if nombre != coordenada {
        return Err(fallo(
            65,
            format!("se pidió `{coordenada}` y llegó `{nombre}`"),
            &["  No se escribe lo que no es lo que se pidió, venga de donde venga."],
        ));
    }
    if !satisface(&version, rango)? {
        return Err(fallo(
            65,
            format!("`{coordenada}` llegó en `{version}`, y se pidió `{rango}`"),
            &["  El obtenedor no interpreta el rango, y por eso se comprueba aquí."],
        ));
    }
    Ok((nombre, version))
}

// ── Los rangos ──────────────────────────────────────────────────────────────

fn partes(v: &str) -> Option<(u64, u64, u64)> {
    let limpio = v.split(['-', '+']).next()?;
    let mut it = limpio.split('.');
    let a = it.next()?.parse().ok()?;
    let b = it.next().unwrap_or("0").parse().ok()?;
    let c = it.next().unwrap_or("0").parse().ok()?;
    Some((a, b, c))
}

/// `^`, `~` y la versión exacta.
///
/// Un rango que este resolutor no sepa leer **falla**, no pasa. Aceptarlo
/// escribiría en el lock que se cumple algo que nadie comprobó, que es la
/// dirección insegura: `>=1.0 <2.0` está en la gramática y todavía no aquí.
fn satisface(version: &str, rango: &str) -> Result<bool, Fallo> {
    let r = rango.trim();
    let Some(v) = partes(version) else {
        return Err(fallo(
            65,
            format!("`{version}` no es una versión semver"),
            &[],
        ));
    };
    let (op, resto) = match r.strip_prefix('^') {
        Some(x) => ('^', x),
        None => match r.strip_prefix('~') {
            Some(x) => ('~', x),
            None => ('=', r),
        },
    };
    let Some(p) = partes(resto) else {
        return Err(fallo(
            65,
            format!("no se sabe leer el rango `{rango}`"),
            &[
                "  Se admiten `^X.Y[.Z]`, `~X.Y[.Z]` y una versión exacta. Los rangos con",
                "  `>=` y `<` están en la gramática y no aquí — y aceptarlos sin",
                "  comprobarlos escribiría en el lock que se cumple algo que nadie miró.",
            ],
        ));
    };
    let techo = match op {
        // `^0.1` no es `<1.0.0`: en `0.x` cada minor puede romper, y esa es la
        // convención que usan todos los ecosistemas que la resolvieron antes.
        '^' if p.0 == 0 => (0, p.1 + 1, 0),
        '^' => (p.0 + 1, 0, 0),
        '~' => (p.0, p.1 + 1, 0),
        _ => return Ok(v == p),
    };
    Ok(v >= p && v < techo)
}

// ── Lo que se escribe ───────────────────────────────────────────────────────

fn relativa(raiz: &Path, dir: &Path) -> String {
    dir.strip_prefix(raiz)
        .unwrap_or(dir)
        .to_string_lossy()
        .replace('\\', "/")
}

/// El digest de lo que hay en el árbol, con el mismo `digest::package` del
/// bundle — y sobre **el mismo conjunto de documentos** que empaquetaría
/// `ore pack`.
///
/// Eso último no es un detalle: `01-distribucion` §2 promete que **el contenedor
/// no cambia la identidad**, y digerir aquí el manifiesto del miembro —que
/// `pack` excluye por ser del workspace— rompía la promesa. Salió midiendo: el
/// mismo directorio daba `b28cae9b` por un lado y `d47f6ee8` por el otro.
fn digest_de(pkg: &Package, miembro: &Path) -> String {
    ore_core::digest::package(&ore_core::link::publicables(&del_miembro(pkg, miembro)))
}

/// Los documentos de un miembro, sacados del workspace ya cargado.
///
/// Se filtra en vez de volver a leer el directorio, y no es una optimización: un
/// miembro **puede no ser un directorio**. Un paquete importado es un `.oob`, y
/// releerlo como carpeta daba el digest de la nada — `e3b0c442…`, el SHA de la
/// cadena vacía, escrito tan campante en un lock.
fn del_miembro(pkg: &Package, miembro: &Path) -> Package {
    let miembros = ore_core::link::miembros(pkg);
    Package {
        root: pkg.root.clone(),
        docs: pkg
            .docs
            .iter()
            .filter(|d| ore_core::link::miembro_de(&miembros, &d.path) == Some(miembro))
            .map(|d| ore_core::link::Loaded {
                path: d.path.clone(),
                kind: d.kind,
                root: d.root.clone(),
            })
            .collect(),
        cedar: Vec::new(),
        generated: Vec::new(),
        sobres: Vec::new(),
    }
}

/// El programa que sirve el log, y tampoco está aquí dentro.
const REGISTRADOR: &str = "ore-log";

/// Lo que este árbol ha visto de un log: su última cabeza dada por buena.
struct Observacion {
    id: String,
    tamano: u64,
    raiz: String,
}

/// En qué logs está probado un miembro, de lo que se comprueba aquí.
///
/// Misma postura que con la firma: se verifica la cabeza —que es lo que
/// convierte una raíz en la afirmación de alguien— y luego la inclusión, con la
/// hoja construida aquí a partir del digest recomputado. Nada de lo que el
/// paquete dice de sí mismo entra en el lock sin pasar por eso.
fn logs_de(pkg: &Package, miembro: &Path, nombre: &str, version: &str) -> Vec<String> {
    let mut out: Vec<String> = pruebas_de(pkg, miembro, nombre, version)
        .into_iter()
        .map(|(id, _, _)| id)
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Las pruebas de transparencia de un miembro que **verifican**: `(log, tamaño,
/// raíz)`.
fn pruebas_de(
    pkg: &Package,
    miembro: &Path,
    nombre: &str,
    version: &str,
) -> Vec<(String, u64, String)> {
    use ore_core::transparencia as t;
    let Some((_, sobre)) = pkg.sobres.iter().find(|(p, _)| p == miembro) else {
        return Vec::new();
    };
    let logs = logs_confiados(pkg);
    if logs.is_empty() {
        return Vec::new();
    }
    let enunciado = ore_core::firma::enunciado(nombre, version, &digest_de(pkg, miembro));
    let firmas: BTreeMap<String, String> = sobre
        .get("signatures")
        .map(|(_, v)| v.items())
        .unwrap_or(&[])
        .iter()
        .filter_map(|f| {
            let leer = |k: &str| f.get(k).and_then(|(_, v)| v.as_str()).map(String::from);
            Some((leer("keyId")?, leer("signature")?))
        })
        .collect();

    sobre
        .get("transparency")
        .map(|(_, v)| v.items())
        .unwrap_or(&[])
        .iter()
        .filter_map(|e| {
            let leer = |k: &str| e.get(k).and_then(|(_, v)| v.as_str()).unwrap_or_default();
            let publica = logs.get(leer("logId"))?;
            let tamano = leer("treeSize").parse::<u64>().ok()?;
            let indice = leer("index").parse::<u64>().ok()?;
            let raiz = t::de_hex(leer("root"))?;
            ore_core::firma::verificar(
                ore_core::firma::ED25519,
                publica,
                leer("rootSignature"),
                &t::cabeza(leer("logId"), tamano, &raiz),
            )
            .ok()?;
            let hoja = t::hoja(
                t::entrada(&enunciado, leer("keyId"), firmas.get(leer("keyId"))?).as_bytes(),
            );
            let camino: Vec<t::Hash> = e
                .get("inclusion")
                .map(|(_, v)| v.items())
                .unwrap_or(&[])
                .iter()
                .filter_map(|h| h.as_str().and_then(t::de_hex))
                .collect();
            t::inclusion(&hoja, indice, tamano, &camino, &raiz).ok()?;
            Some((leer("logId").to_string(), tamano, t::a_hex(&raiz)))
        })
        .collect()
}

/// `logId → clave pública` del manifiesto.
fn logs_confiados(pkg: &Package) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for d in pkg.docs.iter().filter(|d| d.kind == Kind::OntologyConfig) {
        for l in d.section("trustedLogs").map(|n| n.items()).unwrap_or(&[]) {
            let leer = |c: &str| l.get(c).and_then(|(_, v)| v.as_str()).map(String::from);
            if let (Some(id), Some(pk)) = (leer("id"), leer("publicKey")) {
                out.insert(id, pk);
            }
        }
    }
    out
}

/// La observación de cada log que se escribe en el lock, **avanzada con prueba**.
///
/// Aquí está la mitad de la transparencia que no cabe en un paquete: quien
/// publica no sabe qué cabeza viste tú la última vez, así que la prueba de que
/// el log **extiende** lo que ya viste no puede viajar dentro del `.oob`. Se
/// pide, como se pide un paquete.
///
/// Y por eso vive en `lock` y no en `validate`: aquí se delega, allí no se toca
/// la red. `validate` comprueba lo que ya está en el árbol —inclusión, cabeza
/// firmada, y que la raíz sea la que este lock fijó— y eso es hermético.
///
/// Sin esto, la única garantía sería *«el log dijo esto»*, que es lo que dice
/// cualquier firma. Lo que convierte eso en *«y no ha dicho nunca otra cosa»* es
/// la consistencia.
fn observaciones(
    pkg: &Package,
    entradas: &[Entrada],
    previas: &BTreeMap<String, (u64, String)>,
) -> Result<Vec<Observacion>, Fallo> {
    use ore_core::transparencia as t;
    // La cabeza más alta que traiga cualquier paquete, por log.
    let mut vistas: BTreeMap<String, (u64, String)> = BTreeMap::new();
    for e in entradas {
        for (id, tamano, raiz) in pruebas_de(pkg, &e.sitio, &e.nombre, &e.version) {
            let hueco = vistas.entry(id).or_insert((tamano, raiz.clone()));
            if tamano > hueco.0 {
                *hueco = (tamano, raiz);
            }
        }
    }
    // Y las que ya estaban, aunque ningun paquete las mencione esta vez: una
    // observacion no se pierde porque este lock no la haya vuelto a ver. Si se
    // perdiera, el ancla contra la bifurcacion se borraria sola con solo dejar
    // de traer la prueba.
    for (id, (tamano, raiz)) in previas {
        vistas.entry(id.clone()).or_insert((*tamano, raiz.clone()));
    }

    let mut out = Vec::new();
    for (id, (tamano, raiz)) in vistas {
        let Some((antes, raiz_antes)) = previas.get(&id) else {
            // Primera cabeza de este log. No hay nada anterior con lo que ser
            // consistente, y exigirlo dejaría imposible empezar.
            out.push(Observacion { id, tamano, raiz });
            continue;
        };
        if *antes == tamano {
            if *raiz_antes != raiz {
                return Err(bifurcacion(&id, tamano, raiz_antes, &raiz));
            }
            out.push(Observacion { id, tamano, raiz });
            continue;
        }
        // El grande tiene que extender al pequeño, venga el pequeño del lock o
        // del paquete: un `.oob` viejo se comprueba igual que uno nuevo.
        let (menor, r_menor, mayor, r_mayor) = if *antes < tamano {
            (*antes, raiz_antes.clone(), tamano, raiz.clone())
        } else {
            (tamano, raiz.clone(), *antes, raiz_antes.clone())
        };
        let prueba = pedir_consistencia(&id, menor, mayor)?;
        let (Some(a), Some(b)) = (t::de_hex(&r_menor), t::de_hex(&r_mayor)) else {
            return Err(fallo(65, format!("`{id}`: una raíz no es un hash"), &[]));
        };
        t::consistencia(menor, &a, mayor, &b, &prueba).map_err(|e| {
            fallo(
                65,
                format!(
                    "`{id}` no demuestra que el árbol de {mayor} extienda al de {menor}: {}",
                    e.como_texto()
                ),
                &[
                    "  Un log que no puede probar que solo ha crecido no es un log de",
                    "  transparencia: es una lista firmada, y una lista firmada puede tener",
                    "  dos versiones sin que ninguna prueba de inclusión lo note.",
                ],
            )
        })?;
        out.push(Observacion {
            id,
            tamano: mayor,
            raiz: r_mayor,
        });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

fn bifurcacion(id: &str, tamano: u64, antes: &str, ahora: &str) -> Fallo {
    let mut f = fallo(
        65,
        format!("`{id}` da dos raíces para el árbol de {tamano} entradas"),
        &[
            "  El lock fijó una y lo que llega trae otra. Dos historias del mismo log es",
            "  una bifurcación, y es el ataque entero: ninguna prueba de inclusión la ve,",
            "  porque cada una cuadra contra la suya.",
            "  Aquí no hay nada que arreglar en el árbol. Hay un log que no se puede usar.",
        ],
    );
    // Las dos raíces, literales. Es lo único de este error que se puede llevar
    // a otra parte: quien lo reciba tiene que poder enseñárselas a alguien.
    f.ayuda.push(format!("  lock: {antes}"));
    f.ayuda.push(format!("  trae: {ahora}"));
    f
}

/// Lo que el lock que ya existe fija de cada log.
///
/// Se lee del árbol cargado y no del fichero: el lock **es** un documento del
/// paquete desde que el cargador lo abre, y releerlo aparte habría sido una
/// segunda forma de leer lo mismo.
fn raices_previas(pkg: &Package) -> BTreeMap<String, (u64, String)> {
    let mut out = BTreeMap::new();
    let Some(l) = pkg.docs.iter().find(|d| ore_core::normalize::es_lock(d)) else {
        return out;
    };
    if let Some((_, ls)) = l.root.get("logs") {
        for x in ls.items() {
            let leer = |c: &str| x.get(c).and_then(|(_, v)| v.as_str()).map(String::from);
            if let (Some(id), Some(t), Some(r)) = (leer("id"), leer("treeSize"), leer("root"))
                && let Ok(t) = t.parse::<u64>()
            {
                out.insert(id, (t, r));
            }
        }
    }
    out
}

/// Pide la prueba de que un árbol extiende a otro. Se delega, como todo lo que
/// sale de esta máquina.
fn pedir_consistencia(
    id: &str,
    desde: u64,
    hasta: u64,
) -> Result<Vec<ore_core::transparencia::Hash>, Fallo> {
    let peticion = ore_core::json::Json::obj([
        ("from", ore_core::json::Json::Int(desde as i64)),
        ("op", ore_core::json::Json::s("consistency")),
        ("to", ore_core::json::Json::Int(hasta as i64)),
    ])
    .jcs();
    let bruta = crate::lector::ejecutar(REGISTRADOR, &[], Some(&peticion)).map_err(|f| Fallo {
        codigo: f.codigo,
        mensaje: format!("`{id}`: {}", f.mensaje),
        ayuda: f.ayuda,
    })?;
    let r = ore_core::parse::parse(&bruta)
        .map_err(|e| fallo(65, format!("`{REGISTRADOR}` no devolvió JSON: {e:?}"), &[]))?;
    Ok(r.get("consistency")
        .map(|(_, v)| v.items())
        .unwrap_or(&[])
        .iter()
        .filter_map(|h| h.as_str().and_then(ore_core::transparencia::de_hex))
        .collect())
}

/// Quién firmó un miembro, **de lo que se puede comprobar aquí y ahora**.
///
/// Se verifica cada firma del sobre contra las claves que el consumidor declara
/// en `trustedKeys`, y solo sale lo que verifica. Una firma de una clave
/// desconocida no entra: no hay con qué comprobarla, y escribirla en el lock la
/// convertiría en un hecho por el mero acto de anotarla.
///
/// El digest se recomputa del árbol en vez de leerse del sobre por la misma
/// razón por la que el `.oob` no lleva el suyo dentro: **un número que un lector
/// no debe creerse acaba creído**.
fn firmantes_de(pkg: &Package, miembro: &Path, nombre: &str, version: &str) -> Vec<String> {
    let Some((_, sobre)) = pkg.sobres.iter().find(|(p, _)| p == miembro) else {
        return Vec::new(); // un miembro que es un directorio no tiene sobre
    };
    let confianza = claves(pkg);
    if confianza.is_empty() {
        return Vec::new();
    }
    let digest = digest_de(pkg, miembro);
    let enunciado = ore_core::firma::enunciado(nombre, version, &digest);
    let mut out: Vec<String> = sobre
        .get("signatures")
        .map(|(_, v)| v.items())
        .unwrap_or(&[])
        .iter()
        .filter_map(|f| {
            let leer = |k: &str| f.get(k).and_then(|(_, v)| v.as_str()).unwrap_or_default();
            let (alg, publica) = confianza.get(leer("keyId"))?;
            (leer("algorithm") == alg
                && ore_core::firma::verificar(alg, publica, leer("signature"), &enunciado).is_ok())
            .then(|| leer("keyId").to_string())
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// `keyId → (algoritmo, clave pública)` del manifiesto.
fn claves(pkg: &Package) -> BTreeMap<String, (String, String)> {
    let mut out = BTreeMap::new();
    for d in pkg.docs.iter().filter(|d| d.kind == Kind::OntologyConfig) {
        for k in d.section("trustedKeys").map(|n| n.items()).unwrap_or(&[]) {
            let leer = |c: &str| k.get(c).and_then(|(_, v)| v.as_str()).map(String::from);
            if let (Some(id), Some(pk)) = (leer("id"), leer("publicKey")) {
                let alg = leer("algorithm").unwrap_or_else(|| ore_core::firma::ED25519.to_string());
                out.insert(id, (alg, pk));
            }
        }
    }
    out
}

/// Qué aporta un paquete, **derivado de lo que tiene dentro**. Lo derivable no
/// se declara (P2), y una lista escrita a mano en un artefacto generado sería
/// dos veces lo mismo.
fn provides_de(pkg: &Package, miembro: &Path) -> BTreeMap<String, Vec<String>> {
    let pkg = del_miembro(pkg, miembro);
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for d in &pkg.docs {
        let clave = match d.kind {
            Kind::Property => "concepts",
            Kind::Lattice => "lattices",
            Kind::ConduitPolicy => "conduits",
            Kind::Ruleset => "rulesets",
            Kind::Interface => "interfaces",
            Kind::Entity => "entities",
            Kind::Function => "functions",
            Kind::Resolution => "resolutions",
            // `Binding`, `Package` y el manifiesto no se aportan: el primero
            // dice dónde está el dato de QUIEN LO PUBLICA, y los otros dos son
            // el paquete, no algo dentro de él.
            _ => continue,
        };
        if let Some(q) = d.qname() {
            out.entry(clave.to_string()).or_default().push(q);
        }
    }
    for v in out.values_mut() {
        v.sort();
        v.dedup();
    }
    out
}

fn nombre_del_workspace(pkg: &Package) -> String {
    pkg.docs
        .iter()
        .find(|d| d.kind == Kind::OntologyConfig)
        .and_then(|d| {
            let n = d.meta("name")?.as_str()?;
            let v = d.meta("version")?.as_str()?;
            Some(format!("{n}@{v}"))
        })
        .unwrap_or_else(|| "desconocido".into())
}

fn escribir(para: &str, entradas: &[Entrada], logs: &[Observacion]) -> String {
    let mut s = String::from(
        "# =============================================================================\n\
         # GENERADO POR ORE — NO EDITAR A MANO\n\
         #\n\
         # Resolución determinista de dependencias. Se compromete a Git.\n\
         # Regenerar: `ore lock`   ·   Verificar sin modificar: `ore lock --check`\n\
         #\n\
         # Todo lo de aquí se resolvió CONTRA EL ÁRBOL. `ore` no habla por la red, así\n\
         # que un paquete se resuelve si está vendorizado como un miembro del workspace\n\
         # y si no, esto falla en vez de inventar una entrada. El día que exista un\n\
         # registro, traerlo se delegará como se delega leer una fuente.\n\
         #\n\
         # `digest` es el de los documentos que hay en el árbol, con el mismo cómputo\n\
         # que el del bundle. NO es el digest de un paquete publicado: ese formato no\n\
         # existe todavía, y decir que lo es sería afirmar una procedencia que nadie\n\
         # puede comprobar. Por lo mismo no hay `profiles`: sus digests no se pueden\n\
         # computar de nada, y uno inventado es peor que ninguno.\n\
         #\n\
         # `signedBy` es lo que se COMPROBÓ, no lo que el paquete dice de sí mismo: solo\n\
         # entra una firma que verifique contra una clave declarada en `trustedKeys`. Y\n\
         # una vez escrita obliga — quitar la firma del árbol deja de compilar, que es lo\n\
         # que impide saltarse la comprobación borrando un campo.\n\
         # =============================================================================\n\
         \n\
         lockfileVersion: 1\n",
    );
    let _ = writeln!(s, "generatedFor: {para}\n");
    s.push_str("packages:\n");
    for e in entradas {
        let _ = write!(
            s,
            "\n  - name: {}\n    version: {}\n    resolved: file:{}\n    digest: {}\n    range: \"{}\"\n",
            e.nombre, e.version, e.ruta, e.digest, e.rango
        );
        if e.raiz {
            s.push_str("    requestedBy: root\n");
        }
        if !e.firmantes.is_empty() {
            let _ = writeln!(s, "    signedBy: [{}]", e.firmantes.join(", "));
        }
        if !e.logs.is_empty() {
            let _ = writeln!(s, "    logged: [{}]", e.logs.join(", "));
        }
        if !e.provides.is_empty() {
            s.push_str("    provides:\n");
            for (clave, valores) in &e.provides {
                let _ = writeln!(s, "      {clave}: [{}]", valores.join(", "));
            }
        }
    }
    // Las cabezas van al final y **fuera de `packages`**, porque no son de
    // ningún paquete: son de este árbol. Es lo último que vio de cada log, y lo
    // que hace que una raíz distinta para el mismo tamaño se note.
    if !logs.is_empty() {
        s.push_str("\nlogs:\n");
        for l in logs {
            let _ = write!(
                s,
                "\n  - id: {}\n    treeSize: {}\n    root: {}\n",
                l.id, l.tamano, l.raiz
            );
        }
    }
    s
}

fn informe(entradas: &[Entrada], ruta: &Path, traidas: &[String], cambios: &[Cambio]) -> String {
    let mut s = format!("  ✓ {}\n", ruta.display());
    for e in entradas {
        let _ = writeln!(
            s,
            "  · {} {} · {} · {}",
            e.nombre,
            e.version,
            e.ruta,
            &e.digest[..e.digest.len().min(19)]
        );
    }
    // Decir «nada se ha traído de fuera» justo después de traerlo era una línea
    // escrita cuando no había obtenedor, y dejó de ser cierta el día que lo
    // hubo. Un informe que no distingue las dos cosas deja de informar de la que
    // importa: qué entró en este árbol desde fuera.
    if traidas.is_empty() {
        s.push_str("\n  Todo estaba en el árbol. No se ha traído nada de fuera.\n");
    } else {
        let _ = writeln!(
            s,
            "\n  {} traída(s) con `ore-fetch` y vendorizada(s) en `vendor/`: {}.",
            traidas.len(),
            traidas.join(", ")
        );
        s.push_str(
            "  Verificadas —paquete, versión y digest— antes de escribirlas, que es lo\n\
             \x20 que permite que su origen no tenga que ser de confianza. A partir de\n\
             \x20 aquí este árbol compila sin nadie.\n",
        );
        impacto(&mut s, cambios);
    }
    s
}

/// Qué cambia **en el árbol de quien lee esto**, que es la única versión de la
/// pregunta que se puede accionar.
///
/// Se dice también cuando no cambia nada. «Ninguna propiedad tuya se mueve» es
/// información —significa que se puede aceptar sin mirar— y callarlo dejaría el
/// silencio haciendo dos trabajos: el de «no pasa nada» y el de «esto no se
/// comprobó».
///
/// No hay tope ni resumen. Un informe que dijera *«y 14 más»* volvería a poner a
/// quien lo lee donde estaba, que es compilando para enterarse.
fn impacto(s: &mut String, cambios: &[Cambio]) {
    s.push_str("\n  Y esto es lo que cambia EN TU ÁRBOL:\n");
    if cambios.is_empty() {
        s.push_str(
            "  · nada. Ninguna propiedad tuya cambia de clasificación ni se queda sin\n\
             \x20   regla que la cubra.\n",
        );
        return;
    }

    let mut mueven = Vec::new();
    let mut sin = Vec::new();
    let mut con = Vec::new();
    for c in cambios {
        match c {
            Cambio::Clasificacion {
                propiedad,
                reticulo,
                antes,
                despues,
            } => {
                // `—` y no un hueco en blanco: que una etiqueta no estuviera
                // antes es un dato, y el hueco se lee como un fallo de formato.
                let (a, d) = (
                    antes.as_deref().unwrap_or("—"),
                    despues.as_deref().unwrap_or("—"),
                );
                mueven.push(format!("      {propiedad} · {reticulo}: {a} → {d}"));
            }
            Cambio::SinCobertura {
                propiedad,
                clases,
                porque,
            } => sin.push(format!(
                "      {propiedad} · exige {} · lo pide {}",
                lista(clases),
                if porque.is_empty() {
                    "el vocabulario".to_string()
                } else {
                    porque.join(", ")
                }
            )),
            Cambio::ConCobertura { propiedad, clases } => {
                con.push(format!("      {propiedad} · ya no exige {}", lista(clases)));
            }
        }
    }

    for (titulo, lineas) in [
        ("cambia(n) de clasificación", &mueven),
        ("se queda(n) sin regla que la(s) cubra", &sin),
        ("deja(n) de faltarle una regla", &con),
    ] {
        if lineas.is_empty() {
            continue;
        }
        let _ = writeln!(s, "  · {} {titulo}", lineas.len());
        for l in lineas {
            let _ = writeln!(s, "{l}");
        }
    }

    if !sin.is_empty() {
        s.push_str(
            "\n  Lo del medio es `OOS8001` con fecha futura: el lock ya está escrito, y\n\
             \x20 la próxima compilación se parará ahí.\n",
        );
    }
}

/// `` `authorization` y `retention` ``.
fn lista(clases: &std::collections::BTreeSet<String>) -> String {
    clases
        .iter()
        .map(|c| format!("`{c}`"))
        .collect::<Vec<_>>()
        .join(" y ")
}

// ── Comprobaciones ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// En `0.x` cada minor puede romper, y por eso `^0.1` no llega a `0.2`. Es
    /// la convención de todos los ecosistemas que resolvieron esto antes, y
    /// tratarla como `^1.x` dejaría pasar una versión rompedora.
    #[test]
    fn el_cero_mayor_es_estricto() {
        assert!(satisface("0.1.0", "^0.1").unwrap());
        assert!(satisface("0.1.9", "^0.1").unwrap());
        assert!(!satisface("0.2.0", "^0.1").unwrap());
        assert!(!satisface("0.0.9", "^0.1").unwrap());
    }

    #[test]
    fn el_acento_circunflejo_y_la_tilde_no_son_lo_mismo() {
        assert!(satisface("2.9.0", "^2.1").unwrap());
        assert!(!satisface("3.0.0", "^2.1").unwrap());
        assert!(satisface("2.1.9", "~2.1").unwrap());
        assert!(!satisface("2.2.0", "~2.1").unwrap());
    }

    #[test]
    fn una_version_exacta_es_exacta() {
        assert!(satisface("1.2.3", "1.2.3").unwrap());
        assert!(!satisface("1.2.4", "1.2.3").unwrap());
    }

    /// Un rango que no se sabe leer falla. Aceptarlo escribiría en el lock que
    /// se cumple algo que nadie comprobó, que es la dirección insegura.
    #[test]
    fn un_rango_que_no_se_sabe_leer_no_pasa() {
        assert!(satisface("1.0.0", ">=1.0 <2.0").is_err());
    }
}
