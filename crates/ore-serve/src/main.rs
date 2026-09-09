//! `ore-serve` — **el plano de control**, fuera del compilador.
//!
//! # Por qué esto es un binario aparte, y no `ore serve`
//!
//! Es la **cuarta vez** que este árbol delega, y por la misma razón que las tres
//! anteriores: `ore` no puede abrir un socket, y no por promesa —
//! `ore-cli/tests/dependencias.rs` lee el `Cargo.lock` y falla si aparece una
//! crate de red, de TLS o de FFI en su cierre—.
//!
//! | | qué delega | ADR |
//! |---|---|---|
//! | `ore-read-<tipo>` | leer filas de un origen | 0008 |
//! | `ore-maintain` | correr el circuito Δ | 0013 |
//! | `ore-store-<tipo>` | sellar y subir el artefacto | 0015 |
//! | **`ore-serve`** | **atender a un cliente** | **0020** |
//!
//! Y aquí la delegación compra algo que en las otras tres era un efecto
//! secundario: **el proceso que mira a internet no es el que decide qué
//! significan las cosas**.
//!
//! # Lo que este proceso puede hacer, y lo que no puede aunque quiera
//!
//! Corre los verbos herméticos de `ore` —los que contestan desde el árbol de
//! ficheros— y **no puede** correr los que hablan con un origen. No porque haya
//! una regla que lo diga: porque el binario `ore` no lleva cliente TLS, y la
//! lista de [`mando::HERMETICOS`] es una segunda cerradura sobre una puerta que
//! ya estaba tapiada.
//!
//! ⇒ Este servidor **no necesita la credencial de ningún origen**. Lo que toca
//! el mundo se va a un Job con la imagen de drivers, su identidad y su cuota.
//!
//! # Los dos defectos, que son los dos negativos
//!
//! - **sin `--identidad`, las rutas de datos no se montan.** El defecto no es
//!   «cualquiera»: es «no hay identidad» ⇒ no hay superficie;
//! - **escucha en `127.0.0.1` si nadie dice otra cosa.** Un servidor que se ata
//!   al mundo por defecto es un servidor que se expuso por omisión.
//!
//! Los dos son la misma frase que el resto del proyecto: *omitir no deja nada
//! abierto, lo CIERRA*.

mod git;
mod mando;
mod rutas;

use ore_entrada::{http, identidad};

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::ExitCode;

const USO: &str = "\
ore-serve — el plano de control de ORE

  ore-serve [--repo DIR | --forja URL] [--bind DIRECCION] [--ore RUTA]
            [--identidad MODO] [--no-es-produccion]

  --repo DIR             la raíz del repositorio ontológico (por defecto, `.`)
  --forja URL            el repositorio en la forja. Cada petición CLONA, y la
                         que escribe empuja
  --testigo-fichero R    de dónde leer el testigo de la forja. Un FICHERO, y por
                         la misma razón que `--jwks`: en un clúster, una variable
                         de entorno con un secreto viene de un `Secret`, y un
                         `Secret` vive en etcd. Si no se dice, `FORJA_TOKEN`
  --bind DIRECCION       dónde escuchar (por defecto, 127.0.0.1:8080)
  --ore RUTA             el binario `ore` (por defecto, `ore` del PATH)
  --identidad MODO       de dónde sale el sujeto. Sin esto, las rutas de datos
                         NO SE MONTAN. Modos: cabecera, oidc
  --no-es-produccion     segundo interruptor del modo de banco
  --emisor URL           `oidc`: el emisor esperado, `https://…/realms/<realm>`
  --audiencia NOMBRE     `oidc`: NUESTRA audiencia. Un token del mismo realm
                         para otro servicio no vale aquí
  --jwks FICHERO         `oidc`: el juego de llaves. Un FICHERO, no una URL:
                         este proceso no va a buscarlas — ver `oidc.rs`
  -h, --help             esto
";

