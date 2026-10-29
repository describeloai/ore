//! Filtros, orden y límite de CockroachDB de verdad (ADR 0080 §6, paso 4): lo que el kit no alcanza. Se salta sola si no hay laboratorio:
//!
//! ```text
//! COCKROACH_LAB_URLS=cockroachdb://kit@localhost:26257/kit,…  ORE_COCKROACH_LABORATORIO=1  cargo test -p ore-cockroach --test filtros
//! ```
//!
//! Los valores llevan comillas, barras y trozos de SQL: un valor que se pegara a la consulta sin atar diría otra cosa. Las tablas grandes
//! (`lab_reparto`) son las de `tests/reparto.rs`; si no están, esas pruebas se saltan.

use arrow_array::RecordBatch;
use arrow_array::cast::AsArray;
use arrow_array::types::Int64Type;
use arrow_ipc::reader::StreamReader;
use ore_cockroach::Cockroach;
use ore_cockroach::lectura::Token;
use ore_driver::origen::Origen;
use ore_driver::{Codigo, Fallo};

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

fn preparar(url: &str) {
    let root = url.replace("//kit@", "//root@");
    let mut a = ore_cockroach::tls::abrir(&root)
        .unwrap_or_else(|f| panic!("{root}: {f:?}"))
        .cliente;
    a.batch_execute("SET default_transaction_read_only = false")
        .unwrap();
    let sqls = [
        "DROP TABLE IF EXISTS lab_filtros",
        "DROP TYPE IF EXISTS ore_semaforo",
        "CREATE TYPE ore_semaforo AS ENUM ('rojo', 'verde')",
        "CREATE TABLE lab_filtros (id INT8 PRIMARY KEY, texto STRING, n INT8, d DATE, ts TIMESTAMPTZ, b BYTES, u UUID, e ore_semaforo, \
         dec DECIMAL(10,2), f FLOAT8, bo BOOL)",
        // Los valores, uno por línea; la 9 es todo nulo.
        r#"INSERT INTO lab_filtros VALUES
 (1, 'ana',   10, '2026-01-01', '2026-01-01 10:00:00+00', '\xdeadbeef', '61f0c404-5cb3-11e7-907b-a6006ad3dba0', 'rojo',  1.50, 0.5,  true),
 (2, 'Ana',   20, '2026-06-15', '2026-06-15 12:00:00+02', '\x00',       '00000000-0000-4000-8000-000000000002', 'verde', 2.50, 1.5,  false),
 (3, 'o''brien', 30, '2026-12-31', '2026-12-31 23:59:59.999999+00', '\xff', '00000000-0000-4000-8000-000000000003', 'rojo', 3.50, -2.0, true),
 (4, 'a\b',   40, '2027-01-01', '2027-01-01 00:00:00+00', '\x01',       '00000000-0000-4000-8000-000000000004', 'verde', 4.50, 2.5,  false),
 (5, 'x''; DROP TABLE lab_filtros; --', 50, '2027-02-02', '2027-02-02 02:02:02+00', '\x02', '00000000-0000-4000-8000-000000000005', 'rojo', 5.50, 3.5, true),
 (6, '100%',  60, '2027-03-03', '2027-03-03 03:03:03+00', '\x03',       '00000000-0000-4000-8000-000000000006', 'verde', 6.50, 4.5,  false),
 (7, '',      70, '2027-04-04', '2027-04-04 04:04:04+00', '\x04',       '00000000-0000-4000-8000-000000000007', 'rojo',  7.50, 5.5,  true),
 (8, 'éñü',   80, '2027-05-05', '2027-05-05 05:05:05+00', '\x05',       '00000000-0000-4000-8000-000000000008', 'verde', 8.50, 6.5,  false),
 (9, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL)"#,
        "GRANT SELECT ON TABLE lab_filtros TO kit",
    ];
    for sql in sqls {
        a.batch_execute(sql)
            .unwrap_or_else(|e| panic!("{url}: {sql}: {}", ore_cockroach::tls::causa(&e)));
    }
}

