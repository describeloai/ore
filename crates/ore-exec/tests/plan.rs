//! M1: **el plan es un artefacto, y se rechaza sin abrir una conexión.**
//!
//! Todo lo de aquí corre sin una sola variable de entorno configurada, y eso no
//! es una casualidad del banco de pruebas: es la propiedad. Planificar sale del
//! bundle más la petición, así que se prueba con la misma maquinaria que L0.

use ore_exec::{Consulta, Identidad, Motor, Rechazo};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn ejemplo() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("vendor/oos/examples/acme-retail")
        .leak()
}

fn caso(nombre: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("casos")
        .join(nombre)
}

fn analista() -> Identidad {
    Identidad {
        emisor: "https://id.acme.example".into(),
        audiencia: "ore".into(),
        sujeto: "emp-42".into(),
        roles: vec!["hr_analyst".into()],
        claims: BTreeMap::from([
            ("employeeId".to_string(), "emp-42".to_string()),
            ("departmentId".to_string(), "finanzas".to_string()),
        ]),
    }
}

fn consulta(props: &[&str], claves: &[&str]) -> Consulta {
    Consulta {
        quien: analista(),
        accion: "read".into(),
        purpose: "compensation_review".into(),
        entidad: "hr.Employee".into(),
        propiedades: props.iter().map(|p| p.to_string()).collect(),
        claves: claves.iter().map(|k| vec![k.to_string()]).collect(),
        travesia: None,
        instante: None,
        sla: None,
    }
}

#[test]
fn las_cuatro_fases_y_el_ambito_convertido_en_filtro() {
    let m = Motor::cargar(ejemplo()).expect("el ejemplo carga");
    let plan = m
        .planificar(&consulta(
            &["hr.Employee.baseSalary", "hr.Employee.grade"],
            &["emp-7"],
        ))
        .expect("con clave y con rol tiene que haber plan");

    // ① sobrevive lo autorizado, y lo podado dice por qué.
    assert!(plan.autorizadas.contains_key("hr.Employee.baseSalary"));
    assert!(
        plan.podadas.contains_key("hr.Employee.grade"),
        "`grade` es `high` y ninguna política la alcanza: se poda y se dice"
    );

    // ③ una lectura, con la columna FÍSICA. Y `grade` no está: lo que ① podó no
    // llega a pedirse, que es la mitad del argumento de §3.
    assert_eq!(plan.lecturas.len(), 1, "{:?}", plan.lecturas);
    let l = &plan.lecturas[0];
    assert_eq!(l.datasource, "hr_workday");
    assert_eq!(
        l.proyeccion.get("baseSalary").map(String::as_str),
        Some("Compensation_Data.Base_Pay.Amount")
    );
    assert!(!l.proyeccion.contains_key("grade"), "{:?}", l.proyeccion);

    // Y el ámbito se ha convertido en un PREDICADO que viaja al origen, con el
    // valor que trajo el principal. Es la decisión del ámbito, de punta a punta.
    assert_eq!(l.filtros.len(), 1, "{:?}", l.filtros);
    assert_eq!(
        l.filtros[0].columna,
        "Organization_Data.Cost_Center_Reference"
    );
    assert_eq!(l.filtros[0].valor, "finanzas");
    assert_eq!(l.filtros[0].ambito, "eu.gdpr-minimization#own-department");
}

