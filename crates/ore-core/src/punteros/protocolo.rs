//! **La puerta al otro lado de un proceso** (0079 §19.1–§19.2): el protocolo, su cliente
//! ([`Proceso`]) y su servidor ([`Servidor`]), para quien no puede tener la puerta dentro: `ore`
//! no abre un socket, y los Jobs no tienen la credencial del catálogo. Lo usan `ore` (cliente y
//! `ore punteros-servir`), ore-serve (la puerta heredada, ·5c·3) y `ore-catalogo punteros` (·5c·2).
//!
//! # El protocolo
//!
//! `ore` no abre un socket (la guarda `el_binario_que_se_distribuye_no_sabe_
//! hablar_por_la_red`): como con `ore-store-<gcs>`, habla con un hijo por stdin
//! y stdout. `ORE_PUNTEROS` es la orden: palabras separadas por espacios, sin
//! comillas (`/ruta/ore punteros-servir`, `ore-catalogo punteros --esquema t_x`).
//! Sin la variable (o vacía), [`Local`]. El hijo arranca con la primera
//! pregunta, hereda el directorio y el stderr de `ore`, y termina cuando `ore`
//! cierra su stdin.
//!
//! **Una petición por línea, una respuesta por línea**, en JSON de una línea
//! (las claves en cualquier orden). Cada petición lleva `op`. La respuesta es
//! `{"ok": <valor>}` o `{"fallo": {"codigo": …, …}}`.
//!
//! La primera petición **abre la puerta**; antes, cualquier otra es un 503:
//!
//! ```text
//! → {"op":"abrir","raiz":"/arbol","carpeta":"datasets"|null,"ajenos":"/trabajo/ramas"|null,"ref":"r"|null}
//! ← {"ok":true}
//! ```
//!
//! `raiz`, `carpeta` y `ajenos` son lo que `ore` recibió (relativos a su
//! directorio, que el hijo hereda); `ref` es `ORE_REF`. Quien sirve con
//! ficheros no usa `ref` (el directorio ya es la ref); el catálogo no usa los
//! directorios.
//!
//! | `op` | Argumentos | `ok` |
//! |---|---|---|
//! | `uno` | `nombre` | `null` o un **leído** |
//! | `todos` | — | `{"<nombre>": <leído>, …}` |
//! | `historia` | `nombre`, `hasta` (entero) | `[{"id","cuando","quien": s\|null,"texto"}, …]`, de la más nueva a la más vieja |
//! | `escribir` | `cambios`, `contexto` | `true` |
//! | `reclamados` | — | `{"datasets":[s],"claves":[s],"ubicaciones":{"<dataset>":[s]},"colecciones":[[dataset, ml]]}` |
//! | `version` | — | una cadena |
//! | `crear` | `rama`, `desde` | `true` |
//! | `borrar` | `rama` | `true` |
//! | `traer` | `rama` | `true` |
//!
//! - Un **leído** es `{"texto": s, "de": "aqui"|"main"}`: el texto tal como
//!   está guardado; el nodo es su análisis.
//! - Un **cambio** es `{"nombre": s, "que": Q, "esperado": E}`, con
//!   `Q = "retirar" | {"sustituir": <objeto>} | {"poner": {"campos": [[k, v], …], "quitar": [k, …]}}`
//!   (los campos en su orden, como pares) y
//!   `E = "cualquiera" | "nada" | {"ml": s}`.
//! - El **contexto** es `{"autor": s, "operacion": s, "idempotencia": s|null}`.
//! - Un **fallo** lleva su `codigo` y lo de su variante: `409`
//!   `{"nombre","esperado": E,"hay": s|null}` (el *compare-and-set*; nada se
//!   escribió), `422` `{"nombre"}`, `423` `{"ref"}`, `503` `{"mensaje"}`.
//!
//! Los valores van como JSON de `ore_core` (`Json::jcs`) y vuelven por
//! `Json::de_node_fiel`: una cadena sigue siendo cadena, un `null` nulo.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Mutex, OnceLock};

use super::puerta::{
    Cambio, Contexto, De, Esperado, Fallo, Leido, Punteros, Que, Ramas, Reclamados, Version,
};
use crate::json::Json;
use crate::parse::{Node, Style};

/// La variable que dice con quién hablar: una orden (`ore-catalogo punteros --esquema t_x`), o
/// `fd:<lee>,<escribe>` para la puerta heredada.
pub const VARIABLE: &str = "ORE_PUNTEROS";

// ── El protocolo: de y a JSON ───────────────────────────────────────────────

fn nulo() -> Json {
    Json::Crudo("null".into())
}

fn es_nulo(n: &Node) -> bool {
    matches!(n, Node::Scalar { raw, style: Style::Plain, .. } if raw == "null")
}

