//! El lector: de una fuente viva a un **catálogo**.
//!
//! # El compilador no habla con la nube, y eso no es una promesa
//!
//! `main.rs` rotula la sección del compilador *«CI · hermético: sin red, sin
//! credenciales, sin reloj»*. Esa línea puede significar dos cosas muy distintas,
//! y la diferencia es exactamente la que este proyecto lleva persiguiendo:
//!
//! - **Una política**: el binario sabe hablar por la red y se abstiene.
//! - **Una propiedad**: el binario **no sabe** hablar por la red.
//!
//! Lo segundo se comprueba mirando el árbol de dependencias; lo primero solo se
//! puede creer. Y la herméticidad no es una propiedad de un subcomando: es del
//! **artefacto**. Una pila TLS enlazada para `discover` está igual de presente
//! en `compile`.
//!
//! Medido, no supuesto. `ore` hoy enlaza **28 crates**, ninguna nativa. Un
//! cliente HTTPS mínimo —`reqwest` a secas, sin OAuth, sin el modelo REST de
//! BigQuery, sin un segundo driver— son **91**, cinco de ellas cripto o FFI.
//! Triplicar el árbol para que `discover` llame a una API le quitaría a `compile`
//! la única afirmación que podía demostrar.
//!
//! Y desde que existe `ore-read-postgres` esto no se sostiene sobre la buena
//! voluntad: `tests/dependencias.rs` lee el cierre de `ore-cli` en `Cargo.lock` y
//! falla si aparece una crate de red, de TLS o de FFI. La primera vez que corrió
//! corrigió la cifra que había escrita aquí, que era otra.
//!
//! # Cómo habla entonces: delegando
//!
//! ORE **no** abre un socket: ejecuta un lector —`ore-read-<tipo>`, uno por
//! familia— y lee su salida. Tres consecuencias, y las tres son buenas:
//!
//! 1. **La credencial nunca entra en el espacio de direcciones de `ore`.** La
//!    resuelve el lector: `ore-read-postgres` la lleva en la URL,
//!    `ore-read-bigquery` usa el token de la cuenta que corre.
//! 2. **El sistema de tipos de la fuente vive del otro lado de la costura.** El
//!    inductor recibe tipos de OOS; nunca ve un `NUMERIC`.
//! 3. **Añadir una fuente no añade una dependencia a `ore`**: añade un lector
//!    en el PATH.
//!
//! Hasta A3 de BigQuery (2026-09-26) había una excepción: la receta del
//! catálogo de BigQuery vivía aquí y ejecutaba el CLI `bq`. Se mudó al verbo
//! `catalogo` de `ore-read-bigquery` cuando ese driver pasó a hablar REST, y con
//! ella se fueron sus pruebas.
//!
//! # El lector es un programa ajeno, y puede fallar
//!
//! Puede faltar, no estar autenticado, o estar roto de formas que no son culpa de
//! nadie aquí. Su stderr es lo único accionable que existe, así que **se
//! muestra literal**. Un lector que dijera «no se pudo leer la fuente»
//! convertiría un problema de cinco minutos en una tarde.
//!
//! Y en Windows un programa puede ser un `.cmd`: se resuelven contra `PATH` y
//! `PATHEXT`, porque `CreateProcess` no lo hace por su cuenta.

use ore_core::json::Json;
use ore_core::parse;
use ore_driver::catalogo::Catalogo;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const MANIFIESTO: &str = "ontology.config.yaml";
const SECRETOS: &str = ".env.local";

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

/// Lee el catálogo de una fuente declarada en el manifiesto y lo devuelve en el
/// mismo JSON que acepta `--from`. Que sean el mismo texto no es comodidad: es lo
/// que permite probar la costura por los dos lados.
pub fn catalogo(raiz: &Path, fuente: &str) -> Result<String, Fallo> {
    let (tipo, env) = declaracion(raiz, fuente)?;
    let url = url(raiz, &env, fuente)?;
    // Sin excepciones desde A3 de BigQuery: su receta vivía aquí y ejecutaba
    // `bq`; ahora es el verbo `catalogo` de `ore-read-bigquery`, como el de
    // cualquier otra familia.
    externo(&tipo, fuente, &url)
}

// ── El manifiesto y el secreto ──────────────────────────────────────────────

