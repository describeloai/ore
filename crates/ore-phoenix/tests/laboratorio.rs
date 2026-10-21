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
    // Un valor que no es de la columna es `operador`, no una consulta que Phoenix rechace.
    p.filtros = vec![ore_driver::Filtro::uno("ID", "eq", "uno")];
    assert_eq!(leer(&p).unwrap_err().codigo, Codigo::Operador);
    // Y un filtro de verdad se empuja: la clave exacta es un `POINT LOOKUP` de una fila.
    p.filtros = vec![ore_driver::Filtro::uno("ID", "eq", "1")];
    assert_eq!(leer(&p).unwrap().1, 1);
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

// ── M3 · el catálogo contra el laboratorio ─────────────────────────────────────────────────────────────

fn catalogo_lab(url: &str) -> ore_driver::catalogo::Catalogo {
    let texto = verbos::catalogo(url, "phoenix").expect("catálogo");
    ore_driver::catalogo::Catalogo::leer(&texto).expect("lo entiende el lector")
}

#[test]
fn el_catalogo_del_laboratorio_tiene_lo_que_hay_y_nada_de_lo_que_no_se_lee() {
    let Some(u) = url() else { return };
    let c = catalogo_lab(&u);
    let n: Vec<&str> = c.tablas.iter().map(|t| t.nombre.as_str()).collect();
    for esperado in [
        "SUELTA",
        "ORE_X0.PEDIDOS",
        "ORE_X0.PEDIDOS_CERRADOS",
        "ORE_X0.SALADA",
        "ORE_X0.EVENTOS",
        "ORE_X0.INQUILINOS",
        "ORE_X0.INMUTABLE",
        "ORE_X0.minusculas",
        "ORE_X0_AJENO.SECRETOS",
    ] {
        assert!(n.contains(&esperado), "falta {esperado}: {n:?}");
    }
    // Ni el sistema, ni el índice, ni las vistas de inquilino (sin `tenant=` no se leen).
    assert!(
        !n.iter()
            .any(|x| x.starts_with("SYSTEM") || x.contains("IDX_") || x.contains("VISTA_T")),
        "{n:?}"
    );
    // Lo que cataloga, se lee: la tabla con minúsculas citadas y la sin esquema.
    for objeto in ["ORE_X0.minusculas", "SUELTA"] {
        let p = peticion(objeto, &[("id", "ID")]).unwrap();
        assert!(leer(&p).expect(objeto).1 >= 1, "{objeto}");
    }
}

#[test]
fn con_tenant_las_vistas_de_inquilino_entran_en_el_catalogo() {
    let Some(u) = url() else { return };
    let c = catalogo_lab(&format!("{u}?tenant=t1"));
    let n: Vec<&str> = c.tablas.iter().map(|t| t.nombre.as_str()).collect();
    assert!(n.contains(&"ORE_X0.VISTA_T1"), "{n:?}");
    let v = c
        .tablas
        .iter()
        .find(|t| t.nombre == "ORE_X0.VISTA_T1")
        .unwrap();
    assert_eq!(v.clase, "view");
}

