//! **El lote dorado** (ORE 0075, fase 3): lo que `carga` produce hoy, fijado antes de tocar cómo se arma.
//!
//! La fase 3 sustituye los builders de `carga::lote` por los de `ore-arrow`. Esta prueba es la puerta: cada
//! caso vuelca el lote entero (el esquema con **los metadatos de cada campo**, los valores y la huella) y lo
//! compara con lo guardado en `tests/lote_dorado/`. Si un cambio mueve una sola línea, falla.
//!
//! ⚠️ La huella **no** incluye los metadatos del campo: una marca `geoarrow.wkb` o un CRS perdidos no la
//! moverían. Por eso el volcado los escribe, y los casos de geometría los afirman además a mano, para que
//! regenerar las instantáneas sin mirar no pueda esconder esa pérdida.
//!
//! Regenerar (solo con una razón dicha en el commit): `ORE_ACTUALIZAR=1 cargo test -p ore-store --test lote_dorado`.

use arrow_array::cast::AsArray;
use arrow_array::{
    ArrayRef, Date64Array, Float32Array, Int32Array, LargeBinaryArray, LargeStringArray,
    RecordBatch, StringArray, Time32SecondArray, TimestampNanosecondArray,
};
use arrow_schema::{DataType, Field, Schema, TimeUnit};
use ore_store::carga::{self, Fila};
use std::collections::BTreeMap;
use std::fmt::Write;
use std::sync::Arc;

const MARCA: &str = "ARROW:extension:name";
const META: &str = "ARROW:extension:metadata";

fn volcado(l: &RecordBatch) -> String {
    use arrow_array::Array;
    let mut s = String::new();
    for f in l.schema().fields() {
        let mut md: Vec<_> = f.metadata().iter().collect();
        md.sort();
        writeln!(
            s,
            "campo {} : {} nullable={} metadatos={md:?}",
            f.name(),
            f.data_type(),
            f.is_nullable()
        )
        .unwrap();
    }
    writeln!(s, "filas {}", l.num_rows()).unwrap();
    for i in 0..l.num_rows() {
        let celdas: Vec<String> = l
            .columns()
            .iter()
            .map(|c| {
                if c.is_null(i) {
                    "∅".to_string()
                } else {
                    arrow_cast::display::array_value_to_string(c, i).unwrap()
                }
            })
            .collect();
        writeln!(s, "{i}: {}", celdas.join(" | ")).unwrap();
    }
    writeln!(s, "huella {}", carga::huella(std::slice::from_ref(l))).unwrap();
    s
}

/// Compara con la instantánea (o la escribe con `ORE_ACTUALIZAR=1`).
fn fijado(caso: &str, actual: &str) {
    let ruta = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/lote_dorado")
        .join(format!("{caso}.txt"));
    if std::env::var_os("ORE_ACTUALIZAR").is_some() {
        std::fs::create_dir_all(ruta.parent().unwrap()).unwrap();
        std::fs::write(&ruta, actual).unwrap();
        return;
    }
    let esperado = std::fs::read_to_string(&ruta)
        .unwrap_or_else(|e| panic!("falta {}: {e}", ruta.display()))
        .replace("\r\n", "\n");
    assert_eq!(esperado, actual, "el caso `{caso}` ya no es el de antes");
}

