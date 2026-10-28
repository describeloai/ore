//! El transporte contra servidores ClickHouse de verdad (ADR 0077 §4). Se salta sola si no hay laboratorio:
//!
//! ```text
//! CLICKHOUSE_LAB_URLS=clickhouse://kit@localhost:8123/kit,clickhouse://kit@localhost:8124/kit  cargo test -p ore-clickhouse --test laboratorio
//! ```
//!
//! Cada URL es un servidor (el laboratorio tiene 24.8, 25.3, 25.8 y 26.4) con un usuario `kit` sin contraseña.

use ore_clickhouse::transporte::{Cliente, Param, fallo_de_io};
use ore_driver::Codigo;
use std::io::Read;

fn servidores() -> Vec<String> {
    match std::env::var("CLICKHOUSE_LAB_URLS") {
        Ok(v) if !v.trim().is_empty() => v.split(',').map(|s| s.trim().to_string()).collect(),
        _ => {
            eprintln!("sin CLICKHOUSE_LAB_URLS: no hay laboratorio, se salta");
            Vec::new()
        }
    }
}

fn en_cada_servidor(prueba: impl Fn(&str, &Cliente, &str)) {
    for url in servidores() {
        let c = Cliente::abrir(&url).unwrap_or_else(|f| panic!("{url}: {f:?}"));
        let version = c
            .texto("SELECT version()")
            .unwrap_or_else(|f| panic!("{url}: {f:?}"));
        prueba(&url, &c, version.trim());
    }
}

#[test]
fn la_version_responde() {
    en_cada_servidor(|url, _, v| {
        eprintln!("{url} → {v}");
        assert!(v.split('.').count() >= 3, "{url}: {v}");
    });
}

#[test]
fn una_tabla_que_no_existe_es_un_objeto_y_un_usuario_que_no_existe_es_una_credencial() {
    en_cada_servidor(|url, c, v| {
        let f = c
            .texto("SELECT * FROM kit.no_existe_nunca")
            .expect_err("tabla inexistente");
        assert_eq!(f.codigo, Codigo::Objeto, "{url} {v}: {f:?}");
        let malo = Cliente::abrir(&url.replace("kit@", "nadie@")).unwrap();
        let f = malo.texto("SELECT 1").expect_err("usuario inexistente");
        assert_eq!(f.codigo, Codigo::Credencial, "{url} {v}: {f:?}");
    });
}

#[test]
fn una_columna_que_no_existe_es_un_cambio_de_esquema() {
    en_cada_servidor(|url, c, v| {
        c.ejecutar(
            "CREATE TABLE IF NOT EXISTS kit.lab_esquema (id Int64) ENGINE = MergeTree ORDER BY id",
        )
        .unwrap();
        let f = c
            .texto("SELECT nada FROM kit.lab_esquema")
            .expect_err("columna inexistente");
        assert_eq!(f.codigo, Codigo::Esquema, "{url} {v}: {f:?}");
    });
}

#[test]
fn un_flujo_grande_llega_entero() {
    en_cada_servidor(|url, c, v| {
        let mut f = c
            .flujo(
                "SELECT number FROM numbers(3000000)",
                "RowBinary",
                &[],
                &[],
                None,
            )
            .unwrap();
        let mut n = 0u64;
        let mut buf = vec![0u8; 1 << 16];
        loop {
            match f.read(&mut buf) {
                Ok(0) => break,
                Ok(k) => n += k as u64,
                Err(e) => panic!("{url} {v}: {:?}", fallo_de_io(&e)),
            }
        }
        assert_eq!(n, 3_000_000 * 8, "{url} {v}");
    });
}

/// **La prueba que importa**: un error a mitad de flujo, en cada versión, llega como error y con los datos de antes.
#[test]
fn un_error_a_mitad_de_flujo_es_un_error_en_cada_version() {
    en_cada_servidor(|url, c, v| {
        let mut f = c
            .flujo(
                "SELECT number, throwIf(number = 5000000, 'boom a mitad') FROM numbers(10000000)",
                "RowBinary",
                &[],
                &[],
                None,
            )
            .unwrap();
        let mut n = 0u64;
        let mut buf = vec![0u8; 1 << 16];
        let fallo = loop {
            match f.read(&mut buf) {
                Ok(0) => panic!(
                    "{url} {v}: el flujo terminó como bueno con {n} bytes: el error se perdió"
                ),
                Ok(k) => n += k as u64,
                Err(e) => break fallo_de_io(&e),
            }
        };
        assert!(fallo.mensaje.contains("Code: 395"), "{url} {v}: {fallo:?}");
        assert!(
            fallo.mensaje.contains("boom a mitad"),
            "{url} {v}: {fallo:?}"
        );
        assert!(
            !fallo.mensaje.contains("__exception__"),
            "{url} {v}: el marco del error llegó al mensaje: {}",
            fallo.mensaje
        );
        // Los datos de antes del error **pueden terminar a mitad de una fila** (medido: 34 732 179 bytes no son múltiplo de los 9 de la
        // fila): el servidor vuelca lo que tenía en el búfer cuando falla. Quien decodifica tiene que descartar la fila parcial
        // cuando llega el error. Lo que sí se cumple: no llega más allá de donde falla (la fila 5 000 000) más un bloque.
        assert!(n <= (5_000_000 + 2 * 65_409) * 9, "{url} {v}: {n} bytes");
        eprintln!(
            "{url} {v}: {n} bytes de datos y luego `{}`",
            &fallo.mensaje[..40.min(fallo.mensaje.len())]
        );
    });
}

#[test]
fn un_parametro_con_de_todo_llega_intacto() {
    en_cada_servidor(|url, c, v| {
        let valor = "a b+c&d=e\\N 日本 'x' \"y\"";
        let mut f = c
            .flujo(
                "SELECT {x:String}",
                "RowBinary",
                &[("x", Param::Texto(valor))],
                &[],
                None,
            )
            .unwrap();
        let mut b = Vec::new();
        f.read_to_end(&mut b)
            .unwrap_or_else(|e| panic!("{url} {v}: {:?}", fallo_de_io(&e)));
        // RowBinary de un String: longitud LEB128 y los bytes.
        assert_eq!(b[0] as usize, valor.len(), "{url} {v}");
        assert_eq!(&b[1..], valor.as_bytes(), "{url} {v}");
    });
}
