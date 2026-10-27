//! `carga::lote` (texto del driver → lote tipado) frente a la misma tarea con `ore-arrow` (ADR 0075, fase 3).
//!   cargo run --release -p ore-store --example medir-lote
use ore_arrow::{Columna, RecordBatchBuilder};
use ore_base::tipos::Fisico;
use ore_store::carga::{self, Fila};
use std::collections::BTreeMap;
use std::time::Instant;

const N: usize = 500_000;

fn main() {
    let esquema: BTreeMap<String, String> = [
        ("e", "Integer"),
        ("r", "Float"),
        ("s", "String"),
        ("f", "Date"),
        ("i", "DateTimeTz"),
        ("d", "Decimal<12, 2>"),
        ("l", "Boolean"),
        ("b", "Binary"),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    let filas: Vec<Fila> = (0..N)
        .map(|i| {
            let mut f = Fila::new();
            if i % 20 != 7 {
                f.insert("e".into(), i.to_string());
                f.insert("r".into(), format!("{}.5", i));
                f.insert("s".into(), format!("cliente-{:05}-ñandú", i % 1000));
                f.insert(
                    "f".into(),
                    format!("2024-{:02}-{:02}", 1 + i % 12, 1 + i % 28),
                );
                f.insert(
                    "i".into(),
                    format!("2026-10-26T10:11:{:02}.123456Z", i % 60),
                );
                f.insert("d".into(), format!("{}.{:02}", i, i % 100));
                f.insert("l".into(), (i % 2 == 0).to_string());
                f.insert("b".into(), "BwcHBwcHBwcHBwcHBwcHBw==".into());
            }
            f
        })
        .collect();

    // A · carga::lote, como está hoy.
    let t = Instant::now();
    let l = carga::lote(&esquema, &filas).unwrap();
    let a = t.elapsed();
    assert!(l.sin_estrechar.is_empty(), "{:?}", l.sin_estrechar);
    assert_eq!(l.lote.num_rows(), N);

    // B · lo mismo con ore-arrow: se analiza el texto en una pasada por columna y se escribe tipado, con capacidad.
    let t = Instant::now();
    let fisicos: Vec<(String, Fisico, String)> = esquema
        .iter()
        .map(|(n, tipo)| {
            let f = ore_base::types::parse_type(tipo)
                .map(|t| Fisico::de(&t))
                .unwrap();
            (n.clone(), f, tipo.clone())
        })
        .collect();
    let cols: Vec<Columna> = fisicos
        .iter()
        .map(|(n, f, t)| Columna::new(n, *f, t))
        .collect();
    let mut b = RecordBatchBuilder::con_capacidad(&cols, N).unwrap();
    // Por columnas, como `carga`: cada columna se recorre entera.
    let mut lotes_por_columna = Vec::new();
    for (i, (nombre, fisico, _)) in fisicos.iter().enumerate() {
        let mut un = RecordBatchBuilder::con_capacidad(&cols[i..=i], N).unwrap();
        for f in &filas {
            let mut e = un.col(0);
            match f.get(nombre) {
                None => e.nulo(),
                Some(t) => match fisico.analizar(t) {
                    Some(v) => e.valor(&v).unwrap(),
                    None => panic!("`{t}` no analiza como {}", fisico.nombre()),
                },
            }
            un.fila_escrita();
        }
        lotes_por_columna.push(un.finish().unwrap());
    }
    let _ = &mut b;
    let bt = t.elapsed();
    assert_eq!(lotes_por_columna[0].num_rows(), N);

    // C · solo buscar en la fila y analizar el texto, sin escribir nada.
    let t = Instant::now();
    let mut vistos = 0usize;
    for (nombre, fisico, _) in &fisicos {
        for f in &filas {
            if let Some(t) = f.get(nombre)
                && fisico.analizar(t).is_some()
            {
                vistos += 1;
            }
        }
    }
    let ct = t.elapsed();
    assert!(vistos > 0);
    // D · solo buscar en la fila (sin analizar).
    let t = Instant::now();
    let mut hay = 0usize;
    for (nombre, _, _) in &fisicos {
        for f in &filas {
            if f.get(nombre).is_some() {
                hay += 1;
            }
        }
    }
    let dt = t.elapsed();
    assert!(hay > 0);

    println!("{N} filas × 8 columnas, de texto a lote tipado");
    println!(
        "  A · carga::lote (hoy)            : {:>6.0} ms",
        a.as_secs_f64() * 1e3
    );
    println!(
        "  B · ore-arrow, columna a columna : {:>6.0} ms  ({:.2}× carga)",
        bt.as_secs_f64() * 1e3,
        bt.as_secs_f64() / a.as_secs_f64()
    );
    println!(
        "  C · solo buscar y analizar       : {:>6.0} ms  ({:.0} % de A)",
        ct.as_secs_f64() * 1e3,
        100.0 * ct.as_secs_f64() / a.as_secs_f64()
    );
    println!(
        "  D · solo buscar en la fila       : {:>6.0} ms  ({:.0} % de A)",
        dt.as_secs_f64() * 1e3,
        100.0 * dt.as_secs_f64() / a.as_secs_f64()
    );
}
