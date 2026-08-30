//! `ore dev` — el servidor de contexto, por MCP sobre stdio.
//!
//! Registro: [ADR 0005](../../../docs/decisions/0005-la-superficie-de-contexto.md).
//!
//! # Qué sirve, y por qué es aburrido
//!
//! El contrato. Nada más. `ore export --format graphql` ya ejecutó los cuatro
//! pasos de `01-emision-graphql` §4 —descartar por madurez, descartar por
//! clasificación, aplicar máscaras y podar lo que quedó vacío—, así que **el
//! SDL que llega aquí ya pasó por el conducto**.
//!
//! > **El servidor no vuelve a filtrar.** Sirve lo que el contrato contiene, y
//! > nunca más de eso.
//!
//! Eso es `DESIGN` §3.8 —*la política se aplica en un punto único, nunca el
//! consumidor*— aplicado a nosotros mismos, y tiene una consecuencia que se ve
//! leyendo este fichero: **no hay una sola línea de lógica de política**. No lee
//! retículos, no compara niveles, no consulta `ConduitPolicy`. Que sea aburrido
//! es la propiedad, no la carencia.
//!
//! Y de ahí la regla para lo que venga: **toda respuesta se deriva del SDL
//! emitido, no del paquete.** Una herramienta que leyera el paquete podría
//! contar lo que el conducto quitó, y lo haría sin que nadie lo notase.
//!
//! # Por qué no hace falta un analizador de JSON
//!
//! MCP es JSON-RPC 2.0, una línea por mensaje. `ore-core` no lleva analizador de
//! JSON ([ADR 0002](../../../docs/decisions/0002-sin-validador-de-json-schema.md))
//! y no hace falta ninguno: **JSON es un subconjunto de YAML**, así que el
//! analizador de eventos que ya existe lee una petición sin enterarse de que no
//! es un documento OOS. Se emite con `json::Json`, que ya produce JCS.
//!
//! El binario sigue en cinco crates.

use ore_core::json::Json;
use ore_core::link::Package;
use ore_core::parse::{self, Node, Style};
use std::io::{BufRead, Write};

const NOMBRE: &str = "ore";
const PROTOCOLO_POR_DEFECTO: &str = "2024-11-05";

const URI_ESQUEMA: &str = "oos://schema.graphql";
const URI_BUNDLE: &str = "oos://bundle.json";

/// Lo que este proceso sirve, resuelto una vez al arrancar.
struct Contexto {
    sdl: String,
    digest: String,
    paquete: String,
}

impl Contexto {
    fn nuevo(pkg: &Package) -> Result<Self, String> {
        Ok(Contexto {
            sdl: ore_core::graphql::emit(pkg)?,
            digest: ore_core::digest::bundle(pkg),
            paquete: pkg
                .docs
                .iter()
                .find(|d| d.kind == ore_core::document::Kind::Package)
                .and_then(|d| {
                    let n = d.meta("name")?.as_str()?.to_string();
                    let v = d.meta("version")?.as_str()?.to_string();
                    Some(format!("{n}@{v}"))
                })
                .unwrap_or_else(|| "desconocido".into()),
        })
    }

    /// El bloque de un tipo, extraído **del SDL** y no del paquete: así no puede
    /// contar lo que el conducto quitó, porque no lo tiene delante.
    fn tipo(&self, nombre: &str) -> Option<String> {
        let aguja = format!("type {nombre} ");
        let i = self
            .sdl
            .find(&aguja)
            .or_else(|| self.sdl.find(&format!("type {nombre}{{")))?;
        let fin = self.sdl[i..].find("\n}")? + i + 2;
        Some(self.sdl[i..fin].to_string())
    }

    fn tipos(&self) -> Vec<String> {
        self.sdl
            .lines()
            .filter_map(|l| l.strip_prefix("type "))
            .filter_map(|l| l.split([' ', '{']).next())
            .filter(|n| *n != "Query")
            .map(String::from)
            .collect()
    }
}

// ── El bucle ────────────────────────────────────────────────────────────────

pub fn servir(pkg: &Package) -> Result<(), String> {
    let ctx = Contexto::nuevo(pkg)?;

    // A `stderr`, porque `stdout` es el canal del protocolo: una línea que no
    // sea JSON-RPC ahí rompe al cliente.
    eprintln!("ore dev · {} · {}", ctx.paquete, ctx.digest);
    eprintln!(
        "  {} tipos en el contrato. L1: no se toca un dato.",
        ctx.tipos().len()
    );

    let entrada = std::io::stdin();
    let mut salida = std::io::stdout().lock();
    for linea in entrada.lock().lines() {
        let linea = linea.map_err(|e| format!("no se pudo leer de stdin: {e}"))?;
        if linea.trim().is_empty() {
            continue;
        }
        if let Some(respuesta) = atender(&ctx, &linea) {
            writeln!(salida, "{}", respuesta.jcs())
                .map_err(|e| format!("no se pudo escribir en stdout: {e}"))?;
            salida.flush().ok();
        }
    }
    Ok(())
}

