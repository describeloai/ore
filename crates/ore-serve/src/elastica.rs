//! Elastic Sessions S2a (docs/products/0073-elastic-sessions.md §2.3) · **Los escalones de una
//! sesión**: a cuánto crece cuando lo pide, sin red. Quien la redimensiona es
//! `ore-redimensionar` (`crates/ore-k8s`); aquí sólo se decide a qué.
//!
//! - **Escalones que se doblan**: 4 → 8 → 16 → 32 → 48 GiB, con CPU 2 → 4 → 6 →
//!   8 → 8. El techo, 48 GiB, es lo que cabe en un nodo `n2-highmem-8` con
//!   holgura (D-E4); por encima, E.
//! - **De uno en uno** al crecer, salvo que se pida de un golpe (la previsión de
//!   S4 sabe cuánto hará falta antes de correr).
//! - **Bajar** va directo al escalón pedido, nunca por debajo de la base.
//! - La base es la de C1 (`malla/51-el-puesto.yaml`): 768Mi pedidos y 4Gi de
//!   tope. Por encima, lo pedido es el tope: lo que una sesión crece, el nodo se
//!   lo guarda (y la `ResourceQuota` lo cuenta).

/// Lo que un puesto sabe de su tamaño (`Puesto::elastica`).
#[derive(Debug, Default, Clone)]
pub struct Elastica {
    /// Se abrió en el session pool (`--session-pool`): sólo esa sesión crece.
    pub en_el_pool: bool,
    /// El escalón que tiene ahora (0: la base).
    pub escalon: usize,
    /// Un redimensionado en vuelo: el siguiente espera a que acabe.
    pub escalando: bool,
    /// S2b · Lo que creció (o bajó) durante la ejecución que corre: el `growth`
    /// de su fila en executions. Empieza vacío con cada ejecución, como lo leído.
    pub crecido_ahora: Vec<Crecimiento>,
    /// S2b · Y si quiso crecer y no pudo, por qué (`infeasible`, `deferred`,
    /// `quota`): su `growth_refused`.
    pub negado_ahora: Option<String>,
    /// S4a · El escalón previsto para la ejecución que corre (`prevision.rs`):
    /// hasta ahí, y no más, puede crecer *de un golpe* (D-S4-3).
    pub previsto: Option<usize>,
    /// S4b · Lo previsto por el plan al recibir una celda `sql` nueva, por el
    /// número de la primera celda de su ejecución: `al_dar` lo usa si S4a no
    /// recuerda la consulta.
    pub por_el_plan: std::collections::BTreeMap<u64, usize>,
}

/// S2b · Un cambio de tamaño hecho, para executions (`growth`).
#[derive(Debug, Clone, PartialEq)]
pub struct Crecimiento {
    /// Milisegundos de época.
    pub at_ms: i64,
    pub from_mb: u64,
    pub to_mb: u64,
    pub cpu_m: u64,
    /// El `motivo` con que se pidió.
    pub reason: String,
}

/// S2b · **El presupuesto de una celda para sus sesiones** (D-E3): lo que el
/// nodo les guarda a todas juntas, en MiB. Es la cuota de `cq-<celda>-sesiones`
/// (`malla/11-el-inquilino.yaml`, 40Gi). Kueue la cuenta sólo al ABRIR, con la
/// base de cada sesión, y no ve lo que crece después (S0·a, medido): la cuenta
/// de lo crecido es de `ore-serve`, que es quien redimensiona.
pub const PRESUPUESTO_MB: u64 = 40 * 1024;

/// S2b · Lo que el nodo les guarda ahora a estas sesiones (las del pool):
/// la base pide 768Mi; cada escalón por encima, su tope.
pub fn ocupado_mb<'a>(sesiones: impl IntoIterator<Item = &'a Elastica>) -> u64 {
    sesiones
        .into_iter()
        .filter(|e| e.en_el_pool)
        .map(|e| escalon(e.escalon).memoria_pedida_mb)
        .sum()
}

