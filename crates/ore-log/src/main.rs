//! `ore-log` — el log de transparencia de referencia, **fuera** del compilador.
//!
//! El contrato es el de siempre y por la razón de siempre:
//!
//! - la petición entra por **stdin**, nunca por `argv`;
//! - la respuesta sale por **stdout**, en JCS;
//! - lo que haya que contar sale por **stderr**, y `ore` lo muestra literal.
//!
//! # Qué hace, y qué NO hace
//!
//! Anota entradas y sirve pruebas. **No decide nada**: no valida el paquete, no
//! comprueba la firma que anota y no rechaza una entrada por su contenido. Un
//! log que filtrara sería un log en el que hay que confiar, y el punto entero de
//! esto es no tener que hacerlo — lo que se anota se puede leer, y quien lo lea
//! saca sus conclusiones.
//!
//! Lo único que garantiza es lo que su nombre dice: **solo crece**. Una entrada
//! anotada no se borra ni se cambia, y quien tenga una raíz vieja puede
//! comprobarlo con una prueba de consistencia sin creerse nada.
//!
//! # Tres operaciones
//!
//! | `op` | Qué devuelve |
//! |---|---|
//! | `append` | dónde quedó la entrada, y la prueba de que está |
//! | `inclusion` | la prueba de una entrada que ya estaba |
//! | `consistency` | que el árbol de ahora extiende al de `from` |
//!
//! `--public` imprime la clave con la que firma su cabeza, que es lo que hay que
//! copiar a `trustedLogs`. Va por `argv` porque una clave pública es pública.
//!
//! # Y el fichero es la lista, en el orden en que llegó
//!
//! Una entrada por línea, en JCS. El orden **es** el log, así que el fichero no
//! se ordena ni se deduplica: dos veces lo mismo son dos entradas, y eso es
//! correcto — que alguien publique dos veces la misma versión es justo la clase
//! de hecho que un log existe para dejar por escrito.

use ore_core::json::Json;
use ore_core::transparencia as t;
use std::io::Read as _;
use std::process::ExitCode;

const DIR: &str = "ORE_LOG_DIR";
const LOG_ID: &str = "ORE_LOG_ID";
const ENTRADAS: &str = "entries.jsonl";
const CLAVE: &str = "log.key";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let r = match args.as_slice() {
        [bandera] if bandera == "--public" => publica(),
        [] => atender(),
        _ => Err("uso: `ore-log` (la peticion por stdin) o `ore-log --public`".into()),
    };
    match r {
        Ok(salida) => {
            println!("{salida}");
            ExitCode::SUCCESS
        }
        Err(m) => {
            eprintln!("ore-log: {m}");
            ExitCode::FAILURE
        }
    }
}

fn atender() -> Result<String, String> {
    let mut entrada = String::new();
    std::io::stdin()
        .read_to_string(&mut entrada)
        .map_err(|e| format!("no se pudo leer stdin: {e}"))?;
    if entrada.trim().is_empty() {
        return Err(
            "no llego nada por stdin. La peticion va por ahi y no por la linea \
                    de ordenes, porque `argv` lo lee cualquier proceso de la maquina"
                .into(),
        );
    }
    let p =
        ore_core::parse::parse(&entrada).map_err(|e| format!("la peticion no analiza: {e:?}"))?;
    let campo = |k: &str| p.get(k).and_then(|(_, v)| v.as_str()).map(String::from);
    let entero = |k: &str| {
        p.get(k)
            .and_then(|(_, v)| v.as_str())
            .and_then(|s| s.parse::<u64>().ok())
    };

    match campo("op").as_deref() {
        Some("append") => anotar(&campo("entry").ok_or("`append` sin `entry`")?),
        Some("inclusion") => probar(&campo("entry").ok_or("`inclusion` sin `entry`")?),
        Some("consistency") => extender(
            entero("from").ok_or("`consistency` sin `from`")?,
            entero("to"),
        ),
        Some(otra) => Err(format!("`op: {otra}` no existe")),
        None => Err("la peticion no dice que `op` se quiere".into()),
    }
}

/// Anota, y devuelve la prueba en el mismo viaje.
///
/// Juntas y no en dos pasos porque **son un solo hecho**: quien anota necesita
/// la prueba contra el árbol en el que acaba de entrar, y pedirla aparte dejaría
/// una ventana en la que el árbol ya creció y la respuesta habla de otro.
fn anotar(entrada: &str) -> Result<String, String> {
    let mut hojas = leer()?;
    let indice = hojas.len() as u64;
    hojas.push(t::hoja(entrada.as_bytes()));

    let ruta = dir()?.join(ENTRADAS);
    let anterior = std::fs::read_to_string(&ruta).unwrap_or_default();
    // Se reescribe entero en vez de abrir en modo `append` para que una linea a
    // medias no deje el log ilegible. Un log de verdad usaria algo mejor; este
    // existe para demostrar el contrato, no para aguantar carga.
    std::fs::write(&ruta, format!("{anterior}{entrada}\n"))
        .map_err(|e| format!("no se pudo escribir `{}`: {e}", ruta.display()))?;

    respuesta(&hojas, indice)
}

