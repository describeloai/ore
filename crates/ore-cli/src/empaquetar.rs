//! `ore pack` — el paquete publicable, y por qué no es un `tar.gz`.
//!
//! Normativo: [`spec/v1alpha6/01-distribucion.md`]. La decisión de la versión
//! entera cabe en una línea, y era la respuesta que no parecía la evidente:
//!
//! > **Un `.oob` es la forma canónica escrita en un fichero.**
//!
//! Un archivo comprimido —lo que uno escribe primero— lleva marcas de tiempo,
//! orden de entradas, permisos y nivel de compresión: **el mismo paquete produce
//! bytes distintos**, y el digest deja de ser función del contenido. Habría
//! hecho falta inventar un «formato de archivo determinista». La forma canónica
//! **ya es** una serialización determinista de un paquete y su digest **ya
//! está** definido, así que no hay formato nuevo: hay un sobre.
//!
//! # El contenedor no cambia la identidad
//!
//! El digest de un `.oob` es el del paquete —sobre las identidades de sus
//! documentos, nunca sobre las rutas (`digest` §5.2)—, así que **el mismo
//! paquete vendorizado como árbol y publicado como `.oob` digiere igual**. Un
//! lock resuelto contra el árbol sigue valiendo el día que ese paquete se
//! publique.
//!
//! Si el digest hubiera sido el del fichero, cambiar de contenedor habría sido
//! indistinguible de cambiar de paquete.
//!
//! # Y el digest NO va dentro
//!
//! Se podría: se computa sobre `documents` y se guarda al lado, sin
//! autorreferencia. No va, y el motivo no es técnico — **un número que un lector
//! no debe creerse acaba creído**. Lo que se verifica es contra el lock de quien
//! consume, nunca contra lo que el fichero dice de sí mismo.

use ore_core::document::Kind;
use ore_core::json::Json;
use ore_core::link::Package;
use std::path::Path;
use std::process::ExitCode;

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

