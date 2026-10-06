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
    if pasarela().is_some() {
        let salida = preguntar(tipo, fuente, "catalog", url, vec![])?;
        parse::parse(&salida).map_err(|e| {
            fallo(
                65,
                format!("lo que devolvió la pasarela no analiza: {e:?}"),
                &[],
            )
        })?;
        return Ok(salida);
    }
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

// ── 0053 F8 · una vía ───────────────────────────────────────────────────────

/// Dónde está la pasarela (`ORE_PASARELA`, `host:puerto`). Con ella, lo que
/// mira un origen —catalogar, comprobar, explorar, el testigo— **no lanza un
/// conector**: lo pide a `ore-federation`, que lo hace en la cola del origen.
/// Sin ella (una máquina, el CLI en un portátil), el conector, como siempre.
pub fn pasarela() -> Option<String> {
    std::env::var("ORE_PASARELA")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// **Pregunta a un origen**: `ruta` es `catalog`, `check`, `explore` o
/// `witness` (con `extra`: `objeto` y quizá `cursor`). Por la pasarela si la
/// hay; si no, el verbo del conector con su entrada de siempre.
pub fn preguntar(
    tipo: &str,
    fuente: &str,
    ruta: &str,
    url: &str,
    extra: Vec<(&'static str, Json)>,
) -> Result<String, Fallo> {
    if let Some(p) = pasarela() {
        let mut campos = vec![
            ("origen", Json::s(fuente)),
            ("tipo", Json::s(tipo)),
            ("url", Json::s(url)),
        ];
        campos.extend(extra);
        return por_la_pasarela(&p, ruta, &Json::obj(campos).jcs())
            .map_err(|m| fallo(69, ore_driver::tapar(&m, url), &[]));
    }
    let programa = format!("ore-read-{tipo}");
    let coordenada = || Json::obj([("url", Json::s(url))]).jcs();
    let (args, entrada) = match ruta {
        "catalog" => (
            vec!["catalogo".to_string(), fuente.to_string()],
            url.to_string(),
        ),
        "check" => (vec!["check".to_string()], coordenada()),
        "explore" => (vec!["explorar".to_string()], coordenada()),
        // 0053 F9·3: `versiones` lee su petición entera (`extra`: `peticion`).
        "versions" => {
            let mut m = match extra.into_iter().find(|(k, _)| *k == "peticion") {
                Some((_, Json::Obj(m))) => m,
                _ => Default::default(),
            };
            m.insert("url".into(), Json::s(url));
            (vec!["versiones".to_string()], Json::Obj(m).jcs())
        }
        _ => {
            let mut c = vec![("url", Json::s(url))];
            c.extend(extra);
            (vec!["testigo".to_string()], Json::obj(c).jcs())
        }
    };
    ejecutar(&programa, &args, Some(&entrada))
}

/// **Una lectura de copia por la pasarela** (0053 F8·3): `POST /v1/read` con
/// `cuerpo` (ya con `perfil: "copia"`), el flujo Arrow a `destino` y sus
/// *trailers*: si no acaba `completo`, es un fallo (una copia cortada no es
/// una copia). Devuelve las filas.
pub fn leer_por_la_pasarela(
    destino_pasarela: &str,
    cuerpo: &str,
    destino: &mut impl std::io::Write,
) -> Result<u64, String> {
    use std::io::{BufRead as _, Read as _, Write as _};
    use std::net::ToSocketAddrs as _;
    let dir = destino_pasarela
        .to_socket_addrs()
        .map_err(|e| format!("la pasarela `{destino_pasarela}`: {e}"))?
        .next()
        .ok_or_else(|| format!("la pasarela `{destino_pasarela}` no tiene dirección"))?;
    let mut s = std::net::TcpStream::connect_timeout(&dir, std::time::Duration::from_secs(5))
        .map_err(|e| format!("la pasarela `{destino_pasarela}` no contesta: {e}"))?;
    // La espera en la cola (5 min) y entre dos lotes: holgado.
    s.set_read_timeout(Some(std::time::Duration::from_secs(600)))
        .ok();
    let req = format!(
        "POST /v1/read HTTP/1.1
host: pasarela
content-type: application/json
content-length: {}
te: trailers
connection: close

{cuerpo}",
        cuerpo.len()
    );
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut c = std::io::BufReader::new(s);
    let mut linea = String::new();
    c.read_line(&mut linea).map_err(|e| e.to_string())?;
    let codigo: u16 = linea
        .split_whitespace()
        .nth(1)
        .and_then(|x| x.parse().ok())
        .ok_or("la pasarela contestó algo que no es HTTP")?;
    let mut troceado = false;
    let mut largo: Option<usize> = None;
    loop {
        let mut l = String::new();
        c.read_line(&mut l).map_err(|e| e.to_string())?;
        let l = l.trim_end();
        if l.is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
            if k == "transfer-encoding" && v.eq_ignore_ascii_case("chunked") {
                troceado = true;
            }
            if k == "content-length" {
                largo = v.parse().ok();
            }
        }
    }
    if codigo != 200 {
        let mut b = String::new();
        match largo {
            Some(n) => {
                let mut v = vec![0; n];
                c.read_exact(&mut v).map_err(|e| e.to_string())?;
                b = String::from_utf8_lossy(&v).into_owned();
            }
            None => {
                let _ = c.read_to_string(&mut b);
            }
        }
        let m = parse::parse(b.trim())
            .ok()
            .and_then(|n| {
                n.get("mensaje")
                    .and_then(|(_, v)| v.as_str())
                    .map(String::from)
            })
            .unwrap_or_else(|| b.trim().chars().take(300).collect());
        return Err(format!("la pasarela contestó {codigo}: {m}"));
    }
    if !troceado {
        return Err("la pasarela no mandó un flujo (sin trailers no se sabe cómo acabó)".into());
    }
    // Los trozos, al fichero; después, los trailers.
    loop {
        let mut l = String::new();
        c.read_line(&mut l).map_err(|e| e.to_string())?;
        let n = usize::from_str_radix(l.trim().split(';').next().unwrap_or(""), 16)
            .map_err(|_| format!("un trozo con un tamaño raro: {:?}", l.trim()))?;
        if n == 0 {
            break;
        }
        let mut v = vec![0; n];
        c.read_exact(&mut v).map_err(|e| e.to_string())?;
        destino.write_all(&v).map_err(|e| e.to_string())?;
        let mut crlf = [0u8; 2];
        c.read_exact(&mut crlf).map_err(|e| e.to_string())?;
    }
    let mut finales = std::collections::BTreeMap::new();
    loop {
        let mut l = String::new();
        if c.read_line(&mut l).map_err(|e| e.to_string())? == 0 {
            break;
        }
        let l = l.trim_end();
        if l.is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            finales.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    let fin = |k: &str| finales.get(k).cloned().unwrap_or_default();
    if fin("ore-estado") != "completo" {
        return Err(format!(
            "la lectura no acabó completa: {} {}",
            fin("ore-estado"),
            fin("ore-motivo")
        ));
    }
    Ok(fin("ore-filas").parse().unwrap_or(0))
}

/// **El flujo de `bajar` por la pasarela** (0053 F9·3): `POST /v1/fetch`. Se
/// lee como el `stdout` del conector; al acabar, [`Troceado::fin`] dice si
/// acabó bien (sus *trailers*).
pub fn bajar_por_la_pasarela(destino_pasarela: &str, cuerpo: &str) -> Result<Troceado, String> {
    use std::io::{BufRead as _, Write as _};
    use std::net::ToSocketAddrs as _;
    let dir = destino_pasarela
        .to_socket_addrs()
        .map_err(|e| format!("la pasarela `{destino_pasarela}`: {e}"))?
        .next()
        .ok_or_else(|| format!("la pasarela `{destino_pasarela}` no tiene dirección"))?;
    let mut s = std::net::TcpStream::connect_timeout(&dir, std::time::Duration::from_secs(5))
        .map_err(|e| format!("la pasarela `{destino_pasarela}` no contesta: {e}"))?;
    s.set_read_timeout(Some(std::time::Duration::from_secs(600)))
        .ok();
    let req = format!(
        "POST /v1/fetch HTTP/1.1\r\nhost: pasarela\r\ncontent-type: application/json\r\ncontent-length: {}\r\nte: trailers\r\nconnection: close\r\n\r\n{cuerpo}",
        cuerpo.len()
    );
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut c = std::io::BufReader::new(s);
    let mut linea = String::new();
    c.read_line(&mut linea).map_err(|e| e.to_string())?;
    let codigo: u16 = linea
        .split_whitespace()
        .nth(1)
        .and_then(|x| x.parse().ok())
        .ok_or("la pasarela contestó algo que no es HTTP")?;
    let mut troceado = false;
    loop {
        let mut l = String::new();
        c.read_line(&mut l).map_err(|e| e.to_string())?;
        let l = l.trim_end();
        if l.is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':')
            && k.trim().eq_ignore_ascii_case("transfer-encoding")
            && v.trim().eq_ignore_ascii_case("chunked")
        {
            troceado = true;
        }
    }
    if codigo != 200 || !troceado {
        let mut b = String::new();
        let _ = std::io::Read::read_to_string(&mut c, &mut b);
        let m = parse::parse(b.trim())
            .ok()
            .and_then(|n| {
                n.get("mensaje")
                    .and_then(|(_, v)| v.as_str())
                    .map(String::from)
            })
            .unwrap_or_else(|| b.trim().chars().take(300).collect());
        return Err(format!("la pasarela contestó {codigo}: {m}"));
    }
    Ok(Troceado {
        c,
        queda: 0,
        acabado: false,
        finales: Default::default(),
    })
}

/// Un cuerpo HTTP troceado (`chunked`), leído como un flujo; al final, sus
/// *trailers*.
pub struct Troceado {
    c: std::io::BufReader<std::net::TcpStream>,
    queda: usize,
    acabado: bool,
    finales: std::collections::BTreeMap<String, String>,
}

impl std::io::Read for Troceado {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        use std::io::BufRead as _;
        if self.acabado || b.is_empty() {
            return Ok(0);
        }
        if self.queda == 0 {
            let mut l = String::new();
            self.c.read_line(&mut l)?;
            let n = usize::from_str_radix(l.trim().split(';').next().unwrap_or(""), 16)
                .map_err(|_| std::io::Error::other(format!("un trozo raro: {:?}", l.trim())))?;
            if n == 0 {
                self.acabado = true;
                loop {
                    let mut l = String::new();
                    if self.c.read_line(&mut l)? == 0 {
                        break;
                    }
                    let l = l.trim_end();
                    if l.is_empty() {
                        break;
                    }
                    if let Some((k, v)) = l.split_once(':') {
                        self.finales
                            .insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
                    }
                }
                return Ok(0);
            }
            self.queda = n;
        }
        let k = self.queda.min(b.len());
        let n = std::io::Read::read(&mut self.c, &mut b[..k])?;
        if n == 0 {
            return Err(std::io::Error::other("la pasarela cortó el flujo a medias"));
        }
        self.queda -= n;
        if self.queda == 0 {
            let mut crlf = [0u8; 2];
            std::io::Read::read_exact(&mut self.c, &mut crlf)?;
        }
        Ok(n)
    }
}

