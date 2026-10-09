//! **Una biblioteca de SharePoint como origen, contra el Graph de mentira**
//! (ADR 0061 O5·1, `pruebas-de-fuego/graph-de-mentira.py`, arrancado con
//! `PAGINA=2` y `CADA_429=7`): no hay emulador de Graph, y el de mentira está
//! escrito de la documentación. La URL llega en `ORE_LAB_SHAREPOINT` (con su
//! `endpoint`), el token en `ORE_GRAPH_TOKEN`; el test siembra la muestra él
//! mismo. Sin la URL no hay laboratorio, y el test lo dice.

use ore_graph::{Clase, Graph};
use ore_objetos::huella::{Calculo, QuickXor};
use ore_objetos::{Origen, Rechazo};
use std::io::Read;

const UNO: &[u8] = b"%PDF-1.4\n% uno\n%%EOF\n";
const OTRO: &[u8] = b"%PDF-1.4\n% UNO\n%%EOF\n";

fn huella(b: &[u8]) -> String {
    let mut q = QuickXor::default();
    q.sumar(b);
    q.texto()
}

struct Mentira(String);

impl Mentira {
    fn url(&self, que: &str, ruta: &str, mas: &str) -> String {
        format!(
            "{}/_mentira{que}?sitio=sites/Finanzas&biblioteca=Documentos&ruta={}{mas}",
            self.0,
            ruta.replace(' ', "%20")
        )
    }
    fn subir(&self, ruta: &str, datos: &[u8]) {
        ureq::put(&self.url("", ruta, ""))
            .send_bytes(datos)
            .unwrap();
    }
    fn subir_como(&self, ruta: &str, tipo: &str) {
        ureq::put(&self.url("", ruta, &format!("&tipo={tipo}")))
            .send_bytes(b"x")
            .unwrap();
    }
    fn tocar(&self, ruta: &str) {
        ureq::post(&self.url("/tocar", ruta, "")).call().unwrap();
    }
    fn recortar(&self, ruta: &str, v: &str) {
        ureq::delete(&self.url("/version", ruta, &format!("&version={v}")))
            .call()
            .unwrap();
    }
    fn cuentas(&self) -> String {
        ureq::get(&format!("{}/_mentira/cuentas", self.0))
            .call()
            .unwrap()
            .into_string()
            .unwrap()
    }
}

