//! **La clave compuesta** (ficha 0072, N4) contra el cluster simulado: `usuario#dia`, donde el usuario es texto que
//! acaba en `#` y el dia es un `int` de Java (`i32be`), de −3 a 3. Cuatro usuarios, una region cada uno.
//!
//! Las partes se declaran como columnas `_clave#1`, `_clave#2`, con su tipo y, en el `physicalType`, su codificacion y
//! donde acaban (`utf8:fin=23`, `i32be`). Un filtro sobre ellas se empuja como un recorrido por prefijo **y ademas se
//! vuelve a evaluar** sobre los valores decodificados: lo empujado es un superconjunto, nunca decide el resultado.

#[path = "../../ore-hbase-rpc/tests/comun/simulado.rs"]
mod simulado;

use std::io::Cursor;
use std::sync::Arc;

use arrow_array::cast::AsArray;
use arrow_array::types::Int64Type;
use arrow_ipc::reader::StreamReader;
use ore_driver::origen::Origen;
use ore_driver::{Codigo, Fallo, Filtro, Peticion};
use ore_hbase::lectura::{Hbase, Token};
use simulado::{Accion, Celdas, Llamada, Mundo, Sim};

const USUARIOS: [&str; 4] = ["ana", "beto", "carla", "dani"];

fn clave(usuario: &str, dia: i32) -> Vec<u8> {
    [usuario.as_bytes(), b"#", &dia.to_be_bytes()].concat()
}

fn nada(_: &Llamada, _: &mut Mundo) -> Accion {
    Accion::Normal
}

fn celdas() -> Celdas {
    Arc::new(|k| {
        let i = k.iter().position(|b| *b == b'#').unwrap_or(0);
        vec![(
            b"f".to_vec(),
            b"v".to_vec(),
            format!("{}", String::from_utf8_lossy(&k[..i])).into_bytes(),
        )]
    })
}

fn sim() -> Sim {
    let mut claves: Vec<Vec<u8>> = USUARIOS
        .iter()
        .flat_map(|u| (-3..=3).map(move |d| clave(u, d)))
        .collect();
    claves.sort();
    Sim::con_filas(claves, 4, celdas(), nada)
}

const TIPOS: &[(&str, &str, &str)] = &[
    ("_clave#1", "String", "utf8:fin=23"),
    ("_clave#2", "Integer", "i32be"),
    ("f:v", "String", "utf8"),
];

fn peticion(s: &Sim, cols: &[&str], filtros: Vec<Filtro>) -> Peticion {
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
        filtros,
        tipos: TIPOS
            .iter()
            .map(|(c, t, _)| (c.to_string(), t.to_string()))
            .collect(),
        codificaciones: TIPOS
            .iter()
            .map(|(c, _, e)| (c.to_string(), e.to_string()))
            .collect(),
        ..Peticion::default()
    }
}

fn leer(p: &Peticion) -> (Result<u64, Fallo>, Vec<u8>) {
    let mut s = Hbase::conectar(&p.url).unwrap();
    let mut salida = Vec::new();
    let r = Hbase::leer(&mut s, &Token::default(), p, &mut salida);
    (r, salida)
}

/// Las filas como `(usuario, dia)`, ordenadas.
fn filas(p: &Peticion) -> Vec<(String, i64)> {
    let (r, bytes) = leer(p);
    r.unwrap();
    let mut v = Vec::new();
    for lote in StreamReader::try_new(Cursor::new(bytes), None).unwrap() {
        let l = lote.unwrap();
        for i in 0..l.num_rows() {
            v.push((
                l.column(0).as_string::<i32>().value(i).to_string(),
                l.column(1).as_primitive::<Int64Type>().value(i),
            ));
        }
    }
    v.sort();
    v
}

fn de(usuario: &str, dias: std::ops::RangeInclusive<i64>) -> Vec<(String, i64)> {
    dias.map(|d| (usuario.to_string(), d)).collect()
}

