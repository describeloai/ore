//! **Lo que cada conector declara** (`docs/federation.md` §1.4), leído de su
//! verbo `capacidades` al arrancar, y la comprobación de una petición contra
//! ello **antes** de tocar el origen.
//!
//! Es la versión dinámica de `ore_driver::Capacidades::admite`: aquí el
//! conector es otro proceso y lo que sabe se lee, no se enlaza.

use ore_driver::{Fallo, Peticion};
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Capacidades {
    /// Lo que el conector contestó, tal cual: `GET /v1/connectors` lo sirve.
    pub json: String,
    pub protocolo: i64,
    pub operadores: Vec<String>,
    pub limit: bool,
    pub order_by: bool,
    pub servir: bool,
}

impl Capacidades {
    /// La línea que contesta `capacidades`.
    pub fn leer(texto: &str) -> Result<Capacidades, String> {
        let n = ore_core::parse::parse(texto.trim())
            .map_err(|e| format!("`capacidades` no es JSON: {e:?}"))?;
        // El analizador deja los escalares como texto: `true` es "true".
        let si = |k: &str| n.get(k).and_then(|(_, v)| v.as_str()) == Some("true");
        Ok(Capacidades {
            json: texto.trim().to_string(),
            protocolo: n
                .get("protocolo")
                .and_then(|(_, v)| v.as_str())
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            operadores: n
                .get("operadores")
                .map(|(_, v)| {
                    v.items()
                        .iter()
                        .filter_map(|o| o.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            limit: si("limit"),
            order_by: si("orderBy"),
            servir: si("servir"),
        })
    }

    /// Pregunta al conector: `<programa> capacidades`, con 10 s de plazo.
    pub fn preguntar(programa: &Path) -> Result<Capacidades, String> {
        use std::process::{Command, Stdio};
        let mut hijo = Command::new(programa)
            .arg("capacidades")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("no arranca `{}`: {e}", programa.display()))?;
        let t = std::time::Instant::now();
        loop {
            match hijo.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if t.elapsed() > Duration::from_secs(10) => {
                    let _ = hijo.kill();
                    let _ = hijo.wait();
                    return Err(format!("`{}` no contesta en 10 s", programa.display()));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
                Err(e) => return Err(e.to_string()),
            }
        }
        let mut texto = String::new();
        if let Some(mut s) = hijo.stdout.take() {
            let _ = std::io::Read::read_to_string(&mut s, &mut texto);
        }
        let c = Capacidades::leer(&texto)?;
        if c.protocolo < 2 || !c.servir {
            return Err(format!(
                "`{}` no es un conector v2 que sepa `servir`",
                programa.display()
            ));
        }
        Ok(c)
    }

    /// **Si esta petición se puede servir tal cual**, o el `operador` que lo
    /// impide: lo que el conector no declaró no viaja (ignorarlo devolvería
    /// otras filas sin fallar).
    pub fn admite(&self, p: &Peticion) -> Result<(), Fallo> {
        for f in &p.filtros {
            if !self.operadores.iter().any(|o| o == &f.operador) {
                return Err(Fallo::operador(format!(
                    "`{}` sobre `{}`: el conector no lo sabe poner (sabe {})",
                    f.operador,
                    f.columna,
                    self.operadores.join(", ")
                )));
            }
        }
        if p.limit.is_some() && !self.limit {
            return Err(Fallo::operador("`limit`: el conector no lo sabe poner"));
        }
        if !p.orden.is_empty() && !self.order_by {
            return Err(Fallo::operador("`orderBy`: el conector no lo sabe poner"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lee_lo_que_declara_y_niega_lo_demas() {
        let c = Capacidades::leer(
            r#"{"agregados":false,"conector":"s3","estimar":true,"juntas":false,"limit":true,"operadores":["eq","in"],"orderBy":false,"protocolo":2,"servir":true,"version":"2.0.0"}"#,
        )
        .unwrap();
        assert_eq!(c.operadores, ["eq", "in"]);
        assert!(c.limit && !c.order_by && c.servir);
        assert_eq!(c.protocolo, 2);
        let p = ore_driver::leer_peticion(
            r#"{"objeto":"t","url":"x://h","proyeccion":{"a":"a"},"filtros":[{"columna":"a","operador":"like","valor":"x%"}]}"#,
        )
        .unwrap();
        let f = c.admite(&p).unwrap_err();
        assert_eq!(f.codigo, ore_driver::Codigo::Operador);
        let p = ore_driver::leer_peticion(
            r#"{"objeto":"t","url":"x://h","proyeccion":{"a":"a"},"orderBy":[{"columna":"a"}]}"#,
        )
        .unwrap();
        assert!(c.admite(&p).is_err(), "orderBy sin declararlo");
        let p = ore_driver::leer_peticion(
            r#"{"objeto":"t","url":"x://h","proyeccion":{"a":"a"},"filtros":[{"columna":"a","operador":"in","valor":["1","2"]}],"limit":3}"#,
        )
        .unwrap();
        assert!(c.admite(&p).is_ok());
    }
}