pub fn pack(
    raiz: &Path,
    destino: Option<&Path>,
    firmar: Option<&str>,
    anotar: Option<&str>,
) -> ExitCode {
    match intentar(raiz, destino, firmar, anotar) {
        // Sin `-o` el `.oob` sale por **stdout** y el resumen por stderr, que es
        // la forma de todo lo que emite aquí: lo que se canaliza es el
        // artefacto, y lo que se lee es lo otro. Con `-o` no hay nada que
        // canalizar y el resumen es la salida.
        Ok((bytes, resumen)) => {
            match destino {
                Some(_) => print!("{resumen}"),
                None => {
                    use std::io::Write as _;
                    let _ = std::io::stdout().write_all(bytes.as_bytes());
                    eprint!("{resumen}");
                }
            }
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

fn intentar(
    raiz: &Path,
    destino: Option<&Path>,
    firmar: Option<&str>,
    anotar: Option<&str>,
) -> Result<(String, String), Fallo> {
    if !raiz.is_dir() {
        return Err(fallo(
            66, // EX_NOINPUT
            format!("`{}` no es un directorio de paquete", raiz.display()),
            &[],
        ));
    }

    // §5 · lo que no valida no se publica. Repartir algo que no compila es
    // repartir un problema en vez de un paquete, y el que lo importe lo
    // descubrirá en su propio árbol, que es donde no puede arreglarlo.
    let diags = ore_core::validate_package(raiz);
    if let Some(d) = diags.first() {
        eprintln!("{}", d.render(raiz));
        return Err(fallo(
            65, // EX_DATAERR
            format!("`{}` no valida, así que no se empaqueta", raiz.display()),
            &["  Publicar lo que no compila reparte un problema en vez de un paquete."],
        ));
    }

    let (pkg, _) = ore_core::validate::cargar_paquete(raiz);
    let publicables = ore_core::link::publicables(&pkg);
    let (nombre, version) = identidad(&pkg)?;
    sin_fuentes_ajenas(&publicables)?;

    let canonica = ore_core::normalize::package(&publicables);
    if canonica.is_empty() {
        return Err(fallo(
            65,
            "no hay ningún documento que publicar",
            &["  Un `.oob` sin documentos es un fichero que nadie puede importar."],
        ));
    }
    let digest = ore_core::digest::package(&publicables);
    let mut campos = vec![
        ("oobVersion", Json::Int(1)),
        ("package", Json::s(&nombre)),
        ("version", Json::s(&version)),
        ("oos", Json::s(oos_de(&publicables))),
        ("documents", Json::Obj(canonica.into_iter().collect())),
    ];
    if let Some(id) = firmar {
        let (json, firma) = firmas(id, &nombre, &version, &digest)?;
        campos.push(("signatures", json));
        // Anotar exige haber firmado, y no es una limitacion tecnica: la hoja
        // del log es el enunciado Y QUIEN LO FIRMO. Sin firma, lo que quedaria
        // anotado es que un paquete existio — que no es lo que un log de
        // transparencia sirve para demostrar.
        if let Some(log) = anotar {
            let enunciado = ore_core::firma::enunciado(&nombre, &version, &digest);
            campos.push(("transparency", transparencia(log, id, &enunciado, &firma)?));
        }
    } else if anotar.is_some() {
        return Err(fallo(
            64, // EX_USAGE
            "`--log` sin `--sign`",
            &[
                "  La entrada del log es el enunciado y QUIEN LO FIRMO. Sin firma, anotar",
                "  dejaria constancia de que un paquete existio, que no es lo que un log de",
                "  transparencia sirve para demostrar.",
            ],
        ));
    }
    let sobre = Json::obj(campos);
    // JCS y no `pretty`: dos productores conformes escriben LOS MISMOS BYTES, y
    // eso es el peldaño 1. Un `.oob` no se lee a mano.
    let bytes = sobre.jcs();

    let donde = match destino {
        None => "-".to_string(),
        Some(ruta) => {
            if let Some(d) = ruta.parent().filter(|d| !d.as_os_str().is_empty()) {
                std::fs::create_dir_all(d).map_err(|e| {
                    fallo(73, format!("no se pudo crear `{}`: {e}", d.display()), &[])
                })?;
            }
            std::fs::write(ruta, &bytes).map_err(|e| {
                fallo(
                    73,
                    format!("no se pudo escribir `{}`: {e}", ruta.display()),
                    &[],
                )
            })?;
            ruta.display().to_string()
        }
    };

    let resumen = format!(
        "  ✓ {donde}\n  · {nombre} {version} · {} documentos · {} bytes\n  · {digest}\n\n\
         \x20 El digest es el del PAQUETE, no el del fichero: el mismo paquete sin\n\
         \x20 empaquetar digiere igual, así que el contenedor no cambia la identidad.\n",
        publicables.docs.len(),
        bytes.len()
    );
    Ok((bytes, resumen))
}

/// El programa que firma, y **no está aquí dentro**.
const FIRMADOR: &str = "ore-sign";

/// Delega la firma y devuelve lo que va en el sobre.
///
/// `ore` no toca una clave privada, así que no firma: construye el **enunciado**
/// —la coordenada y el digest, en forma canónica— y se lo pasa a un programa del
/// usuario. Es la misma frontera que `source add` traza para un secreto, y la
/// misma delegación que `ore lock` hace para traer.
///
/// La asimetría con verificar es el punto entero: comprobar una firma no
/// necesita nada que no esté ya en el árbol, así que eso sí vive dentro y un
/// `.oob` que no case se para sin haber confiado en nadie.
///
/// Lo que devuelve el firmador **no se cree**: se verifica aquí mismo contra la
/// clave pública que el propio firmador declara, y una firma que no case no se
/// escribe. Publicar un `.oob` con una firma rota repartiría un paquete que no
/// se puede usar, y el fallo saldría en el árbol de otro.
fn firmas(id: &str, paquete: &str, version: &str, digest: &str) -> Result<(Json, String), Fallo> {
    let enunciado = ore_core::firma::enunciado(paquete, version, digest);
    let peticion = Json::obj([("keyId", Json::s(id)), ("statement", Json::s(&enunciado))]).jcs();
    let pedir = |args: &[String]| {
        crate::lector::ejecutar(FIRMADOR, args, (args.is_empty()).then_some(&*peticion)).map_err(
            |f| Fallo {
                codigo: f.codigo,
                mensaje: f.mensaje,
                ayuda: f.ayuda,
            },
        )
    };
    let firma = pedir(&[])?.trim().to_string();
    let publica = pedir(&["--public".into(), id.into()])?.trim().to_string();

    ore_core::firma::verificar(ore_core::firma::ED25519, &publica, &firma, &enunciado).map_err(
        |e| {
            fallo(
                65, // EX_DATAERR
                format!(
                    "`{FIRMADOR}` devolvió una firma que no verifica: {}",
                    e.como_texto()
                ),
                &[
                    "  Se comprueba aquí porque publicar un `.oob` con una firma rota reparte",
                    "  un paquete que nadie puede usar, y el fallo saldría en el árbol de otro.",
                ],
            )
        },
    )?;

    let json = Json::Arr(vec![Json::obj([
        ("algorithm", Json::s(ore_core::firma::ED25519)),
        ("keyId", Json::s(id)),
        ("signature", Json::s(&firma)),
    ])]);
    Ok((json, firma))
}

/// El programa que anota en el log, y tampoco esta aqui dentro.
const LOG: &str = "ore-log";

/// Anota el enunciado firmado en un log y se trae la prueba de que esta.
///
/// # Por que se anota lo firmado y no el paquete
///
/// Porque lo que hay que poder demostrar mas tarde no es *«este paquete
/// existio»* sino *«esta clave dijo esto»*. Una firma dice de quien es un
/// paquete y no dice nada sobre si esa clave le ha dicho lo mismo a todo el
/// mundo: quien la tenga puede firmar dos `0.2.0` distintos —uno para el auditor
/// y otro para ti— y las dos firmas verifican. Ninguna comprobacion local puede
/// distinguirlas, porque el defecto no esta en lo que tienes.
///
/// Un log no lo impide. Lo que garantiza es que **no se pueda hacer en
/// privado**, y eso basta: lo que se anota, se lee.
///
/// # Y lo que devuelve tampoco se cree
///
/// La prueba se comprueba aqui mismo, contra la clave que el propio log publica.
/// Es la misma postura que con la firma: publicar un `.oob` con una prueba rota
/// repartiria un paquete que no se puede usar, y el fallo saldria en el arbol de
/// otro.
fn transparencia(log: &str, key_id: &str, enunciado: &str, firma: &str) -> Result<Json, Fallo> {
    use ore_core::transparencia as t;
    let entrada = t::entrada(enunciado, key_id, firma);
    let peticion = Json::obj([("op", Json::s("append")), ("entry", Json::s(&entrada))]).jcs();
    let pedir = |args: &[String]| {
        crate::lector::ejecutar(LOG, args, (args.is_empty()).then_some(&*peticion)).map_err(|f| {
            Fallo {
                codigo: f.codigo,
                mensaje: f.mensaje,
                ayuda: f.ayuda,
            }
        })
    };
    let bruta = pedir(&[])?;
    let publica = pedir(&["--public".into()])?.trim().to_string();

    let r = ore_core::parse::parse(&bruta)
        .map_err(|e| fallo(65, format!("`{LOG}` no devolvio JSON: {e:?}"), &[]))?;
    let campo = |k: &str| r.get(k).and_then(|(_, v)| v.as_str()).unwrap_or_default();
    let entero = |k: &str| campo(k).parse::<u64>().unwrap_or(u64::MAX);
    let camino: Vec<t::Hash> = r
        .get("inclusion")
        .map(|(_, v)| v.items())
        .unwrap_or(&[])
        .iter()
        .filter_map(|h| h.as_str().and_then(t::de_hex))
        .collect();

    let (indice, tamano) = (entero("index"), entero("treeSize"));
    let raiz = t::de_hex(campo("root")).ok_or_else(|| {
        fallo(
            65,
            format!("`{LOG}` devolvio una raiz que no es un hash"),
            &[],
        )
    })?;

    // La cabeza firmada primero: una prueba de inclusion demuestra que la hoja
    // esta en un arbol CON ESA RAIZ, y no dice de donde salio la raiz.
    // Cualquiera construye un arbol con la hoja que quiera y presenta una prueba
    // impecable de algo que ningun log ha visto.
    ore_core::firma::verificar(
        ore_core::firma::ED25519,
        &publica,
        campo("rootSignature"),
        &t::cabeza(log, tamano, &raiz),
    )
    .map_err(|e| {
        fallo(
            65,
            format!(
                "`{LOG}` devolvio una cabeza que no verifica: {}",
                e.como_texto()
            ),
            &[
                "  Sin cabeza firmada, una raiz es un numero que alguien escribio: cualquiera",
                "  construye un arbol con la hoja que quiera y prueba su inclusion en el.",
            ],
        )
    })?;
    t::inclusion(&t::hoja(entrada.as_bytes()), indice, tamano, &camino, &raiz).map_err(|e| {
        fallo(
            65,
            format!(
                "`{LOG}` devolvio una prueba que no verifica: {}",
                e.como_texto()
            ),
            &["  Se comprueba aqui: repartir una prueba rota es repartir un paquete inusable."],
        )
    })?;

    Ok(Json::Arr(vec![Json::obj([
        ("index", Json::Int(indice as i64)),
        (
            "inclusion",
            Json::Arr(camino.iter().map(|h| Json::s(t::a_hex(h))).collect()),
        ),
        ("keyId", Json::s(key_id)),
        ("logId", Json::s(log)),
        ("root", Json::s(t::a_hex(&raiz))),
        ("rootSignature", Json::s(campo("rootSignature"))),
        ("treeSize", Json::Int(tamano as i64)),
    ])]))
}

fn identidad(pkg: &Package) -> Result<(String, String), Fallo> {
    let d = pkg
        .docs
        .iter()
        .find(|d| d.kind == Kind::Package)
        .ok_or_else(|| {
            fallo(
                65,
                "no hay un `package.yaml` que diga qué paquete es este",
                &["  Sin identidad no hay coordenada, y sin coordenada nadie puede importarlo."],
            )
        })?;
    let campo = |k: &str| d.meta(k).and_then(|n| n.as_str()).map(String::from);
    match (campo("name"), campo("version")) {
        (Some(n), Some(v)) => Ok((n, v)),
        _ => Err(fallo(
            65,
            "el paquete no declara `name` o `version`",
            &["  Son la coordenada con la que otro lo importa: `01-package` §2.1."],
        )),
    }
}

/// §5 · un `Binding` dice dónde está el dato **de quien publica**, y viaja hacia
/// alguien que no tiene esa fuente. No es un error de forma: es publicar la
/// infraestructura de otro.
fn sin_fuentes_ajenas(pkg: &Package) -> Result<(), Fallo> {
    let culpables: Vec<String> = pkg
        .docs
        .iter()
        .filter(|d| d.kind == Kind::Binding)
        .filter_map(|d| d.qname())
        .collect();
    if culpables.is_empty() {
        return Ok(());
    }
    Err(fallo(
        65,
        format!(
            "hay {} binding(s) en lo que se iba a publicar",
            culpables.len()
        ),
        &[
            "  Un binding dice dónde está el dato DE QUIEN PUBLICA, y viaja hacia alguien",
            "  que no tiene esa fuente. Un paquete publicable dice qué significan las cosas;",
            "  dónde están es de cada uno.",
        ],
    ))
}

/// La mayor `apiVersion` que use alguno de sus documentos, **derivada**. Es lo
/// que permite a un consumidor rechazar un paquete del futuro sin abrirlo.
fn oos_de(pkg: &Package) -> String {
    pkg.docs
        .iter()
        .filter_map(|d| d.root.get("apiVersion").and_then(|(_, v)| v.as_str()))
        .max_by_key(|v| {
            v.rsplit("v1alpha")
                .next()
                .and_then(|n| n.parse::<u32>().ok())
                .unwrap_or(0)
        })
        .unwrap_or("oos.dev/v1alpha1")
        .to_string()
}