impl Troceado {
    /// Cómo acabó: `Ok` si los *trailers* dicen `completo`.
    pub fn fin(self) -> Result<(), String> {
        if !self.acabado {
            return Err("el flujo no llegó a su fin".into());
        }
        match self.finales.get("ore-estado").map(String::as_str) {
            Some("completo") => Ok(()),
            otro => Err(format!(
                "{} {}",
                otro.unwrap_or("sin estado"),
                self.finales.get("ore-motivo").cloned().unwrap_or_default()
            )),
        }
    }
}

/// `POST /v1/{ruta}` a la pasarela, por HTTP plano dentro del clúster. Un
/// código distinto de 200 es un fallo con su mensaje.
fn por_la_pasarela(destino: &str, ruta: &str, cuerpo: &str) -> Result<String, String> {
    use std::io::{Read as _, Write as _};
    use std::net::ToSocketAddrs as _;
    let dir = destino
        .to_socket_addrs()
        .map_err(|e| format!("la pasarela `{destino}`: {e}"))?
        .next()
        .ok_or_else(|| format!("la pasarela `{destino}` no tiene dirección"))?;
    let mut s = std::net::TcpStream::connect_timeout(&dir, std::time::Duration::from_secs(5))
        .map_err(|e| format!("la pasarela `{destino}` no contesta: {e}"))?;
    // El plazo de un verbo en la pasarela es 10 min; uno más para la cola.
    s.set_read_timeout(Some(std::time::Duration::from_secs(660)))
        .ok();
    let req = format!(
        "POST /v1/{ruta} HTTP/1.1\r\nhost: pasarela\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{cuerpo}",
        cuerpo.len()
    );
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut todo = Vec::new();
    s.read_to_end(&mut todo).map_err(|e| e.to_string())?;
    let texto = String::from_utf8_lossy(&todo);
    let (cabeza, cuerpo) = texto
        .split_once("\r\n\r\n")
        .ok_or("la pasarela contestó algo que no es HTTP")?;
    let codigo: u16 = cabeza
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .ok_or("la pasarela contestó algo que no es HTTP")?;
    if codigo == 200 {
        return Ok(cuerpo.to_string());
    }
    let mensaje = parse::parse(cuerpo.trim())
        .ok()
        .and_then(|n| {
            n.get("mensaje")
                .and_then(|(_, v)| v.as_str())
                .map(String::from)
        })
        .unwrap_or_else(|| cuerpo.trim().chars().take(300).collect());
    Err(format!("la pasarela contestó {codigo}: {mensaje}"))
}

