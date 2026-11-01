//! **Paso 2: leer de verdad**, por REST y por Bulk, contra una org (ADR 0083
//! §1.6). Se decodifica el flujo IPC que sale y se comparan las dos vías fila a
//! fila: Salesforce las formatea distinto (X0 C10) y el resultado tiene que ser
//! el mismo.
//!
//! Van `#[ignore]`: piden una org. Para correrlas, como `laboratorio.rs`:
//!
//! ```text
//! ORE_SALESFORCE_LAB_URL="salesforce://<dominio>/?cliente=<Consumer Key>&usuario=<usuario>"
//! ORE_SALESFORCE_CLAVE_FICHERO=<ruta del PEM>
//! cargo test -p ore-salesforce --test lectura_real -- --ignored --nocapture --test-threads=1
//! ```

use arrow_array::cast::AsArray;
use arrow_array::types::{Decimal128Type, Int64Type, TimestampMicrosecondType};
use arrow_array::{Array, RecordBatch};
use arrow_ipc::reader::StreamReader;
use arrow_schema::DataType;
use ore_driver::origen::Origen;
use ore_driver::{Fallo, Filtro, Orden, Peticion, Valor};
use ore_salesforce::identidad::clave_del_entorno;
use ore_salesforce::lectura::{
    Modo, Politica, Salesforce, Sesion, Token, leer_con, presupuesto_con,
};
use ore_salesforce::transporte::{codificar, peticiones};

const ENV_URL: &str = "ORE_SALESFORCE_LAB_URL";

fn sesion() -> Option<Sesion> {
    let url = std::env::var(ENV_URL).ok()?;
    clave_del_entorno().ok()?;
    Some(Salesforce::conectar(&url).expect("conectar"))
}

fn leer(s: &mut Sesion, p: &Peticion, pol: Politica) -> Result<Vec<RecordBatch>, Fallo> {
    let mut buf: Vec<u8> = Vec::new();
    leer_con(s, &Token::default(), p, &mut buf, &pol)?;
    Ok(StreamReader::try_new(std::io::Cursor::new(buf), None)
        .unwrap()
        .map(Result::unwrap)
        .collect())
}

/// Filas de un SOQL crudo (REST), para tener una referencia independiente.
fn soql(s: &Sesion, q: &str) -> Vec<serde_json::Value> {
    let c = &s.cliente;
    let mut ruta = c.api(&format!("/query?q={}", codificar(q)));
    let mut todas = Vec::new();
    loop {
        let v: serde_json::Value = serde_json::from_str(&c.get(&ruta).unwrap()).unwrap();
        todas.extend(v["records"].as_array().cloned().unwrap_or_default());
        match v["nextRecordsUrl"].as_str() {
            Some(n) => ruta = n.to_string(),
            None => return todas,
        }
    }
}

/// Una celda como texto estable, para comparar vías.
fn texto(b: &RecordBatch, c: usize, f: usize) -> String {
    let a = b.column(c);
    if a.is_null(f) {
        return "∅".into();
    }
    match a.data_type() {
        DataType::Utf8 => a.as_string::<i32>().value(f).to_string(),
        DataType::Int64 => a.as_primitive::<Int64Type>().value(f).to_string(),
        DataType::Decimal128(_, _) => a.as_primitive::<Decimal128Type>().value(f).to_string(),
        DataType::Timestamp(_, _) => a
            .as_primitive::<TimestampMicrosecondType>()
            .value(f)
            .to_string(),
        DataType::Boolean => a.as_boolean().value(f).to_string(),
        t => format!("<{t:?}>"),
    }
}

/// Las filas de todos los lotes, ordenadas (REST y Bulk no dan el mismo orden).
fn celdas(lotes: &[RecordBatch]) -> Vec<Vec<String>> {
    let mut v: Vec<Vec<String>> = lotes
        .iter()
        .flat_map(|b| {
            (0..b.num_rows()).map(move |f| (0..b.num_columns()).map(|c| texto(b, c, f)).collect())
        })
        .collect();
    v.sort();
    v
}

fn pet(cols: &[&str], filtros: Vec<Filtro>) -> Peticion {
    Peticion {
        objeto: "Account".into(),
        proyeccion: cols
            .iter()
            .map(|c| (c.to_string(), c.to_string()))
            .collect(),
        filtros,
        ..Default::default()
    }
}

fn rest() -> Politica {
    Politica::con(Modo::Rest)
}

fn bulk() -> Politica {
    Politica::con(Modo::Bulk)
}

