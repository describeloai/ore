//! Elastic Sessions S4a (docs/elastic-sessions.md §1.7) · **Lo que una consulta
//! ya vista va a necesitar**: el escalón en que acabó la otra vez, para que la
//! próxima empiece ahí, de un golpe, en vez de subir de uno en uno.
//!
//! - **La llave** es el `fingerprint` de executions; lo recordado es de la celda
//!   (este `ore-serve`), no de la persona (D-S4-1).
//! - **Manda la última** ejecución de cada `fingerprint` (D-S4-2), con las reglas
//!   de [`lo_que_ensena`].
//! - En memoria (D-S4-4): tope [`TOPE`], se olvida la más vieja.
//!
//! Crecer mientras corre (el vigilante) sigue siempre: si la previsión se queda
//! corta, sube; si se pasa, la sesión baja a los 10 min.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::ejecuciones::Ejecucion;
use crate::elastica::{ESCALONES, escalon};
use crate::puestos::Puesto;

/// Cuántas consultas se recuerdan.
pub const TOPE: usize = 10_000;

/// El `reason` del crecimiento que pide la previsión (executions, `growth`).
pub const MOTIVO: &str = "forecast";

/// Lo recordado: `fingerprint → (escalón, cuándo)`, por orden de llegada.
#[derive(Debug, Default)]
pub(crate) struct Prevision {
    memoria: Mutex<Memoria>,
}

#[derive(Debug, Default)]
struct Memoria {
    escalones: HashMap<String, (usize, u64)>,
    reloj: u64,
}

impl Prevision {
    /// Apunta lo que enseña `e`, si enseña algo.
    pub(crate) fn aprender(&self, e: &Ejecucion) {
        if let Some(i) = lo_que_ensena(e) {
            self.recordar(crate::ejecuciones::huella(&e.statement), i);
        }
    }

    pub(crate) fn recordar(&self, huella: String, i: usize) {
        let mut m = self.memoria.lock().unwrap();
        m.reloj += 1;
        let ahora = m.reloj;
        if m.escalones.len() >= TOPE && !m.escalones.contains_key(&huella) {
            // La más vieja se olvida (pasa sólo con la memoria llena).
            if let Some(vieja) = m
                .escalones
                .iter()
                .min_by_key(|(_, (_, cuando))| *cuando)
                .map(|(k, _)| k.clone())
            {
                m.escalones.remove(&vieja);
            }
        }
        m.escalones.insert(huella, (i, ahora));
    }

    /// El escalón recordado para `huella`.
    pub(crate) fn prever(&self, huella: &str) -> Option<usize> {
        self.memoria
            .lock()
            .unwrap()
            .escalones
            .get(huella)
            .map(|(i, _)| *i)
    }
}

/// **Qué escalón enseña una ejecución terminada** (§1.7), o nada:
///
/// - sólo del session pool (`memory_final_mb`);
/// - murió por memoria (`OutOfMemoryError`, el kernel; `OutOfMemoryException`,
///   DuckDB) o se le negó crecer: **uno más** que el final, porque quería más;
/// - terminó `ok`: su escalón final; y si empezó por la previsión, no creció
///   más y su pico cabe en la mitad del tope del escalón de abajo, **uno menos**;
/// - un error de otro tipo no enseña nada.
pub(crate) fn lo_que_ensena(e: &Ejecucion) -> Option<usize> {
    let final_mb = u64::try_from(e.memory_final_mb?).ok()?;
    let fin = ESCALONES.iter().position(|(tope, _)| *tope == final_mb)?;
    let ultimo = ESCALONES.len() - 1;
    let por_memoria = matches!(
        e.error_name.as_deref(),
        Some("OutOfMemoryError" | "OutOfMemoryException")
    );
    if por_memoria || e.growth_refused.is_some() {
        return Some((fin + 1).min(ultimo));
    }
    if e.status != "ok" {
        return None;
    }
    let solo_la_prevision = e.growth.len() == 1 && e.growth[0].reason == MOTIVO;
    let pequeno = e
        .peak_memory_mb
        .and_then(|p| u64::try_from(p).ok())
        .is_some_and(|p| fin > 0 && p * 2 <= escalon(fin - 1).memoria_tope_mb);
    Some(if solo_la_prevision && pequeno {
        fin - 1
    } else {
        fin
    })
}

