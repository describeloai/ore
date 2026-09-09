//! `ore-cofre` — **el custodio**. Guarda el material cifrado y lo abre a quien
//! puede.
//!
//! # Por qué no vive dentro de `ore-iam`
//!
//! Porque no cabe, literalmente. `ore-iam` sale de `scratch`: sin shell, sin
//! certificados, sin cliente HTTP. Un KMS se habla por HTTPS, así que **no puede
//! abrir un secreto aunque tuviera permiso** — y no es la red quien lo impide,
//! es la imagen.
//!
//! ⭐⭐ Y eso no es un problema que resolver: es la propiedad. **El que decide
//! quién puede no es el que puede abrir.** `ore-iam` autoriza y es físicamente
//! incapaz de descifrar; este proceso descifra y no decide nada — la decisión la
//! toma el código de `ore-iam`, que enlaza como biblioteca.
//!
//! ⚠️ La tentación, cuando llegue la prisa, será darle a `ore-iam` un cliente
//! HTTPS «para no tener otro binario». Ese día la propiedad se pierde **con una
//! línea en un `Cargo.toml`**, no con un error.
//!
//! # Y es la quinta vez que este árbol reparte lo mismo
//!
//! ```text
//!   leer un origen         ore-read-<tipo>,  no `ore`
//!   subir un artefacto     ore-store-<tipo>, no `ore`
//!   atender a un cliente   ore-serve,        no `ore`
//!   traer el JWKS          50-jwks,          no `ore-serve`
//!   ABRIR UN SECRETO       esto,             no `ore-iam`
//! ```
//!
//! # Por qué va POR INQUILINO
//!
//! Porque si una sola pieza tuviera a la vez el material y el uso de la llave,
//! abriría todo lo que alcanzase — autorizara quien autorizara. La separación no
//! puede ser de responsabilidades: es de **alcance**. Una KEK por organización
//! (`019`) y un custodio por inquilino, con su cuenta de Google atada a SU
//! clave. Así la tabla del material puede ser compartida: el custodio de un
//! inquilino puede LEER el cifrado de otro y no puede abrirlo.
//!
//! ⇒ Y el segundo cerrojo no lo ponemos nosotros: lo pone el IAM de la nube
//! sobre una llave. Está probado en `malla/99-el-cerrojo-de-la-llave.yaml`.

mod kms;
mod rutas;

use ore_entrada::{http, identidad};
use ore_iam::base;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Mutex;

const USO: &str = "\
ore-cofre — el custodio: guarda el material cifrado y lo abre a quien puede

  ore-cofre servir [--bind DIRECCION] --identidad MODO --emisor URL
                   --audiencia AUD --jwks FICHERO
                   --kms PROGRAMA --lugar REGION

  La base sale de `COFRE_URL`. No hay valor por defecto: una cadena de conexión
  por defecto es apuntar a una base que nadie eligió.

  ⛔ Y ese usuario NO es el de `ore-iam`. La `020` reparte los permisos entre dos
  papeles: quien dice quién puede no alcanza el material, y quien lo abre sólo
  lee de `iam` lo justo para autorizar.
";

fn valor(args: &[String], que: &str) -> Option<String> {
    args.iter()
        .position(|a| a == que)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) != Some("servir") {
        print!("{USO}");
        return ExitCode::from(64);
    }

    let url = match std::env::var("COFRE_URL") {
        Ok(u) if !u.is_empty() => u,
        _ => {
            eprintln!("✗ falta `COFRE_URL`.");
            eprintln!("  Sin ella este proceso no sabe de qué base guarda el material, y un");
            eprintln!("  valor por defecto apuntaría a una que nadie eligió.");
            return ExitCode::from(64);
        }
    };

    // ⛔ Sin cliente no hay custodio. Se dice al arrancar y no en la primera
    //   petición: un proceso que acepta conexiones y no puede hacer su trabajo
    //   es peor que uno que no arranca.
    let Some(programa) = valor(&args, "--kms").map(PathBuf::from) else {
        eprintln!("✗ falta `--kms`, el cliente de la nube.");
        eprintln!("  Este programa no habla con el KMS: habla con él. Y es una RUTA y no un");
        eprintln!("  nombre para que un despliegue no dependa del `PATH` del contenedor.");
        return ExitCode::from(64);
    };
    let Some(lugar) = valor(&args, "--lugar") else {
        eprintln!("✗ falta `--lugar`, la región del llavero.");
        eprintln!("  `iam.organizacion.kek` guarda el NOMBRE de la llave; de dónde se");
        eprintln!("  alcanza es configuración del despliegue, y cambia sin que nadie mienta.");
        return ExitCode::from(64);
    };

    let bind = valor(&args, "--bind").unwrap_or_else(|| "127.0.0.1:8095".into());
    let jwks = valor(&args, "--jwks").map(PathBuf::from);
    let ajustes = identidad::Ajustes {
        modo: valor(&args, "--identidad")
            .as_deref()
            .map(|s| Box::leak(s.to_string().into_boxed_str()) as &str),
        no_es_produccion: args.iter().any(|a| a == "--no-es-produccion"),
        emisor: valor(&args, "--emisor").map(|s| Box::leak(s.into_boxed_str()) as &str),
        audiencia: valor(&args, "--audiencia").map(|s| Box::leak(s.into_boxed_str()) as &str),
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

    let base = match base::conectar(&url) {
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

    eprintln!("ore-cofre · {bind}");
    eprintln!("  identidad    {dicho}");
    eprintln!("  cliente      {}", kms::ruta_de(&programa));
    eprintln!("  lugar        {lugar}");
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
        eprintln!("  Las rutas del cofre NO están montadas: no hay proveedor de identidad.");
        eprintln!("  Un almacén de secretos sin identidad no es un almacén: es un cajón.");
    }
    eprintln!();

    let servidor = rutas::Servidor {
        base: Mutex::new(base),
        emisor: valor(&args, "--emisor").unwrap_or_default(),
        identidad: proveedor,
        kms: kms::Kms { programa, lugar },
    };
    match http::servir(escucha, move |p| servidor.atender(p)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("✗ el servidor terminó: {e}");
            ExitCode::from(70)
        }
    }
}
