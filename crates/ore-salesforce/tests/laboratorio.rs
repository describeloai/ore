//! **El laboratorio**: el traductor y la identidad contra una org de verdad
//! (ADR 0083 §4). Las pruebas puras dicen que el SOQL es el que escribimos; esto
//! dice que **Salesforce lo acepta y da las filas que SQL daría**.
//!
//! Van `#[ignore]`: piden una org. Para correrlas:
//!
//! ```text
//! ORE_SALESFORCE_LAB_URL="salesforce://<dominio>/?cliente=<Consumer Key>&usuario=<usuario>"
//! ORE_SALESFORCE_CLAVE_FICHERO=<ruta del PEM>
//! cargo test -p ore-salesforce --test laboratorio -- --ignored --nocapture
//! ```
//!
//! La org del X0 trae las cuentas, contactos y oportunidades `X0-` de
//! `pruebas-de-fuego/salesforce-x0.py`; lo que no encuentra, lo dice y sigue.

use ore_driver::{Filtro, Orden, Peticion};
use ore_salesforce::identidad::{ENV_CLAVE_FICHERO, clave_del_entorno};
use ore_salesforce::lectura::{abrir, describir};
use ore_salesforce::transporte::{Cliente, codificar};
use ore_salesforce::{Objeto, traducir};

const ENV_URL: &str = "ORE_SALESFORCE_LAB_URL";

fn cliente() -> Option<Cliente> {
    let url = std::env::var(ENV_URL).ok()?;
    if clave_del_entorno().is_err() {
        eprintln!("laboratorio: falta `{ENV_CLAVE_FICHERO}`: omitido");
        return None;
    }
    Some(abrir(&url).expect("la URL del laboratorio"))
}

/// Filas de un SOQL (REST), siguiendo `nextRecordsUrl`.
fn filas(c: &Cliente, soql: &str) -> Vec<serde_json::Value> {
    let mut todas = Vec::new();
    let mut ruta = c.api(&format!("/query?q={}", codificar(soql)));
    loop {
        let t = c
            .get(&ruta)
            .unwrap_or_else(|f| panic!("Salesforce rechazó `{soql}`: {f}"));
        let v: serde_json::Value = serde_json::from_str(&t).unwrap();
        todas.extend(v["records"].as_array().cloned().unwrap_or_default());
        match v["nextRecordsUrl"].as_str() {
            Some(n) => ruta = n.to_string(),
            None => return todas,
        }
    }
}

fn cuenta(c: &Cliente, donde: &str) -> usize {
    filas(c, &format!("SELECT Id FROM Account WHERE {donde}")).len()
}

fn pet(cols: &[&str]) -> Peticion {
    Peticion {
        objeto: "Account".into(),
        proyeccion: cols
            .iter()
            .map(|c| (c.to_string(), c.to_string()))
            .collect(),
        ..Default::default()
    }
}

fn soql(c: &Cliente, p: &Peticion) -> (Objeto, Vec<String>) {
    let o = describir(c, &p.objeto).expect("describe");
    let t = traducir(p, &o).expect("traduce");
    (o, t.consultas)
}

/// F2: el JWT de Rust, firmado con `rsa`, lo canjea una org real.
#[test]
#[ignore = "pide una org de Salesforce"]
fn el_jwt_firmado_en_rust_lo_acepta_la_org() {
    let Some(c) = cliente() else { return };
    let t = c.get(&c.api("/limits")).expect("el canje y /limits");
    let v: serde_json::Value = serde_json::from_str(&t).unwrap();
    assert!(v["DailyApiRequests"]["Max"].as_i64().unwrap() > 0);
    assert!(
        c.uso().is_some(),
        "Sforce-Limit-Info llega en cada respuesta (F4)"
    );
    eprintln!("cuota: {:?}", c.uso());
}

/// A3: el `describe` real, con lo que dijo X0.
#[test]
#[ignore = "pide una org de Salesforce"]
fn el_describe_real_dice_lo_que_midio_x0() {
    let Some(c) = cliente() else { return };
    let o = describir(&c, "Account").unwrap();
    assert!(o.consultable && o.campos.len() > 60);
    assert!(o.campo("BillingAddress").unwrap().es_compuesto());
    assert!(o.campo("BillingCity").unwrap().seleccionable());
    assert_eq!(
        o.campo("Name").unwrap().compuesto_de.as_deref(),
        Some("Name")
    );
    assert!(!o.campo("Description").unwrap().filtrable, "B7");
    assert_eq!(o.cursor(), Some("SystemModstamp"));
    assert!(!o.columnas().any(|c| c.es_compuesto()));
}

