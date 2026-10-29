//! La lectura en tramos de CockroachDB de verdad (ADR 0080 §6, paso 3): cuándo se reparte, que devuelve las mismas filas que una
//! conexión, que todas las conexiones leen **de un mismo instante** y que cancelar corta todas. Se salta sola si no hay laboratorio:
//!
//! ```text
//! COCKROACH_LAB_URLS=cockroachdb://kit@localhost:26257/kit,…  ORE_COCKROACH_LABORATORIO=1  cargo test -p ore-cockroach --test reparto
//! ```
//!
//! Crea tablas de 250 000 a 300 000 filas en la base `kit` de cada nodo la primera vez (de medio minuto a unos minutos según la versión);
//! después se reusan. Los tests comparten esas tablas y miran las sesiones del servidor, así que corren de uno en uno.

use arrow_array::RecordBatch;
use arrow_array::cast::AsArray;
use arrow_array::types::Int64Type;
use arrow_ipc::reader::StreamReader;
use ore_cockroach::Cockroach;
use ore_cockroach::lectura::Token;
use ore_cockroach::leer::{CLAVES_PARA_REPARTIR, reparto};
use ore_driver::origen::Origen;
use ore_driver::{Codigo, Fallo};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// Las filas de las tablas grandes (más que [`CLAVES_PARA_REPARTIR`]).
const N: i64 = 300_000;
const N_HASH: i64 = 250_000;

static UNO_A_UNO: Mutex<()> = Mutex::new(());

