//! **Lo que solo una org de mentira deja provocar** (ADR 0083 paso 5): un token que caduca, un `503`, la
//! cuota agotada, un reloj adelantado, un CSV con comas y saltos de línea, un job de Bulk que hay que abortar.
//!
//! Van `#[ignore]`: piden `pruebas-de-fuego/salesforce-de-mentira.py` en marcha. Lo hace
//! `pruebas-de-fuego/o7-salesforce.sh`; a mano:
//!
//! ```text
//! python3 pruebas-de-fuego/salesforce-de-mentira.py 8794 &
//! ORE_LAB_SALESFORCE_MENTIRA=http://127.0.0.1:8794 ORE_SALESFORCE_ENDPOINT=http://127.0.0.1:8794 \
//!   ORE_SALESFORCE_CLAVE_FICHERO=<PEM> cargo test -p ore-salesforce --test mentira -- --ignored --test-threads=1
//! ```
//!
//! Cada prueba siembra su propio objeto y deja la org como la encontró (cuota, reloj, fallos).

use arrow_array::RecordBatch;
use arrow_ipc::reader::StreamReader;
use ore_driver::origen::Origen;
use ore_driver::{Codigo, Fallo, Peticion};
use ore_salesforce::identidad::clave_del_entorno;
use ore_salesforce::lectura::{Modo, Politica, Salesforce, Sesion, Token, leer_con};
use ore_salesforce::verbos;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

const ENV_MENTIRA: &str = "ORE_LAB_SALESFORCE_MENTIRA";
const URL: &str = "salesforce://kit.my.salesforce.com/?cliente=3MVGkit&usuario=kit%40lab.ore";

fn org() -> Option<String> {
    let m = std::env::var(ENV_MENTIRA).ok()?;
    clave_del_entorno().ok()?;
    Some(m.trim_end_matches('/').to_string())
}

fn admin(m: &str, ruta: &str, cuerpo: Value) -> Value {
    let t = ureq::post(&format!("{m}/_mentira/{ruta}"))
        .set("content-type", "application/json")
        .send_string(&cuerpo.to_string())
        .unwrap_or_else(|e| panic!("`{ruta}`: {e}"))
        .into_string()
        .unwrap();
    serde_json::from_str(&t).unwrap_or(Value::Null)
}

fn cuentas(m: &str) -> Value {
    let t = ureq::get(&format!("{m}/_mentira/cuentas"))
        .call()
        .unwrap()
        .into_string()
        .unwrap();
    serde_json::from_str(&t).unwrap()
}

fn sesion() -> Sesion {
    Salesforce::conectar(URL).expect("conectar")
}

fn leer(s: &mut Sesion, p: &Peticion, pol: Politica) -> Result<Vec<RecordBatch>, Fallo> {
    let mut buf: Vec<u8> = Vec::new();
    leer_con(s, &Token::default(), p, &mut buf, &pol)?;
    Ok(StreamReader::try_new(std::io::Cursor::new(buf), None)
        .unwrap()
        .map(Result::unwrap)
        .collect())
}

fn filas(l: &[RecordBatch]) -> usize {
    l.iter().map(RecordBatch::num_rows).sum()
}

/// Un objeto propio con `n` filas `Nombre__c = f-<i>`.
fn objeto_simple(m: &str, nombre: &str, n: usize) {
    admin(
        m,
        "objeto",
        json!({
            "nombre": nombre,
            "campos": [{"nombre": "Nombre__c", "tipo": "string", "largo": 80}],
            "filas": (0..n).map(|i| json!({"Nombre__c": format!("f-{i}")})).collect::<Vec<_>>(),
        }),
    );
}

fn peticion(objeto: &str) -> Peticion {
    Peticion {
        objeto: objeto.into(),
        proyeccion: vec![("id".into(), "Id".into()), ("n".into(), "Nombre__c".into())],
        ..Default::default()
    }
}

fn rest() -> Politica {
    Politica::con(Modo::Rest)
}

/// Al terminar, aunque la prueba falle: la org como estaba.
struct Restaurar(String);
impl Drop for Restaurar {
    fn drop(&mut self) {
        admin(&self.0, "cuota", json!({"usadas": 0, "max": 15000}));
        admin(&self.0, "reloj", json!({"desfase_s": 0}));
        admin(&self.0, "fallos", json!({"cada_503": 0}));
        admin(&self.0, "lenta", json!({"segundos": 9}));
    }
}

/// El token caduca (la org acorta la sesión): el primer `401` lo renueva y la lectura sigue.
#[test]
#[ignore = "pide la org de mentira"]
fn un_token_caducado_se_renueva_solo_al_primer_401() {
    let Some(m) = org() else { return };
    let _r = Restaurar(m.clone());
    objeto_simple(&m, "Caduca__c", 3);
    let mut s = sesion();
    assert_eq!(
        filas(&leer(&mut s, &peticion("Caduca__c"), rest()).unwrap()),
        3
    );
    let antes = cuentas(&m)["canjes"].as_u64().unwrap();
    admin(&m, "caducar", json!({}));
    // El mismo cliente, con el token ya muerto: no falla, canjea otro.
    assert_eq!(
        filas(&leer(&mut s, &peticion("Caduca__c"), rest()).unwrap()),
        3
    );
    assert_eq!(cuentas(&m)["canjes"].as_u64().unwrap(), antes + 1);
}