/// `fullScan` es una autorización, no una descripción — y la negativa se
/// respeta. Agregar compensación de toda la plantilla es exactamente el plan que
/// necesitaría recorrido completo, y Workday lo prohíbe por escrito.
///
/// **Quién la declara cambió con v1alpha8, y la negativa no.** Antes era el
/// binding `hr.workday`; ahora `acme-retail` no tiene bindings y la fuente
/// física de `hr.Employee` es la vista que la respalda, `hr.empleados`, cuyas
/// capacidades salen de `reads` de la tabla. Es exactamente lo que
/// `Rechazo::PlanRechazado` dice de su campo: *nombra a quien las declaró — un
/// binding, o la vista que toca la fuente*.
#[test]
fn un_recorrido_completo_que_la_fuente_prohibe_rechaza_el_plan() {
    let m = Motor::cargar(ejemplo()).expect("el ejemplo carga");
    let mut c = consulta(&["hr.Employee.baseSalary"], &[]);
    c.accion = "aggregate".into();
    c.purpose = "workforce_analytics".into();

    let r = m
        .planificar(&c)
        .expect_err("sin claves y sin filtro, es un escaneo");
    let Rechazo::PlanRechazado { binding, campo, .. } = &r else {
        panic!("tenía que rechazar el plan, y salió {r:?}");
    };
    assert_eq!(campo, "fullScan");
    assert_eq!(
        binding, "hr.empleados",
        "sin bindings, quien declara las capacidades es la vista que respalda"
    );
}

/// §5.1 · sin `capabilities`, un binding sirve **la búsqueda por clave y nada
/// más**. No es una limitación arbitraria: es lo único que se deduce de que el
/// binding declare un objeto y cubra la clave.
#[test]
fn sin_capacidades_solo_hay_busqueda_por_clave() {
    let m = Motor::cargar(&caso("sin-capacidades")).expect("el caso carga");
    let quien = Identidad {
        emisor: "https://id.example".into(),
        audiencia: "ore".into(),
        sujeto: "emp-42".into(),
        roles: vec!["analyst".into()],
        claims: BTreeMap::from([("employeeId".to_string(), "emp-42".to_string())]),
    };
    let mut c = Consulta {
        quien,
        accion: "read".into(),
        purpose: "compensation_review".into(),
        entidad: "hr.Employee".into(),
        propiedades: vec!["hr.Employee.nationalId".into()],
        claves: vec![vec!["emp-7".to_string()]],
        travesia: None,
        instante: None,
        sla: None,
    };

    // Con clave, plan.
    let plan = m.planificar(&c).expect("por clave sí");
    assert_eq!(plan.lecturas[0].claves, vec![vec!["emp-7".to_string()]]);

    // Sin clave, no — y el rechazo nombra el campo que falta.
    c.claves.clear();
    let r = m.planificar(&c).expect_err("sin clave no");
    let Rechazo::PlanRechazado { campo, .. } = &r else {
        panic!("{r:?}");
    };
    assert_eq!(campo, "capabilities");
}

/// **La fase ③ lee de la vista por su raíz.** La entidad nombra a `iberia`,
/// `iberia` sale de `empleados`, y `empleados` toca `erp`. La lectura va a
/// `erp` / `public.employees` con las columnas **compuestas** por la cadena
/// —`dni` es `nationalId` en `iberia` y `national_id` en la raíz— y con las
/// capacidades de la vista que toca la fuente, que son las de la fuente.
///
/// Ningún binding en el caso. Es el primer plan del proyecto sin uno.
#[test]
fn la_fase_tres_lee_de_la_vista_por_su_raiz() {
    let m = Motor::cargar(&caso("con-vista")).expect("el caso carga");
    let c = Consulta {
        quien: Identidad {
            emisor: "https://id.example".into(),
            audiencia: "ore".into(),
            sujeto: "emp-42".into(),
            roles: vec!["analyst".into()],
            claims: BTreeMap::from([("employeeId".to_string(), "emp-42".to_string())]),
        },
        accion: "read".into(),
        purpose: "compensation_review".into(),
        entidad: "hr.Employee".into(),
        propiedades: vec!["hr.Employee.dni".into()],
        claves: vec![vec!["emp-7".to_string()]],
        travesia: None,
        instante: None,
        sla: None,
    };
    let plan = m.planificar(&c).expect("por clave, y con rol, hay plan");
    assert_eq!(plan.lecturas.len(), 1, "{:?}", plan.lecturas);
    let l = &plan.lecturas[0];
    assert_eq!(l.datasource, "erp");
    assert_eq!(l.objeto, "public.employees");
    assert_eq!(
        l.proyeccion.get("dni").map(String::as_str),
        Some("national_id"),
        "la columna es la de la RAÍZ, compuesta por la cadena: {:?}",
        l.proyeccion
    );
    assert_eq!(l.clave_columnas, vec!["employee_id".to_string()]);

    // Y sin clave, `fullScan: cheap` de la vista que toca la fuente lo permite:
    // las capacidades son de la raíz, no de `iberia`, que no declara ninguna.
    let mut sin_clave = c.clone();
    sin_clave.claves.clear();
    assert!(m.planificar(&sin_clave).is_ok(), "la raíz admite recorrido");
}

