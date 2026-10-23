//! **`POST /consultas`** (Query Q3, `docs/query.md` §3.2): la admisión de una
//! consulta de Query. `{texto, rama?}` → en qué clase corre (L, G o E), por qué y
//! con cuánta memoria e hilos, decidido antes de correr por lo que va a leer
//! (`ore explain --estimate`, en la rama: el reparto y, del lago, sus manifiestos).
//!
//! Todavía no ejecuta ni encola: eso es de los trabajadores de `ore-consulta`
//! (Q4), que es donde hace falta resolver los nombres y prestar las credenciales
//! de esta consulta. Lo que sí hace ya es decir que no a lo que no se admite —una
//! ruta con `..` (Q0b, §2.7) o lo que el reparto niega— y cuánto tardó decidir.

use ore_core::admision::{self, Umbrales};
use ore_core::json::Json;

use crate::rutas::Servidor;
use ore_entrada::http::Respuesta;
use ore_entrada::identidad::Identidad;

impl Servidor {
    pub(crate) fn admitir_consulta(&self, _sujeto: &Identidad, cuerpo: &str) -> Respuesta {
        let t0 = std::time::Instant::now();
        let n = match ore_core::parse::parse(cuerpo) {
            Ok(n) => n,
            Err(_) => return Respuesta::error(400, "el cuerpo no es JSON"),
        };
        let campo = |k: &str| n.get(k).and_then(|(_, v)| v.as_str()).map(String::from);
        let texto = campo("texto").unwrap_or_default();
        if texto.trim().is_empty() {
            return Respuesta::error(422, "una consulta quiere `texto`");
        }
        let rama = campo("rama").filter(|r| !r.is_empty() && r != "main");
        let malas = admision::rutas_sospechosas(&texto);
        if !malas.is_empty() {
            return Respuesta::error(
                422,
                format!(
                    "una ruta de la consulta sale de donde está (`..` o `%2e`): {}",
                    malas.join(", ")
                ),
            );
        }
        let linea = match self.explicar(rama.as_deref(), &texto, true, true) {
            Ok(l) => l,
            Err(r) => return r,
        };
        let plan = match ore_core::parse::parse(&linea) {
            Ok(p) => p,
            Err(_) => return Respuesta::error(502, "`ore explain` no devolvió JSON"),
        };
        // Lo que el reparto niega (gobierno, coste, interruptor), con su código.
        if plan.get("ok").and_then(|(_, v)| v.as_str()) == Some("false") {
            let c = |k: &str| {
                plan.get(k)
                    .and_then(|(_, v)| v.as_str())
                    .unwrap_or("")
                    .to_string()
            };
            let http = c("http").parse::<u16>().unwrap_or(422);
            let mut r = Respuesta::error(http, c("mensaje"));
            if let Json::Obj(m) = &mut r.cuerpo {
                m.insert("codigo".into(), Json::s(c("codigo")));
                m.insert("tabla".into(), Json::s(c("tabla")));
            }
            return r;
        }
        let entrada = admision::entrada_de_explain(&plan);
        let decision = admision::admitir(&entrada, &Umbrales::default());
        Respuesta::ok(Json::obj([
            ("admision", decision.json()),
            (
                "lee",
                Json::obj([
                    ("filas", Json::Int(entrada.filas as i64)),
                    ("bytes", Json::Int(entrada.bytes as i64)),
                    ("estimada", Json::Bool(entrada.estimada)),
                    (
                        "en_vivo",
                        Json::Arr(entrada.en_vivo.iter().map(Json::s).collect()),
                    ),
                ]),
            ),
            (
                "estimacion",
                plan.get("estimacion")
                    .map(|(_, v)| Json::de_node(v))
                    .unwrap_or_else(|| Json::obj([])),
            ),
            ("ms", Json::Int(t0.elapsed().as_millis() as i64)),
        ]))
    }
}
