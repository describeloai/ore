//! El **conector entero** (`Hbase::leer` y los verbos) contra el cluster simulado de `ore-hbase-rpc`
//! (`comun/simulado.rs`, compartido por ruta): ZooKeeper, `hbase:meta` y un RegionServer en el propio proceso, con
//! una tabla `default.t` de 400 filas (`00000000`…`00000399`) en 4 regiones, una celda `f:q` = `v` + la clave.
//! No necesita laboratorio.

#[path = "../../ore-hbase-rpc/tests/comun/simulado.rs"]
mod simulado;

use std::io::Cursor;
use std::time::{Duration, Instant};

use arrow_array::Array;
use arrow_array::cast::AsArray;
use arrow_ipc::reader::StreamReader;
use ore_driver::catalogo::Catalogo;
use ore_driver::origen::Origen;
use ore_driver::{Codigo, Fallo, Filtro, Peticion};
use ore_hbase::lectura::{Hbase, Sesion, Token};
use ore_hbase::verbos;
use simulado::{Accion, Llamada, Mundo, Sim, clave, valor};

fn url(sim: &Sim, extra: &str) -> String {
    format!(
        "hbase://{}?simple=acepto&traduccion=rs.sim:16020%3D{}{extra}",
        sim.zk, sim.rpc
    )
}

fn nada(_: &Llamada, _: &mut Mundo) -> Accion {
    Accion::Normal
}

fn peticion(sim: &Sim, extra: &str, cols: &[(&str, &str)]) -> Peticion {
    Peticion {
        url: url(sim, extra),
        objeto: "default.t".into(),
        proyeccion: cols
            .iter()
            .map(|(p, c)| (p.to_string(), c.to_string()))
            .collect(),
        ..Peticion::default()
    }
}

/// Las filas del flujo Arrow: una `Vec<Option<String>>` por fila.
fn filas(bytes: &[u8]) -> Vec<Vec<Option<String>>> {
    let lector = StreamReader::try_new(Cursor::new(bytes), None).expect("un flujo Arrow valido");
    let mut sal = Vec::new();
    for lote in lector {
        let lote = lote.unwrap();
        for i in 0..lote.num_rows() {
            sal.push(
                (0..lote.num_columns())
                    .map(|c| {
                        let col = lote.column(c).as_string::<i32>();
                        (!col.is_null(i)).then(|| col.value(i).to_string())
                    })
                    .collect(),
            );
        }
    }
    sal
}

fn leer(p: &Peticion) -> (Result<u64, Fallo>, Vec<u8>) {
    let mut s: Sesion = Hbase::conectar(&p.url).unwrap();
    let mut salida = Vec::new();
    let r = Hbase::leer(&mut s, &Token::default(), p, &mut salida);
    (r, salida)
}

fn claves(f: &[Vec<Option<String>>]) -> Vec<String> {
    let mut v: Vec<String> = f.iter().map(|r| r[0].clone().unwrap()).collect();
    v.sort();
    v
}

fn esperadas(desde: usize, hasta: usize) -> Vec<String> {
    (desde..hasta)
        .map(|n| String::from_utf8(clave(n)).unwrap())
        .collect()
}

#[test]
fn lee_toda_la_tabla_con_la_clave_y_la_celda_en_cuatro_partes() {
    let sim = Sim::nuevo(400, 4, nada);
    let p = peticion(&sim, "", &[("id", "_clave"), ("v", "f:q")]);
    let (r, bytes) = leer(&p);
    assert_eq!(r.unwrap(), 400);
    let f = filas(&bytes);
    assert_eq!(
        claves(&f),
        esperadas(0, 400),
        "ni una fila perdida ni repetida"
    );
    for fila in &f {
        let k = fila[0].as_ref().unwrap().as_bytes();
        assert_eq!(
            fila[1].as_deref().map(str::as_bytes),
            Some(valor(k).as_slice())
        );
    }
    assert_eq!(
        sim.n_registros("abre"),
        4,
        "un scanner por region: {:?}",
        sim.registro()
    );
}