/// `None` para una notificación: JSON-RPC dice que un mensaje sin `id` no lleva
/// respuesta, y contestarla es lo que cuelga a un cliente estricto.
fn atender(ctx: &Contexto, linea: &str) -> Option<Json> {
    let peticion = match parse::parse(linea) {
        Ok(n) => n,
        // Sin `id` no hay a quién contestarle: un JSON ilegible no tiene número
        // de petición, así que se descarta en silencio como manda JSON-RPC.
        Err(_) => return None,
    };
    let metodo = campo(&peticion, "method")?.to_string();
    let id = peticion.get("id").map(|(_, v)| escalar(v));

    let resultado = match metodo.as_str() {
        "initialize" => Ok(inicializar(ctx, &peticion)),
        "ping" => Ok(Json::obj([])),
        "resources/list" => Ok(recursos()),
        "resources/read" => leer_recurso(ctx, &peticion),
        "tools/list" => Ok(herramientas()),
        "tools/call" => invocar(ctx, &peticion),
        // Las notificaciones (`notifications/…`) no llevan `id` y caen aquí:
        // el `id?` de abajo las convierte en silencio, que es lo correcto.
        otro => Err(format!("método `{otro}` no soportado")),
    };

    let id = id?;
    Some(match resultado {
        Ok(r) => Json::Obj(
            [
                ("jsonrpc".to_string(), Json::s("2.0")),
                ("id".to_string(), id),
                ("result".to_string(), r),
            ]
            .into_iter()
            .collect(),
        ),
        Err(m) => Json::Obj(
            [
                ("jsonrpc".to_string(), Json::s("2.0")),
                ("id".to_string(), id),
                (
                    "error".to_string(),
                    Json::obj([("code", Json::Int(-32601)), ("message", Json::s(m))]),
                ),
            ]
            .into_iter()
            .collect(),
        ),
    })
}

// ── Los métodos ─────────────────────────────────────────────────────────────

/// La versión del protocolo **se devuelve, no se impone**: esta superficie es de
/// solo lectura y no usa ninguna capacidad que haya cambiado entre revisiones,
/// así que rechazar a un cliente por su fecha sería rechazarlo por nada.
fn inicializar(ctx: &Contexto, p: &Node) -> Json {
    let version = p
        .get("params")
        .and_then(|(_, v)| v.get("protocolVersion"))
        .and_then(|(_, v)| v.as_str())
        .unwrap_or(PROTOCOLO_POR_DEFECTO)
        .to_string();
    Json::obj([
        ("protocolVersion", Json::s(version)),
        (
            "capabilities",
            Json::obj([("resources", Json::obj([])), ("tools", Json::obj([]))]),
        ),
        (
            "serverInfo",
            Json::obj([
                ("name", Json::s(NOMBRE)),
                ("version", Json::s(env!("CARGO_PKG_VERSION"))),
            ]),
        ),
        (
            "instructions",
            Json::s(
                "Sirve el CONTRATO de una ontologia OOS: que se puede pedir y con que forma. \
                 El esquema ya paso por el conducto `contextSurface`, asi que lo que no esta \
                 en el no existe para este consumidor — no esta prohibido, esta ausente. \
                 No hay acceso a datos: es nivel L1.",
            ),
        ),
        ("_meta", meta(ctx)),
    ])
}

fn recursos() -> Json {
    Json::obj([(
        "resources",
        Json::Arr(vec![
            Json::obj([
                ("uri", Json::s(URI_ESQUEMA)),
                ("name", Json::s("Contrato de consumo (GraphQL SDL)")),
                ("mimeType", Json::s("application/graphql")),
                (
                    "description",
                    Json::s(
                        "El esquema emitido del bundle. Contiene exactamente lo que el \
                         conducto `contextSurface` admite.",
                    ),
                ),
            ]),
            Json::obj([
                ("uri", Json::s(URI_BUNDLE)),
                ("name", Json::s("Identidad del bundle")),
                ("mimeType", Json::s("application/json")),
                (
                    "description",
                    Json::s("El digest del que salio este contrato, y el paquete que lo produjo."),
                ),
            ]),
        ]),
    )])
}

fn leer_recurso(ctx: &Contexto, p: &Node) -> Result<Json, String> {
    let uri = p
        .get("params")
        .and_then(|(_, v)| v.get("uri"))
        .and_then(|(_, v)| v.as_str())
        .ok_or("falta `uri`")?;
    let (mime, texto) = match uri {
        URI_ESQUEMA => ("application/graphql", ctx.sdl.clone()),
        URI_BUNDLE => (
            "application/json",
            Json::obj([
                ("bundle", Json::s(&ctx.digest)),
                ("package", Json::s(&ctx.paquete)),
                ("level", Json::s("L1")),
            ])
            .pretty(),
        ),
        otro => return Err(format!("recurso `{otro}` desconocido")),
    };
    Ok(Json::obj([(
        "contents",
        Json::Arr(vec![Json::obj([
            ("uri", Json::s(uri)),
            ("mimeType", Json::s(mime)),
            ("text", Json::s(texto)),
        ])]),
    )]))
}

