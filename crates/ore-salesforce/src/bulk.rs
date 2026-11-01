//! **Bulk API 2.0 query** (ADR 0083 §1.6, X0 C): el volumen, por CSV.
//!
//! ```text
//!   POST  /jobs/query                 {operation, query, contentType: CSV, …}  → id
//!   GET   /jobs/query/<id>            hasta JobComplete        (cada sondeo es una petición: C7)
//!   GET   /jobs/query/<id>/results    ?maxRecords=N[&locator=L]   → CSV con su cabecera
//!                                     `Sforce-Locator: <siguiente>` | `null`
//!   PATCH /jobs/query/<id>            {"state":"Aborted"}      si se cancela o falla
//! ```
//!
//! Lo medido que manda aquí:
//!
//! - **Un job tarda de 3 a 23 s aunque devuelva pocas filas** (C8): solo vale la
//!   pena con volumen. Quien decide es [`crate::lectura`].
//! - **Cada sondeo cuenta como petición de API** (C7): 73 peticiones para ~18
//!   jobs. De ahí [`espera`], que crece, y no un sondeo por segundo.
//! - No admite compuestos, `GROUP BY`, `COUNT()`, `OFFSET`, subconsultas
//!   padre→hijo ni blobs (C2–C4): **las consultas del traductor nunca los
//!   llevan** (columnas planas, sin agregados).
//! - Cada página trae **su cabecera** de CSV (C1).

use crate::csv;
use crate::lectura::Control;
use crate::transporte::{Cliente, codificar};
use ore_driver::{Codigo, Fallo};
use std::time::Duration;

/// Quien recibe cada fila (las celdas, en el orden de las columnas pedidas) y
/// dice con `false` que pare: se llegó al `limit`.
pub type PorFila<'a> = dyn FnMut(&[Option<String>]) -> Result<bool, Fallo> + 'a;

/// Filas por página de resultados. Acota la memoria de una página de CSV.
pub const MAX_REGISTROS: u32 = 10_000;

/// Lo más que se espera entre dos sondeos.
const ESPERA_TOPE: u64 = 5;

/// La espera antes del sondeo `n`: 1, 2, 3, 4, 5, 5, 5… segundos. Un job de
/// 3–23 s son ~7 sondeos y no ~20 (C7, C8).
pub fn espera(n: u32) -> Duration {
    Duration::from_secs(u64::from(n + 1).min(ESPERA_TOPE))
}

/// Duerme `d` mirando cada 100 ms si hay que cortar.
fn dormir(d: Duration, ctl: &Control) -> Result<(), Fallo> {
    let paso = Duration::from_millis(100);
    let mut dormido = Duration::ZERO;
    while dormido < d {
        ctl.vigilar()?;
        std::thread::sleep(paso);
        dormido += paso;
    }
    Ok(())
}

/// La posición en la fila de CSV de cada columna pedida (sin distinguir
/// mayúsculas: Salesforce devuelve el nombre de la API tal cual, pero un
/// objeto con `__c` puede cambiar de caja).
pub fn posiciones(cabecera: &[Option<String>], columnas: &[String]) -> Result<Vec<usize>, Fallo> {
    columnas
        .iter()
        .map(|c| {
            cabecera
                .iter()
                .position(|h| h.as_deref().is_some_and(|h| h.eq_ignore_ascii_case(c)))
                .ok_or_else(|| {
                    Fallo::new(
                        Codigo::Esquema,
                        format!("Bulk no devolvió la columna `{c}` (la cabecera es {cabecera:?})"),
                    )
                })
        })
        .collect()
}

/// Lee una consulta por Bulk (`query`, o `queryAll` para incluir los borrados de la
/// Papelera, X0 D6) y entrega cada fila a `fila`, que devuelve `false`
/// para parar. Devuelve `true` si se paró así. **Si falla o se corta, aborta el
/// job**: uno vivo sigue gastando la cuota de 24 h.
pub fn ejecutar(
    c: &Cliente,
    operacion: &str,
    soql: &str,
    columnas: &[String],
    ctl: &Control,
    fila: &mut PorFila<'_>,
) -> Result<bool, Fallo> {
    let cuerpo = serde_json::json!({
        "operation": operacion,
        "query": soql,
        "contentType": "CSV",
        "columnDelimiter": "COMMA",
        "lineEnding": "LF",
    })
    .to_string();
    let r = c.enviar("POST", &c.api("/jobs/query"), Some(&cuerpo), &[])?;
    let v: serde_json::Value = serde_json::from_str(&r.cuerpo)
        .map_err(|e| Fallo::origen(format!("Bulk no contestó JSON al crear el job: {e}")))?;
    let id = v["id"]
        .as_str()
        .ok_or_else(|| Fallo::origen("Bulk no devolvió el `id` del job"))?
        .to_string();
    let hecho = esperar(c, &id, ctl).and_then(|()| paginas(c, &id, columnas, ctl, fila));
    if hecho.is_err() {
        abortar(c, &id);
    }
    hecho
}