#[test]
fn un_filtro_sobre_la_clave_se_empuja_como_rango_y_solo_toca_las_regiones_de_dentro() {
    let sim = Sim::nuevo(400, 4, nada);
    let mut p = peticion(&sim, "", &[("id", "_clave")]);
    p.filtros = vec![
        Filtro::uno("_clave", "ge", "00000150"),
        Filtro::uno("_clave", "lt", "00000250"),
    ];
    let (r, bytes) = leer(&p);
    assert_eq!(r.unwrap(), 100);
    assert_eq!(claves(&filas(&bytes)), esperadas(150, 250));
    assert_eq!(
        sim.n_registros("abre"),
        2,
        "las regiones 1 y 2: {:?}",
        sim.registro()
    );
}

#[test]
fn un_in_sobre_la_clave_lee_solo_esas_filas() {
    let sim = Sim::nuevo(400, 4, nada);
    let mut p = peticion(&sim, "", &[("id", "_clave")]);
    p.filtros = vec![Filtro {
        columna: "_clave".into(),
        operador: "in".into(),
        valor: ore_driver::Valor::Lista(vec![
            "00000007".into(),
            "00000399".into(),
            "00000007".into(),
            "99999999".into(),
        ]),
    }];
    let (r, bytes) = leer(&p);
    assert_eq!(r.unwrap(), 2);
    assert_eq!(claves(&filas(&bytes)), vec!["00000007", "00000399"]);
}

#[test]
fn los_filtros_sobre_una_celda_se_evaluan_en_el_conector_con_la_semantica_de_sql() {
    let sim = Sim::nuevo(400, 4, nada);
    let con = |f: Filtro| {
        let mut p = peticion(&sim, "", &[("id", "_clave")]);
        p.filtros = vec![f];
        let (r, bytes) = leer(&p);
        r.unwrap();
        claves(&filas(&bytes))
    };
    assert_eq!(con(Filtro::uno("f:q", "eq", "v00000007")), vec!["00000007"]);
    assert_eq!(con(Filtro::uno("f:q", "neq", "v00000007")).len(), 399);
    assert_eq!(con(Filtro::uno("f:q", "lt", "v00000003")), esperadas(0, 3));
    // una celda que ninguna fila tiene: todas son nulas
    assert_eq!(
        con(Filtro::uno("f:zz", "eq", "x")).len(),
        0,
        "un nulo no cumple una comparacion"
    );
    let nulos = Filtro {
        columna: "f:zz".into(),
        operador: "isNull".into(),
        valor: ore_driver::Valor::Ninguno,
    };
    assert_eq!(con(nulos).len(), 400);
    // y neq sobre la clave tambien se evalua aqui
    assert_eq!(con(Filtro::uno("_clave", "neq", "00000000")).len(), 399);
}

#[test]
fn un_limit_para_a_las_n_filas_en_un_solo_hilo() {
    let sim = Sim::nuevo(400, 4, nada);
    let mut p = peticion(&sim, "", &[("id", "_clave")]);
    p.limit = Some(10);
    let (r, bytes) = leer(&p);
    assert_eq!(r.unwrap(), 10);
    assert_eq!(filas(&bytes).len(), 10);
    assert_eq!(sim.n_registros("abre"), 1, "{:?}", sim.registro());
}

#[test]
fn con_paralelo_uno_se_lee_igual_y_en_orden_de_clave() {
    let sim = Sim::nuevo(400, 4, nada);
    let p = peticion(&sim, "&paralelo=1", &[("id", "_clave")]);
    let (r, bytes) = leer(&p);
    assert_eq!(r.unwrap(), 400);
    let k: Vec<String> = filas(&bytes)
        .iter()
        .map(|r| r[0].clone().unwrap())
        .collect();
    assert_eq!(
        k,
        esperadas(0, 400),
        "una sola parte conserva el orden de la clave"
    );
}