/// Los verbos de un conector que **miran un origen** y que, con la pasarela,
/// sólo hace ella (0053 F8·4). `bajar` y `versiones` (las colecciones de
/// medios) aún no: son de F9.
const VERBOS_DE_LA_PASARELA: [&str; 7] = [
    "leer",
    "catalogo",
    "check",
    "explorar",
    "testigo",
    // 0053 F9·3: las colecciones de medios.
    "bajar",
    "versiones",
];

/// **Una vía** (0053 F8·4): con `ORE_PASARELA`, lanzar `ore-read-<tipo>` para
/// uno de esos verbos es un fallo, no una lectura por la puerta de atrás. Un
/// camino que se olvide de `preguntar` o de `leer_por_la_pasarela` se ve aquí,
/// y no en el origen de un cliente.
fn una_via(programa: &str, args: &[String]) -> Result<(), Fallo> {
    una_via_con(programa, args, pasarela().is_some())
}

fn una_via_con(programa: &str, args: &[String], hay_pasarela: bool) -> Result<(), Fallo> {
    let verbo = args.first().map(String::as_str).unwrap_or("catalogo");
    if hay_pasarela && programa.starts_with("ore-read-") && VERBOS_DE_LA_PASARELA.contains(&verbo) {
        return Err(fallo(
            70, // EX_SOFTWARE
            format!(
                "`{programa} {verbo}` con la pasarela puesta: lo que mira un origen va por ella (una vía)"
            ),
            &[
                "  Es un fallo de ORE, no del origen: este camino tendría que pedírselo a la pasarela.",
            ],
        ));
    }
    Ok(())
}