pub fn declaracion(raiz: &Path, fuente: &str) -> Result<(String, String), Fallo> {
    let ruta = raiz.join(MANIFIESTO);
    let texto = std::fs::read_to_string(&ruta).map_err(|e| {
        fallo(
            66, // EX_NOINPUT
            format!("no se pudo leer `{}`: {e}", ruta.display()),
            &["  `ore init` crea uno."],
        )
    })?;
    let arbol = parse::parse(&texto)
        .map_err(|e| fallo(65, format!("`{MANIFIESTO}` no analiza: {e:?}"), &[]))?;

    let ds = arbol
        .get("datasources")
        .map(|(_, v)| v.items())
        .unwrap_or(&[]);
    let Some(d) = ds
        .iter()
        .find(|it| it.get("name").and_then(|(_, v)| v.as_str()) == Some(fuente))
    else {
        let nombres: Vec<&str> = ds
            .iter()
            .filter_map(|it| it.get("name").and_then(|(_, v)| v.as_str()))
            .collect();
        let ayuda = if nombres.is_empty() {
            "  No hay ninguna declarada. `ore source add --name <n> <url>`.".to_string()
        } else {
            format!("  Declaradas: {}", nombres.join(", "))
        };
        return Err(Fallo {
            codigo: 65,
            mensaje: format!("`{fuente}` no está declarada en `{MANIFIESTO}`"),
            ayuda: vec![ayuda],
        });
    };

    let campo = |k: &str| d.get(k).and_then(|(_, v)| v.as_str()).map(String::from);
    let tipo = campo("type").ok_or_else(|| {
        fallo(
            65,
            format!("la fuente `{fuente}` no declara `type`"),
            &["  Sin tipo no hay receta que aplicar, y adivinarla sería inventarla."],
        )
    })?;
    let env = campo("connectionEnv").ok_or_else(|| {
        fallo(
            65,
            format!("la fuente `{fuente}` no declara `connectionEnv`"),
            &["  Es el campo que dice DÓNDE está la conexión. Sin él no hay dónde mirar."],
        )
    })?;
    Ok((tipo, env))
}

/// El entorno del proceso manda; `.env.local` es el respaldo local.
///
/// Que ORE lea `.env.local` no es comodidad: `source add` lo **escribe**, y un
/// fichero que se escribe y nadie lee es la misma figura que este proyecto lleva
/// encontrando una y otra vez. En CI no existe, y ahí manda el entorno.
pub fn url(raiz: &Path, env: &str, fuente: &str) -> Result<String, Fallo> {
    if let Ok(v) = std::env::var(env)
        && !v.trim().is_empty()
    {
        return Ok(v.trim().to_string());
    }
    if let Ok(texto) = std::fs::read_to_string(raiz.join(SECRETOS)) {
        for linea in texto.lines() {
            let l = linea.trim();
            let l = l.strip_prefix("export ").unwrap_or(l);
            if l.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = l.split_once('=')
                && k.trim() == env
            {
                let v = v.trim().trim_matches('"').trim_matches('\'');
                if !v.is_empty() {
                    return Ok(v.to_string());
                }
            }
        }
    }
    Err(Fallo {
        codigo: 69, // EX_UNAVAILABLE
        mensaje: format!("`{env}` no está definida"),
        ayuda: vec![
            format!("  La declara la fuente `{fuente}`, y no se inventa."),
            "  Defínela en el entorno, o en `.env.local` del repositorio.".to_string(),
        ],
    })
}

// ── La costura de extensión ─────────────────────────────────────────────────

/// Cada tipo se busca como `ore-read-<tipo>` en el `PATH`, al modo de los
/// subcomandos de `git` o `cargo`. Desde A3 de BigQuery no hay excepciones.
///
/// La URL viaja por **stdin**, nunca por la línea de órdenes: `argv` es legible
/// por cualquier proceso de la máquina, y para casi todo lo que no sea BigQuery
/// la URL lleva la credencial dentro.
fn externo(tipo: &str, fuente: &str, url: &str) -> Result<String, Fallo> {
    let programa = format!("ore-read-{tipo}");
    if resolver(&programa).is_none() {
        return Err(fallo(
            69,
            format!("no hay lector para una fuente de tipo `{tipo}`"),
            &[
                "  ORE no lleva lectores dentro: delega. Pon un `ore-read-<tipo>`",
                "  en el PATH que lea la URL por stdin y",
                "  escriba un catálogo por stdout, o pásale uno hecho con `--from`.",
                "  No se inventa un lector, igual que no se inventa un tipo.",
            ],
        ));
    }
    // El verbo, explicito. Desde que el driver tiene dos —`catalogo` y `leer`—
    // deducirlo del contenido de stdin seria adivinar
    // (`docs/decisions/0008-el-protocolo-del-driver.md`).
    let salida = ejecutar(
        &programa,
        &["catalogo".to_string(), fuente.to_string()],
        Some(url),
    )?;
    // Se comprueba que analiza aquí para que el error diga QUIÉN lo produjo.
    parse::parse(&salida).map_err(|e| {
        fallo(
            65,
            format!("lo que devolvió `{programa}` no analiza: {e:?}"),
            &["  Un lector externo escribe un catálogo JSON por stdout."],
        )
    })?;
    Ok(salida)
}

