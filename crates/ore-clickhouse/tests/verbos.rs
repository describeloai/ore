//! Los verbos propios (`catalogo`, `check`, `explorar`, `testigo`, `cambio`) y la lectura con `FINAL`, contra servidores de verdad
//! (ADR 0077 §6, paso 8). Se saltan solos sin `CLICKHOUSE_LAB_URLS`; necesitan la semilla del kit (`ore-kit --banco clickhouse`).

use ore_clickhouse::ClickHouse;
use ore_clickhouse::lectura::Token;
use ore_clickhouse::transporte::Cliente;
use ore_clickhouse::verbos::{cambio, catalogo, check, explorar, testigo};
use ore_driver::origen::Origen;

fn servidores() -> Vec<String> {
    match std::env::var("CLICKHOUSE_LAB_URLS") {
        Ok(v) if !v.trim().is_empty() => v.split(',').map(|s| s.trim().to_string()).collect(),
        _ => {
            eprintln!("sin CLICKHOUSE_LAB_URLS: no hay laboratorio, se salta");
            Vec::new()
        }
    }
}

fn coordenada(url: &str, objeto: &str) -> String {
    format!(r#"{{"url":"{url}","objeto":"{objeto}"}}"#)
}

#[test]
fn check_dice_que_si_y_dice_por_que_no() {
    for url in servidores() {
        let ok = check(&coordenada(&url, "")).unwrap();
        assert_eq!(ok, r#"{"ok":true}"#, "{url}");
        let mal = check(&coordenada(&url.replace("kit@", "nadie@"), "")).unwrap();
        assert!(
            mal.contains(r#""ok":false"#) && mal.contains("porque"),
            "{url}: {mal}"
        );
        assert!(
            !mal.contains("nadie@"),
            "la URL no sale en el motivo: {mal}"
        );
        let sin_base = url.replace("/kit", "/no_existe_esta_base");
        let mal = check(&coordenada(&sin_base, "")).unwrap();
        assert!(mal.contains(r#""ok":false"#), "{url}: {mal}");
    }
}

#[test]
fn el_catalogo_trae_los_tipos_la_obligatoriedad_y_ninguna_clave() {
    for url in servidores() {
        let c = catalogo(&url, "clickhouse").unwrap_or_else(|e| panic!("{url}: {e}"));
        assert!(
            c.contains("kit.tipos") && c.contains("kit.grande"),
            "{url}: {c}"
        );
        // Los tipos de OOS, no los de ClickHouse; los de ClickHouse van en `origen`.
        assert!(
            c.contains("Decimal<12, 2>") && c.contains("Nullable(Decimal(12, 2))"),
            "{url}"
        );
        assert!(
            !c.contains("primaryKey"),
            "{url}: ClickHouse no tiene claves únicas"
        );
        assert!(
            !c.contains("system."),
            "{url}: las bases del servidor no se catalogan"
        );
    }
}

#[test]
fn explorar_cuenta_los_objetos_por_base_y_testigo_no_inventa() {
    for url in servidores() {
        let e = explorar(&coordenada(&url, "")).unwrap();
        assert!(
            e.contains(r#""nombre": "kit""#) || e.contains("kit"),
            "{url}: {e}"
        );
        assert!(e.contains("solo esa"), "{url}: {e}");
        assert_eq!(
            testigo(&coordenada(&url, "")).unwrap(),
            r#"{"modo":"none"}"#,
            "{url}"
        );
    }
}

#[test]
fn la_huella_de_una_mergetree_cambia_con_cada_escritura_y_no_sin_ellas() {
    for url in servidores() {
        let c = Cliente::abrir(&url).unwrap();
        c.ejecutar("DROP TABLE IF EXISTS kit.lab_cambio").unwrap();
        c.ejecutar(
            "CREATE TABLE kit.lab_cambio (id Int64, v Int64) ENGINE = MergeTree ORDER BY id",
        )
        .unwrap();
        let huella = || cambio(&coordenada(&url, "kit.lab_cambio")).unwrap();
        let vacia = huella();
        assert!(vacia.contains(r#""respalda":true"#), "{url}: {vacia}");
        assert_eq!(huella(), vacia, "{url}: sin escribir, la misma huella");
        c.ejecutar("INSERT INTO kit.lab_cambio VALUES (1, 1), (2, 2)")
            .unwrap();
        let tras_insertar = huella();
        assert_ne!(tras_insertar, vacia, "{url}: un INSERT cambia la huella");
        c.ejecutar(
            "ALTER TABLE kit.lab_cambio UPDATE v = 9 WHERE id = 1 SETTINGS mutations_sync = 2",
        )
        .unwrap();
        let tras_mutar = huella();
        assert_ne!(
            tras_mutar, tras_insertar,
            "{url}: un UPDATE cambia la huella"
        );
        c.ejecutar("ALTER TABLE kit.lab_cambio DELETE WHERE id = 2 SETTINGS mutations_sync = 2")
            .unwrap();
        assert_ne!(huella(), tras_mutar, "{url}: un DELETE cambia la huella");
        c.ejecutar("TRUNCATE TABLE kit.lab_cambio").unwrap();
        c.ejecutar("DROP TABLE kit.lab_cambio").unwrap();
    }
}

#[test]
fn lo_que_no_tiene_huella_que_respalde_lo_dice_y_lo_que_no_existe_es_un_error() {
    for url in servidores() {
        let c = Cliente::abrir(&url).unwrap();
        c.ejecutar("DROP TABLE IF EXISTS kit.lab_memoria").unwrap();
        c.ejecutar("CREATE TABLE kit.lab_memoria (id Int64) ENGINE = Memory")
            .unwrap();
        let m = cambio(&coordenada(&url, "kit.lab_memoria")).unwrap();
        assert!(m.contains(r#""respalda":false"#), "{url}: {m}");
        let v = cambio(&coordenada(&url, "kit.lenta")).unwrap();
        assert!(v.contains(r#""respalda":false"#), "{url}: una vista: {v}");
        assert!(
            cambio(&coordenada(&url, "kit.no_existe_nunca")).is_err(),
            "{url}"
        );
        c.ejecutar("DROP TABLE kit.lab_memoria").unwrap();
    }
}

fn filas_de(url: &str, objeto: &str) -> u64 {
    let p = ore_driver::leer_peticion(&format!(
        r#"{{"url":"{url}","objeto":"{objeto}","proyeccion":{{"id":"id","v":"v"}},"formato":"arrow"}}"#
    ))
    .unwrap();
    let mut s = ClickHouse::conectar(url).unwrap();
    ClickHouse::leer(&mut s, &Token::default(), &p, &mut Vec::new()).unwrap()
}

#[test]
fn una_replacing_se_lee_con_final_y_una_mergetree_con_los_mismos_datos_no() {
    for url in servidores() {
        let c = Cliente::abrir(&url).unwrap();
        for (t, motor) in [
            ("lab_replacing", "ReplacingMergeTree(v)"),
            ("lab_plana", "MergeTree"),
        ] {
            c.ejecutar(&format!("DROP TABLE IF EXISTS kit.{t}"))
                .unwrap();
            c.ejecutar(&format!(
                "CREATE TABLE kit.{t} (id Int64, v Int64) ENGINE = {motor} ORDER BY id"
            ))
            .unwrap();
            // Dos inserciones, dos *parts*: hasta que se fusionen, la clave 1 está dos veces en disco.
            c.ejecutar(&format!("INSERT INTO kit.{t} VALUES (1, 1), (2, 1)"))
                .unwrap();
            c.ejecutar(&format!("INSERT INTO kit.{t} VALUES (1, 2)"))
                .unwrap();
        }
        assert_eq!(
            filas_de(&url, "kit.lab_replacing"),
            2,
            "{url}: cada clave una vez"
        );
        assert_eq!(
            filas_de(&url, "kit.lab_plana"),
            3,
            "{url}: tal como está en disco"
        );
        for t in ["lab_replacing", "lab_plana"] {
            c.ejecutar(&format!("DROP TABLE kit.{t}")).unwrap();
        }
    }
}