/// S2b · ¿Cabe `pedido` más, sobre lo que ya ocupan las demás?
pub fn cabe(ocupado_por_las_demas: u64, pedido_mb: u64) -> bool {
    ocupado_por_las_demas + pedido_mb <= PRESUPUESTO_MB
}

/// `(memoria_tope_mb, cpu_tope_m)` de cada escalón; el 0 es la base.
pub const ESCALONES: [(u64, u64); 5] = [
    (4096, 2000),
    (8192, 4000),
    (16384, 6000),
    (32768, 8000),
    (49152, 8000),
];

/// La memoria que la base PIDE (C1): el tope del escalón 0 es lo que puede usar.
pub const BASE_PEDIDA_MB: u64 = 768;

/// Un escalón, como se le pide a Kubernetes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Escalon {
    pub indice: usize,
    pub memoria_pedida_mb: u64,
    pub memoria_tope_mb: u64,
    pub cpu_tope_m: u64,
}

pub fn escalon(i: usize) -> Escalon {
    let i = i.min(ESCALONES.len() - 1);
    let (tope, cpu) = ESCALONES[i];
    Escalon {
        indice: i,
        memoria_pedida_mb: if i == 0 { BASE_PEDIDA_MB } else { tope },
        memoria_tope_mb: tope,
        cpu_tope_m: cpu,
    }
}

/// A qué escalón ir desde `actual` cuando se piden `pedido_mb` de tope.
/// `techo` dice que se pidió más de lo que una sesión puede tener (48 GiB).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Decision {
    pub escalon: Escalon,
    pub techo: bool,
}

pub fn decidir(actual: usize, pedido_mb: u64, de_un_golpe: bool) -> Decision {
    let ultimo = ESCALONES.len() - 1;
    let actual = actual.min(ultimo);
    let techo = pedido_mb > ESCALONES[ultimo].0;
    // El menor escalón que da lo pedido (o el último, si nada lo da).
    let quiere = ESCALONES
        .iter()
        .position(|(tope, _)| *tope >= pedido_mb)
        .unwrap_or(ultimo);
    let i = if quiere > actual && !de_un_golpe {
        actual + 1
    } else {
        quiere
    };
    Decision {
        escalon: escalon(i),
        techo,
    }
}

/// S4a · Lo que decide `scale`: *de un golpe* sólo hasta el escalón `previsto`
/// para la ejecución que corre (D-S4-3); sin previsión, de uno en uno.
pub fn decidir_hasta(
    actual: usize,
    pedido_mb: u64,
    de_un_golpe: bool,
    previsto: Option<usize>,
) -> Decision {
    match (de_un_golpe, previsto) {
        (true, Some(i)) => decidir(actual, pedido_mb.min(escalon(i).memoria_tope_mb), true),
        _ => decidir(actual, pedido_mb, false),
    }
}

/// S4b · Por cuánto se multiplica lo que el plan dice que el motor construye.
/// ⚠️ Sin calibrar: S4b·0 lo mide contra lo real de executions (§1.8).
pub const FACTOR_DEL_PLAN: f64 = 1.0;

/// S4b · Lo que una peligrosa construye tiene que caber entero (no derrama):
/// el doble de margen.
pub const MARGEN_PELIGROSA: f64 = 2.0;

/// S4b · Por el plan no se pasa de este escalón, 32 GiB (D-S4-6): DuckDB estima
/// mal los grupos, y lo que falte lo sube el vigilante.
pub const TECHO_DEL_PLAN: usize = 3;

/// S4b · El reparto de DuckDB, como en el SDK (`_DUCKDB_POR_CIENTO`): el 60 %
/// del pod, el 45 % con una reservada o peligrosa.
fn reparto_pct(r: ore_core::admision::Riesgo) -> u64 {
    match r {
        ore_core::admision::Riesgo::Segura => 60,
        _ => 45,
    }
}

