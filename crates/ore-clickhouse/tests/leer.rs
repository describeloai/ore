//! `leer` contra servidores ClickHouse de verdad (ADR 0077 §6, paso 3). Se salta sola si no hay laboratorio
//! (`CLICKHOUSE_LAB_URLS`, ver `tests/conectar.rs`). Necesita la tabla `kit.tipos` que carga el banco del kit
//! (`ore-kit --banco clickhouse`).

use ore_clickhouse::ClickHouse;
use ore_clickhouse::lectura::Token;
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

fn peticion(url: &str, objeto: &str, proy: &str) -> ore_driver::Peticion {
    ore_driver::leer_peticion(&format!(
        r#"{{"url":"{url}","objeto":"{objeto}","proyeccion":{proy},"formato":"arrow"}}"#
    ))
    .expect("petición")
}

#[test]
fn la_semilla_se_lee_entera_con_cada_usuario() {
    for url in servidores() {
        for usuario in ["kit", "ore_r1", "ore_r2"] {
            let u = url.replace("kit@", &format!("{usuario}@"));
            let mut s = ClickHouse::conectar(&u).unwrap_or_else(|f| panic!("{u}: {f:?}"));
            let p = peticion(
                &u,
                "kit.tipos",
                r#"{"id":"id","importe":"importe","texto":"texto","fecha":"fecha","t":"instante","b":"logico"}"#,
            );
            let mut salida = Vec::new();
            let n = ClickHouse::leer(&mut s, &Token::default(), &p, &mut salida)
                .unwrap_or_else(|f| panic!("{u} ({}): {f:?}", s.version));
            assert!(n >= 5, "{u}: {n} filas");
            assert!(salida.len() > 100, "{u}: {} bytes", salida.len());
        }
    }
}

#[test]
fn una_tabla_que_no_existe_es_un_objeto_y_una_columna_que_no_existe_un_esquema() {
    for url in servidores() {
        let mut s = ClickHouse::conectar(&url).unwrap();
        let f = ClickHouse::leer(
            &mut s,
            &Token::default(),
            &peticion(&url, "kit.no_existe_nunca", r#"{"a":"id"}"#),
            &mut Vec::new(),
        )
        .expect_err("tabla inexistente");
        assert_eq!(f.codigo, Codigo::Objeto, "{url}: {f:?}");
        let f = ClickHouse::leer(
            &mut s,
            &Token::default(),
            &peticion(&url, "kit.tipos", r#"{"a":"no_existe_nunca"}"#),
            &mut Vec::new(),
        )
        .expect_err("columna inexistente");
        assert_eq!(f.codigo, Codigo::Esquema, "{url}: {f:?}");
    }
}

fn con_filtros(url: &str, extra: &str) -> ore_driver::Peticion {
    ore_driver::leer_peticion(&format!(
        r#"{{"url":"{url}","objeto":"kit.tipos","proyeccion":{{"id":"id"}},"formato":"arrow",{extra}}}"#
    ))
    .expect("petición")
}

#[test]
fn los_filtros_el_orden_y_el_limite_se_empujan_con_cada_usuario() {
    for url in servidores() {
        for usuario in ["kit", "ore_r1", "ore_r2"] {
            let u = url.replace("kit@", &format!("{usuario}@"));
            let mut s = ClickHouse::conectar(&u).unwrap_or_else(|f| panic!("{u}: {f:?}"));
            // `readonly=1` no admite ajustes por consulta, pero sí parámetros del servidor.
            let p = con_filtros(
                &u,
                r#""filtros":[{"columna":"entero","operador":"in","valor":["1","2","99"]},{"columna":"texto","operador":"like","valor":"%a%"}],
                   "orderBy":[{"columna":"id","direccion":"desc"}],"limit":2"#,
            );
            let n = ClickHouse::leer(&mut s, &Token::default(), &p, &mut Vec::new())
                .unwrap_or_else(|f| panic!("{u} ({}): {f:?}", s.version));
            assert!(n <= 2, "{u}: {n} filas con limit 2");
        }
    }
}

#[test]
fn un_entero_que_no_cabe_en_la_columna_no_llega_al_servidor() {
    for url in servidores() {
        let c = ore_clickhouse::transporte::Cliente::abrir(&url).unwrap();
        c.ejecutar("DROP TABLE IF EXISTS kit.lab_pequeno").unwrap();
        c.ejecutar(
            "CREATE TABLE kit.lab_pequeno (id Int64, p UInt8) ENGINE = MergeTree ORDER BY id",
        )
        .unwrap();
        // 44 es lo que el servidor haría de 300 si lo dejáramos pasar (UInt8 desborda en silencio).
        c.ejecutar("INSERT INTO kit.lab_pequeno VALUES (1, 44), (2, 200)")
            .unwrap();
        let mut s = ClickHouse::conectar(&url).unwrap();
        let p = ore_driver::leer_peticion(&format!(
            r#"{{"url":"{url}","objeto":"kit.lab_pequeno","proyeccion":{{"id":"id"}},"formato":"arrow",
                "filtros":[{{"columna":"p","operador":"eq","valor":"300"}}]}}"#
        ))
        .unwrap();
        let f = ClickHouse::leer(&mut s, &Token::default(), &p, &mut Vec::new())
            .expect_err("300 no cabe en UInt8");
        assert_eq!(f.codigo, Codigo::Operador, "{url}: {f:?}");
        let p = ore_driver::leer_peticion(&format!(
            r#"{{"url":"{url}","objeto":"kit.lab_pequeno","proyeccion":{{"id":"id"}},"formato":"arrow",
                "filtros":[{{"columna":"p","operador":"eq","valor":"44"}}]}}"#
        ))
        .unwrap();
        let n = ClickHouse::leer(&mut s, &Token::default(), &p, &mut Vec::new()).unwrap();
        assert_eq!(n, 1, "{url}");
        c.ejecutar("DROP TABLE kit.lab_pequeno").unwrap();
    }
}

#[test]
fn estimar_dice_las_filas_sin_leer_y_una_vista_no_estima() {
    for url in servidores() {
        let mut s = ClickHouse::conectar(&url).unwrap();
        let dice = ClickHouse::estimar(&mut s, &con_filtros(&url, r#""filtros":[]"#).clone());
        // `kit.tipos` tiene la semilla (11 filas); el objeto de `con_filtros` es esa tabla.
        let e = dice.unwrap_or_else(|f| panic!("{url}: {f:?}"));
        assert!(
            e.contains(r#""filas":"#) && e.contains("system.tables"),
            "{url}: {e}"
        );
        let mut grande = con_filtros(&url, r#""filtros":[]"#);
        grande.objeto = "kit.grande".into();
        let e = ClickHouse::estimar(&mut s, &grande).unwrap();
        assert!(e.contains(r#""filas":1000000"#), "{url}: {e}");
        let mut vista = con_filtros(&url, r#""filtros":[]"#);
        vista.objeto = "kit.lenta".into();
        let f = ClickHouse::estimar(&mut s, &vista).expect_err("una vista no estima");
        assert_eq!(f.codigo, Codigo::Operador, "{url}: {f:?}");
        let mut nada = con_filtros(&url, r#""filtros":[]"#);
        nada.objeto = "kit.no_existe_nunca".into();
        let f = ClickHouse::estimar(&mut s, &nada).expect_err("no existe");
        assert_eq!(f.codigo, Codigo::Objeto, "{url}: {f:?}");
    }
}
