//! Cancelar **por TLS** (ADR 0080 §6, paso 5): `cancel_query` de `rust-postgres` abre otra conexión al servidor para mandar la petición, y esa
//! tiene que hablar TLS con la misma CA que la sesión. Se salta sola si no hay nodo seguro:
//!
//! ```text
//! COCKROACH_LAB_SEGURO=cockroachdb://kit:kit-secreto@localhost:26270/kit  COCKROACH_LAB_CA=C:/ore-lab/ore-crdb-254-tls-ca.crt \
//!   cargo test -p ore-cockroach --test cancelar
//! ```
//!
//! **Sin** `ORE_COCKROACH_LABORATORIO` (deja pasar el texto plano). Usa `public.lenta`, que deja el banco del kit, y que `kit` pueda ver sus
//! consultas (`VIEWACTIVITY`: lo da `laboratorio.sh`).

use ore_cockroach::Cockroach;
use ore_cockroach::lectura::Token;
use ore_driver::Codigo;
use ore_driver::origen::Origen;
use std::time::{Duration, Instant};

fn nodo() -> Option<String> {
    if std::env::var("ORE_COCKROACH_LABORATORIO").as_deref() == Ok("1") {
        eprintln!("ORE_COCKROACH_LABORATORIO=1: esta prueba exige que NO esté puesta, se salta");
        return None;
    }
    let (Ok(url), Ok(ca)) = (
        std::env::var("COCKROACH_LAB_SEGURO"),
        std::env::var("COCKROACH_LAB_CA"),
    ) else {
        eprintln!("sin COCKROACH_LAB_SEGURO y COCKROACH_LAB_CA: no hay nodo seguro, se salta");
        return None;
    };
    let pem = std::fs::read_to_string(&ca).unwrap_or_else(|e| panic!("{ca}: {e}"));
    let pem: String = pem
        .trim()
        .chars()
        .map(|c| match c {
            '\n' => "%0A".to_string(),
            ' ' => "%20".to_string(),
            c => c.to_string(),
        })
        .collect();
    Some(format!("{url}?sslmode=verify-full&sslrootcert={pem}"))
}

fn vivas(s: &mut ore_cockroach::lectura::Sesion) -> i64 {
    s.cliente
        .query_one(
            "SELECT count(*) FROM [SHOW CLUSTER QUERIES] WHERE application_name = 'ore-read-cockroach' AND query LIKE '%lenta%' \
             AND query NOT LIKE '%CLUSTER QUERIES%'",
            &[],
        )
        .unwrap_or_else(|e| panic!("`kit` no puede ver sus consultas (¿VIEWACTIVITY?): {}", ore_cockroach::tls::causa(&e)))
        .get(0)
}

#[test]
fn cancelar_llega_al_servidor_por_tls_y_se_dice_cancelada_no_tiempo() {
    let Some(url) = nodo() else { return };
    let mut mirar = Cockroach::conectar(&url).unwrap_or_else(|f| panic!("{f:?}"));
    let existe: bool = mirar
        .cliente
        .query_one("SELECT to_regclass('public.lenta') IS NOT NULL", &[])
        .unwrap()
        .get(0);
    if !existe {
        eprintln!(
            "sin `public.lenta` en el nodo seguro (corre el banco del kit contra él): se salta"
        );
        return;
    }
    let mut s = Cockroach::conectar(&url).unwrap();
    let token = Token::default();
    let p = ore_driver::leer_peticion(&format!(
        r#"{{"url":"{}","objeto":"public.lenta","proyeccion":{{"id":"id"}},"formato":"arrow"}}"#,
        url.replace('"', "\\\"")
    ))
    .expect("petición");
    let t = Instant::now();
    let resultado = std::thread::scope(|sc| {
        let tk = token.clone();
        sc.spawn(move || {
            std::thread::sleep(Duration::from_millis(700));
            Cockroach::cancelar(&tk);
        });
        Cockroach::leer(&mut s, &token, &p, &mut Vec::new())
    });
    let tardo = t.elapsed();
    let f = resultado.expect_err("la lectura debía cancelarse");
    // Sin la cancelación, `lenta` tarda 9 s: si el `cancel_query` no llegó por TLS, la consulta corre hasta el final.
    assert!(
        tardo < Duration::from_secs(4),
        "tardó {tardo:?}: la cancelación no llegó al servidor"
    );
    assert_eq!(
        f.codigo,
        Codigo::Origen,
        "una cancelación pedida no es un tiempo agotado: {f:?}"
    );
    assert!(f.mensaje.starts_with("cancelada"), "{f:?}");
    assert!(!f.reintentable, "{f:?}");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        vivas(&mut mirar),
        0,
        "tras cancelar sigue viva la consulta en el servidor"
    );
}