fn servidores() -> Vec<String> {
    let lab = std::env::var("ORE_COCKROACH_LABORATORIO").as_deref() == Ok("1");
    match std::env::var("COCKROACH_LAB_URLS") {
        Ok(v) if !v.trim().is_empty() && lab => {
            let urls: Vec<String> = v.split(',').map(|s| s.trim().to_string()).collect();
            // Los tests comparten las tablas: se preparan una sola vez por proceso.
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

fn escritura(url: &str) -> postgres::Client {
    let root = url.replace("//kit@", "//root@");
    let mut a = ore_cockroach::tls::abrir(&root)
        .unwrap_or_else(|f| panic!("{root}: {f:?}"))
        .cliente;
    a.batch_execute("SET default_transaction_read_only = false")
        .unwrap();
    a
}

fn contar(a: &mut postgres::Client, tabla: &str) -> i64 {
    a.query_one(&format!("SELECT count(*) FROM {tabla}"), &[])
        .map(|f| f.get(0))
        .unwrap_or(0)
}

/// Crea (si no están enteras) las tablas del test. Idempotente: son 10⁵ filas, no se rehacen en cada vuelta.
fn preparar(url: &str) {
    let mut a = escritura(url);
    let mut ejecutar = |sql: &str| {
        a.batch_execute(sql)
            .unwrap_or_else(|e| panic!("{url}: {sql}: {}", ore_cockroach::tls::causa(&e)))
    };
    if contar(&mut escritura(url), "lab_reparto") != N {
        ejecutar("DROP TABLE IF EXISTS lab_reparto");
        ejecutar(
            "CREATE TABLE lab_reparto (id INT8 PRIMARY KEY, v INT8 NOT NULL DEFAULT 0, texto STRING)",
        );
        for desde in (1..=N).step_by(100_000) {
            let hasta = (desde + 99_999).min(N);
            ejecutar(&format!(
                "INSERT INTO lab_reparto SELECT g, 0, 'fila-' || g::STRING FROM generate_series({desde}, {hasta}) AS g"
            ));
        }
    }
    if contar(&mut escritura(url), "lab_hash") != N_HASH {
        ejecutar("DROP TABLE IF EXISTS lab_hash");
        // La clave *hash-sharded* lleva una columna oculta delante: es la trampa que Fivetran listó.
        ejecutar("CREATE TABLE lab_hash (id INT8 PRIMARY KEY USING HASH, texto STRING)");
        for desde in (1..=N_HASH).step_by(125_000) {
            let hasta = (desde + 124_999).min(N_HASH);
            ejecutar(&format!(
                "INSERT INTO lab_hash SELECT g, 'fila-' || g::STRING FROM generate_series({desde}, {hasta}) AS g"
            ));
        }
    }
    ejecutar("DROP TABLE IF EXISTS lab_pequena");
    ejecutar("CREATE TABLE lab_pequena (id INT8 PRIMARY KEY, texto STRING)");
    ejecutar(
        "INSERT INTO lab_pequena SELECT g, 'fila-' || g::STRING FROM generate_series(1, 1000) AS g",
    );
    ejecutar("DROP TABLE IF EXISTS lab_uuid");
    ejecutar("CREATE TABLE lab_uuid (id UUID PRIMARY KEY DEFAULT gen_random_uuid(), n INT8)");
    ejecutar("INSERT INTO lab_uuid (n) SELECT g FROM generate_series(1, 1000) AS g");
    ejecutar("DROP TABLE IF EXISTS lab_escasa");
    ejecutar("CREATE TABLE lab_escasa (id INT8 PRIMARY KEY, texto STRING)");
    // Pocas filas con una clave muy abierta: mucho rango, casi nada dentro. Todas deben salir.
    ejecutar(
        "INSERT INTO lab_escasa SELECT g * 1000000, 'fila-' || g::STRING FROM generate_series(1, 5) AS g",
    );
    for t in [
        "lab_reparto",
        "lab_hash",
        "lab_pequena",
        "lab_uuid",
        "lab_escasa",
    ] {
        ejecutar(&format!("GRANT SELECT ON TABLE {t} TO kit"));
    }
    ejecutar("GRANT UPDATE ON TABLE lab_reparto TO kit");
}

fn peticion(url: &str, objeto: &str, proy: &str, extra: &str) -> ore_driver::Peticion {
    ore_driver::leer_peticion(&format!(
        r#"{{"url":"{url}","objeto":"{objeto}","proyeccion":{proy},"formato":"arrow"{extra}}}"#
    ))
    .expect("petición")
}

fn lotes(salida: &[u8]) -> Vec<RecordBatch> {
    StreamReader::try_new(salida, None)
        .expect("flujo Arrow")
        .map(|l| l.expect("lote"))
        .collect()
}

fn leer(url: &str, objeto: &str, proy: &str) -> Result<Vec<RecordBatch>, Fallo> {
    let mut s = Cockroach::conectar(url)?;
    let mut salida = Vec::new();
    Cockroach::leer(
        &mut s,
        &Token::default(),
        &peticion(url, objeto, proy, ""),
        &mut salida,
    )?;
    Ok(lotes(&salida))
}

/// Con cuántas conexiones lee `f`: el máximo de sesiones de `ore-read-cockroach` que se ven en el servidor mientras corre.
fn conexiones_simultaneas<T>(url: &str, f: impl FnOnce() -> T) -> (T, usize) {
    let mut config: postgres::Config = url
        .split('?')
        .next()
        .unwrap()
        .replace("cockroachdb://", "postgresql://")
        .replace("//kit@", "//root@")
        .parse()
        .unwrap();
    config.application_name("monitor");
    let mut m = config.connect(postgres::NoTls).unwrap();
    let parar = AtomicBool::new(false);
    let maximo = AtomicUsize::new(0);
    let r = std::thread::scope(|s| {
        s.spawn(|| {
            while !parar.load(Ordering::SeqCst) {
                let n: i64 = m
                    .query_one("SELECT count(*) FROM [SHOW CLUSTER SESSIONS] WHERE application_name = 'ore-read-cockroach'", &[])
                    .map(|f| f.get(0))
                    .unwrap_or(0);
                maximo.fetch_max(n as usize, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(15));
            }
        });
        let r = f();
        parar.store(true, Ordering::SeqCst);
        r
    });
    (r, maximo.load(Ordering::SeqCst))
}

#[test]
fn se_reparte_por_la_primera_columna_entera_de_la_clave_y_no_en_lo_demas() {
    let _g = UNO_A_UNO.lock().unwrap_or_else(|e| e.into_inner());
    for url in servidores() {
        let mut c = ore_cockroach::tls::abrir(&url).unwrap().cliente;
        let r = |c: &mut postgres::Client, objeto: &str, extra: &str| {
            reparto(c, &peticion(&url, objeto, r#"{"id":"id"}"#, extra))
                .unwrap_or_else(|f| panic!("{url} {objeto}: {f:?}"))
        };
        let (ts, grande) = r(&mut c, "public.lab_reparto", "")
            .unwrap_or_else(|| panic!("{url}: no repartió una tabla de {N} filas"));
        assert_eq!(
            (grande.columna.as_str(), grande.minimo, grande.maximo),
            ("id", 1, N as i128),
            "{url}"
        );
        assert!(
            ts.contains('.'),
            "{url}: el instante es un HLC (`<ns>.<lógico>`): {ts}"
        );
        // Una clave hash-sharded lleva una columna oculta delante: se parte por `id`, no por ella.
        let (_, hash) = r(&mut c, "public.lab_hash", "")
            .unwrap_or_else(|| panic!("{url}: no repartió la hash-sharded"));
        assert_eq!(
            (hash.columna.as_str(), hash.minimo, hash.maximo),
            ("id", 1, N_HASH as i128),
            "{url}"
        );
        assert!(
            r(&mut c, "lab_reparto", "").is_some(),
            "{url}: sin esquema vale `public`"
        );
        // Lo que no se reparte:
        assert!(
            r(&mut c, "public.lab_pequena", "").is_none(),
            "{url}: mil filas no se reparten"
        );
        assert!(
            r(&mut c, "public.lab_uuid", "").is_none(),
            "{url}: una clave UUID no se parte por rangos de enteros"
        );
        assert!(
            r(&mut c, "public.lenta", "").is_none(),
            "{url}: una vista no tiene clave"
        );
        assert!(
            r(&mut c, "kit.public.lab_reparto", "").is_none(),
            "{url}: tres partes"
        );
        assert!(
            r(&mut c, "public.lab_reparto", r#","limit":10"#).is_none(),
            "{url}: con límite se lee en orden, con una conexión"
        );
        // Mucho rango y pocas filas: el criterio es el ancho de la clave, que es lo que cuesta partir.
        let (_, escasa) = r(&mut c, "public.lab_escasa", "").unwrap();
        assert!(
            escasa.maximo - escasa.minimo + 1 >= CLAVES_PARA_REPARTIR,
            "{url}"
        );
    }
}

#[test]
fn leer_en_tramos_da_las_mismas_filas_que_con_una_conexion_y_en_varias() {
    let _g = UNO_A_UNO.lock().unwrap_or_else(|e| e.into_inner());
    for url in servidores() {
        for (objeto, n) in [("public.lab_reparto", N), ("public.lab_hash", N_HASH)] {
            let ((varias, concurrencia), (una, solo_una)) = (
                conexiones_simultaneas(&url, || {
                    leer(&url, objeto, r#"{"id":"id","texto":"texto"}"#)
                }),
                conexiones_simultaneas(&format!("{url}?tramos=1"), || {
                    leer(
                        &format!("{url}?tramos=1"),
                        objeto,
                        r#"{"id":"id","texto":"texto"}"#,
                    )
                }),
            );
            let (varias, una) = (
                varias.unwrap_or_else(|f| panic!("{url}: {f:?}")),
                una.unwrap_or_else(|f| panic!("{url}: {f:?}")),
            );
            assert!(
                concurrencia >= 3,
                "{url} {objeto}: en tramos se vieron {concurrencia} sesiones a la vez (se esperan 4 + la de la petición)"
            );
            assert!(
                solo_una <= 1,
                "{url} {objeto}: con `tramos=1` se vieron {solo_una} sesiones a la vez"
            );

            for (etiqueta, bs) in [("varias", &varias), ("una", &una)] {
                let mut ids: Vec<i64> = bs
                    .iter()
                    .flat_map(|b| b.column(0).as_primitive::<Int64Type>().values().to_vec())
                    .collect();
                // Cada fila, entera y una sola vez, y la columna de al lado es la de su fila.
                assert_eq!(ids.len() as i64, n, "{url} {objeto} ({etiqueta}): filas");
                for b in bs.iter() {
                    let (id, tx) = (
                        b.column(0).as_primitive::<Int64Type>(),
                        b.column(1).as_string::<i32>(),
                    );
                    for i in 0..b.num_rows() {
                        assert_eq!(
                            tx.value(i),
                            format!("fila-{}", id.value(i)),
                            "{url} {objeto} ({etiqueta})"
                        );
                    }
                }
                ids.sort_unstable();
                assert!(
                    ids.iter().copied().eq(1..=n),
                    "{url} {objeto} ({etiqueta}): faltan o sobran ids"
                );
            }
        }
    }
}

#[test]
fn una_tabla_de_clave_escasa_se_lee_entera_aunque_casi_todos_los_trozos_esten_vacios() {
    let _g = UNO_A_UNO.lock().unwrap_or_else(|e| e.into_inner());
    for url in servidores() {
        let bs = leer(&url, "public.lab_escasa", r#"{"id":"id"}"#)
            .unwrap_or_else(|f| panic!("{url}: {f:?}"));
        let mut ids: Vec<i64> = bs
            .iter()
            .flat_map(|b| b.column(0).as_primitive::<Int64Type>().values().to_vec())
            .collect();
        ids.sort_unstable();
        assert_eq!(
            ids,
            vec![1_000_000, 2_000_000, 3_000_000, 4_000_000, 5_000_000],
            "{url}"
        );
        // Y lo que no se reparte (una clave UUID, una vista sin clave) sigue leyéndose entero.
        let n: usize = leer(&url, "public.lab_uuid", r#"{"n":"n"}"#)
            .unwrap()
            .iter()
            .map(|b| b.num_rows())
            .sum();
        assert_eq!(n, 1000, "{url}");
    }
}

/// El control: **sin** un instante compartido, dos lecturas separadas en el tiempo ven cosas distintas. Si esto no pasara, el test de
/// después no demostraría nada.
fn con_escritor<T>(url: &str, f: impl FnOnce() -> T) -> (T, u64) {
    let parar = AtomicBool::new(false);
    let escrituras = AtomicUsize::new(0);
    let r = std::thread::scope(|s| {
        s.spawn(|| {
            let mut w = escritura(url);
            while !parar.load(Ordering::SeqCst) {
                // Dos filas de extremos opuestos de la clave, en una transacción: una foto coherente las ve siempre iguales.
                if w.batch_execute(&format!(
                    "UPDATE lab_reparto SET v = v + 1 WHERE id IN (1, {N})"
                ))
                .is_ok()
                {
                    escrituras.fetch_add(1, Ordering::SeqCst);
                }
            }
        });
        std::thread::sleep(Duration::from_millis(100));
        let r = f();
        parar.store(true, Ordering::SeqCst);
        r
    });
    (r, escrituras.load(Ordering::SeqCst) as u64)
}

#[test]
fn todas_las_conexiones_leen_de_un_mismo_instante() {
    let _g = UNO_A_UNO.lock().unwrap_or_else(|e| e.into_inner());
    for url in servidores() {
        // Control: dos lecturas sueltas, separadas, de los dos extremos de la clave.
        let ((a, b), escrituras) = con_escritor(&url, || {
            let mut c = ore_cockroach::tls::abrir(&url).unwrap().cliente;
            let a: i64 = c
                .query_one("SELECT v FROM lab_reparto WHERE id = 1", &[])
                .unwrap()
                .get(0);
            std::thread::sleep(Duration::from_millis(300));
            let b: i64 = c
                .query_one(&format!("SELECT v FROM lab_reparto WHERE id = {N}"), &[])
                .unwrap()
                .get(0);
            (a, b)
        });
        assert!(
            escrituras > 5,
            "{url}: el escritor apenas escribió ({escrituras})"
        );
        assert_ne!(
            a, b,
            "{url}: sin un instante compartido, los dos extremos debían verse desgarrados (control)"
        );

        // El conector, varias veces, con el escritor escribiendo: los dos extremos siempre coinciden.
        let (vueltas, escrituras) = con_escritor(&url, || {
            (0..8)
                .map(|_| {
                    let bs = leer(&url, "public.lab_reparto", r#"{"id":"id","v":"v"}"#)
                        .unwrap_or_else(|f| panic!("{url}: {f:?}"));
                    let mut primera = None;
                    let mut ultima = None;
                    for b in &bs {
                        let (id, v) = (
                            b.column(0).as_primitive::<Int64Type>(),
                            b.column(1).as_primitive::<Int64Type>(),
                        );
                        for i in 0..b.num_rows() {
                            match id.value(i) {
                                1 => primera = Some(v.value(i)),
                                x if x == N => ultima = Some(v.value(i)),
                                _ => {}
                            }
                        }
                    }
                    (primera.expect("id 1"), ultima.expect("último id"))
                })
                .collect::<Vec<_>>()
        });
        assert!(
            escrituras > 20,
            "{url}: el escritor apenas escribió ({escrituras})"
        );
        for (i, (p, u)) in vueltas.iter().enumerate() {
            assert_eq!(
                p, u,
                "{url}, vuelta {i}: las conexiones leyeron de instantes distintos ({vueltas:?})"
            );
        }
        assert!(
            vueltas.windows(2).any(|w| w[0] != w[1]),
            "{url}: el escritor no se vio entre vuelta y vuelta ({vueltas:?})"
        );
    }
}

#[test]
fn cancelar_corta_todas_las_conexiones_de_una_lectura_en_tramos() {
    let _g = UNO_A_UNO.lock().unwrap_or_else(|e| e.into_inner());
    for url in servidores() {
        let mut s = Cockroach::conectar(&url).unwrap();
        let token = Token::default();
        let p = peticion(&url, "public.grande", r#"{"id":"id","nota":"nota"}"#, "");
        // `grande` son 10⁶ filas con clave `id`: la deja el banco del kit. Si no está, no hay nada que cancelar aquí.
        if ore_cockroach::leer::reparto(&mut s.cliente, &p)
            .ok()
            .flatten()
            .is_none()
        {
            eprintln!(
                "{url}: sin `public.grande` repartible (el banco del kit no se ha cargado): se salta"
            );
            continue;
        }
        let t = Instant::now();
        let (resultado, _) = std::thread::scope(|sc| {
            let tk = token.clone();
            sc.spawn(move || {
                std::thread::sleep(Duration::from_millis(250));
                Cockroach::cancelar(&tk);
            });
            (Cockroach::leer(&mut s, &token, &p, &mut Vec::new()), ())
        });
        let tardo = t.elapsed();
        match resultado {
            Err(f) => assert_eq!(f.codigo, Codigo::Origen, "{url}: {f:?}"),
            Ok(n) => {
                eprintln!(
                    "{url}: la lectura acabó ({n} filas, {tardo:?}) antes de la cancelación: no prueba nada"
                );
                continue;
            }
        }
        // Ninguna consulta suya sigue viva en el servidor.
        std::thread::sleep(Duration::from_millis(300));
        let mut m = escritura(&url);
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
            "{url}: tras cancelar quedan {vivas} consultas vivas"
        );
        assert!(
            tardo < Duration::from_secs(5),
            "{url}: cancelar tardó {tardo:?}"
        );
    }
}
