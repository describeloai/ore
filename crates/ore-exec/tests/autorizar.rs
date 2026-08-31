//! El criterio central de M0: **dos principales, mismo recurso, veredictos
//! distintos — y el veredicto nombra la política que decidió.**
//!
//! Y la mitad que Cedar no da: un `Deny` mudo no sirve. Aquí se distingue
//! *«ninguna política alcanza esto»* de *«hay políticas que lo alcanzan y
//! ninguna casó»*, que es lo único que le sirve a quien escribe políticas.

use ore_exec::{Denegacion, Identidad, Motor, Peticion, Veredicto};
use std::collections::BTreeMap;
use std::path::Path;

fn ejemplo() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("vendor/oos/examples/acme-retail")
        .leak()
}

fn peticion(roles: &[&str], propiedad: &str, purpose: &str) -> Peticion {
    Peticion {
        quien: Identidad {
            emisor: "https://id.acme.example".into(),
            audiencia: "ore".into(),
            sujeto: "emp-42".into(),
            roles: roles.iter().map(|r| r.to_string()).collect(),
            claims: BTreeMap::from([
                ("employeeId".to_string(), "emp-42".to_string()),
                ("departmentId".to_string(), "finanzas".to_string()),
            ]),
        },
        accion: "read".into(),
        propiedad: propiedad.into(),
        purpose: purpose.into(),
    }
}

#[test]
fn dos_principales_mismo_recurso_veredictos_distintos() {
    let m = Motor::cargar(ejemplo()).expect("el ejemplo carga");
    let recurso = "hr.Employee.baseSalary";

    let con = m.autorizar(&peticion(&["hr_analyst"], recurso, "compensation_review"));
    let sin = m.autorizar(&peticion(&[], recurso, "compensation_review"));

    // El que tiene el rol pasa, y el veredicto trae lo que hay que aplicar.
    let Veredicto::Permitido {
        politicas,
        obligaciones,
        mascaras,
        ambitos,
    } = &con
    else {
        panic!("con el rol tenía que pasar, y salió {con:?}");
    };
    // El `@id` NUESTRO, no el `policyN` de Cedar: es la identidad que sobrevive
    // a mover la política de línea (ADR 0003).
    assert_eq!(politicas, &["hr-analyst-reads-masked-comp"], "{con:?}");
    assert_eq!(obligaciones, &["mask:LAST4"], "{con:?}");
    assert!(mascaras.is_empty(), "{con:?}");
    assert_eq!(
        ambitos,
        &["eu.gdpr-minimization#own-department"],
        "el veredicto tiene que traer el ámbito, o el ejecutor no sabría qué filtro empujar"
    );

    // El mismo recurso y la misma finalidad, sin el rol: denegado.
    let Veredicto::Denegado { porque, .. } = &sin else {
        panic!("sin el rol tenía que denegar, y salió {sin:?}");
    };
    // Y NO mudo: hay políticas que alcanzan esa propiedad, y se nombran.
    let Denegacion::NingunaCaso { candidatas } = porque else {
        panic!("la denegación tenía que decir contra qué mirar, y dijo {porque:?}");
    };
    assert!(
        candidatas.contains(&"hr-analyst-reads-masked-comp".to_string()),
        "{candidatas:?}"
    );
}

/// Un `forbid` sí tiene a quién señalar, y hay que señalarlo.
#[test]
fn el_forbid_se_nombra() {
    let m = Motor::cargar(ejemplo()).expect("el ejemplo carga");
    let mut p = peticion(&["hr_analyst"], "hr.Employee.nationalId", "regulatory_reporting");
    p.accion = "export".into();

    let v = m.autorizar(&p);
    let Veredicto::Denegado { politicas, porque } = &v else {
        panic!("el DNI no sale de ningún modo, y salió {v:?}");
    };
    assert_eq!(porque, &Denegacion::Prohibida, "{v:?}");
    assert_eq!(politicas, &["forbid-national-id-egress"], "{v:?}");
}

