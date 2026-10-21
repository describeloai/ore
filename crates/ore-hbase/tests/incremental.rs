//! **El incremental** (ficha 0072, N5) contra el cluster simulado: `start`/`end` sobre `_ts`, la marca de tiempo de la
//! fila, en dos pasos (las claves que cambiaron en el rango y, de esas, las filas completas).
//!
//! Cuarenta filas `k0000000n`; la celda `f:a` de la fila `n` tiene la marca `1000 + n`, y la celda `f:b` la misma, salvo
//! en las filas multiplos de cuatro, donde alguien la **actualizo** despues: `2000 + n`. Asi hay filas con una celda
//! dentro del rango y la otra fuera, que es lo que un `Scan` con rango de tiempo devolveria a medias.

#[path = "../../ore-hbase-rpc/tests/comun/simulado.rs"]
mod simulado;

use std::io::Cursor;
use std::sync::Arc;

use arrow_array::cast::AsArray;
use arrow_array::types::Int64Type;
use arrow_ipc::reader::StreamReader;
use ore_driver::origen::Origen;
use ore_driver::{Codigo, Fallo, Filtro, Peticion};
use ore_hbase::incremental::CLAVES_POR_TANDA;
use ore_hbase::lectura::{Hbase, Token};
use simulado::{Accion, Celdas, Llamada, Mundo, Sim, clave};

fn nada(_: &Llamada, _: &mut Mundo) -> Accion {
    Accion::Normal
}

fn celdas() -> Celdas {
    Arc::new(|k| {
        vec![
            (b"f".to_vec(), b"a".to_vec(), [b"a".as_slice(), k].concat()),
            (b"f".to_vec(), b"b".to_vec(), [b"b".as_slice(), k].concat()),
        ]
    })
}

fn numero(k: &[u8]) -> u64 {
    String::from_utf8_lossy(k).parse().unwrap()
}

/// `f:a` vale `1000 + n`; `f:b`, lo mismo salvo en los multiplos de cuatro (`2000 + n`).
fn marcas() -> simulado::Marcas {
    Arc::new(|k, _, q| {
        let n = numero(k);
        if q == b"b" && n.is_multiple_of(4) {
            2000 + n
        } else {
            1000 + n
        }
    })
}

fn sim_con(
    n: usize,
    regiones: usize,
    guion: impl FnMut(&Llamada, &mut Mundo) -> Accion + Send + 'static,
) -> Sim {
    let s = Sim::con_filas((0..n).map(clave).collect(), regiones, celdas(), guion);
    s.mundo.lock().unwrap().marca_de = marcas();
    s
}

fn sim() -> Sim {
    sim_con(40, 4, nada)
}

const TIPOS: &[(&str, &str, &str)] = &[
    ("_clave", "String", "utf8"),
    ("_ts", "Integer", "i64be"),
    ("f:a", "String", "utf8"),
    ("f:b", "String", "utf8"),
];

fn peticion(s: &Sim, cols: &[&str], start: Option<&str>, end: Option<&str>) -> Peticion {
    Peticion {
        url: format!(
            "hbase://{}?simple=acepto&traduccion=rs.sim:16020%3D{}&paralelo=1",
            s.zk, s.rpc
        ),
        objeto: "default.t".into(),
        proyeccion: cols
            .iter()
            .map(|c| (c.to_string(), c.to_string()))
            .collect(),
        tipos: TIPOS
            .iter()
            .map(|(c, t, _)| (c.to_string(), t.to_string()))
            .collect(),
        codificaciones: TIPOS
            .iter()
            .map(|(c, _, e)| (c.to_string(), e.to_string()))
            .collect(),
        start: start.map(String::from),
        end: end.map(String::from),
        cursor: (start.is_some() || end.is_some()).then(|| "_ts".to_string()),
        ..Peticion::default()
    }
}

fn leer(p: &Peticion) -> (Result<u64, Fallo>, Vec<u8>) {
    let mut s = Hbase::conectar(&p.url).unwrap();
    let mut salida = Vec::new();
    let r = Hbase::leer(&mut s, &Token::default(), p, &mut salida);
    (r, salida)
}

/// Una fila entera: `(n, _ts, f:a, f:b)`.
type Fila = (u64, i64, Option<String>, Option<String>);

const TODAS: &[&str] = &["_clave", "_ts", "f:a", "f:b"];