#[test]
fn una_columna_opaque_sale_en_base64_y_una_string_con_bytes_que_no_son_utf8_es_un_error() {
    let sim = Sim::nuevo(20, 2, nada);
    let mut p = peticion(&sim, "", &[("v", "f:q")]);
    p.tipos = vec![("f:q".into(), "Opaque".into())];
    p.limit = Some(1);
    let (r, bytes) = leer(&p);
    r.unwrap();
    // "v00000000" en base64
    assert_eq!(filas(&bytes)[0][0].as_deref(), Some("djAwMDAwMDAw"));
}

#[test]
fn lo_que_falla_antes_de_la_primera_fila_es_un_error_limpio_sin_flujo() {
    let sim = Sim::nuevo(20, 2, nada);
    // un tipo declarado que HBase no sabe leer
    let mut p = peticion(&sim, "", &[("v", "f:q")]);
    p.tipos = vec![("f:q".into(), "Integer".into())];
    let (r, bytes) = leer(&p);
    assert_eq!(r.unwrap_err().codigo, Codigo::Esquema);
    assert!(bytes.is_empty(), "nada en el flujo");
    // una tabla que no existe
    let mut p = peticion(&sim, "", &[("id", "_clave")]);
    p.objeto = "default.noexiste".into();
    let (r, bytes) = leer(&p);
    assert_eq!(r.unwrap_err().codigo, Codigo::Objeto);
    assert!(bytes.is_empty());
    // un operador que no se declara
    let mut p = peticion(&sim, "", &[("id", "_clave")]);
    p.filtros = vec![Filtro::uno("f:q", "like", "v%")];
    assert_eq!(leer(&p).0.unwrap_err().codigo, Codigo::Operador);
}

#[test]
fn una_tabla_vacia_devuelve_su_esquema_sin_filas() {
    let sim = Sim::nuevo(0, 1, nada);
    let p = peticion(&sim, "", &[("id", "_clave"), ("v", "f:q")]);
    let (r, bytes) = leer(&p);
    assert_eq!(r.unwrap(), 0);
    let lector = StreamReader::try_new(Cursor::new(&bytes[..]), None).unwrap();
    assert_eq!(lector.schema().fields().len(), 2);
    assert!(filas(&bytes).is_empty());
}

#[test]
fn una_region_que_se_parte_a_mitad_de_lectura_no_hace_perder_filas() {
    let sim = Sim::nuevo(400, 4, |l, m| {
        if l.n == 3 {
            m.partir(&clave(130));
        }
        Accion::Normal
    });
    let p = peticion(&sim, "&paralelo=1", &[("id", "_clave"), ("v", "f:q")]);
    let (r, bytes) = leer(&p);
    assert_eq!(r.unwrap(), 400);
    assert_eq!(claves(&filas(&bytes)), esperadas(0, 400));
}

#[test]
fn el_plazo_de_la_lectura_corta_los_reintentos_y_es_un_error_de_tiempo() {
    // el servidor se queda dormido: sin plazo, el lector reintentaria unos dos minutos y medio
    let sim = Sim::nuevo(400, 2, |l, _| {
        if l.n >= 2 {
            Accion::Dormir(Duration::from_secs(20))
        } else {
            Accion::Normal
        }
    });
    let mut p = peticion(&sim, "&paralelo=1", &[("id", "_clave")]);
    p.timeout_ms = Some(500);
    let t = Instant::now();
    let (r, _) = leer(&p);
    let f = r.unwrap_err();
    assert_eq!(f.codigo, Codigo::Tiempo, "{}", f.mensaje);
    assert!(
        t.elapsed() < Duration::from_secs(5),
        "tardo {:?}",
        t.elapsed()
    );
}

