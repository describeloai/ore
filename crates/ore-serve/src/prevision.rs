//! Elastic Sessions S4a (docs/decisions/0073-elastic-sessions.md §2.6) · **Lo que una consulta
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

/// S4b · El de la previsión por el plan, para distinguirla en executions
/// (D-S4-7). Para aprender, las dos son previsión.
pub const MOTIVO_PLAN: &str = "plan";

fn es_prevision(reason: &str) -> bool {
    reason == MOTIVO || reason == MOTIVO_PLAN
}

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

/// Lo que hace falta saber de una ejecución para aprender de ella: de la
/// ejecución que acaba de terminar, o de su fila en executions (al arrancar).
#[derive(Debug, Default)]
pub(crate) struct Visto {
    pub status: String,
    pub error_name: Option<String>,
    pub memory_final_mb: Option<i64>,
    pub peak_memory_mb: Option<i64>,
    pub growth_refused: bool,
    /// Creció una sola vez, y fue lo previsto.
    pub solo_la_prevision: bool,
}

impl Visto {
    fn de_ejecucion(e: &Ejecucion) -> Visto {
        Visto {
            status: e.status.to_string(),
            error_name: e.error_name.clone(),
            memory_final_mb: e.memory_final_mb,
            peak_memory_mb: e.peak_memory_mb,
            growth_refused: e.growth_refused.is_some(),
            solo_la_prevision: e.growth.len() == 1 && es_prevision(&e.growth[0].reason),
        }
    }

    /// De una fila de executions como la devuelve `ore-store leer` (todo
    /// texto; un nulo es la columna que falta).
    fn de_fila(f: &ore_core::parse::Node) -> Visto {
        let texto = |k: &str| texto_de(f, k);
        let entero = |k: &str| texto(k).and_then(|v| v.parse::<i64>().ok());
        let motivos: Vec<String> = texto("growth")
            .and_then(|g| ore_core::parse::parse(&g).ok())
            .map(|g| {
                g.items()
                    .iter()
                    .filter_map(|c| {
                        c.get("reason")
                            .and_then(|(_, r)| r.as_str())
                            .map(String::from)
                    })
                    .collect()
            })
            .unwrap_or_default();
        Visto {
            status: texto("status").unwrap_or_default(),
            error_name: texto("error_name"),
            memory_final_mb: entero("memory_final_mb"),
            peak_memory_mb: entero("peak_memory_mb"),
            growth_refused: texto("growth_refused").is_some(),
            solo_la_prevision: motivos.len() == 1 && es_prevision(&motivos[0]),
        }
    }
}

/// Un campo de texto de una fila, si no está vacío.
fn texto_de(f: &ore_core::parse::Node, k: &str) -> Option<String> {
    f.get(k)
        .and_then(|(_, v)| v.as_str())
        .filter(|v| !v.is_empty())
        .map(String::from)
}

/// **Qué escalón enseña una ejecución terminada** (§1.7): ver [`ensena`].
pub(crate) fn lo_que_ensena(e: &Ejecucion) -> Option<usize> {
    ensena(&Visto::de_ejecucion(e))
}

/// **Qué escalón enseña lo visto** (§1.7), o nada:
///
/// - sólo del session pool (`memory_final_mb`);
/// - murió por memoria (`OutOfMemoryError`, el kernel; `OutOfMemoryException`,
///   DuckDB) o se le negó crecer: **uno más** que el final, porque quería más;
/// - terminó `ok`: su escalón final; y si empezó por la previsión, no creció
///   más y su pico cabe en la mitad del tope del escalón de abajo, **uno menos**;
/// - un error de otro tipo no enseña nada.
pub(crate) fn ensena(v: &Visto) -> Option<usize> {
    let final_mb = u64::try_from(v.memory_final_mb?).ok()?;
    let fin = ESCALONES.iter().position(|(tope, _)| *tope == final_mb)?;
    let ultimo = ESCALONES.len() - 1;
    let por_memoria = matches!(
        v.error_name.as_deref(),
        Some("OutOfMemoryError" | "OutOfMemoryException")
    );
    if por_memoria || v.growth_refused {
        return Some((fin + 1).min(ultimo));
    }
    if v.status != "ok" {
        return None;
    }
    let pequeno = v
        .peak_memory_mb
        .and_then(|p| u64::try_from(p).ok())
        .is_some_and(|p| fin > 0 && p * 2 <= escalon(fin - 1).memoria_tope_mb);
    Some(if v.solo_la_prevision && pequeno {
        fin - 1
    } else {
        fin
    })
}