fn fila(celdas: &[(&str, &str)]) -> Fila {
    celdas
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn esquema(pares: &[(&str, &str)]) -> BTreeMap<String, String> {
    pares
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn marca(campo: &Field, crs_meta: &str, tipo: &str) {
    assert_eq!(campo.data_type(), &DataType::Binary, "`{}`", campo.name());
    assert_eq!(
        campo.metadata()[MARCA],
        "geoarrow.wkb",
        "`{}`",
        campo.name()
    );
    assert_eq!(campo.metadata()[META], crs_meta, "`{}`", campo.name());
    assert_eq!(campo.metadata()["ore:type"], tipo, "`{}`", campo.name());
}

const EWKB_4326: &str = "0101000020E6100000406A1327F7BB0DC053E8BCC62E454440";
const POINT_EMPTY: &str = "0101000000000000000000F87F000000000000F87F";
const POINT_1_2: &str = "0101000000000000000000F03F0000000000000040";

/// El camino del texto del driver: los diez físicos, geometría, nulos, filas vacías, lo que cae a texto.
#[test]
fn lote_desde_texto() {
    let esquema = esquema(&[
        ("b", "Binary"),
        ("d", "Decimal<12, 2>"),
        ("e", "Integer"),
        ("f", "Date"),
        ("fh", "DateTime"),
        ("g", "Geometry<Any, EPSG:4326>"),
        ("h", "Time"),
        ("i", "DateTimeTz"),
        ("l", "Boolean"),
        ("r", "Float"),
        ("s", "String"),
        // Lo que no analiza: la columna entera se queda como texto, y el informe lo dice.
        ("malo", "Integer"),
        ("gmalo", "Geometry<Any, EPSG:4326>"),
        // Una columna que ninguna fila trae.
        ("ausente", "Integer"),
        // Compuestos: el texto del driver no los estrecha.
        ("lista", "list<Integer>"),
    ]);
    let filas = vec![
        fila(&[
            ("b", "3q2+7w=="),
            ("d", "1234.56"),
            ("e", "42"),
            ("f", "2024-05-06"),
            ("fh", "2026-10-26T10:11:12.123456"),
            ("g", EWKB_4326),
            ("h", "10:11:12.5"),
            ("i", "2026-10-26T10:11:12.123456Z"),
            ("l", "true"),
            ("r", "0.5"),
            ("s", "ñandú"),
            ("malo", "1"),
            ("gmalo", "0101000000"),
            ("lista", "[1,2]"),
        ]),
        // Una fila sin nada.
        fila(&[]),
        fila(&[
            ("b", ""),
            ("d", "-0.05"),
            ("e", "-7"),
            ("g", POINT_EMPTY),
            ("i", "2026-10-26T12:11:12+02:00"),
            ("l", "false"),
            ("s", ""),
            ("malo", "abc"),
        ]),
        fila(&[
            ("d", "99999999.99"),
            ("e", "9223372036854775807"),
            ("f", "1970-01-01"),
            ("r", "-1e3"),
            ("s", "a,b \"c\""),
        ]),
    ];
    let l = carga::lote(&esquema, &filas).unwrap();
    let mut s = volcado(&l.lote);
    writeln!(s, "sin_estrechar {:?}", l.sin_estrechar).unwrap();
    fijado("lote_desde_texto", &s);

    // A mano, para que regenerar la instantánea sin mirar no esconda una pérdida.
    let sch = l.lote.schema();
    marca(
        sch.field_with_name("g").unwrap(),
        r#"{"crs":"EPSG:4326"}"#,
        "Geometry<Any, EPSG:4326>",
    );
    assert_eq!(
        sch.field_with_name("i").unwrap().data_type(),
        &DataType::Timestamp(TimeUnit::Microsecond, Some("+00:00".into())),
        "un instante lleva la zona del contrato del lago"
    );
    assert_eq!(
        sch.field_with_name("malo").unwrap().data_type(),
        &DataType::Utf8
    );
    assert_eq!(
        sch.field_with_name("gmalo").unwrap().data_type(),
        &DataType::Utf8
    );
    assert!(l.sin_estrechar.contains_key("malo") && l.sin_estrechar.contains_key("gmalo"));
}

#[test]
fn lote_sin_filas_y_sin_columnas() {
    let esquema = esquema(&[
        ("e", "Integer"),
        ("g", "Geometry<Any, EPSG:4326>"),
        ("i", "DateTimeTz"),
    ]);
    let vacio = carga::lote(&esquema, &[]).unwrap();
    let mut s = volcado(&vacio.lote);
    writeln!(s, "sin_estrechar {:?}", vacio.sin_estrechar).unwrap();
    // Hoy un esquema sin columnas no es un lote: Arrow pide columnas o un recuento de filas. Se fija tal cual.
    let nada = carga::lote(&BTreeMap::new(), &[fila(&[("e", "1")])]).err();
    writeln!(s, "-- sin columnas: {nada:?}").unwrap();
    fijado("lote_sin_filas", &s);
    marca(
        vacio.lote.schema().field_with_name("g").unwrap(),
        r#"{"crs":"EPSG:4326"}"#,
        "Geometry<Any, EPSG:4326>",
    );
}

/// El camino de lo que ya llega como Arrow (GeoPandas, DuckDB con spatial, un `write()` de otro SDK).
#[test]
fn normalizar_lo_que_llega_como_arrow() {
    let ewkb = ore_base::geo::de_hex(EWKB_4326).unwrap();
    let punto = ore_base::geo::de_hex(POINT_1_2).unwrap();
    let marcado = |n: &str, t: DataType, m: &str| {
        Field::new(n, t, true).with_metadata(
            [
                (MARCA.to_string(), "geoarrow.wkb".to_string()),
                (META.to_string(), m.to_string()),
            ]
            .into(),
        )
    };
    let projjson = r#"{"crs": {"type": "ProjectedCRS", "name": "ETRS89 / UTM zone 30N", "id": {"authority": "EPSG", "code": 25830}}}"#;
    let campos = vec![
        Field::new("id", DataType::Int32, true),
        Field::new("x", DataType::Float32, true),
        Field::new("t", DataType::LargeUtf8, true),
        Field::new(
            "ts",
            DataType::Timestamp(TimeUnit::Nanosecond, Some("Europe/Madrid".into())),
            true,
        ),
        Field::new("d64", DataType::Date64, true),
        Field::new("t32", DataType::Time32(TimeUnit::Second), true),
        // Un binario sin marca es un `Binary` cualquiera (v1alpha35).
        Field::new("bin", DataType::LargeBinary, true),
        marcado("geo_ore", DataType::LargeBinary, r#"{"crs":"EPSG:4326"}"#),
        marcado("geo_pandas", DataType::Binary, projjson),
        marcado(
            "geo_esf",
            DataType::Binary,
            r#"{"crs":"OGC:CRS84","edges":"spherical"}"#,
        ),
    ];
    let cols: Vec<ArrayRef> = vec![
        Arc::new(Int32Array::from(vec![Some(1), None])),
        Arc::new(Float32Array::from(vec![Some(0.5f32), None])),
        Arc::new(LargeStringArray::from(vec![Some("hola"), None])),
        Arc::new(
            TimestampNanosecondArray::from(vec![Some(1_792_914_255_123_456_789), None])
                .with_timezone("Europe/Madrid"),
        ),
        Arc::new(Date64Array::from(vec![Some(86_400_000), None])),
        Arc::new(Time32SecondArray::from(vec![Some(3_661), None])),
        Arc::new(LargeBinaryArray::from(vec![
            Some(&[0xde_u8, 0xad][..]),
            None,
        ])),
        Arc::new(LargeBinaryArray::from(vec![Some(ewkb.as_slice()), None])),
        Arc::new(arrow_array::BinaryArray::from(vec![
            Some(punto.as_slice()),
            None,
        ])),
        Arc::new(arrow_array::BinaryArray::from(vec![
            Some(punto.as_slice()),
            None,
        ])),
    ];
    let entrada = RecordBatch::try_new(Arc::new(Schema::new(campos)), cols).unwrap();
    let n = carga::normalizar(&entrada).unwrap();
    fijado("normalizar", &volcado(&n));

    let sch = n.schema();
    marca(
        sch.field_with_name("geo_ore").unwrap(),
        r#"{"crs":"EPSG:4326"}"#,
        "Geometry<Any, EPSG:4326>",
    );
    marca(
        sch.field_with_name("geo_pandas").unwrap(),
        r#"{"crs":"EPSG:25830"}"#,
        "Geometry<Any, EPSG:25830>",
    );
    marca(
        sch.field_with_name("geo_esf").unwrap(),
        r#"{"crs":"OGC:CRS84","edges":"spherical"}"#,
        "Geography",
    );
    assert!(
        sch.field_with_name("bin")
            .unwrap()
            .metadata()
            .get(MARCA)
            .is_none(),
        "un binario sin marca no es una geometría"
    );
    assert_eq!(
        sch.field_with_name("bin").unwrap().data_type(),
        &DataType::Binary
    );
    // Una marca sin CRS se niega.
    let sin_crs = RecordBatch::try_new(
        Arc::new(Schema::new(vec![marcado("g", DataType::Binary, "{}")])),
        vec![Arc::new(arrow_array::BinaryArray::from(vec![Some(
            punto.as_slice(),
        )]))],
    )
    .unwrap();
    assert!(carga::normalizar(&sin_crs).unwrap_err().contains("sin CRS"));
}

/// El flujo del driver en Arrow contra su contrato: la geometría entra por el códec, nunca por un `cast`.
#[test]
fn al_contrato_con_geometria_como_texto_y_como_binario() {
    let destino = Arc::new(Schema::new(vec![
        carga::campo_del_oos("n", "Integer"),
        carga::campo_del_oos("g", "Geometry<Any, EPSG:4326>"),
        carga::campo_del_oos("t", "String"),
        carga::campo_del_oos("i", "DateTimeTz"),
    ]));
    let ewkb = ore_base::geo::de_hex(EWKB_4326).unwrap();
    let entrada = |g: ArrayRef, tipo: DataType| {
        RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("n", DataType::Int32, true),
                Field::new("g", tipo, true),
                Field::new("t", DataType::LargeUtf8, true),
                Field::new(
                    "i",
                    DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
                    true,
                ),
            ])),
            vec![
                Arc::new(Int32Array::from(vec![Some(1), None])),
                g,
                Arc::new(LargeStringArray::from(vec![Some("x"), None])),
                Arc::new(
                    arrow_array::TimestampMicrosecondArray::from(vec![
                        Some(1_792_914_255_123_456),
                        None,
                    ])
                    .with_timezone("UTC"),
                ),
            ],
        )
        .unwrap()
    };
    let de_texto = carga::al_contrato(
        &entrada(
            Arc::new(StringArray::from(vec![Some(EWKB_4326), None])),
            DataType::Utf8,
        ),
        &destino,
    )
    .unwrap();
    let de_binario = carga::al_contrato(
        &entrada(
            Arc::new(arrow_array::BinaryArray::from(vec![
                Some(ewkb.as_slice()),
                None,
            ])),
            DataType::Binary,
        ),
        &destino,
    )
    .unwrap();
    let mut s = volcado(&de_texto);
    s.push_str("-- como binario\n");
    s.push_str(&volcado(&de_binario));
    // Un valor que no es una geometría niega el lote entero, con la fila.
    let roto = carga::al_contrato(
        &entrada(
            Arc::new(StringArray::from(vec![Some("0101000000"), None])),
            DataType::Utf8,
        ),
        &destino,
    )
    .unwrap_err();
    writeln!(s, "-- roto: {roto}").unwrap();
    fijado("al_contrato", &s);
    for l in [&de_texto, &de_binario] {
        marca(
            l.schema().field_with_name("g").unwrap(),
            r#"{"crs":"EPSG:4326"}"#,
            "Geometry<Any, EPSG:4326>",
        );
        assert_eq!(
            l.column_by_name("g")
                .unwrap()
                .as_binary::<i32>()
                .value(0)
                .len(),
            21
        );
    }
}
