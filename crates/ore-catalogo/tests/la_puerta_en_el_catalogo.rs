//! **La puerta sobre el catálogo** (0079 §19.2, E1a·5c·1): la v2 contra un Postgres de verdad.
//!
//! Necesita `ORE_CATALOGO_PRUEBAS` (la URL de un Postgres donde se pueda crear un esquema); sin ella,
//! cada prueba lo dice y no mide nada. Cada una trabaja en su esquema y lo borra al acabar.
//!
//! ```text
//! docker network create ore-pruebas
//! docker run -d --name pg --network ore-pruebas -e POSTGRES_PASSWORD=p postgres:16
//! ORE_CATALOGO_PRUEBAS=postgres://postgres:p@pg:5432/postgres cargo test -p ore-catalogo
//! ```

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use ore_catalogo::almacen::{
    Almacen, CambioRuta, EsperadoMl, FalloEscritura, QueRuta, VersionRuta,
};
use ore_core::json::Json;
use postgres::{Client, NoTls};

/// Un esquema propio, que se borra al soltarse.
struct Prueba {
    url: String,
    esquema: String,
}

impl Drop for Prueba {
    fn drop(&mut self) {
        if let Ok(mut c) = Client::connect(&self.url, NoTls) {
            let _ = c.batch_execute(&format!("DROP SCHEMA IF EXISTS {} CASCADE", self.esquema));
        }
    }
}