/// B3: `!=` incluye los nulos en SOQL; la traducción tiene que dar lo de SQL.
#[test]
#[ignore = "pide una org de Salesforce"]
fn neq_numerico_da_lo_que_da_sql_y_no_lo_que_da_soql() {
    let Some(c) = cliente() else { return };
    let con_valor = cuenta(&c, "NumberOfEmployees != NULL");
    let iguales = cuenta(&c, "NumberOfEmployees = 100");
    let ingenuo = cuenta(&c, "NumberOfEmployees != 100");
    let mut p = pet(&["Id"]);
    p.filtros
        .push(Filtro::uno("NumberOfEmployees", "neq", "100"));
    let (_, q) = soql(&c, &p);
    let traducido = filas(&c, &q[0]).len();
    eprintln!(
        "sin nulo: {con_valor}, iguales: {iguales}; SOQL ingenuo {ingenuo}; traducido {traducido}"
    );
    assert_eq!(
        traducido,
        con_valor - iguales,
        "SQL: ni el 100 ni los nulos"
    );
    assert!(ingenuo > traducido, "el `!=` ingenuo trae los nulos (B3)");
}

/// B9: sin `NULLS LAST`, los nulos van primero en los dos sentidos.
#[test]
#[ignore = "pide una org de Salesforce"]
fn el_orden_traducido_pone_los_nulos_al_final_en_los_dos_sentidos() {
    let Some(c) = cliente() else { return };
    for descendente in [false, true] {
        let mut p = pet(&["Industry"]);
        p.orden = vec![Orden {
            columna: "Industry".into(),
            descendente,
        }];
        p.limit = Some(3);
        let (_, q) = soql(&c, &p);
        let f = filas(&c, &q[0]);
        assert_eq!(f.len(), 3);
        assert!(
            f.iter().all(|r| !r["Industry"].is_null()),
            "desc={descendente}: {f:?}"
        );
    }
}

/// B2 y la regla del residual: el servidor no distingue mayúsculas, así que
/// el `eq` de texto trae de más y el residual lo deja exacto.
#[test]
#[ignore = "pide una org de Salesforce"]
fn eq_de_texto_trae_de_mas_y_el_residual_lo_marca() {
    let Some(c) = cliente() else { return };
    let mut p = pet(&["Name"]);
    p.filtros.push(Filtro::uno("Name", "eq", "x0-cuenta-001"));
    let o = describir(&c, "Account").unwrap();
    let t = traducir(&p, &o).unwrap();
    let f = filas(&c, &t.consultas[0]);
    if f.is_empty() {
        eprintln!("laboratorio: no hay cuentas X0-: omitido");
        return;
    }
    assert_eq!(
        f[0]["Name"], "X0-Cuenta-001",
        "SOQL devolvió el nombre con otras mayúsculas"
    );
    assert_eq!(
        t.residuales.len(),
        1,
        "y por eso el filtro queda como residual"
    );
}

/// B14: una lista `IN` de 1.200 ids no cabe por GET (`431`/`414`): se parte, y
/// cada trozo lo acepta Salesforce. La unión son exactamente las filas reales.
#[test]
#[ignore = "pide una org de Salesforce"]
fn una_lista_in_de_1200_ids_se_parte_y_cada_trozo_se_acepta() {
    let Some(c) = cliente() else { return };
    let reales: Vec<String> = filas(&c, "SELECT Id FROM Contact LIMIT 400")
        .iter()
        .map(|r| r["Id"].as_str().unwrap().to_string())
        .collect();
    if reales.is_empty() {
        eprintln!("laboratorio: no hay contactos: omitido");
        return;
    }
    let mut ids = reales.clone();
    ids.extend((0..1200 - ids.len().min(1200)).map(|i| format!("003fj{i:010}AAA")));
    let mut p = pet(&["Id"]);
    p.objeto = "Contact".into();
    p.filtros.push(Filtro {
        columna: "Id".into(),
        operador: "in".into(),
        valor: ore_driver::Valor::Lista(ids),
    });
    let o = describir(&c, "Contact").unwrap();
    let t = traducir(&p, &o).unwrap();
    assert!(t.consultas.len() > 1, "1200 ids no caben en una consulta");
    let mut total = 0;
    for q in &t.consultas {
        assert!(q.len() <= ore_salesforce::traductor::MAX_SOQL);
        total += filas(&c, q).len();
    }
    eprintln!(
        "{} consultas, {total} filas de {} ids reales",
        t.consultas.len(),
        reales.len()
    );
    assert_eq!(total, reales.len(), "ni se pierde ni se repite");
}

/// B1 + C: los compuestos y el texto largo se leen por SOQL con lo que
/// `describe` dice que se lee.
#[test]
#[ignore = "pide una org de Salesforce"]
fn todas_las_columnas_planas_de_account_se_piden_de_una_vez() {
    let Some(c) = cliente() else { return };
    let o = describir(&c, "Account").unwrap();
    let cols: Vec<&str> = o.columnas().map(|k| k.nombre.as_str()).collect();
    let mut p = pet(&cols);
    p.limit = Some(1);
    let t = traducir(&p, &o).unwrap();
    assert_eq!(filas(&c, &t.consultas[0]).len(), 1);
    eprintln!(
        "{} columnas planas en una sola consulta de {} caracteres",
        cols.len(),
        t.consultas[0].len()
    );
}
