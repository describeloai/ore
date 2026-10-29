//! `timeoutMs` de CockroachDB de verdad (ADR 0080 §6, paso 5): el tiempo lo corta el servidor, también en la lectura en tramos, y el límite no se
//! queda en la sesión. Se salta sola si no hay laboratorio:
//!
//! ```text
//! COCKROACH_LAB_URLS=cockroachdb://kit@localhost:26257/kit,…  ORE_COCKROACH_LABORATORIO=1  cargo test -p ore-cockroach --test tiempo
//! ```
//!
//! Usa `public.lenta` y `public.grande` (las deja el banco del kit: corre `ore-kit --banco cockroach` antes) y `lab_filtros` (de `--test filtros`).

use ore_cockroach::Cockroach;
use ore_cockroach::lectura::Token;
use ore_driver::origen::Origen;
use ore_driver::{Codigo, Fallo};
use std::time::{Duration, Instant};

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

fn leer(
    s: &mut ore_cockroach::lectura::Sesion,
    url: &str,
    objeto: &str,
    extra: &str,
) -> Result<u64, Fallo> {
    let p = ore_driver::leer_peticion(&format!(
        r#"{{"url":"{url}","objeto":"{objeto}","proyeccion":{{"id":"id"}},"formato":"arrow"{extra}}}"#
    ))
    .expect("petición");
    Cockroach::leer(s, &Token::default(), &p, &mut Vec::new())
}

/// ¿Está el objeto? Las tablas del kit no existen hasta que el banco las carga. Se mira el catálogo: leer `lenta` costaría sus 9 s.
fn hay(url: &str, objeto: &str) -> bool {
    let mut s = Cockroach::conectar(url).unwrap();
    s.cliente
        .query_one(
            "SELECT to_regclass($1) IS NOT NULL",
            &[&ore_cockroach::leer::citado(objeto)],
        )
        .map(|f| f.get::<_, bool>(0))
        .unwrap_or(false)
}

#[test]
fn el_servidor_corta_una_consulta_que_no_da_ni_una_fila() {
    for url in servidores() {
        if !hay(&url, "public.lenta") {
            eprintln!("{url}: sin `public.lenta` (corre el banco del kit antes): se salta");
            continue;
        }
        let mut s = Cockroach::conectar(&url).unwrap();
        let t = Instant::now();
        // `lenta` duerme 9 s antes de su primera fila: el reloj del cliente no llega a mirarse, quien corta es el servidor.
        let f = leer(&mut s, &url, "public.lenta", r#","timeoutMs":1000"#)
            .expect_err("debía agotar el tiempo");
        let tardo = t.elapsed();
        assert_eq!(f.codigo, Codigo::Tiempo, "{url}: {f:?} en {tardo:?}");
        assert!(f.reintentable, "{url}: {f:?}");
        assert!(
            tardo < Duration::from_millis(3000),
            "{url}: tardó {tardo:?} con timeoutMs=1000 (sin él, 9 s): {f:?}"
        );
        assert!(
            f.mensaje.contains("57014"),
            "{url}: lo dice el servidor, no el reloj del cliente: {f:?}"
        );
        assert!(
            tardo < Duration::from_millis(3000),
            "{url}: tardó {tardo:?} con timeoutMs=1000 (sin él, 9 s)"
        );
        assert!(
            tardo >= Duration::from_millis(900),
            "{url}: cortó antes de tiempo: {tardo:?}"
        );
    }
}

#[test]
fn el_tiempo_corta_tambien_la_lectura_en_tramos_y_no_deja_consultas_vivas() {
    for url in servidores() {
        if !hay(&url, "public.grande") {
            eprintln!("{url}: sin `public.grande`: se salta");
            continue;
        }
        let mut s = Cockroach::conectar(&url).unwrap();
        // 10⁶ filas repartidas en 4 conexiones tardan más de 30 ms, aunque sea poco.
        let t = Instant::now();
        let f = leer(&mut s, &url, "public.grande", r#","timeoutMs":30"#)
            .expect_err("debía agotar el tiempo");
        assert_eq!(f.codigo, Codigo::Tiempo, "{url}: {f:?}");
        assert!(
            t.elapsed() < Duration::from_secs(3),
            "{url}: {:?}",
            t.elapsed()
        );
        // Nada suyo sigue vivo en el servidor.
        std::thread::sleep(Duration::from_millis(400));
        let root = url.replace("//kit@", "//root@");
        let mut m = ore_cockroach::tls::abrir(&root).unwrap().cliente;
        let vivas: i64 = m
            .query_one(
                "SELECT count(*) FROM [SHOW CLUSTER QUERIES] WHERE application_name = 'ore-read-cockroach' AND query LIKE '%grande%' \
                 AND query NOT LIKE '%CLUSTER QUERIES%'",
                &[],
            )
            .unwrap()
            .get(0);
        assert_eq!(
            vivas, 0,
            "{url}: tras agotar el tiempo quedan {vivas} consultas vivas"
        );
    }
}

#[test]
fn un_orden_caro_sin_indice_tambien_se_corta() {
    for url in servidores() {
        if !hay(&url, "public.grande") {
            continue;
        }
        let mut s = Cockroach::conectar(&url).unwrap();
        // Ordenar 10⁶ filas por una columna de texto sin índice: una sola conexión, sin reparto.
        let f = leer(
            &mut s,
            &url,
            "public.grande",
            r#","orderBy":[{"columna":"nota","direccion":"desc"}],"timeoutMs":50"#,
        )
        .expect_err("debía agotar el tiempo");
        assert_eq!(f.codigo, Codigo::Tiempo, "{url}: {f:?}");
    }
}

#[test]
fn con_tiempo_de_sobra_lee_y_el_limite_no_se_queda_en_la_sesion() {
    for url in servidores() {
        if !hay(&url, "public.grande") {
            continue;
        }
        let mut s = Cockroach::conectar(&url).unwrap();
        // Con holgura, la lectura entera sale.
        let n = leer(&mut s, &url, "public.grande", r#","timeoutMs":120000"#)
            .unwrap_or_else(|f| panic!("{url}: {f:?}"));
        assert_eq!(n, 1_000_000, "{url}");
        // Tras agotar el tiempo, la misma sesión sigue sirviendo y **sin** límite: `SET LOCAL` no se filtra a la sesión.
        assert!(leer(&mut s, &url, "public.grande", r#","timeoutMs":30"#).is_err());
        let ts: String = s
            .cliente
            .query_one("SHOW statement_timeout", &[])
            .unwrap()
            .get(0);
        assert!(
            ts == "0" || ts == "0s" || ts.starts_with("0 "),
            "{url}: la sesión se quedó con statement_timeout = {ts}"
        );
        let n = leer(
            &mut s,
            &url,
            "public.grande",
            r#","orderBy":[{"columna":"id","direccion":"asc"}],"limit":10"#,
        )
        .unwrap_or_else(|f| panic!("{url}: {f:?}"));
        assert_eq!(n, 10, "{url}");
    }
}

#[test]
fn un_tiempo_que_alcanza_no_cambia_el_resultado() {
    for url in servidores() {
        let mut s = Cockroach::conectar(&url).unwrap();
        // `lab_filtros` (9 filas) la crea `--test filtros`; si no está, se salta.
        match leer(&mut s, &url, "public.lab_filtros", r#","timeoutMs":5000"#) {
            Ok(n) => assert_eq!(n, 9, "{url}"),
            Err(f) if f.codigo == Codigo::Objeto => eprintln!("{url}: sin `lab_filtros`: se salta"),
            Err(f) => panic!("{url}: {f:?}"),
        }
    }
}
