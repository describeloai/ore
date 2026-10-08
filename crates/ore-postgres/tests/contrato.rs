//! **El contrato contra un Postgres de verdad** (0058 P4·1).
//!
//! Corre si hay `ORE_POSTGRES_PRUEBA_URL` (una base vacía que se puede ensuciar);
//! sin ella se salta y lo dice. Las celdas son fijas: lo que aquí se prueba es
//! que la organización de la celda separa todo, y que la base cierra lo que dice
//! el contrato (un id por organización, una operación en curso por proyecto).
//!
//! P4·2: el almacenamiento es un [`Apunta`], que hace lo que se le diga y apunta
//! lo que se le pidió; contra el de verdad se prueba en vivo (`p42.sh`).

use ore_core::parse;
use ore_entrada::http::Peticion;
use ore_postgres::almacen::{Almacen, Fallo, Origen};
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
        vec!["001-el-esqueleto", "002-el-tenant-y-las-ramas"]
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
        url: Some(url),
    })
}

/// Un almacenamiento que apunta lo que se le pide y contesta lo que se le diga.
#[derive(Default)]
struct Apunta {
    pedido: Mutex<Vec<String>>,
    falla: Mutex<Option<Fallo>>,
}

impl Apunta {
    fn apuntar(&self, que: String) -> Result<(), Fallo> {
        self.pedido.lock().unwrap().push(que);
        match self.falla.lock().unwrap().clone() {
            Some(f) => Err(f),
            None => Ok(()),
        }
    }
    fn pedido(&self) -> Vec<String> {
        std::mem::take(&mut *self.pedido.lock().unwrap())
    }
}

impl Almacen for Apunta {
    fn asegurar_tenant(&self, t: &str) -> Result<(), Fallo> {
        self.apuntar(format!("tenant {t}"))
    }
    fn asegurar_timeline(&self, t: &str, tl: &str, o: Option<Origen>) -> Result<(), Fallo> {
        self.apuntar(format!("timeline {t} {tl} {o:?}"))
    }
    fn borrar_tenant(&self, t: &str) -> Result<(), Fallo> {
        self.apuntar(format!("borrar {t}"))
    }
}

