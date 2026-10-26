//! Lo que cuesta, en Oracle, armar el lote de Arrow desde las celdas ya leídas (ORE 0075).
//!   cargo run --release -p ore-oracle --example medir
//!
//! Las celdas llegan como texto canónico (`Option<String>`), como las deja `lectura.rs`. Se mide solo lo que
//! hace `Constructor::push`: analizar el texto y armar el lote; leer de la red y decodificar la celda nativa
//! queda fuera, y es previo.
use ore_arrow::{Columna, RecordBatchBuilder};
use ore_base::tipos::{Fisico, Valor};
use ore_oracle::{Constructor, Tipo};
use std::time::Instant;

const N: usize = 500_000;

fn main() {
    let tipos: Vec<(String, Tipo)> = vec![
        ("e".into(), Tipo::Entero),
        ("r".into(), Tipo::Real),
        ("t".into(), Tipo::Texto),
        ("fh".into(), Tipo::FechaHora),
        ("i".into(), Tipo::Instante),
        (
            "d".into(),
            Tipo::Decimal {
                precision: 12,
                escala: 2,
            },
        ),
        ("b".into(), Tipo::Binario),
        ("l".into(), Tipo::Logico),
    ];
    let filas: Vec<Vec<Option<String>>> = (0..N)
        .map(|i| {
            if i % 20 == 7 {
                return vec![None; 8];
            }
            vec![
                Some(i.to_string()),
                Some(format!("{i}.5")),
                Some(format!("cliente-{:05}-ñandú", i % 1000)),
                Some(format!("2026-10-25 13:14:{:02}.123456", i % 60)),
                Some(format!("2026-10-25 07:44:{:02}.123456+00", i % 60)),
                Some(format!("{i}.{:02}", i % 100)),
                Some("3q2+7w==".to_string()),
                Some(if i % 2 == 0 { "true" } else { "false" }.to_string()),
            ]
        })
        .collect();

    // A · el Constructor de hoy: analizar + `Vec<Option<Valor>>` + `push`.
    let t = Instant::now();
    // Con lotes de `FILAS_POR_LOTE` filas, como los emite `lectura.rs`.
    let mut c = Constructor::nuevo(&tipos).unwrap();
    let mut emitidas = 0usize;
    for f in &filas {
        c.push(f).unwrap();
        if c.filas() >= ore_oracle::lectura::FILAS_POR_LOTE {
            emitidas += c.finish().unwrap().num_rows();
        }
    }
    emitidas += c.finish().unwrap().num_rows();
    let a = t.elapsed();
    assert_eq!(emitidas, N);

    // B · lo mismo, escribiendo con el `Escritor` tipado y con la capacidad reservada; el texto va directo.
    let fisicos: Vec<Fisico> = tipos.iter().map(|(_, t)| t.fisico()).collect();
    let cols: Vec<Columna> = tipos
        .iter()
        .map(|(p, t)| Columna::new(p, t.fisico(), t.oos()))
        .collect();
    let t = Instant::now();
    let mut b = RecordBatchBuilder::con_capacidad(&cols, N).unwrap();
    for f in &filas {
        for (i, (celda, fisico)) in f.iter().zip(&fisicos).enumerate() {
            let mut e = b.col(i);
            match celda {
                None => e.nulo(),
                Some(t) if *fisico == Fisico::Texto => e.texto(t).unwrap(),
                Some(t) => e.valor(&fisico.analizar(t).unwrap()).unwrap(),
            }
        }
        b.fila_escrita();
    }
    let lote_b = b.finish().unwrap();
    let bt = t.elapsed();
    assert_eq!(lote_b.num_rows(), N);

    // C · solo analizar el texto, sin escribir nada.
    let t = Instant::now();
    let mut ok = 0usize;
    for f in &filas {
        for (celda, fisico) in f.iter().zip(&fisicos) {
            if let Some(t) = celda
                && let Some(v) = fisico.analizar(t)
                && !matches!(v, Valor::Texto(_))
            {
                ok += 1;
            }
        }
    }
    let ct = t.elapsed();
    assert!(ok > 0);

    // D · cuánto cuesta analizar cada columna por separado.
    let mut por_columna = Vec::new();
    for (i, (nombre, _)) in tipos.iter().enumerate() {
        let t = Instant::now();
        let mut ok = 0usize;
        for f in &filas {
            if let Some(t) = &f[i]
                && fisicos[i].analizar(t).is_some()
            {
                ok += 1;
            }
        }
        assert!(ok > 0);
        por_columna.push((nombre.clone(), fisicos[i], t.elapsed()));
    }

    println!("{N} filas × 8 columnas, de las celdas de Oracle (texto canónico) al lote");
    println!(
        "  A · Constructor::push (hoy)        : {:>6.0} ms",
        a.as_secs_f64() * 1e3
    );
    println!(
        "  B · Escritor tipado + capacidad     : {:>6.0} ms  ({:.2}× A)",
        bt.as_secs_f64() * 1e3,
        bt.as_secs_f64() / a.as_secs_f64()
    );
    println!(
        "  C · solo analizar el texto          : {:>6.0} ms  ({:.0} % de A)",
        ct.as_secs_f64() * 1e3,
        100.0 * ct.as_secs_f64() / a.as_secs_f64()
    );
    println!("  D · analizar, columna a columna:");
    for (n, f, d) in por_columna {
        println!(
            "      {n:<3} {:<22} {:>5.0} ms  ({:>4.0} ns por celda)",
            f.nombre(),
            d.as_secs_f64() * 1e3,
            d.as_secs_f64() * 1e9 / N as f64
        );
    }
}
