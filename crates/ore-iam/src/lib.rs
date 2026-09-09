//! `ore-iam` — **el plano de identidad y acceso de la plataforma**.
//!
//! # Por qué no vive dentro de `ore-serve`
//!
//! Porque `ore-serve` es **un consumidor** del plano de identidad: verifica un
//! token y pregunta quién es. Colgarle la administración haría que el plano
//! dependiera de uno de sus consumidores — y esa regla no la inventamos aquí,
//! la escribió la plataforma antes de tener este código:
//!
//! > *«la consola y la API de administración NO viven en `api/`, porque `api/`
//! > es un CONSUMIDOR, y colgarle la administración haría que la superficie
//! > dependiera de uno de sus consumidores.»*
//!
//! Y hay una razón medida que pesa más que la cita: **`ore-serve` se despliega
//! por inquilino** —vive en `t-demo`, con la forja de `t-demo`— y
//! `iam.organizacion` es el censo de **todos**. Un pod de un inquilino dueño
//! del censo de los demás no es una capa mal puesta: es la pertenencia de otras
//! organizaciones escribible desde el plano de un cliente.
//!
//! # Las dos caras, y por qué no son la misma
//!
//! ```text
//!   CLI    un operador, con las credenciales del clúster     `fundar`
//!   HTTP   una persona, con un token del realm               todo lo demás
//! ```
//!
//! `fundar` no puede ser una petición autenticada: la primera organización se
//! crea cuando todavía no existe ninguna persona con potestad, así que no
//! habría con qué autenticarla. Es la misma figura que `ore init`.
//!
//! ⛔ **Y la cara de línea de órdenes no debe crecer.** El día que
//! `ore-iam conceder` exista por `argv`, existirá una forma de conceder que no
//! pasa por la identidad de nadie.
//!
//! # Y lo que este binario no hace, todavía
//!
//! `invitar`, `admitir`, `conceder` y `revocar`. Se dice en vez de dejar el
//! hueco callado.

// ⭐⭐ ES UNA BIBLIOTECA ADEMAS DE UN BINARIO, Y NO POR GUSTO.
//
// `ore-cofre` —el custodio— tiene que decidir si alguien puede abrir un secreto,
// y esa pregunta ya esta contestada aqui: `potestad::exige`, `contenidas_en`, la
// union de conjuntos de la `016`, y el hecho de que «no perteneces» y «no
// puedes» den el MISMO mensaje.
//
// ⛔ Si el custodio se escribiera su propia version, tendriamos DOS motores de
//   autorizacion contestando la misma pregunta — que es exactamente lo que la
//   `0023` rechaza de Vault, y seria peor hacerlo nosotros mismos por descuido.
//
// ⇒ Por eso esto se expone, y `main.rs` es una llamada de cinco lineas.
pub mod base;
pub mod fundar;
pub mod id;
pub mod potestad;
pub mod rutas;
pub mod verbos;

use ore_entrada::{http, identidad};
use std::net::TcpListener;
use std::process::ExitCode;
use std::sync::Mutex;

const USO: &str = "\
ore-iam — el plano de identidad y acceso

  ore-iam fundar --organizacion NOMBRE --emisor URL --sub SUB [--correo C] [--arbol P/R]
  ore-iam servir [--bind DIRECCION] [--identidad MODO] …

  `--arbol` es COMO SE LLAMA su arbol —`<propietario>/<repositorio>`—, no donde
  vive ni si existe. Por defecto `t-<organizacion>/ontologia`. Fundar lo declara;
  crear el repositorio es otro acto y lo hace quien puede salir a la red.

  La base sale de `IAM_URL`. No hay valor por defecto: una cadena de conexión
  por defecto es apuntar a una base que nadie eligió.
";

