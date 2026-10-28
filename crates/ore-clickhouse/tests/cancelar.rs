//! Tiempo y cancelación contra servidores ClickHouse de verdad (ADR 0077 §6, paso 6). Necesitan la vista `kit.lenta` que carga el banco del
//! kit (`ore-kit --banco clickhouse`): duerme 9 s por bloque antes de dar una fila. Se saltan solas sin `CLICKHOUSE_LAB_URLS`.

use ore_clickhouse::ClickHouse;
use ore_clickhouse::lectura::Token;
use ore_clickhouse::transporte::Cliente;
use ore_driver::Codigo;
use ore_driver::origen::Origen;
use std::time::{Duration, Instant};

fn servidores() -> Vec<String> {
    match std::env::var("CLICKHOUSE_LAB_URLS") {
        Ok(v) if !v.trim().is_empty() => v.split(',').map(|s| s.trim().to_string()).collect(),
        _ => {
            eprintln!("sin CLICKHOUSE_LAB_URLS: no hay laboratorio, se salta");
            Vec::new()
        }
    }
}

fn lenta(url: &str, extra: &str) -> ore_driver::Peticion {
    ore_driver::leer_peticion(&format!(
        r#"{{"url":"{url}","objeto":"kit.lenta","proyeccion":{{"id":"id"}},"formato":"arrow"{extra}}}"#
    ))
    .expect("petición")
}

/// Las consultas de `kit.lenta` vivas en el servidor, sin contar la que pregunta.
fn vivas(url: &str) -> u64 {
    let c = Cliente::abrir(&url.replace("ore_r1@", "kit@")).unwrap();
    c.texto(
        "SELECT count() FROM system.processes WHERE query LIKE '%lenta%' AND query NOT LIKE '%system.processes%'",
    )
    .unwrap()
    .trim()
    .parse()
    .unwrap()
}

fn esperar_a_que_no_haya(url: &str, tope: Duration) -> Option<Duration> {
    let t = Instant::now();
    while t.elapsed() < tope {
        if vivas(url) == 0 {
            return Some(t.elapsed());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    None
}

#[test]
fn timeout_ms_corta_con_tiempo_y_no_deja_nada_vivo_con_cada_perfil() {
    for url in servidores() {
        // `ore_r1` es `readonly=1`: no puede mandar `max_execution_time`, así que corta el cliente.
        for usuario in ["kit", "ore_r1"] {
            let u = url.replace("kit@", &format!("{usuario}@"));
            let mut s = ClickHouse::conectar(&u).unwrap();
            let t = Instant::now();
            let f = ClickHouse::leer(
                &mut s,
                &Token::default(),
                &lenta(&u, r#","timeoutMs":1000"#),
                &mut Vec::new(),
            )
            .expect_err("timeout");
            assert_eq!(f.codigo, Codigo::Tiempo, "{u} ({}): {f:?}", s.version);
            assert!(
                t.elapsed() < Duration::from_secs(8),
                "{u}: {:?}",
                t.elapsed()
            );
            // Con `readonly=1` no se puede mandar `cancel_http_readonly_queries_on_client_close` ni, sin el permiso, hacer `KILL`: la consulta
            // que no emite nada sigue hasta que intente escribir y se encuentre la conexión cerrada (ADR 0077 §6, paso 6: límite medido).
            let (tope, que) = if usuario == "kit" {
                (12, "nada vivo")
            } else {
                (30, "viva hasta su primer bloque")
            };
            let hasta = esperar_a_que_no_haya(&u, Duration::from_secs(tope));
            assert!(hasta.is_some(), "{u}: sigue viva {tope} s después");
            eprintln!("{u}: {que}, a los {:?}", hasta.unwrap());
        }
    }
}

#[test]
fn cancelar_mata_la_consulta_en_el_servidor_y_la_lectura_acaba() {
    for url in servidores() {
        let u = url.clone();
        let token = Token::default();
        let t2 = token.clone();
        let hilo = std::thread::spawn(move || {
            let mut s = ClickHouse::conectar(&u).unwrap();
            ClickHouse::leer(&mut s, &t2, &lenta(&u, ""), &mut Vec::new())
        });
        std::thread::sleep(Duration::from_millis(1500));
        assert_eq!(
            vivas(&url),
            1,
            "{url}: la consulta debía estar en el servidor"
        );
        ClickHouse::cancelar(&token);
        let hasta = esperar_a_que_no_haya(&url, Duration::from_secs(12));
        assert!(
            hasta.is_some(),
            "{url}: sigue viva 12 s después de cancelar"
        );
        eprintln!(
            "{url}: cancelada y sin consulta viva a los {:?}",
            hasta.unwrap()
        );
        // La lectura acaba sola, con la cancelación como razón.
        let f = hilo.join().unwrap().expect_err("cancelada");
        assert!(f.mensaje.contains("cancelada"), "{url}: {f:?}");
    }
}