fn filas(p: &Peticion) -> Vec<Fila> {
    let (r, bytes) = leer(p);
    r.unwrap();
    let mut v = Vec::new();
    for lote in StreamReader::try_new(Cursor::new(bytes), None).unwrap() {
        let l = lote.unwrap();
        let texto = |c: usize, i: usize| {
            (!l.column(c).is_null(i)).then(|| l.column(c).as_string::<i32>().value(i).to_string())
        };
        for i in 0..l.num_rows() {
            v.push((
                numero(texto(0, i).unwrap().as_bytes()),
                l.column(1).as_primitive::<Int64Type>().value(i),
                texto(2, i),
                texto(3, i),
            ));
        }
    }
    v.sort();
    v
}

fn ns(v: &[Fila]) -> Vec<u64> {
    v.iter().map(|f| f.0).collect()
}

fn abiertos(s: &Sim) -> usize {
    s.registro()
        .iter()
        .filter(|l| l.starts_with("abre t,"))
        .count()
}

#[test]
fn el_rango_es_exclusivo_por_el_principio_e_inclusivo_por_el_final() {
    let s = sim();
    // f:a de la fila n vale 1000 + n: (1020, 1030] son las filas 21 a 30 (las multiplos de cuatro tambien tienen
    // f:b a 2000 + n, fuera del rango, y no cuentan para entrar).
    let r = filas(&peticion(&s, TODAS, Some("1020"), Some("1030")));
    assert_eq!(ns(&r), (21..=30).collect::<Vec<_>>());
    // los bordes: 1020 no entra (exclusivo), 1030 si
    assert!(!ns(&r).contains(&20) && ns(&r).contains(&30));
}

#[test]
fn las_filas_llegan_completas_aunque_solo_una_celda_cambiara_en_el_rango() {
    let s = sim();
    let r = filas(&peticion(&s, TODAS, Some("1020"), Some("1030")));
    for (n, _, a, b) in &r {
        let k = format!("{n:08}");
        assert_eq!(a.as_deref(), Some(format!("a{k}").as_str()), "f:a de {n}");
        // la 24 y la 28 tienen f:b a 2000 + n: fuera del rango, y aun asi llega
        assert_eq!(b.as_deref(), Some(format!("b{k}").as_str()), "f:b de {n}");
    }
}

#[test]
fn la_marca_de_la_fila_es_la_mayor_de_sus_celdas_y_no_la_del_rango() {
    let s = sim();
    let r = filas(&peticion(&s, TODAS, Some("1020"), Some("1030")));
    let ts = |n: u64| r.iter().find(|f| f.0 == n).unwrap().1;
    assert_eq!(ts(25), 1025, "las dos celdas a 1025");
    assert_eq!(
        ts(24),
        2024,
        "f:b se actualizo despues: esa es la marca de la fila"
    );
}

#[test]
fn una_celda_actualizada_hace_cambiar_a_su_fila() {
    let s = sim();
    // 1999 exclusivo: solo lo que tiene alguna celda a 2000 o mas, es decir las multiplos de cuatro
    let r = filas(&peticion(&s, TODAS, Some("1999"), None));
    assert_eq!(ns(&r), (0..40).step_by(4).collect::<Vec<_>>());
    // y llegan con la celda que NO cambio
    assert!(r.iter().all(|f| f.2.is_some() && f.3.is_some()));
}

#[test]
fn sin_start_es_desde_el_principio_y_sin_end_es_hasta_donde_haya() {
    let s = sim();
    let r = filas(&peticion(&s, TODAS, None, Some("1005")));
    assert_eq!(ns(&r), (0..=5).collect::<Vec<_>>());
    let r = filas(&peticion(&s, TODAS, Some("1035"), None));
    // 36..=39 por f:a, y los multiplos de cuatro por f:b
    let mut esperado: Vec<u64> = (36..40).collect();
    esperado.extend((0..40).step_by(4));
    esperado.sort();
    esperado.dedup();
    assert_eq!(ns(&r), esperado);
}

#[test]
fn un_rango_sin_cambios_no_da_filas_ni_falla() {
    let s = sim();
    assert!(filas(&peticion(&s, TODAS, Some("9000"), None)).is_empty());
    // start >= end: vacio sin preguntar a HBase (que rechaza un TimeRange al reves)
    let antes = abiertos(&s);
    assert!(filas(&peticion(&s, TODAS, Some("1030"), Some("1030"))).is_empty());
    assert_eq!(abiertos(&s), antes, "no se abrio ningun scanner");
}

