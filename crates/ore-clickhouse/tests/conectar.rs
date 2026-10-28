//! `conectar` contra servidores ClickHouse de verdad (ADR 0077 §6, paso 2). Se salta sola si no hay laboratorio:
//!
//! ```text
//! CLICKHOUSE_LAB_URLS=clickhouse://kit@localhost:8124/kit,...  cargo test -p ore-clickhouse --test conectar
//! ```
//!
//! Además de `kit`, cada servidor lleva tres usuarios sin contraseña que son perfiles: `ore_r1` (`readonly=1`), `ore_r2`
//! (`readonly=2`) y `ore_brk` (`result_overflow_mode='break'`).

use ore_clickhouse::ClickHouse;
use ore_clickhouse::lectura::SUELO;
use ore_driver::Codigo;
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

fn como(url: &str, usuario: &str) -> String {
    url.replace("kit@", &format!("{usuario}@"))
}

#[test]
fn el_usuario_normal_conecta_y_puede_mandar_todos_los_ajustes() {
    for url in servidores() {
        let s = ClickHouse::conectar(&url).unwrap_or_else(|f| panic!("{url}: {f:?}"));
        assert_eq!(s.readonly, 0, "{url}");
        assert_eq!(s.ajustes.len(), 2, "{url} {}: {:?}", s.version, s.ajustes);
        let v = ore_clickhouse::lectura::version_mayor_menor(&s.version).expect("versión");
        assert!(v >= SUELO, "{url}: {}", s.version);
    }
}

#[test]
fn readonly_1_conecta_pero_no_manda_ningun_ajuste() {
    for url in servidores() {
        let s =
            ClickHouse::conectar(&como(&url, "ore_r1")).unwrap_or_else(|f| panic!("{url}: {f:?}"));
        assert_eq!(s.readonly, 1, "{url}");
        assert!(s.ajustes.is_empty(), "{url}: {:?}", s.ajustes);
    }
}

#[test]
fn readonly_2_conecta_y_manda_los_ajustes() {
    for url in servidores() {
        let s =
            ClickHouse::conectar(&como(&url, "ore_r2")).unwrap_or_else(|f| panic!("{url}: {f:?}"));
        assert_eq!(s.readonly, 2, "{url}");
        assert_eq!(s.ajustes.len(), 2, "{url}: {:?}", s.ajustes);
    }
}

#[test]
fn un_perfil_con_overflow_break_se_rechaza_y_dice_cual() {
    for url in servidores() {
        let f = ClickHouse::conectar(&como(&url, "ore_brk"))
            .err()
            .unwrap_or_else(|| panic!("{url}: debió rechazarse"));
        assert_eq!(f.codigo, Codigo::Operador, "{url}: {f:?}");
        assert!(f.mensaje.contains("result_overflow_mode"), "{url}: {f:?}");
    }
}

#[test]
fn un_usuario_inexistente_es_una_credencial() {
    for url in servidores() {
        let f = ClickHouse::conectar(&como(&url, "nadie"))
            .err()
            .unwrap_or_else(|| panic!("{url}: debió fallar"));
        assert_eq!(f.codigo, Codigo::Credencial, "{url}: {f:?}");
    }
}

#[test]
fn un_puerto_cerrado_es_una_conexion() {
    // Sin laboratorio también corre: no necesita servidor.
    let f = ClickHouse::conectar("clickhouse://kit@127.0.0.1:1/kit")
        .err()
        .expect("nada escucha en el 1");
    assert_eq!(f.codigo, Codigo::Conexion, "{f:?}");
}