/// S4a·3 · Las columnas de executions que hacen falta para recordar.
pub const COLUMNAS: &str =
    "fingerprint,ended_at,status,error_name,memory_final_mb,peak_memory_mb,growth,growth_refused";

/// S4a·3 · Cuántos días atrás se recuerda al arrancar.
pub const DIAS: i64 = 7;

/// S4a·3 · **Recordar lo de executions**: las filas que contesta `ore-store
/// leer` (la cabecera y una por línea), de `desde_dia` (`AAAA-MM-DD`) en
/// adelante, en el orden en que terminaron: así manda la última de cada
/// `fingerprint`, como si `ore-serve` no se hubiera ido. Devuelve cuántas
/// enseñaron algo.
pub(crate) fn calentar(memoria: &Prevision, lineas: &str, desde_dia: &str) -> usize {
    let mut filas: Vec<(String, String, Visto)> = lineas
        .lines()
        .skip(1)
        .filter_map(|l| ore_core::parse::parse(l).ok())
        .filter_map(|f| {
            let fin = texto_de(&f, "ended_at")?;
            let huella = texto_de(&f, "fingerprint")?;
            (fin.get(..10)? >= desde_dia).then(|| (fin, huella, Visto::de_fila(&f)))
        })
        .collect();
    filas.sort_by(|a, b| a.0.cmp(&b.0));
    let mut n = 0;
    for (_, huella, v) in filas {
        if let Some(i) = ensena(&v) {
            memoria.recordar(huella, i);
            n += 1;
        }
    }
    n
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

/// Al dar la celda `n` de `p`: si empieza una ejecución ya vista (S4a) o, si
/// no, una que el plan pesó al recibirla (S4b, `Elastica::por_el_plan`), y lo
/// previsto está por encima del escalón que tiene la sesión, ese escalón queda
/// previsto (`Elastica::previsto`) y se devuelve su tope y el motivo, para el
/// sobre de la celda (`previsto_mb`, `previsto_motivo`). Lo medido manda sobre
/// lo estimado (D-S4-5). Sin nada, nada previsto; la segunda y siguientes
/// sentencias de un guion siguen con lo de la primera. Sólo en el session pool.
pub(crate) fn al_dar(memoria: &Prevision, p: &mut Puesto, n: u64) -> Option<(u64, &'static str)> {
    if !p.elastica.en_el_pool {
        return None;
    }
    let huella = huella_al_dar(p, n)?;
    // Lo del plan es de esta ejecución y de las de antes que no llegaron a correr.
    let del_plan = p.elastica.por_el_plan.remove(&n);
    p.elastica.por_el_plan.retain(|k, _| *k > n);
    p.elastica.previsto = None;
    let (i, motivo) = match memoria.prever(&huella) {
        Some(i) => (i, MOTIVO),
        None => (del_plan?, MOTIVO_PLAN),
    };
    p.elastica.previsto = Some(i).filter(|i| *i > p.elastica.escalon);
    p.elastica
        .previsto
        .map(|i| (escalon(i).memoria_tope_mb, motivo))
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
            log: None,
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
        assert_eq!(al_dar(&m, &mut p, 1), Some((32768, "forecast")));
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
        assert_eq!(al_dar(&m, &mut p, 5), Some((16384, "forecast")));
        // La segunda sigue con lo de la primera.
        assert_eq!(al_dar(&m, &mut p, 6), None);
        assert_eq!(p.elastica.previsto, Some(2));
    }

    #[test]
    fn al_arrancar_recuerda_lo_de_la_semana_y_manda_la_ultima() {
        let m = Prevision::default();
        let lineas = [
            r#"{"plan":"system.default.executions"}"#,
            // la de hace más de una semana no cuenta
            r#"{"fingerprint":"vieja","ended_at":"2026-10-01 10:00:00+00","status":"ok","memory_final_mb":"32768"}"#,
            // dos de la misma: manda la que terminó después, aunque venga antes
            r#"{"fingerprint":"a","ended_at":"2026-10-24 12:00:00+00","status":"ok","memory_final_mb":"16384"}"#,
            r#"{"fingerprint":"a","ended_at":"2026-10-23 09:00:00+00","status":"ok","memory_final_mb":"49152"}"#,
            // una que murió por memoria quiere uno más
            r#"{"fingerprint":"m","ended_at":"2026-10-24 08:00:00+00","status":"failed","error_name":"OutOfMemoryError","memory_final_mb":"16384"}"#,
            // la previsión que sobró baja uno, leída del JSON de growth
            r#"{"fingerprint":"p","ended_at":"2026-10-24 08:00:00+00","status":"ok","memory_final_mb":"32768","peak_memory_mb":"3000","growth":"[{\"at_ms\":1,\"from_mb\":4096,\"to_mb\":32768,\"cpu_m\":8000,\"reason\":\"forecast\"}]"}"#,
            // fuera del pool, o sin huella: nada
            r#"{"fingerprint":"l","ended_at":"2026-10-24 08:00:00+00","status":"ok"}"#,
            r#"{"ended_at":"2026-10-24 08:00:00+00","status":"ok","memory_final_mb":"8192"}"#,
            "no es JSON",
        ]
        .join("\n");
        assert_eq!(calentar(&m, &lineas, "2026-10-17"), 4);
        assert_eq!(m.prever("vieja"), None);
        assert_eq!(m.prever("a"), Some(2));
        assert_eq!(m.prever("m"), Some(3));
        assert_eq!(m.prever("p"), Some(2));
        assert_eq!(m.prever("l"), None);
    }

    #[test]
    fn sin_recuerdo_manda_el_plan_y_con_recuerdo_lo_medido() {
        let m = Prevision::default();
        let mut p = crate::puestos::prueba::un_puesto("agente:ana");
        p.elastica.en_el_pool = true;
        p.celdas
            .insert(1, celda("select k, mode(v) from t group by k", None));
        p.celdas
            .insert(2, celda("select k, mode(v) from t group by k", None));
        p.celdas.insert(3, celda("select 1", None));
        // Nunca vista: lo del plan, con su motivo.
        p.elastica.por_el_plan.insert(1, 3);
        assert_eq!(al_dar(&m, &mut p, 1), Some((32768, "plan")));
        assert!(p.elastica.por_el_plan.is_empty());
        // Ya vista (terminó en 16): lo medido manda, aunque el plan diga 32.
        let mut e = fila("ok", Some(16384));
        e.statement = "select k, mode(v) from t group by k".into();
        m.aprender(&e);
        p.elastica.por_el_plan.insert(2, 3);
        assert_eq!(al_dar(&m, &mut p, 2), Some((16384, "forecast")));
        // Lo de una celda que no llegó a correr no se queda para siempre.
        p.elastica.por_el_plan.insert(2, 3);
        assert_eq!(al_dar(&m, &mut p, 3), None);
        assert!(p.elastica.por_el_plan.is_empty());
    }

    #[test]
    fn la_prevision_por_el_plan_que_sobra_tambien_baja() {
        let mut e = fila("ok", Some(32768));
        e.growth = vec![crecio(MOTIVO_PLAN, 32768)];
        e.peak_memory_mb = Some(2000);
        assert_eq!(lo_que_ensena(&e), Some(2));
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
