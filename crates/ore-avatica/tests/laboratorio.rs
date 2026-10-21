//! El cliente contra un **PQS de verdad** (el laboratorio de `pruebas-de-fuego/phoenix/`, sembrado con
//! `semilla.py`). Solo corre con `PHOENIX_LAB_URL=http://127.0.0.1:8765/`; sin ella, no hace nada.

use std::time::Instant;

use ore_avatica::{Conexion, Error, Opciones, Valor};

fn url() -> Option<String> {
    std::env::var("PHOENIX_LAB_URL")
        .ok()
        .filter(|u| !u.is_empty())
}

fn abrir(frame: i32) -> Option<Conexion> {
    let mut o = Opciones::nuevas(url()?);
    o.filas_por_frame = frame;
    Some(Conexion::abrir(&o).expect("abrir contra el laboratorio"))
}

#[test]
fn un_barrido_completo_de_262144_filas_llega_entero_y_en_orden() {
    let Some(c) = abrir(10_000) else { return };
    let t = Instant::now();
    let mut l = c
        .ejecutar("SELECT id, cliente, importe, creado, estado FROM ore_x0.pedidos")
        .unwrap();
    let (mut filas, mut frames, mut suma, mut anterior) = (0u64, 0, 0i128, 0i64);
    while let Some(f) = l.siguiente_frame().unwrap() {
        frames += 1;
        for r in &f {
            let Valor::Entero(id) = r[0] else {
                panic!("{r:?}")
            };
            assert!(
                id > anterior,
                "el barrido de una clave simple sale ordenado: {anterior} → {id}"
            );
            anterior = id;
            suma += i128::from(id);
            assert!(matches!(r[3], Valor::Nulo), "creado es nulo en la semilla");
        }
        filas += f.len() as u64;
    }
    let n = 262_144u64;
    assert_eq!((filas, l.leidas()), (n, n));
    assert_eq!(
        suma,
        i128::from(n) * i128::from(n + 1) / 2,
        "ni una fila de más ni de menos"
    );
    assert!((27..=28).contains(&frames), "{frames} frames de 10 000");
    eprintln!(
        "barrido: {n} filas en {frames} frames, {:.1}s ({:.0} filas/s)",
        t.elapsed().as_secs_f64(),
        n as f64 / t.elapsed().as_secs_f64()
    );
}

#[test]
fn varias_lecturas_en_paralelo_por_rangos_de_la_clave_suman_la_tabla() {
    let Some(_) = url() else { return };
    let t = Instant::now();
    let hilos: Vec<_> = (0..4)
        .map(|i| {
            std::thread::spawn(move || {
                let c = abrir(20_000).unwrap();
                let (a, b) = (1 + i * 65_536, 1 + (i + 1) * 65_536);
                let mut l = c.ejecutar(&format!("SELECT id, cliente, importe, creado, estado FROM ore_x0.pedidos WHERE id >= {a} AND id < {b}")).unwrap();
                let mut n = 0;
                while let Some(f) = l.siguiente_frame().unwrap() {
                    n += f.len();
                }
                n
            })
        })
        .collect();
    let total: usize = hilos.into_iter().map(|h| h.join().unwrap()).sum();
    assert_eq!(total, 262_144);
    eprintln!(
        "4 conexiones en paralelo: {total} filas en {:.1}s",
        t.elapsed().as_secs_f64()
    );
}

#[test]
fn los_tipos_del_laboratorio_son_los_de_las_trazas() {
    let Some(c) = abrir(100) else { return };
    let mut l = c
        .ejecutar("SELECT * FROM ore_x0.tipos ORDER BY k1")
        .unwrap();
    let filas = l.siguiente_frame().unwrap().unwrap();
    assert_eq!(filas.len(), 2);
    let col = |n: &str| l.columnas().iter().position(|c| c.nombre == n).unwrap();
    let a = &filas[0];
    assert_eq!(a[col("T_DEC")], Valor::Decimal("12345.678".into()));
    assert_eq!(
        a[col("T_DECSIN")],
        Valor::Decimal("3.14159265358979323846".into())
    );
    assert_eq!(a[col("T_TS")], Valor::Entero(1_792_503_959_123)); // los nanosegundos no llegan
    assert_eq!(a[col("T_DATE")], Valor::Entero(20_746)); // días: la hora de un DATE no llega
    assert_eq!(
        a[col("T_AVAR")],
        Valor::Lista(vec![Valor::Texto("x".into()), Valor::Texto("y".into())])
    );
    assert!(filas[1][col("T_BIG")] == Valor::Nulo);
    // `CAST(date AS TIMESTAMP)` recupera la hora (0071 §3.4).
    let mut l = c
        .ejecutar("SELECT CAST(t_date AS TIMESTAMP) FROM ore_x0.tipos WHERE k1 = 'a'")
        .unwrap();
    assert_eq!(
        l.siguiente_frame().unwrap().unwrap()[0][0],
        Valor::Entero(1_792_503_959_000)
    );
}