/// `503 Server Unavailable` con `Retry-After`: se reintenta, y salen las filas.
#[test]
#[ignore = "pide la org de mentira"]
fn un_503_se_reintenta_y_salen_las_filas() {
    let Some(m) = org() else { return };
    let _r = Restaurar(m.clone());
    objeto_simple(&m, "Cinco03__c", 4);
    let mut s = sesion();
    admin(&m, "fallos", json!({"cada_503": 2}));
    let t = Instant::now();
    assert_eq!(
        filas(&leer(&mut s, &peticion("Cinco03__c"), rest()).unwrap()),
        4
    );
    assert!(
        t.elapsed() >= Duration::from_millis(900),
        "esperó el Retry-After de 1 s: {:?}",
        t.elapsed()
    );
}

/// Cuota diaria ya agotada por la org: `REQUEST_LIMIT_EXCEEDED` no se reintenta (esperar segundos no la devuelve).
#[test]
#[ignore = "pide la org de mentira"]
fn la_cuota_agotada_no_se_reintenta() {
    let Some(m) = org() else { return };
    let _r = Restaurar(m.clone());
    objeto_simple(&m, "Cuota__c", 2);
    let mut s = sesion();
    // Una lectura antes: el cliente conoce ya el uso y el `describe`, y no vuelve a preguntar.
    assert_eq!(
        filas(&leer(&mut s, &peticion("Cuota__c"), rest()).unwrap()),
        2
    );
    // Ahora la org dice basta en la siguiente petición (6 > 5), a mitad de lectura.
    admin(&m, "cuota", json!({"usadas": 5, "max": 5}));
    let t = Instant::now();
    let e = leer(&mut s, &peticion("Cuota__c"), rest()).unwrap_err();
    assert!(
        e.mensaje.contains("cuota diaria") && !e.reintentable,
        "{e:?}"
    );
    assert!(
        t.elapsed() < Duration::from_secs(3),
        "no se reintentó: {:?}",
        t.elapsed()
    );
}

/// Cuota casi gastada **antes** de empezar: no se empieza, y se dice que se pospone.
#[test]
#[ignore = "pide la org de mentira"]
fn con_la_cuota_por_encima_del_tope_no_se_empieza() {
    let Some(m) = org() else { return };
    let _r = Restaurar(m.clone());
    objeto_simple(&m, "Pospone__c", 2);
    let mut s = sesion();
    admin(&m, "cuota", json!({"usadas": 13_000, "max": 15_000}));
    let antes = cuentas(&m)["lecturas"].as_u64().unwrap();
    let e = leer(&mut s, &peticion("Pospone__c"), rest()).unwrap_err();
    assert!(e.reintentable && e.mensaje.contains("se pospone"), "{e:?}");
    assert_eq!(
        cuentas(&m)["lecturas"].as_u64().unwrap(),
        antes,
        "ni una consulta"
    );
}