fn esperar(c: &Cliente, id: &str, ctl: &Control) -> Result<(), Fallo> {
    let mut n = 0;
    loop {
        ctl.vigilar()?;
        let t = c.get(&c.api(&format!("/jobs/query/{id}")))?;
        let v: serde_json::Value = serde_json::from_str(&t).unwrap_or_default();
        match v["state"].as_str().unwrap_or("") {
            "JobComplete" => return Ok(()),
            e @ ("Failed" | "Aborted") => {
                return Err(Fallo::origen(format!(
                    "el job de Bulk `{id}` terminó `{e}`: {}",
                    v["errorMessage"].as_str().unwrap_or("sin motivo")
                )));
            }
            _ => {}
        }
        dormir(espera(n), ctl)?;
        n += 1;
    }
}

fn paginas(
    c: &Cliente,
    id: &str,
    columnas: &[String],
    ctl: &Control,
    fila: &mut PorFila<'_>,
) -> Result<bool, Fallo> {
    let mut locator: Option<String> = None;
    loop {
        ctl.vigilar()?;
        let mut ruta = format!("/jobs/query/{id}/results?maxRecords={MAX_REGISTROS}");
        if let Some(l) = &locator {
            ruta.push_str(&format!("&locator={}", codificar(l)));
        }
        let r = c.enviar("GET", &c.api(&ruta), None, &[])?;
        let mut filas =
            csv::leer(&r.cuerpo).map_err(|m| Fallo::origen(format!("el CSV de Bulk: {m}")))?;
        if !filas.is_empty() {
            let cabecera = filas.remove(0);
            let pos = posiciones(&cabecera, columnas)?;
            for f in &filas {
                let celdas: Vec<Option<String>> =
                    pos.iter().map(|i| f.get(*i).cloned().flatten()).collect();
                if !fila(&celdas)? {
                    return Ok(true);
                }
            }
        }
        locator = r
            .cabecera("sforce-locator")
            .filter(|l| !l.is_empty() && *l != "null")
            .map(str::to_string);
        if locator.is_none() {
            return Ok(false);
        }
    }
}

/// Aborta un job. Mejor esfuerzo: si falla, el job caduca solo.
fn abortar(c: &Cliente, id: &str) {
    let _ = c.enviar(
        "PATCH",
        &c.api(&format!("/jobs/query/{id}")),
        Some(r#"{"state":"Aborted"}"#),
        &[],
    );
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// C7: el sondeo cuesta una petición; la espera crece hasta un tope.
    #[test]
    fn la_espera_crece_y_se_para_en_cinco_segundos() {
        let v: Vec<u64> = (0..8).map(|n| espera(n).as_secs()).collect();
        assert_eq!(v, [1, 2, 3, 4, 5, 5, 5, 5]);
        // Un job de 23 s (el peor medido): 6 sondeos, no 23.
        let (mut t, mut sondeos) = (0, 0);
        while t < 23 {
            t += espera(sondeos).as_secs();
            sondeos += 1;
        }
        assert_eq!(sondeos, 7, "{t}");
    }

    fn cab(h: &[&str]) -> Vec<Option<String>> {
        h.iter().map(|x| Some(x.to_string())).collect()
    }

    #[test]
    fn las_columnas_se_buscan_en_la_cabecera_en_su_orden() {
        let p = posiciones(
            &cab(&["Name", "Id", "Industry"]),
            &["Id".into(), "Name".into()],
        )
        .unwrap();
        assert_eq!(p, [1, 0]);
        // Sin distinguir mayúsculas.
        assert_eq!(posiciones(&cab(&["x__C"]), &["x__c".into()]).unwrap(), [0]);
    }

    #[test]
    fn una_columna_que_bulk_no_devolvio_es_esquema() {
        let e = posiciones(&cab(&["Id"]), &["Name".into()]).unwrap_err();
        assert_eq!(e.codigo, Codigo::Esquema);
        assert!(e.mensaje.contains("`Name`"), "{}", e.mensaje);
    }
}