pub fn ejecutar(programa: &str, args: &[String], entrada: Option<&str>) -> Result<String, Fallo> {
    una_via(programa, args)?;
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
        return Err(fallo_del_hijo(
            programa,
            &ruta,
            salida.status,
            &salida.stderr,
        ));
    }
    String::from_utf8(salida.stdout)
        .map_err(|_| fallo(65, format!("`{programa}` no devolvió UTF-8"), &[]))
}

/// Su stderr literal es lo único accionable que existe. Resumirlo convertiría
/// un problema de cinco minutos en una tarde: un driver avisa de que no hay
/// credencial, o de que la fuente no responde, y las dos cosas se arreglan
/// solas en cuanto se leen.
fn fallo_del_hijo(
    programa: &str,
    ruta: &Path,
    estado: std::process::ExitStatus,
    stderr: &[u8],
) -> Fallo {
    let err = String::from_utf8_lossy(stderr);
    let mut ayuda = vec![format!("  {}", ruta.display())];
    ayuda.extend(
        err.lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| format!("  │ {l}")),
    );
    Fallo {
        codigo: 69,
        mensaje: format!(
            "`{programa}` falló ({})",
            estado
                .code()
                .map_or_else(|| "sin código".to_string(), |c| c.to_string())
        ),
        ayuda,
    }
}

