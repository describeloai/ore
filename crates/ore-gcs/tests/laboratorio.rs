//! **Un bucket de GCS como origen, contra el emulador** (ADR 0061 O2·1): el
//! `Origen` de `ore-gcs` contra `fake-gcs-server`, con la muestra que siembra
//! la prueba de fuego (`docs/a.pdf` en dos generaciones: `% uno` y luego
//! `% UNO`). La URL llega en `ORE_LAB_GCS`; sin ella, no hay laboratorio y el
//! test lo dice. Lo que el emulador no sabe (la firma, `testIamPermissions`, el
//! token) se comprueba que se dice como tal, no que funcione.

use ore_objetos::huella::{Calculo, Crc32c};
use ore_objetos::{Origen, Rechazo};
use std::io::Read;

#[test]
fn un_bucket_de_gcs_como_origen() {
    let Ok(url) = std::env::var("ORE_LAB_GCS") else {
        eprintln!("sin ORE_LAB_GCS no hay laboratorio: nada que probar (o2-gcs.sh)");
        return;
    };
    let g = ore_gcs::Gcs::de_url(&url).expect("la URL");
    let o: &dyn Origen = &g;

    // el listado: la generación es el validador
    let objs = o.listar("docs/").expect("listar");
    assert_eq!(objs.len(), 2, "{objs:?}");
    let a = objs
        .iter()
        .find(|x| x.clave == "docs/a.pdf")
        .expect("docs/a.pdf");
    assert!(a.etag.parse::<u64>().is_ok(), "la generación: {}", a.etag);

    // las versiones: las dos generaciones de a.pdf, una vigente
    let vs: Vec<_> = o
        .listar_versiones("docs/")
        .expect("versiones")
        .into_iter()
        .filter(|v| v.clave == "docs/a.pdf")
        .collect();
    assert_eq!(vs.len(), 2, "{vs:?}");
    let hoy = vs.iter().find(|v| v.actual).expect("la vigente");
    let antes = vs.iter().find(|v| !v.actual).expect("la de antes");
    assert_eq!(hoy.version, a.etag);

    // la huella sin bajar: el crc32c de los bytes de hoy
    let mut l = o
        .leer_fijado(&hoy.clave, &hoy.version, "", None)
        .expect("leer hoy");
    let mut bytes = Vec::new();
    l.lector.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"%PDF-1.4\n% UNO\n%%EOF\n");
    let mut c = Crc32c::default();
    c.sumar(&bytes);
    assert_eq!(
        o.huella_de(&hoy.clave, &hoy.version).unwrap(),
        Some(c.texto())
    );

    // la generación de antes se sigue leyendo, entera y con su huella
    let (mut r, ab) = o
        .abrir_version(&antes.clave, &antes.version)
        .expect("la de antes");
    let mut viejos = Vec::new();
    r.read_to_end(&mut viejos).unwrap();
    assert_eq!(viejos, b"%PDF-1.4\n% uno\n%%EOF\n");
    let mut c = Crc32c::default();
    c.sumar(&viejos);
    assert_eq!(ab.huella, Some(c.texto()), "x-goog-hash");
    assert_eq!(ab.tamano, Some(21));

    // un rango de la vigente
    let mut l = o
        .leer_fijado(&hoy.clave, &hoy.version, "", Some("bytes=0-3"))
        .expect("rango");
    let mut b = Vec::new();
    l.lector.read_to_end(&mut b).unwrap();
    assert_eq!((l.estado, b.as_slice()), (206, &b"%PDF"[..]));

    // una generación que no está: cambiado, nunca otros bytes
    match o.leer_fijado(&hoy.clave, "999", "", None) {
        Err(Rechazo::Cambiado(_)) => {}
        Err(e) => panic!("otro rechazo: {e:?}"),
        Ok(l) => panic!("dio {} de una generación que no está", l.estado),
    }
    assert!(
        o.abrir("docs/a.pdf", "999").is_err(),
        "fijado por un validador que no es"
    );

    // lo que el emulador no sabe se dice como tal
    assert_eq!(
        g.permisos(&["storage.objects.list"]).expect("permisos"),
        None
    );
    assert!(
        o.firmar(&hoy.clave, &hoy.version, "application/pdf", "inline", 60)
            .is_err(),
        "sin IAM no se firma: se dice, no se inventa"
    );
    eprintln!("gcs: listar, 2 generaciones, crc32c, la de antes, rango, 404 → cambiado");
}
