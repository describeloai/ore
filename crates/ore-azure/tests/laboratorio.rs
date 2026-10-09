//! **Un contenedor de Azure como origen, contra Azurite** (ADR 0061 O3·1): el
//! `Origen` de `ore-azure` por HTTPS con `--oauth basic` (un Bearer: Azurite
//! mira emisor, audiencia y fechas, no la firma), sobre la muestra que siembra
//! el laboratorio: `docs/a.pdf` reescrito (`% uno` y luego `% UNO`, por Put
//! Blob: con `Content-MD5`), `docs/Nueva carpeta/b.pdf`, y `grande.bin` por
//! bloques (sin MD5). La URL llega en `ORE_LAB_AZURE` y el token en
//! `ORE_AZURE_TOKEN`; sin ellos, no hay laboratorio y el test lo dice.
//!
//! Azurite no versiona (O3·0): todo se fija por ETag, y lo que se prueba de la
//! versión es la guarda —un `versionid` que ignora no basta para dar otros
//! bytes, porque el ETag va siempre—. El versionado, contra Azure de verdad.

use ore_objetos::huella::{Calculo, Md5};
use ore_objetos::{Origen, Rechazo, etag_de};
use std::io::Read;

const UNO: &[u8] = b"%PDF-1.4\n% UNO\n%%EOF\n";

#[test]
fn un_contenedor_de_azure_como_origen() {
    let Ok(url) = std::env::var("ORE_LAB_AZURE") else {
        eprintln!("sin ORE_LAB_AZURE no hay laboratorio: nada que probar (o3-azure.sh)");
        return;
    };
    let az = ore_azure::Azure::de_url(&url).expect("la URL");
    let o: &dyn Origen = &az;

    // el listado
    let objs = o.listar("docs/").expect("listar");
    assert_eq!(objs.len(), 2, "{objs:?}");

    // las versiones: sin versionado, la versión es el ETag
    let v = o
        .listar_versiones("docs/")
        .expect("versiones")
        .into_iter()
        .find(|v| v.clave == "docs/a.pdf")
        .expect("docs/a.pdf");
    assert!(v.actual && etag_de(&v.version).is_some(), "{v:?}");

    // la huella sin bajar: el Content-MD5 de los bytes de hoy
    let mut c = Md5::default();
    c.sumar(UNO);
    assert_eq!(o.huella_de(&v.clave, &v.version).unwrap(), Some(c.texto()));
    // un blob por bloques no tiene: se coteja por tamaño
    assert_eq!(o.huella_de("grande.bin", "").unwrap(), None);

    // entero y un rango, fijados
    let mut l = o
        .leer_fijado(&v.clave, &v.version, "", None)
        .expect("entero");
    let mut b = Vec::new();
    l.lector.read_to_end(&mut b).unwrap();
    assert_eq!((l.estado, b.as_slice()), (200, UNO));
    let mut l = o
        .leer_fijado(&v.clave, &v.version, "", Some("bytes=0-3"))
        .expect("rango");
    let mut b = Vec::new();
    l.lector.read_to_end(&mut b).unwrap();
    assert_eq!((l.estado, b.as_slice()), (206, &b"%PDF"[..]));

    // un ETag que ya no es: cambiado, nunca otros bytes
    match o.leer_fijado(&v.clave, "etag:0xdead", "", None) {
        Err(Rechazo::Cambiado(_)) => {}
        Err(e) => panic!("otro rechazo: {e:?}"),
        Ok(l) => panic!("dio {} con un ETag que no es", l.estado),
    }
    // ⭐ la guarda de O3·0: un `versionid` que Azurite ignora, con un ETag que
    //   no casa, tampoco da los bytes vigentes
    match o.leer_fijado(&v.clave, "2020-01-01T00:00:00.0000000Z", "\"0xdead\"", None) {
        Err(Rechazo::Cambiado(_)) => {}
        Err(e) => panic!("otro rechazo: {e:?}"),
        Ok(l) => panic!("dio {} de una versión que no está", l.estado),
    }
    let (mut r, ab) = o.abrir_version(&v.clave, &v.version).expect("abrir");
    let mut b = Vec::new();
    r.read_to_end(&mut b).unwrap();
    assert_eq!((b.as_slice(), ab.huella), (UNO, Some(c.texto())));

    // la URL: de un blob fijado por ETag, ninguna (D-O1)…
    assert_eq!(
        o.firmar(&v.clave, &v.version, "application/pdf", "inline", 60)
            .unwrap(),
        None
    );
    // …y la SAS de delegación del vigente se baja, con su tipo y su
    // disposición; manipulada, 403
    let u = az
        .firmar(&v.clave, None, "application/pdf", "inline", 60)
        .expect("la SAS");
    assert!(u.contains("sr=b&") && u.contains("sig="));
    let agente = ore_gcp::cliente().unwrap();
    let x = agente.get(&u).call().expect("bajar por la SAS");
    assert_eq!(x.header("content-disposition"), Some("inline"));
    assert_eq!(x.header("content-type"), Some("application/pdf"));
    let mut b = Vec::new();
    x.into_reader().read_to_end(&mut b).unwrap();
    assert_eq!(b, UNO);
    let mala = u.replace("sp=r", "sp=rw");
    match agente.get(&mala).call() {
        Err(ureq::Error::Status(403, _)) => {}
        otro => panic!("una SAS manipulada: {:?}", otro.map(|r| r.status())),
    }

    // un contenedor que no está
    let otro = ore_azure::Azure::de_url(&url.replace("/cubo/", "/otro/")).unwrap();
    let e = Origen::listar(&otro, "").unwrap_err();
    assert!(e.contains("ContainerNotFound"), "{e}");
    eprintln!(
        "azure: listar, ETag, md5 (y sin él), entero, rango, 412 → cambiado, la guarda, la SAS"
    );
}