#[test]
fn un_esquema_en_la_url_limita_el_catalogo_y_explorar_lo_lista() {
    let Some(u) = url() else { return };
    let c = catalogo_lab(&format!("{u}/ORE_X0"));
    assert!(c.tablas.iter().all(|t| t.nombre.starts_with("ORE_X0.")));
    let out = verbos::explorar(&format!(r#"{{"url":"{u}"}}"#)).expect("explorar");
    assert!(
        out.contains("\"ORE_X0\"") && out.contains("\"ORE_X0_AJENO\""),
        "{out}"
    );
}

// ── M4 · los filtros contra Phoenix de verdad ──────────────────────────────────────────────────────────

/// Una tabla de pruebas propia, cargada por el PQS: lo que el kit no trae (barras, un `DATE` con hora).
fn preparar(url: &str, sentencias: &[&str]) {
    let c = ore_phoenix::transporte::Conexion::abrir(url).expect("abrir");
    c.avatica.autocommit(true).expect("autocommit");
    for s in sentencias {
        c.avatica
            .ejecutar_orden(s)
            .unwrap_or_else(|e| panic!("{s}: {e}"));
    }
}

fn ids(url: &str, objeto: &str, filtros: Vec<ore_driver::Filtro>) -> Result<Vec<i64>, Fallo> {
    let p = Peticion {
        url: url.into(),
        objeto: objeto.into(),
        proyeccion: vec![("id".into(), "ID".into())],
        filtros,
        orden: vec![ore_driver::Orden {
            columna: "ID".into(),
            descendente: false,
        }],
        ..Default::default()
    };
    let (lotes, _) = leer(&p)?;
    Ok(lotes
        .iter()
        .flat_map(|b| {
            b.column(0)
                .as_primitive::<Int64Type>()
                .iter()
                .flatten()
                .collect::<Vec<_>>()
        })
        .collect())
}

#[test]
fn un_valor_con_barras_y_comillas_llega_intacto_y_no_rompe_la_sentencia() {
    let Some(u) = url() else { return };
    preparar(
        &u,
        &[
            "CREATE TABLE IF NOT EXISTS ORE_X0.LAB_BARRAS (ID BIGINT NOT NULL PRIMARY KEY, T VARCHAR)",
            r"UPSERT INTO ORE_X0.LAB_BARRAS VALUES (1, 'a\\b')",
            r"UPSERT INTO ORE_X0.LAB_BARRAS VALUES (2, 'x\\')",
            r"UPSERT INTO ORE_X0.LAB_BARRAS VALUES (3, 'o''neil')",
            r"UPSERT INTO ORE_X0.LAB_BARRAS VALUES (4, 'ab')",
        ],
    );
    let eq = |v: &str| vec![ore_driver::Filtro::uno("T", "eq", v)];
    // El valor de ORE es `a\b` (tres caracteres): Phoenix lo guardó así y el filtro lo encuentra.
    assert_eq!(ids(&u, "ORE_X0.LAB_BARRAS", eq(r"a\b")).unwrap(), [1]);
    // Acaba en barra: sin duplicarla, se comería la comilla de cierre.
    assert_eq!(ids(&u, "ORE_X0.LAB_BARRAS", eq(r"x\")).unwrap(), [2]);
    assert_eq!(ids(&u, "ORE_X0.LAB_BARRAS", eq("o'neil")).unwrap(), [3]);
    // Lo que sería una inyección es solo un valor que no está.
    assert!(
        ids(&u, "ORE_X0.LAB_BARRAS", eq(r"x\' OR 1=1 --"))
            .unwrap()
            .is_empty()
    );
    // `in` y `neq` pasan por el mismo escape; el vacío es nulo y no se confunde con nada.
    let en = vec![ore_driver::Filtro {
        columna: "T".into(),
        operador: "in".into(),
        valor: ore_driver::Valor::Lista(vec![r"a\b".into(), "".into(), "ab".into()]),
    }];
    assert_eq!(ids(&u, "ORE_X0.LAB_BARRAS", en).unwrap(), [1, 4]);
    let neq = vec![ore_driver::Filtro::uno("T", "neq", "")];
    assert_eq!(ids(&u, "ORE_X0.LAB_BARRAS", neq).unwrap(), [1, 2, 3, 4]);
    // Un `like` con barra no se empuja: el servidor lo leería distinto.
    let e = ids(
        &u,
        "ORE_X0.LAB_BARRAS",
        vec![ore_driver::Filtro::uno("T", "like", r"a\_")],
    )
    .unwrap_err();
    assert_eq!(e.codigo, Codigo::Operador);
}

#[test]
fn un_date_con_hora_se_filtra_como_instante() {
    let Some(u) = url() else { return };
    preparar(
        &u,
        &[
            "CREATE TABLE IF NOT EXISTS ORE_X0.LAB_FECHAS (ID BIGINT NOT NULL PRIMARY KEY, D DATE, T TIMESTAMP)",
            "UPSERT INTO ORE_X0.LAB_FECHAS VALUES (1, TO_DATE('2026-10-04 10:30:00', 'yyyy-MM-dd HH:mm:ss', 'GMT'), TIMESTAMP '2026-10-04 10:30:00.123')",
            "UPSERT INTO ORE_X0.LAB_FECHAS VALUES (2, TO_DATE('2026-10-05 00:00:00', 'yyyy-MM-dd HH:mm:ss', 'GMT'), TIMESTAMP '2026-10-05 00:00:00')",
        ],
    );
    let f = |c: &str, op: &str, v: &str| vec![ore_driver::Filtro::uno(c, op, v)];
    // Sin declarar, un `DATE` es un instante y se compara con su hora.
    assert_eq!(
        ids(
            &u,
            "ORE_X0.LAB_FECHAS",
            f("D", "eq", "2026-10-04T10:30:00Z")
        )
        .unwrap(),
        [1]
    );
    assert_eq!(
        ids(
            &u,
            "ORE_X0.LAB_FECHAS",
            f("D", "ge", "2026-10-04T10:30:00.001Z")
        )
        .unwrap(),
        [2]
    );
    // Un `TIMESTAMP` compara los milisegundos que se ven.
    assert_eq!(
        ids(
            &u,
            "ORE_X0.LAB_FECHAS",
            f("T", "eq", "2026-10-04T10:30:00.123Z")
        )
        .unwrap(),
        [1]
    );
}

// ── M5 · la lectura partida contra Phoenix de verdad ───────────────────────────────────────────────────

/// Los `id` de `KIT.GRANDE` (10⁶ filas, la semilla del kit) leídos con `paralelo` conexiones, y cuánto tardó.
fn ids_de_grande(paralelo: usize) -> Option<(Vec<i64>, std::time::Duration)> {
    let u = url()?;
    let mut p = peticion("KIT.GRANDE", &[("id", "id")])?;
    p.url = format!("{u}?paralelo={paralelo}");
    let t = Instant::now();
    let (lotes, filas) = leer(&p).ok()?; // sin la semilla del kit, no hay nada que comparar
    let ids: Vec<i64> = lotes
        .iter()
        .flat_map(|b| {
            b.column(0)
                .as_primitive::<Int64Type>()
                .iter()
                .flatten()
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(ids.len() as u64, filas);
    Some((ids, t.elapsed()))
}

#[test]
fn partida_en_cuatro_da_las_mismas_filas_que_sin_partir_y_tarda_menos() {
    let Some((mut una, t_una)) = ids_de_grande(1) else {
        return;
    };
    let (mut cuatro, t_cuatro) = ids_de_grande(4).expect("la misma tabla");
    println!("una conexión: {t_una:?} · cuatro: {t_cuatro:?}");
    assert_eq!(una.len(), 1_000_000);
    assert_eq!(cuatro.len(), una.len(), "ni una fila de más ni de menos");
    una.sort_unstable();
    cuatro.sort_unstable();
    assert_eq!(una, cuatro, "las mismas filas, sin repetir ni perder");
    assert_eq!(una.first(), Some(&1));
    assert_eq!(una.last(), Some(&1_000_000));
    assert!(una.windows(2).all(|w| w[0] < w[1]), "sin repetidas");
    // Medido: 22,9 s → 6,2 s. Un margen holgado: el laboratorio no es un banco de pruebas de rendimiento.
    assert!(
        t_cuatro < t_una,
        "partir no ayudó: {t_una:?} → {t_cuatro:?}"
    );
}

#[test]
fn con_filtro_orden_o_limite_la_lectura_no_se_parte() {
    let Some(u) = url() else { return };
    let mut p = peticion("KIT.GRANDE", &[("id", "id")]).unwrap();
    p.url = format!("{u}?paralelo=4");
    p.orden = vec![ore_driver::Orden {
        columna: "id".into(),
        descendente: true,
    }];
    p.limit = Some(5);
    let Ok((lotes, _)) = leer(&p) else { return };
    let ids: Vec<i64> = lotes[0]
        .column(0)
        .as_primitive::<Int64Type>()
        .iter()
        .flatten()
        .collect();
    // El orden y el límite se respetan: las cinco últimas, de mayor a menor.
    assert_eq!(ids, [1_000_000, 999_999, 999_998, 999_997, 999_996]);
    // Un filtro sí se parte con el resto: el rango se suma a él, no lo sustituye.
    let mut p = peticion("KIT.GRANDE", &[("id", "id")]).unwrap();
    p.url = format!("{u}?paralelo=4");
    p.filtros = vec![ore_driver::Filtro::uno("grupo", "eq", "7")];
    let (_, filas) = leer(&p).unwrap();
    assert_eq!(filas, 10_000);
}

// ── M7 · lo incremental por ROW_TIMESTAMP, contra Phoenix de verdad ────────────────────────────────────

fn ahora_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

/// Un instante como literal de Phoenix.
fn instante(ms: i64) -> String {
    format!(
        "TIMESTAMP '{}'",
        ore_core::tipos::Valor::FechaHora(ms * 1000).texto(&ore_core::tipos::Fisico::FechaHora)
    )
}

/// Lo que el `testigo` dice de un instante: el ordinal que luego vuelve como `start`.
fn ordinal_de(ms: i64) -> String {
    ore_core::tipos::Valor::Instante(ms * 1000).texto(&ore_core::tipos::Fisico::Instante)
}

/// `(modo, valor)` del verbo `testigo`.
fn testigo_de(
    url: &str,
    objeto: &str,
    cursor: Option<&str>,
) -> Result<(String, Option<String>), String> {
    let cursor = cursor.map_or(String::new(), |c| format!(r#","cursor":"{c}""#));
    let entrada = format!(r#"{{"objeto":"{objeto}","url":"{url}"{cursor}}}"#);
    let salida = verbos::testigo(&entrada)?;
    let n = ore_core::parse::parse(&salida).expect("el testigo es JSON");
    let campo = |k: &str| n.get(k).and_then(|(_, v)| v.as_str()).map(String::from);
    Ok((campo("modo").expect("modo"), campo("valor")))
}

fn rango_de(
    url: &str,
    objeto: &str,
    cursor: &str,
    start: Option<&str>,
    end: Option<&str>,
) -> Result<Vec<i64>, Fallo> {
    let p = Peticion {
        url: url.into(),
        objeto: objeto.into(),
        proyeccion: vec![("id".into(), "ID".into())],
        orden: vec![ore_driver::Orden {
            columna: "ID".into(),
            descendente: false,
        }],
        cursor: Some(cursor.into()),
        start: start.map(String::from),
        end: end.map(String::from),
        ..Default::default()
    };
    let (lotes, _) = leer(&p)?;
    Ok(lotes
        .iter()
        .flat_map(|b| {
            b.column(0)
                .as_primitive::<Int64Type>()
                .iter()
                .flatten()
                .collect::<Vec<_>>()
        })
        .collect())
}

fn sin_espacios(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

#[test]
fn el_incremental_por_row_timestamp_de_punta_a_punta() {
    let Some(u) = url() else { return };
    let base = ahora_ms() - 3_600_000;
    preparar(
        &u,
        &[
            "DROP TABLE IF EXISTS ore_x0.inc_m7",
            "CREATE TABLE ore_x0.inc_m7 (ts TIMESTAMP NOT NULL, id BIGINT NOT NULL, v VARCHAR, CONSTRAINT pk PRIMARY KEY (ts ROW_TIMESTAMP, id))",
        ],
    );
    let alta = |id: i64, ms: i64, v: &str| {
        preparar(
            &u,
            &[&format!(
                "UPSERT INTO ore_x0.inc_m7 VALUES ({}, {id}, '{v}')",
                instante(ms)
            )],
        )
    };
    let objeto = "ORE_X0.INC_M7";

    // El catálogo la declara: upsert por la clave, `witness: field` por la ROW_TIMESTAMP.
    let cat = sin_espacios(&verbos::catalogo(&u, "x").expect("catalogo"));
    // El catálogo ordena las claves: `changes` va antes que `name`, dentro del mismo objeto.
    let i = cat
        .find("\"name\":\"ORE_X0.INC_M7\"")
        .expect("INC_M7 en el catálogo");
    let suyo = &cat[cat[..i].rfind("\"changes\"").expect("changes")..i];
    for esperado in [
        r#""mode":"upsert""#,
        r#""key":["TS","ID"]"#,
        r#""witness":"field""#,
        r#""field":"TS""#,
    ] {
        assert!(suyo.contains(esperado), "falta {esperado} en {suyo:.600}");
    }

    // 1. La primera copia: cinco filas, hasta el testigo.
    for i in 1..=5 {
        alta(i, base + i * 1000, "a");
    }
    let (modo, v1) = testigo_de(&u, objeto, Some("TS")).unwrap();
    assert_eq!(modo, "field");
    let v1 = v1.expect("hay filas por debajo del tope");
    assert_eq!(
        v1,
        ordinal_de(base + 5000),
        "la última marca que no pasa del tope"
    );
    assert_eq!(
        rango_de(&u, objeto, "TS", None, Some(&v1)).unwrap(),
        vec![1, 2, 3, 4, 5]
    );

    // 2. Tres altas más: el siguiente refresco trae esas tres, ni una menos ni una repetida (el borde, id 5, no).
    for i in 6..=8 {
        alta(i, base + 10_000 + i * 1000, "b");
    }
    let (_, v2) = testigo_de(&u, objeto, Some("TS")).unwrap();
    let v2 = v2.unwrap();
    assert!(v2 > v1, "el testigo avanza: {v2} > {v1}");
    assert_eq!(
        rango_de(&u, objeto, "TS", Some(&v1), Some(&v2)).unwrap(),
        vec![6, 7, 8]
    );
    assert_eq!(
        rango_de(&u, objeto, "TS", Some(&v2), Some(&v2)).unwrap(),
        Vec::<i64>::new()
    );

    // 3. Una fila con la marca del futuro no hace saltar el cursor: el testigo no se mueve y la fila entrará
    //    cuando llegue su hora (sin el tope, todo lo que se escribiera hasta entonces quedaría por debajo).
    alta(99, ahora_ms() + 86_400_000, "futura");
    let (_, v3) = testigo_de(&u, objeto, Some("TS")).unwrap();
    assert_eq!(
        v3.as_deref(),
        Some(v2.as_str()),
        "el testigo no salta al futuro"
    );
    assert_eq!(
        rango_de(&u, objeto, "TS", Some(&v2), Some(&v2)).unwrap(),
        Vec::<i64>::new()
    );

    // 4. Una fila con la marca de ahora todavía no es del testigo (el rezago); con `rezago=0` sí.
    let reciente = ahora_ms();
    alta(100, reciente, "reciente");
    let (_, v4) = testigo_de(&u, objeto, Some("TS")).unwrap();
    assert_eq!(
        v4.as_deref(),
        Some(v2.as_str()),
        "dentro del rezago no se cubre"
    );
    let sin_rezago = format!("{u}?rezago=0");
    let (_, v5) = testigo_de(&sin_rezago, objeto, Some("TS")).unwrap();
    assert!(
        v5.as_deref().unwrap() >= ordinal_de(reciente).as_str(),
        "con rezago=0 sí: {v5:?}"
    );

    // 5. Lo que un cursor no puede ver, dicho con un test: una fila escrita tarde con una marca vieja, una
    //    actualización (la marca es parte de la clave) y un borrado.
    alta(50, base + 2500, "tarde");
    preparar(
        &u,
        &[&format!(
            "UPSERT INTO ore_x0.inc_m7 VALUES ({}, 7, 'ACTUALIZADA')",
            instante(base + 10_000 + 7 * 1000)
        )],
    );
    preparar(&u, &["DELETE FROM ore_x0.inc_m7 WHERE id = 8"]);
    assert_eq!(
        rango_de(&u, objeto, "TS", Some(&v2), Some(&v2)).unwrap(),
        Vec::<i64>::new(),
        "ni la tardía, ni la actualizada, ni el borrado se ven por encima del cursor"
    );
    let todas = ids(&u, objeto, vec![]).unwrap();
    assert!(
        todas.contains(&50) && todas.contains(&7) && !todas.contains(&8),
        "{todas:?}"
    );

    // 6. Sin `cursor`, `none`; con otra columna, o en una tabla sin ROW_TIMESTAMP, se rechaza.
    assert_eq!(testigo_de(&u, objeto, None).unwrap(), ("none".into(), None));
    let e = testigo_de(&u, objeto, Some("V")).unwrap_err();
    assert!(e.contains("barrido entero"), "{e}");
    let e = rango_de(&u, objeto, "V", Some("a"), None).unwrap_err();
    assert_eq!(e.codigo, Codigo::Operador, "{e:?}");
    preparar(
        &u,
        &[
            "DROP TABLE IF EXISTS ore_x0.inc_sin",
            "CREATE TABLE ore_x0.inc_sin (id BIGINT NOT NULL PRIMARY KEY, ts TIMESTAMP)",
        ],
    );
    let e = testigo_de(&u, "ORE_X0.INC_SIN", Some("TS")).unwrap_err();
    assert!(
        e.contains("no declara ninguna columna ROW_TIMESTAMP"),
        "{e}"
    );
    let e = rango_de(
        &u,
        "ORE_X0.INC_SIN",
        "TS",
        Some("2026-01-01T00:00:00Z"),
        None,
    )
    .unwrap_err();
    assert_eq!(e.codigo, Codigo::Operador, "{e:?}");

    // 7. Una tabla vacía no es un fallo: es `field` sin valor.
    preparar(
        &u,
        &[
            "DROP TABLE IF EXISTS ore_x0.inc_vacia",
            "CREATE TABLE ore_x0.inc_vacia (ts TIMESTAMP NOT NULL, id BIGINT NOT NULL, CONSTRAINT pk PRIMARY KEY (ts ROW_TIMESTAMP, id))",
        ],
    );
    assert_eq!(
        testigo_de(&u, "ORE_X0.INC_VACIA", Some("TS")).unwrap(),
        ("field".into(), None)
    );
}

#[test]
fn row_timestamp_en_segunda_posicion_se_lee_pero_no_se_declara_ni_se_pregunta() {
    let Some(u) = url() else { return };
    let base = ahora_ms() - 3_600_000;
    preparar(
        &u,
        &[
            "DROP TABLE IF EXISTS ore_x0.inc_seg",
            "CREATE TABLE ore_x0.inc_seg (cliente BIGINT NOT NULL, ts TIMESTAMP NOT NULL, id BIGINT, CONSTRAINT pk PRIMARY KEY (cliente, ts ROW_TIMESTAMP))",
            &format!(
                "UPSERT INTO ore_x0.inc_seg VALUES (1, {}, 11)",
                instante(base)
            ),
            &format!(
                "UPSERT INTO ore_x0.inc_seg VALUES (1, {}, 12)",
                instante(base + 5000)
            ),
        ],
    );
    // El catálogo no la declara: su testigo costaría un barrido de la tabla.
    let cat = sin_espacios(&verbos::catalogo(&u, "x").expect("catalogo"));
    let i = cat
        .find("\"name\":\"ORE_X0.INC_SEG\"")
        .expect("INC_SEG en el catálogo");
    let suyo = &cat[cat[..i].rfind("\"changes\"").expect("changes")..i];
    assert!(
        suyo.starts_with("\"changes\":{\"mode\":\"none\",\"witness\":\"none\"}"),
        "{suyo:.200}"
    );
    let e = testigo_de(&u, "ORE_X0.INC_SEG", Some("TS")).unwrap_err();
    assert!(e.contains("barrido"), "{e}");
    // Pero el rango sí se sabe servir: es una condición más, y Phoenix poda los ficheros por su marca.
    let (lotes, _) = leer(&Peticion {
        url: u.clone(),
        objeto: "ORE_X0.INC_SEG".into(),
        proyeccion: vec![("id".into(), "ID".into())],
        cursor: Some("TS".into()),
        start: Some(ordinal_de(base)),
        ..Default::default()
    })
    .unwrap();
    let ids: Vec<i64> = lotes
        .iter()
        .flat_map(|b| {
            b.column(0)
                .as_primitive::<Int64Type>()
                .iter()
                .flatten()
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(ids, vec![12]);
}

#[test]
fn row_timestamp_bigint_y_date_tambien_son_cursores() {
    let Some(u) = url() else { return };
    let base = ahora_ms() - 3_600_000;
    preparar(
        &u,
        &[
            "DROP TABLE IF EXISTS ore_x0.inc_big",
            "CREATE TABLE ore_x0.inc_big (t BIGINT NOT NULL, id BIGINT NOT NULL, CONSTRAINT pk PRIMARY KEY (t ROW_TIMESTAMP, id))",
            &format!("UPSERT INTO ore_x0.inc_big VALUES ({base}, 1)"),
            &format!("UPSERT INTO ore_x0.inc_big VALUES ({}, 2)", base + 5000),
            "DROP TABLE IF EXISTS ore_x0.inc_date",
            "CREATE TABLE ore_x0.inc_date (d DATE NOT NULL, id BIGINT NOT NULL, CONSTRAINT pk PRIMARY KEY (d ROW_TIMESTAMP, id))",
            &format!(
                "UPSERT INTO ore_x0.inc_date VALUES (CAST({} AS DATE), 1)",
                instante(base)
            ),
            &format!(
                "UPSERT INTO ore_x0.inc_date VALUES (CAST({} AS DATE), 2)",
                instante(base + 5000)
            ),
        ],
    );
    // BIGINT: el ordinal es el propio entero.
    let (modo, v) = testigo_de(&u, "ORE_X0.INC_BIG", Some("T")).unwrap();
    assert_eq!(modo, "field");
    assert_eq!(v.as_deref(), Some((base + 5000).to_string().as_str()));
    assert_eq!(
        rango_de(&u, "ORE_X0.INC_BIG", "T", None, v.as_deref()).unwrap(),
        vec![1, 2]
    );
    assert_eq!(
        rango_de(
            &u,
            "ORE_X0.INC_BIG",
            "T",
            Some(&base.to_string()),
            v.as_deref()
        )
        .unwrap(),
        vec![2]
    );
    // Una columna DATE que se proyecta Y se ordena: con el alias `AS "D"` Phoenix daba 502 (referencia ambigua).
    let (lotes, filas) = leer(&Peticion {
        url: u.clone(),
        objeto: "ORE_X0.INC_DATE".into(),
        proyeccion: vec![("d".into(), "D".into()), ("id".into(), "ID".into())],
        orden: vec![ore_driver::Orden {
            columna: "D".into(),
            descendente: true,
        }],
        ..Default::default()
    })
    .expect("ordenar por un DATE que también se proyecta");
    assert_eq!(filas, 2, "{lotes:?}");
    // DATE: Phoenix guarda la hora; el ordinal es un instante.
    let (_, v) = testigo_de(&u, "ORE_X0.INC_DATE", Some("D")).unwrap();
    assert_eq!(v.as_deref(), Some(ordinal_de(base + 5000).as_str()));
    assert_eq!(
        rango_de(
            &u,
            "ORE_X0.INC_DATE",
            "D",
            Some(&ordinal_de(base)),
            v.as_deref()
        )
        .unwrap(),
        vec![2]
    );
}

#[test]
fn el_tiempo_limite_corta_tambien_la_sonda_de_la_particion() {
    // Una vista que filtra y no da ninguna fila: sus dos sondas de los extremos de la clave (`ORDER BY id LIMIT 1`,
    // ascendente y descendente) son un barrido cada una. Antes de M7 corrían sin el `timeoutMs` de la petición
    // (3,8 s la primera y más de 60 s la segunda en 262 144 filas) y la lectura no volvía a tiempo.
    let Some(u) = url() else { return };
    preparar(
        &u,
        &[
            "DROP VIEW IF EXISTS ore_x0.pedidos_nunca",
            "CREATE VIEW ore_x0.pedidos_nunca AS SELECT * FROM ore_x0.pedidos \
             WHERE REGEXP_SUBSTR(TO_CHAR(id, '#') || '-' || estado, '(1|12|123|1234)+-[0-9]*x') IS NOT NULL",
        ],
    );
    let p = Peticion {
        url: u,
        objeto: "ORE_X0.PEDIDOS_NUNCA".into(),
        proyeccion: vec![("id".into(), "ID".into())],
        timeout_ms: Some(1000),
        ..Default::default()
    };
    let t = Instant::now();
    let e = leer(&p).expect_err("la vista no da filas y el límite es de un segundo");
    let dt = t.elapsed();
    assert_eq!(e.codigo, Codigo::Tiempo, "{e:?}");
    assert!(
        // Con la sonda acotada son ~2,3 s (un segundo de sonda y uno de lectura); sin acotar, 3,8 s de sonda más uno
        // de lectura: ~5,8 s. El umbral está en medio a propósito (un 6 no distinguía una cosa de la otra).
        dt < std::time::Duration::from_secs(4),
        "tardó {dt:?}: la sonda no respeta el límite"
    );
}
