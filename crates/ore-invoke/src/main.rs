//! `ore-invoke` — **el invocador delegado**, fuera del compilador (ADR 0029 ⑤).
//!
//! `ore` no abre un socket, y una `Function` con `runtime: model` es una
//! llamada por la red: al gateway de modelos de la plataforma, con el token de
//! la celda. Así que la llamada la hace este programa, y `ore invoke` lo
//! canaliza como canaliza a `ore-read-<tipo>` y a `ore-store-<tipo>`.
//!
//! # El protocolo
//!
//! - **stdin**: la petición en JSON, **una línea** —la puerta, el id servido,
//!   el `prompt`, la forma de `output` (campo → tipo) y, si se quiere, la
//!   concurrencia y `max_tokens`—, y después **las filas de `over`, una por
//!   línea**, como objetos JSON de cadenas: exactamente lo que `ore-store-gcs
//!   leer` produce;
//! - **stdout**: **una línea por fila, en el mismo orden**: `{"fila": …,
//!   "output": …, "tokens": {"entrada", "salida"}, "ms"}` si el modelo
//!   contestó, `{"fila": …, "error": "…"}` si no. Una fila que falla no tumba
//!   la corrida: se dice y se sigue;
//! - **stderr**: lo que haya que contar.
//!
//! El token va en el entorno (`MODELO_TOKEN`) y no en la petición: la petición
//! se imprime en los registros del Job y el token no.
//!
//! # Lo que este programa NO sabe
//!
//! Qué es una vista, ni una entidad, ni un conducto, ni dónde aterriza lo que
//! devuelve. Recibe filas y un prompt, y devuelve lo que el modelo dijo de cada
//! una, ya en la forma que `output` declara. Es a propósito tan tonto como el
//! lector y el almacén: la única capacidad que añade es hablar con la puerta.
//!
//! # La forma de la respuesta
//!
//! Al modelo se le pide **un objeto JSON con exactamente las claves de
//! `output`**, y se toma el primer objeto que aparezca en lo que contesta. Si
//! `output` está vacío (la gramática lo admite: leer y no devolver nada), lo
//! que contesta va entero bajo `texto`. `temperature: 0`, porque el mismo
//! commit sobre la misma copia tiene que dar, en lo que de un modelo depende,
//! la misma respuesta.

use ore_core::json::Json;
use ore_core::parse::{self, Node};
use std::collections::BTreeMap;
use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::Instant;

const AGENTE: &str = "ore-invoke/0.1";

#[derive(Debug)]
struct Peticion {
    puerta: String,
    modelo: String,
    prompt: String,
    output: BTreeMap<String, String>,
    concurrencia: usize,
    max_tokens: u32,
}

