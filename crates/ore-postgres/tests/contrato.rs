//! **El contrato contra un Postgres de verdad** (0058 P4·1).
//!
//! Corre si hay `ORE_POSTGRES_PRUEBA_URL` (una base vacía que se puede ensuciar);
//! sin ella se salta y lo dice. Las celdas son fijas: lo que aquí se prueba es
//! que la organización de la celda separa todo, y que la base cierra lo que dice
//! el contrato (un id por organización, una operación en curso por proyecto).

use ore_core::parse;
use ore_entrada::http::Peticion;
use ore_postgres::api::Servidor;
use ore_postgres::celda::{Celda, Fijas};
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

/// Las dos pruebas comparten la base y la limpian al empezar: una detrás de otra.
static UNA_A_LA_VEZ: Mutex<()> = Mutex::new(());

fn servidor() -> Option<Servidor> {
    let Ok(url) = std::env::var("ORE_POSTGRES_PRUEBA_URL") else {
        eprintln!("(sin ORE_POSTGRES_PRUEBA_URL: el contrato contra Postgres no se prueba)");
        return None;
    };
    let mut c = ore_postgres::base::conectar(&url).expect("conectar");
    c.batch_execute("drop schema if exists plano cascade; drop table if exists public.migracion")
        .expect("limpiar");
    assert_eq!(
        ore_postgres::base::migrar(&mut c).expect("migrar"),
        vec!["001-el-esqueleto"]
    );
    // Dos veces es una: no aplica nada.
    assert!(
        ore_postgres::base::migrar(&mut c)
            .expect("migrar")
            .is_empty()
    );
    let celda = |id: &str, org: &str| Celda {
        id: id.into(),
        nombre: id.into(),
        organizacion: org.into(),
    };
    Some(Servidor {
        base: Mutex::new(c),
        celdas: Box::new(Fijas(HashMap::from([
            ("a".to_string(), celda("cel_a", "org_1")),
            ("a2".to_string(), celda("cel_a2", "org_1")),
            ("b".to_string(), celda("cel_b", "org_2")),
        ]))),
    })
}

fn pide(s: &Servidor, token: &str, metodo: &str, ruta: &str, cuerpo: &str) -> (u16, String) {
    let mut cabeceras = BTreeMap::new();
    if !token.is_empty() {
        cabeceras.insert("authorization".into(), format!("Bearer {token}"));
    }
    let r = s.atender(&Peticion {
        metodo: metodo.into(),
        ruta: ruta.into(),
        cabeceras,
        cuerpo: cuerpo.into(),
        consulta: BTreeMap::new(),
    });
    (r.codigo, r.cuerpo.jcs())
}

fn campo(cuerpo: &str, camino: &[&str]) -> String {
    let n = parse::parse(cuerpo).expect("json");
    let mut nodo = &n;
    for k in camino {
        nodo = nodo
            .get(k)
            .unwrap_or_else(|| panic!("falta {k} en {cuerpo}"))
            .1;
    }
    nodo.as_str().unwrap_or_default().to_string()
}