fn herramientas() -> Json {
    let sin_argumentos = Json::obj([
        ("type", Json::s("object")),
        ("properties", Json::obj([])),
        ("required", Json::Arr(vec![])),
    ]);
    let un_tipo = Json::obj([
        ("type", Json::s("object")),
        (
            "properties",
            Json::obj([(
                "type",
                Json::obj([
                    ("type", Json::s("string")),
                    (
                        "description",
                        Json::s("Nombre del tipo, tal como aparece en el contrato."),
                    ),
                ]),
            )]),
        ),
        ("required", Json::Arr(vec![Json::s("type")])),
    ]);
    Json::obj([(
        "tools",
        Json::Arr(vec![
            Json::obj([
                ("name", Json::s("ontology_schema")),
                (
                    "description",
                    Json::s(
                        "Devuelve el contrato completo en GraphQL SDL: los tipos que se \
                         pueden pedir, sus campos y sus claves.",
                    ),
                ),
                ("inputSchema", sin_argumentos),
            ]),
            Json::obj([
                ("name", Json::s("ontology_describe")),
                (
                    "description",
                    Json::s(
                        "Describe un tipo del contrato. Lo extrae del propio esquema, asi \
                         que no puede contar nada que el contrato no diga.",
                    ),
                ),
                ("inputSchema", un_tipo),
            ]),
        ]),
    )])
}

fn invocar(ctx: &Contexto, p: &Node) -> Result<Json, String> {
    let params = p.get("params").map(|(_, v)| v).ok_or("faltan `params`")?;
    let nombre = params
        .get("name")
        .and_then(|(_, v)| v.as_str())
        .ok_or("falta `name`")?;
    match nombre {
        "ontology_schema" => Ok(texto(ctx, ctx.sdl.clone())),
        "ontology_describe" => {
            let tipo = params
                .get("arguments")
                .and_then(|(_, a)| a.get("type"))
                .and_then(|(_, v)| v.as_str())
                .ok_or("falta el argumento `type`")?;
            match ctx.tipo(tipo) {
                Some(bloque) => Ok(texto(ctx, bloque)),
                // Un tipo ausente y un tipo podado por el conducto dan la MISMA
                // respuesta, y tiene que ser asi: distinguirlos revelaria que
                // existe algo que el contrato no declara.
                //
                // Y NO se devuelve el nombre pedido. Repetirlo seria inofensivo
                // —quien pregunta ya lo conocia— pero deja de valer la invariante
                // que hace legible este servidor: TODA cadena que emite viene del
                // contrato. Una propiedad absoluta se comprueba; una con una
                // excepcion razonable, no.
                None => Ok(texto(
                    ctx,
                    format!(
                        "Ese tipo no esta en el contrato.\nTipos disponibles: {}",
                        ctx.tipos().join(", ")
                    ),
                )),
            }
        }
        otro => Err(format!("herramienta `{otro}` desconocida")),
    }
}

fn texto(ctx: &Contexto, cuerpo: String) -> Json {
    Json::obj([
        (
            "content",
            Json::Arr(vec![Json::obj([
                ("type", Json::s("text")),
                ("text", Json::s(cuerpo)),
            ])]),
        ),
        ("isError", Json::Bool(false)),
        ("_meta", meta(ctx)),
    ])
}

/// El digest viaja con cada respuesta. `DESIGN` §3.4 promete que *«¿que sabia el
/// agente el martes a las 14:32?»* se contesta con un commit y una marca de
/// agua; esto es esa marca de agua, puesta donde se usa.
fn meta(ctx: &Contexto) -> Json {
    Json::obj([(
        "oos",
        Json::obj([
            ("bundle", Json::s(&ctx.digest)),
            ("package", Json::s(&ctx.paquete)),
        ]),
    )])
}

// ── Lectura de la peticion ──────────────────────────────────────────────────

fn campo<'a>(n: &'a Node, clave: &str) -> Option<&'a str> {
    n.get(clave).and_then(|(_, v)| v.as_str())
}

/// El `id` se devuelve **con el tipo con el que llego**. El analizador conserva
/// el estilo del escalar, asi que un `1` sin comillas vuelve como numero y un
/// `"a"` como cadena — que es lo que un cliente estricto comprueba.
fn escalar(v: &Node) -> Json {
    match v {
        Node::Scalar {
            raw,
            style: Style::Plain,
            ..
        } => raw.parse::<i64>().map(Json::Int).unwrap_or(Json::s(raw)),
        Node::Scalar { raw, .. } => Json::s(raw),
        _ => Json::s(""),
    }
}