/// **La misma fase ③, con el puntero fuera de la vista.** `con-tabla` es
/// `con-vista` en v1alpha8: el objeto se registra una vez como `kind: Table`,
/// la vista solo lo nombra, y las capacidades salen de `reads` — que describen
/// a `erp` y no a quien lo consulta.
///
/// Lo que este caso afirma es que **la fase ③ no se entera**: pide a una
/// fuente, un objeto, unas columnas y unas capacidades, y de dónde salió el
/// puntero es asunto de quien lo declaró. Las cuatro afirmaciones son las
/// mismas que las de `con-vista`, palabra por palabra.
///
/// Y `con-vista` **se queda**: un documento v1alpha7 sigue compilando mientras
/// v1alpha1 sea normativo, así que el ejecutor tiene que seguir sirviéndolo.
/// Los dos casos juntos son, aquí, lo que `valid/mixed-versions` es en la
/// suite de conformidad.
#[test]
fn la_fase_tres_lee_de_la_tabla_por_la_raiz_de_la_vista() {
    let m = Motor::cargar(&caso("con-tabla")).expect("el caso carga");
    let c = Consulta {
        quien: Identidad {
            emisor: "https://id.example".into(),
            audiencia: "ore".into(),
            sujeto: "emp-42".into(),
            roles: vec!["analyst".into()],
            claims: BTreeMap::from([("employeeId".to_string(), "emp-42".to_string())]),
        },
        accion: "read".into(),
        purpose: "compensation_review".into(),
        entidad: "hr.Employee".into(),
        propiedades: vec!["hr.Employee.dni".into()],
        claves: vec![vec!["emp-7".to_string()]],
        travesia: None,
        instante: None,
        sla: None,
    };
    let plan = m.planificar(&c).expect("por clave, y con rol, hay plan");
    assert_eq!(plan.lecturas.len(), 1, "{:?}", plan.lecturas);
    let l = &plan.lecturas[0];

    // La fuente y el objeto salen de la TABLA, no de la vista.
    assert_eq!(l.datasource, "erp");
    assert_eq!(l.objeto, "public.employees");

    // Y la columna sigue siendo la de la raíz, compuesta por la cadena: `dni`
    // es `nationalId` en `iberia` y `national_id` en la tabla.
    assert_eq!(
        l.proyeccion.get("dni").map(String::as_str),
        Some("national_id"),
        "la columna es la de la RAÍZ, compuesta por la cadena: {:?}",
        l.proyeccion
    );
    assert_eq!(l.clave_columnas, vec!["employee_id".to_string()]);

    // Sin clave, `reads.fullScan: cheap` de la TABLA lo permite. Es la
    // afirmación de T2: la capacidad ya no se lee de `capabilities` de la vista
    // —que en v1alpha8 no existe— sino de la cara `I` del objeto.
    let mut sin_clave = c.clone();
    sin_clave.claves.clear();
    assert!(
        m.planificar(&sin_clave).is_ok(),
        "`reads` de la tabla admite recorrido"
    );
}

