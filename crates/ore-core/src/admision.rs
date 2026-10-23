//! **La admisión de una consulta** (Query Q3, `docs/query.md` §3.4): en qué clase
//! corre —L, G o E— y con cuánta memoria e hilos, decidido ANTES de correr por lo
//! que va a leer. Nunca se sube de clase porque algo falle: si la estimación duda,
//! se elige lo grande (una consulta pequeña en un trabajador grande y caliente
//! cuesta milisegundos; una que se cae, minutos).
//!
//! Es una función pura: la entrada sale de `ore explain --estimate --json` (el
//! reparto, con lo que se lee del lago por sus manifiestos) y los umbrales, de lo
//! medido en Q0 (§2.1): con ≤ 0,14 GB la sesión contesta en ≤ 2 s; con 0,4–1 GB
//! ya tarda 2–19 s, y el trabajador grande hace 600 M filas / 10 GB en ≤ 50 s.

use crate::json::Json;
use crate::parse::Node;

/// Dónde corre una consulta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clase {
    /// Una instancia pequeña de DuckDB en un trabajador caliente.
    L,
    /// Una instancia de DuckDB que toma casi un trabajador entero.
    G,
    /// Spark elástico: lo que no cabe en un nodo.
    E,
}

impl Clase {
    pub fn nombre(self) -> &'static str {
        match self {
            Clase::L => "L",
            Clase::G => "G",
            Clase::E => "E",
        }
    }
}

/// Los umbrales y la forma del trabajador. Opción de `ore-serve`, no constantes.
#[derive(Debug, Clone)]
pub struct Umbrales {
    /// Hasta aquí, L: bytes leídos y filas.
    pub l_bytes: u64,
    pub l_filas: u64,
    /// Por encima, E (si se puede): lo que un nodo no aguanta con holgura.
    pub e_bytes: u64,
    /// La memoria de un trabajador (MB) y la parte que se reparte (%): Q0b midió
    /// que cuatro topes de 8 GB juntos pasan su suma un 12 %.
    pub trabajador_mb: u64,
    pub reparto_pct: u64,
    pub l_memoria_mb: u64,
    pub l_hilos: u64,
    pub g_hilos: u64,
}

impl Default for Umbrales {
    fn default() -> Self {
        Umbrales {
            l_bytes: 500_000_000,
            l_filas: 50_000_000,
            e_bytes: 150_000_000_000,
            trabajador_mb: 49_152,
            reparto_pct: 80,
            l_memoria_mb: 2_048,
            l_hilos: 2,
            g_hilos: 8,
        }
    }
}

/// Lo que la admisión sabe de una consulta antes de correrla.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Entrada {
    /// Se analizó y se estimó todo lo del lago que lee.
    pub estimada: bool,
    pub filas: u64,
    pub bytes: u64,
    /// Tablas leídas en vivo de un origen: sólo DuckDB las lee (`/federation/read`).
    pub en_vivo: Vec<String>,
    /// Lo que no se pudo estimar, y por qué.
    pub sin_estimar: Vec<String>,
}

/// La decisión.
#[derive(Debug, Clone, PartialEq)]
pub struct Admision {
    pub clase: Clase,
    pub razon: String,
    pub memoria_mb: u64,
    pub hilos: u64,
    pub avisos: Vec<String>,
}

impl Admision {
    pub fn json(&self) -> Json {
        Json::obj([
            ("clase", Json::s(self.clase.nombre())),
            ("razon", Json::s(self.razon.as_str())),
            ("memoria_mb", Json::Int(self.memoria_mb as i64)),
            ("hilos", Json::Int(self.hilos as i64)),
            (
                "avisos",
                Json::Arr(self.avisos.iter().map(|a| Json::s(a.as_str())).collect()),
            ),
        ])
    }
}

