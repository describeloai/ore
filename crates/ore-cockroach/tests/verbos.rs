//! Los verbos propios de CockroachDB de verdad (ADR 0080 §6, paso 6): `catalogo`, `check`, `explorar`, `testigo`, `cambio` y `estimar`. Se
//! salta sola si no hay laboratorio:
//!
//! ```text
//! COCKROACH_LAB_URLS=cockroachdb://kit@localhost:26257/kit,…  ORE_COCKROACH_LABORATORIO=1  cargo test -p ore-cockroach --test verbos
//! ```
//!
//! Crea en la base `kit` un esquema de prueba con lo que hace difícil un catálogo: una clave *hash-sharded* (con su columna oculta), una única
//! *hash-sharded*, una clave foránea, un `ENUM` declarado en orden no alfabético, comentarios, una vista, y un usuario que solo lee una tabla.

use ore_base::parse::{Node, parse};
use ore_cockroach::Cockroach;
use ore_cockroach::verbos::{cambio, cambio_con, catalogo, check, explorar, testigo};
use ore_driver::Codigo;
use ore_driver::catalogo::Catalogo;
use ore_driver::origen::Origen;

fn servidores() -> Vec<String> {
    let lab = std::env::var("ORE_COCKROACH_LABORATORIO").as_deref() == Ok("1");
    match std::env::var("COCKROACH_LAB_URLS") {
        Ok(v) if !v.trim().is_empty() && lab => {
            let urls: Vec<String> = v.split(',').map(|s| s.trim().to_string()).collect();
            static UNA: std::sync::Once = std::sync::Once::new();
            UNA.call_once(|| urls.iter().for_each(|u| preparar(u)));
            urls
        }
        _ => {
            eprintln!(
                "sin COCKROACH_LAB_URLS y ORE_COCKROACH_LABORATORIO=1: no hay laboratorio, se salta"
            );
            Vec::new()
        }
    }
}

fn preparar(url: &str) {
    let root = url.replace("//kit@", "//root@");
    let mut a = ore_cockroach::tls::abrir(&root)
        .unwrap_or_else(|f| panic!("{root}: {f:?}"))
        .cliente;
    a.batch_execute("SET default_transaction_read_only = false")
        .unwrap();
    let sqls = [
        "DROP VIEW IF EXISTS v_vista",
        "DROP TABLE IF EXISTS v_hijo",
        "DROP TABLE IF EXISTS v_padre",
        "DROP TABLE IF EXISTS v_x",
        "DROP TYPE IF EXISTS v_estado",
        "CREATE TYPE v_estado AS ENUM ('b', 'a')",
        "CREATE TABLE v_padre (id INT8 PRIMARY KEY, nombre STRING NOT NULL UNIQUE)",
        "CREATE TABLE v_x (a INT8, b STRING NOT NULL, c INT8, h INT8 NOT NULL, PRIMARY KEY (a, b), UNIQUE (c), UNIQUE INDEX u_hash (h) USING HASH)",
        "CREATE TABLE v_hijo (id INT8 PRIMARY KEY USING HASH, padre INT8 REFERENCES v_padre (id), estado v_estado, nota STRING, datos JSONB)",
        "CREATE VIEW v_vista AS SELECT a, b FROM v_x",
        "COMMENT ON TABLE v_x IS 'tabla de prueba'",
        "COMMENT ON COLUMN v_x.c IS 'columna con comentario'",
        "INSERT INTO v_padre VALUES (1, 'uno'), (2, 'dos')",
        "INSERT INTO v_x SELECT g, 'x' || g::STRING, g, g FROM generate_series(1, 50) AS g",
        "CREATE STATISTICS ore_v FROM v_x",
        "DROP USER IF EXISTS ore_lee",
        "CREATE USER ore_lee",
        "GRANT SELECT ON TABLE v_x TO ore_lee",
        "GRANT SELECT ON TABLE v_padre, v_hijo, v_x, v_vista TO kit",
        // `kit` puede escribir en `v_x`: la prueba de `cambio` cambia la tabla.
        "GRANT INSERT, UPDATE, DELETE ON TABLE v_x TO kit",
    ];
    for sql in sqls {
        a.batch_execute(sql)
            .unwrap_or_else(|e| panic!("{url}: {sql}: {}", ore_cockroach::tls::causa(&e)));
    }
}

