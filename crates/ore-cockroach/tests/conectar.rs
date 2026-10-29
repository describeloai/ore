//! `conectar` contra nodos CockroachDB `--insecure` de verdad (ADR 0080 §6, paso 1). Se salta sola si no hay laboratorio:
//!
//! ```text
//! COCKROACH_LAB_URLS=cockroachdb://kit@localhost:26257/kit,cockroachdb://kit@localhost:26258/kit,cockroachdb://kit@localhost:26259/kit \
//! ORE_COCKROACH_LABORATORIO=1  cargo test -p ore-cockroach --test conectar
//! ```
//!
//! `COCKROACH_LAB_POSTGRES` (opcional) es la URL de un PostgreSQL de verdad: `conectar` debe decir que no es CockroachDB.
//! El nodo seguro, con TLS y SCRAM, lo prueba `tests/tls.rs`.

use ore_cockroach::Cockroach;
use ore_cockroach::lectura::{SUELO, version_mayor_menor};
use ore_driver::Codigo;
use ore_driver::origen::Origen;

fn servidores() -> Vec<String> {
    let lab = std::env::var("ORE_COCKROACH_LABORATORIO").as_deref() == Ok("1");
    match std::env::var("COCKROACH_LAB_URLS") {
        Ok(v) if !v.trim().is_empty() && lab => {
            v.split(',').map(|s| s.trim().to_string()).collect()
        }
        _ => {
            eprintln!(
                "sin COCKROACH_LAB_URLS y ORE_COCKROACH_LABORATORIO=1: no hay laboratorio, se salta"
            );
            Vec::new()
        }
    }
}

fn como(url: &str, usuario: &str) -> String {
    url.replace("//kit@", &format!("//{usuario}@"))
}

/// La misma URL, con otra base (la última parte de la ruta).
fn en_la_base(url: &str, base: &str) -> String {
    let (resto, _) = url.rsplit_once('/').expect("la URL lleva base");
    format!("{resto}/{base}")
}

#[test]
fn el_usuario_normal_conecta_y_su_version_pasa_el_suelo() {
    for url in servidores() {
        let s = Cockroach::conectar(&url).unwrap_or_else(|f| panic!("{url}: {f:?}"));
        let v = version_mayor_menor(&s.version)
            .unwrap_or_else(|| panic!("{url}: versión {:?}", s.version));
        assert!(v >= SUELO, "{url}: {}", s.version);
    }
}

#[test]
fn la_sesion_es_de_solo_lectura() {
    for url in servidores() {
        let mut s = Cockroach::conectar(&url).unwrap_or_else(|f| panic!("{url}: {f:?}"));
        let ro: String = s
            .cliente
            .query_one("SHOW default_transaction_read_only", &[])
            .unwrap()
            .get(0);
        assert_eq!(ro, "on", "{url}");
        // Y de verdad no escribe: aunque `kit` tenga permiso para crear la tabla.
        let e = s
            .cliente
            .batch_execute("CREATE TABLE ore_no_debe_existir (a INT8)")
            .expect_err("la sesión es de solo lectura");
        assert_eq!(
            e.as_db_error().map(|d| d.code().code()),
            Some("25006"),
            "{url}: {e}"
        );
    }
}

#[test]
fn un_usuario_sin_privilegios_lee_la_version() {
    // X0 lo midió con `ore_r` sobre una tabla; aquí, con un usuario que ni siquiera tiene base.
    for url in servidores() {
        // El usuario se crea como `root`, por una sesión que sí escribe.
        let root = en_la_base(&como(&url, "root"), "defaultdb");
        let mut a = ore_cockroach::tls::abrir(&root).unwrap_or_else(|f| panic!("{root}: {f:?}"));
        // En dos lotes: un lote entero ya arranca en la transacción de solo lectura.
        a.cliente
            .batch_execute("SET default_transaction_read_only = false")
            .unwrap();
        a.cliente
            .batch_execute("CREATE USER IF NOT EXISTS ore_sin_nada")
            .unwrap();
        let s = Cockroach::conectar(&como(&root, "ore_sin_nada"))
            .unwrap_or_else(|f| panic!("{url}: {f:?}"));
        assert!(
            version_mayor_menor(&s.version).is_some(),
            "{url}: {}",
            s.version
        );
    }
}

#[test]
fn una_base_que_no_existe_es_un_objeto() {
    for url in servidores() {
        let f = match Cockroach::conectar(&en_la_base(&url, "no_existe_esta")) {
            Err(f) => f,
            Ok(_) => panic!("{url}: conectó a una base que no existe"),
        };
        assert_eq!(f.codigo, Codigo::Objeto, "{url}: {f:?}");
    }
}

#[test]
fn un_servidor_que_no_esta_es_conexion() {
    if servidores().is_empty() {
        return;
    }
    let f = match Cockroach::conectar("cockroachdb://kit@127.0.0.1:1/kit") {
        Err(f) => f,
        Ok(_) => panic!("conectó a un puerto cerrado"),
    };
    assert_eq!(f.codigo, Codigo::Conexion, "{f:?}");
    assert!(f.reintentable, "{f:?}");
}

#[test]
fn postgresql_de_verdad_no_es_cockroachdb() {
    let (Ok(url), true) = (
        std::env::var("COCKROACH_LAB_POSTGRES"),
        std::env::var("ORE_COCKROACH_LABORATORIO").as_deref() == Ok("1"),
    ) else {
        eprintln!("sin COCKROACH_LAB_POSTGRES: se salta");
        return;
    };
    let f = match Cockroach::conectar(&url) {
        Err(f) => f,
        Ok(_) => panic!("{url}: tomó PostgreSQL por CockroachDB"),
    };
    assert!(f.mensaje.contains("no es CockroachDB"), "{f:?}");
}

#[test]
fn sin_la_variable_del_laboratorio_el_nodo_inseguro_se_niega() {
    if std::env::var("ORE_COCKROACH_LABORATORIO").as_deref() == Ok("1") {
        return; // con ella puesta, esta comprobación no tiene sentido
    }
    let f = match Cockroach::conectar("cockroachdb://kit@localhost:26257/kit?sslmode=disable") {
        Err(f) => f,
        Ok(_) => panic!("conectó sin TLS fuera del laboratorio"),
    };
    assert_eq!(f.codigo, Codigo::Credencial, "{f:?}");
    assert!(f.mensaje.contains("no se admite"), "{f:?}");
}