#[test]
fn una_celda_crea_y_lee_y_otra_organizacion_no_lo_ve() {
    let _turno = UNA_A_LA_VEZ.lock().unwrap_or_else(|e| e.into_inner());
    let Some(s) = servidor() else { return };
    let (c, r) = pide(
        &s,
        "a",
        "POST",
        "/v1/postgres/proyectos",
        r#"{"id":"ventas","dueno":"user:ana"}"#,
    );
    assert_eq!(c, 202, "{r}");
    assert_eq!(campo(&r, &["operacion", "hecha"]), "true");
    assert_eq!(campo(&r, &["proyecto", "dueno"]), "user:ana");
    assert_eq!(campo(&r, &["proyecto", "celda"]), "cel_a");
    let op = campo(&r, &["operacion", "id"]);
    assert!(op.starts_with("op_"), "{op}");

    // Leerlo, por sí y en la lista.
    let (c, r) = pide(&s, "a", "GET", "/v1/postgres/proyectos/ventas", "");
    assert_eq!(c, 200, "{r}");
    assert_eq!(campo(&r, &["estado", "observado"]), "listo");
    let (_, r) = pide(&s, "a", "GET", "/v1/postgres/proyectos", "");
    assert!(r.contains(r#""id":"ventas""#), "{r}");

    // Otra celda de LA MISMA organización lo ve: el proyecto es de la organización.
    assert_eq!(
        pide(&s, "a2", "GET", "/v1/postgres/proyectos/ventas", "").0,
        200
    );

    // Otra organización, no: ni leerlo, ni en su lista, ni borrarlo, ni su operación.
    assert_eq!(
        pide(&s, "b", "GET", "/v1/postgres/proyectos/ventas", "").0,
        404
    );
    let (c, r) = pide(&s, "b", "GET", "/v1/postgres/proyectos", "");
    assert_eq!((c, r.as_str()), (200, r#"{"proyectos":[]}"#));
    assert_eq!(
        pide(&s, "b", "DELETE", "/v1/postgres/proyectos/ventas", "").0,
        404
    );
    assert_eq!(
        pide(
            &s,
            "b",
            "GET",
            &format!("/v1/postgres/operaciones/{op}"),
            ""
        )
        .0,
        404
    );

    // Y puede tener SU `ventas`: el id es por organización.
    assert_eq!(
        pide(
            &s,
            "b",
            "POST",
            "/v1/postgres/proyectos",
            r#"{"id":"ventas"}"#
        )
        .0,
        202
    );

    // Repetir no crea dos; lo que no vale, 400; sin celda, 401.
    assert_eq!(
        pide(
            &s,
            "a",
            "POST",
            "/v1/postgres/proyectos",
            r#"{"id":"ventas"}"#
        )
        .0,
        409
    );
    assert_eq!(
        pide(&s, "a", "POST", "/v1/postgres/proyectos", r#"{"id":"-x"}"#).0,
        400
    );
    assert_eq!(
        pide(
            &s,
            "a",
            "POST",
            "/v1/postgres/proyectos",
            r#"{"id":"x","dueno":"team:t"}"#
        )
        .0,
        400
    );
    assert_eq!(pide(&s, "a", "POST", "/v1/postgres/proyectos", "").0, 400);
    assert_eq!(pide(&s, "", "GET", "/v1/postgres/proyectos", "").0, 401);
    assert_eq!(pide(&s, "z", "GET", "/v1/postgres/proyectos", "").0, 401);

    // Borrar: el de `a`, y el de `b` sigue.
    let (c, r) = pide(&s, "a", "DELETE", "/v1/postgres/proyectos/ventas", "");
    assert_eq!(c, 202, "{r}");
    assert_eq!(campo(&r, &["operacion", "tipo"]), "borrar-proyecto");
    assert_eq!(
        pide(&s, "a", "GET", "/v1/postgres/proyectos/ventas", "").0,
        404
    );
    assert_eq!(
        pide(&s, "b", "GET", "/v1/postgres/proyectos/ventas", "").0,
        200
    );
    // La operación de crear queda: es la historia.
    assert_eq!(
        pide(
            &s,
            "a",
            "GET",
            &format!("/v1/postgres/operaciones/{op}"),
            ""
        )
        .0,
        200
    );
}

#[test]
fn una_operacion_en_curso_por_proyecto_la_cierra_la_base() {
    let _turno = UNA_A_LA_VEZ.lock().unwrap_or_else(|e| e.into_inner());
    let Some(_) = servidor() else { return };
    let url = std::env::var("ORE_POSTGRES_PRUEBA_URL").unwrap();
    let mut c = ore_postgres::base::conectar(&url).unwrap();
    let mete = |c: &mut postgres::Client, id: &str| {
        c.execute(
            "insert into plano.operacion (id, organizacion, proyecto, tipo, celda) values ($1, 'o', 'p', 't', 'c')",
            &[&id],
        )
    };
    mete(&mut c, "op_1").unwrap();
    let e = mete(&mut c, "op_2").unwrap_err();
    assert!(
        ore_postgres::base::choca(&e, "una_en_curso_por_proyecto"),
        "{e:?}"
    );
}
