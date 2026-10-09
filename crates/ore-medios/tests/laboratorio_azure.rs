//! **Una virtual sobre un contenedor de Azure, de verdad** (ADR 0061 O3·4): el
//! camino de `ore-medios` —`origenes::de(fuente)`, con el cliente guardado
//! entre peticiones, y el `Origen` de `ore-azure`— contra Azurite. Lo arranca
//! `pruebas-de-fuego/o3-azure.sh`, que da la URL en `ORE_LAB_AZURE` (y el
//! Bearer en `ORE_AZURE_TOKEN`); sin ella, no hay nada que probar y el test lo
//! dice.
//!
//! Azurite no versiona: el ítem se fija por su ETag (D-O1). Sobre `docs/a.pdf`:
//! el rango (`206`), por el final (que Blob no entiende y se resuelve con el
//! tamaño), entero con el `Content-MD5` que Azure da, un ETag que ya no es
//! (`media/cambiado`, nunca otros bytes), y la URL firmada, que un ítem fijado
//! sólo por ETag no da: se abre por `content`.

use ore_objetos::Rechazo;
use ore_objetos::huella::{Calculo, Md5};
use std::io::Read;

#[test]
fn una_virtual_sobre_un_contenedor_de_azure() {
    let Ok(url) = std::env::var("ORE_LAB_AZURE") else {
        eprintln!("sin ORE_LAB_AZURE no hay laboratorio: nada que probar (o3-azure.sh)");
        return;
    };
    let o = ore_medios::origenes::de(&url).unwrap_or_else(|r| panic!("{}", r.cuerpo.jcs()));
    let v = o
        .listar_versiones("docs/")
        .expect("versiones")
        .into_iter()
        .find(|v| v.clave == "docs/a.pdf" && v.actual)
        .expect("docs/a.pdf no está");

    let leer = |rango: Option<&str>| {
        let mut l = o
            .leer_fijado(&v.clave, &v.version, &v.etag, rango)
            .unwrap_or_else(|e| panic!("{e:?}"));
        let mut b = Vec::new();
        l.lector.read_to_end(&mut b).unwrap();
        (l.estado, b)
    };
    assert_eq!(leer(Some("bytes=0-3")), (206, b"%PDF".to_vec()));
    assert_eq!(leer(Some("bytes=-6")), (206, b"%%EOF\n".to_vec()));

    // entero, y su MD5 es el que Azure da sin bajarlo
    let (estado, b) = leer(None);
    assert_eq!((estado, b.len() as u64), (200, v.tamano));
    let mut c = Md5::default();
    c.sumar(&b);
    assert_eq!(o.huella_de(&v.clave, &v.version).unwrap(), Some(c.texto()));

    // un ETag que ya no es: cambiado, nunca otros bytes
    match o.leer_fijado(&v.clave, "etag:0xdead", "", None) {
        Err(Rechazo::Cambiado(_)) => {}
        Err(e) => panic!("otro rechazo: {e:?}"),
        Ok(l) => panic!("dio {} con un ETag que no es", l.estado),
    }

    // la misma fuente, otra petición: el mismo cliente, y lee igual
    let o2 = ore_medios::origenes::de(&url).unwrap_or_else(|r| panic!("{}", r.cuerpo.jcs()));
    assert_eq!(
        o2.listar("docs/").unwrap().len(),
        o.listar("docs/").unwrap().len()
    );

    // fijado sólo por ETag: sin URL firmada (D-O1), se abre por `content`
    assert_eq!(
        o.firmar(&v.clave, &v.version, "application/pdf", "inline", 60)
            .unwrap(),
        None
    );
    eprintln!(
        "azure por ore-medios: rango, por el final, entero con md5, ETag viejo → cambiado, sin URL (D-O1)"
    );
}