/// **La forma más fuerte de aplicar una máscara es no pedir la columna.**
#[test]
fn una_propiedad_redactada_no_llega_a_la_proyeccion() {
    let m = Motor::cargar(&caso("redactado")).expect("el caso carga");
    let c = Consulta {
        quien: Identidad {
            emisor: "https://id.example".into(),
            audiencia: "ore".into(),
            sujeto: "emp-42".into(),
            roles: vec!["analyst".into()],
            claims: BTreeMap::from([("employeeId".to_string(), "emp-42".to_string())]),
        },
        accion: "read".into(),
        purpose: "compensation_review".into(),
        entidad: "hr.Employee".into(),
        propiedades: vec!["hr.Employee.nationalId".into()],
        claves: vec![vec!["emp-7".to_string()]],
        travesia: None,
        instante: None,
        sla: None,
    };

    // La política PERMITE, y aun así la columna no se pide: la máscara es
    // `redact`, y redactar después habría traído el valor.
    let r = m.planificar(&c);
    match r {
        Err(Rechazo::NoAutorizado { porque }) => {
            assert!(
                porque.iter().any(|p| p.contains("redactada")),
                "el plan queda vacío porque la única propiedad se redacta, y tiene que decirlo: \
                 {porque:?}"
            );
        }
        otro => panic!("una propiedad redactada no puede acabar en una proyección: {otro:?}"),
    }
}

/// **G1 aplicado a L2.** Y con la forma canónica del bundle, no con una segunda
/// definición de determinismo que podría divergir de la primera.
#[test]
fn mismas_entradas_mismo_plan_byte_a_byte() {
    let m = Motor::cargar(ejemplo()).expect("el ejemplo carga");
    let c = consulta(&["hr.Employee.baseSalary", "hr.Employee.bonus"], &["emp-7"]);
    let a = m.planificar(&c).expect("hay plan").canonico();
    let b = m.planificar(&c).expect("hay plan").canonico();
    assert_eq!(a, b);
    // Y la forma canónica es JCS: sin espacios y con las claves ordenadas.
    assert!(a.starts_with('{') && !a.contains(": "), "{a}");
}

/// El plan se produce **sin ninguna fuente configurada**. La ontología de
/// referencia declara `ACME_WORKDAY_URL` y nadie la ha puesto.
#[test]
fn planificar_no_necesita_ninguna_credencial() {
    assert!(
        std::env::var("ACME_WORKDAY_URL").is_err(),
        "esta prueba no vale nada si la variable está puesta"
    );
    let m = Motor::cargar(ejemplo()).expect("el ejemplo carga");
    assert!(
        m.planificar(&consulta(&["hr.Employee.baseSalary"], &["emp-7"]))
            .is_ok()
    );
}

/// Si ① lo poda todo, no hay plan — y la condición se nombra.
#[test]
fn si_la_politica_lo_poda_todo_no_hay_plan() {
    let m = Motor::cargar(ejemplo()).expect("el ejemplo carga");
    let mut c = consulta(&["hr.Employee.baseSalary"], &["emp-7"]);
    c.quien.roles.clear();
    let r = m
        .planificar(&c)
        .expect_err("sin rol no hay nada autorizado");
    assert!(matches!(r, Rechazo::NoAutorizado { .. }), "{r:?}");
}