#[test]
fn el_catalogo_del_laboratorio() {
    let Some(c) = abrir(100) else { return };
    let t = c.tablas(Some("ORE_X0"), None, &[]).unwrap();
    let (n, sal) = (
        t.indice("TABLE_NAME").unwrap(),
        t.indice("SALT_BUCKETS").unwrap(),
    );
    let salada = t
        .filas
        .iter()
        .find(|f| f[n] == Valor::Texto("SALADA".into()))
        .expect("SALADA");
    assert_eq!(salada[sal], Valor::Entero(4));
    // Con sal, la columna de sal no se enseña.
    let cols = c.columnas(Some("ORE_X0"), Some("SALADA")).unwrap();
    let nombres: Vec<&Valor> = cols
        .filas
        .iter()
        .map(|f| &f[cols.indice("COLUMN_NAME").unwrap()])
        .collect();
    assert_eq!(
        nombres,
        [&Valor::Texto("ID".into()), &Valor::Texto("V".into())]
    );
    // La clave primaria: `KEY_SEQ`.
    let tipos = c.columnas(Some("ORE_X0"), Some("TIPOS")).unwrap();
    let k = tipos.indice("KEY_SEQ").unwrap();
    assert!(
        tipos
            .filas
            .iter()
            .filter(|f| f[k] != Valor::Nulo && f[k] != Valor::Entero(0))
            .count()
            >= 2,
        "k1 y k2 son la clave"
    );
}

#[test]
fn los_errores_reales_y_el_tiempo_limite() {
    let Some(c) = abrir(100) else { return };
    let Err(e) = c.ejecutar("SELECT * FROM no_existe") else {
        panic!()
    };
    assert!(
        matches!(&e, Error::Servidor { mensaje, .. } if mensaje.contains("ERROR 1012 (42M03)")),
        "{e:?}"
    );

    let mut o = Opciones::nuevas(url().unwrap());
    o.propiedades
        .insert("phoenix.query.timeoutMs".into(), "300".into());
    let c = Conexion::abrir(&o).unwrap();
    let Err(e) = c.ejecutar(
        "SELECT COUNT(*) FROM ore_x0.pedidos a, ore_x0.pedidos b WHERE a.importe = b.importe",
    ) else {
        panic!()
    };
    assert!(e.es_tiempo_limite(), "{e:?}");
}

#[test]
fn la_conexion_sigue_sirviendo_despues_de_un_error() {
    let Some(c) = abrir(100) else { return };
    assert!(c.ejecutar("SELEC 1").is_err());
    let mut l = c.ejecutar("SELECT COUNT(*) FROM ore_x0.salada").unwrap();
    assert_eq!(
        l.siguiente_frame().unwrap().unwrap()[0][0],
        Valor::Entero(40)
    );
}

#[test]
#[ignore = "exploración: aperturas simultáneas"]
fn explorar_aperturas() {
    let Some(u) = url() else { return };
    // secuencial
    let mut mal = 0;
    for _ in 0..20 {
        if Conexion::abrir(&Opciones::nuevas(u.clone())).is_err() {
            mal += 1;
        }
    }
    eprintln!("secuencial: {mal}/20 fallos");
    for hilos in [2usize, 4, 8] {
        let b = std::sync::Arc::new(std::sync::Barrier::new(hilos));
        let mut mal = 0;
        for _ in 0..5 {
            let hs: Vec<_> = (0..hilos)
                .map(|_| {
                    let (u, b) = (u.clone(), b.clone());
                    std::thread::spawn(move || {
                        b.wait();
                        Conexion::abrir(&Opciones::nuevas(u))
                            .map(|_| ())
                            .map_err(|e| e.to_string())
                    })
                })
                .collect();
            for h in hs {
                if let Err(e) = h.join().unwrap() {
                    mal += 1;
                    eprintln!("   {}", &e[e.len().saturating_sub(90)..]);
                }
            }
        }
        eprintln!("{hilos} a la vez x5 rondas: {mal}/{} fallos", hilos * 5);
    }
}
