//! **El lote dorado de objetos** (ORE 0075, fase 6): lo que `filas.rs` produce hoy, fijado antes de tocar cómo se arma.
//!
//! Se lee por el camino de siempre (`leer` sobre un origen en memoria) y se vuelca el flujo IPC tal como sale: el
//! esquema con sus metadatos, cada columna, lo que dice `Leido` (filas, ficheros, rescatados) y, si la lectura para, su
//! mensaje. Cubre los diez físicos por CSV y por JSONL, lo que no encaja con y sin `_rescued_data`, los filtros (que
//! arman sus escalares con el mismo constructor), el listado y un Parquet con partición. Se compara con
//! `tests/dorado/*.txt`; si un cambio mueve una línea, falla.
//!
//! Regenerar (solo con una razón dicha en el commit): `ORE_ACTUALIZAR=1 cargo test -p ore-read-objetos dorado`.

use super::*;
use ore_objetos::memoria::EnMemoria;
use std::fmt::Write as _;

const TIPOS: &[(&str, &str)] = &[
    ("entero", "Integer"),
    ("real", "Float"),
    ("logico", "Boolean"),
    ("decimal", "Decimal<12, 2>"),
    ("fecha", "Date"),
    ("hora", "Time"),
    ("fechahora", "DateTime"),
    ("instante", "DateTimeTz"),
    ("texto", "String"),
    ("binario", "Binary"),
];