#[test]
fn pedir_solo_la_clave_no_trae_las_filas() {
    let s = sim();
    let p = peticion(&s, &["_clave"], Some("1020"), Some("1030"));
    let (r, bytes) = leer(&p);
    assert_eq!(r.unwrap(), 10);
    let n: usize = StreamReader::try_new(Cursor::new(bytes), None)
        .unwrap()
        .map(|l| l.unwrap().num_rows())
        .sum();
    assert_eq!(n, 10);
    // un solo paso: un scanner por region que toca, ninguno del paso 2
    assert!(abiertos(&s) <= 4, "{}", abiertos(&s));
}

#[test]
fn con_paralelo_el_incremental_da_lo_mismo() {
    let s = sim();
    let mut p = peticion(&s, TODAS, Some("1020"), Some("1030"));
    p.url = p.url.replace("paralelo=1", "paralelo=4");
    assert_eq!(ns(&filas(&p)), (21..=30).collect::<Vec<_>>());
}

#[test]
fn la_marca_sin_rango_tambien_se_lee_y_es_la_de_la_fila() {
    let s = sim();
    let r = filas(&peticion(&s, TODAS, None, None));
    assert_eq!(r.len(), 40);
    assert_eq!(r.iter().find(|f| f.0 == 24).unwrap().1, 2024);
    assert_eq!(r.iter().find(|f| f.0 == 25).unwrap().1, 1025);
}

#[test]
fn una_fila_borrada_entre_los_dos_pasos_no_sale() {
    // un cliente borra la fila 25 despues de que el paso 1 la viera y antes de que el paso 2 la traiga
    let s = sim_con(40, 1, |l: &Llamada, m: &mut Mundo| {
        if l.abre && l.aperturas == 2 {
            m.filas.retain(|k| numero(k) != 25);
        }
        Accion::Normal
    });
    let r = filas(&peticion(&s, TODAS, Some("1020"), Some("1030")));
    let mut esperado: Vec<u64> = (21..=30).collect();
    esperado.retain(|n| *n != 25);
    assert_eq!(ns(&r), esperado);
}

#[test]
fn mas_claves_que_una_tanda_se_traen_por_tandas_y_en_orden() {
    let n = CLAVES_POR_TANDA * 2 + 300;
    let s = Sim::con_filas((0..n).map(clave).collect(), 3, celdas(), nada);
    s.mundo.lock().unwrap().marca_de = Arc::new(|_, _, _| 5000);
    let r = filas(&peticion(&s, TODAS, Some("4999"), None));
    assert_eq!(r.len(), n);
    assert_eq!(ns(&r), (0..n as u64).collect::<Vec<_>>());
    assert!(
        r.iter()
            .all(|f| f.1 == 5000 && f.2.is_some() && f.3.is_some())
    );
    // tres scanners del paso 1 (uno por region) y, del paso 2, uno al menos por cada una de las tres tandas
    assert!(abiertos(&s) >= 3 + 3, "{}", abiertos(&s));
}

#[test]
fn un_rango_sobre_otra_columna_o_sin_cursor_se_niega_y_se_dice_por_que() {
    let s = sim();
    let mut p = peticion(&s, TODAS, Some("1"), None);
    p.cursor = Some("f:a".into());
    let e = leer(&p).0.unwrap_err();
    assert_eq!(e.codigo, Codigo::Operador);
    assert!(
        e.mensaje.contains("_ts") && e.mensaje.contains("f:a"),
        "{}",
        e.mensaje
    );
    p.cursor = None;
    assert_eq!(leer(&p).0.unwrap_err().codigo, Codigo::Operador);
    let mut q = peticion(&s, TODAS, Some("ayer"), None);
    assert_eq!(leer(&q).0.unwrap_err().codigo, Codigo::Operador);
    // la marca no se filtra: se avanza con start/end
    q = peticion(&s, TODAS, None, None);
    q.filtros = vec![Filtro::uno("_ts", "gt", "1000")];
    assert_eq!(leer(&q).0.unwrap_err().codigo, Codigo::Operador);
    // y no hubo flujo a medias
    assert!(leer(&q).1.is_empty());
}

#[test]
fn la_marca_declarada_como_otra_cosa_que_un_long_es_un_error_de_esquema() {
    let s = sim();
    let mut p = peticion(&s, TODAS, Some("1020"), None);
    p.tipos.retain(|(c, _)| c != "_ts");
    p.codificaciones.retain(|(c, _)| c != "_ts");
    p.tipos.push(("_ts".into(), "String".into()));
    p.codificaciones.push(("_ts".into(), "utf8".into()));
    let e = leer(&p).0.unwrap_err();
    assert_eq!(e.codigo, Codigo::Esquema);
    assert!(
        e.mensaje.contains("_ts") && e.mensaje.contains("epoch-ms"),
        "{}",
        e.mensaje
    );
}
