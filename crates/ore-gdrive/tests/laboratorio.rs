//! **Una carpeta de Drive como origen, contra el Drive de verdad** (ADR 0082
//! paso 1): no hay emulador de Drive, así que el laboratorio es una carpeta
//! sembrada por el X0 (`x0-ore`) y compartida con la cuenta que lee. La URL
//! llega en `ORE_LAB_GDRIVE` (`gdrive://<id de la carpeta>`), la cuenta en
//! `ORE_GDRIVE_CUENTA` y el token con el que se suplanta en `ORE_GCP_TOKEN`.
//! Sin la URL no hay laboratorio, y el test lo dice.
//!
//! La carpeta tiene: dos `a.csv` en la raíz (uno con 5 revisiones), `sub/c.csv`,
//! un Doc y una Hoja nativos, un acceso directo y un fichero en la papelera.

use ore_gdrive::{Clase, Gdrive};
use ore_objetos::huella::{Calculo, Md5};
use ore_objetos::{Origen, Rechazo};
use std::io::Read;

fn md5(b: &[u8]) -> String {
    let mut c = Md5::default();
    c.sumar(b);
    c.texto()
}

#[test]
fn una_carpeta_de_drive_como_origen() {
    let Ok(url) = std::env::var("ORE_LAB_GDRIVE") else {
        eprintln!("sin ORE_LAB_GDRIVE no hay laboratorio: nada que probar (ADR 0082)");
        return;
    };
    let g = Gdrive::de_url(&url).unwrap();

    // El árbol: los nativos y los accesos directos se distinguen, la papelera no
    // se ve, y los dos `a.csv` llevan su ID.
    let (items, saltados) = g.recorrer("").unwrap();
    let rutas: Vec<&str> = items.iter().map(|i| i.ruta.as_str()).collect();
    assert_eq!(rutas.len(), 5, "{rutas:?}");
    assert_eq!(rutas.iter().filter(|r| r.starts_with("a~")).count(), 2);
    assert!(rutas.contains(&"sub/c.csv"));
    assert!(rutas.contains(&"doc nativo.txt") && rutas.contains(&"hoja nativa.csv"));
    assert_eq!(saltados, 1, "el acceso directo");
    assert!(!rutas.iter().any(|r| r.contains("borrado")), "la papelera");
    let nativos = items
        .iter()
        .filter(|i| matches!(i.clase, Clase::Nativo(_)))
        .count();
    assert_eq!(nativos, 2);

    // El rasgo: sólo los binarios, con el validador `<id>@<revisión>`.
    let objetos = Origen::listar(&g, "").unwrap();
    assert_eq!(objetos.len(), 5, "tres binarios y dos nativos");
    let c = objetos.iter().find(|o| o.clave == "sub/c.csv").unwrap();
    assert_eq!((c.tamano, c.etag.contains('@')), (4, true));
    // Con prefijo: sólo lo que cuelga de él.
    assert_eq!(Origen::listar(&g, "sub/").unwrap().len(), 1);

    // Leer fijado: los bytes de esa revisión, y su md5 es el que Drive dijo.
    let mut b = Vec::new();
    Origen::abrir(&g, &c.clave, &c.etag)
        .unwrap()
        .read_to_end(&mut b)
        .unwrap();
    assert_eq!(b, b"k\nv\n");
    let h = Origen::huella_de(&g, &c.clave, &c.etag).unwrap();
    assert_eq!(
        h.as_deref(),
        Some(md5(&b).as_str()),
        "el md5 de Drive = el de los bytes"
    );

    // Rango, y por el final (X0 B3).
    assert_eq!(Origen::rango(&g, "sub/c.csv", "0-0").unwrap(), b"k");
    assert_eq!(Origen::rango(&g, "sub/c.csv", "-2").unwrap(), b"v\n");
    assert_eq!(
        Origen::rango_de(&g, &c.clave, "0-1", &c.etag).unwrap(),
        b"k\n"
    );

    // Las versiones: una por fichero, la vigente.
    let vs = Origen::listar_versiones(&g, "sub/").unwrap();
    assert_eq!(vs.len(), 1);
    assert!(vs[0].actual && vs[0].version == c.etag);
    let (mut l, a) = Origen::abrir_version(&g, &c.clave, &c.etag).unwrap();
    let mut b2 = Vec::new();
    l.read_to_end(&mut b2).unwrap();
    assert_eq!(
        (b2, a.tamano, a.huella.as_deref()),
        (b, Some(4), h.as_deref())
    );

    // Una revisión que ya no es la actual (aquí, inventada) no da otros bytes.
    let (id, _) = c.etag.split_once('@').unwrap();
    let vieja = format!("{id}@revision-que-no-esta");
    assert!(matches!(
        Origen::leer_fijado(&g, &c.clave, &vieja, "", None),
        Err(Rechazo::Cambiado(_))
    ));
    assert!(Origen::abrir(&g, &c.clave, &vieja).is_err());
    let e = Origen::huella_de(&g, &c.clave, &vieja).unwrap_err();
    assert!(e.contains("no longer exists"), "{e}");

    // Un nativo se exporta (X0 D1): el Doc a `txt` sin el BOM que Drive pone, la
    // Hoja a `csv` (con CRLF y sin salto final). Drive ignora el rango de un
    // export (paso 2): se corta de lo exportado.
    let doc = objetos
        .iter()
        .find(|o| o.clave == "doc nativo.txt")
        .unwrap();
    let hoja = objetos
        .iter()
        .find(|o| o.clave == "hoja nativa.csv")
        .unwrap();
    assert!(
        doc.etag.contains("#v") && !doc.etag.contains('@'),
        "{}",
        doc.etag
    );
    assert_eq!(Origen::rango(&g, "doc nativo.txt", "0-3").unwrap(), b"Hola");
    assert_eq!(Origen::rango(&g, "doc nativo.txt", "-5").unwrap(), b"Drive");
    let mut csv = Vec::new();
    Origen::abrir(&g, &hoja.clave, &hoja.etag)
        .unwrap()
        .read_to_end(&mut csv)
        .unwrap();
    assert_eq!(csv, b"a,b\r\n1,2\r\n3,4");
    let h = Origen::huella_de(&g, &hoja.clave, &hoja.etag).unwrap();
    assert_eq!(
        h.as_deref(),
        Some(md5(&csv).as_str()),
        "la huella es la de lo exportado"
    );
    let l = Origen::leer_fijado(&g, &hoja.clave, &hoja.etag, "", Some("bytes=0-2")).unwrap();
    assert_eq!(l.estado, 206);
    assert_eq!(l.cabecera("content-range"), Some("bytes 0-2/13"));
    assert_eq!(l.cabecera("content-type"), Some("text/csv"));
    let mut t = Vec::new();
    {
        let mut r = l.lector;
        r.read_to_end(&mut t).unwrap();
    }
    assert_eq!(t, b"a,b");
    assert!(matches!(
        Origen::leer_fijado(&g, &hoja.clave, &hoja.etag, "", Some("bytes=99-100")),
        Err(Rechazo::Rango(_))
    ));
    // Una versión que ya no es la del nativo no da otros bytes.
    let vieja = hoja.etag.replace("#v", "#v0");
    assert!(matches!(
        Origen::leer_fijado(&g, &hoja.clave, &vieja, "", None),
        Err(Rechazo::Cambiado(_))
    ));
    assert!(Origen::abrir(&g, &hoja.clave, &vieja).is_err());
    // Las versiones de la carpeta: una por fichero, también los nativos.
    assert_eq!(Origen::listar_versiones(&g, "").unwrap().len(), 5);

    // Una carpeta que no está es un 404, no un listado vacío.
    let e = Origen::listar(&g, "no/esta/").unwrap_err();
    assert!(e.contains("is not in the Drive folder"), "{e}");
}