struct Opciones {
    repo: PathBuf,
    forja: Option<String>,
    bind: String,
    ore: PathBuf,
    identidad: Option<String>,
    no_es_produccion: bool,
    emisor: Option<String>,
    audiencia: Option<String>,
    jwks: Option<PathBuf>,
    /// Dónde leer el testigo de la forja. Un FICHERO, como `--jwks` y por la
    /// misma razón: un valor en `argv` lo lee cualquier proceso, y un valor en
    /// un `Secret` de Kubernetes vive en etcd.
    testigo_fichero: Option<PathBuf>,
}

fn leer_opciones() -> Result<Option<Opciones>, String> {
    let mut o = Opciones {
        repo: PathBuf::from("."),
        forja: None,
        bind: "127.0.0.1:8080".into(),
        ore: PathBuf::from("ore"),
        identidad: None,
        no_es_produccion: false,
        emisor: None,
        audiencia: None,
        jwks: None,
        testigo_fichero: None,
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut valor = |que: &str| -> Result<String, String> {
            args.next()
                .ok_or_else(|| format!("`{que}` necesita un valor"))
        };
        match a.as_str() {
            "-h" | "--help" => return Ok(None),
            "--repo" => o.repo = PathBuf::from(valor("--repo")?),
            "--forja" => o.forja = Some(valor("--forja")?),
            "--testigo-fichero" => {
                o.testigo_fichero = Some(PathBuf::from(valor("--testigo-fichero")?))
            }
            "--bind" => o.bind = valor("--bind")?,
            "--ore" => o.ore = PathBuf::from(valor("--ore")?),
            "--identidad" => o.identidad = Some(valor("--identidad")?),
            "--no-es-produccion" => o.no_es_produccion = true,
            "--emisor" => o.emisor = Some(valor("--emisor")?),
            "--audiencia" => o.audiencia = Some(valor("--audiencia")?),
            "--jwks" => o.jwks = Some(PathBuf::from(valor("--jwks")?)),
            otro => return Err(format!("opción desconocida: `{otro}`")),
        }
    }
    Ok(Some(o))
}

