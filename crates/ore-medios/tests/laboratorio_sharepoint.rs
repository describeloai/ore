//! **Una virtual sobre una biblioteca de SharePoint** (ADR 0061 O5·4): el
//! camino de `ore-medios` —`origenes::de(fuente)`, con el cliente guardado
//! entre peticiones, y el `Origen` de `ore-graph`— contra el Graph de mentira
//! (`pruebas-de-fuego/graph-de-mentira.py`; no hay emulador de Graph). Lo
//! arranca `pruebas-de-fuego/o5-sharepoint.sh`, que da la URL en
//! `ORE_LAB_SHAREPOINT` (con su `endpoint`) y el token en `ORE_GRAPH_TOKEN`;
//! sin ella, no hay nada que probar y el test lo dice.
//!
//! El test siembra `medios/x.pdf` (dos versiones) y la fija en la segunda: el
//! rango (`206`), por el final, entero con su `quickXorHash`; **una versión
//! nueva no cambia lo fijado** —se sigue sirviendo la segunda, ya por su id—;
//! recortada, `media/cambiado`; y sin URL firmada (D-O5).

use ore_objetos::Rechazo;
use ore_objetos::huella::{Calculo, QuickXor};
use std::io::{Read, Write};

const DOS: &[u8] = b"%PDF-1.4\n% UNO\n%%EOF\n";

/// Una petición a la siembra del Graph de mentira (sin token: sólo escucha en
/// 127.0.0.1), con lo justo de HTTP/1.1.
fn sembrar(base: &str, metodo: &str, que: &str, mas: &str, cuerpo: &[u8]) {
    let hostport = base.trim_start_matches("http://");
    let mut s = std::net::TcpStream::connect(hostport).expect("el Graph de mentira");
    let camino = format!(
        "/_mentira{que}?sitio=sites/Finanzas&biblioteca=Activos%20del%20sitio&ruta=medios/x.pdf{mas}"
    );
    write!(
        s,
        "{metodo} {camino} HTTP/1.1\r\nHost: {hostport}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        cuerpo.len()
    )
    .unwrap();
    s.write_all(cuerpo).unwrap();
    let mut r = String::new();
    s.read_to_string(&mut r).unwrap();
    assert!(
        r.starts_with("HTTP/1.1 2") || r.starts_with("HTTP/1.0 2"),
        "{r}"
    );
}

#[test]
fn una_virtual_sobre_una_biblioteca_de_sharepoint() {
    let Ok(url) = std::env::var("ORE_LAB_SHAREPOINT") else {
        eprintln!("sin ORE_LAB_SHAREPOINT no hay laboratorio: nada que probar (o5-sharepoint.sh)");
        return;
    };
    let base = url
        .split("endpoint=")
        .nth(1)
        .expect("el endpoint")
        .to_string();
    sembrar(&base, "PUT", "", "", b"%PDF-1.4\n% uno\n%%EOF\n");
    sembrar(&base, "PUT", "", "", DOS);

    let o = ore_medios::origenes::de(&url).unwrap_or_else(|r| panic!("{}", r.cuerpo.jcs()));
    let v = o
        .listar_versiones("medios/")
        .expect("versiones")
        .into_iter()
        .find(|v| v.clave == "medios/x.pdf")
        .expect("medios/x.pdf no está");
    assert!(v.version.ends_with("@2.0"), "{v:?}");

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
    let (estado, b) = leer(None);
    assert_eq!((estado, b.as_slice()), (200, DOS));
    let mut q = QuickXor::default();
    q.sumar(&b);
    assert_eq!(o.huella_de(&v.clave, &v.version).unwrap(), Some(q.texto()));

    // ⭐ una versión nueva: lo fijado sigue siendo la segunda, ya por su id
    sembrar(
        &base,
        "PUT",
        "",
        "",
        b"%PDF-1.4\n% la tercera, otra\n%%EOF\n",
    );
    let (_, b) = leer(None);
    assert_eq!(b, DOS, "lo fijado no cambia con una versión nueva");
    assert_eq!(leer(Some("bytes=-6")).1, b"%%EOF\n");

    // la misma fuente, otra petición: el mismo cliente, y lee igual
    let o2 = ore_medios::origenes::de(&url).unwrap_or_else(|r| panic!("{}", r.cuerpo.jcs()));
    assert_eq!(o2.listar("medios/").unwrap().len(), 1);

    // sin URL firmada (D-O5): la de SharePoint es al portador y no se fija
    assert_eq!(
        o.firmar(&v.clave, &v.version, "application/pdf", "inline", 60)
            .unwrap(),
        None
    );

    // recortada por la biblioteca: cambiado, nunca otros bytes
    sembrar(&base, "DELETE", "/version", "&version=2.0", b"");
    match o.leer_fijado(&v.clave, &v.version, &v.etag, None) {
        Err(Rechazo::Cambiado(_)) => {}
        Err(e) => panic!("otro rechazo: {e:?}"),
        Ok(l) => panic!("dio {} con una versión recortada", l.estado),
    }
    eprintln!(
        "sharepoint por ore-medios: rango, por el final, entero con quickxor, una versión nueva no cambia lo fijado, sin URL, recortada → cambiado"
    );
}
