//! El conector contra un **PQS de verdad** (el laboratorio de `pruebas-de-fuego/phoenix/`, sembrado con
//! `semilla.py`). Solo corre con `PHOENIX_LAB_URL=phoenix://127.0.0.1:8765`; sin ella, no hace nada.

use std::io::Cursor;
use std::time::Instant;

use arrow_array::cast::AsArray;
use arrow_array::types::{Int64Type, TimestampMicrosecondType};
use arrow_array::{Array, RecordBatch};
use arrow_ipc::reader::StreamReader;
use ore_driver::origen::Origen;
use ore_driver::{Codigo, Fallo, Peticion};
use ore_phoenix::lectura::{Phoenix, Token};
use ore_phoenix::verbos;

fn url() -> Option<String> {
    std::env::var("PHOENIX_LAB_URL")
        .ok()
        .filter(|u| !u.is_empty())
}

fn peticion(objeto: &str, cols: &[(&str, &str)]) -> Option<Peticion> {
    Some(Peticion {
        url: url()?,
        objeto: objeto.into(),
        proyeccion: cols
            .iter()
            .map(|(p, c)| (p.to_string(), c.to_string()))
            .collect(),
        ..Default::default()
    })
}

fn leer(p: &Peticion) -> Result<(Vec<RecordBatch>, u64), Fallo> {
    let mut s = Phoenix::conectar(&p.url)?;
    let mut salida = Vec::new();
    let filas = Phoenix::leer(&mut s, &Token::default(), p, &mut salida)?;
    let lector = StreamReader::try_new(Cursor::new(salida), None).expect("un flujo Arrow IPC");
    Ok((lector.map(|b| b.expect("lote")).collect(), filas))
}

#[test]
fn una_tabla_de_262144_filas_llega_entera_en_lotes_acotados() {
    let Some(p) = peticion(
        "ORE_X0.PEDIDOS",
        &[
            ("id", "ID"),
            ("cliente", "CLIENTE"),
            ("importe", "IMPORTE"),
            ("estado", "ESTADO"),
        ],
    ) else {
        return;
    };
    let t = Instant::now();
    let (lotes, filas) = leer(&p).expect("leer");
    let n = 262_144u64;
    assert_eq!(filas, n);
    assert_eq!(lotes.iter().map(|b| b.num_rows() as u64).sum::<u64>(), n);
    // La memoria es un frame y un lote, no la tabla: ningún lote pasa de 8 192 filas.
    assert!(
        lotes.iter().all(|b| b.num_rows() <= 8192),
        "{:?}",
        lotes.iter().map(|b| b.num_rows()).max()
    );
    let suma: i128 = lotes
        .iter()
        .flat_map(|b| {
            b.column(0)
                .as_primitive::<Int64Type>()
                .values()
                .iter()
                .map(|v| i128::from(*v))
        })
        .sum();
    assert_eq!(
        suma,
        i128::from(n) * i128::from(n + 1) / 2,
        "ni una fila de más ni de menos"
    );
    // El importe es un `DECIMAL(12,2)`: exacto.
    let schema = lotes[0].schema();
    assert_eq!(
        schema.field(2).data_type(),
        &arrow_schema::DataType::Decimal128(12, 2)
    );
    eprintln!(
        "pedidos: {n} filas en {} lotes, {:.1}s ({:.0} filas/s)",
        lotes.len(),
        t.elapsed().as_secs_f64(),
        n as f64 / t.elapsed().as_secs_f64()
    );
}

#[test]
fn limit_se_empuja_y_para_en_las_n_filas() {
    let Some(mut p) = peticion("ORE_X0.PEDIDOS", &[("id", "ID")]) else {
        return;
    };
    p.limit = Some(7);
    let (_, filas) = leer(&p).expect("leer");
    assert_eq!(filas, 7);
}

#[test]
fn los_tipos_de_phoenix_en_arrow_contra_el_servidor() {
    let cols: Vec<(String, String)> = [
        "K1", "K2", "T_TINY", "T_UINT", "T_ULONG", "T_DEC", "T_DECSIN", "T_BOOL", "T_VAR", "T_BIN",
        "T_DATE", "T_TIME", "T_TS", "T_AINT",
    ]
    .iter()
    .map(|c| (c.to_lowercase(), c.to_string()))
    .collect();
    let Some(mut p) = peticion("ORE_X0.TIPOS", &[]) else {
        return;
    };
    p.proyeccion = cols;
    let (lotes, _) = leer(&p).expect("leer");
    let b = &lotes[0];
    let col = |n: &str| b.column_by_name(n).unwrap();
    assert_eq!(
        col("t_ulong").as_primitive::<Int64Type>().value(0),
        9_000_000_000_000_000_000
    );
    assert_eq!(
        col("t_decsin").as_string::<i32>().value(0),
        "3.14159265358979323846"
    );
    assert_eq!(col("t_var").as_string::<i32>().value(0), "héllo ñ 日本");
    assert_eq!(col("t_aint").as_string::<i32>().value(0), "[1,2,3]");
    assert_eq!(
        col("t_ts")
            .as_primitive::<TimestampMicrosecondType>()
            .value(0),
        1_792_503_959_123_000
    );
    // Un `DATE` sin declarar llega **con su hora** (`CAST … AS TIMESTAMP`): 2026-10-20 13:45:59.
    assert_eq!(
        col("t_date").data_type(),
        &arrow_schema::DataType::Timestamp(arrow_schema::TimeUnit::Microsecond, Some("UTC".into()))
    );
    assert_eq!(
        col("t_date")
            .as_primitive::<TimestampMicrosecondType>()
            .value(0),
        1_792_503_959_000_000
    );
    assert!(
        col("t_dec")
            .as_primitive::<arrow_array::types::Decimal128Type>()
            .value(0)
            == 12_345_678
    );
    assert!(col("t_tiny").is_null(1) && col("t_aint").is_null(1));
}