fn main() -> std::process::ExitCode {
    match correr() {
        Ok(salida) => {
            print!("{salida}");
            std::process::ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn correr() -> Result<String, String> {
    let mut texto = String::new();
    std::io::stdin()
        .read_to_string(&mut texto)
        .map_err(|e| format!("no se pudo leer la entrada: {e}"))?;
    let mut lineas = texto.lines().filter(|l| !l.trim().is_empty());
    let primera = lineas
        .next()
        .ok_or("la entrada está vacía: se esperaba la petición en la primera línea")?;
    let pet = Arc::new(peticion(primera)?);
    let filas: Vec<String> = lineas.map(String::from).collect();
    let token = std::env::var("MODELO_TOKEN").ok().filter(|t| !t.is_empty());
    if token.is_none() {
        eprintln!("aviso · sin `MODELO_TOKEN`: la puerta pedirá identidad");
    }
    let token = Arc::new(token);

    // N hilos sobre una cola de índices; cada uno deja su línea en su sitio.
    // El orden de salida es el de entrada: quien lea puede casar fila y
    // respuesta sin buscar.
    let resultados: Arc<Mutex<Vec<Option<String>>>> = Arc::new(Mutex::new(vec![None; filas.len()]));
    let filas = Arc::new(filas);
    let siguiente = Arc::new(Mutex::new(0usize));
    let hilos = pet.concurrencia.clamp(1, 16).min(filas.len().max(1));
    let mut manos = Vec::with_capacity(hilos);
    for _ in 0..hilos {
        let (pet, filas, token, resultados, siguiente) = (
            Arc::clone(&pet),
            Arc::clone(&filas),
            Arc::clone(&token),
            Arc::clone(&resultados),
            Arc::clone(&siguiente),
        );
        manos.push(std::thread::spawn(move || {
            let agente = cliente();
            loop {
                let i = {
                    let mut s = siguiente.lock().unwrap();
                    let i = *s;
                    if i >= filas.len() {
                        break;
                    }
                    *s += 1;
                    i
                };
                let linea = una(&agente, &pet, token.as_deref(), &filas[i]);
                resultados.lock().unwrap()[i] = Some(linea);
            }
        }));
    }
    for m in manos {
        let _ = m.join();
    }
    let mut out = String::new();
    for l in resultados.lock().unwrap().iter().flatten() {
        out.push_str(l);
        out.push('\n');
    }
    Ok(out)
}

fn peticion(linea: &str) -> Result<Peticion, String> {
    let n = parse::parse(linea).map_err(|e| format!("la petición no analiza: {e:?}"))?;
    let s = |k: &str| -> Result<String, String> {
        n.get(k)
            .and_then(|(_, v)| v.as_str())
            .filter(|v| !v.is_empty())
            .map(String::from)
            .ok_or_else(|| format!("a la petición le falta `{k}`"))
    };
    let entero = |k: &str, por_defecto: u64| -> u64 {
        n.get(k)
            .and_then(|(_, v)| v.as_str())
            .and_then(|v| v.parse().ok())
            .unwrap_or(por_defecto)
    };
    let output = n
        .get("output")
        .map(|(_, o)| {
            o.entries()
                .iter()
                .filter_map(|(k, v)| {
                    Some((
                        k.as_str()?.to_string(),
                        v.as_str().unwrap_or("String").to_string(),
                    ))
                })
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    Ok(Peticion {
        puerta: s("puerta")?.trim_end_matches('/').to_string(),
        modelo: s("modelo")?,
        prompt: s("prompt")?,
        output,
        concurrencia: entero("concurrencia", 4) as usize,
        max_tokens: entero("max_tokens", 64) as u32,
    })
}

fn cliente() -> ureq::Agent {
    let mut b = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(120));
    if let Ok(tls) = native_tls::TlsConnector::new() {
        b = b.tls_connector(Arc::new(tls));
    }
    b.build()
}

/// Una fila: el mensaje, la llamada, y lo que vuelve ya en la forma de `output`.
fn una(agente: &ureq::Agent, pet: &Peticion, token: Option<&str>, fila: &str) -> String {
    let fila_json = match objeto_plano(fila) {
        Ok(f) => f,
        Err(e) => return Json::obj([("fila", Json::s(fila)), ("error", Json::s(e))]).jcs(),
    };
    let fila_out = Json::Obj(
        fila_json
            .iter()
            .map(|(k, v)| (k.clone(), Json::s(v)))
            .collect(),
    );
    match llamar(agente, pet, token, &fila_json) {
        Ok((output, entrada, salida, ms)) => Json::obj([
            ("fila", fila_out),
            ("ms", Json::Int(ms)),
            ("output", output),
            (
                "tokens",
                Json::obj([
                    ("entrada", Json::Int(entrada)),
                    ("salida", Json::Int(salida)),
                ]),
            ),
        ])
        .jcs(),
        Err(e) => Json::obj([("fila", fila_out), ("error", Json::s(e))]).jcs(),
    }
}

fn objeto_plano(linea: &str) -> Result<BTreeMap<String, String>, String> {
    let n = parse::parse(linea).map_err(|e| format!("la fila no analiza: {e:?}"))?;
    let mut out = BTreeMap::new();
    for (k, v) in n.entries() {
        let Some(nombre) = k.as_str() else { continue };
        let Some(x) = v.as_str() else {
            return Err(format!(
                "`{nombre}` no es un escalar: una fila es un objeto plano"
            ));
        };
        out.insert(nombre.to_string(), x.to_string());
    }
    Ok(out)
}

fn llamar(
    agente: &ureq::Agent,
    pet: &Peticion,
    token: Option<&str>,
    fila: &BTreeMap<String, String>,
) -> Result<(Json, i64, i64, i64), String> {
    let sistema = if pet.output.is_empty() {
        pet.prompt.clone()
    } else {
        let forma = pet
            .output
            .iter()
            .map(|(k, t)| format!("\"{k}\": {t}"))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "{}\n\nContesta SOLO con un objeto JSON con exactamente estas claves: {{{forma}}}. Sin texto alrededor.",
            pet.prompt
        )
    };
    let usuario = fila
        .iter()
        .map(|(k, v)| format!("{k}: {v}"))
        .collect::<Vec<_>>()
        .join("\n");
    let cuerpo = Json::obj([
        ("max_tokens", Json::Int(pet.max_tokens as i64)),
        (
            "messages",
            Json::Arr(vec![
                Json::obj([("role", Json::s("system")), ("content", Json::s(&sistema))]),
                Json::obj([("role", Json::s("user")), ("content", Json::s(&usuario))]),
            ]),
        ),
        ("model", Json::s(&pet.modelo)),
        ("temperature", Json::Int(0)),
    ])
    .jcs();

    let t0 = Instant::now();
    // La puerta puede estar un momento sin backend (medido en I4: el gateway
    // da por caído un backend cuya sonda tarda más de 3 s y lo recupera 5 s
    // después) y una fila no es culpable de eso: 502, 503, 429 y un fallo de
    // transporte se reintentan tres veces con espera creciente. Cualquier
    // otro código —400, 401, 404— es una respuesta, y se dice a la primera.
    let mut texto = None;
    let mut ultimo = String::new();
    for (intento, espera) in [0u64, 2, 5, 10].into_iter().enumerate() {
        if espera > 0 {
            std::thread::sleep(std::time::Duration::from_secs(espera));
        }
        let mut req = agente
            .post(&format!("{}/chat/completions", pet.puerta))
            .set("user-agent", AGENTE)
            .set("content-type", "application/json");
        if let Some(t) = token {
            req = req.set("authorization", &format!("Bearer {t}"));
        }
        match req.send_string(&cuerpo) {
            Ok(r) => {
                texto = Some(
                    r.into_string()
                        .map_err(|e| format!("la respuesta no se pudo leer: {e}"))?,
                );
                break;
            }
            Err(ureq::Error::Status(c, r)) => {
                let cuerpo = r.into_string().unwrap_or_default();
                let linea = cuerpo
                    .lines()
                    .next()
                    .unwrap_or("")
                    .chars()
                    .take(160)
                    .collect::<String>();
                ultimo = format!("la puerta contestó {c}: {linea}");
                if !matches!(c, 502 | 503 | 429) {
                    return Err(ultimo);
                }
            }
            Err(ureq::Error::Transport(t)) => ultimo = format!("no se alcanzó la puerta: {t}"),
        }
        if intento == 3 {
            return Err(format!("{ultimo} (tras 4 intentos)"));
        }
    }
    let texto = texto.ok_or_else(|| ultimo.clone())?;
    let ms = t0.elapsed().as_millis() as i64;
    let n = parse::parse(&texto).map_err(|e| format!("la respuesta no analiza: {e:?}"))?;
    let contenido = n
        .get("choices")
        .and_then(|(_, c)| c.items().first())
        .and_then(|c| c.get("message"))
        .and_then(|(_, m)| m.get("content"))
        .and_then(|(_, c)| c.as_str())
        .ok_or("la respuesta no trae `choices[0].message.content`")?
        .to_string();
    let uso = |k: &str| -> i64 {
        n.get("usage")
            .and_then(|(_, u)| u.get(k))
            .and_then(|(_, v)| v.as_str())
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    };
    let output = if pet.output.is_empty() {
        Json::obj([("texto", Json::s(contenido.trim()))])
    } else {
        extraer(&contenido, &pet.output)?
    };
    Ok((output, uso("prompt_tokens"), uso("completion_tokens"), ms))
}

/// El primer objeto JSON de lo que el modelo dijo, con las claves de `output`
/// y nada más. Un campo que el modelo no dio queda ausente (un nulo es la
/// propiedad ausente, como en el driver); si no hay objeto, es un error de esa
/// fila, dicho con lo que contestó.
fn extraer(contenido: &str, output: &BTreeMap<String, String>) -> Result<Json, String> {
    // El PRIMER objeto equilibrado, no «del primer `{` al último `}`»: un modelo
    // base sigue hablando después de contestar (medido con DeepSeek-V2-Lite el
    // 2026-09-17: inventa turnos `User:`/`Assistant:` con más objetos dentro), y
    // lo que vale es lo que dijo primero.
    let (Some(a), Some(z)) = (contenido.find('{'), primer_objeto(contenido)) else {
        return Err(format!(
            "el modelo no contestó un objeto JSON: {}",
            contenido.trim().chars().take(120).collect::<String>()
        ));
    };
    let n: Node = parse::parse(&contenido[a..=z]).map_err(|_| {
        format!(
            "lo que el modelo contestó no analiza: {}",
            &contenido[a..=z]
        )
    })?;
    let mut m = BTreeMap::new();
    for k in output.keys() {
        if let Some((_, v)) = n.get(k) {
            if let Some(s) = v.as_str() {
                m.insert(k.clone(), Json::s(s));
            } else {
                // Un objeto o una lista donde se esperaba un escalar: se guarda
                // su forma canónica, que es lo más honesto que se puede decir.
                m.insert(k.clone(), Json::s(a_json(v).jcs()));
            }
        }
    }
    if m.is_empty() {
        return Err(format!(
            "el modelo contestó un objeto sin ninguna de las claves de `output`: {}",
            &contenido[a..=z]
        ));
    }
    Ok(Json::Obj(m))
}

/// Dónde cierra el primer `{` del texto, contando llaves fuera de cadenas.
fn primer_objeto(s: &str) -> Option<usize> {
    let a = s.find('{')?;
    let (mut nivel, mut en_cadena, mut escapada) = (0usize, false, false);
    for (i, c) in s[a..].char_indices() {
        if en_cadena {
            match c {
                _ if escapada => escapada = false,
                '\\' => escapada = true,
                '"' => en_cadena = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => en_cadena = true,
            '{' => nivel += 1,
            '}' => {
                nivel -= 1;
                if nivel == 0 {
                    return Some(a + i);
                }
            }
            _ => {}
        }
    }
    None
}

fn a_json(n: &Node) -> Json {
    if let Some(s) = n.as_str() {
        return Json::s(s);
    }
    if !n.entries().is_empty() {
        return Json::Obj(
            n.entries()
                .iter()
                .filter_map(|(k, v)| Some((k.as_str()?.to_string(), a_json(v))))
                .collect(),
        );
    }
    Json::Arr(n.items().iter().map(a_json).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output() -> BTreeMap<String, String> {
        [("categoriaEs".to_string(), "String".to_string())].into()
    }

    #[test]
    fn extrae_el_objeto_aunque_el_modelo_hable_alrededor() {
        let j = extraer("Claro: {\"categoriaEs\": \"Bebés\"} ¿algo más?", &output()).unwrap();
        assert_eq!(j.jcs(), "{\"categoriaEs\":\"Bebés\"}");
    }

    #[test]
    fn se_queda_con_el_primer_objeto_aunque_el_modelo_siga_hablando() {
        let c = "{
  \"categoriaEs\": \"Telefonía\"
}

User: otra cosa

Assistant:
{
  \"categoriaEs\": \"Otra\"
}";
        assert_eq!(
            extraer(c, &output()).unwrap().jcs(),
            "{\"categoriaEs\":\"Telefonía\"}"
        );
        let c = "{\"categoriaEs\": \"Llaves { } dentro\"} y {\"categoriaEs\": \"no\"}";
        assert_eq!(
            extraer(c, &output()).unwrap().jcs(),
            "{\"categoriaEs\":\"Llaves { } dentro\"}"
        );
    }

    #[test]
    fn sin_objeto_es_un_error_de_esa_fila() {
        let e = extraer("bebés", &output()).unwrap_err();
        assert!(e.contains("no contestó un objeto JSON"), "{e}");
        let e = extraer("{\"otra\": 1}", &output()).unwrap_err();
        assert!(e.contains("sin ninguna de las claves"), "{e}");
    }

    #[test]
    fn la_peticion_lleva_lo_que_hace_falta_y_defaults() {
        let p = peticion("{\"puerta\":\"http://x:8000/v1/\",\"modelo\":\"m\",\"prompt\":\"p\",\"output\":{\"a\":\"String\"}}").unwrap();
        assert_eq!(p.puerta, "http://x:8000/v1");
        assert_eq!(p.concurrencia, 4);
        assert_eq!(p.max_tokens, 64);
        assert!(
            peticion("{\"puerta\":\"x\"}")
                .unwrap_err()
                .contains("`modelo`")
        );
    }
}