fn otra_conexion() -> postgres::Client {
    ore_postgres::base::conectar(&std::env::var("ORE_POSTGRES_PRUEBA_URL").unwrap()).unwrap()
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
    // P4·2: la operación nace en curso y el proyecto, nuevo, con su tenant ya nombrado.
    assert_eq!(campo(&r, &["operacion", "hecha"]), "false");
    assert_eq!(campo(&r, &["proyecto", "estado", "observado"]), "nuevo");
    assert_eq!(campo(&r, &["proyecto", "dueno"]), "user:ana");
    assert_eq!(campo(&r, &["proyecto", "celda"]), "cel_a");
    let tenant = campo(&r, &["proyecto", "tenant"]);
    assert_eq!(tenant.len(), 32, "{tenant}");
    let op = campo(&r, &["operacion", "id"]);
    assert!(op.starts_with("op_"), "{op}");

    // Borrar mientras se crea: 409, una operación a la vez.
    assert_eq!(
        pide(&s, "a", "DELETE", "/v1/postgres/proyectos/ventas", "").0,
        409
    );

    // El reconciliador: tenant y main, en ese orden, y la operación, hecha.
    let almacen = Apunta::default();
    let mut c2 = otra_conexion();
    assert_eq!(
        ore_postgres::reconciliador::vuelta(&mut c2, &almacen),
        Ok(1)
    );
    let pedido = almacen.pedido();
    assert_eq!(pedido.len(), 2, "{pedido:?}");
    assert_eq!(pedido[0], format!("tenant {tenant}"));
    assert!(pedido[1].starts_with(&format!("timeline {tenant} ")) && pedido[1].ends_with("None"));
    let (_, r) = pide(
        &s,
        "a",
        "GET",
        &format!("/v1/postgres/operaciones/{op}"),
        "",
    );
    assert_eq!(campo(&r, &["hecha"]), "true", "{r}");
    // Y no repite lo hecho.
    assert_eq!(
        ore_postgres::reconciliador::vuelta(&mut c2, &almacen),
        Ok(0)
    );

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

    // Y puede tener SU `ventas`: el id es por organización (y su tenant, otro).
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
    assert_eq!(campo(&r, &["operacion", "hecha"]), "false");
    // Mientras se borra, se ve borrando y no sale en la lista.
    let (_, r) = pide(&s, "a", "GET", "/v1/postgres/proyectos/ventas", "");
    assert_eq!(campo(&r, &["estado", "observado"]), "borrando");
    assert_eq!(
        pide(&s, "a", "GET", "/v1/postgres/proyectos", "").1,
        r#"{"proyectos":[]}"#
    );
    // (y la vuelta crea antes el de `b`, que estaba en curso)
    assert_eq!(
        ore_postgres::reconciliador::vuelta(&mut c2, &almacen),
        Ok(2)
    );
    assert!(almacen.pedido().contains(&format!("borrar {tenant}")));
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
fn lo_pasajero_se_reintenta_y_lo_definitivo_falla() {
    let _turno = UNA_A_LA_VEZ.lock().unwrap_or_else(|e| e.into_inner());
    let Some(s) = servidor() else { return };
    let mut c2 = otra_conexion();
    let almacen = Apunta::default();

    // Pasajero: sigue en curso, con el motivo y un intento más, y no vuelve hasta su hora.
    *almacen.falla.lock().unwrap() = Some(Fallo::Reintentar("tenant aún no activo".into()));
    let (_, r) = pide(&s, "a", "POST", "/v1/postgres/proyectos", r#"{"id":"uno"}"#);
    let op = campo(&r, &["operacion", "id"]);
    assert_eq!(
        ore_postgres::reconciliador::vuelta(&mut c2, &almacen),
        Ok(1)
    );
    let (_, r) = pide(
        &s,
        "a",
        "GET",
        &format!("/v1/postgres/operaciones/{op}"),
        "",
    );
    assert_eq!(campo(&r, &["estado"]), "en-curso", "{r}");
    assert_eq!(campo(&r, &["error"]), "tenant aún no activo");
    assert_eq!(
        ore_postgres::reconciliador::vuelta(&mut c2, &almacen),
        Ok(0)
    );
    // Llega su hora y sale.
    *almacen.falla.lock().unwrap() = None;
    c2.execute("update plano.operacion set siguiente = now()", &[])
        .unwrap();
    assert_eq!(
        ore_postgres::reconciliador::vuelta(&mut c2, &almacen),
        Ok(1)
    );
    let (_, r) = pide(
        &s,
        "a",
        "GET",
        &format!("/v1/postgres/operaciones/{op}"),
        "",
    );
    assert_eq!(campo(&r, &["estado"]), "hecha", "{r}");

    // Definitivo: fallida a la primera, y el proyecto, fallido.
    *almacen.falla.lock().unwrap() = Some(Fallo::Definitivo("400 mal".into()));
    let (_, r) = pide(&s, "a", "POST", "/v1/postgres/proyectos", r#"{"id":"dos"}"#);
    let op = campo(&r, &["operacion", "id"]);
    ore_postgres::reconciliador::vuelta(&mut c2, &almacen).unwrap();
    let (_, r) = pide(
        &s,
        "a",
        "GET",
        &format!("/v1/postgres/operaciones/{op}"),
        "",
    );
    assert_eq!(campo(&r, &["estado"]), "fallida", "{r}");
    assert!(campo(&r, &["error"]).starts_with("400 mal"), "{r}");
    let (_, r) = pide(&s, "a", "GET", "/v1/postgres/proyectos/dos", "");
    assert_eq!(campo(&r, &["estado", "observado"]), "fallido", "{r}");
}

#[test]
fn si_la_base_corta_la_conexion_el_api_vuelve_solo() {
    let _turno = UNA_A_LA_VEZ.lock().unwrap_or_else(|e| e.into_inner());
    let Some(s) = servidor() else { return };
    assert_eq!(pide(&s, "a", "GET", "/v1/postgres/proyectos", "").0, 200);
    // Lo que hace un reinicio de la base: matar las conexiones de los demás.
    let mut c2 = otra_conexion();
    let muertas: i64 = c2
        .query_one(
            "select count(*) from (select pg_terminate_backend(pid) from pg_stat_activity
              where pid <> pg_backend_pid() and datname = current_database()) x",
            &[],
        )
        .unwrap()
        .get(0);
    assert!(muertas >= 1);
    std::thread::sleep(std::time::Duration::from_millis(200));
    let (c, r) = pide(&s, "a", "GET", "/v1/postgres/proyectos", "");
    assert_eq!(c, 200, "{r}");
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