fn probar(entrada: &str) -> Result<String, String> {
    let hojas = leer()?;
    let buscada = t::hoja(entrada.as_bytes());
    let indice = hojas
        .iter()
        .position(|h| *h == buscada)
        .ok_or("esa entrada no esta en el log")? as u64;
    respuesta(&hojas, indice)
}

fn respuesta(hojas: &[t::Hash], indice: u64) -> Result<String, String> {
    let raiz = t::raiz(hojas);
    let tamano = hojas.len() as u64;
    Ok(Json::obj([
        ("index", Json::Int(indice as i64)),
        ("treeSize", Json::Int(tamano as i64)),
        ("root", Json::s(t::a_hex(&raiz))),
        ("rootSignature", Json::s(firmar_cabeza(tamano, &raiz)?)),
        (
            "inclusion",
            Json::Arr(
                t::camino_de_inclusion(hojas, indice)
                    .iter()
                    .map(|h| Json::s(t::a_hex(h)))
                    .collect(),
            ),
        ),
    ])
    .jcs())
}

/// La prueba entre DOS tamanos, no solo «desde X hasta ahora».
///
/// `hasta` es opcional y por defecto es el tamano actual, que es el caso comun.
/// Pero hace falta poder pedir un punto intermedio: quien consume compara la
/// cabeza que trae un paquete con la que anoto en su lock, y ninguna de las dos
/// tiene por que ser la de ahora. Un log que solo supiera hablar del presente
/// obligaria a creerse el salto.
fn extender(desde: u64, hasta: Option<u64>) -> Result<String, String> {
    let todas = leer()?;
    let tamano = hasta.unwrap_or(todas.len() as u64);
    if tamano > todas.len() as u64 {
        return Err(format!(
            "se pide el arbol de {tamano} y el log tiene {}",
            todas.len()
        ));
    }
    if desde > tamano {
        return Err(format!(
            "se pide consistencia de {desde} a {tamano}: un log no encoge"
        ));
    }
    let hojas = &todas[..tamano as usize];
    let raiz = t::raiz(hojas);
    Ok(Json::obj([
        ("treeSize", Json::Int(tamano as i64)),
        ("root", Json::s(t::a_hex(&raiz))),
        ("rootSignature", Json::s(firmar_cabeza(tamano, &raiz)?)),
        (
            "consistency",
            Json::Arr(
                t::prueba_de_consistencia(hojas, desde)
                    .iter()
                    .map(|h| Json::s(t::a_hex(h)))
                    .collect(),
            ),
        ),
    ])
    .jcs())
}

/// Las hojas, en el orden del fichero. El orden **es** el log.
fn leer() -> Result<Vec<t::Hash>, String> {
    let ruta = dir()?.join(ENTRADAS);
    let texto = std::fs::read_to_string(&ruta).unwrap_or_default();
    Ok(texto
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| t::hoja(l.as_bytes()))
        .collect())
}

fn dir() -> Result<std::path::PathBuf, String> {
    let d = std::env::var(DIR).map_err(|_| {
        format!(
            "`{DIR}` no esta definida. Este log vive en un directorio: es el caso que se \
             puede escribir sin un servicio, y el que demuestra que el contrato no depende \
             de uno"
        )
    })?;
    let d = std::path::PathBuf::from(d);
    std::fs::create_dir_all(&d).map_err(|e| format!("no se pudo crear `{}`: {e}", d.display()))?;
    Ok(d)
}

fn identidad() -> String {
    std::env::var(LOG_ID).unwrap_or_else(|_| "ore-log".into())
}

/// La clave con la que este log firma **su cabeza**, no los paquetes.
///
/// Son dos autoridades distintas y conviene no confundirlas: quien publica un
/// paquete afirma *esto es mio*; el log afirma *esto lo he visto y esta es toda
/// mi lista*. Un log que firmara paquetes seria un segundo publicador.
fn par() -> Result<ed25519_compact::KeyPair, String> {
    let ruta = dir()?.join(CLAVE);
    let texto = std::fs::read_to_string(&ruta)
        .map_err(|e| format!("no se pudo leer `{}`: {e}", ruta.display()))?;
    let semilla = ore_core::firma::hex(texto.trim())
        .filter(|s| s.len() == 32)
        .ok_or_else(|| {
            format!(
                "`{}` no es una semilla de 32 bytes en hexadecimal",
                ruta.display()
            )
        })?;
    let mut fija = [0u8; 32];
    fija.copy_from_slice(&semilla);
    Ok(ed25519_compact::KeyPair::from_seed(
        ed25519_compact::Seed::new(fija),
    ))
}

fn firmar_cabeza(tamano: u64, raiz: &t::Hash) -> Result<String, String> {
    let c = t::cabeza(&identidad(), tamano, raiz);
    Ok(ore_core::firma::a_hex(
        par()?.sk.sign(c.as_bytes(), None).as_ref(),
    ))
}

fn publica() -> Result<String, String> {
    Ok(ore_core::firma::a_hex(par()?.pk.as_ref()))
}