fn valor(args: &[String], que: &str) -> Option<String> {
    args.iter()
        .position(|a| a == que)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

pub fn arrancar(args: Vec<String>) -> ExitCode {
    let Some(mando) = args.first().map(String::as_str) else {
        print!("{USO}");
        return ExitCode::from(64);
    };

    let url = match std::env::var("IAM_URL") {
        Ok(u) if !u.is_empty() => u,
        _ => {
            eprintln!("✗ falta `IAM_URL`. Sin ella este proceso no sabe qué base administra,");
            eprintln!("  y un valor por defecto sería apuntar a una que nadie eligió.");
            return ExitCode::from(64);
        }
    };

    match mando {
        "fundar" => fundar_mando(&args, &url),
        "servir" => servir_mando(&args, &url),
        "-h" | "--help" => {
            print!("{USO}");
            ExitCode::SUCCESS
        }
        otro => {
            eprintln!("✗ mando desconocido: `{otro}`\n\n{USO}");
            ExitCode::from(64)
        }
    }
}

fn fundar_mando(args: &[String], url: &str) -> ExitCode {
    let (Some(org), Some(emisor), Some(sub)) = (
        valor(args, "--organizacion"),
        valor(args, "--emisor"),
        valor(args, "--sub"),
    ) else {
        eprintln!("✗ `fundar` necesita `--organizacion`, `--emisor` y `--sub`.");
        eprintln!("  El `sub` es el del emisor: es a quién se le entrega la organización,");
        eprintln!("  y sin él no hay dueño — una organización sin dueño no la administra nadie.");
        return ExitCode::from(64);
    };
    let correo = valor(args, "--correo");
    // ⭐ Opcional: sin el se deriva `t-<organizacion>/ontologia`. Existe para el
    //   dia que una organizacion traiga SU repositorio — la derivacion no tendria
    //   donde ponerlo, y un valor por defecto no es una imposicion.
    let arbol = valor(args, "--arbol");

    let mut c = match base::conectar(url) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("✗ {e}");
            return ExitCode::from(69); // EX_UNAVAILABLE
        }
    };
    match fundar::fundar(
        &mut c,
        &fundar::Peticion {
            organizacion: &org,
            emisor: &emisor,
            sub: &sub,
            correo: correo.as_deref(),
            arbol: arbol.as_deref(),
        },
    ) {
        Ok(j) => {
            println!("{}", j.pretty());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("✗ {e}");
            ExitCode::from(65) // EX_DATAERR
        }
    }
}

fn servir_mando(args: &[String], url: &str) -> ExitCode {
    let bind = valor(args, "--bind").unwrap_or_else(|| "127.0.0.1:8090".into());
    let jwks = valor(args, "--jwks").map(std::path::PathBuf::from);
    let ajustes = identidad::Ajustes {
        modo: valor(args, "--identidad")
            .as_deref()
            .map(|s| Box::leak(s.to_string().into_boxed_str()) as &str),
        no_es_produccion: args.iter().any(|a| a == "--no-es-produccion"),
        emisor: valor(args, "--emisor").map(|s| Box::leak(s.into_boxed_str()) as &str),
        audiencia: valor(args, "--audiencia").map(|s| Box::leak(s.into_boxed_str()) as &str),
        jwks: jwks.as_deref(),
    };
    let (proveedor, dicho) = match identidad::resolver(&ajustes) {
        Ok(Some((p, d))) => (Some(p), d),
        Ok(None) => (None, "sin configurar".to_string()),
        Err(m) => {
            eprintln!("✗ {m}");
            return ExitCode::from(64);
        }
    };
    let con_identidad = proveedor.is_some();

    let base = match base::conectar(url) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("✗ {e}");
            return ExitCode::from(69);
        }
    };
    let escucha = match TcpListener::bind(&bind) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("✗ no se pudo escuchar en `{bind}`: {e}");
            return ExitCode::from(73);
        }
    };

    eprintln!("ore-iam · {bind}");
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
        eprintln!("  Una superficie de administración sin identidad no es una superficie:");
        eprintln!("  es una puerta abierta.");
    }
    eprintln!();

    let servidor = rutas::Servidor {
        base: Mutex::new(base),
        // El emisor va al servidor porque la identidad de una persona es
        // `(emisor, sub)`. Sin `--emisor` no hay contra que resolverla.
        emisor: valor(args, "--emisor").unwrap_or_default(),
        identidad: proveedor,
    };
    match http::servir(escucha, move |p| servidor.atender(p)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("✗ el servidor terminó: {e}");
            ExitCode::from(70)
        }
    }
}