#[test]
fn el_nombre_sin_mayusculas_se_resuelve_como_lo_haria_phoenix() {
    let Some(p) = peticion("ore_x0.salada", &[("id", "ID"), ("v", "V")]) else {
        return;
    };
    let (_, filas) = leer(&p).expect("leer");
    assert_eq!(filas, 40);
}

#[test]
fn los_errores_en_sus_codigos() {
    let Some(p) = peticion("ORE_X0.NO_EXISTE", &[("a", "A")]) else {
        return;
    };
    assert_eq!(leer(&p).unwrap_err().codigo, Codigo::Objeto);
    let p = peticion("ORE_X0.PEDIDOS", &[("x", "NO_EXISTE")]).unwrap();
    let e = leer(&p).unwrap_err();
    assert_eq!(e.codigo, Codigo::Esquema);
    assert!(e.mensaje.contains("\"ID\""), "{}", e.mensaje);
    let mut p = peticion("ORE_X0.PEDIDOS", &[("id", "ID")]).unwrap();
    p.filtros = vec![ore_driver::Filtro::uno("ID", "eq", "1")];
    assert_eq!(leer(&p).unwrap_err().codigo, Codigo::Operador);
    // Un tipo declarado que el dato no lleva.
    let mut p = peticion("ORE_X0.PEDIDOS", &[("estado", "ESTADO")]).unwrap();
    p.tipos = vec![("ESTADO".into(), "Integer".into())];
    assert_eq!(leer(&p).unwrap_err().codigo, Codigo::Esquema);
}

#[test]
fn un_tiempo_limite_de_la_peticion_corta_en_el_servidor() {
    let Some(mut p) = peticion("ORE_X0.PEDIDOS", &[("id", "ID"), ("cliente", "CLIENTE")]) else {
        return;
    };
    p.timeout_ms = Some(1);
    let e = leer(&p).unwrap_err();
    assert_eq!(e.codigo, Codigo::Tiempo, "{e:?}");
    assert!(e.reintentable);
}

#[test]
fn cancelar_a_mitad_deja_de_pedir_frames() {
    let Some(mut p) = peticion(
        "ORE_X0.PEDIDOS",
        &[("id", "ID"), ("cliente", "CLIENTE"), ("estado", "ESTADO")],
    ) else {
        return;
    };
    // Frames chicos: que haya donde cortar.
    p.url = format!("{}?frame=1000", p.url);
    let token = Token::default();
    let t = token.clone();
    // `leer` rearma el token al empezar: una cancelación que llega antes de que empiece se pierde (por
    // diseño: es de la lectura anterior). Se insiste hasta que la lectura acaba.
    let acabada = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let a = acabada.clone();
    let hilo = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(300));
        while !a.load(std::sync::atomic::Ordering::SeqCst) {
            Phoenix::cancelar(&t);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    });
    let mut s = Phoenix::conectar(&p.url).unwrap();
    let mut salida = Vec::new();
    let resultado = Phoenix::leer(&mut s, &token, &p, &mut salida);
    acabada.store(true, std::sync::atomic::Ordering::SeqCst);
    hilo.join().unwrap();
    let e = resultado.unwrap_err();
    assert!(e.mensaje.contains("cancelada con"), "{e:?}");
    // Cortó antes de leerla entera: la sesión sigue sirviendo y el token se rearma al empezar.
    let filas: u64 = e
        .mensaje
        .split_whitespace()
        .filter_map(|w| w.parse().ok())
        .next()
        .unwrap();
    assert!(filas < 262_144, "{filas}");
    let mut p2 = peticion("ORE_X0.SALADA", &[("id", "ID")]).unwrap();
    p2.limit = Some(3);
    let mut salida = Vec::new();
    assert_eq!(Phoenix::leer(&mut s, &token, &p2, &mut salida).unwrap(), 3);
}

#[test]
fn check_testigo_y_cambio() {
    let Some(u) = url() else { return };
    let ok = verbos::check(&format!("{{\"url\":\"{u}\"}}")).unwrap();
    assert!(ok.contains("\"ok\":true"), "{ok}");
    // Con un esquema en la URL: lista sus tablas y lee una fila de una.
    let ok = verbos::check(&format!("{{\"url\":\"{u}/ORE_X0\"}}")).unwrap();
    assert!(ok.contains("\"ok\":true"), "{ok}");
    // Un esquema que no existe lo dice, con su causa.
    let mal = verbos::check(&format!("{{\"url\":\"{u}/NO_EXISTE\"}}")).unwrap();
    assert!(
        mal.contains("\"ok\":false") && mal.contains("NO_EXISTE"),
        "{mal}"
    );
    // Un servidor que no está.
    let mal = verbos::check("{\"url\":\"phoenix://127.0.0.1:1\"}").unwrap();
    assert!(mal.contains("\"ok\":false"), "{mal}");
    // El testigo no inventa una marca; el cambio no respalda, y una tabla que no está es un error.
    assert!(
        verbos::testigo(&format!("{{\"url\":\"{u}\"}}"))
            .unwrap()
            .contains("\"modo\":\"none\"")
    );
    let c = verbos::cambio(&format!(
        "{{\"url\":\"{u}\",\"objeto\":\"ORE_X0.PEDIDOS\"}}"
    ))
    .unwrap();
    assert!(c.contains("\"respalda\":false"), "{c}");
    assert!(
        verbos::cambio(&format!(
            "{{\"url\":\"{u}\",\"objeto\":\"ORE_X0.NO_EXISTE\"}}"
        ))
        .is_err()
    );
}