/// **Un programa en marcha, con su salida sin leer** (ADR 0043).
///
/// [`ejecutar`] espera a que el programa acabe y devuelve su salida entera: con
/// las filas de una tabla, eso es la tabla en la memoria de `ore`. Esto la deja
/// abierta para encauzarla a otro programa. El stderr se recoge en un hilo
/// aparte: un programa que escribe mucho por ahí y nadie lo lee se bloquea.
pub struct Lanzado {
    programa: String,
    ruta: PathBuf,
    hijo: std::process::Child,
    pub stdin: Option<std::process::ChildStdin>,
    pub stdout: Option<std::process::ChildStdout>,
    stderr: Option<std::thread::JoinHandle<Vec<u8>>>,
}

/// Lanza `programa` y le escribe `entrada`. Con `abierta`, su stdin sigue
/// abierto para escribirle más; sin ella, se cierra.
pub fn lanzar(
    programa: &str,
    args: &[String],
    entrada: &str,
    abierta: bool,
) -> Result<Lanzado, Fallo> {
    una_via(programa, args)?;
    use std::io::{Read as _, Write as _};
    let ruta = resolver(programa).ok_or_else(|| {
        fallo(
            69,
            format!("no se encontró `{programa}` en el PATH"),
            &["  Es el programa que habla con la fuente. ORE no lo lleva dentro."],
        )
    })?;
    let mut hijo = Command::new(&ruta)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            fallo(
                69,
                format!("no se pudo ejecutar `{}`: {e}", ruta.display()),
                &[],
            )
        })?;
    let mut err = hijo.stderr.take();
    let stderr = std::thread::spawn(move || {
        let mut b = Vec::new();
        if let Some(e) = err.as_mut() {
            let _ = e.read_to_end(&mut b);
        }
        b
    });
    let mut stdin = hijo.stdin.take();
    if let Some(s) = stdin.as_mut() {
        let _ = s.write_all(entrada.as_bytes());
    }
    Ok(Lanzado {
        programa: programa.to_string(),
        ruta,
        stdout: hijo.stdout.take(),
        stdin: if abierta { stdin } else { None },
        hijo,
        stderr: Some(stderr),
    })
}

impl Lanzado {
    /// Espera a que acabe; lo que quede en stdout se lee y se devuelve.
    pub fn esperar(mut self) -> Result<Vec<u8>, Fallo> {
        use std::io::Read as _;
        drop(self.stdin.take());
        let mut salida = Vec::new();
        if let Some(mut o) = self.stdout.take() {
            let _ = o.read_to_end(&mut salida);
        }
        let estado = self
            .hijo
            .wait()
            .map_err(|e| fallo(69, format!("`{}` no terminó: {e}", self.programa), &[]))?;
        let err = self
            .stderr
            .take()
            .and_then(|h| h.join().ok())
            .unwrap_or_default();
        if !estado.success() {
            return Err(fallo_del_hijo(&self.programa, &self.ruta, estado, &err));
        }
        Ok(salida)
    }