/// Una cadena, o `None` si es `null` o no está.
fn cadena(n: &Node, k: &str) -> Option<String> {
    let (_, v) = n.get(k)?;
    if es_nulo(v) {
        return None;
    }
    v.as_str().map(String::from)
}

fn mal(que: &str) -> Fallo {
    Fallo::Almacen(format!("protocolo de los punteros: {que}"))
}

fn cadena_o_fallo(n: &Node, k: &str) -> Result<String, Fallo> {
    cadena(n, k).ok_or_else(|| mal(&format!("falta `{k}`")))
}

fn opcional(s: Option<&str>) -> Json {
    s.map_or_else(nulo, Json::s)
}

fn esperado_a_json(e: &Esperado) -> Json {
    match e {
        Esperado::Cualquiera => Json::s("cualquiera"),
        Esperado::Nada => Json::s("nada"),
        Esperado::Ml(m) => Json::obj([("ml", Json::s(m))]),
    }
}

fn esperado_de_json(n: &Node) -> Result<Esperado, Fallo> {
    match n.as_str() {
        Some("cualquiera") => Ok(Esperado::Cualquiera),
        Some("nada") => Ok(Esperado::Nada),
        _ => cadena(n, "ml")
            .map(Esperado::Ml)
            .ok_or_else(|| mal("`esperado` no es `cualquiera`, `nada` ni `{ml}`")),
    }
}

pub(crate) fn fallo_a_json(f: &Fallo) -> Json {
    let mut m = vec![("codigo", Json::Int(i64::from(f.codigo())))];
    match f {
        Fallo::Conflicto {
            nombre,
            esperado,
            hay,
        } => {
            m.push(("nombre", Json::s(nombre)));
            m.push(("esperado", esperado_a_json(esperado)));
            m.push(("hay", opcional(hay.as_deref())));
        }
        Fallo::Nombre(n) => m.push(("nombre", Json::s(n))),
        Fallo::Protegida(r) => m.push(("ref", Json::s(r))),
        Fallo::Almacen(t) => m.push(("mensaje", Json::s(t))),
    }
    Json::obj([("fallo", Json::obj(m))])
}

pub(crate) fn fallo_de_json(f: &Node) -> Fallo {
    let codigo = f.get("codigo").and_then(|(_, v)| v.as_str()).unwrap_or("");
    let texto = |k: &str| cadena(f, k).unwrap_or_default();
    match codigo {
        "409" => Fallo::Conflicto {
            nombre: texto("nombre"),
            esperado: f
                .get("esperado")
                .and_then(|(_, e)| esperado_de_json(e).ok())
                .unwrap_or(Esperado::Cualquiera),
            hay: cadena(f, "hay"),
        },
        "422" => Fallo::Nombre(texto("nombre")),
        "423" => Fallo::Protegida(texto("ref")),
        "503" => Fallo::Almacen(texto("mensaje")),
        otro => Fallo::Almacen(format!(
            "el servidor de los punteros falló con `{otro}`: {}",
            texto("mensaje")
        )),
    }
}

fn leido_a_json(l: &Leido) -> Json {
    Json::obj([
        ("texto", Json::s(&l.texto)),
        (
            "de",
            Json::s(match l.de {
                De::Aqui => "aqui",
                De::Main => "main",
            }),
        ),
    ])
}

fn leido_de_json(n: &Node) -> Result<Leido, Fallo> {
    let texto = cadena_o_fallo(n, "texto")?;
    let puntero = crate::parse::parse(&texto)
        .map_err(|e| mal(&format!("un puntero que no analiza: {e:?}")))?;
    let de = match cadena(n, "de").as_deref() {
        Some("main") => De::Main,
        _ => De::Aqui,
    };
    Ok(Leido { puntero, texto, de })
}

fn cambio_a_json(c: &Cambio) -> Json {
    let que = match &c.que {
        Que::Retirar => Json::s("retirar"),
        Que::Sustituir(j) => Json::obj([("sustituir", j.clone())]),
        Que::Poner { campos, quitar } => Json::obj([(
            "poner",
            Json::obj([
                (
                    "campos",
                    Json::Arr(
                        campos
                            .iter()
                            .map(|(k, v)| Json::Arr(vec![Json::s(k), v.clone()]))
                            .collect(),
                    ),
                ),
                ("quitar", Json::Arr(quitar.iter().map(Json::s).collect())),
            ]),
        )]),
    };
    Json::obj([
        ("nombre", Json::s(&c.nombre)),
        ("que", que),
        ("esperado", esperado_a_json(&c.esperado)),
    ])
}