fn main() -> ExitCode {
    let o = match leer_opciones() {
        Ok(None) => {
            print!("{USO}");
            return ExitCode::SUCCESS;
        }
        Ok(Some(o)) => o,
        Err(m) => {
            eprintln!("✗ {m}\n\n{USO}");
            return ExitCode::from(64); // EX_USAGE
        }
    };

    let proveedor = match identidad::resolver(&identidad::Ajustes {
        modo: o.identidad.as_deref(),
        no_es_produccion: o.no_es_produccion,
        emisor: o.emisor.as_deref(),
        audiencia: o.audiencia.as_deref(),
        jwks: o.jwks.as_deref(),
    }) {
        Ok(p) => p,
        Err(m) => {
            eprintln!("✗ {m}");
            return ExitCode::from(64);
        }
    };
    // `resolver` devuelve el proveedor **y una linea que lo describe**. La
    // imprime aqui y no alli: una biblioteca que escribe en `stderr` decide por
    // su consumidor donde va su salida, y eso no es suyo.
    let (proveedor, dicho) = match proveedor {
        Some((p, d)) => (Some(p), d),
        None => (None, "sin configurar".to_string()),
    };
    let con_identidad = proveedor.is_some();

    // El árbol: o un directorio, o la forja. **Nunca los dos** — un servidor
    // que tuviera dos sitios donde vive el árbol tendría dos verdades.
    let arbol = match &o.forja {
        Some(url) => match testigo(&o) {
            Some(t) => rutas::Arbol::Forja(git::Forja {
                url: url.clone(),
                testigo: t,
            }),
            None => {
                eprintln!(
                    "✗ `--forja` necesita el testigo, y hay dos formas de darlo:
                       `--testigo-fichero RUTA`  ← preferida: el valor no pasa por etcd
                       `FORJA_TOKEN`             ← el entorno, que sigue valiendo
                     Por `argv` NO se acepta: lo lee cualquier proceso de la maquina."
                );
                return ExitCode::from(64);
            }
        },
        None => {
            if !o.repo.is_dir() {
                eprintln!("✗ `{}` no es un directorio", o.repo.display());
                return ExitCode::from(66); // EX_NOINPUT
            }
            rutas::Arbol::Directorio(o.repo.clone())
        }
    };

    let escucha = match TcpListener::bind(&o.bind) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("✗ no se pudo escuchar en `{}`: {e}", o.bind);
            return ExitCode::from(73); // EX_CANTCREAT
        }
    };

    eprintln!("ore-serve · {}", o.bind);
    eprintln!(
        "  arbol        {}",
        match &o.forja {
            Some(u) => format!("forja · {u}"),
            None => rutas::ruta_de(&o.repo),
        }
    );
    eprintln!("  motor        {}", rutas::ruta_de(&o.ore));
    eprintln!("  identidad    {dicho}");
    eprintln!();
    for (metodo, ruta, montada) in rutas::mapa(con_identidad) {
        eprintln!(
            "  {}  {:-6} {}",
            if montada { "·" } else { "✗" },
            metodo,
            ruta
        );
    }
    if !con_identidad {
        eprintln!();
        eprintln!("  Las rutas de datos NO están montadas: no hay proveedor de identidad.");
        eprintln!("  No es un fallo — es el defecto. Sin sujeto no hay superficie.");
    }
    eprintln!();

    let servidor = rutas::Servidor {
        binario: o.ore,
        arbol,
        identidad: proveedor,
    };

    match http::servir(escucha, move |p| servidor.atender(p)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("✗ el servidor terminó: {e}");
            ExitCode::from(70) // EX_SOFTWARE
        }
    }
}

/// El testigo de la forja: de un FICHERO si se dijo dónde, y si no del entorno.
///
/// ── ⭐⭐ Por qué un fichero, y por qué ganó al entorno ───────────────────────
///
/// El entorno ya era mejor que `argv` —que lo lee cualquier proceso de la
/// máquina— pero en Kubernetes una variable de entorno con un secreto dentro
/// viene de un `Secret`, y **un `Secret` vive en etcd**: en el disco del plano
/// de control, en sus copias, y al alcance de cualquiera que pueda leer
/// `Secret` en ese namespace.
///
/// ⇒ Con un fichero, el valor puede venir de un `emptyDir` de MEMORIA que
/// rellena un contenedor de inicio desde el almacén de la plataforma. No toca
/// etcd, no toca el disco, y muere con el pod.
///
/// ⭐ Y es exactamente la forma que este binario ya eligió para `--jwks`: *«un
/// FICHERO, no una URL: este proceso no va a buscar las llaves»*. Aquí igual —
/// no va a buscar el testigo: se lo dejan puesto.
///
/// ⚠️ El entorno sigue valiendo, y no por compatibilidad: en una máquina, sin
/// clúster y sin almacén, es la forma sensata. Lo que NO se acepta sigue siendo
/// `argv`.
fn testigo(o: &Opciones) -> Option<String> {
    if let Some(f) = &o.testigo_fichero {
        return match std::fs::read_to_string(f) {
            // ⛔ `trim`: un fichero escrito con `echo` acaba en `\n`, y un
            //   testigo con un salto de línea al final falla en la forja con un
            //   401 que no dice nada de saltos de línea.
            Ok(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
            Ok(_) => {
                eprintln!("✗ `{}` está vacío", rutas::ruta_de(f));
                None
            }
            Err(e) => {
                eprintln!("✗ no se pudo leer `{}`: {e}", rutas::ruta_de(f));
                None
            }
        };
    }
    match std::env::var("FORJA_TOKEN") {
        Ok(t) if !t.is_empty() => Some(t),
        _ => None,
    }
}
