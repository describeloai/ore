//! **Una virtual sobre lo que habla S3, de verdad** (ADR 0061 O1·3): el camino
//! de `ore-medios` —`origenes::de(fuente)` y el `Origen` del proveedor— contra
//! los emuladores del laboratorio. Lo arranca
//! `pruebas-de-fuego/o1-los-que-hablan-s3.sh`, que da las URLs de cada uno en
//! `ORE_LAB_S3` (`nombre=url` separados por `|`, con la credencial de prueba);
//! sin él, no hay nada que probar y el test lo dice.
//!
//! En cada origen, sobre `docs/a.pdf` de la muestra: el rango (`206`), el ítem
//! entero, un `If-Match` que ya no casa (`media/cambiado`, nunca otros bytes),
//! y la URL firmada: la da un origen con versiones y la niega uno que sólo fija
//! por ETag (D-O1).

use ore_objetos::{Rechazo, etag_de};
use std::io::Read;

const A_PDF: &[u8] = b"%PDF-1.4\n% uno\n%%EOF\n";

#[test]
fn una_virtual_sobre_lo_que_habla_s3() {
    let Ok(lab) = std::env::var("ORE_LAB_S3") else {
        eprintln!("sin ORE_LAB_S3 no hay laboratorio: nada que probar (o1-los-que-hablan-s3.sh)");
        return;
    };
    for par in lab.split('|').filter(|p| !p.is_empty()) {
        let (nombre, url) = par.split_once('=').expect("nombre=url");
        let o = ore_medios::origenes::de(url)
            .unwrap_or_else(|r| panic!("{nombre}: {}", r.cuerpo.jcs()));
        let v = o
            .listar_versiones("docs/")
            .unwrap_or_else(|e| panic!("{nombre}: {e}"))
            .into_iter()
            .find(|v| v.clave == "docs/a.pdf" && v.actual)
            .unwrap_or_else(|| panic!("{nombre}: docs/a.pdf no está"));
        let por_etag = etag_de(&v.version).is_some();

        // el rango
        let mut l = o
            .leer_fijado(&v.clave, &v.version, &v.etag, Some("bytes=0-3"))
            .unwrap_or_else(|e| panic!("{nombre}: {e:?}"));
        let mut b = Vec::new();
        l.lector.read_to_end(&mut b).unwrap();
        assert_eq!((l.estado, b.as_slice()), (206, &b"%PDF"[..]), "{nombre}");
        assert!(
            l.cabecera("content-range")
                .is_some_and(|c| c.starts_with("bytes 0-3/")),
            "{nombre}"
        );

        // entero
        let mut l = o.leer_fijado(&v.clave, &v.version, &v.etag, None).unwrap();
        let mut b = Vec::new();
        l.lector.read_to_end(&mut b).unwrap();
        assert_eq!((l.estado, b.as_slice()), (200, A_PDF), "{nombre}");

        // un If-Match que ya no casa: cambiado, nunca otros bytes
        match o.leer_fijado(&v.clave, &v.version, "\"otro\"", None) {
            Err(Rechazo::Cambiado(_)) => {}
            Err(e) => panic!("{nombre}: otro rechazo: {e:?}"),
            Ok(l) => panic!("{nombre}: dio {} con un ETag que no es", l.estado),
        }

        // la URL firmada: con versiones sí; fijado sólo por ETag, no (D-O1)
        let url = o
            .firmar(&v.clave, &v.version, "application/pdf", "inline", 60)
            .unwrap_or_else(|e| panic!("{nombre}: {e}"));
        assert_eq!(url.is_some(), !por_etag, "{nombre}: {}", v.version);
        if let Some(u) = url {
            assert!(
                u.contains("X-Amz-Signature=") && !u.contains("secret"),
                "{nombre}"
            );
        }
        eprintln!(
            "{nombre}: fija por {} · rango, entero, 412 y la URL {}",
            if por_etag { "ETag" } else { "versión" },
            if por_etag { "negada" } else { "firmada" }
        );
    }
}