/// **La prueba central del paso 2:** las dos vías dan las mismas filas, con los
/// mismos tipos, aunque Salesforce las formatee distinto (X0 C10).
#[test]
#[ignore = "pide una org de Salesforce"]
fn rest_y_bulk_dan_las_mismas_filas_con_los_mismos_tipos() {
    let Some(mut s) = sesion() else { return };
    let p = pet(
        &[
            "Id",
            "Name",
            "NumberOfEmployees",
            "X0_Formula__c",
            "CreatedDate",
            "IsDeleted",
            "BillingCity",
        ],
        vec![Filtro::uno("Name", "like", "X0-Cuenta-%")],
    );
    let r = celdas(&leer(&mut s, &p, rest()).expect("rest"));
    let b = celdas(&leer(&mut s, &p, bulk()).expect("bulk"));
    eprintln!(
        "{} filas por REST y {} por Bulk; muestra: {:?}",
        r.len(),
        b.len(),
        r.first()
    );
    assert!(r.len() >= 100, "la siembra del X0 trae ~148 cuentas");
    assert_eq!(r, b, "REST y Bulk tienen que dar lo mismo");
}

/// X0 C11: lo difícil sobrevive a las dos vías: `NA`, comas, comillas, saltos y Unicode.
#[test]
#[ignore = "pide una org de Salesforce"]
fn lo_dificil_sobrevive_a_las_dos_vias() {
    let Some(mut s) = sesion() else { return };
    let adversos = Filtro {
        columna: "Name".into(),
        operador: "in".into(),
        valor: Valor::Lista(vec![
            "NA".into(),
            "None".into(),
            "null".into(),
            "N/A".into(),
        ]),
    };
    let desc = "fila 0, con coma, \"comillas\"\nsalto de línea · ñ 日本 😀";
    for (nombre, pol) in [("rest", rest()), ("bulk", bulk())] {
        let f = celdas(&leer(&mut s, &pet(&["Name"], vec![adversos.clone()]), pol).unwrap());
        assert_eq!(
            f.len(),
            4,
            "{nombre}: `NA`, `None`, `null` y `N/A` son nombres, no nulos: {f:?}"
        );
        let d = celdas(
            &leer(
                &mut s,
                &pet(
                    &["Description"],
                    vec![Filtro::uno("Name", "eq", "X0-Cuenta-000")],
                ),
                pol,
            )
            .unwrap(),
        );
        assert_eq!(d, vec![vec![desc.to_string()]], "{nombre}");
    }
}

/// B2: SOQL devuelve la cuenta con otras mayúsculas; el residual la quita.
#[test]
#[ignore = "pide una org de Salesforce"]
fn el_residual_deja_exacto_lo_que_soql_trae_de_mas() {
    let Some(mut s) = sesion() else { return };
    for pol in [rest(), bulk()] {
        let mal = celdas(
            &leer(
                &mut s,
                &pet(&["Id"], vec![Filtro::uno("Name", "eq", "x0-cuenta-001")]),
                pol,
            )
            .unwrap(),
        );
        let bien = celdas(
            &leer(
                &mut s,
                &pet(&["Id"], vec![Filtro::uno("Name", "eq", "X0-Cuenta-001")]),
                pol,
            )
            .unwrap(),
        );
        assert_eq!((mal.len(), bien.len()), (0, 1), "{pol:?}");
        // `Name` solo estaba para el residual: no sale en el lote.
        assert_eq!(bien[0].len(), 1);
    }
}

/// B3 + B2: el `neq` de texto es el de SQL: no devuelve nulos y sí las variantes de mayúsculas.
#[test]
#[ignore = "pide una org de Salesforce"]
fn neq_de_texto_da_lo_que_da_sql() {
    let Some(mut s) = sesion() else { return };
    let esperado = soql(
        &s,
        "SELECT Industry FROM Account WHERE Name LIKE 'X0-Cuenta-%'",
    )
    .iter()
    .filter(|r| r["Industry"].as_str().is_some_and(|i| i != "Banking"))
    .count();
    let p = pet(
        &["Id"],
        vec![
            Filtro::uno("Name", "like", "X0-Cuenta-%"),
            Filtro::uno("Industry", "neq", "Banking"),
        ],
    );
    let leidas = celdas(&leer(&mut s, &p, rest()).unwrap()).len();
    eprintln!("SQL esperaría {esperado}; se leyeron {leidas}");
    assert_eq!(leidas, esperado);
}

/// El `LIMIT` no baja al servidor con un residual: corta quien lee, y da n exactas.
#[test]
#[ignore = "pide una org de Salesforce"]
fn un_limit_con_residual_da_exactamente_n_filas() {
    let Some(mut s) = sesion() else { return };
    let mut p = pet(
        &["Id", "Name"],
        vec![Filtro::uno("Name", "like", "X0-Cuenta-%")],
    );
    p.limit = Some(5);
    assert_eq!(celdas(&leer(&mut s, &p, rest()).unwrap()).len(), 5);
}