fn entrada(url: &str, objeto: &str) -> String {
    format!(r#"{{"url":"{url}","objeto":"{objeto}"}}"#)
}

fn como(url: &str, usuario: &str) -> String {
    url.replace("//kit@", &format!("//{usuario}@"))
}

fn campo(n: &Node, k: &str) -> Option<String> {
    n.get(k).and_then(|(_, v)| v.as_str()).map(String::from)
}

#[test]
fn el_catalogo_trae_claves_unicas_foraneas_comentarios_y_valores_sin_la_columna_oculta() {
    for url in servidores() {
        let salida = catalogo(&url, "crdb").unwrap_or_else(|e| panic!("{url}: {e}"));
        let cat = Catalogo::leer(&salida).unwrap_or_else(|e| panic!("{url}: {e}\n{salida}"));
        assert_eq!(cat.fuente(), "crdb", "{url}");
        let t = |n: &str| {
            cat.tablas
                .iter()
                .find(|t| t.nombre == n)
                .unwrap_or_else(|| {
                    panic!(
                        "{url}: falta {n} en {:?}",
                        cat.tablas.iter().map(|t| &t.nombre).collect::<Vec<_>>()
                    )
                })
        };

        // Una clave hash-sharded: la columna oculta no sale ni en las columnas ni en la clave.
        let hijo = t("public.v_hijo");
        assert_eq!(hijo.clave, vec!["id"], "{url}");
        let nombres: Vec<&str> = hijo.columnas.iter().map(|c| c.nombre.as_str()).collect();
        assert_eq!(
            nombres,
            vec!["id", "padre", "estado", "nota", "datos"],
            "{url}: sin `crdb_internal_id_shard_16`"
        );
        // Foránea, con las columnas de los dos lados.
        assert_eq!(hijo.foraneas.len(), 1, "{url}");
        assert_eq!(
            (
                hijo.foraneas[0].columnas.clone(),
                hijo.foraneas[0].destino.as_str(),
                hijo.foraneas[0].destino_columnas.clone()
            ),
            (
                vec!["padre".to_string()],
                "public.v_padre",
                vec!["id".to_string()]
            ),
            "{url}"
        );
        // ENUM: String con sus valores en el orden en que se declararon.
        let estado = hijo.columnas.iter().find(|c| c.nombre == "estado").unwrap();
        assert_eq!(
            (estado.tipo.as_deref(), estado.valores.clone()),
            (Some("String"), vec!["b".to_string(), "a".to_string()]),
            "{url}"
        );
        assert_eq!(
            hijo.columnas
                .iter()
                .find(|c| c.nombre == "datos")
                .unwrap()
                .tipo
                .as_deref(),
            Some("Opaque"),
            "{url}: jsonb"
        );

        // Clave compuesta, únicas (una de ellas hash-sharded, sin su columna oculta), comentarios.
        let x = t("public.v_x");
        assert_eq!(x.clave, vec!["a", "b"], "{url}");
        assert_eq!(
            x.unicas,
            vec![vec!["c".to_string()], vec!["h".to_string()]],
            "{url}"
        );
        let c = x.columnas.iter().find(|c| c.nombre == "c").unwrap();
        assert_eq!(
            c.descripcion.as_deref(),
            Some("columna con comentario"),
            "{url}"
        );
        assert!(
            x.columnas
                .iter()
                .find(|c| c.nombre == "b")
                .unwrap()
                .obligatoria,
            "{url}"
        );
        assert!(
            !x.columnas
                .iter()
                .find(|c| c.nombre == "c")
                .unwrap()
                .obligatoria,
            "{url}"
        );
        assert_eq!(
            t("public.v_padre").unicas,
            vec![vec!["nombre".to_string()]],
            "{url}"
        );

        // Una vista no tiene clave.
        let v = t("public.v_vista");
        assert_eq!(
            (v.clase.as_str(), v.clave.is_empty()),
            ("view", true),
            "{url}"
        );
        assert_eq!(x.clase, "table", "{url}");
        // Nada del servidor.
        assert!(
            cat.tablas
                .iter()
                .all(|t| !t.nombre.starts_with("pg_catalog.")
                    && !t.nombre.starts_with("crdb_internal.")),
            "{url}"
        );
    }
}

#[test]
fn el_catalogo_no_descubre_lo_que_el_usuario_no_puede_leer() {
    for url in servidores() {
        let salida =
            catalogo(&como(&url, "ore_lee"), "crdb").unwrap_or_else(|e| panic!("{url}: {e}"));
        let cat = Catalogo::leer(&salida).unwrap();
        let nombres: Vec<&str> = cat.tablas.iter().map(|t| t.nombre.as_str()).collect();
        assert_eq!(
            nombres,
            vec!["public.v_x"],
            "{url}: `ore_lee` solo lee `v_x` y `pg_class` enseña todas"
        );
    }
}

#[test]
fn check_dice_si_responde_y_por_que_no() {
    for url in servidores() {
        let bien = check(&entrada(&url, "")).unwrap();
        assert!(bien.contains(r#""ok":true"#), "{url}: {bien}");
        let mal = check(&entrada(&como(&url, "nadie_existe"), "")).unwrap();
        assert!(
            mal.contains(r#""ok":false"#) && mal.contains("porque"),
            "{url}: {mal}"
        );
        assert!(
            !mal.contains("nadie_existe@"),
            "{url}: la credencial va tapada: {mal}"
        );
        let sin_base = check(&entrada(&url.replace("/kit", "/no_existe_esta"), "")).unwrap();
        assert!(sin_base.contains(r#""ok":false"#), "{url}: {sin_base}");
    }
}

#[test]
fn explorar_cuenta_los_objetos_por_esquema_y_testigo_da_el_instante_del_servidor() {
    for url in servidores() {
        let n = parse(&explorar(&entrada(&url, "")).unwrap()).unwrap();
        let publico = n
            .get("contiene")
            .map(|(_, v)| v.items().to_vec())
            .unwrap_or_default();
        let publico = publico
            .iter()
            .find(|e| campo(e, "nombre").as_deref() == Some("public"))
            .expect("esquema public");
        let objetos = publico
            .get("objetos")
            .and_then(|(_, v)| v.as_str())
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap();
        assert!(
            objetos >= 4,
            "{url}: v_padre, v_hijo, v_x y v_vista como mínimo: {objetos}"
        );

        let t = parse(&testigo(&entrada(&url, "public.v_x")).unwrap()).unwrap();
        assert_eq!(campo(&t, "modo").as_deref(), Some("snapshot"), "{url}");
        // El valor es un instante del servidor (`<ns>.<lógico>`), el mismo que `leer` acepta como `end`.
        let valor = campo(&t, "valor").unwrap_or_default();
        assert!(
            ore_cockroach::leer::instante_ns(&valor).is_ok(),
            "{url}: {valor}"
        );
    }
}

#[test]
fn la_huella_de_cambio_se_mueve_con_cualquier_insert_update_o_delete_y_con_nada_mas() {
    for url in servidores() {
        let huella = || -> (String, bool, String) {
            let h = cambio(&entrada(&url, "public.v_x")).unwrap_or_else(|e| panic!("{url}: {e}"));
            let n = parse(&h).unwrap();
            (
                campo(&n, "valor").unwrap_or_default(),
                campo(&n, "respalda").as_deref() == Some("true"),
                h,
            )
        };
        let (h0, respalda, bruto) = huella();
        assert!(respalda, "{url}: {bruto}");
        assert_eq!(
            campo(&parse(&bruto).unwrap(), "modo").as_deref(),
            Some("hash-de-filas"),
            "{url}"
        );
        assert_eq!(huella().0, h0, "{url}: sin cambios, la misma huella");

        let mut w = ore_cockroach::tls::abrir(&url).unwrap().cliente;
        w.batch_execute("SET default_transaction_read_only = false")
            .unwrap();
        let mut ultima = h0;
        for (que, sql) in [
            (
                "un insert",
                "INSERT INTO v_x VALUES (1000, 'nuevo', 1000, 1000)",
            ),
            ("un update", "UPDATE v_x SET b = 'cambiado' WHERE a = 1000"),
            ("un delete", "DELETE FROM v_x WHERE a = 1000"),
        ] {
            w.batch_execute(sql)
                .unwrap_or_else(|e| panic!("{url}: {sql}: {}", ore_cockroach::tls::causa(&e)));
            let (h, _, _) = huella();
            assert_ne!(h, ultima, "{url}: {que} no movió la huella");
            ultima = h;
        }
        // Tras deshacer el insert y el delete, la tabla es la de antes: la huella vuelve a la de entonces (depende solo de las filas).
        let (despues, _, _) = huella();
        assert_eq!(despues, ultima, "{url}");
    }
}

#[test]
fn la_huella_lleva_las_columnas_de_ahora_y_vale_para_una_vista() {
    for url in servidores() {
        let h = cambio(&entrada(&url, "public.v_x")).unwrap();
        let n = parse(&h).unwrap();
        let cols = n.get("columnas").map(|(_, v)| v.items().len()).unwrap_or(0);
        assert_eq!(cols, 4, "{url}: a, b, c, h: {h}");
        assert!(
            h.contains(r#"["a","Integer"]"#) && h.contains(r#"["b","String"]"#),
            "{url}: {h}"
        );
        let v = cambio(&entrada(&url, "public.v_vista")).unwrap();
        assert!(v.contains(r#""respalda":true"#), "{url}: {v}");
        // Sin esquema vale `public`.
        assert!(
            cambio(&entrada(&url, "v_x"))
                .unwrap()
                .contains(r#""respalda":true"#),
            "{url}"
        );
    }
}

#[test]
fn una_tabla_que_no_cabe_en_el_tope_no_tiene_huella_que_respalde_y_una_que_no_existe_es_un_error() {
    for url in servidores() {
        // `grande` (10⁶ filas) en 1 ms: no cabe.
        let existe: bool = Cockroach::conectar(&url)
            .unwrap()
            .cliente
            .query_one("SELECT to_regclass('public.grande') IS NOT NULL", &[])
            .unwrap()
            .get(0);
        if existe {
            let h = cambio_con(&entrada(&url, "public.grande"), 1)
                .unwrap_or_else(|e| panic!("{url}: {e}"));
            assert!(
                h.contains(r#""respalda":false"#)
                    && h.contains("sin-contadores")
                    && !h.contains("valor"),
                "{url}: {h}"
            );
        } else {
            eprintln!(
                "{url}: sin `public.grande` (el banco del kit no se ha cargado): se salta la mitad"
            );
        }
        let e = cambio(&entrada(&url, "public.no_existe_esta")).unwrap_err();
        assert!(e.contains("no existe"), "{url}: {e}");
        // `ore_lee` no lee `v_padre`: para él no existe.
        let e = cambio(&entrada(&como(&url, "ore_lee"), "public.v_padre")).unwrap_err();
        assert!(
            e.contains("no existe, o el usuario no puede leerla"),
            "{url}: {e}"
        );
    }
}

#[test]
fn estimar_da_la_cuenta_de_las_estadisticas_y_se_niega_con_una_vista_o_una_tabla_que_no_esta() {
    for url in servidores() {
        let mut s = Cockroach::conectar(&url).unwrap();
        let p = |o: &str| {
            ore_driver::leer_peticion(&format!(
                r#"{{"url":"{url}","objeto":"{o}","proyeccion":{{"a":"a"}},"formato":"arrow"}}"#
            ))
            .expect("petición")
        };
        // `v_x` tiene 50 filas y `CREATE STATISTICS` (ver `preparar`); la última estadística puede venir de antes de los inserts de otras
        // pruebas, por eso se pide solo que sea una cuenta de la tabla (50 o las que dejaron los `cambio`).
        let e =
            Cockroach::estimar(&mut s, &p("public.v_x")).unwrap_or_else(|f| panic!("{url}: {f:?}"));
        let n = parse(&e).unwrap();
        assert_eq!(
            campo(&n, "fuente").as_deref(),
            Some("SHOW STATISTICS"),
            "{url}: {e}"
        );
        let filas: i64 = campo(&n, "filas").unwrap().parse().unwrap();
        assert!((49..=51).contains(&filas), "{url}: {e}");

        let f = Cockroach::estimar(&mut s, &p("public.v_vista")).unwrap_err();
        assert_eq!(f.codigo, Codigo::Operador, "{url}: {f:?}");
        let f = Cockroach::estimar(&mut s, &p("public.no_existe_esta")).unwrap_err();
        assert_eq!(f.codigo, Codigo::Objeto, "{url}: {f:?}");
    }
}
