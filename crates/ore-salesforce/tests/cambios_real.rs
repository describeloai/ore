//! **Paso 4: los cambios**, contra una org (ADR 0083 §1.8): altas, actualizaciones
//! y borrados reales, leídos por el cursor `SystemModstamp` y por `queryAll`.
//!
//! Van `#[ignore]`: piden una org. Para correrlas, como `lectura_real.rs`:
//!
//! ```text
//! ORE_SALESFORCE_LAB_URL="salesforce://<dominio>/?cliente=<Consumer Key>&usuario=<usuario>"
//! ORE_SALESFORCE_CLAVE_FICHERO=<ruta del PEM>
//! cargo test -p ore-salesforce --test cambios_real -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Cada prueba crea sus propias cuentas `X0-P4-…` y las borra al terminar.

use arrow_array::cast::AsArray;
use arrow_array::{Array, RecordBatch};
use arrow_ipc::reader::StreamReader;
use ore_driver::origen::Origen;
use ore_driver::{Fallo, Peticion};
use ore_salesforce::identidad::clave_del_entorno;
use ore_salesforce::lectura::{Modo, Politica, Salesforce, Sesion, Token, leer_con};
use ore_salesforce::valores::texto_de_instante;
use ore_salesforce::verbos;
use std::collections::BTreeMap;

const ENV_URL: &str = "ORE_SALESFORCE_LAB_URL";

fn sesion() -> Option<Sesion> {
    let url = std::env::var(ENV_URL).ok()?;
    clave_del_entorno().ok()?;
    Some(Salesforce::conectar(&url).expect("conectar"))
}

fn url() -> String {
    std::env::var(ENV_URL).unwrap()
}

fn leer(s: &mut Sesion, p: &Peticion, pol: Politica) -> Result<Vec<RecordBatch>, Fallo> {
    let mut buf: Vec<u8> = Vec::new();
    leer_con(s, &Token::default(), p, &mut buf, &pol)?;
    Ok(StreamReader::try_new(std::io::Cursor::new(buf), None)
        .unwrap()
        .map(Result::unwrap)
        .collect())
}

/// `Id → (nombre, operación)` de un changelog.
fn cambios(lotes: &[RecordBatch]) -> BTreeMap<String, (Option<String>, String)> {
    let mut m = BTreeMap::new();
    for b in lotes {
        let (id, nombre, op) = (
            b.column_by_name("id").unwrap().as_string::<i32>(),
            b.column_by_name("nombre").unwrap().as_string::<i32>(),
            b.column_by_name("op").unwrap().as_string::<i32>(),
        );
        for f in 0..b.num_rows() {
            let n = (!nombre.is_null(f)).then(|| nombre.value(f).to_string());
            m.insert(id.value(f).to_string(), (n, op.value(f).to_string()));
        }
    }
    m
}

fn changelog(desde: &str) -> Peticion {
    Peticion {
        objeto: "Account".into(),
        proyeccion: vec![
            ("id".into(), "Id".into()),
            ("nombre".into(), "Name".into()),
            ("op".into(), "_ore_op".into()),
        ],
        cursor: Some("SystemModstamp".into()),
        start: Some(desde.into()),
        ..Default::default()
    }
}