/// S4b · **A qué escalón empieza una consulta nueva por su plan**
/// (docs/products/0073-elastic-sessions.md §2.6): el menor cuyo reparto da lo que el motor
/// construye (`Entrada::memoria`, por [`FACTOR_DEL_PLAN`], y por
/// [`MARGEN_PELIGROSA`] si es peligrosa), sin pasar de [`TECHO_DEL_PLAN`].
/// Nada si cabe en la base o si el plan no se pudo estimar entero.
pub fn escalon_por_el_plan(e: &ore_core::admision::Entrada, sql: &str) -> Option<usize> {
    if !e.estimada || e.memoria == 0 {
        return None;
    }
    let r = ore_core::admision::riesgo(sql);
    let margen = if r == ore_core::admision::Riesgo::Peligrosa {
        MARGEN_PELIGROSA
    } else {
        1.0
    };
    let hace_falta_mb =
        (e.memoria as f64 * FACTOR_DEL_PLAN * margen / (1u64 << 20) as f64).ceil() as u64;
    let pct = reparto_pct(r);
    let i = ESCALONES
        .iter()
        .position(|(tope, _)| tope * pct / 100 >= hace_falta_mb)
        .unwrap_or(ESCALONES.len() - 1)
        .min(TECHO_DEL_PLAN);
    (i > 0).then_some(i)
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// El presupuesto es la cuota de memoria de `cq-<celda>-sesiones`.
    #[test]
    fn el_presupuesto_es_la_cola_de_las_sesiones() {
        let t = include_str!("../../../malla/11-el-inquilino.yaml");
        let cola = t
            .split(
                "
---
",
            )
            .find(|d| d.contains("name: cq-demo-sesiones"))
            .expect("la ClusterQueue de las sesiones");
        assert!(cola.contains("nominalQuota: \"40Gi\""), "{cola}");
        assert_eq!(PRESUPUESTO_MB, 40 * 1024);
    }

    #[test]
    fn lo_ocupado_es_la_base_o_el_tope_de_cada_una_del_pool() {
        let e = |en_el_pool, escalon| Elastica {
            en_el_pool,
            escalon,
            ..Default::default()
        };
        let todas = [e(true, 0), e(true, 2), e(false, 4), e(true, 1)];
        assert_eq!(ocupado_mb(&todas), 768 + 16384 + 8192);
        assert!(cabe(40960 - 768, 768));
        assert!(!cabe(40960 - 767, 768));
        // Diez sesiones en reposo y una a 32 GiB caben; la segunda a 32, no.
        assert!(cabe(10 * 768, 32768));
        assert!(!cabe(10 * 768 + 32768, 32768));
    }

    #[test]
    fn la_base_es_la_de_c1_y_arriba_lo_pedido_es_el_tope() {
        assert_eq!(
            escalon(0),
            Escalon {
                indice: 0,
                memoria_pedida_mb: 768,
                memoria_tope_mb: 4096,
                cpu_tope_m: 2000
            }
        );
        assert_eq!(
            escalon(2),
            Escalon {
                indice: 2,
                memoria_pedida_mb: 16384,
                memoria_tope_mb: 16384,
                cpu_tope_m: 6000
            }
        );
        assert_eq!(escalon(9).indice, 4);
        // El tope de la base es el `limits.memory` de `malla/51` (y su `ORE_MEMORIA_MB`, ⑱).
        let p = include_str!("../../../malla/51-el-puesto.yaml");
        assert!(p.contains("limits:   {cpu: \"2\", memory: 4Gi}"));
        assert!(p.contains("requests: {cpu: \"100m\", memory: 768Mi}"));
    }

    #[test]
    fn crece_de_uno_en_uno() {
        assert_eq!(decidir(0, 5000, false).escalon.indice, 1);
        assert_eq!(decidir(0, 40000, false).escalon.indice, 1);
        assert_eq!(decidir(1, 40000, false).escalon.indice, 2);
        // Lo que ya tiene, no cambia.
        assert_eq!(decidir(2, 10000, false).escalon.indice, 2);
    }

    #[test]
    fn de_un_golpe_va_a_lo_que_hace_falta() {
        assert_eq!(decidir(0, 20000, true).escalon.indice, 3);
        assert_eq!(decidir(0, 16384, true).escalon.indice, 2);
    }

    #[test]
    fn de_un_golpe_solo_hasta_lo_previsto() {
        // Sin previsión, el pod que pide 48 de un golpe sube uno.
        assert_eq!(decidir_hasta(0, 49152, true, None).escalon.indice, 1);
        // Con 32 previstos, llega a 32 y no más.
        assert_eq!(decidir_hasta(0, 49152, true, Some(3)).escalon.indice, 3);
        assert_eq!(decidir_hasta(0, 32768, true, Some(3)).escalon.indice, 3);
        // Lo que pide de menos, lo que pide.
        assert_eq!(decidir_hasta(0, 8192, true, Some(3)).escalon.indice, 1);
        // Sin `de_un_golpe`, la previsión no cambia nada.
        assert_eq!(decidir_hasta(0, 49152, false, Some(3)).escalon.indice, 1);
        // Bajar va directo, con o sin previsión.
        assert_eq!(decidir_hasta(3, 4096, false, Some(3)).escalon.indice, 0);
    }

    fn por_el_plan(memoria: u64) -> ore_core::admision::Entrada {
        ore_core::admision::Entrada {
            estimada: true,
            memoria,
            ..Default::default()
        }
    }

    #[test]
    fn por_el_plan_el_escalon_cuyo_reparto_da_lo_que_construye() {
        const MIB: u64 = 1 << 20;
        let segura = "select k, sum(v) from t group by k";
        // Lo que cabe en el 60 % de la base (2457 MiB), nada.
        assert_eq!(escalon_por_el_plan(&por_el_plan(2000 * MIB), segura), None);
        assert_eq!(escalon_por_el_plan(&por_el_plan(0), segura), None);
        // 3433 MiB (la agrupación de 300 M, con los grupos que estima DuckDB): 8 GiB.
        assert_eq!(
            escalon_por_el_plan(&por_el_plan(3_600_000_000), segura),
            Some(1)
        );
        // Una reservada reparte el 45 %: lo mismo (3815 MiB) pide más.
        assert_eq!(
            escalon_por_el_plan(&por_el_plan(4_000_000_000), segura),
            Some(1)
        );
        assert_eq!(
            escalon_por_el_plan(&por_el_plan(4_000_000_000), "select median(v) from t"),
            Some(2)
        );
        // `mode` de 300 M (4578 MiB), con el doble de margen y al 45 %: 32 GiB.
        assert_eq!(
            escalon_por_el_plan(
                &por_el_plan(4_800_000_000),
                "select k, mode(v) from t group by k"
            ),
            Some(3)
        );
        // Nunca más de 32 GiB por el plan (D-S4-6).
        assert_eq!(
            escalon_por_el_plan(&por_el_plan(u64::MAX / 4), segura),
            Some(3)
        );
        // Sin estimar entero, nada.
        let mut e = por_el_plan(40_000_000_000);
        e.estimada = false;
        assert_eq!(escalon_por_el_plan(&e, segura), None);
    }

    #[test]
    fn el_techo_es_48_gib() {
        let d = decidir(3, 60000, false);
        assert_eq!(d.escalon.memoria_tope_mb, 49152);
        assert!(d.techo);
        assert!(!decidir(3, 49152, false).techo);
        assert_eq!(decidir(4, 99999, true).escalon.indice, 4);
    }

    #[test]
    fn bajar_va_directo_y_nunca_debajo_de_la_base() {
        assert_eq!(decidir(4, 4096, false).escalon.indice, 0);
        assert_eq!(decidir(4, 10000, false).escalon.indice, 2);
        assert_eq!(decidir(2, 1, false).escalon, escalon(0));
    }
}