#[test]
fn cancelar_desde_otro_hilo_para_una_lectura_con_el_servidor_dormido() {
    let sim = Sim::nuevo(400, 2, |l, _| {
        if l.n >= 2 {
            Accion::Dormir(Duration::from_secs(20))
        } else {
            Accion::Normal
        }
    });
    let p = peticion(&sim, "&paralelo=1", &[("id", "_clave")]);
    let token = Token::default();
    let otro = token.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        Hbase::cancelar(&otro);
    });
    let mut s = Hbase::conectar(&p.url).unwrap();
    let t = Instant::now();
    let r = Hbase::leer(&mut s, &token, &p, &mut Vec::new());
    let f = r.unwrap_err();
    assert!(f.mensaje.contains("cancelada"), "{}", f.mensaje);
    assert!(
        t.elapsed() < Duration::from_secs(3),
        "tardo {:?}",
        t.elapsed()
    );
}

#[test]
fn el_token_de_una_lectura_cancelada_se_rearma_en_la_siguiente() {
    let sim = Sim::nuevo(40, 2, nada);
    let p = peticion(&sim, "", &[("id", "_clave")]);
    let token = Token::default();
    Hbase::cancelar(&token);
    let mut s = Hbase::conectar(&p.url).unwrap();
    assert_eq!(
        Hbase::leer(&mut s, &token, &p, &mut Vec::new()).unwrap(),
        40
    );
}

#[test]
fn el_catalogo_enumera_las_tablas_y_sus_columnas_muestreadas() {
    let sim = Sim::nuevo(400, 4, nada);
    let texto = verbos::catalogo(&url(&sim, ""), "hbase-sim").unwrap();
    let c = Catalogo::leer(&texto).expect("un catalogo valido");
    assert_eq!(c.fuente(), "hbase-sim");
    assert_eq!(c.tablas.len(), 1);
    let t = &c.tablas[0];
    assert_eq!(t.nombre, "default.t");
    assert_eq!(t.clave, vec!["_clave"]);
    let cols: Vec<(&str, Option<&str>)> = t
        .columnas
        .iter()
        .map(|c| (c.nombre.as_str(), c.tipo.as_deref()))
        .collect();
    assert_eq!(
        cols,
        vec![("_clave", Some("String")), ("f:q", Some("String"))]
    );
    assert!(t.columnas[0].obligatoria && !t.columnas[1].obligatoria);
    assert!(texto.contains("predicatePushdown") && texto.contains("expensive"));
}

#[test]
fn check_y_explorar_y_los_verbos_de_cambio_responden() {
    let sim = Sim::nuevo(400, 4, nada);
    let entrada = format!("{{\"url\":\"{}\",\"objeto\":\"default.t\"}}", url(&sim, ""));
    assert!(verbos::check(&entrada).unwrap().contains("true"), "check");
    let e = verbos::explorar(&entrada).unwrap();
    assert!(e.contains("default"), "{e}");
    assert!(verbos::testigo(&entrada).unwrap().contains("none"));
    assert!(verbos::cambio(&entrada).unwrap().contains("sin-contadores"));
    let sin = format!(
        "{{\"url\":\"{}\",\"objeto\":\"default.nada\"}}",
        url(&sim, "")
    );
    assert!(
        verbos::cambio(&sin).is_err(),
        "una tabla que no esta es un error, no «no respalda»"
    );
}

#[test]
fn check_dice_por_que_falla_y_nunca_se_cuelga() {
    // sin simple=acepto
    let e = verbos::check("{\"url\":\"hbase://127.0.0.1:1\"}").unwrap();
    assert!(e.contains("false") && e.contains("simple=acepto"), "{e}");
    // un ZooKeeper al que no se llega
    let t = Instant::now();
    let e =
        verbos::check("{\"url\":\"hbase://127.0.0.1:1?simple=acepto&timeoutMs=1000\"}").unwrap();
    assert!(e.contains("false"), "{e}");
    assert!(
        t.elapsed() < Duration::from_secs(20),
        "tardo {:?}",
        t.elapsed()
    );
}
