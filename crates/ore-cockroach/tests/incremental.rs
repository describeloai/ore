//! El incremental de CockroachDB de verdad (ADR 0080 §6, paso 7): el rango por una **columna** (`cursor`, `start` exclusivo, `end` inclusivo) y el
//! rango por la **posición del origen** (sin `cursor`: dos instantes del servidor, `AS OF SYSTEM TIME`). Se salta sola si no hay laboratorio:
//!
//! ```text
//! COCKROACH_LAB_URLS=cockroachdb://kit@localhost:26257/kit,…  ORE_COCKROACH_LABORATORIO=1  cargo test -p ore-cockroach --test incremental
//! ```
//!
//! La parte repartida usa `lab_hash` (250 000 filas, clave *hash-sharded*), que deja `tests/reparto.rs`; si no está, esa prueba se salta.

use arrow_array::RecordBatch;
use arrow_array::cast::AsArray;
use arrow_array::types::Int64Type;
use arrow_ipc::reader::StreamReader;
use ore_base::parse::parse;
use ore_cockroach::Cockroach;
use ore_cockroach::lectura::Token;
use ore_cockroach::verbos::testigo;
use ore_driver::origen::Origen;
use ore_driver::{Codigo, Fallo};
use std::sync::Mutex;

static UNO_A_UNO: Mutex<()> = Mutex::new(());

fn servidores() -> Vec<String> {
    let lab = std::env::var("ORE_COCKROACH_LABORATORIO").as_deref() == Ok("1");
    match std::env::var("COCKROACH_LAB_URLS") {
        Ok(v) if !v.trim().is_empty() && lab => {
            let urls: Vec<String> = v.split(',').map(|s| s.trim().to_string()).collect();
            static UNA: std::sync::Once = std::sync::Once::new();
            UNA.call_once(|| urls.iter().for_each(|u| preparar(u)));
            urls
        }
        _ => {
            eprintln!(
                "sin COCKROACH_LAB_URLS y ORE_COCKROACH_LABORATORIO=1: no hay laboratorio, se salta"
            );
            Vec::new()
        }
    }
}

fn escritura(url: &str) -> postgres::Client {
    let root = url.replace("//kit@", "//root@");
    let mut a = ore_cockroach::tls::abrir(&root)
        .unwrap_or_else(|f| panic!("{root}: {f:?}"))
        .cliente;
    a.batch_execute("SET default_transaction_read_only = false")
        .unwrap();
    a
}

/// Diez filas: `id` 1..10, `v = 'r<id>'`, `dia = 2026-01-<id>`, `creado = 2026-01-<id> 10:00:00+00`.
fn preparar(url: &str) {
    let mut a = escritura(url);
    for sql in [
        "DROP VIEW IF EXISTS lab_inc_v",
        "DROP TABLE IF EXISTS lab_inc",
        "CREATE TABLE lab_inc (id INT8 PRIMARY KEY, v STRING, dia DATE, creado TIMESTAMPTZ)",
        "INSERT INTO lab_inc SELECT g, 'r' || g::STRING, ('2026-01-01'::DATE + (g - 1)::INT), ('2026-01-01 10:00:00+00'::TIMESTAMPTZ + ((g - 1) || ' days')::INTERVAL) \
         FROM generate_series(1, 10) AS g",
        "CREATE VIEW lab_inc_v AS SELECT id, v FROM lab_inc",
        "GRANT SELECT ON TABLE lab_inc, lab_inc_v TO kit",
    ] {
        a.batch_execute(sql)
            .unwrap_or_else(|e| panic!("{url}: {sql}: {}", ore_cockroach::tls::causa(&e)));
    }
    // Si una vuelta anterior se cortó a medias, `lab_hash` puede haber quedado cambiada.
    let _ = a.batch_execute("UPDATE lab_hash SET texto = 'fila-' || id::STRING WHERE id <= 1000 AND texto != 'fila-' || id::STRING");
}

