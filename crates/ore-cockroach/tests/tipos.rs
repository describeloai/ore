//! Los tipos de CockroachDB de verdad (ADR 0080 §6, paso 2): se crea una tabla con ellos, se lee con `leer` y se comparan los valores del
//! flujo Arrow con los que se insertaron. Se salta sola si no hay laboratorio:
//!
//! ```text
//! COCKROACH_LAB_URLS=cockroachdb://kit@localhost:26257/kit,…  ORE_COCKROACH_LABORATORIO=1  cargo test -p ore-cockroach --test tipos
//! ```

use arrow_array::cast::AsArray;
use arrow_array::{Array, RecordBatch};
use arrow_cast::display::array_value_to_string;
use arrow_ipc::reader::StreamReader;
use ore_cockroach::Cockroach;
use ore_cockroach::lectura::Token;
use ore_driver::origen::Origen;

fn servidores() -> Vec<String> {
    let lab = std::env::var("ORE_COCKROACH_LABORATORIO").as_deref() == Ok("1");
    match std::env::var("COCKROACH_LAB_URLS") {
        Ok(v) if !v.trim().is_empty() && lab => {
            let urls: Vec<String> = v.split(',').map(|s| s.trim().to_string()).collect();
            // Los tests corren en paralelo y comparten la tabla: se prepara una sola vez por proceso.
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

/// `(columna, tipo, valor insertado)` de la primera fila; la segunda lleva `NULL` en todas menos en `id`.
const COLUMNAS: &[(&str, &str, &str)] = &[
    ("i2", "INT2", "7"),
    ("i4", "INT4", "-2147483648"),
    ("i8", "INT8", "9223372036854775807"),
    ("f4", "FLOAT4", "1.5"),
    ("f8", "FLOAT8", "-1e10"),
    ("dec", "DECIMAL", "123456789.123456789012345"),
    ("dec_ps", "DECIMAL(10,2)", "12.34"),
    (
        "dec_grande",
        "DECIMAL(40,2)",
        "12345678901234567890123456789012345678.12",
    ),
    ("txt", "STRING", "'hola, \"mundo\"'"),
    ("vacio", "STRING", "''"),
    ("byt", "BYTES", "'\\xdeadbeef'"),
    ("bool", "BOOL", "true"),
    ("fecha", "DATE", "'2026-10-29'"),
    ("hora", "TIME", "'12:34:56.789'"),
    ("horatz", "TIMETZ", "'12:34:56+02'"),
    ("ts", "TIMESTAMP", "'2026-10-29 12:34:56.123456'"),
    ("tstz", "TIMESTAMPTZ", "'2026-10-29 12:34:56.123456+02'"),
    ("intervalo", "INTERVAL", "'1 year 2 mons 3 days 04:05:06'"),
    ("uuid", "UUID", "'61f0c404-5cb3-11e7-907b-a6006ad3dba0'"),
    ("json", "JSONB", "'{\"a\":[1,2,{\"b\":null}]}'"),
    ("inet", "INET", "'192.168.1.1/24'"),
    ("bits", "BIT(5)", "B'10101'"),
    ("arr_int", "INT8[]", "ARRAY[1,2,NULL,4]"),
    ("arr_txt", "STRING[]", "ARRAY['a','b,c','d\"e']"),
    ("arr_vacio", "INT8[]", "ARRAY[]::INT8[]"),
    ("estado", "ore_estado", "'activo'"),
    ("geog", "GEOGRAPHY", "'POINT(1 2)'"),
    ("geom", "GEOMETRY", "'POINT(1 2)'"),
    ("virtual", "INT8 AS (i2 * 2) VIRTUAL", ""),
];

fn leer(url: &str, objeto: &str, columnas: &[&str]) -> Result<RecordBatch, ore_driver::Fallo> {
    let proy = columnas
        .iter()
        .map(|c| format!("\"{c}\":\"{c}\""))
        .collect::<Vec<_>>()
        .join(",");
    let p = ore_driver::leer_peticion(&format!(
        r#"{{"url":"{url}","objeto":"{objeto}","proyeccion":{{{proy}}},"formato":"arrow"}}"#
    ))
    .expect("petición");
    let mut s = Cockroach::conectar(url)?;
    let mut salida = Vec::new();
    Cockroach::leer(&mut s, &Token::default(), &p, &mut salida)?;
    let lotes: Vec<RecordBatch> = StreamReader::try_new(&salida[..], None)
        .expect("flujo Arrow")
        .map(|l| l.expect("lote"))
        .collect();
    assert_eq!(lotes.len(), 1, "{url}: un lote");
    Ok(lotes.into_iter().next().unwrap())
}

fn preparar(url: &str) {
    let root = url.replace("//kit@", "//root@");
    let mut a = ore_cockroach::tls::abrir(&root).unwrap_or_else(|f| panic!("{root}: {f:?}"));
    a.cliente
        .batch_execute("SET default_transaction_read_only = false")
        .unwrap();
    for sql in [
        "DROP TABLE IF EXISTS lab_tipos".to_string(),
        "DROP TYPE IF EXISTS ore_estado".to_string(),
        "CREATE TYPE ore_estado AS ENUM ('activo', 'inactivo')".to_string(),
        format!(
            "CREATE TABLE lab_tipos (id INT8 PRIMARY KEY, {})",
            COLUMNAS
                .iter()
                .map(|(c, t, _)| format!("{c} {t}"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        format!(
            "INSERT INTO lab_tipos (id, {}) VALUES (1, {})",
            COLUMNAS
                .iter()
                .filter(|(_, _, v)| !v.is_empty())
                .map(|(c, _, _)| *c)
                .collect::<Vec<_>>()
                .join(", "),
            COLUMNAS
                .iter()
                .filter(|(_, _, v)| !v.is_empty())
                .map(|(_, _, v)| *v)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        "INSERT INTO lab_tipos (id) VALUES (2)".to_string(),
        "GRANT SELECT ON TABLE lab_tipos TO kit".to_string(),
    ] {
        a.cliente
            .batch_execute(&sql)
            .unwrap_or_else(|e| panic!("{url}: {sql}: {}", ore_cockroach::tls::causa(&e)));
    }
}

/// `(columna, tipo de Arrow, valor de la primera fila)`, comprobado a mano contra 22.2, 24.3 y 25.4. Todo lo que no es exacto (decimal sin
/// precisión o de más de 38 cifras, intervalo, JSON, inet, bits, geografía…) sale como su texto.
const ESPERADO: &[(&str, &str, &str)] = &[
    ("id", "Int64", "1"),
    ("i2", "Int64", "7"),
    ("i4", "Int64", "-2147483648"),
    ("i8", "Int64", "9223372036854775807"),
    ("f4", "Float64", "1.5"),
    ("f8", "Float64", "-10000000000.0"),
    ("dec", "Utf8", "123456789.123456789012345"),
    ("dec_ps", "Decimal128(10, 2)", "12.34"),
    (
        "dec_grande",
        "Utf8",
        "12345678901234567890123456789012345678.12",
    ),
    ("txt", "Utf8", "hola, \"mundo\""),
    ("vacio", "Utf8", ""),
    ("byt", "Binary", "deadbeef"),
    ("bool", "Boolean", "true"),
    ("fecha", "Date32", "2026-10-29"),
    ("hora", "Time64(Microsecond)", "12:34:56.789"),
    (
        "ts",
        "Timestamp(Microsecond, None)",
        "2026-10-29T12:34:56.123456",
    ),
    // `+02` son las 12:34 en Madrid: el instante es 10:34 UTC.
    (
        "tstz",
        "Timestamp(Microsecond, Some(\"+00:00\"))",
        "2026-10-29T10:34:56.123456Z",
    ),
    ("intervalo", "Utf8", "1 year 2 mons 3 days 04:05:06"),
    ("uuid", "Utf8", "61f0c404-5cb3-11e7-907b-a6006ad3dba0"),
    ("json", "Utf8", "{\"a\": [1, 2, {\"b\": null}]}"),
    ("inet", "Utf8", "192.168.1.1/24"),
    ("bits", "Utf8", "10101"),
    ("estado", "Utf8", "activo"),
    // EWKB en hexadecimal, tal cual lo escribe CockroachDB; la geometría nativa es de un paso posterior (ADR 0080 §3.1 D).
    (
        "geog",
        "Utf8",
        "0101000020E6100000000000000000F03F0000000000000040",
    ),
    ("geom", "Utf8", "0101000000000000000000F03F0000000000000040"),
    ("virtual", "Int64", "14"),
];

#[test]
fn los_tipos_que_se_leen_dan_los_valores_que_se_insertaron() {
    for url in servidores() {
        let mut nombres: Vec<&str> = ESPERADO.iter().map(|(c, _, _)| *c).collect();
        nombres.extend(["horatz", "arr_int", "arr_txt", "arr_vacio"]);
        let b = leer(&url, "public.lab_tipos", &nombres).unwrap_or_else(|f| panic!("{url}: {f:?}"));
        assert_eq!(b.num_rows(), 2, "{url}");
        for (i, (nombre, tipo, valor)) in ESPERADO.iter().enumerate() {
            let col = b.column(i);
            assert_eq!(format!("{:?}", col.data_type()), *tipo, "{url}: {nombre}");
            assert_eq!(
                array_value_to_string(col, 0).unwrap(),
                *valor,
                "{url}: {nombre}"
            );
            // La segunda fila lleva nulo en todo menos en `id`: el nulo no es la cadena vacía ni el cero.
            assert_eq!(
                col.is_null(1),
                *nombre != "id",
                "{url}: {nombre} en la fila nula"
            );
        }
        assert!(
            !b.column(10).is_null(0),
            "{url}: la cadena vacía no es nulo"
        );

        // `TIMETZ` sale como texto y **cambia de una versión a otra**: `12:34:56+02:00:00` en 22.2, `12:34:56+02` desde 24.3.
        let c = b.column(ESPERADO.len());
        let tz = array_value_to_string(c, 0).unwrap();
        assert!(tz.starts_with("12:34:56+02"), "{url}: {tz}");

        // Los arrays de un escalar exacto son listas de Arrow, con sus nulos.
        let lista = |n: usize| b.column(ESPERADO.len() + n).as_list::<i32>().value(0);
        let enteros = lista(1);
        let enteros = enteros.as_primitive::<arrow_array::types::Int64Type>();
        assert_eq!(enteros.len(), 4, "{url}");
        assert_eq!(
            (enteros.value(0), enteros.value(1), enteros.value(3)),
            (1, 2, 4),
            "{url}"
        );
        assert!(enteros.is_null(2), "{url}: el NULL de dentro del array");
        let textos = lista(2);
        let textos = textos.as_string::<i32>();
        assert_eq!(
            (textos.value(0), textos.value(1), textos.value(2)),
            ("a", "b,c", "d\"e"),
            "{url}"
        );
        assert_eq!(lista(3).len(), 0, "{url}: un array vacío no es nulo");
    }
}

#[test]
fn una_columna_que_no_existe_es_un_cambio_de_esquema_y_una_tabla_que_no_existe_es_un_objeto() {
    for url in servidores() {
        let f = match leer(&url, "public.lab_tipos", &["id", "ya_no_esta"]) {
            Err(f) => f,
            Ok(_) => panic!("{url}: leyó una columna que no existe"),
        };
        assert_eq!(f.codigo, ore_driver::Codigo::Esquema, "{url}: {f:?}");
        let f = match leer(&url, "public.no_existe_esta", &["id"]) {
            Err(f) => f,
            Ok(_) => panic!("{url}: leyó una tabla que no existe"),
        };
        assert_eq!(f.codigo, ore_driver::Codigo::Objeto, "{url}: {f:?}");
    }
}

#[test]
fn la_lectura_no_escribe_y_la_sesion_sigue_sirviendo_despues_de_un_fallo() {
    for url in servidores() {
        let mut s = Cockroach::conectar(&url).unwrap_or_else(|f| panic!("{url}: {f:?}"));
        let p = |objeto: &str| {
            ore_driver::leer_peticion(&format!(
                r#"{{"url":"{url}","objeto":"{objeto}","proyeccion":{{"id":"id"}},"formato":"arrow"}}"#
            ))
            .expect("petición")
        };
        // Un fallo no deja la sesión en medio de una transacción: la siguiente lectura (servir) funciona.
        assert!(
            Cockroach::leer(
                &mut s,
                &Token::default(),
                &p("public.no_existe_esta"),
                &mut Vec::new()
            )
            .is_err()
        );
        let n = Cockroach::leer(
            &mut s,
            &Token::default(),
            &p("public.lab_tipos"),
            &mut Vec::new(),
        )
        .unwrap_or_else(|f| panic!("{url}: {f:?}"));
        assert_eq!(n, 2, "{url}");
    }
}
