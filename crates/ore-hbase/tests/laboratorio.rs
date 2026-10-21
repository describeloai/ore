//! El conector contra un **HBase de verdad** (el laboratorio de `pruebas-de-fuego/hbase/`, con `TestTable`: 10^6
//! filas de 100 bytes en 4 regiones). Solo corre con `HBASE_LAB_ZK=127.0.0.1:2181` (y, si el laboratorio esta en
//! otros puertos, `HBASE_LAB_TRADUCCION=127.0.0.1:16020=127.0.0.1:26020`); sin ella, no hace nada. La tabla `mixta`
//! (celdas de texto, binarias y dispersas) la crea la propia prueba con `docker exec` si hay `HBASE_LAB_CAOS=hb`.

use std::io::Cursor;
use std::process::{Command, Stdio};
use std::time::Instant;

use arrow_array::Array;
use arrow_array::cast::AsArray;
use arrow_ipc::reader::StreamReader;
use ore_core::tipos::{Fisico, Valor};
use ore_driver::catalogo::Catalogo;
use ore_driver::origen::Origen;
use ore_driver::{Filtro, Peticion};
use ore_hbase::codificacion::{Codificacion, codificar};
use ore_hbase::lectura::{Hbase, Token};
use ore_hbase::verbos;

fn url(paralelo: usize) -> Option<String> {
    let zk = std::env::var("HBASE_LAB_ZK")
        .ok()
        .filter(|u| !u.is_empty())?;
    let mut u = format!("hbase://{zk}?simple=acepto&paralelo={paralelo}");
    if let Some(t) = std::env::var("HBASE_LAB_TRADUCCION")
        .ok()
        .filter(|t| !t.is_empty())
    {
        u.push_str(&format!("&traduccion={}", t.replace('=', "%3D")));
    }
    Some(u)
}

fn peticion(url: String, objeto: &str, cols: &[(&str, &str)]) -> Peticion {
    Peticion {
        url,
        objeto: objeto.into(),
        proyeccion: cols
            .iter()
            .map(|(p, c)| (p.to_string(), c.to_string()))
            .collect(),
        ..Peticion::default()
    }
}

fn leer(p: &Peticion) -> (u64, Vec<u8>) {
    let mut s = Hbase::conectar(&p.url).unwrap();
    let mut salida = Vec::new();
    let n = Hbase::leer(&mut s, &Token::default(), p, &mut salida).expect("la lectura");
    (n, salida)
}