// ── Ejecutar un programa ajeno ──────────────────────────────────────────────

/// `CreateProcess` no consulta `PATHEXT`, así que hay que resolver a mano. Y no
/// solo por Windows: saber qué fichero exacto se va a ejecutar es lo que permite
/// nombrarlo en el error.
///
/// **Las extensiones van primero**, y eso costó un error. En el `bin` del SDK de
/// Google conviven `bq` —un guion de shell— y `bq.cmd`. Los dos son ficheros, así
/// que probar el nombre desnudo primero encontraba el guion y `CreateProcess`
/// respondía *«%1 no es una aplicación Win32 válida»*: en Windows `is_file()` no
/// es «es ejecutable», y los dos candidatos tienen exactamente el mismo aspecto.
/// Donde `PATHEXT` no existe —todo lo que no sea Windows— la lista está vacía y el
/// nombre desnudo es el único candidato, que es lo correcto allí.
pub fn resolver(programa: &str) -> Option<PathBuf> {
    let exts: Vec<OsString> = std::env::var_os("PATHEXT")
        .map(|p| {
            p.to_string_lossy()
                .split(';')
                .filter(|e| !e.is_empty())
                .map(OsString::from)
                .collect()
        })
        .unwrap_or_default();
    for dir in std::env::split_paths(&std::env::var_os("PATH")?) {
        let base = dir.join(programa);
        for e in &exts {
            let mut con = base.clone().into_os_string();
            con.push(e);
            let p = PathBuf::from(con);
            if p.is_file() {
                return Some(p);
            }
        }
        if base.is_file() {
            return Some(base);
        }
    }
    None
}

pub fn ejecutar(programa: &str, args: &[String], entrada: Option<&str>) -> Result<String, Fallo> {
    let ruta = resolver(programa).ok_or_else(|| {
        fallo(
            69,
            format!("no se encontró `{programa}` en el PATH"),
            &["  Es el programa que habla con la fuente. ORE no lo lleva dentro."],
        )
    })?;

    let mut cmd = Command::new(&ruta);
    cmd.args(args)
        .stdin(if entrada.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut hijo = cmd.spawn().map_err(|e| {
        fallo(
            69,
            format!("no se pudo ejecutar `{}`: {e}", ruta.display()),
            &[],
        )
    })?;
    if let Some(t) = entrada
        && let Some(mut s) = hijo.stdin.take()
    {
        use std::io::Write as _;
        let _ = s.write_all(t.as_bytes());
    }
    let salida = hijo
        .wait_with_output()
        .map_err(|e| fallo(69, format!("`{programa}` no terminó: {e}"), &[]))?;

    if !salida.status.success() {
        // Su stderr literal es lo único accionable que existe. Resumirlo aquí
        // convertiría un problema de cinco minutos en una tarde: un driver avisa
        // de que no hay credencial, o de que la fuente no responde, y las dos
        // cosas se arreglan solas en cuanto se leen.
        let err = String::from_utf8_lossy(&salida.stderr);
        let mut ayuda = vec![format!("  {}", ruta.display())];
        ayuda.extend(
            err.lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| format!("  │ {l}")),
        );
        return Err(Fallo {
            codigo: 69,
            mensaje: format!(
                "`{programa}` falló ({})",
                salida
                    .status
                    .code()
                    .map_or_else(|| "sin código".to_string(), |c| c.to_string())
            ),
            ayuda,
        });
    }
    String::from_utf8(salida.stdout)
        .map_err(|_| fallo(65, format!("`{programa}` no devolvió UTF-8"), &[]))
}

// ── Comprobaciones ──────────────────────────────────────────────────────────