fn prueba() -> Option<(Prueba, Almacen)> {
    static N: AtomicUsize = AtomicUsize::new(0);
    let Ok(url) = std::env::var("ORE_CATALOGO_PRUEBAS") else {
        eprintln!("⚠ sin ORE_CATALOGO_PRUEBAS: la puerta del catálogo no se prueba aquí");
        return None;
    };
    let esquema = format!(
        "t_p{}_{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    );
    let mut c = Client::connect(&url, NoTls).expect("conectar");
    c.batch_execute(&format!("CREATE SCHEMA {esquema}"))
        .expect("crear el esquema");
    let a = Almacen::conectar(&url, &esquema).expect("almacén");
    Some((Prueba { url, esquema }, a))
}

/// Una conexión cruda al esquema, para preparar lo que la puerta no hace (lo del andamio).
fn crudo(p: &Prueba) -> Client {
    let mut c = Client::connect(&p.url, NoTls).unwrap();
    c.batch_execute(&format!("SET search_path TO {}", p.esquema))
        .unwrap();
    c
}

const AQUI: &str = "datasets/ventas/default/pedidos.json";
const ANTES: &str = "datasets/ventas_pedidos.json";

fn ml(n: u32) -> String {
    format!("gs://b/ore/v2/catalogo/ventas/default/pedidos/metadata/{n:05}-x.metadata.json")
}

fn rutas() -> Vec<String> {
    vec![AQUI.into(), ANTES.into()]
}

fn poner(campos: &[(&str, Json)], esperado: EsperadoMl) -> CambioRuta {
    CambioRuta {
        rutas: rutas(),
        esperado,
        que: QueRuta::Poner {
            campos: campos
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect::<BTreeMap<_, _>>(),
            quitar: Vec::new(),
        },
    }
}

fn campo(texto: &str, k: &str) -> Option<String> {
    let n = ore_core::parse::parse(texto).unwrap();
    n.get(k)
        .and_then(|(_, v)| v.as_str())
        .filter(|v| *v != "null")
        .map(String::from)
}

fn ml_de(a: &mut Almacen, r: &str) -> Option<String> {
    a.efectivo(r, &rutas())
        .unwrap()
        .and_then(|(_, t)| campo(&t, "metadata_location"))
}

#[test]
fn nacer_poner_encima_y_el_dataset_de_su_metadata() {
    let Some((_p, mut a)) = prueba() else { return };
    a.escribir(
        "main",
        "persona:ana",
        "commit",
        None,
        &[poner(
            &[
                ("metadata_location", Json::s(ml(1))),
                ("estado", Json::s("copiada")),
                ("motivo", Json::s("x")),
            ],
            EsperadoMl::Nada,
        )],
    )
    .unwrap();
    let (de, t) = a.efectivo("main", &rutas()).unwrap().unwrap();
    assert_eq!(de, "aqui");
    assert_eq!(
        campo(&t, "dataset").as_deref(),
        Some("catalogo/ventas/default/pedidos")
    );
    a.escribir(
        "main",
        "persona:ana",
        "commit",
        None,
        &[CambioRuta {
            rutas: rutas(),
            esperado: EsperadoMl::Ml(ml(1)),
            que: QueRuta::Poner {
                campos: [("metadata_location".to_string(), Json::s(ml(2)))].into(),
                quitar: vec!["motivo".into()],
            },
        }],
    )
    .unwrap();
    let (_, t) = a.efectivo("main", &rutas()).unwrap().unwrap();
    assert_eq!(campo(&t, "estado").as_deref(), Some("copiada"));
    assert_eq!(campo(&t, "motivo"), None);
    assert_eq!(campo(&t, "metadata_location"), Some(ml(2)));
}

#[test]
fn el_compare_and_set_es_de_todos_o_ninguno() {
    let Some((p, mut a)) = prueba() else { return };
    a.escribir(
        "main",
        "s",
        "commit",
        None,
        &[poner(
            &[("metadata_location", Json::s(ml(1)))],
            EsperadoMl::Nada,
        )],
    )
    .unwrap();
    let otra = CambioRuta {
        rutas: vec!["datasets/ventas/default/otra.json".into()],
        ..poner(&[("metadata_location", Json::s(ml(9)))], EsperadoMl::Nada)
    };
    let mal = poner(
        &[("metadata_location", Json::s(ml(3)))],
        EsperadoMl::Ml(ml(2)),
    );
    let antes = crudo(&p)
        .query_one("SELECT count(*) FROM commits", &[])
        .unwrap()
        .get::<_, i64>(0);
    let e = a
        .escribir("main", "s", "commit", None, &[otra, mal])
        .unwrap_err();
    assert_eq!(
        e,
        FalloEscritura::Conflicto {
            indice: 1,
            hay: Some(ml(1))
        }
    );
    // Nada se escribió: ni la otra, ni un commit.
    assert!(
        a.efectivo("main", &["datasets/ventas/default/otra.json".to_string()])
            .unwrap()
            .is_none()
    );
    let despues = crudo(&p)
        .query_one("SELECT count(*) FROM commits", &[])
        .unwrap()
        .get::<_, i64>(0);
    assert_eq!(antes, despues);
    // Nacer sobre lo que ya está, también.
    let e = a
        .escribir("main", "s", "commit", None, &[poner(&[], EsperadoMl::Nada)])
        .unwrap_err();
    assert!(matches!(e, FalloEscritura::Conflicto { indice: 0, .. }));
}

#[test]
fn el_de_antes_se_lee_y_al_escribir_se_retira() {
    let Some((p, mut a)) = prueba() else { return };
    // Lo que el andamio trajo de git: el de antes.
    let texto = format!("{{\"metadata_location\": \"{}\"}}", ml(1));
    crudo(&p)
        .execute(
            "SELECT replicar_main('abc', 'Ana <ana@x>', now(), $1, $2, $3)",
            &[&vec![ANTES], &vec![Some("b1")], &vec![Some(texto.as_str())]],
        )
        .unwrap();
    assert_eq!(ml_de(&mut a, "main"), Some(ml(1)));
    a.escribir(
        "main",
        "s",
        "commit",
        None,
        &[poner(
            &[("metadata_location", Json::s(ml(2)))],
            EsperadoMl::Ml(ml(1)),
        )],
    )
    .unwrap();
    let vista = a.vista_entera("main").unwrap();
    assert_eq!(vista.len(), 1, "el de antes ya no está: {vista:?}");
    assert_eq!(vista[0].0, AQUI);
    // La historia sigue a los dos: la de su sitio y la de antes, y el autor sin correo.
    let h = a.historia("main", &rutas(), 10).unwrap();
    assert_eq!(h.len(), 2, "{h:?}");
    assert_eq!(campo(&h[1].texto, "metadata_location"), Some(ml(1)));
    assert_eq!(h[1].id, "abc");
    assert_eq!(h[1].quien.as_deref(), Some("Ana"));
    assert!(h[0].id.starts_with('c'));
    // Retirar: ni el de su sitio ni el de antes.
    a.escribir(
        "main",
        "s",
        "retirar",
        None,
        &[CambioRuta {
            rutas: rutas(),
            esperado: EsperadoMl::Cualquiera,
            que: QueRuta::Retirar,
        }],
    )
    .unwrap();
    assert!(a.efectivo("main", &rutas()).unwrap().is_none());
    assert!(a.vista_entera("main").unwrap().is_empty());
}

#[test]
fn una_rama_hereda_de_main_al_dia_y_lo_que_escribe_es_suyo() {
    let Some((_p, mut a)) = prueba() else { return };
    let w = |a: &mut Almacen, r: &str, n: u32, e: EsperadoMl| {
        a.escribir(
            r,
            "s",
            "commit",
            None,
            &[poner(&[("metadata_location", Json::s(ml(n)))], e)],
        )
    };
    w(&mut a, "main", 1, EsperadoMl::Nada).unwrap();
    // Escribir en una rama que no está: 404.
    assert_eq!(
        w(&mut a, "r", 5, EsperadoMl::Cualquiera).unwrap_err(),
        FalloEscritura::SinRama("r".into())
    );
    a.crear_rama("r", "main").unwrap();
    let v0 = a.version("r").unwrap();
    assert_eq!(a.efectivo("r", &rutas()).unwrap().unwrap().0, "main");
    // main se mueve: la rama lo ve al día.
    w(&mut a, "main", 2, EsperadoMl::Ml(ml(1))).unwrap();
    assert_eq!(ml_de(&mut a, "r"), Some(ml(2)));
    assert_ne!(
        a.version("r").unwrap(),
        v0,
        "lo de main cambia lo que ve la rama"
    );
    // El CAS en la rama, contra lo heredado: lo de main de hoy.
    assert!(w(&mut a, "r", 3, EsperadoMl::Ml(ml(1))).is_err());
    w(&mut a, "r", 3, EsperadoMl::Ml(ml(2))).unwrap();
    let (de, _) = a.efectivo("r", &rutas()).unwrap().unwrap();
    assert_eq!(de, "aqui", "escribir sobre lo heredado lo hace propio");
    assert_eq!(ml_de(&mut a, "main"), Some(ml(2)), "main no cambia");
    // Lo propio manda aunque main se mueva.
    w(&mut a, "main", 4, EsperadoMl::Ml(ml(2))).unwrap();
    assert_eq!(ml_de(&mut a, "r"), Some(ml(3)));
    // La historia de lo propio: lo suyo y lo de main hasta su salida (v1), no lo posterior.
    let h: Vec<String> = a
        .historia("r", &rutas(), 10)
        .unwrap()
        .iter()
        .filter_map(|v: &VersionRuta| campo(&v.texto, "metadata_location"))
        .collect();
    assert_eq!(h, vec![ml(3), ml(1)]);
    // Otra rama que no lo tocó: lo hereda y su historia es la de main entera.
    a.crear_rama("s", "main").unwrap();
    let h: Vec<String> = a
        .historia("s", &rutas(), 10)
        .unwrap()
        .iter()
        .filter_map(|v| campo(&v.texto, "metadata_location"))
        .collect();
    assert_eq!(h, vec![ml(4), ml(2), ml(1)]);
    // Retirar en la rama deja una lápida: la rama no lo ve, main sí.
    a.escribir(
        "r",
        "s",
        "retirar",
        None,
        &[CambioRuta {
            rutas: rutas(),
            esperado: EsperadoMl::Cualquiera,
            que: QueRuta::Retirar,
        }],
    )
    .unwrap();
    assert!(a.efectivo("r", &rutas()).unwrap().is_none());
    assert!(a.vista_entera("r").unwrap().is_empty());
    assert_eq!(ml_de(&mut a, "main"), Some(ml(4)));
    // Lo vivo: lo de main y nada de la rama (la lápida no reclama).
    let vivos = a.vivos().unwrap();
    assert_eq!(vivos.len(), 1, "{vivos:?}");
    // Traer main mueve el punto de salida; borrar la rama suelta lo suyo.
    a.traer_main("s").unwrap();
    a.borrar_rama("r").unwrap();
    assert_eq!(ml_de(&mut a, "r"), None, "una rama borrada no está");
}

#[test]
fn la_misma_clave_dos_veces_es_una_escritura() {
    let Some((p, mut a)) = prueba() else { return };
    let c = poner(&[("metadata_location", Json::s(ml(1)))], EsperadoMl::Nada);
    let x = a
        .escribir("main", "s", "commit", Some("k-1"), std::slice::from_ref(&c))
        .unwrap();
    // Repetida, aunque su CAS ya no se cumpla: la de antes, sin tocar nada.
    let y = a
        .escribir("main", "s", "commit", Some("k-1"), std::slice::from_ref(&c))
        .unwrap();
    assert_eq!(x, y);
    let n: i64 = crudo(&p)
        .query_one("SELECT count(*) FROM commits", &[])
        .unwrap()
        .get(0);
    assert_eq!(n, 1);
}

#[test]
fn dos_que_compiten_por_el_mismo_puntero_gana_uno() {
    let Some((p, mut a)) = prueba() else { return };
    a.escribir(
        "main",
        "s",
        "commit",
        None,
        &[poner(
            &[("metadata_location", Json::s(ml(1)))],
            EsperadoMl::Nada,
        )],
    )
    .unwrap();
    let hilos: Vec<_> = (0..8)
        .map(|i| {
            let (url, esquema) = (p.url.clone(), p.esquema.clone());
            std::thread::spawn(move || {
                let mut a = Almacen::conectar(&url, &esquema).unwrap();
                a.escribir(
                    "main",
                    "s",
                    "commit",
                    None,
                    &[poner(
                        &[("metadata_location", Json::s(ml(10 + i)))],
                        EsperadoMl::Ml(ml(1)),
                    )],
                )
                .is_ok()
            })
        })
        .collect();
    let ganan = hilos
        .into_iter()
        .map(|h| h.join().unwrap())
        .filter(|g| *g)
        .count();
    assert_eq!(ganan, 1, "con el mismo CAS, gana exactamente uno");
}

#[test]
fn un_esquema_v1_se_lleva_a_la_v2_y_conserva_lo_que_tenia() {
    let Ok(url) = std::env::var("ORE_CATALOGO_PRUEBAS") else {
        return;
    };
    let esquema = format!("t_v1_{}", std::process::id());
    let p = Prueba {
        url: url.clone(),
        esquema: esquema.clone(),
    };
    let mut c = Client::connect(&url, NoTls).unwrap();
    c.batch_execute(&format!(
        "CREATE SCHEMA {esquema}; SET search_path TO {esquema};"
    ))
    .unwrap();
    c.batch_execute(include_str!("../src/esquema_v1.sql"))
        .unwrap();
    c.batch_execute("INSERT INTO esquema_version (version) VALUES (1)")
        .unwrap();
    let texto = format!("{{\"metadata_location\": \"{}\"}}", ml(1));
    c.execute(
        "SELECT replicar_main('abc', 'a', now(), $1, $2, $3)",
        &[&vec![AQUI], &vec![Some("b1")], &vec![Some(texto.as_str())]],
    )
    .unwrap();
    let mut a = Almacen::conectar(&url, &esquema).unwrap();
    let v: i32 = crudo(&p)
        .query_one("SELECT max(version) FROM esquema_version", &[])
        .unwrap()
        .get(0);
    assert_eq!(
        v,
        ore_catalogo::esquema::VERSION,
        "hasta la de hoy, de una vez"
    );
    assert_eq!(ml_de(&mut a, "main"), Some(ml(1)));
    a.escribir(
        "main",
        "s",
        "commit",
        None,
        &[poner(
            &[("metadata_location", Json::s(ml(2)))],
            EsperadoMl::Ml(ml(1)),
        )],
    )
    .unwrap();
    // Conectar otra vez no vuelve a aplicar nada.
    Almacen::conectar(&url, &esquema).unwrap();
}