/// Ninguna política alcanza esa propiedad. **No es un fallo: es P4.**
#[test]
fn sin_politica_que_lo_alcance_es_denegacion_por_defecto() {
    let m = Motor::cargar(ejemplo()).expect("el ejemplo carga");
    let v = m.autorizar(&peticion(
        &["hr_analyst"],
        "customers.Customer.email",
        "compensation_review",
    ));
    let Veredicto::Denegado { porque, .. } = &v else {
        panic!("nadie la autoriza, y salió {v:?}");
    };
    assert_eq!(porque, &Denegacion::SinPolitica, "{v:?}");
}

/// Una finalidad que la política no admite deniega, y **dice contra qué mirar**.
#[test]
fn una_finalidad_ajena_no_deniega_en_mudo() {
    let m = Motor::cargar(ejemplo()).expect("el ejemplo carga");
    let v = m.autorizar(&peticion(&["hr_analyst"], "hr.Employee.baseSalary", "marketing"));
    let Veredicto::Denegado { porque, .. } = &v else {
        panic!("`marketing` no está entre las finalidades de esa política, y salió {v:?}");
    };
    assert!(
        matches!(porque, Denegacion::NingunaCaso { .. }),
        "una denegación por finalidad tiene que nombrar las candidatas, y dijo {porque:?}"
    );
}

/// La frontera de `06-request`: un token acuñado para otro destinatario es un
/// token robado, aunque lo firme quien debe. Y eso **no es una denegación**: es
/// una petición que no existe.
#[test]
fn otro_emisor_u_otra_audiencia_no_son_una_peticion() {
    let m = Motor::cargar(ejemplo()).expect("el ejemplo carga");

    let mut ajeno = peticion(&["hr_analyst"], "hr.Employee.baseSalary", "compensation_review");
    ajeno.quien.emisor = "https://id.otra.example".into();
    assert!(
        matches!(m.autorizar(&ajeno), Veredicto::Invalida(_)),
        "un emisor ajeno no puede producir una decisión de política"
    );

    let mut otra = peticion(&["hr_analyst"], "hr.Employee.baseSalary", "compensation_review");
    otra.quien.audiencia = "otro-servicio".into();
    assert!(
        matches!(m.autorizar(&otra), Veredicto::Invalida(_)),
        "una audiencia ajena tampoco"
    );
}

/// Y una reclamación que el esquema no declara **no llega al evaluador**, aunque
/// venga firmada. Es P4 en la entrada, y lo hace cumplir el propio almacén.
#[test]
fn una_reclamacion_no_declarada_no_se_cree() {
    let m = Motor::cargar(ejemplo()).expect("el ejemplo carga");
    let mut p = peticion(&["hr_analyst"], "hr.Employee.baseSalary", "compensation_review");
    p.quien.claims.insert("clearance".into(), "SECRET".into());
    let v = m.autorizar(&p);
    assert!(
        matches!(v, Veredicto::Invalida(_)),
        "la reclamación de más tenía que invalidar la petición, y salió {v:?}"
    );
}