/// Una propiedad **autorizada** que ninguna fuente mapea desaparecería de la
/// proyección sin decirlo, y el plan diría ✓ sobre un dato que nunca va a
/// llegar. Que una fuente no lo mapee todo es legal; **callarlo, no**.
///
/// Salió al ejecutar `ore-exec plan` contra la ontología de referencia:
/// `nationalId` estaba autorizada para `read` y el binding de Workday no la
/// mapeaba.
///
/// # Por qué esto ya no se prueba contra `acme-retail`
///
/// Porque allí dejó de poder ocurrir, y por dos cambios distintos. Al ejemplo
/// se le dio columna y campo para `nationalId` —no tenerlos era un defecto
/// suyo— y, más de fondo, **en v1alpha8 una propiedad sin campo es `OOS2022` y
/// no compila**: la comprobación se mudó de tiempo de ejecución a tiempo de
/// compilación.
///
/// En v1alpha7 sigue siendo legal —con bindings otro podía cubrirla, y con
/// vistas nadie obliga a que una las mapee todas— así que aquí es donde esta
/// afirmación tiene que vivir, y por eso `con-vista` declara un `telefono` que
/// su vista no expone. Los dos regímenes conviven mientras v1alpha1 sea
/// normativo, y el ejecutor tiene que servir los dos.
#[test]
fn una_propiedad_que_ninguna_fuente_sirve_se_dice_en_vez_de_desaparecer() {
    let m = Motor::cargar(&caso("con-vista")).expect("el caso carga");
    let c = Consulta {
        quien: Identidad {
            emisor: "https://id.example".into(),
            audiencia: "ore".into(),
            sujeto: "emp-42".into(),
            roles: vec!["analyst".into()],
            claims: BTreeMap::from([("employeeId".to_string(), "emp-42".to_string())]),
        },
        accion: "read".into(),
        purpose: "compensation_review".into(),
        entidad: "hr.Employee".into(),
        propiedades: vec!["hr.Employee.dni".into(), "hr.Employee.telefono".into()],
        claves: vec![vec!["emp-7".to_string()]],
        travesia: None,
        instante: None,
        sla: None,
    };
    let plan = m.planificar(&c).expect("hay plan");

    assert!(plan.autorizadas.contains_key("hr.Employee.dni"));
    assert!(
        !plan.autorizadas.contains_key("hr.Employee.telefono"),
        "no se puede prometer una propiedad que no se va a pedir"
    );
    let porque = plan
        .podadas
        .get("hr.Employee.telefono")
        .expect("tiene que estar podada, con motivo");
    assert!(porque.contains("ningún binding"), "{porque}");
}

/// **La fase ② deja de recibir las claves de fuera.** Con índice, la travesía
/// las computa en local — y por eso, cuando el motor abre una conexión, ya sabe
/// exactamente qué claves pide.
#[test]
fn con_indice_la_fase_dos_produce_las_claves() {
    use ore_exec::{Topologia, Travesia};
    let raiz = Path::new(env!("CARGO_MANIFEST_DIR")).join("casos/jerarquia");
    let digest = {
        let m = Motor::cargar(&raiz).expect("carga");
        ore_core::digest::bundle(&m.paquete)
    };
    let t = Topologia::construir(
        &digest,
        "2026-08-31T12:00:00Z",
        &[
            ("hr.Employee.manager".into(), "emp-42".into(), "jefa".into()),
            ("hr.Employee.manager".into(), "jefa".into(), "ceo".into()),
        ],
    );
    let fichero = std::env::temp_dir().join("ore-topo-fase2.bin");
    std::fs::write(&fichero, t.bytes()).expect("se escribe");

    let mut m = Motor::cargar(&raiz).expect("carga");
    let quien = Identidad {
        emisor: "https://id.example".into(),
        audiencia: "ore".into(),
        sujeto: "emp-42".into(),
        // Con rol: asi ① pasa por la politica de rol y la fase ② se prueba
        // por separado de la jerarquia del principal.
        roles: vec!["analyst".into()],
        claims: BTreeMap::from([("employeeId".to_string(), "emp-42".to_string())]),
    };
    let c = Consulta {
        quien,
        accion: "read".into(),
        purpose: "compensation_review".into(),
        entidad: "hr.Employee".into(),
        propiedades: vec!["hr.Employee.baseSalary".into()],
        claves: vec![],
        travesia: Some(Travesia {
            relacion: "hr.Employee.manager".into(),
            desde: "emp-42".into(),
            saltos: 3,
        }),
        instante: None,
        sla: None,
    };

    // Sin índice: **no es que no haya vecinos, es que no se pudo mirar**.
    let r = m.planificar(&c).expect_err("sin índice no hay travesía");
    assert!(matches!(r, Rechazo::TravesiaNoDisponible { .. }), "{r:?}");

    // Con índice: las claves salen del grafo, no de la consulta.
    m.cargar_topologia(&fichero).expect("es de este bundle");
    let plan = m.planificar(&c).expect("con índice sí");
    assert_eq!(
        plan.claves,
        vec![vec!["ceo".to_string()], vec!["jefa".to_string()]],
        "la fase ② tiene que haber producido las claves de la cadena"
    );
    assert_eq!(plan.lecturas[0].claves.len(), 2);
}