fn unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// Crea una cuenta y devuelve su `Id`.
fn crear(s: &Sesion, nombre: &str) -> String {
    let c = &s.cliente;
    let r = c
        .enviar(
            "POST",
            &c.api("/sobjects/Account"),
            Some(&format!(r#"{{"Name":"{nombre}","NumberOfEmployees":1}}"#)),
            &[],
        )
        .expect("crear");
    let v: serde_json::Value = serde_json::from_str(&r.cuerpo).unwrap();
    v["id"].as_str().unwrap().to_string()
}

fn borrar(s: &Sesion, id: &str) {
    let c = &s.cliente;
    let _ = c.enviar(
        "DELETE",
        &c.api(&format!("/sobjects/Account/{id}")),
        None,
        &[],
    );
}

fn actualizar(s: &Sesion, id: &str, empleados: u32) {
    let c = &s.cliente;
    c.enviar(
        "PATCH",
        &c.api(&format!("/sobjects/Account/{id}")),
        Some(&format!(r#"{{"NumberOfEmployees":{empleados}}}"#)),
        &[],
    )
    .expect("actualizar");
}

fn dormir(s: u64) {
    std::thread::sleep(std::time::Duration::from_secs(s));
}

fn exacta(modo: Modo) -> Politica {
    Politica {
        relectura: 0,
        ..Politica::con(modo)
    }
}

/// Lo que se espera de un changelog con un alta, una actualización y un borrado.
fn comprobar(c: &BTreeMap<String, (Option<String>, String)>, a: &str, b: &str, d: &str, tag: &str) {
    let alta = &c[a];
    let cambio = &c[b];
    let baja = &c[d];
    assert_eq!(alta.1, "upsert", "{tag}: el alta es un upsert");
    assert!(
        alta.0.as_deref().is_some_and(|n| n.starts_with("X0-P4-")),
        "{tag}: {alta:?}"
    );
    assert_eq!(cambio.1, "upsert", "{tag}: la actualización es un upsert");
    assert_eq!(baja.1, "delete", "{tag}: el borrado es un delete");
    assert_eq!(
        baja.0, None,
        "{tag}: un borrado no trae el documento, solo la clave"
    );
}

/// **La prueba central del paso 4:** un alta, una actualización y un borrado reales
/// salen como `upsert`, `upsert` y `delete`, por REST y por Bulk (`queryAll`).
#[test]
#[ignore = "pide una org de Salesforce"]
fn un_alta_una_actualizacion_y_un_borrado_salen_como_upsert_y_delete() {
    let Some(mut s) = sesion() else { return };
    let t0 = s.cliente.ahora().unwrap();
    let desde = texto_de_instante(t0);
    dormir(2);
    let n = unix();
    let a = crear(&s, &format!("X0-P4-{n}-alta"));
    let b = crear(&s, &format!("X0-P4-{n}-cambio"));
    let d = crear(&s, &format!("X0-P4-{n}-baja"));
    dormir(2);
    actualizar(&s, &b, 4242);
    borrar(&s, &d);
    dormir(3);

    let rest = cambios(&leer(&mut s, &changelog(&desde), exacta(Modo::Rest)).expect("rest"));
    eprintln!("REST: {rest:?}");
    comprobar(&rest, &a, &b, &d, "REST");

    let bulk = cambios(&leer(&mut s, &changelog(&desde), exacta(Modo::Bulk)).expect("bulk"));
    eprintln!("Bulk: {bulk:?}");
    comprobar(&bulk, &a, &b, &d, "Bulk");
    assert_eq!(rest, bulk, "las dos vías dan el mismo changelog");

    borrar(&s, &a);
    borrar(&s, &b);
}

/// El `start` se retrasa la relectura: una lectura que empieza **después** de los
/// cambios los re-entrega igual. Es lo que absorbe el borde, y la clave `Id`
/// hace que re-entregar no duplique nada.
#[test]
#[ignore = "pide una org de Salesforce"]
fn la_relectura_solapada_re_entrega_el_borde() {
    let Some(mut s) = sesion() else { return };
    let n = unix();
    let a = crear(&s, &format!("X0-P4-{n}-solape"));
    dormir(3);
    // El inicio está DESPUÉS del alta: sin relectura no sale; con ella, sí.
    let despues = texto_de_instante(s.cliente.ahora().unwrap());
    dormir(2);
    let sin = cambios(&leer(&mut s, &changelog(&despues), exacta(Modo::Rest)).unwrap());
    let con = cambios(&leer(&mut s, &changelog(&despues), Politica::con(Modo::Rest)).unwrap());
    eprintln!(
        "sin relectura: {} filas; con ella: {}",
        sin.len(),
        con.len()
    );
    assert!(
        !sin.contains_key(&a),
        "sin relectura, un alta anterior al inicio no sale"
    );
    assert_eq!(
        con[&a].1, "upsert",
        "con la relectura, el borde se re-entrega"
    );
    borrar(&s, &a);
}

/// **El límite de la Papelera**: un `start` anterior a lo que guarda se niega,
/// con su motivo, en lugar de servir un changelog al que le faltan borrados.
#[test]
#[ignore = "pide una org de Salesforce"]
fn un_inicio_anterior_a_la_papelera_se_niega() {
    let Some(mut s) = sesion() else { return };
    let e = leer(
        &mut s,
        &changelog("2026-01-01T00:00:00Z"),
        exacta(Modo::Rest),
    )
    .unwrap_err();
    eprintln!("{e:?}");
    assert_eq!(e.codigo, ore_driver::Codigo::Operador);
    assert!(
        e.mensaje.contains("Papelera") && e.mensaje.contains("copia completa"),
        "{}",
        e.mensaje
    );
}

/// Quien lee por cursor **sin pedir `_ore_op`** recibe lo de siempre: sin columna de
/// más y sin borrados mezclados.
#[test]
#[ignore = "pide una org de Salesforce"]
fn sin_pedir_la_operacion_el_rango_es_el_de_siempre() {
    let Some(mut s) = sesion() else { return };
    let desde = texto_de_instante(s.cliente.ahora().unwrap());
    dormir(2);
    let n = unix();
    let a = crear(&s, &format!("X0-P4-{n}-plano"));
    let b = crear(&s, &format!("X0-P4-{n}-plano-baja"));
    borrar(&s, &b);
    dormir(3);
    let p = Peticion {
        proyeccion: vec![("id".into(), "Id".into()), ("nombre".into(), "Name".into())],
        ..changelog(&desde)
    };
    let l = leer(&mut s, &p, exacta(Modo::Rest)).unwrap();
    let nombres: Vec<String> = l[0]
        .schema()
        .fields()
        .iter()
        .map(|f| f.name().clone())
        .collect();
    assert_eq!(nombres, ["id", "nombre"], "sin columna `op`");
    let ids: Vec<&str> = l
        .iter()
        .flat_map(|b| (0..b.num_rows()).map(move |f| b.column(0).as_string::<i32>().value(f)))
        .collect();
    assert!(ids.contains(&a.as_str()), "el alta sale");
    assert!(
        !ids.contains(&b.as_str()),
        "el borrado no sale: una marca de agua no ve borrados"
    );
    borrar(&s, &a);
}

/// El testigo es la hora de la org, y un objeto sin cambios dice `none`.
#[test]
#[ignore = "pide una org de Salesforce"]
fn el_testigo_es_la_hora_de_la_org_y_lo_que_no_cambia_dice_none() {
    let Some(_s) = sesion() else { return };
    let coord = |o: &str| format!(r#"{{"url": "{}", "objeto": "{o}"}}"#, url());
    let t = verbos::testigo(&coord("Account")).unwrap();
    eprintln!("{t}");
    assert!(t.contains(r#""modo":"field""#), "{t}");
    let valor = t
        .split(r#""valor":""#)
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    let org = ore_salesforce::valores::instante(valor).unwrap();
    let aqui = i64::try_from(unix()).unwrap() * 1_000_000;
    assert!(
        (org - aqui).abs() < 120_000_000,
        "la org dice {valor}; aquí son {}",
        texto_de_instante(aqui)
    );
    // `OpportunityLineItem` borra sin pasar por la Papelera: no declara cambios.
    assert_eq!(
        verbos::testigo(&coord("OpportunityLineItem")).unwrap(),
        r#"{"modo":"none"}"#
    );
    assert!(
        verbos::testigo(&format!(r#"{{"url": "{}"}}"#, url()))
            .unwrap_err()
            .contains("pide el objeto")
    );
}

/// El catálogo declara la cara `D` por lo que cada objeto sabe de verdad.
#[test]
#[ignore = "pide una org de Salesforce"]
fn el_catalogo_declara_upsert_solo_donde_hay_borrados_que_leer() {
    let Some(_s) = sesion() else { return };
    let u = format!("{}&objetos=Account,Contact,OpportunityLineItem", url());
    let cat = verbos::catalogo(&u, "salesforce").unwrap();
    let modos = cat.matches(r#""mode": "upsert""#).count();
    let nones = cat.matches(r#""mode": "none""#).count();
    eprintln!("upsert: {modos}, none: {nones}");
    assert_eq!(modos, 2, "Account y Contact");
    assert_eq!(nones, 1, "OpportunityLineItem");
    assert!(cat.contains(r#""field": "SystemModstamp""#));
}

/// La huella del objeto: sin contadores, no se respalda.
#[test]
#[ignore = "pide una org de Salesforce"]
fn la_huella_no_se_respalda() {
    let Some(_s) = sesion() else { return };
    let h = verbos::cambio(&format!(r#"{{"url": "{}", "objeto": "Account"}}"#, url())).unwrap();
    assert!(h.contains(r#""respalda":false"#), "{h}");
}
