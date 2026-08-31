//! La fase ④ y la respuesta: **ensamblar sobre flujos ya reducidos**.
//!
//! Ya reducidos, y esa es toda la diferencia. Cada lectura vino con su
//! proyección y sus claves, así que ④ junta lo poco que llegó en vez de filtrar
//! lo mucho que podría haber llegado — que es la lectura amplificada de §2 con
//! otro nombre.
//!
//! # Los ejes que la respuesta lleva
//!
//! | | Qué contesta |
//! |---|---|
//! | **digest** | qué significaba |
//! | **marca de agua** | hasta cuándo era cierto |
//! | **instante** | cuándo se autorizó |
//!
//! Los dos primeros los fijó `05-ejecutor` §7; el tercero salió del
//! [ADR 0007](../../docs/decisions/0007-enlazar-el-evaluador-de-cedar.md): Cedar
//! tiene extensión `datetime`, así que una política puede ser **función del
//! tiempo**, y sin ese eje *«la misma pregunta devolvió cosas distintas»* no se
//! distingue de un fallo.
//!
//! # El motor no lee el reloj
//!
//! **El instante llega con la petición**, igual que los atributos del principal.
//! No es escrúpulo: es lo que hace que una respuesta sea reproducible a partir de
//! sus entradas, y lo que impide que el estado degradado dependa de cuándo se
//! ejecute la prueba.

use crate::motor::Motor;
use crate::plan::{Lectura, Plan};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct Respuesta {
    /// Las filas ya ensambladas, con **propiedades** por clave.
    pub filas: Vec<BTreeMap<String, String>>,
    /// **Qué significaba.**
    pub digest: String,
    /// **Hasta cuándo era cierto.** Ausente si no intervino nada materializado.
    pub marca: Option<String>,
    /// **Cuándo se autorizó.**
    pub instante: Option<String>,
    /// Por qué la respuesta está degradada, si lo está. `05-ejecutor` §7 obliga
    /// a declararlo: un dato viejo con aspecto de fresco es peor que un error.
    pub degradado: Option<String>,
    /// Qué hay que aplicar a cada propiedad — las obligaciones que el veredicto
    /// trajo. El ejecutor las transporta; aplicarlas con sujeto es otra pieza.
    pub obligaciones: BTreeMap<String, Vec<String>>,
}

/// Un instante ISO-8601 en UTC —`AAAA-MM-DDTHH:MM:SSZ`— a segundos de época.
///
/// Se escribe aquí en vez de traer una biblioteca de fechas por la misma razón
/// que el JSON: son treinta líneas y deja el punto bajo control. El algoritmo de
/// días es el civil estándar.
pub fn epoca(iso: &str) -> Option<i64> {
    let b = iso.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let n = |a: usize, z: usize| iso.get(a..z)?.parse::<i64>().ok();
    let (y, m, d) = (n(0, 4)?, n(5, 7)?, n(8, 10)?);
    let (h, mi, s) = (n(11, 13)?, n(14, 16)?, n(17, 19)?);
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = if y2 >= 0 { y2 } else { y2 - 399 } / 400;
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let dias = era * 146_097 + doe - 719_468;
    Some(dias * 86_400 + h * 3_600 + mi * 60 + s)
}

/// `30m`, `2h`, `7d` → segundos. El vocabulario es cerrado a propósito: un
/// `freshnessSLA` con una unidad que nadie sabe leer se interpretaría como cero
/// o como infinito, y las dos lecturas son peligrosas en direcciones opuestas.
pub fn duracion(s: &str) -> Option<i64> {
    let (n, u) = s.split_at(s.len().checked_sub(1)?);
    let n: i64 = n.parse().ok()?;
    Some(match u {
        "s" => n,
        "m" => n * 60,
        "h" => n * 3_600,
        "d" => n * 86_400,
        _ => return None,
    })
}

impl Motor {
    /// `(tipo, url)` de una fuente declarada. La credencial sale de la variable
    /// que el manifiesto nombra — **nunca del documento**, que por eso es
    /// publicable.
    fn fuente(&self, nombre: &str) -> Result<(String, String), String> {
        let cfg = self
            .paquete
            .docs
            .iter()
            .find(|d| d.kind == ore_core::document::Kind::OntologyConfig)
            .ok_or("no hay manifiesto raíz")?;
        let ds = cfg
            .section("datasources")
            .ok_or("el manifiesto no declara ninguna fuente")?
            .items()
            .iter()
            .find(|d| d.get("name").and_then(|(_, v)| v.as_str()) == Some(nombre))
            .ok_or_else(|| format!("`{nombre}` no está declarada en el manifiesto"))?;
        let tipo = ds
            .get("type")
            .and_then(|(_, v)| v.as_str())
            .ok_or_else(|| format!("`{nombre}` no declara `type`"))?;
        let var = ds
            .get("connectionEnv")
            .and_then(|(_, v)| v.as_str())
            .ok_or_else(|| format!("`{nombre}` no declara `connectionEnv`"))?;
        let url = std::env::var(var)
            .map_err(|_| format!("la variable `{var}` de `{nombre}` no está puesta"))?;
        Ok((tipo.to_string(), url))
    }