fn cambio_de_json(n: &Node) -> Result<Cambio, Fallo> {
    let nombre = cadena_o_fallo(n, "nombre")?;
    let (_, q) = n.get("que").ok_or_else(|| mal("un cambio sin `que`"))?;
    let que = if q.as_str() == Some("retirar") {
        Que::Retirar
    } else if let Some((_, s)) = q.get("sustituir") {
        Que::Sustituir(Json::de_node_fiel(s))
    } else if let Some((_, p)) = q.get("poner") {
        let campos = p
            .get("campos")
            .map(|(_, c)| {
                c.items()
                    .iter()
                    .map(|par| match par.items() {
                        [k, v] => Ok((
                            k.as_str()
                                .ok_or_else(|| mal("un campo sin nombre"))?
                                .to_string(),
                            Json::de_node_fiel(v),
                        )),
                        _ => Err(mal("un campo no es `[nombre, valor]`")),
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default();
        let quitar = p
            .get("quitar")
            .map(|(_, q)| {
                q.items()
                    .iter()
                    .filter_map(|k| k.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        Que::Poner { campos, quitar }
    } else {
        return Err(mal("`que` no es `retirar`, `sustituir` ni `poner`"));
    };
    let esperado = match n.get("esperado") {
        Some((_, e)) => esperado_de_json(e)?,
        None => Esperado::Cualquiera,
    };
    Ok(Cambio {
        nombre,
        que,
        esperado,
    })
}

fn contexto_a_json(c: &Contexto) -> Json {
    Json::obj([
        ("autor", Json::s(&c.autor)),
        ("operacion", Json::s(&c.operacion)),
        ("idempotencia", opcional(c.idempotencia.as_deref())),
    ])
}

fn contexto_de_json(n: Option<&Node>) -> Contexto {
    match n {
        Some(n) => Contexto {
            autor: cadena(n, "autor").unwrap_or_default(),
            operacion: cadena(n, "operacion").unwrap_or_default(),
            idempotencia: cadena(n, "idempotencia"),
        },
        None => Contexto::default(),
    }
}

fn reclamados_a_json(r: &Reclamados) -> Json {
    let lista = |s: &BTreeSet<String>| Json::Arr(s.iter().map(Json::s).collect());
    Json::obj([
        ("datasets", lista(&r.datasets)),
        ("claves", lista(&r.claves)),
        (
            "ubicaciones",
            Json::Obj(
                r.ubicaciones
                    .iter()
                    .map(|(k, v)| (k.clone(), lista(v)))
                    .collect(),
            ),
        ),
        (
            "colecciones",
            Json::Arr(
                r.colecciones
                    .iter()
                    .map(|(d, m)| Json::Arr(vec![Json::s(d), Json::s(m)]))
                    .collect(),
            ),
        ),
    ])
}

fn reclamados_de_json(n: &Node) -> Reclamados {
    let lista = |k: &str| -> BTreeSet<String> {
        n.get(k)
            .map(|(_, v)| {
                v.items()
                    .iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    };
    let ubicaciones = n
        .get("ubicaciones")
        .map(|(_, u)| {
            u.entries()
                .iter()
                .filter_map(|(k, v)| {
                    Some((
                        k.as_str()?.to_string(),
                        v.items()
                            .iter()
                            .filter_map(|x| x.as_str().map(String::from))
                            .collect(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let colecciones = n
        .get("colecciones")
        .map(|(_, c)| {
            c.items()
                .iter()
                .filter_map(|par| match par.items() {
                    [d, m] => Some((d.as_str()?.to_string(), m.as_str()?.to_string())),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    Reclamados {
        datasets: lista("datasets"),
        claves: lista("claves"),
        ubicaciones,
        colecciones,
    }
}

fn version_a_json(v: &Version) -> Json {
    Json::obj([
        ("id", Json::s(&v.id)),
        ("cuando", Json::s(&v.cuando)),
        ("quien", opcional(v.quien.as_deref())),
        ("texto", Json::s(&v.texto)),
    ])
}

fn version_de_json(n: &Node) -> Result<Version, Fallo> {
    let texto = cadena_o_fallo(n, "texto")?;
    Ok(Version {
        id: cadena(n, "id").unwrap_or_default(),
        cuando: cadena(n, "cuando").unwrap_or_default(),
        quien: cadena(n, "quien"),
        puntero: crate::parse::parse(&texto)
            .map_err(|e| mal(&format!("una versión que no analiza: {e:?}")))?,
        texto,
    })
}

fn peticion(op: &str, args: Vec<(&'static str, Json)>) -> String {
    let mut m: BTreeMap<String, Json> = args.into_iter().map(|(k, v)| (k.into(), v)).collect();
    m.insert("op".into(), Json::s(op));
    Json::Obj(m).jcs()
}

fn ruta_o_nulo(p: Option<&Path>) -> Json {
    p.map_or_else(nulo, |p| Json::s(p.to_string_lossy()))
}

/// El mensaje que abre la puerta.
fn abrir_json(
    raiz: &Path,
    carpeta: Option<&Path>,
    ajenos: Option<&Path>,
    rama: Option<&str>,
) -> String {
    peticion(
        "abrir",
        vec![
            ("raiz", Json::s(raiz.to_string_lossy())),
            ("carpeta", ruta_o_nulo(carpeta)),
            ("ajenos", ruta_o_nulo(ajenos)),
            ("ref", opcional(rama)),
        ],
    )
}

// ── Proceso: el cliente ─────────────────────────────────────────────────────

/// Por dónde viajan las líneas: un hijo, un par de tuberías (la puerta heredada) o, en las
/// pruebas, un servidor en memoria.
pub trait Canal: Send + Sync {
    fn ida_y_vuelta(&self, linea: &str) -> Result<String, Fallo>;
}

/// **La puerta al otro lado de un proceso.** Ver el módulo.
pub struct Proceso {
    canal: Box<dyn Canal>,
    abrir: String,
    abierta: OnceLock<Result<(), Fallo>>,
}

impl Proceso {
    /// Sobre el hijo que arranca `orden`, la primera vez que se pregunta.
    pub fn hijo(
        orden: &str,
        raiz: &Path,
        carpeta: Option<&Path>,
        ajenos: Option<&Path>,
        rama: Option<String>,
    ) -> Proceso {
        Proceso {
            canal: Box::new(Hijo {
                orden: orden.to_string(),
                con: Mutex::new(None),
            }),
            abrir: abrir_json(raiz, carpeta, ajenos, rama.as_deref()),
            abierta: OnceLock::new(),
        }
    }

    /// Sobre un canal cualquiera: [`Tuberias`] (la puerta heredada de ore-serve) o uno propio.
    pub fn por(
        canal: Box<dyn Canal>,
        raiz: &Path,
        carpeta: Option<&Path>,
        ajenos: Option<&Path>,
        rama: Option<String>,
    ) -> Proceso {
        Proceso {
            canal,
            abrir: abrir_json(raiz, carpeta, ajenos, rama.as_deref()),
            abierta: OnceLock::new(),
        }
    }

    fn pedir(&self, op: &str, args: Vec<(&'static str, Json)>) -> Result<Node, Fallo> {
        self.abierta
            .get_or_init(|| responder(&self.canal.ida_y_vuelta(&self.abrir)?).map(|_| ()))
            .clone()?;
        responder(&self.canal.ida_y_vuelta(&peticion(op, args))?)
    }
}

/// La respuesta: lo de `ok`, o el fallo que dice.
fn responder(linea: &str) -> Result<Node, Fallo> {
    let n = crate::parse::parse(linea)
        .map_err(|e| mal(&format!("una respuesta que no analiza ({e:?}): `{linea}`")))?;
    if let Some((_, ok)) = n.get("ok") {
        return Ok(ok.clone());
    }
    match n.get("fallo") {
        Some((_, f)) => Err(fallo_de_json(f)),
        None => Err(mal(&format!(
            "una respuesta sin `ok` ni `fallo`: `{linea}`"
        ))),
    }
}

impl Punteros for Proceso {
    fn uno(&self, nombre: &str) -> Result<Option<Leido>, Fallo> {
        let n = self.pedir("uno", vec![("nombre", Json::s(nombre))])?;
        if es_nulo(&n) {
            return Ok(None);
        }
        leido_de_json(&n).map(Some)
    }

    fn todos(&self) -> Result<BTreeMap<String, Leido>, Fallo> {
        self.pedir("todos", vec![])?
            .entries()
            .iter()
            .map(|(k, v)| {
                let k = k
                    .as_str()
                    .ok_or_else(|| mal("un nombre que no es cadena"))?;
                Ok((k.to_string(), leido_de_json(v)?))
            })
            .collect()
    }

    fn historia(&self, nombre: &str, hasta: usize) -> Result<Vec<Version>, Fallo> {
        self.pedir(
            "historia",
            vec![
                ("nombre", Json::s(nombre)),
                ("hasta", Json::Int(i64::try_from(hasta).unwrap_or(i64::MAX))),
            ],
        )?
        .items()
        .iter()
        .map(version_de_json)
        .collect()
    }

    fn escribir(&self, cambios: &[Cambio], ctx: &Contexto) -> Result<(), Fallo> {
        self.pedir(
            "escribir",
            vec![
                (
                    "cambios",
                    Json::Arr(cambios.iter().map(cambio_a_json).collect()),
                ),
                ("contexto", contexto_a_json(ctx)),
            ],
        )
        .map(|_| ())
    }

    fn reclamados(&self) -> Result<Reclamados, Fallo> {
        self.pedir("reclamados", vec![])
            .map(|n| reclamados_de_json(&n))
    }

    fn version(&self) -> Result<String, Fallo> {
        let n = self.pedir("version", vec![])?;
        n.as_str()
            .map(String::from)
            .ok_or_else(|| mal("la versión no es una cadena"))
    }
}

impl Ramas for Proceso {
    fn crear(&self, rama: &str, desde: &str) -> Result<(), Fallo> {
        self.pedir(
            "crear",
            vec![("rama", Json::s(rama)), ("desde", Json::s(desde))],
        )
        .map(|_| ())
    }
    fn borrar(&self, rama: &str) -> Result<(), Fallo> {
        self.pedir("borrar", vec![("rama", Json::s(rama))])
            .map(|_| ())
    }
    fn traer(&self, rama: &str) -> Result<(), Fallo> {
        self.pedir("traer", vec![("rama", Json::s(rama))])
            .map(|_| ())
    }
}

/// **Un par de tuberías ya abiertas**: la puerta heredada (ore-serve lanza `ore` con dos y sirve
/// por ellas desde su pool, 0079 §19.2 P1).
/// El extremo que lee, con su búfer, y el que escribe.
type Par = (
    BufReader<Box<dyn std::io::Read + Send>>,
    Box<dyn Write + Send>,
);

pub struct Tuberias {
    par: Mutex<Par>,
}

impl Tuberias {
    pub fn nuevas(
        lector: Box<dyn std::io::Read + Send>,
        escritor: Box<dyn Write + Send>,
    ) -> Tuberias {
        Tuberias {
            par: Mutex::new((BufReader::new(lector), escritor)),
        }
    }
}

impl Canal for Tuberias {
    fn ida_y_vuelta(&self, linea: &str) -> Result<String, Fallo> {
        let mut g = self
            .par
            .lock()
            .map_err(|_| Fallo::Almacen("las tuberías de los punteros quedaron rotas".into()))?;
        let roto =
            |e: std::io::Error| Fallo::Almacen(format!("la puerta heredada no contestó: {e}"));
        g.1.write_all(linea.as_bytes()).map_err(roto)?;
        g.1.write_all(b"\n").map_err(roto)?;
        g.1.flush().map_err(roto)?;
        let mut vuelta = String::new();
        if g.0.read_line(&mut vuelta).map_err(roto)? == 0 {
            return Err(Fallo::Almacen(
                "la puerta heredada se cerró sin contestar".into(),
            ));
        }
        Ok(vuelta)
    }
}

/// El hijo, arrancado con la primera línea.
struct Hijo {
    orden: String,
    con: Mutex<Option<Conexion>>,
}

struct Conexion {
    hijo: Child,
    entrada: Option<ChildStdin>,
    salida: BufReader<ChildStdout>,
}

impl Drop for Conexion {
    /// Cerrar su stdin es decirle que acabó; se le espera.
    fn drop(&mut self) {
        drop(self.entrada.take());
        let _ = self.hijo.wait();
    }
}

impl Hijo {
    fn arrancar(&self) -> Result<Conexion, Fallo> {
        let mut palabras = self.orden.split_whitespace();
        let programa = palabras
            .next()
            .ok_or_else(|| Fallo::Almacen(format!("`{VARIABLE}` está vacía")))?;
        let mut hijo = Command::new(programa)
            .args(palabras)
            // Quien sirve no vuelve a pedirse a sí mismo.
            .env_remove(VARIABLE)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| Fallo::Almacen(format!("`{VARIABLE}={}` no arrancó: {e}", self.orden)))?;
        let entrada = hijo.stdin.take();
        let salida = hijo
            .stdout
            .take()
            .ok_or_else(|| Fallo::Almacen("el servidor de los punteros, sin stdout".into()))?;
        Ok(Conexion {
            hijo,
            entrada,
            salida: BufReader::new(salida),
        })
    }
}

impl Canal for Hijo {
    fn ida_y_vuelta(&self, linea: &str) -> Result<String, Fallo> {
        let mut g = self
            .con
            .lock()
            .map_err(|_| Fallo::Almacen("la conexión con los punteros quedó rota".into()))?;
        if g.is_none() {
            *g = Some(self.arrancar()?);
        }
        let Some(c) = g.as_mut() else {
            return Err(Fallo::Almacen("sin conexión con los punteros".into()));
        };
        let roto = |e: std::io::Error| Fallo::Almacen(format!("`{}` no contestó: {e}", self.orden));
        let entrada = c
            .entrada
            .as_mut()
            .ok_or_else(|| Fallo::Almacen("la entrada de los punteros, cerrada".into()))?;
        entrada.write_all(linea.as_bytes()).map_err(roto)?;
        entrada.write_all(b"\n").map_err(roto)?;
        entrada.flush().map_err(roto)?;
        let mut vuelta = String::new();
        if c.salida.read_line(&mut vuelta).map_err(roto)? == 0 {
            return Err(Fallo::Almacen(format!(
                "`{}` se cerró sin contestar",
                self.orden
            )));
        }
        Ok(vuelta)
    }
}

// ── El servidor de referencia ──────────────────────────────────────────────

/// Una puerta que también gobierna ramas: lo que sirve un [`Servidor`].
pub trait Puerta: Punteros + Ramas + Send {}
impl<T: Punteros + Ramas + Send> Puerta for T {}

/// Lo que pide `abrir`: la raíz del árbol, la carpeta de los punteros, lo que reclaman las demás
/// ramas y la ref. Quien sirve con ficheros usa los directorios; el catálogo, la ref.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Apertura {
    pub raiz: PathBuf,
    pub carpeta: Option<PathBuf>,
    pub ajenos: Option<PathBuf>,
    pub rama: Option<String>,
}

/// Abre la puerta que pide un `abrir`.
pub type Abridor = Box<dyn Fn(&Apertura) -> Result<Box<dyn Puerta>, Fallo> + Send>;

/// Lo que sirve una conexión: la puerta que abre su `abrir`.
pub struct Servidor {
    abridor: Abridor,
    puerta: Option<Box<dyn Puerta>>,
}

impl Servidor {
    pub fn nuevo(abridor: Abridor) -> Servidor {
        Servidor {
            abridor,
            puerta: None,
        }
    }

    /// Una línea de petición, una de respuesta.
    pub fn atender(&mut self, linea: &str) -> String {
        let r = match crate::parse::parse(linea) {
            Ok(n) => self.una(&n),
            Err(e) => Err(mal(&format!("una petición que no analiza: {e:?}"))),
        };
        match r {
            Ok(j) => Json::obj([("ok", j)]).jcs(),
            Err(f) => fallo_a_json(&f).jcs(),
        }
    }

    fn una(&mut self, n: &Node) -> Result<Json, Fallo> {
        let op = cadena_o_fallo(n, "op")?;
        if op == "abrir" {
            let a = Apertura {
                raiz: PathBuf::from(cadena_o_fallo(n, "raiz")?),
                carpeta: cadena(n, "carpeta").map(PathBuf::from),
                ajenos: cadena(n, "ajenos").map(PathBuf::from),
                rama: cadena(n, "ref"),
            };
            self.puerta = Some((self.abridor)(&a)?);
            return Ok(Json::Bool(true));
        }
        let p = self
            .puerta
            .as_deref()
            .ok_or_else(|| mal("la puerta no está abierta: primero `abrir`"))?;
        let nombre = || cadena_o_fallo(n, "nombre");
        let rama = || cadena_o_fallo(n, "rama");
        match op.as_str() {
            "uno" => Ok(p.uno(&nombre()?)?.as_ref().map_or_else(nulo, leido_a_json)),
            "todos" => Ok(Json::Obj(
                p.todos()?
                    .iter()
                    .map(|(k, l)| (k.clone(), leido_a_json(l)))
                    .collect(),
            )),
            "historia" => {
                let hasta = n
                    .get("hasta")
                    .and_then(|(_, h)| h.as_str()?.parse::<usize>().ok())
                    .ok_or_else(|| mal("falta `hasta`"))?;
                Ok(Json::Arr(
                    p.historia(&nombre()?, hasta)?
                        .iter()
                        .map(version_a_json)
                        .collect(),
                ))
            }
            "escribir" => {
                let cambios: Vec<Cambio> = n
                    .get("cambios")
                    .map(|(_, c)| {
                        c.items()
                            .iter()
                            .map(cambio_de_json)
                            .collect::<Result<Vec<_>, _>>()
                    })
                    .transpose()?
                    .unwrap_or_default();
                let ctx = contexto_de_json(n.get("contexto").map(|(_, c)| c));
                p.escribir(&cambios, &ctx)?;
                Ok(Json::Bool(true))
            }
            "reclamados" => Ok(reclamados_a_json(&p.reclamados()?)),
            "version" => Ok(Json::s(p.version()?)),
            "crear" => {
                p.crear(&rama()?, &cadena_o_fallo(n, "desde")?)?;
                Ok(Json::Bool(true))
            }
            "borrar" => {
                p.borrar(&rama()?)?;
                Ok(Json::Bool(true))
            }
            "traer" => {
                p.traer(&rama()?)?;
                Ok(Json::Bool(true))
            }
            otra => Err(mal(&format!("no hay `{otra}`"))),
        }
    }
}

/// **Servir** por líneas: una petición por línea de `entrada`, su respuesta en `salida`, hasta que
/// `entrada` se cierre.
pub fn servir(
    entrada: impl BufRead,
    mut salida: impl Write,
    s: &mut Servidor,
) -> std::io::Result<()> {
    for linea in entrada.lines() {
        let linea = linea?;
        if linea.trim().is_empty() {
            continue;
        }
        let r = s.atender(&linea);
        writeln!(salida, "{r}")?;
        salida.flush()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::punteros::Ficheros;

    /// El servidor de referencia: ficheros sobre lo que diga `abrir`.
    fn de_ficheros() -> Servidor {
        Servidor::nuevo(Box::new(|a: &Apertura| {
            let f = Ficheros::en(&a.raiz).con_ajenos(a.ajenos.as_deref());
            Ok(Box::new(match &a.carpeta {
                Some(c) => f.con_carpeta(c),
                None => f,
            }) as Box<dyn Puerta>)
        }))
    }

    /// El servidor, en memoria: las mismas líneas, sin hijo.
    struct EnMemoria(Mutex<Servidor>);

    impl Canal for EnMemoria {
        fn ida_y_vuelta(&self, linea: &str) -> Result<String, Fallo> {
            Ok(self.0.lock().unwrap().atender(linea))
        }
    }

    fn tmp(nombre: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "ore-puerta-{nombre}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn proceso(raiz: &Path, ajenos: Option<&Path>) -> Proceso {
        Proceso {
            canal: Box::new(EnMemoria(Mutex::new(de_ficheros()))),
            abrir: abrir_json(raiz, None, ajenos, Some("r")),
            abierta: OnceLock::new(),
        }
    }

    fn ml(n: u32) -> String {
        format!("gs://b/ore/v2/catalogo/ventas/default/pedidos/metadata/{n:05}-x.metadata.json")
    }

    fn poner(nombre: &str, campos: Vec<(&str, Json)>, esperado: Esperado) -> Cambio {
        Cambio {
            nombre: nombre.into(),
            que: Que::Poner {
                campos: campos.into_iter().map(|(k, v)| (k.into(), v)).collect(),
                quitar: vec!["motivo".into()],
            },
            esperado,
        }
    }

    /// Cada operación, por el protocolo, contesta lo mismo que la puerta local.
    #[test]
    fn cada_operacion_cruza_el_protocolo_igual() {
        let d = tmp("ops");
        let ajenos = tmp("ajenos");
        let p = proceso(&d, Some(&ajenos));
        let l = Ficheros::en(&d).con_ajenos(Some(&ajenos));
        let ctx = Contexto {
            autor: "ana".into(),
            operacion: "copia".into(),
            idempotencia: Some("k".into()),
        };
        assert!(p.uno("ventas.pedidos").unwrap().is_none());
        p.escribir(
            &[
                poner(
                    "ventas.pedidos",
                    vec![
                        ("metadata_location", Json::s(ml(1))),
                        ("filas", Json::Int(5)),
                        ("cinco", Json::s("5")),
                        ("datos_de", nulo()),
                        ("ok", Json::Bool(true)),
                        ("clave", Json::s("k1")),
                    ],
                    Esperado::Nada,
                ),
                Cambio {
                    nombre: "ventas.otra".into(),
                    que: Que::Sustituir(Json::obj([
                        ("kind", Json::s("MediaCollection")),
                        ("metadata_location", Json::s(ml(9))),
                    ])),
                    esperado: Esperado::Cualquiera,
                },
            ],
            &ctx,
        )
        .unwrap();
        let (a, b) = (p.uno("ventas.pedidos").unwrap().unwrap(), {
            l.uno("ventas.pedidos").unwrap().unwrap()
        });
        assert_eq!(a.texto, b.texto);
        assert_eq!(a.de, b.de);
        assert!(a.texto.contains("\"filas\": 5") && a.texto.contains("\"cinco\": \"5\""));
        assert!(a.texto.contains("\"datos_de\": null"), "{}", a.texto);
        let (ta, tb) = (p.todos().unwrap(), l.todos().unwrap());
        assert_eq!(
            ta.iter().map(|(k, v)| (k, &v.texto)).collect::<Vec<_>>(),
            tb.iter().map(|(k, v)| (k, &v.texto)).collect::<Vec<_>>()
        );
        assert_eq!(p.reclamados().unwrap(), l.reclamados().unwrap());
        assert!(
            p.reclamados()
                .unwrap()
                .colecciones
                .contains(&("catalogo/ventas/default/pedidos".into(), ml(9)))
        );
        // Retirar, por el protocolo.
        p.escribir(
            &[Cambio {
                nombre: "ventas.otra".into(),
                que: Que::Retirar,
                esperado: Esperado::Ml(ml(9)),
            }],
            &Contexto::default(),
        )
        .unwrap();
        assert!(l.uno("ventas.otra").unwrap().is_none());
        assert!(
            !d.join("datasets/ventas/default/otra.json").exists()
                && d.join("datasets/ventas/default").is_dir(),
            "retira el suyo y la carpeta que aún tiene otro se queda"
        );
        // Sin git: la historia y la versión fallan igual (503) a los dos lados.
        assert_eq!(
            p.version().unwrap_err(),
            l.version().unwrap_err(),
            "el mismo fallo"
        );
        assert_eq!(
            p.historia("ventas.pedidos", 3).unwrap_err().codigo(),
            l.historia("ventas.pedidos", 3).unwrap_err().codigo()
        );
        // Las ramas: en ficheros no hacen nada, y contestan.
        p.crear("r", "main").unwrap();
        p.traer("r").unwrap();
        p.borrar("r").unwrap();
    }

    /// La historia y la versión de un árbol de git, por el protocolo.
    #[test]
    fn la_historia_y_la_version_cruzan() {
        let d = tmp("git");
        let git = |args: &[&str]| {
            let o = Command::new("git")
                .current_dir(&d)
                .args(args)
                .output()
                .unwrap();
            assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.name", "quien"]);
        git(&["config", "user.email", "quien@invalido"]);
        git(&["config", "commit.gpgsign", "false"]);
        let p = proceso(&d, None);
        for n in 1..=2 {
            p.escribir(
                &[poner(
                    "ventas.pedidos",
                    vec![("metadata_location", Json::s(ml(n)))],
                    Esperado::Cualquiera,
                )],
                &Contexto::default(),
            )
            .unwrap();
            git(&["add", "-A"]);
            git(&["commit", "-q", "-m", "v"]);
        }
        let l = Ficheros::en(&d);
        let (ha, hb) = (
            p.historia("ventas.pedidos", 10).unwrap(),
            l.historia("ventas.pedidos", 10).unwrap(),
        );
        assert_eq!(ha.len(), 2);
        for (a, b) in ha.iter().zip(&hb) {
            assert_eq!(
                (&a.id, &a.cuando, &a.quien, &a.texto),
                (&b.id, &b.cuando, &b.quien, &b.texto)
            );
        }
        assert_eq!(p.version().unwrap(), l.version().unwrap());
    }

    /// Cada fallo vuelve como salió: el 409 con su nombre, lo esperado y lo que hay.
    #[test]
    fn cada_fallo_vuelve_como_salio() {
        for f in [
            Fallo::Conflicto {
                nombre: "ventas.a".into(),
                esperado: Esperado::Ml(ml(2)),
                hay: Some(ml(1)),
            },
            Fallo::Conflicto {
                nombre: "ventas.a".into(),
                esperado: Esperado::Nada,
                hay: None,
            },
            Fallo::Conflicto {
                nombre: "ventas.a".into(),
                esperado: Esperado::Cualquiera,
                hay: Some("null".into()),
            },
            Fallo::Nombre("sin_base".into()),
            Fallo::Protegida("main".into()),
            Fallo::Almacen("no contestó \"nadie\"\nde verdad".into()),
        ] {
            let linea = fallo_a_json(&f).jcs();
            assert!(!linea.contains('\n'), "una línea: {linea}");
            assert_eq!(responder(&linea).unwrap_err(), f, "{linea}");
        }
        // Y de punta a punta: el 409, el 422 y el 503 de verdad.
        let d = tmp("fallos");
        let p = proceso(&d, Some(&d.join("no-esta")));
        p.escribir(
            &[poner(
                "ventas.a",
                vec![("metadata_location", Json::s(ml(1)))],
                Esperado::Nada,
            )],
            &Contexto::default(),
        )
        .unwrap();
        let e = p
            .escribir(
                &[poner("ventas.a", vec![], Esperado::Ml(ml(2)))],
                &Contexto::default(),
            )
            .unwrap_err();
        assert_eq!(
            e,
            Fallo::Conflicto {
                nombre: "ventas.a".into(),
                esperado: Esperado::Ml(ml(2)),
                hay: Some(ml(1)),
            }
        );
        assert_eq!(
            p.uno("sin_base").unwrap_err(),
            Fallo::Nombre("sin_base".into())
        );
        assert_eq!(p.reclamados().unwrap_err().codigo(), 503);
        // Sin abrir, nada.
        let mut s = de_ficheros();
        let r = s.atender(&peticion("todos", vec![]));
        assert_eq!(responder(&r).unwrap_err().codigo(), 503);
    }

    /// Escribir es entero: no queda nada fuera, el de antes se retira y lo de
    /// antes se conserva al poner encima.
    #[test]
    fn local_escribe_entero_y_retira_el_de_antes() {
        let d = tmp("entero");
        std::fs::create_dir_all(d.join("datasets")).unwrap();
        std::fs::write(
            d.join("datasets/ventas_pedidos.json"),
            "{\"metadata_location\": \"m0\", \"filas\": 3, \"motivo\": \"x\"}\n",
        )
        .unwrap();
        let l = Ficheros::en(&d);
        l.escribir(
            &[poner(
                "ventas.pedidos",
                vec![("estado", Json::s("error"))],
                Esperado::Ml("m0".into()),
            )],
            &Contexto::default(),
        )
        .unwrap();
        assert!(!d.join("datasets/ventas_pedidos.json").exists());
        let t = std::fs::read_to_string(d.join("datasets/ventas/default/pedidos.json")).unwrap();
        assert!(
            t.contains("\"filas\": 3") && t.contains("\"error\"") && !t.contains("motivo"),
            "{t}"
        );
        let sobra: Vec<_> = std::fs::read_dir(&d)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(".ore-puntero"))
            .collect();
        assert!(sobra.is_empty(), "{sobra:?}");
    }
}
