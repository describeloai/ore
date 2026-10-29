//! `conectar` contra el nodo **seguro** (TLS y SCRAM) del laboratorio (ADR 0080 §6, paso 1). Se salta sola si no hay laboratorio:
//!
//! ```text
//! COCKROACH_LAB_SEGURO=cockroachdb://kit:kit-secreto@localhost:26270/kit \
//! COCKROACH_LAB_CA=C:/ore-lab/ore-crdb-254-tls-ca.crt  cargo test -p ore-cockroach --test tls
//! ```
//!
//! **Sin** `ORE_COCKROACH_LABORATORIO`: la variable deja pasar el texto plano, y aquí se prueba que no se pasa. Con ella puesta,
//! estas pruebas se saltan. El nodo seguro lo levanta `pruebas-de-fuego/cockroach/laboratorio.sh <nombre> <versión> <puerto> seguro`
//! y deja `kit` (escribe, contraseña `kit-secreto`) y `kit_ro` (sólo lee, `kit-ro-secreto`).

use ore_cockroach::Cockroach;
use ore_cockroach::lectura::version_mayor_menor;
use ore_driver::Codigo;
use ore_driver::origen::Origen;

/// `(url del nodo, el PEM de su CA)`, o `None` si no hay laboratorio (o si la variable del laboratorio está puesta).
fn nodo() -> Option<(String, String)> {
    if std::env::var("ORE_COCKROACH_LABORATORIO").as_deref() == Ok("1") {
        eprintln!(
            "ORE_COCKROACH_LABORATORIO=1: estas pruebas exigen que NO esté puesta, se saltan"
        );
        return None;
    }
    let (Ok(url), Ok(ca)) = (
        std::env::var("COCKROACH_LAB_SEGURO"),
        std::env::var("COCKROACH_LAB_CA"),
    ) else {
        eprintln!("sin COCKROACH_LAB_SEGURO y COCKROACH_LAB_CA: no hay laboratorio, se salta");
        return None;
    };
    let pem = std::fs::read_to_string(&ca).unwrap_or_else(|e| panic!("{ca}: {e}"));
    Some((url, pem))
}

/// El PEM, como va dentro de una URL.
fn codificado(pem: &str) -> String {
    pem.trim()
        .chars()
        .map(|c| match c {
            '\n' => "%0A".to_string(),
            ' ' => "%20".to_string(),
            c => c.to_string(),
        })
        .collect()
}

fn con(url: &str, consulta: &str) -> String {
    format!("{url}?{consulta}")
}

#[test]
fn con_la_ca_en_la_url_verify_full_conecta_por_scram() {
    let Some((url, pem)) = nodo() else { return };
    let u = con(
        &url,
        &format!("sslmode=verify-full&sslrootcert={}", codificado(&pem)),
    );
    let s = Cockroach::conectar(&u).unwrap_or_else(|f| panic!("{f:?}"));
    assert!(version_mayor_menor(&s.version).is_some(), "{}", s.version);
    // La contraseña de `kit` está guardada como SCRAM (`system.users.hashedPassword` empieza por `SCRAM-SHA-256$`, comprobado a
    // mano en el laboratorio): el cliente la autentica sin ajustes.
}

#[test]
fn un_usuario_que_solo_lee_conecta_y_lee_la_version() {
    let Some((url, pem)) = nodo() else { return };
    let u = con(
        &url.replace("kit:kit-secreto", "kit_ro:kit-ro-secreto"),
        &format!("sslrootcert={}", codificado(&pem)),
    );
    let s = Cockroach::conectar(&u).unwrap_or_else(|f| panic!("{f:?}"));
    assert!(version_mayor_menor(&s.version).is_some(), "{}", s.version);
}

#[test]
fn una_contrasena_mala_es_credencial_y_no_aparece_en_el_mensaje() {
    let Some((url, pem)) = nodo() else { return };
    let u = con(
        &url.replace("kit-secreto", "mala-mala"),
        &format!("sslrootcert={}", codificado(&pem)),
    );
    let f = match Cockroach::conectar(&u) {
        Err(f) => f,
        Ok(_) => panic!("conectó con una contraseña mala"),
    };
    assert_eq!(f.codigo, Codigo::Credencial, "{f:?}");
    assert!(!f.clone().tapado(&u).mensaje.contains("mala-mala"), "{f:?}");
}