    fn leer_una(&self, l: &Lectura) -> Result<Vec<BTreeMap<String, String>>, String> {
        let (tipo, url) = self.fuente(&l.datasource)?;
        let salida = std::process::Command::new(format!("ore-read-{tipo}"))
            .args(["leer", &l.datasource])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut h| {
                use std::io::Write as _;
                h.stdin
                    .as_mut()
                    .expect("stdin")
                    .write_all(l.peticion(&url).as_bytes())?;
                h.wait_with_output()
            })
            .map_err(|e| format!("no se pudo invocar `ore-read-{tipo}`: {e}"))?;
        if !salida.status.success() {
            return Err(String::from_utf8_lossy(&salida.stderr).trim().to_string());
        }
        Ok(String::from_utf8_lossy(&salida.stdout)
            .lines()
            .filter(|x| !x.trim().is_empty())
            .filter_map(|x| ore_core::parse::parse(x).ok())
            .map(|n| {
                n.entries()
                    .iter()
                    .filter_map(|(k, v)| Some((k.as_str()?.to_string(), v.as_str()?.to_string())))
                    .collect()
            })
            .collect())
    }

    /// Ejecuta un plan y devuelve la respuesta ensamblada.
    ///
    /// `instante` es **cuándo se pregunta**, y llega de fuera: el motor no lee el
    /// reloj. `sla` es el `freshnessSLA` que aplique; si se supera, la respuesta
    /// sale **degradada y diciéndolo**.
    pub fn responder(
        &self,
        p: &Plan,
        instante: Option<&str>,
        sla: Option<&str>,
    ) -> Result<Respuesta, String> {
        // ④ · ensamblar por la clave, sobre lo que ya vino reducido.
        let mut por_clave: BTreeMap<Vec<String>, BTreeMap<String, String>> = BTreeMap::new();
        let mut sueltas: Vec<BTreeMap<String, String>> = Vec::new();

        for l in &p.lecturas {
            for fila in self.leer_una(l)? {
                if p.ensamblar_por.is_empty() {
                    sueltas.push(fila);
                    continue;
                }
                let clave: Vec<String> = p
                    .ensamblar_por
                    .iter()
                    .map(|k| fila.get(k).cloned().unwrap_or_default())
                    .collect();
                // Una fila sin clave no se puede ensamblar, y meterla igual
                // juntaría cosas que no son la misma.
                if clave.iter().any(String::is_empty) {
                    continue;
                }
                por_clave.entry(clave).or_default().extend(fila);
            }
        }
        let filas: Vec<BTreeMap<String, String>> = if p.ensamblar_por.is_empty() {
            sueltas
        } else {
            por_clave.into_values().collect()
        };

        // El estado degradado. Se compara con la marca del índice, que es lo
        // único materializado que interviene en v1.
        let marca = self.topologia.as_ref().map(|t| t.marca.clone());
        let degradado = match (&marca, instante, sla) {
            (Some(m), Some(ahora), Some(sla)) => {
                match (epoca(m), epoca(ahora), duracion(sla)) {
                    (Some(m0), Some(t0), Some(d)) if t0 > m0 + d => Some(format!(
                        "la marca de agua es `{m}` y el `freshnessSLA` es `{sla}`: lo \
                         materializado lleva {} s de retraso",
                        t0 - m0
                    )),
                    _ => None,
                }
            }
            _ => None,
        };

        Ok(Respuesta {
            filas,
            digest: ore_core::digest::bundle(&self.paquete),
            marca,
            instante: instante.map(String::from),
            degradado,
            obligaciones: p.autorizadas.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_epoca_y_la_duracion_se_leen_sin_biblioteca() {
        assert_eq!(epoca("1970-01-01T00:00:00Z"), Some(0));
        // Contrastado contra `datetime` de Python, no contra la propia funcion:
        // comprobar un algoritmo con su propia salida no comprueba nada.
        assert_eq!(epoca("2026-08-31T12:00:00Z"), Some(1_788_177_600));
        // Un bisiesto, que es donde falla el aritmética escrita a ojo.
        assert_eq!(
            epoca("2024-03-01T00:00:00Z").unwrap() - epoca("2024-02-29T00:00:00Z").unwrap(),
            86_400
        );
        assert_eq!(duracion("30m"), Some(1_800));
        assert_eq!(duracion("2h"), Some(7_200));
        // Una unidad que nadie sabe leer no vale cero: no vale.
        assert_eq!(duracion("30x"), None);
    }
}