fn filas(bytes: &[u8]) -> Vec<Vec<Option<String>>> {
    let mut sal = Vec::new();
    for lote in StreamReader::try_new(Cursor::new(bytes), None).unwrap() {
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

fn clave(n: u64) -> String {
    format!("{n:026}")
}

#[test]
fn el_catalogo_de_testtable_dice_su_clave_y_su_celda() {
    let Some(u) = url(4) else { return };
    let texto = verbos::catalogo(&u, "lab").unwrap();
    let c = Catalogo::leer(&texto).unwrap();
    let t = c
        .tablas
        .iter()
        .find(|t| t.nombre == "default.TestTable")
        .expect("TestTable");
    let cols: Vec<_> = t
        .columnas
        .iter()
        .map(|c| (c.nombre.as_str(), c.tipo.as_deref().unwrap()))
        .collect();
    assert_eq!(
        cols,
        vec![("_clave", "String"), ("info0:0", "String")],
        "{texto}"
    );
    assert!(
        c.tablas.iter().all(|t| !t.nombre.starts_with("hbase.")),
        "las tablas del sistema no se catalogan"
    );
}

#[test]
fn un_barrido_de_un_millon_de_filas_en_paralelo_llega_entero() {
    let Some(u) = url(4) else { return };
    let t = Instant::now();
    let (n, bytes) = leer(&peticion(
        u,
        "default.TestTable",
        &[("id", "_clave"), ("v", "info0:0")],
    ));
    let dt = t.elapsed();
    eprintln!(
        "1 000 000 filas con paralelo=4 en {dt:?} ({} MB de Arrow)",
        bytes.len() / 1_000_000
    );
    assert_eq!(n, 1_000_000);
    let f = filas(&bytes);
    assert_eq!(f.len(), 1_000_000);
    let mut ids: Vec<&str> = f.iter().map(|r| r[0].as_deref().unwrap()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 1_000_000, "ni una repetida");
    assert_eq!(
        (ids[0], ids[999_999]),
        (clave(0).as_str(), clave(999_999).as_str())
    );
    assert!(
        f.iter()
            .all(|r| r[1].as_ref().is_some_and(|v| v.len() == 100)),
        "100 bytes por celda"
    );
}

#[test]
fn el_paralelismo_se_mide_de_uno_a_ocho() {
    for paralelo in [1, 2, 4, 8] {
        let Some(u) = url(paralelo) else { return };
        let t = Instant::now();
        let (n, _) = leer(&peticion(
            u,
            "default.TestTable",
            &[("id", "_clave"), ("v", "info0:0")],
        ));
        eprintln!("paralelo={paralelo}: {n} filas en {:?}", t.elapsed());
        assert_eq!(n, 1_000_000);
    }
}

#[test]
fn solo_la_clave_no_trae_los_valores() {
    let Some(u) = url(4) else { return };
    let t = Instant::now();
    let (n, _) = leer(&peticion(u, "default.TestTable", &[("id", "_clave")]));
    eprintln!("solo la clave, 10^6 filas: {:?}", t.elapsed());
    assert_eq!(n, 1_000_000);
}

#[test]
fn un_rango_de_la_clave_trae_exactamente_esas_filas() {
    let Some(u) = url(4) else { return };
    let mut p = peticion(u, "default.TestTable", &[("id", "_clave")]);
    p.filtros = vec![
        Filtro::uno("_clave", "ge", &clave(245_000)),
        Filtro::uno("_clave", "lt", &clave(255_000)),
    ];
    let (n, bytes) = leer(&p);
    assert_eq!(n, 10_000, "cruza el limite entre dos regiones");
    let mut ids: Vec<String> = filas(&bytes)
        .into_iter()
        .map(|r| r[0].clone().unwrap())
        .collect();
    ids.sort();
    assert_eq!(
        (ids[0].as_str(), ids[9_999].as_str()),
        (clave(245_000).as_str(), clave(254_999).as_str())
    );
}

#[test]
fn un_filtro_sobre_el_valor_se_evalua_aqui() {
    let Some(u) = url(4) else { return };
    // el valor de una fila que se lee primero, y cuantas filas lo tienen igual (al menos esa)
    let mut p = peticion(
        u.clone(),
        "default.TestTable",
        &[("id", "_clave"), ("v", "info0:0")],
    );
    p.limit = Some(1);
    let (_, bytes) = leer(&p);
    let fila = filas(&bytes).remove(0);
    let (id, valor) = (fila[0].clone().unwrap(), fila[1].clone().unwrap());
    let mut q = peticion(u, "default.TestTable", &[("id", "_clave")]);
    q.filtros = vec![Filtro::uno("info0:0", "eq", &valor)];
    let (n, bytes) = leer(&q);
    assert!(n >= 1);
    assert!(
        filas(&bytes)
            .iter()
            .any(|r| r[0].as_deref() == Some(id.as_str())),
        "la fila de partida esta"
    );
}

#[test]
fn un_limit_corta_la_lectura() {
    let Some(u) = url(4) else { return };
    let mut p = peticion(u, "default.TestTable", &[("id", "_clave")]);
    p.limit = Some(25);
    let t = Instant::now();
    let (n, _) = leer(&p);
    assert_eq!(n, 25);
    assert!(
        t.elapsed().as_secs() < 3,
        "no leyo la tabla entera: {:?}",
        t.elapsed()
    );
}

#[test]
fn una_tabla_que_no_existe_y_un_tipo_que_no_se_sabe_leer_son_errores_con_su_codigo() {
    let Some(u) = url(4) else { return };
    let mut s = Hbase::conectar(&u).unwrap();
    let mut p = peticion(u.clone(), "default.NoExiste", &[("id", "_clave")]);
    let e = Hbase::leer(&mut s, &Token::default(), &p, &mut Vec::new()).unwrap_err();
    assert_eq!(e.codigo, ore_driver::Codigo::Objeto, "{}", e.mensaje);
    p.objeto = "default.TestTable".into();
    p.proyeccion = vec![("v".into(), "info0:0".into())];
    p.tipos = vec![("info0:0".into(), "Integer".into())];
    let e = Hbase::leer(&mut s, &Token::default(), &p, &mut Vec::new()).unwrap_err();
    assert_eq!(e.codigo, ore_driver::Codigo::Esquema, "{}", e.mensaje);
}

// ── La tabla `mixta`: texto, bytes y celdas que faltan ──────────────────────────────────────────────────────────

fn shell(contenedor: &str, ordenes: &str) {
    let mut p = Command::new("docker")
        .args([
            "exec",
            "-i",
            contenedor,
            "bash",
            "-c",
            "$HBASE_HOME/bin/hbase shell -n",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("docker");
    use std::io::Write;
    p.stdin
        .take()
        .unwrap()
        .write_all(format!("{ordenes}\nexit\n").as_bytes())
        .unwrap();
    p.wait().unwrap();
}

fn con_mixta() -> Option<String> {
    let u = url(2)?;
    let contenedor = std::env::var("HBASE_LAB_CAOS")
        .ok()
        .filter(|c| !c.is_empty())?;
    // `hbase shell -n` aborta en el primer fallo: borrar una tabla que quiza no esta va aparte
    shell(
        &contenedor,
        "disable 'mixta'
drop 'mixta'",
    );
    // cadena cruda: los escapes `\x..` los interpreta Ruby, no Rust
    shell(
        &contenedor,
        r#"create 'mixta', 'f', 'g'
put 'mixta', 'r1', 'f:a', 'hola'
put 'mixta', 'r1', 'f:n', "\x00\x00\x00\x2a"
put 'mixta', 'r1', 'g:z', 'solo r1'
put 'mixta', 'r2', 'f:a', 'adios'
put 'mixta', 'r3', 'f:a', ''"#,
    );
    // Bytes que no son UTF-8 de verdad (0xFF 0xFE): el shell de HBase convertiria "ÿ" en el caracter U+00FF.
    // Por la pasarela REST del laboratorio, que escribe lo que se le da.
    let r = Command::new("docker")
        .args([
            "exec",
            &contenedor,
            "curl",
            "-s",
            "-o",
            "/dev/null",
            "-w",
            "%{http_code}",
            "-X",
            "PUT",
            "-H",
            "Content-Type: application/json",
            "-d",
            r#"{"Row":[{"key":"cjI=","Cell":[{"column":"Zjpu","$":"//4="}]}]}"#,
            "localhost:8080/mixta/r2",
        ])
        .output()
        .expect("docker");
    assert_eq!(
        String::from_utf8_lossy(&r.stdout),
        "200",
        "la escritura por REST"
    );
    Some(u)
}

#[test]
fn el_catalogo_distingue_texto_de_bytes_y_la_lectura_pone_nulos_donde_faltan_celdas() {
    let Some(u) = con_mixta() else { return };
    let c = Catalogo::leer(&verbos::catalogo(&u, "lab").unwrap()).unwrap();
    let t = c
        .tablas
        .iter()
        .find(|t| t.nombre == "default.mixta")
        .expect("mixta");
    let cols: Vec<_> = t
        .columnas
        .iter()
        .map(|c| (c.nombre.as_str(), c.tipo.as_deref().unwrap()))
        .collect();
    // f:n tiene bytes que no son texto en las dos filas que lo tienen: Opaque. Lo demas es texto.
    assert_eq!(
        cols,
        vec![
            ("_clave", "String"),
            ("f:a", "String"),
            ("f:n", "Opaque"),
            ("g:z", "String")
        ],
        "{cols:?}"
    );

    let mut p = peticion(
        u,
        "default.mixta",
        &[("id", "_clave"), ("a", "f:a"), ("n", "f:n"), ("z", "g:z")],
    );
    p.tipos = vec![("f:n".into(), "Opaque".into())];
    let (n, bytes) = leer(&p);
    assert_eq!(n, 3);
    let mut f = filas(&bytes);
    f.sort();
    let s = |x: &str| Some(x.to_string());
    assert_eq!(
        f[0],
        vec![s("r1"), s("hola"), s("AAAAKg=="), s("solo r1")],
        "los bytes de un entero de Java, en base64"
    );
    assert_eq!(
        f[1],
        vec![s("r2"), s("adios"), s("//4="), None],
        "una celda que falta es nula"
    );
    assert_eq!(
        f[2],
        vec![s("r3"), s(""), None, None],
        "la cadena vacia no es un nulo: HBase distingue una celda vacia de una ausente"
    );
}

#[test]
fn leer_como_string_unos_bytes_que_no_son_utf8_es_un_error_que_nombra_la_columna() {
    let Some(u) = con_mixta() else { return };
    let p = peticion(u, "default.mixta", &[("id", "_clave"), ("n", "f:n")]);
    let mut s = Hbase::conectar(&p.url).unwrap();
    let e = Hbase::leer(&mut s, &Token::default(), &p, &mut Vec::new()).unwrap_err();
    assert!(
        e.mensaje.contains("n") && e.mensaje.contains("Opaque"),
        "{}",
        e.mensaje
    );
}

// ── La tabla `tipada`: los bytes los escribe Java ───────────────────────────────────────────────────────────────

/// `pruebas-de-fuego/hbase/tipada.rb`: claves `int` de Java, y celdas `int`, `short`, `long`, `float`, `double`,
/// `boolean`, `BigDecimal`, milisegundos y texto, escritas con `org.apache.hadoop.hbase.util.Bytes.toBytes`.
const TIPADA_RB: &str = include_str!("../../../pruebas-de-fuego/hbase/tipada.rb");

/// Las filas de `tipada`.
const NS: [i32; 8] = [-2_147_483_648, -100, -5, 0, 7, 100, 100_000, 2_147_483_647];

fn con_tipada() -> Option<String> {
    let u = url(2)?;
    let contenedor = std::env::var("HBASE_LAB_CAOS")
        .ok()
        .filter(|c| !c.is_empty())?;
    // `hbase shell -n` aborta en el primer fallo: cada tabla, en su invocacion
    for t in ["tipada", "compuesta"] {
        shell(&contenedor, &format!("disable '{t}'\ndrop '{t}'"));
    }
    shell(&contenedor, TIPADA_RB);
    Some(u)
}

const TIPOS_TIPADA: &[(&str, &str, &str)] = &[
    ("_clave", "Integer", "i32be"),
    ("f:edad", "Integer", "i32be"),
    ("f:corto", "Integer", "i16be"),
    ("f:largo", "Integer", "i64be"),
    ("f:flotante", "Float", "f32be"),
    ("f:doble", "Float", "f64be"),
    ("f:ok", "Boolean", "bool"),
    ("f:total", "Decimal<12, 2>", "decimal"),
    ("f:ms", "DateTimeTz", "epoch-ms"),
    ("f:nombre", "String", "utf8"),
    ("f:texto", "Integer", "texto"),
];

fn pet_tipada(u: &str, cols: &[&str], filtros: Vec<Filtro>) -> Peticion {
    let mut p = peticion(
        u.to_string(),
        "default.tipada",
        &cols.iter().map(|c| (*c, *c)).collect::<Vec<_>>(),
    );
    p.filtros = filtros;
    p.tipos = TIPOS_TIPADA
        .iter()
        .map(|(c, t, _)| (c.to_string(), t.to_string()))
        .collect();
    p.codificaciones = TIPOS_TIPADA
        .iter()
        .map(|(c, _, e)| (c.to_string(), e.to_string()))
        .collect();
    p
}

fn claves_de(u: &str, filtros: Vec<Filtro>) -> Vec<i64> {
    let (_, bytes) = leer(&pet_tipada(u, &["_clave"], filtros));
    let mut v = Vec::new();
    for lote in StreamReader::try_new(Cursor::new(bytes), None).unwrap() {
        let l = lote.unwrap();
        let c = l.column(0).as_primitive::<arrow_array::types::Int64Type>();
        v.extend((0..l.num_rows()).map(|i| c.value(i)));
    }
    v.sort_unstable();
    v
}

#[test]
fn lo_que_java_escribio_con_bytes_to_bytes_se_lee_con_su_valor() {
    use arrow_array::types::{Decimal128Type, Float64Type, Int64Type, TimestampMicrosecondType};
    let Some(u) = con_tipada() else { return };
    let cols = [
        "_clave",
        "f:edad",
        "f:corto",
        "f:largo",
        "f:flotante",
        "f:doble",
        "f:ok",
        "f:total",
        "f:ms",
        "f:nombre",
        "f:texto",
    ];
    let (n, bytes) = leer(&pet_tipada(&u, &cols, vec![]));
    assert_eq!(n, 8);
    let mut vistas = 0;
    for lote in StreamReader::try_new(Cursor::new(bytes), None).unwrap() {
        let l = lote.unwrap();
        for i in 0..l.num_rows() {
            let n = l.column(0).as_primitive::<Int64Type>().value(i);
            let n32 = n as i32;
            assert!(NS.contains(&n32), "{n}");
            let ent = |c: usize| l.column(c).as_primitive::<Int64Type>().value(i);
            assert_eq!(ent(1), i64::from(n32 % 1_000_000), "edad de {n}");
            assert_eq!(ent(2), i64::from(n32 % 30000), "corto de {n}");
            assert_eq!(ent(3), i64::from(n32) * 1_000_000_007, "largo de {n}");
            // float de 32 bits: el mismo redondeo que Java
            assert_eq!(
                l.column(4).as_primitive::<Float64Type>().value(i),
                f64::from((f64::from(n32) * 0.5) as f32),
                "flotante de {n}"
            );
            assert_eq!(
                l.column(5).as_primitive::<Float64Type>().value(i),
                f64::from(n32) / 3.0,
                "doble de {n}"
            );
            assert_eq!(l.column(6).as_boolean().value(i), n32 % 2 == 0, "ok de {n}");
            assert_eq!(
                l.column(7).as_primitive::<Decimal128Type>().value(i),
                i128::from(n32) * 100 + 5,
                "total de {n}"
            );
            assert_eq!(
                l.column(8)
                    .as_primitive::<TimestampMicrosecondType>()
                    .value(i),
                (1_700_000_000_000 + i64::from(n32) * 1000) * 1000,
                "ms de {n}"
            );
            assert_eq!(l.column(9).as_string::<i32>().value(i), format!("n={n32}"));
            assert_eq!(ent(10), n, "texto de {n}: un Integer escrito como texto");
            vistas += 1;
        }
    }
    assert_eq!(vistas, 8);
}

#[test]
fn los_rangos_sobre_la_clave_int_de_java_dan_las_filas_aunque_los_negativos_ordenen_despues() {
    let Some(u) = con_tipada() else { return };
    let f = |op: &str, v: &str| Filtro::uno("_clave", op, v);
    assert_eq!(claves_de(&u, vec![]).len(), 8);
    assert_eq!(
        claves_de(&u, vec![f("lt", "5")]),
        vec![-2_147_483_648, -100, -5, 0]
    );
    assert_eq!(
        claves_de(&u, vec![f("ge", "7")]),
        vec![7, 100, 100_000, 2_147_483_647]
    );
    assert_eq!(
        claves_de(&u, vec![f("ge", "-5"), f("le", "7")]),
        vec![-5, 0, 7]
    );
    assert_eq!(
        claves_de(&u, vec![f("eq", "-2147483648")]),
        vec![-2_147_483_648]
    );
    assert_eq!(
        claves_de(&u, vec![f("gt", "2147483646")]),
        vec![2_147_483_647]
    );
    assert_eq!(
        claves_de(&u, vec![f("lt", "-2147483648")]),
        Vec::<i64>::new()
    );
    let en = Filtro {
        columna: "_clave".into(),
        operador: "in".into(),
        valor: ore_driver::Valor::Lista(vec!["-100".into(), "100".into(), "5".into()]),
    };
    assert_eq!(claves_de(&u, vec![en]), vec![-100, 100]);
}

#[test]
fn los_filtros_sobre_celdas_de_java_se_comparan_como_numeros() {
    let Some(u) = con_tipada() else { return };
    let con = |col: &str, op: &str, v: &str| claves_de(&u, vec![Filtro::uno(col, op, v)]);
    // largo = n * 1_000_000_007: el orden numerico es el de n
    assert_eq!(
        con("f:largo", "gt", "0"),
        vec![7, 100, 100_000, 2_147_483_647]
    );
    // total = n*100 + 5
    assert_eq!(con("f:total", "lt", "0.00"), vec![-2_147_483_648, -100, -5]);
    assert_eq!(
        con("f:ok", "eq", "true"),
        vec![-2_147_483_648, -100, 0, 100, 100_000]
    );
    assert_eq!(con("f:texto", "eq", "7"), vec![7]);
}

#[test]
fn la_tabla_tipada_tiene_tres_regiones_partidas_por_los_bytes_de_un_int() {
    let Some(u) = con_tipada() else { return };
    let d = ore_hbase::url::analizar(&u).unwrap();
    let c = ore_hbase::transporte::Conexion::de(d, None).unwrap();
    let r = c.cliente.regiones("tipada").unwrap();
    assert_eq!(r.len(), 3);
    assert_eq!(r[1].inicio, vec![0, 0, 0, 0]);
    assert_eq!(r[2].inicio, vec![0x80, 0, 0, 0]);
    let _ = (
        Codificacion::I32Be,
        codificar,
        Fisico::Entero,
        Valor::Entero(0),
    );
}

#[test]
fn una_clave_compuesta_que_escribio_java_se_trocea_y_se_empuja_por_prefijo() {
    let Some(u) = con_tipada() else { return };
    let mut p = peticion(
        u,
        "default.compuesta",
        &[("u", "_clave#1"), ("d", "_clave#2"), ("v", "f:v")],
    );
    p.tipos = vec![
        ("_clave#1".into(), "String".into()),
        ("_clave#2".into(), "Integer".into()),
    ];
    p.codificaciones = vec![
        ("_clave#1".into(), "utf8:fin=23".into()),
        ("_clave#2".into(), "i32be".into()),
    ];
    let todas = |p: &Peticion| {
        let (_, bytes) = leer(p);
        let mut v = Vec::new();
        for lote in StreamReader::try_new(Cursor::new(bytes), None).unwrap() {
            let l = lote.unwrap();
            for i in 0..l.num_rows() {
                v.push((
                    l.column(0).as_string::<i32>().value(i).to_string(),
                    l.column(1)
                        .as_primitive::<arrow_array::types::Int64Type>()
                        .value(i),
                    l.column(2).as_string::<i32>().value(i).to_string(),
                ));
            }
        }
        v.sort();
        v
    };
    let todo = todas(&p);
    assert_eq!(todo.len(), 16);
    assert!(
        todo.iter().all(|(u, d, v)| *v == format!("{u}:{d}")),
        "{todo:?}"
    );
    // un usuario, y un rango de dias que cruza el cero: -1 (ordena despues) y 0, y no el 2 ni el -3
    p.filtros = vec![
        Filtro::uno("_clave#1", "eq", "carla"),
        Filtro::uno("_clave#2", "ge", "-1"),
        Filtro::uno("_clave#2", "lt", "2"),
    ];
    let r = todas(&p);
    assert_eq!(
        r.iter()
            .map(|(u, d, _)| (u.as_str(), *d))
            .collect::<Vec<_>>(),
        vec![("carla", -1), ("carla", 0)]
    );
}

// ── El incremental (N5): marcas de tiempo de verdad ─────────────────────────────────────────────────────────────

/// La tabla `cambios` con marcas explicitas: `f:a` de `r<i>` a `1000 + i`, `f:b` igual salvo `r02` y `r04`, cuyo `f:b`
/// se actualizo despues (`5000`, `5001`); `r03` se borro; `r11` tiene la marca en el año 2255.
fn con_cambios() -> Option<String> {
    let u = url(2)?;
    let contenedor = std::env::var("HBASE_LAB_CAOS")
        .ok()
        .filter(|c| !c.is_empty())?;
    shell(&contenedor, "disable 'cambios'\ndrop 'cambios'");
    let mut ordenes = String::from("create 'cambios', 'f'\n");
    for i in 1..=10 {
        ordenes += &format!("put 'cambios', 'r{i:02}', 'f:a', 'a{i}', {}\n", 1000 + i);
        ordenes += &format!("put 'cambios', 'r{i:02}', 'f:b', 'b{i}', {}\n", 1000 + i);
    }
    ordenes += "put 'cambios', 'r02', 'f:b', 'nuevo2', 5000\n";
    ordenes += "put 'cambios', 'r04', 'f:b', 'nuevo4', 5001\n";
    ordenes += "put 'cambios', 'r11', 'f:a', 'futuro', 9000000000000\n";
    ordenes += "deleteall 'cambios', 'r03'\n";
    shell(&contenedor, &ordenes);
    Some(u)
}

fn pet_cambios(u: &str, cols: &[&str], start: Option<&str>, end: Option<&str>) -> Peticion {
    let mut p = peticion(
        u.to_string(),
        "default.cambios",
        &cols.iter().map(|c| (*c, *c)).collect::<Vec<_>>(),
    );
    p.tipos = vec![
        ("_clave".into(), "String".into()),
        ("_ts".into(), "Integer".into()),
        ("f:a".into(), "String".into()),
        ("f:b".into(), "String".into()),
    ];
    p.codificaciones = vec![
        ("_clave".into(), "utf8".into()),
        ("_ts".into(), "i64be".into()),
    ];
    p.start = start.map(String::from);
    p.end = end.map(String::from);
    p.cursor = (start.is_some() || end.is_some()).then(|| "_ts".to_string());
    p
}

/// Como `filas`, pero las columnas enteras (`_ts`) salen como su numero.
fn filas_t(bytes: &[u8]) -> Vec<Vec<Option<String>>> {
    let mut sal = Vec::new();
    for lote in StreamReader::try_new(Cursor::new(bytes), None).unwrap() {
        let lote = lote.unwrap();
        for i in 0..lote.num_rows() {
            sal.push(
                (0..lote.num_columns())
                    .map(|c| {
                        let col = lote.column(c);
                        if col.is_null(i) {
                            None
                        } else if col.data_type() == &arrow_schema::DataType::Int64 {
                            Some(
                                col.as_primitive::<arrow_array::types::Int64Type>()
                                    .value(i)
                                    .to_string(),
                            )
                        } else {
                            Some(col.as_string::<i32>().value(i).to_string())
                        }
                    })
                    .collect(),
            );
        }
    }
    sal
}

fn texto_de(f: &[Option<String>], i: usize) -> String {
    f[i].clone().unwrap_or_else(|| "∅".into())
}

#[test]
fn el_incremental_contra_marcas_de_verdad_trae_filas_completas_y_ve_las_actualizaciones() {
    let Some(u) = con_cambios() else { return };
    let todas = ["_clave", "_ts", "f:a", "f:b"];
    let leer_filas = |p: &Peticion| {
        let (_, bytes) = leer(p);
        let mut v = filas_t(&bytes);
        v.sort();
        v
    };
    // (1005, 1008]: r06, r07, r08, completas
    let r = leer_filas(&pet_cambios(&u, &todas, Some("1005"), Some("1008")));
    let claves: Vec<_> = r.iter().map(|f| texto_de(f, 0)).collect();
    assert_eq!(claves, ["r06", "r07", "r08"]);
    assert_eq!(
        r.iter()
            .map(|f| (texto_de(f, 2), texto_de(f, 3)))
            .collect::<Vec<_>>(),
        [("a6", "b6"), ("a7", "b7"), ("a8", "b8")].map(|(a, b)| (a.to_string(), b.to_string()))
    );
    // una actualizacion: desde 2000 solo r02 y r04, con el f:a ANTIGUO (fuera del rango) y el f:b nuevo, y su marca
    let r = leer_filas(&pet_cambios(&u, &todas, Some("2000"), Some("100000")));
    assert_eq!(r.len(), 2, "{r:?}");
    assert_eq!(
        (
            texto_de(&r[0], 0),
            texto_de(&r[0], 1),
            texto_de(&r[0], 2),
            texto_de(&r[0], 3)
        ),
        ("r02".into(), "5000".into(), "a2".into(), "nuevo2".into())
    );
    assert_eq!(
        (texto_de(&r[1], 0), texto_de(&r[1], 1), texto_de(&r[1], 3)),
        ("r04".into(), "5001".into(), "nuevo4".into())
    );
    // una fila borrada no aparece, y el rango abierto no trae lo del año 2255 si el fin es el testigo
    let t = verbos::testigo(&format!(
        "{{\"url\":\"{u}\",\"objeto\":\"default.cambios\",\"cursor\":\"_ts\"}}"
    ))
    .unwrap();
    let limite = t
        .split("\"valor\":\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    let ahora = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    let limite_n: i64 = limite.parse().unwrap();
    assert!(
        (ahora - 5_000 - limite_n).abs() < 5_000,
        "el testigo es el reloj menos el rezago: {t} (ahora {ahora})"
    );
    let r = leer_filas(&pet_cambios(&u, &todas, None, Some(limite)));
    let claves: Vec<_> = r.iter().map(|f| texto_de(f, 0)).collect();
    assert!(!claves.contains(&"r03".to_string()), "borrada: {claves:?}");
    assert!(
        !claves.contains(&"r11".to_string()),
        "con marca del futuro: {claves:?}"
    );
    assert_eq!(claves.len(), 9, "{claves:?}");
    // sin tope entra lo del año 2255
    let r = leer_filas(&pet_cambios(&u, &todas, Some("1010"), None));
    let claves: Vec<_> = r.iter().map(|f| texto_de(f, 0)).collect();
    // r10 tiene la marca 1010 y `start` es exclusivo: no entra
    assert_eq!(claves, ["r02", "r04", "r11"]);
}

#[test]
fn el_incremental_con_solo_la_clave_es_un_solo_paso_contra_un_servidor_de_verdad() {
    let Some(u) = con_cambios() else { return };
    let (n, bytes) = leer(&pet_cambios(&u, &["_clave"], Some("1005"), Some("1008")));
    assert_eq!(n, 3);
    let mut v: Vec<_> = filas_t(&bytes).iter().map(|f| texto_de(f, 0)).collect();
    v.sort();
    assert_eq!(v, ["r06", "r07", "r08"]);
}

#[test]
fn mas_de_una_tanda_de_claves_se_trae_con_el_filtro_de_filas_en_un_servidor_de_verdad() {
    let Some(u) = url(1) else { return };
    // `TestTable` se cargo hace un rato: su marca es la de la carga, y desde 0 entran todas. Se piden mas de dos tandas.
    let mut p = peticion(u, "default.TestTable", &[("k", "_clave"), ("t", "_ts")]);
    p.tipos = vec![("_ts".into(), "Integer".into())];
    p.codificaciones = vec![("_ts".into(), "i64be".into())];
    p.start = Some("0".into());
    p.cursor = Some("_ts".into());
    p.limit = Some(2_500);
    let t = Instant::now();
    let (n, bytes) = leer(&p);
    assert_eq!(n, 2_500);
    let f = filas_t(&bytes);
    assert_eq!(f.len(), 2_500);
    assert!(
        f.iter()
            .all(|r| r[1].as_ref().is_some_and(|m| m.len() >= 12)),
        "marcas en ms"
    );
    eprintln!("2500 filas por el incremental: {:?}", t.elapsed());
}
