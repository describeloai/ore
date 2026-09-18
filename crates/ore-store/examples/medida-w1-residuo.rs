//! **Medida W1 · el residuo a mano.** ¿Cuánto cuesta ejecutar la pregunta de
//! una vista —`where` de igualdad y pertenencia, `fields`, `groupBy` con los
//! cinco agregados, `having`— sobre una copia real, con lo que `ore-store` ya
//! da (`sobre::abrir` + `carga::leer`) y sin ningún motor externo?
//!
//! Se corre sobre un artefacto `ore/v1/<sha>` bajado del bucket:
//!
//! ```text
//! cargo build -p ore-store --example medida-w1-residuo
//! target/debug/examples/medida-w1-residuo <fichero.ore>
//! ```
//!
//! Es instrumento, no producto: `pruebas-de-fuego/medida-w1-ejecutar-la-pregunta.py`
//! lo invoca y asienta lo que salió. Mide tiempo porque compara **dónde** corre
//! (un Job, un verbo servido) y ahí el reloj es la unidad; el motor de vistas
//! cuenta filas miradas (0014), y esto no lo toca.
use ore_store::carga::Fila;
use std::collections::BTreeMap;
use std::time::Instant;

fn col<'a>(f: &'a Fila, k: &str) -> &'a str {
    f.get(k).map(String::as_str).unwrap_or("")
}

fn main() {
    let ruta = std::env::args()
        .nth(1)
        .expect("uso: medida-w1-residuo <fichero.ore>");
    let t0 = Instant::now();
    let bytes = std::fs::read(&ruta).expect("no se pudo leer el fichero");
    let (cab, parquet) = ore_store::sobre::abrir(&bytes).expect("no es un artefacto ORECOPY1");
    let t_abrir = t0.elapsed();
    let t1 = Instant::now();
    let filas = ore_store::carga::leer(parquet).expect("el Parquet no se lee");
    let t_leer = t1.elapsed();

    // Qué columnas hay de verdad: una ausente en una fila es un nulo.
    let mut presentes: BTreeMap<&str, usize> = BTreeMap::new();
    for f in &filas {
        for k in f.keys() {
            *presentes.entry(k.as_str()).or_insert(0) += 1;
        }
    }
    // Las columnas que la cabecera declara: las claves de `esquema`.
    let declaradas = cab
        .split("\"esquema\":{")
        .nth(1)
        .and_then(|r| r.split('}').next())
        .map(|e| e.matches("\":\"").count())
        .unwrap_or(0);
    println!(
        "abrir {t_abrir:?} · leer {t_leer:?} · {} filas · sobre {} B · parquet {} B",
        filas.len(),
        cab.len(),
        parquet.len()
    );
    println!("columnas con algún valor: {presentes:?} (la cabecera declara {declaradas})");

    let clave = presentes
        .keys()
        .find(|k| k.contains("category") || k.contains("state"))
        .copied()
        .unwrap_or("");
    let numerica = presentes
        .keys()
        .find(|k| k.contains("weight") || k.contains("zip"))
        .copied()
        .unwrap_or("");

    // Q1 · where eq + fields
    let t = Instant::now();
    let primero = filas
        .first()
        .map(|f| col(f, clave).to_string())
        .unwrap_or_default();
    let q1 = filas.iter().filter(|f| col(f, clave) == primero).count();
    println!(
        "Q1 where {clave} == {primero:?} → {q1} filas en {:?}",
        t.elapsed()
    );

    // Q2 · where in
    let t = Instant::now();
    let dentro: Vec<String> = filas
        .iter()
        .map(|f| col(f, clave).to_string())
        .take(300)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .take(3)
        .collect();
    let q2 = filas
        .iter()
        .filter(|f| dentro.iter().any(|d| d == col(f, clave)))
        .count();
    println!(
        "Q2 where {clave} in {dentro:?} → {q2} filas en {:?}",
        t.elapsed()
    );

    // Q3 · groupBy clave: count(), avg(numerica), having count >= 100
    let t = Instant::now();
    let mut grupos: BTreeMap<&str, (u64, f64)> = BTreeMap::new();
    for f in &filas {
        let v: f64 = f.get(numerica).and_then(|v| v.parse().ok()).unwrap_or(0.0);
        let g = grupos.entry(col(f, clave)).or_insert((0, 0.0));
        g.0 += 1;
        g.1 += v;
    }
    let q3 = grupos.values().filter(|g| g.0 >= 100).count();
    println!(
        "Q3 groupBy {clave} · count(), avg({numerica}) · having count >= 100 → {q3} grupos de {} en {:?}",
        grupos.len(),
        t.elapsed()
    );

    // Q4 · Run sin recorte: 200 filas como JSON por línea
    let t = Instant::now();
    let mut out = String::new();
    for f in filas.iter().take(200) {
        out.push('{');
        for (i, (k, v)) in f.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(&format!("\"{k}\":\"{v}\""));
        }
        out.push_str("}\n");
    }
    println!("Q4 limit 200 → {} B en {:?}", out.len(), t.elapsed());
    println!("total {:?}", t0.elapsed());
}