/// Un valor, como va dentro de un JSON.
fn js(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn filtro(columna: &str, operador: &str, valor: &str) -> String {
    format!(
        r#"{{"columna":"{columna}","operador":"{operador}","valor":{}}}"#,
        js(valor)
    )
}

fn lista(columna: &str, valores: &[&str]) -> String {
    format!(
        r#"{{"columna":"{columna}","operador":"in","valor":[{}]}}"#,
        valores.iter().map(|v| js(v)).collect::<Vec<_>>().join(",")
    )
}

fn sin_valor(columna: &str, operador: &str) -> String {
    format!(r#"{{"columna":"{columna}","operador":"{operador}"}}"#)
}

/// Lee `id` de `objeto` con estos filtros (y `extra`, que va tal cual en el JSON), **en el orden en que llegan**.
fn ids_de(url: &str, objeto: &str, filtros: &[String], extra: &str) -> Result<Vec<i64>, Fallo> {
    let mut s = Cockroach::conectar(url)?;
    let p = ore_driver::leer_peticion(&format!(
        r#"{{"url":"{}","objeto":"{objeto}","proyeccion":{{"id":"id"}},"formato":"arrow","filtros":[{}]{extra}}}"#,
        js(url).trim_matches('"'),
        filtros.join(",")
    ))
    .expect("petición");
    let mut salida = Vec::new();
    Cockroach::leer(&mut s, &Token::default(), &p, &mut salida)?;
    let lotes: Vec<RecordBatch> = StreamReader::try_new(&salida[..], None)
        .expect("flujo Arrow")
        .map(|l| l.expect("lote"))
        .collect();
    Ok(lotes
        .iter()
        .flat_map(|b| b.column(0).as_primitive::<Int64Type>().values().to_vec())
        .collect())
}

fn con(url: &str, filtros: &[String]) -> Vec<i64> {
    let mut v = ids_de(url, "public.lab_filtros", filtros, "")
        .unwrap_or_else(|f| panic!("{url} {filtros:?}: {f:?}"));
    v.sort_unstable();
    v
}

#[test]
fn un_valor_con_comillas_barras_o_sql_es_un_valor_y_nada_mas() {
    for url in servidores() {
        assert_eq!(
            con(&url, &[filtro("texto", "eq", "o'brien")]),
            vec![3],
            "{url}"
        );
        assert_eq!(
            con(&url, &[filtro("texto", "eq", "a\\b")]),
            vec![4],
            "{url}: una barra es una barra"
        );
        assert_eq!(
            con(
                &url,
                &[filtro("texto", "eq", "x'; DROP TABLE lab_filtros; --")]
            ),
            vec![5],
            "{url}"
        );
        assert_eq!(con(&url, &[filtro("texto", "eq", "éñü")]), vec![8], "{url}");
        assert_eq!(
            con(&url, &[filtro("texto", "eq", "")]),
            vec![7],
            "{url}: la cadena vacía no es nulo"
        );
        assert_eq!(
            con(&url, &[filtro("texto", "like", "%'%")]),
            vec![3, 5],
            "{url}"
        );
        assert_eq!(
            con(&url, &[filtro("texto", "like", "100%")]),
            vec![6],
            "{url}"
        );
        assert_eq!(
            con(&url, &[filtro("texto", "like", "_na")]),
            vec![1, 2].into_iter().filter(|_| true).collect::<Vec<_>>(),
            "{url}"
        );
        assert_eq!(
            con(&url, &[filtro("texto", "neq", "ana")]),
            vec![2, 3, 4, 5, 6, 7, 8],
            "{url}: `neq` no trae los nulos"
        );
        // Tras todo eso, la tabla sigue ahí.
        assert_eq!(con(&url, &[]).len(), 9, "{url}: la tabla ya no está entera");
    }
}

#[test]
fn los_tipos_que_el_kit_no_toca_se_comparan_como_son() {
    for url in servidores() {
        // BYTES: llega en base64 (`3q2+7w==` son `de ad be ef`) y se compara como bytes.
        assert_eq!(
            con(&url, &[filtro("b", "eq", "3q2+7w==")]),
            vec![1],
            "{url}"
        );
        assert_eq!(
            con(&url, &[lista("b", &["3q2+7w==", "AA=="])]),
            vec![1, 2],
            "{url}"
        );
        // UUID, ENUM, BOOL, DECIMAL, FLOAT.
        assert_eq!(
            con(
                &url,
                &[filtro("u", "eq", "61f0c404-5cb3-11e7-907b-a6006ad3dba0")]
            ),
            vec![1],
            "{url}"
        );
        assert_eq!(
            con(&url, &[filtro("e", "eq", "verde")]),
            vec![2, 4, 6, 8],
            "{url}"
        );
        assert_eq!(
            con(&url, &[filtro("bo", "eq", "true")]),
            vec![1, 3, 5, 7],
            "{url}"
        );
        assert_eq!(
            con(&url, &[filtro("dec", "gt", "5.50")]),
            vec![6, 7, 8],
            "{url}"
        );
        assert_eq!(
            con(&url, &[filtro("dec", "eq", "1.5")]),
            vec![1],
            "{url}: 1.5 es 1.50"
        );
        assert_eq!(con(&url, &[filtro("f", "lt", "0")]), vec![3], "{url}");
        // DATE y TIMESTAMPTZ: el instante se compara como instante, con su zona.
        assert_eq!(
            con(
                &url,
                &[
                    filtro("d", "ge", "2026-12-31"),
                    filtro("d", "lt", "2027-03-03")
                ]
            ),
            vec![3, 4, 5],
            "{url}"
        );
        assert_eq!(
            con(&url, &[filtro("ts", "eq", "2026-06-15 10:00:00+00")]),
            vec![2],
            "{url}: 12:00+02 son las 10:00 UTC"
        );
        assert_eq!(
            con(&url, &[filtro("ts", "gt", "2026-12-31 23:59:59+00")]),
            vec![3, 4, 5, 6, 7, 8],
            "{url}"
        );
        // Nulos.
        assert_eq!(con(&url, &[sin_valor("texto", "isNull")]), vec![9], "{url}");
        assert_eq!(
            con(&url, &[sin_valor("n", "isNotNull")]),
            vec![1, 2, 3, 4, 5, 6, 7, 8],
            "{url}"
        );
        // `in` con muchos valores, y con ninguno.
        let muchos: Vec<String> = (1..=1000).map(|i| i.to_string()).collect();
        let muchos: Vec<&str> = muchos.iter().map(String::as_str).collect();
        assert_eq!(
            con(&url, &[lista("id", &muchos)]),
            (1..=9).collect::<Vec<_>>(),
            "{url}"
        );
        assert_eq!(
            con(&url, &[lista("id", &[])]),
            Vec::<i64>::new(),
            "{url}: `in ()` no trae nada"
        );
    }
}

#[test]
fn un_filtro_por_bytes_que_no_es_base64_es_un_defecto_de_quien_pide() {
    for url in servidores() {
        let f = ids_de(
            &url,
            "public.lab_filtros",
            &[filtro("b", "eq", "esto no es base64!")],
            "",
        )
        .unwrap_err();
        assert_eq!(f.codigo, Codigo::Operador, "{url}: {f:?}");
    }
}

#[test]
fn varios_filtros_se_juntan_con_y_y_una_columna_fuera_de_la_proyeccion_filtra() {
    for url in servidores() {
        // `n` no está en la proyección (solo sale `id`).
        assert_eq!(
            con(
                &url,
                &[
                    filtro("n", "ge", "30"),
                    filtro("n", "le", "60"),
                    filtro("e", "eq", "rojo")
                ]
            ),
            vec![3, 5],
            "{url}"
        );
        assert_eq!(
            con(&url, &[filtro("n", "gt", "1000")]),
            Vec::<i64>::new(),
            "{url}"
        );
    }
}

#[test]
fn el_orden_y_el_limite_se_empujan_y_los_nulos_van_al_final() {
    for url in servidores() {
        let leer = |extra: &str| {
            ids_de(&url, "public.lab_filtros", &[], extra)
                .unwrap_or_else(|f| panic!("{url} {extra}: {f:?}"))
        };
        assert_eq!(
            leer(r#","orderBy":[{"columna":"n","direccion":"asc"}]"#),
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9],
            "{url}: nulos al final"
        );
        assert_eq!(
            leer(r#","orderBy":[{"columna":"n","direccion":"desc"}]"#),
            vec![8, 7, 6, 5, 4, 3, 2, 1, 9],
            "{url}: nulos al final también en desc"
        );
        assert_eq!(
            leer(r#","orderBy":[{"columna":"id","direccion":"desc"}],"limit":3"#),
            vec![9, 8, 7],
            "{url}"
        );
        assert_eq!(
            leer(
                r#","orderBy":[{"columna":"bo","direccion":"asc"},{"columna":"id","direccion":"desc"}]"#
            ),
            vec![8, 6, 4, 2, 7, 5, 3, 1, 9],
            "{url}"
        );
        assert_eq!(leer(r#","limit":0"#).len(), 0, "{url}: límite 0");
        assert_eq!(
            leer(r#","limit":100"#).len(),
            9,
            "{url}: límite mayor que la tabla"
        );
        // Texto: el orden de bytes, sin colación. 'A' < 'a' < 'é'.
        let por_texto = leer(r#","orderBy":[{"columna":"texto","direccion":"asc"}],"limit":3"#);
        assert_eq!(por_texto, vec![7, 6, 2], "{url}: '' < '100%' < 'Ana'");
    }
}

#[test]
fn una_columna_que_no_esta_en_un_filtro_o_en_el_orden_es_un_cambio_de_esquema() {
    for url in servidores() {
        let f = ids_de(
            &url,
            "public.lab_filtros",
            &[filtro("ya_no_esta", "eq", "1")],
            "",
        )
        .unwrap_err();
        assert_eq!(f.codigo, Codigo::Esquema, "{url}: {f:?}");
        let f = ids_de(
            &url,
            "public.lab_filtros",
            &[],
            r#","orderBy":[{"columna":"ya_no_esta","direccion":"asc"}]"#,
        )
        .unwrap_err();
        assert_eq!(f.codigo, Codigo::Esquema, "{url}: {f:?}");
    }
}

#[test]
fn los_filtros_se_combinan_con_la_lectura_en_tramos_y_el_limite_la_apaga() {
    for url in servidores() {
        // `lab_reparto` (300 000 filas, clave `id`) la deja `tests/reparto.rs`.
        let existe = ids_de(&url, "public.lab_reparto", &[], r#","limit":1"#).is_ok();
        if !existe {
            eprintln!("{url}: sin `lab_reparto` (corre `--test reparto` antes): se salta");
            continue;
        }
        let rango = [filtro("id", "ge", "100000"), filtro("id", "le", "200000")];
        let mut en_tramos = ids_de(&url, "public.lab_reparto", &rango, "").unwrap();
        let mut con_una =
            ids_de(&format!("{url}?tramos=1"), "public.lab_reparto", &rango, "").unwrap();
        en_tramos.sort_unstable();
        con_una.sort_unstable();
        assert_eq!(en_tramos.len(), 100_001, "{url}");
        assert_eq!(
            en_tramos, con_una,
            "{url}: en tramos y con una conexión, las mismas filas"
        );
        assert_eq!(
            (en_tramos[0], *en_tramos.last().unwrap()),
            (100_000, 200_000),
            "{url}"
        );
        // Un filtro que no es de la clave: cada trozo lo lleva también.
        let multiplos = ids_de(
            &url,
            "public.lab_reparto",
            &[filtro("texto", "like", "fila-1_")],
            "",
        )
        .unwrap();
        assert_eq!(multiplos.len(), 10, "{url}: fila-10 … fila-19");
        // Con límite y orden se lee en orden, con una conexión.
        let primeras = ids_de(
            &url,
            "public.lab_reparto",
            &[],
            r#","orderBy":[{"columna":"id","direccion":"asc"}],"limit":5"#,
        )
        .unwrap();
        assert_eq!(primeras, vec![1, 2, 3, 4, 5], "{url}");
        let ultimas = ids_de(
            &url,
            "public.lab_reparto",
            &[],
            r#","orderBy":[{"columna":"id","direccion":"desc"}],"limit":3"#,
        )
        .unwrap();
        assert_eq!(ultimas, vec![300_000, 299_999, 299_998], "{url}");
    }
}
