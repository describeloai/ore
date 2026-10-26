//! Lo que cuesta armar un lote con `ore-arrow` frente al suelo (los builders de Arrow a pelo).
//!   cargo run --release -p ore-arrow --example medir
use arrow_array::Array;
use arrow_array::builder::*;
use ore_arrow::{Columna, RecordBatchBuilder};
use ore_base::tipos::{Fisico, Valor};
use std::time::Instant;

const N: usize = 2_000_000;

fn nulo(i: usize) -> bool {
    i % 20 == 7
}

fn main() {
    let textos: Vec<String> = (0..1000).map(|i| format!("cliente-{i:05}-ñandú")).collect();
    let bin = [7u8; 16];

    // A · el suelo: los builders de Arrow, sin nada entre medias.
    let t = Instant::now();
    let (mut e, mut r, mut s, mut f, mut ins, mut d, mut b, mut l) = (
        Int64Builder::new(),
        Float64Builder::new(),
        StringBuilder::new(),
        Date32Builder::new(),
        TimestampMicrosecondBuilder::new().with_timezone("UTC"),
        Decimal128Builder::new()
            .with_precision_and_scale(12, 2)
            .unwrap(),
        BinaryBuilder::new(),
        BooleanBuilder::new(),
    );
    for i in 0..N {
        if nulo(i) {
            e.append_null();
            r.append_null();
            s.append_null();
            f.append_null();
            ins.append_null();
            d.append_null();
            b.append_null();
            l.append_null();
        } else {
            e.append_value(i as i64);
            r.append_value(i as f64 * 0.5);
            s.append_value(&textos[i % 1000]);
            f.append_value(20_000 + (i % 400) as i32);
            ins.append_value(1_792_914_255_123_456 + i as i64);
            d.append_value(i as i128 * 100);
            b.append_value(bin);
            l.append_value(i % 2 == 0);
        }
    }
    let n_a = [
        e.finish().len(),
        r.finish().len(),
        s.finish().len(),
        f.finish().len(),
        ins.finish().len(),
        d.finish().len(),
        b.finish().len(),
        l.finish().len(),
    ];
    let a = t.elapsed();

    // B · ore-arrow: una fila de `Option<Valor>` por fila (lo que hoy hacen los orígenes).
    // C · solo construir las filas de entrada, sin entregarlas (lo que cuesta producir los `Valor`).
    let t = Instant::now();
    let mut total = 0usize;
    for i in 0..N {
        let fila: Vec<Option<Valor>> = if nulo(i) {
            vec![None; 8]
        } else {
            vec![
                Some(Valor::Entero(i as i64)),
                Some(Valor::Real(i as f64 * 0.5)),
                Some(Valor::Texto(textos[i % 1000].clone())),
                Some(Valor::Fecha(20_000 + (i % 400) as i32)),
                Some(Valor::Instante(1_792_914_255_123_456 + i as i64)),
                Some(Valor::Decimal(i as i128 * 100)),
                Some(Valor::Bytes(bin.to_vec())),
                Some(Valor::Logico(i % 2 == 0)),
            ]
        };
        total += fila.len();
    }
    let c_t = t.elapsed();
    assert_eq!(total, N * 8);
    let cols = vec![
        Columna::new("e", Fisico::Entero, "Integer"),
        Columna::new("r", Fisico::Real, "Float"),
        Columna::new("s", Fisico::Texto, "String"),
        Columna::new("f", Fisico::Fecha, "Date"),
        Columna::new("i", Fisico::Instante, "DateTimeTz"),
        Columna::new(
            "d",
            Fisico::Decimal {
                precision: 12,
                escala: 2,
            },
            "Decimal<12, 2>",
        ),
        Columna::new("b", Fisico::Binario, "Binary"),
        Columna::new("l", Fisico::Logico, "Boolean"),
    ];
    let t = Instant::now();
    let mut bl = RecordBatchBuilder::nuevo(&cols).unwrap();
    for i in 0..N {
        let fila: Vec<Option<Valor>> = if nulo(i) {
            vec![None; 8]
        } else {
            vec![
                Some(Valor::Entero(i as i64)),
                Some(Valor::Real(i as f64 * 0.5)),
                Some(Valor::Texto(textos[i % 1000].clone())),
                Some(Valor::Fecha(20_000 + (i % 400) as i32)),
                Some(Valor::Instante(1_792_914_255_123_456 + i as i64)),
                Some(Valor::Decimal(i as i128 * 100)),
                Some(Valor::Bytes(bin.to_vec())),
                Some(Valor::Logico(i % 2 == 0)),
            ]
        };
        bl.push(&fila).unwrap();
    }
    let lote = bl.finish().unwrap();
    let b_t = t.elapsed();
    assert_eq!(lote.num_rows(), N);
    assert_eq!(n_a[0], N);

    // D y E · la escritura tipada: sin `Valor`, desde el búfer propio; E con la capacidad ya reservada.
    let tipada = |con_capacidad: bool| {
        let t = Instant::now();
        let mut bl = if con_capacidad {
            RecordBatchBuilder::con_capacidad(&cols, N).unwrap()
        } else {
            RecordBatchBuilder::nuevo(&cols).unwrap()
        };
        for i in 0..N {
            if nulo(i) {
                for c in 0..8 {
                    bl.col(c).nulo();
                }
            } else {
                bl.col(0).entero(i as i64).unwrap();
                bl.col(1).real(i as f64 * 0.5).unwrap();
                bl.col(2).texto(&textos[i % 1000]).unwrap();
                bl.col(3).fecha(20_000 + (i % 400) as i32).unwrap();
                bl.col(4).micros(1_792_914_255_123_456 + i as i64).unwrap();
                bl.col(5).decimal(i as i128 * 100).unwrap();
                bl.col(6).bytes(&bin).unwrap();
                bl.col(7).logico(i % 2 == 0).unwrap();
            }
            bl.fila_escrita();
        }
        let lote = bl.finish().unwrap();
        assert_eq!(lote.num_rows(), N);
        t.elapsed()
    };
    let d_t = tipada(false);
    let e_t = tipada(true);

    println!("{N} filas × 8 columnas");
    println!(
        "  A · builders de Arrow a pelo : {:>7.0} ms",
        a.as_secs_f64() * 1e3
    );
    println!(
        "  B · ore-arrow push (Valor)   : {:>7.0} ms  ({:.1}× el suelo)",
        b_t.as_secs_f64() * 1e3,
        b_t.as_secs_f64() / a.as_secs_f64()
    );
    println!(
        "  C · solo producir las filas  : {:>7.0} ms  (B − C = lo que cuesta ore-arrow: {:.0} ms, {:.1}× el suelo)",
        c_t.as_secs_f64() * 1e3,
        (b_t - c_t).as_secs_f64() * 1e3,
        (b_t - c_t).as_secs_f64() / a.as_secs_f64()
    );
    println!(
        "  D · ore-arrow tipado         : {:>7.0} ms  ({:.2}× el suelo)",
        d_t.as_secs_f64() * 1e3,
        d_t.as_secs_f64() / a.as_secs_f64()
    );
    println!(
        "  E · tipado + capacidad       : {:>7.0} ms  ({:.2}× el suelo)",
        e_t.as_secs_f64() * 1e3,
        e_t.as_secs_f64() / a.as_secs_f64()
    );
}
