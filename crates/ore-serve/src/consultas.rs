//! **`POST /consultas`** (Query Q3, `docs/query.md` §3.2): la admisión de una
//! consulta de Query. `{texto, rama?}` → en qué clase corre (L, G o E), por qué y
//! con cuánta memoria e hilos, decidido antes de correr por lo que va a leer
//! (`ore explain --estimate`, en la rama: el reparto y, del lago, sus manifiestos).
//!
//! Todavía no ejecuta ni encola: eso es de los trabajadores de `ore-consulta`
//! (Q4), que es donde hace falta resolver los nombres y prestar las credenciales
//! de esta consulta. Lo que sí hace ya es decir que no a lo que no se admite —una
//! ruta con `..` (Q0b, §2.7) o lo que el reparto niega— y cuánto tardó decidir.

use ore_core::admision;
use ore_core::json::Json;

use crate::rutas::Servidor;
use ore_entrada::http::Respuesta;
use ore_entrada::identidad::Identidad;

impl Servidor {
    pub(crate) fn admitir_consulta(&self, sujeto: &Identidad, cuerpo: &str) -> Respuesta {
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
        // Query A3 · por el plan, del puesto de la hoja de quien pregunta.
        let hoja = self.hoja_de(sujeto);
        let Some(id) = hoja else {
            return Respuesta::error(
                409,
                "no Query sheet is open: the plan comes from its session (open the sheet first)",
            );
        };
        let a = self.admision_de_lo_pulsado(sujeto, &id, rama.as_deref(), &texto);
        Respuesta::ok(Json::obj([
            (
                "admision",
                a.map(|a| a.json()).unwrap_or_else(|| {
                    Json::obj([
                        ("clase", Json::s("L")),
                        ("razon", Json::s("nothing to weigh: no plan")),
                    ])
                }),
            ),
            ("ms", Json::Int(t0.elapsed().as_millis() as i64)),
        ]))
    }
}