#[test]
fn sin_la_ca_el_certificado_no_se_conoce_y_el_mensaje_dice_que_hacer() {
    let Some((url, _)) = nodo() else { return };
    let f = match Cockroach::conectar(&con(&url, "sslmode=verify-full")) {
        Err(f) => f,
        Ok(_) => panic!("confió en una CA que el sistema no conoce"),
    };
    assert!(f.mensaje.contains("sslrootcert"), "{f:?}");
}

#[test]
fn otra_ca_no_vale() {
    let Some((url, _)) = nodo() else { return };
    // Un certificado de verdad (autofirmado, `CN=otra-ca-de-prueba`, generado con `openssl req -x509`) que no es el de este nodo.
    let otra = "-----BEGIN CERTIFICATE-----\nMIIBbjCCARSgAwIBAgIUCtjNUGV7hEIYWNuVZcSbd2hW6HYwCgYIKoZIzj0EAwIw\nHDEaMBgGA1UEAwwRb3RyYS1jYS1kZS1wcnVlYmEwIBcNMjYxMDI5MTAyODQzWhgP\nMjEyNjEwMDUxMDI4NDNaMBwxGjAYBgNVBAMMEW90cmEtY2EtZGUtcHJ1ZWJhMFkw\nEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEXfyv+r4k+P38DpSgaXvr9O1hj0g/GW4A\nJHzOcB1+anGsvPdT6Lnb0bdR9mK3r34RXZdnunFdQ1fybgfuI6a7JaMyMDAwHQYD\nVR0OBBYEFN2EQ1skVArSNr8TdM4cyewT4ZAtMA8GA1UdEwEB/wQFMAMBAf8wCgYI\nKoZIzj0EAwIDSAAwRQIgVZPrQ+FIKxLjpygGmzwgk/ADFe+A3o5m4DIgqqaN7AUC\nIQDa9zf7KEtGtp2NQkZ0/x9/PM2akEnqKGywi68POPTfAg==\n-----END CERTIFICATE-----";
    let f = match Cockroach::conectar(&con(&url, &format!("sslrootcert={}", codificado(otra)))) {
        Err(f) => f,
        Ok(_) => panic!("conectó con una CA que no firmó el certificado"),
    };
    assert!(f.mensaje.contains("sslrootcert"), "{f:?}");
}

#[test]
fn verify_full_rechaza_un_nombre_que_el_certificado_no_lleva_y_verify_ca_lo_deja() {
    let Some((url, pem)) = nodo() else { return };
    // El certificado lleva `localhost` y `127.0.0.1`; `127.0.0.2` llega al mismo nodo (Docker publica en todas) pero no está.
    let otro = url.replace("localhost", "127.0.0.2");
    let ca = codificado(&pem);
    let f = match Cockroach::conectar(&con(
        &otro,
        &format!("sslmode=verify-full&sslrootcert={ca}"),
    )) {
        Err(f) => f,
        Ok(_) => panic!("verify-full aceptó un nombre que el certificado no lleva"),
    };
    assert!(
        f.mensaje.contains("nombre de host") || f.mensaje.contains("mismatch"),
        "{f:?}"
    );
    let s = Cockroach::conectar(&con(&otro, &format!("sslmode=verify-ca&sslrootcert={ca}")))
        .unwrap_or_else(|f| panic!("verify-ca debía dejarlo: {f:?}"));
    assert!(version_mayor_menor(&s.version).is_some());
}

#[test]
fn el_texto_plano_se_niega_aunque_el_nodo_lo_admitiera() {
    let Some((url, _)) = nodo() else { return };
    for m in ["disable", "allow", "prefer"] {
        let f = match Cockroach::conectar(&con(&url, &format!("sslmode={m}"))) {
            Err(f) => f,
            Ok(_) => panic!("sslmode={m} conectó"),
        };
        assert_eq!(f.codigo, Codigo::Credencial, "{m}: {f:?}");
        assert!(f.mensaje.contains("no se admite"), "{m}: {f:?}");
    }
}