fn peticion(tipo: &str, objeto: &str, proy: &[&str], tipos: &[(&str, &str)]) -> Peticion {
    Peticion {
        url: "s3://b".into(),
        objeto: objeto.into(),
        proyeccion: proy
            .iter()
            .map(|c| (c.to_string(), c.to_string()))
            .collect(),
        formato: Some("arrow".into()),
        fichero: Some(Fichero {
            tipo: tipo.into(),
            cabecera: true,
            separador: ',',
            tipos: tipos
                .iter()
                .map(|(c, t)| (c.to_string(), t.to_string()))
                .collect(),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Lee y vuelca: el esquema (con metadatos), cada lote y `Leido`; o el error.
fn volcar(o: &EnMemoria, p: &Peticion) -> String {
    let mut s = String::new();
    let mut buf: Vec<u8> = Vec::new();
    let l = match leer(o, p, &mut buf, UMBRAL) {
        Ok(l) => l,
        Err(e) => {
            writeln!(s, "  error: {e}").unwrap();
            return s;
        }
    };
    let r = arrow_ipc::reader::StreamReader::try_new(std::io::Cursor::new(buf), None)
        .expect("un flujo Arrow con su marca de fin");
    for f in r.schema().fields() {
        let meta: BTreeMap<_, _> = f.metadata().iter().collect();
        writeln!(
            s,
            "  campo {} {:?} nulo={} {meta:?}",
            f.name(),
            f.data_type(),
            f.is_nullable()
        )
        .unwrap();
    }
    let meta: BTreeMap<_, _> = r.schema().metadata().clone().into_iter().collect();
    writeln!(s, "  esquema {meta:?}").unwrap();
    for (i, lote) in r.enumerate() {
        let lote = lote.unwrap();
        writeln!(s, "  lote {i}: {} filas", lote.num_rows()).unwrap();
        for (f, c) in lote.schema().fields().iter().zip(lote.columns()) {
            let a = format!("{c:?}")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            writeln!(s, "    {} = {a}", f.name()).unwrap();
        }
    }
    writeln!(
        s,
        "  leido: filas {} ficheros {} rescatados {:?}",
        l.filas, l.ficheros, l.rescatados
    )
    .unwrap();
    s
}

fn fijado(caso: &str, actual: &str) {
    let ruta = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/dorado")
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

fn columnas() -> Vec<&'static str> {
    TIPOS.iter().map(|(c, _)| *c).collect()
}

fn con_rescatada() -> Vec<(&'static str, &'static str)> {
    let mut t = TIPOS.to_vec();
    t.push((RESCATADA, "String"));
    t
}

/// Una fila de CSV con el valor de `col` cambiado por `v` (los demás, correctos).
fn fila_csv(col: &str, v: &str) -> String {
    let buenos = [
        ("entero", "42"),
        ("real", "1.5"),
        ("logico", "true"),
        ("decimal", "1234.56"),
        ("fecha", "2026-09-26"),
        ("hora", "10:11:12"),
        ("fechahora", "2026-09-26 10:11:12"),
        ("instante", "2026-09-26 10:11:12+00"),
        ("texto", "hola"),
        ("binario", "aGVsbG8="),
    ];
    buenos
        .iter()
        .map(|(c, b)| if *c == col { v } else { b })
        .collect::<Vec<_>>()
        .join(",")
}

/// Las columnas que se piden, y `_rescued_data` si la tabla la declara.
fn proyeccion(con_rescatada: bool) -> Vec<&'static str> {
    let mut proy = columnas();
    if con_rescatada {
        proy.push(RESCATADA);
    }
    proy
}

const MALOS: &[(&str, &str)] = &[
    ("entero", "siete"),
    ("real", "nada"),
    ("logico", "TRUE"),
    ("decimal", "1.239"),
    ("fecha", "2026-02-30"),
    ("hora", "24:00:00"),
    ("fechahora", "ayer"),
    ("instante", "2026-09-26 10:11:12"),
    ("binario", "sin relleno"),
];

#[test]
fn csv_con_los_diez_fisicos() {
    let cab = columnas().join(",");
    let csv = format!(
        "{cab}\n\
         {}\n\
         {}\n\
         ,,,,,,,,,\n\
         -7,NaN,false,-0.05,1970-01-01,23:59:59.999999,2026-09-26T10:11:12.123456,2026-09-26T12:11:12+02:00,\"con,coma\",\n\
         9223372036854775807,-Infinity,true,9999999999.99,0001-01-01,00:00:00,2026-09-26T00:00:00,1969-12-31 23:59:59.999999+00,\"\",\"\"\n",
        fila_csv("", ""),
        fila_csv("texto", "ñandú"),
    );
    let o = EnMemoria::con(&[("t/a.csv", csv.into_bytes())]);
    let p = peticion("csv", "t/a.csv", &columnas(), TIPOS);
    fijado("csv-fisicos", &volcar(&o, &p));
}

#[test]
fn csv_con_rescatada() {
    let cab = format!("{},sobra", columnas().join(","));
    let mut filas = vec![format!("{},", fila_csv("", ""))];
    for (col, malo) in MALOS {
        filas.push(format!("{},x", fila_csv(col, malo)));
    }
    let csv = format!("{cab}\n{}\n", filas.join("\n"));
    let o = EnMemoria::con(&[("t/a.csv", csv.into_bytes())]);
    let p = peticion("csv", "t/a.csv", &proyeccion(true), &con_rescatada());
    fijado("csv-rescatada", &volcar(&o, &p));
}

#[test]
fn csv_sin_rescatada_para_en_cada_fisico() {
    let mut s = String::new();
    for (col, malo) in MALOS {
        let csv = format!(
            "{}\n{}\n{}\n",
            columnas().join(","),
            fila_csv("", ""),
            fila_csv(col, malo)
        );
        let o = EnMemoria::con(&[("t/a.csv", csv.into_bytes())]);
        let p = peticion("csv", "t/a.csv", &columnas(), TIPOS);
        writeln!(s, "== {col} = {malo}").unwrap();
        s.push_str(&volcar(&o, &p));
    }
    fijado("csv-para", &s);
}

#[test]
fn jsonl_con_los_diez_fisicos() {
    let j = "{\"entero\":42,\"real\":1.5,\"logico\":true,\"decimal\":\"1234.56\",\"fecha\":\"2026-09-26\",\"hora\":\"10:11:12\",\"fechahora\":\"2026-09-26T10:11:12\",\"instante\":\"2026-09-26T10:11:12Z\",\"texto\":\"hola\",\"binario\":\"aGVsbG8=\"}\n\
             {\"entero\":null,\"texto\":\"\"}\n\
             {\"entero\":\"dos\",\"real\":\"nada\",\"logico\":\"x\",\"nuevo\":true,\"texto\":5}\n\
             {\"entero\":7,\"real\":2,\"decimal\":3.5}\n";
    let o = EnMemoria::con(&[("logs/d1.jsonl", j.as_bytes().to_vec())]);
    let p = peticion("jsonl", "logs/", &proyeccion(true), &con_rescatada());
    fijado("jsonl-fisicos", &volcar(&o, &p));
}

#[test]
fn los_filtros_arman_sus_escalares_con_el_constructor() {
    let csv = format!(
        "{}\n{}\n{}\n{}\n",
        columnas().join(","),
        fila_csv("", ""),
        fila_csv("entero", "7"),
        fila_csv("decimal", "0.05")
    );
    let o = EnMemoria::con(&[("t/a.csv", csv.into_bytes())]);
    let mut s = String::new();
    for (col, op, valor) in [
        ("entero", "eq", "42"),
        ("real", "gt", "1"),
        ("logico", "eq", "true"),
        ("decimal", "lt", "1000.00"),
        ("fecha", "eq", "2026-09-26"),
        ("hora", "ge", "10:00:00"),
        ("fechahora", "eq", "2026-09-26 10:11:12"),
        ("instante", "eq", "2026-09-26 10:11:12+00"),
        ("texto", "like", "ho%"),
        ("binario", "eq", "aGVsbG8="),
        ("entero", "eq", "x"),
    ] {
        let mut p = peticion("csv", "t/a.csv", &["entero", "decimal"], TIPOS);
        p.filtros = vec![ore_driver::Filtro::uno(col, op, valor)];
        writeln!(s, "== {col} {op} {valor}").unwrap();
        s.push_str(&volcar(&o, &p));
    }
    fijado("filtros", &s);
}

#[test]
fn el_listado_de_un_object_table() {
    let o = EnMemoria::con(&[
        ("docs/anio=2026/a.pdf", b"aaaa".to_vec()),
        ("docs/anio=2025/b.pdf", b"bb".to_vec()),
    ]);
    let p = Peticion {
        url: "s3://b".into(),
        objeto: "docs/".into(),
        proyeccion: ["key", "size", "contentType", "anio"]
            .iter()
            .map(|c| (c.to_string(), c.to_string()))
            .collect(),
        formato: Some("arrow".into()),
        listado: Some("**/*.pdf".into()),
        ..Default::default()
    };
    fijado("listado", &volcar(&o, &p));
}

#[test]
fn un_parquet_con_particion() {
    let esquema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, true),
        Field::new("unidades", DataType::Int32, true),
    ]));
    let lote = RecordBatch::try_new(
        esquema.clone(),
        vec![
            Arc::new(StringArray::from(vec![Some("a"), None])),
            Arc::new(arrow_array::Int32Array::from(vec![1, 2])),
        ],
    )
    .unwrap();
    let mut buf = Vec::new();
    let mut w = parquet::arrow::ArrowWriter::try_new(&mut buf, esquema, None).unwrap();
    w.write(&lote).unwrap();
    w.close().unwrap();
    let o = EnMemoria::con(&[("v/p/fecha=2026-09-01/a.parquet", buf)]);
    let mut p = peticion(
        "parquet",
        "v/p/",
        &["id", "unidades", "fecha"],
        &[
            ("id", "String"),
            ("unidades", "Integer"),
            ("fecha", "String"),
        ],
    );
    let f = p.fichero.as_mut().unwrap();
    f.patron = Some("**/*.parquet".into());
    f.particiones = vec!["fecha".into()];
    fijado("parquet", &volcar(&o, &p));
}