/// `ore source explore` — **¿qué contiene esta fuente?**
///
/// Delega en el verbo `explorar` del lector, y la URL va por stdin como todo lo
/// demás: **una URL puede llevar una credencial dentro**, y `argv` lo lee
/// cualquier proceso de la máquina. Por eso esto toma el nombre de una fuente
/// declarada y no una URL suelta, aunque para BigQuery la URL no tenga secreto:
/// el mando no puede depender de qué familia sea.
pub fn explorar(raiz: &Path, fuente: &str) -> std::process::ExitCode {
    let (tipo, env) = match declaracion(raiz, fuente) {
        Ok(x) => x,
        Err(f) => return imprimir(f),
    };
    let url = match url(raiz, &env, fuente) {
        Ok(u) => u,
        Err(f) => return imprimir(f),
    };
    let coordenada = Json::obj([("url", Json::s(&url))]).jcs();
    let salida = match ejecutar(
        &format!("ore-read-{tipo}"),
        &["explorar".to_string()],
        Some(&coordenada),
    ) {
        Ok(s) => s,
        Err(f) => return imprimir(f),
    };
    let n = match parse::parse(&salida) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("error: lo que devolvió el lector no analiza: {e:?}");
            return std::process::ExitCode::from(65);
        }
    };
    let items = n.get("contiene").map(|(_, v)| v.items()).unwrap_or(&[]);
    println!("{fuente} · {} en `{tipo}`", items.len());
    for it in items {
        let nombre = it
            .get("nombre")
            .and_then(|(_, v)| v.as_str())
            .unwrap_or("?");
        match it.get("url").and_then(|(_, v)| v.as_str()) {
            // La URL sale **hecha**, y eso no es comodidad: es lo que evita que
            // alguien la componga a mano y se equivoque en el separador.
            Some(u) => println!(
                "  {nombre}
      ore source add --name {nombre} {u}"
            ),
            None => println!("  {nombre}"),
        }
    }
    if let Some(nota) = n.get("nota").and_then(|(_, v)| v.as_str()) {
        println!();
        println!("  {nota}");
    }
    std::process::ExitCode::SUCCESS
}

/// `ore source check` — **¿responde esta fuente?**
///
/// Delega igual que todo lo demás: el verbo `check` del lector, la URL por
/// stdin. `catalogo` contesta *qué hay*, y esto contesta *si contesta*: dos
/// preguntas que fallan por separado.
pub fn comprobar(raiz: &Path, fuente: &str) -> std::process::ExitCode {
    let (tipo, env) = match declaracion(raiz, fuente) {
        Ok(x) => x,
        Err(f) => return imprimir(f),
    };
    let url = match url(raiz, &env, fuente) {
        Ok(u) => u,
        Err(f) => return imprimir(f),
    };
    let programa = format!("ore-read-{tipo}");
    // La coordenada, no la URL pelada: `catalogo` recibe la URL a secas porque
    // es lo unico que necesita, y `check` usa la forma de `leer_coordenada`
    // —`{"url": ...}`— que es la que el protocolo fija para preguntar por un
    // origen. Dos formas para dos preguntas, cada una con su validacion.
    let coordenada = Json::obj([("url", Json::s(&url))]).jcs();
    let salida = match ejecutar(&programa, &["check".to_string()], Some(&coordenada)) {
        Ok(s) => s,
        // Que el lector no esté o no arranque **también** es una respuesta a la
        // pregunta, y la más común: se dice como tal y no como un fallo de otra
        // cosa.
        Err(f) => {
            println!("{fuente} · no · no se pudo preguntar");
            for l in std::iter::once(f.mensaje).chain(f.ayuda) {
                println!("  {l}");
            }
            return std::process::ExitCode::from(f.codigo);
        }
    };
    let n = match parse::parse(&salida) {
        Ok(n) => n,
        Err(e) => {
            println!("{fuente} · no · `{programa}` contestó algo que no analiza: {e:?}");
            return std::process::ExitCode::from(65);
        }
    };
    let ok = n.get("ok").and_then(|(_, v)| v.as_str()) == Some("true");
    let porque = n.get("porque").and_then(|(_, v)| v.as_str()).unwrap_or("");
    if ok {
        println!("{fuente} · sí · `{tipo}` responde");
        return std::process::ExitCode::SUCCESS;
    }
    println!("{fuente} · no · `{tipo}` no responde");
    // El motivo, **literal**: el mensaje del servidor es lo único accionable que
    // existe, y resumirlo convierte cinco minutos en una tarde.
    for l in porque.lines().filter(|l| !l.trim().is_empty()) {
        println!("  {l}");
    }
    std::process::ExitCode::from(69) // EX_UNAVAILABLE
}

