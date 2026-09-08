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
mod http;
mod identidad;
mod mando;
mod rutas;

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::ExitCode;

const USO: &str = "\
ore-serve — el plano de control de ORE

  ore-serve [--repo DIR | --forja URL] [--bind DIRECCION] [--ore RUTA]
            [--identidad MODO] [--no-es-produccion]

  --repo DIR             la raíz del repositorio ontológico (por defecto, `.`)
  --forja URL            el repositorio en la forja. Cada petición CLONA, y la
                         que escribe empuja. El testigo sale de `FORJA_TOKEN`
  --bind DIRECCION       dónde escuchar (por defecto, 127.0.0.1:8080)
  --ore RUTA             el binario `ore` (por defecto, `ore` del PATH)
  --identidad MODO       de dónde sale el sujeto. Sin esto, las rutas de datos
                         NO SE MONTAN. Modos: cabecera
  --no-es-produccion     segundo interruptor del modo de banco
  -h, --help             esto
";

struct Opciones {
    repo: PathBuf,
    forja: Option<String>,
    bind: String,
    ore: PathBuf,
    identidad: Option<String>,
    no_es_produccion: bool,
}

fn leer_opciones() -> Result<Option<Opciones>, String> {
    let mut o = Opciones {
        repo: PathBuf::from("."),
        forja: None,
        bind: "127.0.0.1:8080".into(),
        ore: PathBuf::from("ore"),
        identidad: None,
        no_es_produccion: false,
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
            "--bind" => o.bind = valor("--bind")?,
            "--ore" => o.ore = PathBuf::from(valor("--ore")?),
            "--identidad" => o.identidad = Some(valor("--identidad")?),
            "--no-es-produccion" => o.no_es_produccion = true,
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

    let proveedor = match identidad::resolver(o.identidad.as_deref(), o.no_es_produccion) {
        Ok(p) => p,
        Err(m) => {
            eprintln!("✗ {m}");
            return ExitCode::from(64);
        }
    };
    let con_identidad = proveedor.is_some();

    // El árbol: o un directorio, o la forja. **Nunca los dos** — un servidor
    // que tuviera dos sitios donde vive el árbol tendría dos verdades.
    let arbol = match &o.forja {
        Some(url) => match std::env::var("FORJA_TOKEN") {
            Ok(t) if !t.is_empty() => rutas::Arbol::Forja(git::Forja {
                url: url.clone(),
                testigo: t,
            }),
            _ => {
                eprintln!(
                    "✗ `--forja` necesita el testigo en `FORJA_TOKEN`.
                       No se acepta por la línea de órdenes: `argv` lo lee cualquier proceso."
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
    eprintln!(
        "  identidad    {}",
        match (&o.identidad, o.no_es_produccion) {
            (Some(m), true) => format!("{m}  ⚠️  MODO DE BANCO: el sujeto lo escribe quien llama"),
            (Some(m), false) => m.clone(),
            (None, _) => "sin configurar".into(),
        }
    );
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