fn f(c: &str, op: &str, v: &str) -> Filtro {
    Filtro::uno(c, op, v)
}

#[test]
fn las_partes_de_la_clave_salen_como_columnas_con_su_tipo() {
    let s = sim();
    let r = filas(&peticion(&s, &["_clave#1", "_clave#2"], vec![]));
    assert_eq!(r.len(), 28);
    let mut esperado: Vec<(String, i64)> = USUARIOS.iter().flat_map(|u| de(u, -3..=3)).collect();
    esperado.sort();
    assert_eq!(r, esperado);
}

#[test]
fn un_eq_sobre_la_primera_parte_es_un_recorrido_por_prefijo_y_solo_toca_su_region() {
    let s = sim();
    let r = filas(&peticion(
        &s,
        &["_clave#1", "_clave#2"],
        vec![f("_clave#1", "eq", "beto")],
    ));
    assert_eq!(r, de("beto", -3..=3));
    // La region de beto y, antes, el hueco de bytes que cierra la de ana (`beto#` empieza ahi): un scanner vacio.
    // Lo que importa: no se tocan las de carla ni dani.
    let abiertas: Vec<_> = s
        .registro()
        .into_iter()
        .filter(|l| l.starts_with("abre"))
        .collect();
    assert_eq!(abiertas.len(), 2, "{abiertas:?}");
    assert!(
        abiertas
            .iter()
            .all(|l| l.contains("abre t,,") || l.contains("abre t,beto#")),
        "{abiertas:?}"
    );
}

#[test]
fn un_eq_en_la_primera_parte_y_un_rango_entero_en_la_segunda_cruza_el_cero() {
    let s = sim();
    // dias -1..=1: los no negativos 0 y 1, y el -1 (que ordena despues): dos tramos dentro de la region de ana
    let p = peticion(
        &s,
        &["_clave#1", "_clave#2"],
        vec![
            f("_clave#1", "eq", "ana"),
            f("_clave#2", "ge", "-1"),
            f("_clave#2", "lt", "2"),
        ],
    );
    assert_eq!(filas(&p), de("ana", -1..=1));
    let abiertas = s
        .registro()
        .into_iter()
        .filter(|l| l.starts_with("abre"))
        .collect::<Vec<_>>();
    assert!(
        abiertas.iter().all(|l| l.contains("abre t,,")),
        "solo la region de ana (la primera), nada mas: {abiertas:?}"
    );
}

#[test]
fn un_rango_sobre_una_parte_de_texto_no_se_empuja_pero_se_evalua_bien() {
    let s = sim();
    let r = filas(&peticion(
        &s,
        &["_clave#1", "_clave#2"],
        vec![f("_clave#1", "ge", "carla")],
    ));
    let mut esperado = de("carla", -3..=3);
    esperado.extend(de("dani", -3..=3));
    assert_eq!(r, esperado);
}

#[test]
fn un_filtro_solo_sobre_la_segunda_parte_lee_todo_y_filtra_por_valor() {
    let s = sim();
    let r = filas(&peticion(
        &s,
        &["_clave#1", "_clave#2"],
        vec![f("_clave#2", "gt", "0")],
    ));
    let mut esperado: Vec<(String, i64)> = USUARIOS.iter().flat_map(|u| de(u, 1..=3)).collect();
    esperado.sort();
    assert_eq!(r, esperado);
}

#[test]
fn un_eq_imposible_en_la_segunda_parte_no_lee_nada_y_no_falla() {
    let s = sim();
    let r = filas(&peticion(
        &s,
        &["_clave#1", "_clave#2"],
        vec![
            f("_clave#1", "eq", "ana"),
            f("_clave#2", "gt", "3000000000"),
        ],
    ));
    assert!(r.is_empty());
    // y un usuario que no existe tampoco
    assert!(
        filas(&peticion(
            &s,
            &["_clave#1"],
            vec![f("_clave#1", "eq", "zoe")]
        ))
        .is_empty()
    );
}