/// La huella de la ejecución que empieza con la celda `n` de `p`, como la
/// escribirá executions: la celda suelta, o el guion entero si es su primera
/// sentencia. Nada para la segunda y siguientes de un guion, ni para un Preview.
pub(crate) fn huella_al_dar(p: &Puesto, n: u64) -> Option<String> {
    let c = p.celdas.get(&n)?;
    if c.ensayo.is_some() {
        return None;
    }
    let sentencia = match c.lote {
        Some(l) if l.i > 0 => return None,
        Some(l) => (l.primera..l.primera + l.n as u64)
            .filter_map(|k| p.celdas.get(&k))
            .map(|c| c.texto.trim())
            .collect::<Vec<_>>()
            .join("\n"),
        None => c.texto.trim().to_string(),
    };
    Some(crate::ejecuciones::huella(&sentencia))
}

/// Al dar la celda `n` de `p`: si empieza una ejecución ya vista y lo que
/// necesitó está por encima del escalón que tiene la sesión, ese escalón queda
/// previsto (`Elastica::previsto`) y se devuelve su tope, para el sobre de la
/// celda (`previsto_mb`). Una ejecución nueva sin recuerdo deja nada previsto;
/// la segunda y siguientes sentencias de un guion siguen con lo de la primera.
/// Sólo en el session pool.
pub(crate) fn al_dar(memoria: &Prevision, p: &mut Puesto, n: u64) -> Option<u64> {
    if !p.elastica.en_el_pool {
        return None;
    }
    let huella = huella_al_dar(p, n)?;
    p.elastica.previsto = memoria.prever(&huella).filter(|i| *i > p.elastica.escalon);
    p.elastica.previsto.map(|i| escalon(i).memoria_tope_mb)
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::elastica::Crecimiento;

    fn fila(status: &'static str, final_mb: Option<i64>) -> Ejecucion {
        Ejecucion {
            id: "puesto-ana-python/3".into(),
            kind: "query",
            persona: "ana".into(),
            source_kind: "query",
            source_id: None,
            source_commit: None,
            source_branch: None,
            statement: "select k, mode(v) from t group by k".into(),
            queued_at: 0,
            started_at: 0,
            ended_at: 0,
            status,
            error_name: None,
            error_message: None,
            compute: "L",
            worker: "puesto-ana-python".into(),
            cpu_ms: None,
            peak_memory_mb: None,
            spill_mb: None,
            read: vec![],
            steps: vec![],
            growth: vec![],
            memory_final_mb: final_mb,
            growth_refused: None,
        }
    }

    fn crecio(reason: &str, to_mb: u64) -> Crecimiento {
        Crecimiento {
            at_ms: 0,
            from_mb: 4096,
            to_mb,
            cpu_m: 0,
            reason: reason.into(),
        }
    }

    #[test]
    fn fuera_del_pool_no_ensena_nada() {
        assert_eq!(lo_que_ensena(&fila("ok", None)), None);
        // un tope que no es un escalón (una sesión de antes) tampoco
        assert_eq!(lo_que_ensena(&fila("ok", Some(5000))), None);
    }

    #[test]
    fn la_que_termina_ensena_su_escalon_final() {
        assert_eq!(lo_que_ensena(&fila("ok", Some(4096))), Some(0));
        let mut e = fila("ok", Some(32768));
        e.growth = vec![crecio("memory at 81%", 8192), crecio("spilling", 16384)];
        assert_eq!(lo_que_ensena(&e), Some(3));
    }

    #[test]
    fn la_que_muere_por_memoria_o_no_puede_crecer_quiere_uno_mas() {
        let mut e = fila("failed", Some(16384));
        e.error_name = Some("OutOfMemoryError".into());
        assert_eq!(lo_que_ensena(&e), Some(3));
        e.error_name = Some("OutOfMemoryException".into());
        assert_eq!(lo_que_ensena(&e), Some(3));
        let mut e = fila("ok", Some(8192));
        e.growth_refused = Some("quota".into());
        assert_eq!(lo_que_ensena(&e), Some(2));
        // en el techo, el techo
        let mut e = fila("failed", Some(49152));
        e.error_name = Some("OutOfMemoryError".into());
        assert_eq!(lo_que_ensena(&e), Some(4));
    }

    #[test]
    fn otro_error_no_ensena_nada() {
        let mut e = fila("failed", Some(32768));
        e.error_name = Some("BinderException".into());
        assert_eq!(lo_que_ensena(&e), None);
    }

    #[test]
    fn la_prevision_que_sobra_baja_uno() {
        // Empezó en 32 por la previsión, no creció más y su pico cabe de sobra en 16.
        let mut e = fila("ok", Some(32768));
        e.growth = vec![crecio(MOTIVO, 32768)];
        e.peak_memory_mb = Some(8000);
        assert_eq!(lo_que_ensena(&e), Some(2));
        // Con un pico que no cabe en la mitad de 16, se queda.
        e.peak_memory_mb = Some(9000);
        assert_eq!(lo_que_ensena(&e), Some(3));
        // Si además creció, es que hacía falta: lo final.
        e.peak_memory_mb = Some(1000);
        e.growth.push(crecio("spilling", 49152));
        e.memory_final_mb = Some(49152);
        assert_eq!(lo_que_ensena(&e), Some(4));
        // Sin previsión, un pico pequeño no baja nada: lo subió el vigilante.
        let mut e = fila("ok", Some(8192));
        e.growth = vec![crecio("spilling", 8192)];
        e.peak_memory_mb = Some(100);
        assert_eq!(lo_que_ensena(&e), Some(1));
    }

    #[test]
    fn manda_la_ultima_y_la_huella_no_mira_la_forma() {
        let p = Prevision::default();
        let mut e = fila("ok", Some(32768));
        p.aprender(&e);
        let h = crate::ejecuciones::huella("SELECT k, mode(v)   FROM t GROUP BY k;");
        assert_eq!(p.prever(&h), Some(3));
        e.memory_final_mb = Some(4096);
        p.aprender(&e);
        assert_eq!(p.prever(&h), Some(0));
        assert_eq!(p.prever("otra"), None);
    }

    fn celda(texto: &str, lote: Option<crate::puestos::Lote>) -> crate::puestos::Celda {
        crate::puestos::Celda {
            texto: texto.into(),
            lenguaje: "sql".into(),
            corre: None,
            avisos: vec![],
            enviada: std::time::Instant::now(),
            empezada: None,
            salida: None,
            lote,
            ensayo: None,
        }
    }

    #[test]
    fn al_dar_la_celda_preve_lo_que_necesito_la_otra_vez() {
        let m = Prevision::default();
        let mut e = fila("ok", Some(32768));
        e.statement = "select k, mode(v) from t group by k".into();
        m.aprender(&e);
        let mut p = crate::puestos::prueba::un_puesto("agente:ana");
        p.celdas
            .insert(1, celda("SELECT k, mode(v) FROM t GROUP BY k", None));
        p.celdas.insert(2, celda("select 42", None));
        // Fuera del pool, nada.
        assert_eq!(al_dar(&m, &mut p, 1), None);
        p.elastica.en_el_pool = true;
        assert_eq!(al_dar(&m, &mut p, 1), Some(32768));
        assert_eq!(p.elastica.previsto, Some(3));
        // La siguiente, sin recuerdo, deja de estar prevista.
        assert_eq!(al_dar(&m, &mut p, 2), None);
        assert_eq!(p.elastica.previsto, None);
        // Si la sesión ya está ahí, no hay nada que pedir.
        p.elastica.escalon = 3;
        assert_eq!(al_dar(&m, &mut p, 1), None);
    }

    #[test]
    fn un_guion_se_preve_entero_en_su_primera_sentencia() {
        use crate::puestos::Lote;
        let m = Prevision::default();
        let mut e = fila("ok", Some(16384));
        e.statement = "create table a as select 1\nselect mode(x) from a".into();
        m.aprender(&e);
        let mut p = crate::puestos::prueba::un_puesto("agente:ana");
        p.elastica.en_el_pool = true;
        let l = |i| {
            Some(Lote {
                primera: 5,
                i,
                n: 2,
            })
        };
        p.celdas
            .insert(5, celda("create table a as select 1", l(0)));
        p.celdas.insert(6, celda("select mode(x) from a", l(1)));
        assert_eq!(al_dar(&m, &mut p, 5), Some(16384));
        // La segunda sigue con lo de la primera.
        assert_eq!(al_dar(&m, &mut p, 6), None);
        assert_eq!(p.elastica.previsto, Some(2));
    }

    #[test]
    fn llena_olvida_la_mas_vieja() {
        let p = Prevision::default();
        for k in 0..TOPE {
            p.recordar(format!("h{k}"), 1);
        }
        p.recordar("h0".into(), 2); // la 0 vuelve a ser reciente
        p.recordar("nueva".into(), 3);
        assert_eq!(p.prever("h0"), Some(2));
        assert_eq!(p.prever("h1"), None);
        assert_eq!(p.prever("nueva"), Some(3));
        assert_eq!(p.memoria.lock().unwrap().escalones.len(), TOPE);
    }
}
