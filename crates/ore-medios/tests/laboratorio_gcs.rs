//! **Una virtual sobre un bucket de GCS, de verdad** (ADR 0061 O2·4): el camino
//! de `ore-medios` —`origenes::de(fuente)`, con el cliente guardado entre
//! peticiones, y el `Origen` de `ore-gcs`— contra `fake-gcs-server`. Lo arranca
//! `pruebas-de-fuego/o2-gcs.sh`, que da la URL en `ORE_LAB_GCS`; sin ella, no
//! hay nada que probar y el test lo dice.
//!
//! Sobre `docs/a.pdf` de la muestra: el rango (`206`), el ítem entero con la
//! huella que GCS da (`crc32c`), una generación que no está (`media/cambiado`,
//! nunca otros bytes) y la URL firmada, que el emulador no sabe dar (no hay
//! IAM): se dice como fallo, no como «este origen no firma».

use ore_objetos::Rechazo;
use ore_objetos::huella::{Calculo, Crc32c};
use std::io::Read;

#[test]
fn una_virtual_sobre_un_bucket_de_gcs() {
    let Ok(url) = std::env::var("ORE_LAB_GCS") else {
        eprintln!("sin ORE_LAB_GCS no hay laboratorio: nada que probar (o2-gcs.sh)");
        return;
    };
    let o = ore_medios::origenes::de(&url).unwrap_or_else(|r| panic!("{}", r.cuerpo.jcs()));
    let v = o
        .listar_versiones("docs/")
        .expect("versiones")
        .into_iter()
        .find(|v| v.clave == "docs/a.pdf" && v.actual)
        .expect("docs/a.pdf no está");

    // el rango
    let mut l = o
        .leer_fijado(&v.clave, &v.version, &v.etag, Some("bytes=0-3"))
        .unwrap_or_else(|e| panic!("{e:?}"));
    let mut b = Vec::new();
    l.lector.read_to_end(&mut b).unwrap();
    assert_eq!((l.estado, b.as_slice()), (206, &b"%PDF"[..]));

    // entero, y su crc32c es el que GCS da sin bajarlo
    let mut l = o.leer_fijado(&v.clave, &v.version, &v.etag, None).unwrap();
    let mut b = Vec::new();
    l.lector.read_to_end(&mut b).unwrap();
    assert_eq!((l.estado, b.len() as u64), (200, v.tamano));
    let mut c = Crc32c::default();
    c.sumar(&b);
    assert_eq!(o.huella_de(&v.clave, &v.version).unwrap(), Some(c.texto()));

    // una generación que no está: cambiado, nunca otros bytes
    match o.leer_fijado(&v.clave, "1", "", None) {
        Err(Rechazo::Cambiado(_)) => {}
        Err(e) => panic!("otro rechazo: {e:?}"),
        Ok(l) => panic!("dio {} de una generación que no está", l.estado),
    }

    // la misma fuente, otra petición: el mismo cliente, y lee igual
    let o2 = ore_medios::origenes::de(&url).unwrap_or_else(|r| panic!("{}", r.cuerpo.jcs()));
    assert_eq!(
        o2.listar("docs/").unwrap().len(),
        o.listar("docs/").unwrap().len()
    );

    // la URL firmada: sin IAM no se firma, y se dice (502 por ítem, no «no firma»)
    assert!(
        o.firmar(&v.clave, &v.version, "application/pdf", "inline", 60)
            .is_err()
    );
    eprintln!("gcs por ore-medios: rango, entero con crc32c, generación que no está → cambiado");
}