fn js(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Lee `(id, v)` de `objeto` con `extra` en el JSON de la petición, **en el orden en que llegan**.
fn filas(url: &str, objeto: &str, extra: &str) -> Result<Vec<(i64, String)>, Fallo> {
    let mut s = Cockroach::conectar(url)?;
    let p = ore_driver::leer_peticion(&format!(
        r#"{{"url":"{}","objeto":"{objeto}","proyeccion":{{"id":"id","v":"v"}},"formato":"arrow"{extra}}}"#,
        js(url)
    ))
    .expect("petición");
    let mut salida = Vec::new();
    Cockroach::leer(&mut s, &Token::default(), &p, &mut salida)?;
    let lotes: Vec<RecordBatch> = StreamReader::try_new(&salida[..], None)
        .expect("flujo Arrow")
        .map(|l| l.expect("lote"))
        .collect();
    let mut out = Vec::new();
    for b in &lotes {
        let (id, v) = (
            b.column(0).as_primitive::<Int64Type>(),
            b.column(1).as_string::<i32>(),
        );
        for i in 0..b.num_rows() {
            out.push((id.value(i), v.value(i).to_string()));
        }
    }
    Ok(out)
}

/// Solo los `id` de `objeto`, ordenados: para las tablas grandes, donde las demás columnas son de otro tipo.
fn solo_ids(url: &str, objeto: &str, extra: &str) -> Result<Vec<i64>, Fallo> {
    let mut s = Cockroach::conectar(url)?;
    let p = ore_driver::leer_peticion(&format!(
        r#"{{"url":"{}","objeto":"{objeto}","proyeccion":{{"id":"id"}},"formato":"arrow"{extra}}}"#,
        js(url)
    ))
    .expect("petición");
    let mut salida = Vec::new();
    Cockroach::leer(&mut s, &Token::default(), &p, &mut salida)?;
    let mut v: Vec<i64> = StreamReader::try_new(&salida[..], None)
        .expect("flujo Arrow")
        .flat_map(|b| {
            b.expect("lote")
                .column(0)
                .as_primitive::<Int64Type>()
                .values()
                .to_vec()
        })
        .collect();
    v.sort_unstable();
    Ok(v)
}

fn ids(f: &[(i64, String)]) -> Vec<i64> {
    let mut v: Vec<i64> = f.iter().map(|(i, _)| *i).collect();
    v.sort_unstable();
    v
}

fn rango(cursor: &str, start: Option<&str>, end: Option<&str>) -> String {
    let mut s = format!(r#","cursor":"{cursor}""#);
    if let Some(x) = start {
        s.push_str(&format!(r#","start":"{x}""#));
    }
    if let Some(x) = end {
        s.push_str(&format!(r#","end":"{x}""#));
    }
    s
}

fn posicion(start: Option<&str>, end: Option<&str>) -> String {
    let mut s = String::new();
    if let Some(x) = start {
        s.push_str(&format!(r#","start":"{x}""#));
    }
    if let Some(x) = end {
        s.push_str(&format!(r#","end":"{x}""#));
    }
    s
}

/// El testigo de ahora: el instante del servidor.
fn ahora(url: &str) -> String {
    let t = testigo(&format!(
        r#"{{"url":"{}","objeto":"public.lab_inc"}}"#,
        js(url)
    ))
    .unwrap_or_else(|e| panic!("{url}: {e}"));
    let n = parse(&t).unwrap();
    assert_eq!(
        n.get("modo").and_then(|(_, v)| v.as_str()),
        Some("snapshot"),
        "{url}: {t}"
    );
    n.get("valor")
        .and_then(|(_, v)| v.as_str())
        .map(String::from)
        .unwrap_or_else(|| panic!("{url}: sin valor: {t}"))
}

#[test]
fn el_rango_por_una_columna_es_start_exclusivo_y_end_inclusivo() {
    let _g = UNO_A_UNO.lock().unwrap_or_else(|e| e.into_inner());
    for url in servidores() {
        preparar_inc(&url);
        let leer = |extra: String| {
            ids(&filas(&url, "public.lab_inc", &extra)
                .unwrap_or_else(|f| panic!("{url} {extra}: {f:?}")))
        };
        assert_eq!(
            leer(rango("id", Some("3"), Some("6"))),
            vec![4, 5, 6],
            "{url}: (3, 6]"
        );
        assert_eq!(
            leer(rango("id", Some("7"), None)),
            vec![8, 9, 10],
            "{url}: desde el 7, sin cota arriba"
        );
        assert_eq!(
            leer(rango("id", None, Some("2"))),
            vec![1, 2],
            "{url}: hasta el 2, desde el principio"
        );
        assert_eq!(
            leer(rango("id", Some("5"), Some("5"))),
            Vec::<i64>::new(),
            "{url}: (5, 5] está vacío"
        );
        assert_eq!(
            leer(rango("id", Some("10"), None)),
            Vec::<i64>::new(),
            "{url}: nada después del último"
        );
        // DATE y TIMESTAMPTZ: el borde es exacto.
        assert_eq!(
            leer(rango("dia", Some("2026-01-03"), Some("2026-01-05"))),
            vec![4, 5],
            "{url}"
        );
        assert_eq!(
            leer(rango("creado", Some("2026-01-03 10:00:00+00"), None)),
            (4..=10).collect::<Vec<_>>(),
            "{url}"
        );
        assert_eq!(
            leer(rango("creado", None, Some("2026-01-03 12:00:00+02"))),
            vec![1, 2, 3],
            "{url}: 12:00+02 son las 10:00 UTC, inclusive"
        );
        // Con filtros, orden y límite a la vez.
        let combinado = format!(
            r#"{},"filtros":[{{"columna":"v","operador":"like","valor":"r%"}}],"orderBy":[{{"columna":"id","direccion":"desc"}}],"limit":3"#,
            rango("id", Some("2"), None)
        );
        let f = filas(&url, "public.lab_inc", &combinado).unwrap();
        assert_eq!(
            f.iter().map(|(i, _)| *i).collect::<Vec<_>>(),
            vec![10, 9, 8],
            "{url}"
        );
        // Una columna de cursor que no está es un cambio de esquema.
        let f = filas(
            &url,
            "public.lab_inc",
            &rango("ya_no_esta", Some("1"), None),
        )
        .unwrap_err();
        assert_eq!(f.codigo, Codigo::Esquema, "{url}: {f:?}");
    }
}

#[test]
fn el_rango_por_una_columna_de_una_tabla_grande_da_lo_mismo_en_tramos_y_con_una_conexion() {
    let _g = UNO_A_UNO.lock().unwrap_or_else(|e| e.into_inner());
    for url in servidores() {
        let extra = rango("id", Some("100000"), Some("200000"));
        let Ok(en_tramos) = solo_ids(&url, "public.lab_reparto", &extra) else {
            eprintln!("{url}: sin `lab_reparto` (corre `--test reparto` antes): se salta");
            continue;
        };
        let con_una = solo_ids(&format!("{url}?tramos=1"), "public.lab_reparto", &extra).unwrap();
        assert_eq!(en_tramos.len(), 100_000, "{url}: (100000, 200000]");
        assert_eq!(en_tramos, con_una, "{url}");
        assert_eq!(en_tramos, (100_001..=200_000).collect::<Vec<_>>(), "{url}");
    }
}

#[test]
fn el_testigo_es_un_instante_que_avanza_y_se_ordena_como_texto() {
    let _g = UNO_A_UNO.lock().unwrap_or_else(|e| e.into_inner());
    for url in servidores() {
        let (a, b) = (ahora(&url), ahora(&url));
        assert!(a < b, "{url}: {a} no es anterior a {b}");
        assert_eq!(
            a.len(),
            b.len(),
            "{url}: mismo ancho, por eso se ordenan igual como texto y como número"
        );
    }
}

#[test]
fn el_rango_por_posicion_da_lo_que_se_inserto_o_actualizo_y_nunca_un_borrado() {
    let _g = UNO_A_UNO.lock().unwrap_or_else(|e| e.into_inner());
    for url in servidores() {
        preparar_inc(&url);
        let leer = |extra: String| {
            filas(&url, "public.lab_inc", &extra).unwrap_or_else(|f| panic!("{url} {extra}: {f:?}"))
        };
        let t0 = ahora(&url);
        let mut w = escritura(&url);
        for sql in [
            "INSERT INTO lab_inc (id, v) VALUES (11, 'nuevo11'), (12, 'nuevo12')",
            "UPDATE lab_inc SET v = 'R2' WHERE id = 2",
            "DELETE FROM lab_inc WHERE id = 5",
        ] {
            w.batch_execute(sql).unwrap();
        }
        let t1 = ahora(&url);

        // El delta: lo insertado y lo actualizado. El borrado (id 5) no aparece: no deja rastro.
        let delta = leer(posicion(Some(&t0), Some(&t1)));
        assert_eq!(
            ids(&delta),
            vec![2, 11, 12],
            "{url}: el delta (t0, t1] no trae el borrado"
        );
        assert_eq!(
            delta.iter().find(|(i, _)| *i == 2).unwrap().1,
            "R2",
            "{url}"
        );

        // `end` es el estado EXACTO de ese instante: a t1, sin el 5 y con el 2 cambiado; a t0, como estaba (con el 5 y el 2 viejo).
        let a_t1 = leer(posicion(None, Some(&t1)));
        assert_eq!(
            ids(&a_t1),
            vec![1, 2, 3, 4, 6, 7, 8, 9, 10, 11, 12],
            "{url}"
        );
        let a_t0 = leer(posicion(None, Some(&t0)));
        assert_eq!(
            ids(&a_t0),
            (1..=10).collect::<Vec<_>>(),
            "{url}: a t0 el 5 todavía estaba"
        );
        assert_eq!(
            a_t0.iter().find(|(i, _)| *i == 2).unwrap().1,
            "r2",
            "{url}: y el 2 tenía su valor de entonces"
        );

        // Y sigue siendo así aunque la tabla cambie después: es historia, no una foto de ahora.
        w.batch_execute("UPDATE lab_inc SET v = 'otra vez' WHERE id = 1")
            .unwrap();
        assert_eq!(
            leer(posicion(None, Some(&t0)))
                .iter()
                .find(|(i, _)| *i == 1)
                .unwrap()
                .1,
            "r1",
            "{url}"
        );

        // El siguiente refresco empieza donde acabó el anterior (`start` exclusivo): ni repite ni se salta nada.
        let t2 = ahora(&url);
        w.batch_execute("INSERT INTO lab_inc (id, v) VALUES (13, 'nuevo13')")
            .unwrap();
        let t3 = ahora(&url);
        assert_eq!(
            ids(&leer(posicion(Some(&t1), Some(&t2)))),
            vec![1],
            "{url}: solo el update del 1"
        );
        assert_eq!(
            ids(&leer(posicion(Some(&t2), Some(&t3)))),
            vec![13],
            "{url}: solo el insert del 13"
        );
        assert_eq!(
            ids(&leer(posicion(Some(&t3), Some(&t3)))),
            Vec::<i64>::new(),
            "{url}: (t, t] está vacío"
        );
        // Sin `end`, hasta ahora.
        assert_eq!(
            ids(&leer(posicion(Some(&t3), None))),
            Vec::<i64>::new(),
            "{url}"
        );
        assert_eq!(ids(&leer(posicion(Some(&t2), None))), vec![13], "{url}");

        // Se puede combinar con filtros, orden y límite.
        let f = leer(format!(
            r#"{},"filtros":[{{"columna":"id","operador":"ge","valor":"11"}}],"orderBy":[{{"columna":"id","direccion":"desc"}}],"limit":2"#,
            posicion(Some(&t0), None)
        ));
        assert_eq!(
            f.iter().map(|(i, _)| *i).collect::<Vec<_>>(),
            vec![13, 12],
            "{url}"
        );
    }
}

/// Deja `lab_inc` como la creó `preparar`, para que cada prueba de posición parta de lo mismo.
fn preparar_inc(url: &str) {
    let mut a = escritura(url);
    for sql in [
        "DELETE FROM lab_inc",
        "INSERT INTO lab_inc SELECT g, 'r' || g::STRING, ('2026-01-01'::DATE + (g - 1)::INT), ('2026-01-01 10:00:00+00'::TIMESTAMPTZ + ((g - 1) || ' days')::INTERVAL) FROM generate_series(1, 10) AS g",
    ] {
        a.batch_execute(sql).unwrap();
    }
}

#[test]
fn lo_que_no_es_un_rango_por_posicion_valido_se_dice_antes_de_ir_al_servidor_o_con_su_motivo() {
    let _g = UNO_A_UNO.lock().unwrap_or_else(|e| e.into_inner());
    for url in servidores() {
        preparar_inc(&url);
        let t = ahora(&url);
        let error = |objeto: &str, extra: String| filas(&url, objeto, &extra).unwrap_err();
        // Un instante que no lo es.
        for malo in ["ayer", "2026-10-29 10:00:00", "-5", "1e9"] {
            let f = error("public.lab_inc", posicion(Some(malo), None));
            assert_eq!(f.codigo, Codigo::Operador, "{url} {malo}: {f:?}");
            assert!(
                f.mensaje.contains("testigo"),
                "{url}: el mensaje manda al verbo que lo da: {}",
                f.mensaje
            );
        }
        // El futuro: cada versión de CockroachDB lo dice distinto; aquí, igual y con el motivo.
        let ns: u128 = t.split('.').next().unwrap().parse().unwrap();
        let futuro = format!("{}.0000000000", ns + 3_600_000_000_000);
        let f = error("public.lab_inc", posicion(None, Some(&futuro)));
        assert_eq!(f.codigo, Codigo::Operador, "{url}: {f:?}");
        assert!(f.mensaje.contains("del futuro"), "{url}: {}", f.mensaje);
        // Una vista no guarda cuándo se escribió cada fila.
        let f = error("public.lab_inc_v", posicion(Some(&t), None));
        assert_eq!(f.codigo, Codigo::Operador, "{url}: {f:?}");
        assert!(
            f.mensaje.contains("no es una tabla"),
            "{url}: {}",
            f.mensaje
        );
        // …pero con un `cursor` sobre una columna sí se puede leer por rangos.
        assert_eq!(
            ids(&filas(&url, "public.lab_inc_v", &rango("id", Some("8"), None)).unwrap()),
            vec![9, 10],
            "{url}"
        );
        // Una tabla que no está sigue siendo `objeto`, también con rango.
        let f = error("public.no_existe_esta", posicion(Some(&t), None));
        assert_eq!(f.codigo, Codigo::Objeto, "{url}: {f:?}");
    }
}

#[test]
fn el_rango_por_posicion_en_una_tabla_grande_hash_sharded_da_lo_mismo_en_tramos_y_con_una_conexion()
{
    let _g = UNO_A_UNO.lock().unwrap_or_else(|e| e.into_inner());
    for url in servidores() {
        let hay: bool = Cockroach::conectar(&url)
            .unwrap()
            .cliente
            .query_one("SELECT to_regclass('public.lab_hash') IS NOT NULL", &[])
            .unwrap()
            .get(0);
        if !hay {
            eprintln!("{url}: sin `lab_hash` (corre `--test reparto` antes): se salta");
            continue;
        }
        let mut w = escritura(&url);
        let t0 = ahora(&url);
        // 1 000 filas cambian, entre las 250 000: las demás no se tocan.
        w.batch_execute("UPDATE lab_hash SET texto = texto || 'x' WHERE id <= 1000")
            .unwrap();
        let t1 = ahora(&url);
        w.batch_execute("UPDATE lab_hash SET texto = 'fila-' || id::STRING WHERE id <= 1000")
            .unwrap();

        // `lab_hash` tiene `texto`, no `v`: se proyecta `texto` como `v`.
        let leer = |u: &str| -> Vec<i64> {
            let mut s = Cockroach::conectar(u).unwrap();
            let p = ore_driver::leer_peticion(&format!(
                r#"{{"url":"{}","objeto":"public.lab_hash","proyeccion":{{"id":"id"}},"formato":"arrow","start":"{t0}","end":"{t1}"}}"#,
                js(u)
            ))
            .unwrap();
            let mut salida = Vec::new();
            Cockroach::leer(&mut s, &Token::default(), &p, &mut salida)
                .unwrap_or_else(|f| panic!("{u}: {f:?}"));
            let mut v: Vec<i64> = StreamReader::try_new(&salida[..], None)
                .unwrap()
                .flat_map(|b| {
                    b.unwrap()
                        .column(0)
                        .as_primitive::<Int64Type>()
                        .values()
                        .to_vec()
                })
                .collect();
            v.sort_unstable();
            v
        };
        let (en_tramos, con_una) = (leer(&url), leer(&format!("{url}?tramos=1")));
        assert_eq!(
            en_tramos,
            (1..=1000).collect::<Vec<_>>(),
            "{url}: solo las 1 000 que cambiaron entre t0 y t1"
        );
        assert_eq!(en_tramos, con_una, "{url}");
    }
}