/// **La regla.** Sin estimación, G; en L sólo lo pequeño y estimado; en E sólo lo
/// que un nodo no aguanta y que Spark puede leer (nada en vivo).
pub fn admitir(e: &Entrada, u: &Umbrales) -> Admision {
    let g_memoria = u.trabajador_mb * u.reparto_pct / 100;
    let gb = |b: u64| b as f64 / 1e9;
    let mut avisos = Vec::new();
    if !e.sin_estimar.is_empty() {
        avisos.push(format!("sin estimar: {}", e.sin_estimar.join("; ")));
    }
    let g = |razon: String, avisos: Vec<String>| Admision {
        clase: Clase::G,
        razon,
        memoria_mb: g_memoria,
        hilos: u.g_hilos,
        avisos,
    };
    if !e.estimada {
        return g(
            "no se sabe cuánto lee: el trabajador grande, que aguanta cualquier cosa que quepa en un nodo"
                .into(),
            avisos,
        );
    }
    if e.bytes > u.e_bytes {
        if e.en_vivo.is_empty() {
            return Admision {
                clase: Clase::E,
                razon: format!(
                    "lee {:.1} GB, más de lo que un nodo hace con holgura ({:.0} GB)",
                    gb(e.bytes),
                    gb(u.e_bytes)
                ),
                memoria_mb: g_memoria,
                hilos: u.g_hilos,
                avisos,
            };
        }
        avisos.push(format!(
            "lee {:.1} GB, pero también en vivo ({}): sólo DuckDB lee en vivo",
            gb(e.bytes),
            e.en_vivo.join(", ")
        ));
        return g(
            "lee en vivo de un origen: DuckDB, en el trabajador grande".into(),
            avisos,
        );
    }
    if e.bytes <= u.l_bytes && e.filas <= u.l_filas {
        return Admision {
            clase: Clase::L,
            razon: format!(
                "lee {:.2} GB y {} filas: cabe en una instancia pequeña",
                gb(e.bytes),
                e.filas
            ),
            memoria_mb: u.l_memoria_mb,
            hilos: u.l_hilos,
            avisos,
        };
    }
    g(
        format!(
            "lee {:.2} GB y {} filas: más que una instancia pequeña, cabe en un nodo",
            gb(e.bytes),
            e.filas
        ),
        avisos,
    )
}

