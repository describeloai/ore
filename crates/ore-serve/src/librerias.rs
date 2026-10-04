//! **Las librerías de los registros** (0050 L6·2·1): la ficha de un paquete y
//! la búsqueda, para el panel Libraries y su vista detallada.
//!
//! ```text
//! GET /librerias/<node|python|jvm>/<nombre>      la ficha (`@ambito/n` y `g:a` valen)
//! GET /librerias/<node|python|jvm>               la búsqueda: el texto en `X-Ore-Buscar`
//! ```
//!
//! ⛔ Este proceso no habla TLS (`ore-cli/tests/dependencias.rs`): pregunta
//!   `ore-packages`, que habla con npm, PyPI y Maven Central y con nadie más. Lo
//!   que se le pasa es un ecosistema y un nombre o un texto —nunca una URL—, y
//!   él lo vuelve a validar.
//!
//! Lo público de un registro cambia despacio: cada respuesta se guarda
//! [`VIVE`] en memoria, y una ficha que se abre dos veces no sale dos veces.
use crate::mando;
use crate::rutas::Servidor;
use ore_core::json::Json;
use ore_entrada::http::Respuesta;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Cuánto vale una respuesta guardada.
const VIVE: Duration = Duration::from_secs(600);
/// Cuántas se guardan como mucho: pasado esto, se empieza de cero (una caché de
/// un panel, no un espejo de npm).
const CABEN: usize = 512;

/// Lo guardado: cuándo, el código y el cuerpo, por petición.
type Guardadas = Mutex<HashMap<String, (Instant, u16, String)>>;

fn guardadas() -> &'static Guardadas {
    static G: OnceLock<Guardadas> = OnceLock::new();
    G.get_or_init(|| Mutex::new(HashMap::new()))
}

/// La petición de `ore-packages`, en JSON (con el texto escapado).
fn entrada(entorno: &str, clave: &str, valor: &str) -> String {
    Json::obj([
        ("entorno", Json::s(entorno)),
        (if clave == "q" { "q" } else { "nombre" }, Json::s(valor)),
    ])
    .jcs()
}

impl Servidor {
    /// `GET /librerias/<e>/<nombre>` (ficha) y `GET /librerias/<e>` con
    /// `X-Ore-Buscar` (búsqueda).
    pub(crate) fn librerias(
        &self,
        entorno: &str,
        nombre: Option<&str>,
        buscar: Option<&str>,
    ) -> Respuesta {
        let entorno = match crate::entorno::entorno_valido(entorno) {
            Ok(e) => e,
            Err(r) => return r,
        };
        // El nombre puede llegar con su `@` codificado (`%40types/node`).
        let nombre = nombre.map(decodificar);
        let (clave, valor) = match (
            nombre.as_deref().map(str::trim).filter(|n| !n.is_empty()),
            buscar.map(str::trim).filter(|q| !q.is_empty()),
        ) {
            (Some(n), _) => ("nombre", n),
            (None, Some(q)) => ("q", q),
            (None, None) => {
                return Respuesta::error(
                    422,
                    "o un nombre en el camino (`/librerias/<e>/<nombre>`) o el texto en `X-Ore-Buscar`",
                );
            }
        };
        let llave = format!("{entorno}\u{0}{clave}\u{0}{valor}");
        if let Ok(g) = guardadas().lock()
            && let Some((cuando, codigo, cuerpo)) = g.get(&llave)
            && cuando.elapsed() < VIVE
        {
            return respuesta(*codigo, cuerpo);
        }
        let consultor = self.binario.with_file_name("ore-packages");
        let s = match mando::con_entrada(&consultor, &[], &entrada(entorno, clave, valor)) {
            Ok(s) => s,
            Err(e) => return Respuesta::error(500, e),
        };
        let motivo = || {
            s.stderr
                .lines()
                .next()
                .unwrap_or("")
                .trim_start_matches("✗ ")
                .trim()
                .to_string()
        };
        let (codigo, cuerpo) = match s.codigo {
            0 => (200, s.stdout.trim().to_string()),
            64 => (422, motivo()),
            65 => (
                404,
                format!("`{valor}` no está en el registro de {entorno}"),
            ),
            _ => (502, motivo()),
        };
        // Un fallo del registro no se guarda: el siguiente lo vuelve a intentar.
        if codigo != 502
            && let Ok(mut g) = guardadas().lock()
        {
            if g.len() >= CABEN {
                g.clear();
            }
            g.insert(llave, (Instant::now(), codigo, cuerpo.clone()));
        }
        respuesta(codigo, &cuerpo)
    }
}

/// `%40types%2Fnode` → `@types/node`; lo que no es un `%XX` válido se queda.
fn decodificar(s: &str) -> String {
    let b = s.as_bytes();
    let mut o = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(a), Some(z)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            o.push((a * 16 + z) as u8);
            i += 3;
        } else {
            o.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&o).into_owned()
}

/// 200 con el JSON de `ore-packages` tal cual; lo demás, el error con su frase.
fn respuesta(codigo: u16, cuerpo: &str) -> Respuesta {
    if codigo != 200 {
        return Respuesta::error(codigo, cuerpo.to_string());
    }
    // Un objeto, y nada más: el lector de ORE también lee YAML, y un texto
    // suelto sería un escalar.
    match ore_core::parse::parse(cuerpo).map(|n| Json::de_node(&n)) {
        Ok(j @ Json::Obj(_)) => Respuesta::ok(j),
        _ => Respuesta::error(502, "`ore-packages` no devolvió JSON"),
    }
}

#[cfg(test)]
mod prueba {
    use super::*;

    #[test]
    fn la_entrada_es_json_y_nunca_una_url() {
        assert_eq!(
            entrada("node", "nombre", "lodash"),
            r#"{"entorno":"node","nombre":"lodash"}"#
        );
        assert_eq!(
            entrada("node", "q", "date \"fns\""),
            r#"{"entorno":"node","q":"date \"fns\""}"#
        );
        assert_eq!(decodificar("%40types%2Fnode"), "@types/node");
        assert_eq!(decodificar("100%"), "100%");
    }

    #[test]
    fn la_respuesta_de_ore_packages_pasa_tal_cual() {
        let r = respuesta(
            200,
            r#"{"nombre":"lodash","ultima":"4.17.21","versiones":[{"version":"4.17.21"}]}"#,
        );
        assert_eq!(r.codigo, 200);
        assert!(
            r.cuerpo.jcs().contains(r#""ultima":"4.17.21""#),
            "{}",
            r.cuerpo.jcs()
        );
        assert_eq!(respuesta(404, "no está").codigo, 404);
        assert_eq!(respuesta(200, "no es json {").codigo, 502);
    }
}