#[test]
fn las_partes_se_combinan_con_las_celdas() {
    let s = sim();
    let r = filas(&peticion(
        &s,
        &["_clave#1", "_clave#2", "f:v"],
        vec![
            f("_clave#1", "eq", "dani"),
            f("f:v", "eq", "dani"),
            f("_clave#2", "le", "-2"),
        ],
    ));
    assert_eq!(r, de("dani", -3..=-2));
}

#[test]
fn una_clave_que_no_tiene_la_forma_declarada_dice_cual_es_y_no_se_cuela() {
    // «sin-separador» no lleva `#`: la clave esta mal, no el lector
    let mut claves = vec![clave("ana", 1), b"sin-separador".to_vec()];
    claves.sort();
    let s = Sim::con_filas(claves, 1, celdas(), nada);
    let (r, _) = leer(&peticion(&s, &["_clave#1", "_clave#2"], vec![]));
    let e = r.unwrap_err();
    assert_eq!(e.codigo, Codigo::Esquema);
    assert!(
        e.mensaje.contains("sin-separador") && e.mensaje.contains("fin"),
        "{}",
        e.mensaje
    );
}

#[test]
fn una_declaracion_de_partes_incompleta_es_un_error_de_esquema_antes_de_abrir_el_flujo() {
    let s = sim();
    let mut p = peticion(&s, &["_clave#1"], vec![]);
    p.tipos.retain(|(c, _)| c != "_clave#2");
    p.codificaciones.retain(|(c, _)| c != "_clave#2");
    p.tipos.push(("_clave#3".into(), "String".into()));
    p.codificaciones.push(("_clave#3".into(), "utf8".into()));
    let (r, bytes) = leer(&p);
    let e = r.unwrap_err();
    assert_eq!(e.codigo, Codigo::Esquema);
    assert!(e.mensaje.contains("sin huecos"), "{}", e.mensaje);
    assert!(bytes.is_empty());
    // una parte que no esta declarada
    let mut q = peticion(&s, &["_clave#1"], vec![]);
    q.proyeccion.push(("x".into(), "_clave#9".into()));
    assert_eq!(leer(&q).0.unwrap_err().codigo, Codigo::Esquema);
}

#[test]
fn con_paralelo_la_clave_compuesta_da_lo_mismo() {
    let s = sim();
    let mut p = peticion(
        &s,
        &["_clave#1", "_clave#2"],
        vec![f("_clave#2", "lt", "0")],
    );
    p.url = p.url.replace("paralelo=1", "paralelo=4");
    let mut esperado: Vec<(String, i64)> = USUARIOS.iter().flat_map(|u| de(u, -3..=-1)).collect();
    esperado.sort();
    assert_eq!(filas(&p), esperado);
}

#[test]
fn el_incremental_funciona_con_una_clave_compuesta() {
    let s = sim();
    // las celdas del simulador tienen la marca 1000: `start` 999 las ve todas, `start` 1000 ninguna
    let mut p = peticion(
        &s,
        &["_clave#1", "_clave#2", "_ts"],
        vec![f("_clave#1", "eq", "beto")],
    );
    p.tipos.push(("_ts".into(), "Integer".into()));
    p.codificaciones.push(("_ts".into(), "i64be".into()));
    p.start = Some("999".into());
    p.cursor = Some("_ts".into());
    let (r, bytes) = leer(&p);
    assert_eq!(r.unwrap(), 7);
    let mut n = 0;
    for lote in StreamReader::try_new(Cursor::new(bytes), None).unwrap() {
        let l = lote.unwrap();
        for i in 0..l.num_rows() {
            assert_eq!(l.column(0).as_string::<i32>().value(i), "beto");
            assert_eq!(l.column(2).as_primitive::<Int64Type>().value(i), 1000);
            n += 1;
        }
    }
    assert_eq!(n, 7);
    p.start = Some("1000".into());
    assert_eq!(leer(&p).0.unwrap(), 0);
}