/// **El hueco nombrado.** Una política del lado del principal —*«el que pregunta
/// está bajo esa cadena»*— es expresable y hoy no se puede evaluar: sus aristas
/// viven en el índice de topología, que es de M3.
///
/// Sin él la política **evalúa a falso**, así que sale el mismo `Deny` que si no
/// existiera. Denegar es correcto —P4—; **callar por qué, no**: una denegación
/// por falta de un subsistema tiene exactamente el mismo aspecto que una por
/// política, y ese es el modo de fallo que este proyecto persigue.
#[test]
fn una_politica_que_exige_la_cadena_lo_dice_en_vez_de_denegar_en_mudo() {
    let raiz = Path::new(env!("CARGO_MANIFEST_DIR")).join("casos/jerarquia");
    let m = Motor::cargar(&raiz).expect("el caso carga");

    let p = Peticion {
        quien: Identidad {
            emisor: "https://id.example".into(),
            audiencia: "ore".into(),
            sujeto: "emp-42".into(),
            roles: vec![],
            claims: BTreeMap::from([("employeeId".to_string(), "emp-42".to_string())]),
        },
        accion: "read".into(),
        propiedad: "hr.Employee.baseSalary".into(),
        purpose: "compensation_review".into(),
    };

    let v = m.autorizar(&p);
    let Veredicto::Denegado { porque, .. } = &v else {
        panic!("sin índice no se puede permitir, y salió {v:?}");
    };
    let Denegacion::JerarquiaNoDisponible { candidatas } = porque else {
        panic!(
            "tenía que decir que no pudo evaluarla, no que ninguna casó — dijo {porque:?}"
        );
    };
    assert_eq!(candidatas, &["under-the-ceo-reads-comp"], "{v:?}");
}

/// Y con el índice cargado, **el hueco se cierra**: la misma política que ayer
/// decía *jerarquía no disponible* evalúa, y el que está bajo la cadena pasa.
#[test]
fn con_el_indice_la_cadena_del_principal_se_recorre() {
    use ore_exec::Topologia;
    let raiz = Path::new(env!("CARGO_MANIFEST_DIR")).join("casos/jerarquia");

    // El índice, construido contra ESTE bundle. Sin esa correspondencia no se
    // carga: las aristas serían de un modelo y las políticas de otro.
    let digest = {
        let m = Motor::cargar(&raiz).expect("carga");
        ore_core::digest::bundle(&m.paquete)
    };
    let t = Topologia::construir(
        &digest,
        "2026-08-31T10:00:00Z",
        &[
            ("hr.Employee.manager".into(), "emp-42".into(), "jefa".into()),
            ("hr.Employee.manager".into(), "jefa".into(), "ceo".into()),
        ],
    );
    let fichero = std::env::temp_dir().join("ore-topo-prueba.bin");
    std::fs::write(&fichero, t.bytes()).expect("se escribe");

    let mut m = Motor::cargar(&raiz).expect("carga");
    m.cargar_topologia(&fichero).expect("el índice es de este bundle");

    let p = Peticion {
        quien: Identidad {
            emisor: "https://id.example".into(),
            audiencia: "ore".into(),
            sujeto: "emp-42".into(),
            roles: vec![],
            claims: BTreeMap::from([("employeeId".to_string(), "emp-42".to_string())]),
        },
        accion: "read".into(),
        propiedad: "hr.Employee.baseSalary".into(),
        purpose: "compensation_review".into(),
    };

    // `emp-42` está bajo `ceo` a dos saltos, y la política es `principal in
    // Employee::"ceo"`. Con el índice, casa.
    let v = m.autorizar(&p);
    let Veredicto::Permitido { politicas, .. } = &v else {
        panic!("con el índice la cadena se recorre, y salió {v:?}");
    };
    assert_eq!(politicas, &["under-the-ceo-reads-comp"], "{v:?}");
}

/// Un índice de otro bundle **no se carga**. Las aristas serían de un modelo y
/// las políticas de otro, y esa junta no falla: devuelve filas.
#[test]
fn un_indice_de_otro_bundle_se_rechaza_al_cargarlo() {
    use ore_exec::Topologia;
    let raiz = Path::new(env!("CARGO_MANIFEST_DIR")).join("casos/jerarquia");
    let t = Topologia::construir(
        "sha256:de-otro-sitio",
        "w",
        &[("hr.Employee.manager".into(), "a".into(), "b".into())],
    );
    let fichero = std::env::temp_dir().join("ore-topo-ajena.bin");
    std::fs::write(&fichero, t.bytes()).expect("se escribe");

    let mut m = Motor::cargar(&raiz).expect("carga");
    let e = m.cargar_topologia(&fichero).expect_err("no puede cargarse");
    assert!(e.contains("de-otro-sitio"), "{e}");
}