pub fn imprimir(f: Fallo) -> std::process::ExitCode {
    eprintln!("error: {}", f.mensaje);
    for l in f.ayuda {
        eprintln!("{l}");
    }
    std::process::ExitCode::from(f.codigo)
}

// ── El catálogo, como artefacto ─────────────────────────────────────────────

/// **`ore source catalog`** — leer el catálogo de una fuente **y parar**.
///
/// # El hueco que cierra
///
/// `discover --from` acepta *«un catálogo ya leído, venga de donde venga»*, y
/// hasta hoy **ningún mando emitía uno**. Ni por arriba —`ore source` tenía
/// `add`, `explore` y `check`— ni por abajo, porque para BigQuery el driver
/// `catalogo` se negaba a propósito: esa receta vivía dentro de `ore` (hasta
/// A3 de BigQuery, que la mudó al driver).
///
/// Así que el artefacto de la frontera, el que la mitad de la suite escribe a
/// mano para probar lo que pasa después del driver, no se podía obtener con
/// ninguna orden.
///
/// # Por qué es un verbo de `source` y no una bandera de `discover`
///
/// Porque es la misma clase de pregunta que sus vecinos: `check` pregunta si
/// responde, `explore` qué contiene, y esto qué tiene dentro. Ninguno de los
/// tres escribe un documento OOS.
///
/// Y `discover` ya tenía la separación por dentro —`--source` y `--from` existen
/// porque *«son dos actos, y se piden por separado porque fallan por
/// separado»*—; lo único que faltaba era poder quedarse con lo de en medio.
///
/// # Lo que dice al escribirlo, y por qué eso no es adorno
///
/// **Qué claves de la forma trae este catálogo y cuáles no.** Es la respuesta
/// directa a lo que la medida encontró: una tabla sin `primaryKey` y una tabla
/// cuyo driver se olvidó de emitirlo **se ven exactamente igual**. Enseñar la
/// lista no lo arregla, pero lo hace mirable — y quien conoce el origen sabe
/// cuál de las dos cosas es.
///
/// Sin `--out` va a stdout **y nada más va a stdout**, para que se pueda
/// redirigir a un fichero y dárselo a `--from` tal cual.
pub fn emitir_catalogo(
    raiz: &Path,
    fuente: &str,
    destino: Option<&Path>,
) -> std::process::ExitCode {
    let texto = match catalogo(raiz, fuente) {
        Ok(t) => t,
        Err(f) => return imprimir(f),
    };
    let Some(out) = destino else {
        println!("{texto}");
        return std::process::ExitCode::SUCCESS;
    };
    if let Some(d) = out.parent()
        && !d.as_os_str().is_empty()
        && let Err(e) = std::fs::create_dir_all(d)
    {
        eprintln!("error: no se pudo crear `{}`: {e}", d.display());
        return std::process::ExitCode::from(73); // EX_CANTCREAT
    }
    if let Err(e) = std::fs::write(out, &texto) {
        eprintln!("error: no se pudo escribir `{}`: {e}", out.display());
        return std::process::ExitCode::from(73);
    }

    let cat = match Catalogo::leer(&texto) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: el catálogo recién escrito no se relee: {e}");
            return std::process::ExitCode::from(70); // EX_SOFTWARE
        }
    };
    println!("  ✓ {}", out.display());
    println!(
        "  ✓ {} objeto(s), {} columna(s)",
        cat.tablas.len(),
        cat.tablas.iter().map(|t| t.columnas.len()).sum::<usize>()
    );
    let ausentes = ausentes_de(&texto);
    if !ausentes.is_empty() {
        println!();
        println!("  · este origen no dice: {}", ausentes.join(", "));
        println!("    No es un fallo —la ausencia es una respuesta— pero conviene");
        println!("    mirarlo: una tabla sin clave y una tabla cuyo driver se");
        println!("    olvidó de emitirla se ven igual.");
    }
    println!();
    println!("  ore discover --from {} --out <paquete>", out.display());
    std::process::ExitCode::SUCCESS
}

/// Las claves de [`ore_driver::catalogo::FORMA`] que este catálogo **no** trae.
///
/// Se busca la literal entrecomillada sobre el texto emitido, que es exacto
/// porque el emisor es uno y escribe JSON: una clave está o no está.
fn ausentes_de(texto: &str) -> Vec<&'static str> {
    ore_driver::catalogo::FORMA
        .iter()
        .map(|(k, _, _)| *k)
        .filter(|k| !texto.contains(&format!("\"{k}\"")))
        .collect()
}