/// B9: con `orderBy` y `limit` por Bulk (que admite las dos), el orden es el de SQL.
#[test]
#[ignore = "pide una org de Salesforce"]
fn bulk_con_orden_y_limit_da_las_primeras_n_con_los_nulos_al_final() {
    let Some(mut s) = sesion() else { return };
    let mut p = pet(&["Industry"], vec![]);
    p.orden = vec![Orden {
        columna: "Industry".into(),
        descendente: false,
    }];
    p.limit = Some(3);
    let l = leer(&mut s, &p, bulk()).unwrap();
    let v: Vec<String> = (0..l[0].num_rows()).map(|f| texto(&l[0], 0, f)).collect();
    eprintln!("{v:?}");
    assert_eq!(v.len(), 3);
    assert!(v.iter().all(|x| x != "∅"), "los nulos van al final: {v:?}");
}

/// El cambio automático: con un umbral bajo, la misma lectura va por Bulk y da lo mismo.
#[test]
#[ignore = "pide una org de Salesforce"]
fn el_modo_automatico_pasa_a_bulk_cuando_hay_mas_filas_que_el_umbral() {
    let Some(mut s) = sesion() else { return };
    let p = pet(
        &["Id", "Name"],
        vec![Filtro::uno("Name", "like", "X0-Cuenta-%")],
    );
    let por_rest = celdas(&leer(&mut s, &p, rest()).unwrap());
    let antes = peticiones();
    let auto = celdas(
        &leer(
            &mut s,
            &p,
            Politica {
                modo: Modo::Auto,
                umbral: 10,
                ..Politica::con(Modo::Auto)
            },
        )
        .unwrap(),
    );
    let gastadas = peticiones() - antes;
    eprintln!("Auto con umbral 10: {gastadas} peticiones");
    assert_eq!(auto, por_rest);
    // Por REST, 150 filas son 1-2 peticiones. Por Bulk, como mínimo: la primera
    // página de REST (de donde sale `totalSize`), crear el job, un sondeo y los
    // resultados.
    assert!(
        gastadas >= 4,
        "{gastadas} peticiones: eso no es un job de Bulk"
    );
    // Y con un umbral por encima, se queda en REST.
    let antes = peticiones();
    leer(
        &mut s,
        &p,
        Politica {
            modo: Modo::Auto,
            umbral: 100_000,
            ..Politica::con(Modo::Auto)
        },
    )
    .unwrap();
    assert!(peticiones() - antes <= 2);
}

/// B14: una lista `IN` partida se lee entera y sin repetir.
#[test]
#[ignore = "pide una org de Salesforce"]
fn una_lista_in_de_1200_ids_se_lee_entera() {
    let Some(mut s) = sesion() else { return };
    let reales: Vec<String> = soql(&s, "SELECT Id FROM Contact LIMIT 400")
        .iter()
        .map(|r| r["Id"].as_str().unwrap().to_string())
        .collect();
    let mut ids = reales.clone();
    ids.extend((0..1200 - ids.len()).map(|i| format!("003fj{i:010}AAA")));
    let p = Peticion {
        objeto: "Contact".into(),
        proyeccion: vec![("Id".into(), "Id".into())],
        filtros: vec![Filtro {
            columna: "Id".into(),
            operador: "in".into(),
            valor: Valor::Lista(ids),
        }],
        ..Default::default()
    };
    let l = celdas(&leer(&mut s, &p, rest()).unwrap());
    assert_eq!(l.len(), reales.len());
}

/// El presupuesto: con un tope por debajo del uso, no se empieza; con uno por encima, sí.
#[test]
#[ignore = "pide una org de Salesforce"]
fn el_presupuesto_pospone_con_un_tope_bajo_y_deja_pasar_con_uno_alto() {
    let Some(s) = sesion() else { return };
    // Tope 0: cualquier uso lo alcanza, sea cual sea el de la org hoy (con 1 %
    // la prueba dependía de que ya se hubieran gastado 150 peticiones).
    let e = presupuesto_con(&s.cliente, 0).unwrap_err();
    assert!(e.reintentable && e.mensaje.contains("se pospone"), "{e:?}");
    presupuesto_con(&s.cliente, 100).expect("la cuota de laboratorio no va al 100 %");
}

/// Los tipos de Arrow salen del `describe`.
#[test]
#[ignore = "pide una org de Salesforce"]
fn los_tipos_del_lote_son_los_del_describe() {
    let Some(mut s) = sesion() else { return };
    let mut p = pet(
        &[
            "NumberOfEmployees",
            "X0_Formula__c",
            "CreatedDate",
            "IsDeleted",
            "Name",
        ],
        vec![],
    );
    p.limit = Some(1);
    let l = leer(&mut s, &p, rest()).unwrap();
    let t: Vec<String> = l[0]
        .schema()
        .fields()
        .iter()
        .map(|f| f.data_type().to_string())
        .collect();
    eprintln!("{t:?}");
    assert_eq!(t[0], "Int64");
    assert_eq!(t[1], "Decimal128(18, 0)");
    assert!(t[2].starts_with("Timestamp"), "{}", t[2]);
    assert_eq!(t[3], "Boolean");
    assert_eq!(t[4], "Utf8");
}
