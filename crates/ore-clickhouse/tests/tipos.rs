//! Los tipos de ClickHouse de verdad (ADR 0077 §6, paso 4): se crea una tabla con ellos, se lee con `leer` y se comparan los
//! valores del flujo Arrow con los que se insertaron. Se salta sola si no hay laboratorio (`CLICKHOUSE_LAB_URLS`).

use arrow_array::cast::AsArray;
use arrow_array::types::{Decimal128Type, TimestampMicrosecondType};
use arrow_array::{Array, RecordBatch};
use arrow_ipc::reader::StreamReader;
use ore_clickhouse::ClickHouse;
use ore_clickhouse::lectura::Token;
use ore_clickhouse::transporte::Cliente;
use ore_driver::Codigo;
use ore_driver::origen::Origen;

fn servidores() -> Vec<String> {
    match std::env::var("CLICKHOUSE_LAB_URLS") {
        Ok(v) if !v.trim().is_empty() => v.split(',').map(|s| s.trim().to_string()).collect(),
        _ => {
            eprintln!("sin CLICKHOUSE_LAB_URLS: no hay laboratorio, se salta");
            Vec::new()
        }
    }
}

fn leer(url: &str, objeto: &str, proy: &str) -> Result<RecordBatch, ore_driver::Fallo> {
    let p = ore_driver::leer_peticion(&format!(
        r#"{{"url":"{url}","objeto":"{objeto}","proyeccion":{proy},"formato":"arrow"}}"#
    ))
    .expect("petición");
    let mut s = ClickHouse::conectar(url)?;
    let mut salida = Vec::new();
    ClickHouse::leer(&mut s, &Token::default(), &p, &mut salida)?;
    let lotes: Vec<RecordBatch> = StreamReader::try_new(&salida[..], None)
        .expect("flujo Arrow")
        .map(|l| l.expect("lote"))
        .collect();
    assert_eq!(lotes.len(), 1, "{url}: un lote");
    Ok(lotes.into_iter().next().unwrap())
}

#[test]
fn los_tipos_que_se_leen_dan_los_valores_que_se_insertaron() {
    for url in servidores() {
        let c = Cliente::abrir(&url).unwrap();
        c.ejecutar("DROP TABLE IF EXISTS kit.lab_tipos4").unwrap();
        c.ejecutar(
            "CREATE TABLE kit.lab_tipos4 (id Int64, u UInt64, e Enum8('a' = 1, 'b, (c)' = 2), f FixedString(4), \
             g UUID, v4 IPv4, v6 IPv6, lc LowCardinality(String), n Nullable(UInt64), d DateTime('Europe/Madrid'), \
             dt DateTime64(3), t UInt8) ENGINE = MergeTree ORDER BY id",
        )
        .unwrap();
        c.ejecutar(
            "INSERT INTO kit.lab_tipos4 VALUES (1, 18446744073709551615, 'b, (c)', 'ab', \
             '61f0c404-5cb3-11e7-907b-a6006ad3dba0', '1.2.3.4', '2001:db8::1', 'x', NULL, \
             '2026-10-04 10:00:00', '2026-10-04 10:00:00.123', 200)",
        )
        .unwrap();
        let b = leer(
            &url,
            "kit.lab_tipos4",
            r#"{"u":"u","e":"e","f":"f","g":"g","v4":"v4","v6":"v6","lc":"lc","n":"n","d":"d","dt":"dt","t":"t"}"#,
        )
        .unwrap_or_else(|f| panic!("{url}: {f:?}"));
        assert_eq!(b.num_rows(), 1);
        let texto = |i: usize| b.column(i).as_string::<i32>().value(0).to_string();
        assert_eq!(
            b.column(0).as_primitive::<Decimal128Type>().value(0),
            u64::MAX as i128,
            "{url}"
        );
        assert_eq!(texto(1), "b, (c)", "{url}");
        assert_eq!(texto(2), "ab", "{url}");
        assert_eq!(texto(3), "61f0c404-5cb3-11e7-907b-a6006ad3dba0", "{url}");
        assert_eq!(texto(4), "1.2.3.4", "{url}");
        assert_eq!(texto(5), "2001:db8::1", "{url}");
        assert_eq!(texto(6), "x", "{url}");
        assert!(b.column(7).is_null(0), "{url}");
        // 10:00:00 en Madrid (UTC+2 en octubre) son las 08:00 UTC.
        assert_eq!(
            b.column(8)
                .as_primitive::<TimestampMicrosecondType>()
                .value(0),
            1_791_100_800_000_000,
            "{url}"
        );
        assert_eq!(
            b.column(9)
                .as_primitive::<TimestampMicrosecondType>()
                .value(0),
            1_791_108_000_123_000,
            "{url}"
        );
        c.ejecutar("DROP TABLE kit.lab_tipos4").unwrap();
    }
}

#[test]
fn lo_que_no_cabe_se_rechaza_nombrando_el_tipo_y_no_se_redondea() {
    for url in servidores() {
        let c = Cliente::abrir(&url).unwrap();
        c.ejecutar("DROP TABLE IF EXISTS kit.lab_grandes4").unwrap();
        c.ejecutar(
            "CREATE TABLE kit.lab_grandes4 (id Int64, i Int128, m Map(String, Int64), a Array(Int64)) \
             ENGINE = MergeTree ORDER BY id",
        )
        .unwrap();
        for (col, tipo) in [("i", "Int128"), ("m", "Map"), ("a", "Array")] {
            let f = leer(&url, "kit.lab_grandes4", &format!(r#"{{"x":"{col}"}}"#)).expect_err(tipo);
            assert_eq!(f.codigo, Codigo::Esquema, "{url} {tipo}: {f:?}");
            assert!(f.mensaje.contains(tipo), "{url}: {f:?}");
        }
        c.ejecutar("DROP TABLE kit.lab_grandes4").unwrap();
    }
}
