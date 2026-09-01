//! E4: **un plan cuya caché sirve no abre ninguna conexión al origen.**
//!
//! Y la otra mitad de la frase, que es la que se audita: **la respuesta dice de
//! dónde salió cada lectura**. Tres cosas producen la misma lectura contra el
//! origen —no hay manifiesto, lo hay y está rancio, lo hay y se escribió bajo
//! otra regla— y no significan lo mismo. La tercera es la que alguien tiene que
//! ver.
//!
//! Como el resto de `plan.rs`, todo esto corre **sin una sola variable de
//! entorno**: servirse de la caché es una decisión de planificación, así que se
//! decide antes de que exista ninguna conexión y se puede probar sin ninguna.
//!
//! La caché se prueba con un manifiesto escrito a mano, que es exactamente como
//! se probó el índice de topología antes de que hubiera con qué construirlo.

use ore_exec::{Consulta, Identidad, Motor, Origen};
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

fn consulta() -> Consulta {
    Consulta {
        quien: Identidad {
            emisor: "https://id.acme.example".into(),
            audiencia: "ore".into(),
            sujeto: "emp-42".into(),
            roles: vec!["hr_analyst".into()],
            claims: BTreeMap::from([
                ("employeeId".to_string(), "emp-42".to_string()),
                ("departmentId".to_string(), "finanzas".to_string()),
            ]),
        },
        accion: "read".into(),
        purpose: "compensation_review".into(),
        entidad: "hr.Employee".into(),
        propiedades: vec!["hr.Employee.baseSalary".into()],
        claves: vec![vec!["emp-7".to_string()]],
        travesia: None,
        instante: None,
        sla: None,
    }
}

/// Un manifiesto en un fichero temporal propio de cada prueba: compartir uno
/// haría que dos pruebas en paralelo se pisaran, que es un fallo que aparece y
/// desaparece.
fn manifiesto(nombre: &str, bundle: &str, marca: &str, props: &[&str]) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ore-e4-{}-{nombre}", std::process::id()));
    std::fs::create_dir_all(&d).expect("se crea");
    let ruta = d.join("cache.json");
    let lista: Vec<String> = props.iter().map(|p| format!("\"{p}\"")).collect();
    std::fs::write(
        &ruta,
        format!(
            r#"{{"oreCache":1,"entries":[{{"bundle":"{bundle}","entity":"hr.Employee",
               "properties":[{}],"table":"lago.cache.hr_employee","datasource":"lago",
               "watermark":"{marca}"}}]}}"#,
            lista.join(",")
        ),
    )
    .expect("se escribe");
    ruta
}

fn motor() -> Motor {
    Motor::cargar(ejemplo()).expect("el ejemplo carga")
}

fn bundle(m: &Motor) -> String {
    ore_core::digest::bundle(&m.paquete)
}