/// El testigo es la hora **de la org**: con la org dos horas adelantada, no es la del reloj de quien lee.
#[test]
#[ignore = "pide la org de mentira"]
fn el_testigo_sigue_el_reloj_de_la_org_y_no_el_de_quien_lee() {
    let Some(m) = org() else { return };
    let _r = Restaurar(m.clone());
    objeto_simple(&m, "Reloj__c", 1);
    admin(&m, "reloj", json!({"desfase_s": 7200}));
    let t = verbos::testigo(&format!(r#"{{"url": "{URL}", "objeto": "Reloj__c"}}"#)).unwrap();
    let valor = t
        .split(r#""valor":""#)
        .nth(1)
        .and_then(|r| r.split('"').next())
        .unwrap_or_else(|| panic!("sin valor: {t}"));
    let org = ore_salesforce::valores::instante(valor).unwrap();
    let aqui = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_micros() as i64;
    let dif = (org - aqui) / 1_000_000;
    assert!(
        (7200 - 120..=7200 + 120).contains(&dif),
        "la org va {dif} s por delante, no ~7200 ({valor})"
    );
}

/// REST y Bulk dan **las mismas filas y los mismos tipos**, con lo que más rompe un CSV: comas, comillas, saltos
/// de línea, acentos, la cadena vacía (que Salesforce guarda como nulo) y el nulo (que Bulk escribe `""`).
#[test]
#[ignore = "pide la org de mentira"]
fn rest_y_bulk_dan_lo_mismo_con_un_csv_dificil() {
    let Some(m) = org() else { return };
    let _r = Restaurar(m.clone());
    admin(
        &m,
        "objeto",
        json!({
            "nombre": "Csv__c",
            "campos": [
                {"nombre": "Texto__c", "tipo": "textarea", "largo": 4000},
                {"nombre": "N__c", "tipo": "int"},
                {"nombre": "Importe__c", "tipo": "currency", "precision": 12, "escala": 2},
                {"nombre": "Real__c", "tipo": "double"},
                {"nombre": "Dia__c", "tipo": "date"},
                {"nombre": "Instante__c", "tipo": "datetime"},
                {"nombre": "Si__c", "tipo": "boolean"},
            ],
            "filas": [
                {"Texto__c": "Ana, \"SA\"", "N__c": "1", "Importe__c": "-2.50", "Real__c": "-10000000000", "Dia__c": "2000-02-29", "Instante__c": "2000-02-29T12:30:00.5Z", "Si__c": "true"},
                {"Texto__c": "línea\nnueva\r\ncon CRLF", "N__c": "2147483647", "Importe__c": "9999999999.99", "Real__c": "0.000123", "Dia__c": "1900-01-01", "Instante__c": "2099-12-31T23:59:59Z", "Si__c": "false"},
                {"Texto__c": "", "N__c": null, "Importe__c": null, "Real__c": null, "Dia__c": null, "Instante__c": null, "Si__c": null},
                {"Texto__c": "ñandú, \u{1F980} y \"comillas\"", "N__c": "-2147483648", "Importe__c": "0.01", "Real__c": "1.5", "Dia__c": "2026-10-04", "Instante__c": "1970-01-01T00:00:00Z", "Si__c": "true"},
                {"Texto__c": "\"\"", "N__c": "0", "Importe__c": "0.00", "Real__c": "20", "Dia__c": "2026-01-01", "Instante__c": "2026-01-01T00:00:00Z", "Si__c": "false"},
            ],
        }),
    );
    let p = Peticion {
        objeto: "Csv__c".into(),
        proyeccion: [
            "Texto__c",
            "N__c",
            "Importe__c",
            "Real__c",
            "Dia__c",
            "Instante__c",
            "Si__c",
        ]
        .iter()
        .map(|c| (c.to_lowercase(), c.to_string()))
        .collect(),
        ..Default::default()
    };
    let mut s = sesion();
    let por_rest = leer(&mut s, &p, Politica::con(Modo::Rest)).unwrap();
    let por_bulk = leer(&mut s, &p, Politica::con(Modo::Bulk)).unwrap();
    assert_eq!(filas(&por_rest), 5);
    assert_eq!(por_rest, por_bulk, "REST y Bulk no dicen lo mismo");
    // La cadena vacía es nulo (Salesforce la guarda así); el literal `""` no.
    let t = por_rest[0].column_by_name("texto__c").unwrap();
    assert!(arrow_array::Array::is_null(t, 2), "'' → nulo");
    assert!(
        !arrow_array::Array::is_null(t, 4),
        "el texto `\"\"` no es nulo"
    );
}

/// El modo automático pasa a Bulk al ver que la consulta pasa del umbral, y no antes.
#[test]
#[ignore = "pide la org de mentira"]
fn el_modo_automatico_pasa_a_bulk_solo_por_encima_del_umbral() {
    let Some(m) = org() else { return };
    let _r = Restaurar(m.clone());
    objeto_simple(&m, "Umbral__c", 6);
    let mut s = sesion();
    let jobs = || cuentas(&m)["jobs_creados"].as_u64().unwrap();
    let pol = |umbral| Politica {
        umbral,
        ..Politica::con(Modo::Auto)
    };
    let antes = jobs();
    assert_eq!(
        filas(&leer(&mut s, &peticion("Umbral__c"), pol(100)).unwrap()),
        6
    );
    assert_eq!(jobs(), antes, "por debajo del umbral: REST");
    assert_eq!(
        filas(&leer(&mut s, &peticion("Umbral__c"), pol(5)).unwrap()),
        6
    );
    assert_eq!(jobs(), antes + 1, "por encima: un job de Bulk");
}

/// Si una lectura por Bulk se corta (`timeoutMs`), el job se **aborta**: uno vivo sigue gastando la cuota de 24 h.
#[test]
#[ignore = "pide la org de mentira"]
fn un_job_de_bulk_cortado_por_el_tiempo_se_aborta() {
    let Some(m) = org() else { return };
    let _r = Restaurar(m.clone());
    admin(
        &m,
        "objeto",
        json!({"nombre": "Lento__c", "lenta": true,
            "campos": [{"nombre": "Nombre__c", "tipo": "string", "largo": 80}],
            "filas": [{"Nombre__c": "a"}]}),
    );
    admin(&m, "lenta", json!({"segundos": 30}));
    let mut s = sesion();
    let mut p = peticion("Lento__c");
    p.timeout_ms = Some(2500);
    let e = leer(&mut s, &p, Politica::con(Modo::Bulk)).unwrap_err();
    assert_eq!(e.codigo, Codigo::Tiempo, "{e:?}");
    let vivas = ureq::get(&format!("{m}/_mentira/vivas?objeto=Lento__c"))
        .call()
        .unwrap()
        .into_string()
        .unwrap();
    assert!(
        vivas.contains(r#""vivas": 0"#) || vivas.contains(r#""vivas":0"#),
        "el job sigue vivo: {vivas}"
    );
}