    /// Lo corta: el otro lado del cauce ya no va a leer.
    pub fn matar(mut self) {
        let _ = self.hijo.kill();
        drop(self.stdin.take());
        drop(self.stdout.take());
        let _ = self.hijo.wait();
    }
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
    let salida = match preguntar(&tipo, fuente, "explore", &url, vec![]) {
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
    let salida = match preguntar(&tipo, fuente, "check", &url, vec![]) {
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
    // Permiso a permiso, si el lector los da (BigQuery: jobs, datos, lectura).
    // Es lo que convierte «no responde» en «concede ESTE rol AQUÍ».
    let permisos: Vec<String> = n
        .get("permisos")
        .map(|(_, o)| {
            o.entries()
                .iter()
                .filter_map(|(k, v)| {
                    let campo = |c: &str| v.get(c).and_then(|(_, x)| x.as_str()).unwrap_or("");
                    let marca = match campo("ok") {
                        "true" => "✓",
                        "false" => "✗",
                        _ => "·",
                    };
                    Some(format!(
                        "  {marca} {} · `{}` en {}",
                        k.as_str()?,
                        campo("rol"),
                        campo("donde")
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    if ok {
        println!("{fuente} · sí · `{tipo}` responde");
        for l in &permisos {
            println!("{l}");
        }
        return std::process::ExitCode::SUCCESS;
    }
    println!("{fuente} · no · `{tipo}` no responde");
    for l in &permisos {
        println!("{l}");
    }
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

#[cfg(test)]
mod pruebas_una_via {
    use super::*;

    /// 0053 F9·3 · El flujo troceado de `/v1/fetch`: los bytes, y su final por
    /// los *trailers* (completo, o el error que dice).
    #[test]
    fn el_flujo_de_bajar_y_su_final() {
        use std::io::{Read as _, Write as _};
        let servir = |trailer: &'static str| {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let dir = l.local_addr().unwrap().to_string();
            std::thread::spawn(move || {
                let (mut s, _) = l.accept().unwrap();
                let mut b = [0u8; 4096];
                let _ = s.read(&mut b);
                let r = format!(
                    "HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\ntrailer: ore-estado\r\n\r\n\
                     5\r\nhola \r\n5\r\nmundo\r\n0\r\n{trailer}\r\n\r\n"
                );
                s.write_all(r.as_bytes()).unwrap();
            });
            dir
        };
        let d = servir("ore-estado: completo");
        let mut f = bajar_por_la_pasarela(&d, "{}").unwrap();
        let mut todo = String::new();
        f.read_to_string(&mut todo).unwrap();
        assert_eq!(todo, "hola mundo");
        assert!(f.fin().is_ok());
        let d = servir("ore-estado: error\r\nore-motivo: 403 de S3");
        let mut f = bajar_por_la_pasarela(&d, "{}").unwrap();
        let mut todo = Vec::new();
        f.read_to_end(&mut todo).unwrap();
        assert!(f.fin().unwrap_err().contains("403 de S3"));
    }

    #[test]
    fn con_la_pasarela_ningun_conector_mira_un_origen() {
        let a = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        for v in ["leer", "catalogo", "check", "explorar", "testigo"] {
            assert!(
                una_via_con("ore-read-postgres", &a(&[v]), true).is_err(),
                "{v}"
            );
            assert!(
                una_via_con("ore-read-postgres", &a(&[v]), false).is_ok(),
                "{v}"
            );
        }
        // La forma vieja (`ore-read-postgres <fuente>`) es `catalogo`.
        assert!(una_via_con("ore-read-postgres", &[], true).is_err());
        // F9·3: las colecciones también; lo que no es un conector, sí.
        assert!(una_via_con("ore-read-s3", &a(&["bajar"]), true).is_err());
        assert!(una_via_con("ore-read-s3", &a(&["versiones"]), true).is_err());
        assert!(una_via_con("ore-store-gcs", &a(&["leer"]), true).is_ok());
    }
}
