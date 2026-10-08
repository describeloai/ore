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
use ore_postgres::computos::{Computos, Estado, Vm};
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
        vec![
            "001-el-esqueleto",
            "002-el-tenant-y-las-ramas",
            "003-los-endpoints"
        ]
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

/// Un almacenamiento (y un cómputo) que apunta lo que se le pide y contesta lo
/// que se le diga. Sus VMs nacen ya `Running` y listas.
#[derive(Default)]
struct Apunta {
    pedido: Mutex<Vec<String>>,
    falla: Mutex<Option<Fallo>>,
    vms: Mutex<HashMap<String, String>>,
}

impl Computos for Apunta {
    fn configuracion(
        &self,
        vm: &str,
        t: &str,
        tl: &str,
        ps: &str,
        _: &str,
        replica: bool,
    ) -> String {
        format!("{vm} {t} {tl} {ps} replica={replica}")
    }
    fn estado(&self, vm: &str) -> Result<Option<Estado>, Fallo> {
        Ok(self.vms.lock().unwrap().get(vm).map(|ip| Estado {
            fase: "Running".into(),
            ip_pod: Some(ip.clone()),
            ip_overlay: Some(format!("10.100.128.{}", ip.len())),
        }))
    }
    fn runner_vivo(&self, _: &str) -> Result<bool, Fallo> {
        Ok(false)
    }
    fn crear(&self, vm: &Vm, cfg: &str) -> Result<(), Fallo> {
        self.apuntar(format!("vm-crear {} {} {cfg}", vm.nombre, vm.endpoint))?;
        self.vms
            .lock()
            .unwrap()
            .insert(vm.nombre.into(), "10.1.0.1".into());
        Ok(())
    }
    fn listo(&self, _: &str, _: &str) -> Result<bool, Fallo> {
        Ok(true)
    }
    fn borrar(&self, vm: &str) -> Result<(), Fallo> {
        self.apuntar(format!("vm-borrar {vm}"))?;
        self.vms.lock().unwrap().remove(vm);
        Ok(())
    }
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
    fn borrar_timeline(&self, t: &str, tl: &str) -> Result<(), Fallo> {
        self.apuntar(format!("borrar-timeline {t} {tl}"))
    }
    fn pageserver_de(&self, t: &str) -> Result<String, Fallo> {
        self.apuntar(format!("pageserver {t}"))?;
        Ok("host=ps port=6400".into())
    }
    fn lsn_en_instante(&self, t: &str, tl: &str, i: &str) -> Result<String, Fallo> {
        self.apuntar(format!("instante {t} {tl} {i}"))?;
        Ok("0/1A2B3C".into())
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
        ore_postgres::reconciliador::vuelta(&mut c2, &almacen, &almacen),
        Ok(1)
    );
    let pedido = almacen.pedido();
    assert_eq!(pedido.len(), 4, "{pedido:?}");
    assert_eq!(pedido[0], format!("tenant {tenant}"));
    assert!(pedido[1].starts_with(&format!("timeline {tenant} ")) && pedido[1].ends_with("None"));
    // P4·3·3: y su endpoint de escritura en main, con la especificación de su tenant.
    assert_eq!(pedido[2], format!("pageserver {tenant}"));
    assert!(
        pedido[3].starts_with("vm-crear ep-")
            && pedido[3].contains(" principal ")
            && pedido[3].contains(&tenant),
        "{pedido:?}"
    );
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
        ore_postgres::reconciliador::vuelta(&mut c2, &almacen, &almacen),
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
        ore_postgres::reconciliador::vuelta(&mut c2, &almacen, &almacen),
        Ok(2)
    );
    let pedido = almacen.pedido();
    assert!(pedido.contains(&format!("borrar {tenant}")), "{pedido:?}");
    // Primero sus cómputos, y después el tenant.
    let vm = pedido
        .iter()
        .position(|x| x.starts_with("vm-borrar"))
        .expect("su VM");
    let t = pedido
        .iter()
        .position(|x| x == &format!("borrar {tenant}"))
        .unwrap();
    assert!(vm < t, "{pedido:?}");
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
        ore_postgres::reconciliador::vuelta(&mut c2, &almacen, &almacen),
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
        ore_postgres::reconciliador::vuelta(&mut c2, &almacen, &almacen),
        Ok(0)
    );
    // Llega su hora y sale.
    *almacen.falla.lock().unwrap() = None;
    c2.execute("update plano.operacion set siguiente = now()", &[])
        .unwrap();
    assert_eq!(
        ore_postgres::reconciliador::vuelta(&mut c2, &almacen, &almacen),
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
    ore_postgres::reconciliador::vuelta(&mut c2, &almacen, &almacen).unwrap();
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
fn las_ramas_salen_de_otra_en_la_punta_en_un_lsn_o_en_un_instante() {
    let _turno = UNA_A_LA_VEZ.lock().unwrap_or_else(|e| e.into_inner());
    let Some(s) = servidor() else { return };
    let mut c2 = otra_conexion();
    let almacen = Apunta::default();
    let ramas = "/v1/postgres/proyectos/ventas/ramas";
    let post = |cuerpo: &str| pide(&s, "a", "POST", ramas, cuerpo);

    let (_, r) = pide(
        &s,
        "a",
        "POST",
        "/v1/postgres/proyectos",
        r#"{"id":"ventas"}"#,
    );
    let tenant = campo(&r, &["proyecto", "tenant"]);
    // Mientras el proyecto se crea, ni una rama: una operación a la vez.
    assert_eq!(post(r#"{"id":"dev"}"#).0, 409);
    ore_postgres::reconciliador::vuelta(&mut c2, &almacen, &almacen).unwrap();
    almacen.pedido();

    // main está, y es la primera.
    let (c, r) = pide(&s, "a", "GET", ramas, "");
    assert_eq!(c, 200, "{r}");
    assert!(
        r.contains(r#""id":"main""#) && r.contains(r#""observado":"lista""#),
        "{r}"
    );
    let (_, r) = pide(&s, "a", "GET", &format!("{ramas}/main"), "");
    let main = campo(&r, &["timeline"]);

    // De la punta de main.
    let (c, r) = post(r#"{"id":"dev"}"#);
    assert_eq!(c, 202, "{r}");
    assert_eq!(campo(&r, &["rama", "origen", "rama"]), "main");
    let dev = campo(&r, &["rama", "timeline"]);
    ore_postgres::reconciliador::vuelta(&mut c2, &almacen, &almacen).unwrap();
    assert_eq!(
        almacen.pedido(),
        vec![format!(
            "timeline {tenant} {dev} Some(Origen {{ timeline: \"{main}\", lsn: None }})"
        )]
    );

    // En un instante: se resuelve a un LSN, se guarda y se sale de ahí.
    let (c, r) = post(r#"{"id":"ayer","instante":"2026-10-07T10:00:00+02:00"}"#);
    assert_eq!(c, 202, "{r}");
    assert_eq!(
        campo(&r, &["rama", "origen", "instante"]),
        "2026-10-07T08:00:00.000Z"
    );
    let ayer = campo(&r, &["rama", "timeline"]);
    ore_postgres::reconciliador::vuelta(&mut c2, &almacen, &almacen).unwrap();
    assert_eq!(
        almacen.pedido(),
        vec![
            format!("instante {tenant} {main} 2026-10-07T08:00:00.000Z"),
            format!(
                "timeline {tenant} {ayer} Some(Origen {{ timeline: \"{main}\", lsn: Some(\"0/1A2B3C\") }})"
            ),
        ]
    );
    let (_, r) = pide(&s, "a", "GET", &format!("{ramas}/ayer"), "");
    assert_eq!(campo(&r, &["origen", "lsn"]), "0/1A2B3C");

    // En un LSN, y de otra rama que no es main.
    assert_eq!(
        post(r#"{"id":"fix","padre":"dev","lsn":"0/16B5A50"}"#).0,
        202
    );
    ore_postgres::reconciliador::vuelta(&mut c2, &almacen, &almacen).unwrap();
    assert!(almacen.pedido()[0].contains(r#"lsn: Some("0/16B5A50")"#));

    // Lo que no vale.
    assert_eq!(post(r#"{"id":"dev"}"#).0, 409);
    assert_eq!(post(r#"{"id":"x","padre":"nada"}"#).0, 404);
    assert_eq!(post(r#"{"id":"x","lsn":"16B5A50"}"#).0, 400);
    assert_eq!(post(r#"{"id":"x","instante":"ayer por la tarde"}"#).0, 400);
    assert_eq!(
        post(r#"{"id":"x","lsn":"0/1","instante":"2026-10-07T10:00:00Z"}"#).0,
        400
    );
    assert_eq!(post(r#"{"id":"-x"}"#).0, 400);
    // Otra organización no ve el proyecto, así que tampoco sus ramas.
    assert_eq!(pide(&s, "b", "GET", ramas, "").0, 404);
    assert_eq!(pide(&s, "b", "POST", ramas, r#"{"id":"x"}"#).0, 404);

    // Borrar: main no; dev tiene una hija (fix), no; fix sí, y después dev.
    assert_eq!(pide(&s, "a", "DELETE", &format!("{ramas}/main"), "").0, 409);
    let (c, r) = pide(&s, "a", "DELETE", &format!("{ramas}/dev"), "");
    assert_eq!(c, 409, "{r}");
    assert!(r.contains("fix"), "{r}");
    let (_, r) = pide(&s, "a", "GET", &format!("{ramas}/fix"), "");
    let fix = campo(&r, &["timeline"]);
    assert_eq!(pide(&s, "a", "DELETE", &format!("{ramas}/fix"), "").0, 202);
    ore_postgres::reconciliador::vuelta(&mut c2, &almacen, &almacen).unwrap();
    assert_eq!(
        almacen.pedido(),
        vec![format!("borrar-timeline {tenant} {fix}")]
    );
    assert_eq!(pide(&s, "a", "GET", &format!("{ramas}/fix"), "").0, 404);
    assert_eq!(pide(&s, "a", "DELETE", &format!("{ramas}/dev"), "").0, 202);
    ore_postgres::reconciliador::vuelta(&mut c2, &almacen, &almacen).unwrap();
    let (_, r) = pide(&s, "a", "GET", ramas, "");
    assert!(
        !r.contains(r#""id":"dev""#) && r.contains(r#""id":"ayer""#),
        "{r}"
    );

    // Un instante sin datos: la rama, fallida, a la primera.
    *almacen.falla.lock().unwrap() = Some(Fallo::Definitivo("no hay datos".into()));
    let (_, r) = post(r#"{"id":"antes","instante":"2000-01-01T00:00:00Z"}"#);
    let op = campo(&r, &["operacion", "id"]);
    ore_postgres::reconciliador::vuelta(&mut c2, &almacen, &almacen).unwrap();
    let (_, r) = pide(
        &s,
        "a",
        "GET",
        &format!("/v1/postgres/operaciones/{op}"),
        "",
    );
    assert_eq!(campo(&r, &["estado"]), "fallida", "{r}");
    let (_, r) = pide(&s, "a", "GET", &format!("{ramas}/antes"), "");
    assert_eq!(campo(&r, &["estado", "observado"]), "fallida", "{r}");
}

#[test]
fn los_endpoints_y_el_cerco_de_escritura() {
    let _turno = UNA_A_LA_VEZ.lock().unwrap_or_else(|e| e.into_inner());
    let Some(s) = servidor() else { return };
    let mut c2 = otra_conexion();
    let almacen = Apunta::default();
    let vuelta = |c2: &mut postgres::Client| {
        ore_postgres::reconciliador::vuelta(c2, &almacen, &almacen).unwrap()
    };
    pide(
        &s,
        "a",
        "POST",
        "/v1/postgres/proyectos",
        r#"{"id":"ventas"}"#,
    );
    vuelta(&mut c2);
    let main = "/v1/postgres/proyectos/ventas/ramas/main/endpoints";
    // main ya tiene su endpoint de escritura, listo, con su dirección.
    let (c, r) = pide(&s, "a", "GET", main, "");
    assert_eq!(c, 200, "{r}");
    assert!(
        r.contains(r#""id":"principal""#) && r.contains(r#""observado":"listo""#),
        "{r}"
    );
    assert!(r.contains(r#""direccion":"10.100.128."#), "{r}");
    // ⭐ El cerco, capa 1: otro de escritura en main, no.
    let (c, r) = pide(&s, "a", "POST", main, r#"{"id":"otro"}"#);
    assert_eq!(c, 409, "{r}");
    // Uno de lectura sí, con sus límites, y su especificación es de réplica.
    let (c, r) = pide(
        &s,
        "a",
        "POST",
        main,
        r#"{"id":"lector","tipo":"lectura","cu_min":"0.5","cu_max":"2"}"#,
    );
    assert_eq!(c, 202, "{r}");
    assert_eq!(campo(&r, &["endpoint", "cu", "max"]), "2");
    almacen.pedido();
    vuelta(&mut c2);
    let pedido = almacen.pedido();
    assert!(
        pedido.iter().any(|x| x.starts_with("vm-crear")
            && x.contains(" lector ")
            && x.ends_with("replica=true")),
        "{pedido:?}"
    );
    // Lo que no vale.
    assert_eq!(
        pide(&s, "a", "POST", main, r#"{"id":"x","tipo":"raro"}"#).0,
        400
    );
    assert_eq!(
        pide(
            &s,
            "a",
            "POST",
            main,
            r#"{"id":"x","tipo":"lectura","cu_max":"8"}"#
        )
        .0,
        400
    );
    assert_eq!(
        pide(
            &s,
            "a",
            "POST",
            main,
            r#"{"id":"x","tipo":"lectura","cu_min":"1","cu_max":"0.5"}"#
        )
        .0,
        400
    );
    assert_eq!(
        pide(
            &s,
            "a",
            "POST",
            "/v1/postgres/proyectos/ventas/ramas/nada/endpoints",
            r#"{"id":"x"}"#
        )
        .0,
        404
    );
    assert_eq!(pide(&s, "b", "GET", main, "").0, 404);
    // Una rama con endpoints no se borra.
    pide(
        &s,
        "a",
        "POST",
        "/v1/postgres/proyectos/ventas/ramas",
        r#"{"id":"dev"}"#,
    );
    vuelta(&mut c2);
    let dev = "/v1/postgres/proyectos/ventas/ramas/dev/endpoints";
    assert_eq!(pide(&s, "a", "POST", dev, r#"{"id":"e1"}"#).0, 202);
    vuelta(&mut c2);
    assert_eq!(
        pide(
            &s,
            "a",
            "DELETE",
            "/v1/postgres/proyectos/ventas/ramas/dev",
            ""
        )
        .0,
        409
    );
    // Borrar el de escritura libera la rama: después, otro de escritura sí.
    let (_, r) = pide(&s, "a", "GET", &format!("{dev}/e1"), "");
    let vm = campo(&r, &["vm"]);
    assert_eq!(pide(&s, "a", "DELETE", &format!("{dev}/e1"), "").0, 202);
    almacen.pedido();
    vuelta(&mut c2);
    assert_eq!(almacen.pedido(), vec![format!("vm-borrar {vm}")]);
    assert_eq!(pide(&s, "a", "GET", &format!("{dev}/e1"), "").0, 404);
    assert_eq!(pide(&s, "a", "POST", dev, r#"{"id":"e2"}"#).0, 202);
    vuelta(&mut c2);
    // Y borrar el proyecto quita sus tres VMs antes que el tenant.
    pide(&s, "a", "DELETE", "/v1/postgres/proyectos/ventas", "");
    almacen.pedido();
    vuelta(&mut c2);
    let pedido = almacen.pedido();
    assert_eq!(
        pedido.iter().filter(|x| x.starts_with("vm-borrar")).count(),
        3,
        "{pedido:?}"
    );
    assert!(pedido.last().unwrap().starts_with("borrar "), "{pedido:?}");
    assert!(almacen.vms.lock().unwrap().is_empty());
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