#[test]
fn una_biblioteca_de_sharepoint_como_origen() {
    let Ok(url) = std::env::var("ORE_LAB_SHAREPOINT") else {
        eprintln!(
            "sin ORE_LAB_SHAREPOINT no hay laboratorio: nada que probar (graph-de-mentira.py)"
        );
        return;
    };
    let base = url
        .split("endpoint=")
        .nth(1)
        .expect("el endpoint")
        .to_string();
    let m = Mentira(base);
    m.subir("docs/a.pdf", UNO);
    m.subir("docs/a.pdf", OTRO);
    m.subir("docs/Nueva carpeta/b.pdf", UNO);
    for i in 0..5 {
        m.subir(&format!("docs/relleno-{i}.txt"), b"relleno");
    }
    m.subir_como("docs/Cuaderno", "package");
    m.subir_como("docs/Atajo", "remote");
    let grande: Vec<u8> = (0..(3 << 20)).map(|i| (i % 251) as u8).collect();
    m.subir("grande.bin", &grande);
    m.subir("otra/c.pdf", UNO);

    let g = Graph::de_url(&url).expect("la URL");
    let o: &dyn Origen = &g;

    // listar: en páginas de 2, por carpetas; sin cuadernos ni accesos directos
    let crudo = Graph::listar(&g, "docs/").unwrap();
    assert!(
        crudo
            .iter()
            .any(|i| i.ruta == "docs/Cuaderno" && i.clase == Clase::Cuaderno)
    );
    assert!(
        crudo
            .iter()
            .any(|i| i.ruta == "docs/Atajo" && i.clase == Clase::Acceso)
    );
    let objs = o.listar("docs/").unwrap();
    let claves: Vec<&str> = objs.iter().map(|x| x.clave.as_str()).collect();
    assert_eq!(claves.len(), 7, "{claves:?}");
    assert!(claves.contains(&"docs/a.pdf") && claves.contains(&"docs/Nueva carpeta/b.pdf"));
    assert!(
        !claves
            .iter()
            .any(|c| c.contains("Cuaderno") || c.contains("Atajo"))
    );
    assert_eq!(
        o.listar("docs/a").unwrap().len(),
        1,
        "por prefijo, no sólo por carpeta"
    );
    assert_eq!(o.listar("").unwrap().len(), 9);
    assert!(o.listar("no-esta/").unwrap().is_empty());

    // versiones: la actual, `<id>@2.0`, con su cTag
    let vs = o.listar_versiones("docs/").unwrap();
    let a = vs
        .iter()
        .find(|v| v.clave == "docs/a.pdf")
        .expect("a.pdf")
        .clone();
    assert!(
        a.version.ends_with("@2.0") && a.actual && a.etag.starts_with("\"c:"),
        "{a:?}"
    );
    let id = a.version.split('@').next().unwrap().to_string();
    let vieja = format!("{id}@1.0");

    // la actual, entera, con su huella; la vieja, por su id y sin huella
    let (mut r, ab) = o.abrir_version(&a.clave, &a.version).unwrap();
    let mut b = Vec::new();
    r.read_to_end(&mut b).unwrap();
    assert_eq!((b.as_slice(), ab.tamano), (OTRO, Some(OTRO.len() as u64)));
    assert_eq!(ab.huella, Some(huella(OTRO)));
    assert_eq!(
        o.huella_de(&a.clave, &a.version).unwrap(),
        Some(huella(OTRO))
    );
    let (mut r, ab) = o.abrir_version(&a.clave, &vieja).unwrap();
    let mut b = Vec::new();
    r.read_to_end(&mut b).unwrap();
    assert_eq!((b.as_slice(), ab.huella), (UNO, None));
    assert_eq!(o.huella_de(&a.clave, &vieja).unwrap(), None);

    // rangos, también por el final (el pie de un Parquet)
    assert_eq!(o.rango(&a.clave, "0-3").unwrap(), b"%PDF");
    assert_eq!(o.rango_de(&a.clave, "-6", &a.etag).unwrap(), b"%%EOF\n");
    assert!(o.rango_de(&a.clave, "0-3", "\"c:{otro},9\"").is_err());

    // fijado, como lo sirve ore-medios: la actual por su cTag, la vieja por su id
    let mut l = o
        .leer_fijado(&a.clave, &a.version, &a.etag, Some("bytes=-6"))
        .unwrap();
    let mut b = Vec::new();
    l.lector.read_to_end(&mut b).unwrap();
    assert_eq!((l.estado, b.as_slice()), (206, &b"%%EOF\n"[..]));
    let mut l = o.leer_fijado(&a.clave, &vieja, "", None).unwrap();
    let mut b = Vec::new();
    l.lector.read_to_end(&mut b).unwrap();
    assert_eq!(b, UNO);
    let mut l = o
        .leer_fijado(&a.clave, &vieja, "", Some("bytes=-6"))
        .unwrap();
    let mut b = Vec::new();
    l.lector.read_to_end(&mut b).unwrap();
    assert_eq!(b, b"%%EOF\n");
    assert!(matches!(
        o.leer_fijado(&a.clave, &a.version, &a.etag, Some("bytes=900-999")),
        Err(Rechazo::Rango(_))
    ));

    // tocar sólo los metadatos cambia el eTag y no el cTag: sigue siendo ella
    m.tocar("docs/a.pdf");
    let mut r = o
        .abrir(&a.clave, &a.etag)
        .expect("los metadatos no son el contenido");
    let mut b = Vec::new();
    r.read_to_end(&mut b).unwrap();
    assert_eq!(b, OTRO);

    // ⭐ una versión nueva mientras se lee la actual: la lectura falla
    let gv = o
        .listar_versiones("grande.bin")
        .unwrap()
        .pop()
        .expect("grande.bin");
    let (mut r, _) = o.abrir_version(&gv.clave, &gv.version).unwrap();
    let mut primero = vec![0u8; 1 << 20];
    r.read_exact(&mut primero).unwrap();
    m.subir("grande.bin", &grande);
    let mut resto = Vec::new();
    let e = r
        .read_to_end(&mut resto)
        .expect_err("cambió mientras se leía");
    assert!(e.to_string().contains("cambió"), "{e}");
    // y la que se fijó antes sigue leyéndose, ya como vieja (por su id)
    let (mut r, _) = o.abrir_version(&gv.clave, &gv.version).unwrap();
    let mut b = Vec::new();
    r.read_to_end(&mut b).unwrap();
    assert_eq!(b.len(), grande.len());

    // una versión que la biblioteca recortó: media/cambiado
    m.recortar("docs/a.pdf", "1.0");
    match o.leer_fijado(&a.clave, &vieja, "", None) {
        Err(Rechazo::Cambiado(m)) => assert!(m.contains("1.0"), "{m}"),
        otro => panic!("una versión recortada: {:?}", otro.err()),
    }
    assert!(o.abrir_version(&a.clave, &vieja).is_err());

    // un sitio sin la concesión de Sites.Selected, y una biblioteca que no está
    let rrhh = Graph::de_url(&url.replace("sites/Finanzas", "sites/RRHH")).unwrap();
    let e = o_listar(&rrhh);
    assert!(e.contains("403 accessDenied"), "{e}");
    let otra = Graph::de_url(&url.replace("/Documentos", "/Facturas")).unwrap();
    let e = o_listar(&otra);
    assert!(
        e.contains("no hay una biblioteca `Facturas`") && e.contains("`Documentos`"),
        "{e}"
    );

    // el ritmo: hubo 429 y se esperaron; el token nunca llegó a la descarga
    let (peticiones, _, esperas) = ore_graph::contadores();
    assert!(esperas > 0, "con CADA_429 el Graph de mentira pide esperar");
    let c = m.cuentas();
    assert!(c.contains("\"con_token\": 0"), "{c}");
    eprintln!(
        "sharepoint: listar (páginas, cuadernos, accesos), versiones, actual vigilada y vieja por id, huella quickxor, rangos, fijado, eTag≠cTag, cambio a medias, recortada, 403, biblioteca, {esperas} esperas por 429 en {peticiones} peticiones, sin token en la descarga"
    );
}

fn o_listar(g: &Graph) -> String {
    let o: &dyn Origen = g;
    o.listar("").expect_err("no debía listar")
}