/// La entrada, del JSON de `ore explain --estimate --json`: `entendida`, las
/// `lecturas` (en vivo) y `estimacion` (lo del lago).
pub fn entrada_de_explain(plan: &Node) -> Entrada {
    let entendida = plan.get("entendida").and_then(|(_, v)| v.as_str()) == Some("true");
    let en_vivo: Vec<String> = plan
        .get("lecturas")
        .map(|(_, v)| v.items().to_vec())
        .unwrap_or_default()
        .iter()
        .filter_map(|l| {
            l.get("tabla")
                .and_then(|(_, v)| v.as_str())
                .map(String::from)
        })
        .collect();
    let num = |n: &Node, k: &str| {
        n.get(k)
            .and_then(|(_, v)| v.as_str())
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0)
    };
    let mut sin_estimar = Vec::new();
    let (mut filas, mut bytes) = (0, 0);
    let estimacion = plan.get("estimacion").map(|(_, v)| v);
    if estimacion.is_none() {
        sin_estimar.push("la sentencia no se estimó".to_string());
    }
    for t in estimacion
        .and_then(|e| e.get("tablas"))
        .map(|(_, v)| v.items().to_vec())
        .unwrap_or_default()
    {
        let tabla = t.get("tabla").and_then(|(_, v)| v.as_str()).unwrap_or("?");
        if let Some((_, e)) = t.get("error") {
            sin_estimar.push(format!("{tabla}: {}", e.as_str().unwrap_or("")));
            continue;
        }
        let sin_tamano: Vec<String> = t
            .get("sin_tamano")
            .map(|(_, v)| {
                v.items()
                    .iter()
                    .filter_map(|c| c.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        if !sin_tamano.is_empty() {
            sin_estimar.push(format!(
                "{tabla}: sin tamaño para {}",
                sin_tamano.join(", ")
            ));
        }
        filas += num(&t, "filas");
        bytes += num(&t, "bytes");
    }
    if !entendida {
        sin_estimar.push("el analizador no entiende la sentencia".to_string());
    }
    Entrada {
        estimada: sin_estimar.is_empty(),
        filas,
        bytes,
        en_vivo,
        sin_estimar,
    }
}

/// **Las rutas que no se admiten**: un literal con `..` o `%2e` (Q0b, §2.7: la
/// lista de rutas de DuckDB compara la URL como texto y no la normaliza; la
/// credencial acotada lo para, y esto lo para antes). Devuelve los literales.
pub fn rutas_sospechosas(sql: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut dentro: Option<String> = None;
    let mut comentario = false;
    let b: Vec<char> = sql.chars().collect();
    for (i, &c) in b.iter().enumerate() {
        if comentario {
            comentario = c != '\n';
            continue;
        }
        match &mut dentro {
            Some(l) if c == '\'' => {
                let x = l.to_lowercase();
                if x.contains("..") || x.contains("%2e") {
                    out.push(l.clone());
                }
                dentro = None;
            }
            Some(l) => l.push(c),
            None if c == '\'' => dentro = Some(String::new()),
            None if c == '-' && b.get(i + 1) == Some(&'-') => comentario = true,
            None => {}
        }
    }
    out
}

/// Lo que ocupa una columna de una fila, para pasar de filas a bytes.
const BYTES_POR_COLUMNA: u64 = 8;

/// Los operadores que construyen en memoria lo que reciben (o su lado de
/// construcción): lo que ocupan es lo que pide el motor, no sólo lo que pasa.
const CONSTRUYEN: [&str; 6] = [
    "HASH_GROUP_BY",
    "PERFECT_HASH_GROUP_BY",
    "ORDER_BY",
    "WINDOW",
    "STREAMING_WINDOW",
    "TOP_N",
];

/// Query A2 · **El peso de un plan** (docs/query.md §3.7): de lo que el agente
/// contestó a `POST /puestos/{id}/plan` —`{plan, en_vivo, sin_estimar}`, el
/// `EXPLAIN (FORMAT JSON)` de DuckDB—, la entrada de [`admitir`]. Por cada
/// operador, su cardinalidad estimada y su anchura (las `Projections` que dice,
/// o la de sus hijos); `filas` es la del operador mayor y `bytes` lo mayor de lo
/// que pasa por un operador y lo que se construye en memoria (el lado derecho
/// de un `HASH_JOIN`, una agregación, un orden, una ventana). Lo que se lee, lo
/// que se genera, lo que multiplica un join y lo que se escribe salen del mismo
/// sitio. `None` si no hay plan (`error`, o sin `plan`).
pub fn peso(respuesta: &Node) -> Option<Entrada> {
    if respuesta.get("error").is_some() {
        return None;
    }
    let (_, plan) = respuesta.get("plan")?;
    let lista = |k: &str| -> Vec<String> {
        respuesta
            .get(k)
            .map(|(_, v)| {
                v.items()
                    .iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut p = Peso::default();
    for n in plan.items() {
        p.visitar(n);
    }
    if p.operadores == 0 {
        return None;
    }
    let sin_estimar = lista("sin_estimar");
    Some(Entrada {
        estimada: sin_estimar.is_empty(),
        filas: p.filas,
        bytes: p.bytes,
        en_vivo: lista("en_vivo"),
        sin_estimar,
    })
}

#[derive(Default)]
struct Peso {
    operadores: usize,
    filas: u64,
    bytes: u64,
}

impl Peso {
    /// Recorre `n` y devuelve `(filas, anchura)` de lo que sale de él.
    fn visitar(&mut self, n: &Node) -> (u64, u64) {
        let hijos: Vec<(u64, u64)> = n
            .get("children")
            .map(|(_, v)| v.items().to_vec())
            .unwrap_or_default()
            .iter()
            .map(|h| self.visitar(h))
            .collect();
        let extra = n.get("extra_info").map(|(_, v)| v);
        let est = extra
            .and_then(|e| e.get("Estimated Cardinality"))
            .and_then(|(_, v)| v.as_str())
            .and_then(|v| v.trim_start_matches('~').trim().parse::<u64>().ok());
        let ancho = extra
            .and_then(|e| e.get("Projections"))
            .map(|(_, v)| v.items().len() as u64)
            .filter(|a| *a > 0)
            .or_else(|| hijos.iter().map(|(_, a)| *a).max())
            .unwrap_or(1);
        // Sin estimación propia (una raíz que agrega, un `CREATE_TABLE_AS`), lo
        // que le llega.
        let filas = est.unwrap_or_else(|| hijos.iter().map(|(f, _)| *f).max().unwrap_or(0));
        let nombre = n.get("name").and_then(|(_, v)| v.as_str()).unwrap_or("");
        let pasa = filas
            .saturating_mul(ancho)
            .saturating_mul(BYTES_POR_COLUMNA);
        let construye = match nombre {
            "HASH_JOIN" => hijos
                .get(1)
                .map(|(f, a)| f.saturating_mul(*a).saturating_mul(BYTES_POR_COLUMNA))
                .unwrap_or(0),
            n if CONSTRUYEN.contains(&n) => hijos
                .iter()
                .map(|(f, a)| f.saturating_mul(*a).saturating_mul(BYTES_POR_COLUMNA))
                .max()
                .unwrap_or(pasa),
            _ => 0,
        };
        self.operadores += 1;
        self.filas = self.filas.max(filas);
        self.bytes = self.bytes.max(pasa).max(construye);
        (filas, ancho)
    }
}

/// Lo que ocupa una fila generada, para estimar bytes: unas diez columnas.
const BYTES_POR_FILA_GENERADA: u64 = 100;

/// Query Q4·1b · **Las filas que una sentencia GENERA**: `range(…)` y
/// `generate_series(…)` con argumentos literales (`range(n)`, `range(a, b)`,
/// `range(a, b, paso)`). No leen nada del lago, así que `ore explain` no las
/// ve; pero `create or replace dataset … as select … from range(300000000)`
/// escribe 300 M filas, y eso decide la clase tanto como leerlas.
pub fn filas_generadas(texto: &str) -> u64 {
    let t = texto.to_lowercase();
    let mut total: u64 = 0;
    for f in ["generate_series(", "range("] {
        let mut desde = 0;
        while let Some(i) = t[desde..].find(f) {
            let ini = desde + i;
            desde = ini + f.len();
            // `range(` dentro de `generate_series(` no; ni un `xrange(`.
            if t[..ini]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_')
            {
                continue;
            }
            let Some(fin) = t[desde..].find(')') else {
                continue;
            };
            let args: Option<Vec<i128>> = t[desde..desde + fin]
                .split(',')
                .map(|a| a.trim().replace('_', "").parse::<i128>().ok())
                .collect();
            let n = match args.as_deref() {
                Some([n]) => *n,
                Some([a, b]) => b - a,
                Some([a, b, p]) if *p != 0 => (b - a) / p,
                _ => 0,
            };
            total = total.saturating_add(n.clamp(0, u64::MAX as i128) as u64);
        }
    }
    total
}

/// La entrada con lo que la sentencia genera: sus filas, y sus bytes estimados.
pub fn con_lo_generado(mut e: Entrada, filas: u64) -> Entrada {
    e.filas = e.filas.saturating_add(filas);
    e.bytes = e
        .bytes
        .saturating_add(filas.saturating_mul(BYTES_POR_FILA_GENERADA));
    e
}

#[cfg(test)]
mod pruebas {
    fn plan(nombre: &str, extra: &str) -> Node {
        let p = std::fs::read_to_string(format!(
            "{}/tests/planes/{nombre}.json",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        crate::parse::parse(&format!(r#"{{"plan": {p}{extra}}}"#)).unwrap()
    }

    /// Query A2 · El peso sale del plan, con un solo mecanismo: lo generado,
    /// lo que multiplica un join, lo que se agrupa y lo que se escribe (planes
    /// reales de DuckDB 1.5, `tests/planes/`).
    #[test]
    fn el_peso_sale_del_plan() {
        let u = Umbrales::default();
        let g = peso(&plan("genera_300m", "")).unwrap();
        assert_eq!(g.filas, 300_000_000);
        assert!(g.estimada && g.bytes >= 300_000_000 * 8, "{g:?}");
        assert_eq!(admitir(&g, &u).clase, Clase::G);
        let j = peso(&plan("junta", "")).unwrap();
        assert_eq!(j.filas, 100_000);
        assert_eq!(admitir(&j, &u).clase, Clase::L);
        let a = peso(&plan("agrupa", "")).unwrap();
        assert!(a.filas >= 1_000 && a.bytes > 0, "{a:?}");
        let w = peso(&plan("escribe", "")).unwrap();
        assert!(w.filas > 0, "{w:?}");
        // Lo vivo se dice; lo que no se pudo estimar quita la estimación.
        let v = peso(&plan(
            "junta",
            r#", "en_vivo": ["o.s.t"], "sin_estimar": ["c.s.col"]"#,
        ))
        .unwrap();
        assert_eq!(v.en_vivo, ["o.s.t"]);
        assert!(!v.estimada);
        // Sin plan, nada.
        assert!(peso(&crate::parse::parse(r#"{"error": "x"}"#).unwrap()).is_none());
        assert!(peso(&crate::parse::parse(r#"{"plan": []}"#).unwrap()).is_none());
    }

    /// Q4·1b · Lo que una sentencia genera cuenta: 300 M filas de `range()` no
    /// caben en L aunque no lean nada; 3 M sí.
    #[test]
    fn lo_generado_decide_la_clase() {
        assert_eq!(
            filas_generadas("select range from range(300000000)"),
            300_000_000
        );
        assert_eq!(filas_generadas("from RANGE(1, 3_000_001)"), 3_000_000);
        assert_eq!(filas_generadas("from generate_series(0, 100, 10)"), 10);
        assert_eq!(filas_generadas("from xrange(9) join range(n)"), 0);
        let u = Umbrales::default();
        let vacia = Entrada {
            estimada: true,
            ..Default::default()
        };
        assert_eq!(
            admitir(&con_lo_generado(vacia.clone(), 300_000_000), &u).clase,
            Clase::G
        );
        assert_eq!(
            admitir(&con_lo_generado(vacia.clone(), 30_000_000), &u).clase,
            Clase::G
        );
        assert_eq!(
            admitir(&con_lo_generado(vacia, 3_000_000), &u).clase,
            Clase::L
        );
    }

    use super::*;

    fn entrada(filas: u64, bytes: u64) -> Entrada {
        Entrada {
            estimada: true,
            filas,
            bytes,
            ..Default::default()
        }
    }

    /// La escalera de Q0 (§2.1), con sus `gb`: 6 M a L; 60 M y 600 M a G.
    #[test]
    fn la_escalera_de_q0() {
        let u = Umbrales::default();
        assert_eq!(admitir(&entrada(6_000_000, 55_000_000), &u).clase, Clase::L);
        assert_eq!(
            admitir(&entrada(6_000_000, 136_000_000), &u).clase,
            Clase::L
        );
        assert_eq!(
            admitir(&entrada(60_000_000, 427_000_000), &u).clase,
            Clase::G,
            "60 M filas"
        );
        assert_eq!(
            admitir(&entrada(60_000_000, 1_054_000_000), &u).clase,
            Clase::G
        );
        let g = admitir(&entrada(600_000_000, 10_500_000_000), &u);
        assert_eq!((g.clase, g.memoria_mb, g.hilos), (Clase::G, 39_321, 8));
        let l = admitir(&entrada(6_000_000, 55_000_000), &u);
        assert_eq!((l.memoria_mb, l.hilos), (2_048, 2));
    }

    #[test]
    fn sin_estimacion_va_a_lo_grande_nunca_a_lo_pequeno() {
        let e = Entrada {
            estimada: false,
            sin_estimar: vec!["x.y: sin tamaño".into()],
            ..Default::default()
        };
        let a = admitir(&e, &Umbrales::default());
        assert_eq!(a.clase, Clase::G);
        assert!(a.avisos[0].contains("x.y"), "{a:?}");
    }

    #[test]
    fn lo_que_no_cabe_en_un_nodo_va_a_e_salvo_si_lee_en_vivo() {
        let u = Umbrales::default();
        assert_eq!(
            admitir(&entrada(5_000_000_000, 400_000_000_000), &u).clase,
            Clase::E
        );
        let vivo = Entrada {
            en_vivo: vec!["pg.public.clientes".into()],
            ..entrada(5_000_000_000, 400_000_000_000)
        };
        let a = admitir(&vivo, &u);
        assert_eq!(a.clase, Clase::G);
        assert!(a.avisos.iter().any(|x| x.contains("en vivo")), "{a:?}");
    }

    #[test]
    fn la_entrada_sale_del_explain() {
        let plan = crate::parse::parse(
            r#"{"ok":true,"entendida":true,"lecturas":[{"tabla":"pg.public.clientes"}],"lago":[],
                "estimacion":{"tablas":[{"tabla":"geo.trip","filas":6000000,"bytes":55000000,"sin_tamano":[]},
                                        {"tabla":"geo.otra","error":"no contesta"}],"filas":6000000,"bytes":55000000}}"#,
        )
        .unwrap();
        let e = entrada_de_explain(&plan);
        assert_eq!((e.filas, e.bytes), (6_000_000, 55_000_000));
        assert_eq!(e.en_vivo, vec!["pg.public.clientes"]);
        assert!(
            !e.estimada,
            "una tabla sin estimar deja la consulta sin estimar"
        );
        assert!(e.sin_estimar[0].contains("geo.otra"), "{e:?}");

        let sin = crate::parse::parse(r#"{"ok":true,"entendida":true,"lecturas":[]}"#).unwrap();
        assert!(!entrada_de_explain(&sin).estimada);
    }

    #[test]
    fn las_rutas_con_saltos_se_ven() {
        let sql = "select * from read_parquet('gs://b/vectorial/../otra/x.parquet') \
                   where a = 'b' and c = 'x/%2E%2e/y' -- '../en un comentario'";
        assert_eq!(
            rutas_sospechosas(sql),
            vec!["gs://b/vectorial/../otra/x.parquet", "x/%2E%2e/y"]
        );
        assert!(rutas_sospechosas("select 'a.b', 1.5 from t").is_empty());
    }
}