/// **El criterio 1.** La lectura ya no apunta al origen: apunta a la tabla del
/// lago, y su proyección es la identidad porque la caché nombra sus columnas
/// como las propiedades.
#[test]
fn un_plan_cuya_cache_sirve_no_lee_del_origen() {
    let mut m = motor();
    let f = manifiesto(
        "sirve",
        &bundle(&m),
        "2026-08-31T10:00:00Z",
        &["employeeId", "baseSalary", "departmentId"],
    );
    m.cargar_cache(&f).expect("el manifiesto se lee");

    let plan = m.planificar(&consulta()).expect("hay plan");
    let l = &plan.lecturas[0];
    assert_eq!(l.datasource, "lago", "seguía apuntando al origen");
    assert_eq!(l.objeto, "lago.cache.hr_employee");
    assert_eq!(
        l.proyeccion.get("baseSalary").map(String::as_str),
        Some("baseSalary"),
        "la columna de la caché se llama como la propiedad: {:?}",
        l.proyeccion
    );
    assert_eq!(l.clave_columnas, vec!["employeeId".to_string()]);
    assert!(
        matches!(&l.origen, Origen::Cache { marca } if marca == "2026-08-31T10:00:00Z"),
        "{:?}",
        l.origen
    );

    // Y el ámbito sigue siendo un predicado, **reescrito**: sin esto el filtro
    // que restringe lo que el principal puede ver apuntaría a una columna que
    // esa tabla no tiene, y devolvería filas de más.
    assert_eq!(l.filtros.len(), 1, "{:?}", l.filtros);
    assert_eq!(l.filtros[0].columna, "departmentId");
    assert_eq!(l.filtros[0].valor, "finanzas");

    // Y la forma canónica lo dice: un plan que leyera de dos sitios distintos y
    // saliera igual habría dejado de describir lo que pasó.
    assert!(
        plan.canonico().contains(r#""de":"cache""#),
        "{}",
        plan.canonico()
    );
}

/// **El criterio 2.** Una caché escrita bajo otro bundle manda al origen —eso ya
/// lo probaba `ore_core::cache`— **con el motivo dentro del plan**, que es lo
/// que aquí se comprueba. Sin él, esa lectura sería indistinguible de no tener
/// caché.
#[test]
fn una_cache_de_otro_bundle_manda_al_origen_con_el_motivo_dentro() {
    let mut m = motor();
    let f = manifiesto(
        "regla",
        "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        "2026-08-31T10:00:00Z",
        &["employeeId", "baseSalary", "departmentId"],
    );
    m.cargar_cache(&f).expect("el manifiesto se lee");

    let plan = m.planificar(&consulta()).expect("hay plan");
    let l = &plan.lecturas[0];
    assert_eq!(l.datasource, "hr_workday");
    let Origen::Fuente { porque: Some(x) } = &l.origen else {
        panic!("el motivo tiene que viajar: {:?}", l.origen);
    };
    assert!(x.contains("regla distinta"), "{x}");
    assert!(
        plan.canonico().contains("regla distinta"),
        "{}",
        plan.canonico()
    );
}

/// Sin manifiesto la lectura es del origen **y no da un motivo**, porque no lo
/// hay. Distinguir «no había caché» de «la había y no servía» es la razón de que
/// el campo sea opcional.
#[test]
fn sin_manifiesto_no_se_inventa_un_motivo() {
    let m = motor();
    let plan = m.planificar(&consulta()).expect("hay plan");
    assert!(
        matches!(&plan.lecturas[0].origen, Origen::Fuente { porque: None }),
        "{:?}",
        plan.lecturas[0].origen
    );
}

/// La caché tiene la propiedad que se pide y **no la columna del ámbito**.
/// Servirla dejaría el predicado sin dónde aplicarse, y esa es la que devuelve
/// filas de más — la peor de las tres.
#[test]
fn una_cache_sin_la_columna_del_ambito_no_sirve() {
    let mut m = motor();
    let f = manifiesto(
        "ambito",
        &bundle(&m),
        "2026-08-31T10:00:00Z",
        &["employeeId", "baseSalary"],
    );
    m.cargar_cache(&f).expect("el manifiesto se lee");

    let plan = m.planificar(&consulta()).expect("hay plan");
    let Origen::Fuente { porque: Some(x) } = &plan.lecturas[0].origen else {
        panic!("{:?}", plan.lecturas[0].origen);
    };
    assert!(x.contains("departmentId"), "{x}");
}

/// Y pasado el SLA se va al origen diciendo que lo que hay está rancio — no
/// callándolo, y no confundiéndolo con que no hubiera caché.
#[test]
fn pasado_el_sla_se_va_al_origen_y_lo_dice() {
    let mut m = motor();
    let f = manifiesto(
        "rancia",
        &bundle(&m),
        "2026-08-31T10:00:00Z",
        &["employeeId", "baseSalary", "departmentId"],
    );
    m.cargar_cache(&f).expect("el manifiesto se lee");

    let mut c = consulta();
    c.instante = Some("2026-08-31T12:00:00Z".into());
    c.sla = Some("1h".into());
    let plan = m.planificar(&c).expect("hay plan");
    assert_eq!(plan.lecturas[0].datasource, "hr_workday");
    let Origen::Fuente { porque: Some(x) } = &plan.lecturas[0].origen else {
        panic!("{:?}", plan.lecturas[0].origen);
    };
    assert!(x.contains("rancia"), "{x}");

    // Y dentro del SLA, la misma caché sirve. Sin esta mitad, la anterior
    // pasaría igual con una caché que no sirviera nunca.
    c.instante = Some("2026-08-31T10:30:00Z".into());
    let plan = m.planificar(&c).expect("hay plan");
    assert_eq!(plan.lecturas[0].datasource, "lago");
}
