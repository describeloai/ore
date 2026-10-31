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

/// Los dos tests comparten la carpeta del laboratorio, y uno de ellos escribe en
/// ella: de uno en uno.
static LABORATORIO: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
    let _uno = LABORATORIO.lock().unwrap_or_else(|e| e.into_inner());
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

// ─── Los cambios, en vivo (paso 4) ──────────────────────────────────────────
//
// Escribe en el Drive del laboratorio COMO UN USUARIO (la cuenta de servicio
// sólo lee): `ORE_LAB_GDRIVE_ESCRIBE` es el proyecto de cuota y `ORE_GCP_TOKEN`
// tiene que traer el alcance `drive` (`gcloud auth login --enable-gdrive-access`).
// Crea y borra lo suyo, bajo la carpeta del laboratorio y en el Drive del usuario.

struct Usuario {
    token: String,
    proyecto: String,
}

impl Usuario {
    fn pedir(&self, metodo: &str, url: &str, cuerpo: Option<(&str, &[u8])>) -> String {
        let r = ore_gcp::cliente()
            .unwrap()
            .request(metodo, url)
            .set("authorization", &format!("Bearer {}", self.token))
            .set("x-goog-user-project", &self.proyecto);
        let x = match cuerpo {
            Some((tipo, b)) => r.set("content-type", tipo).send_bytes(b),
            None => r.call(),
        };
        x.unwrap_or_else(|e| panic!("{metodo} {url}: {e}"))
            .into_string()
            .unwrap()
    }

    fn id(texto: &str) -> String {
        let (_, resto) = texto.split_once("\"id\"").expect("el id");
        resto.split('"').nth(1).expect("el id").to_string()
    }

    fn carpeta(&self, nombre: &str, padre: Option<&str>) -> String {
        let p = padre.map_or(String::new(), |p| format!(",\"parents\":[\"{p}\"]"));
        let cuerpo = format!(
            "{{\"name\":\"{nombre}\",\"mimeType\":\"application/vnd.google-apps.folder\"{p}}}"
        );
        Self::id(&self.pedir(
            "POST",
            "https://www.googleapis.com/drive/v3/files?fields=id",
            Some(("application/json", cuerpo.as_bytes())),
        ))
    }

    fn fichero(&self, nombre: &str, padre: &str, contenido: &str) -> String {
        let meta = format!("{{\"name\":\"{nombre}\",\"parents\":[\"{padre}\"]}}");
        let cuerpo = format!(
            "--B\r\nContent-Type: application/json\r\n\r\n{meta}\r\n--B\r\nContent-Type: text/csv\r\n\r\n{contenido}\r\n--B--"
        );
        Self::id(&self.pedir(
            "POST",
            "https://www.googleapis.com/upload/drive/v3/files?uploadType=multipart&fields=id",
            Some(("multipart/related; boundary=B", cuerpo.as_bytes())),
        ))
    }

    fn parchear(&self, id: &str, json: &str, mas: &str) {
        self.pedir(
            "PATCH",
            &format!("https://www.googleapis.com/drive/v3/files/{id}?fields=id{mas}"),
            Some(("application/json", json.as_bytes())),
        );
    }

    fn borrar(&self, id: &str) {
        let _ = ore_gcp::cliente()
            .unwrap()
            .delete(&format!("https://www.googleapis.com/drive/v3/files/{id}"))
            .set("authorization", &format!("Bearer {}", self.token))
            .set("x-goog-user-project", &self.proyecto)
            .call();
    }
}

/// Borra lo que el test creó aunque falle.
struct Limpieza<'a>(&'a Usuario, Vec<String>);

impl Drop for Limpieza<'_> {
    fn drop(&mut self) {
        for id in &self.1 {
            self.0.borrar(id);
        }
    }
}

/// Mira hasta que `cond` se cumple: el *feed* de Drive tarda unos segundos.
fn esperar(g: &Gdrive, que: &str, cond: impl Fn(&[ore_gdrive::Item]) -> bool) {
    for _ in 0..40 {
        let (items, _) = g.recorrer("").unwrap();
        if cond(&items) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_secs(3));
    }
    let (items, _) = g.recorrer("").unwrap();
    let r: Vec<_> = items.iter().map(|i| i.ruta.as_str()).collect();
    panic!("el feed de cambios no mostró «{que}» en 2 minutos: {r:?}");
}

#[test]
fn los_cambios_de_drive_se_siguen_sin_volver_a_recorrer() {
    let (Ok(url), Ok(proyecto), Ok(token)) = (
        std::env::var("ORE_LAB_GDRIVE"),
        std::env::var("ORE_LAB_GDRIVE_ESCRIBE"),
        std::env::var("ORE_GCP_TOKEN"),
    ) else {
        eprintln!("sin ORE_LAB_GDRIVE_ESCRIBE no se escribe en Drive: nada que probar");
        return;
    };
    let raiz = url.trim_start_matches("gdrive://").to_string();
    let _uno = LABORATORIO.lock().unwrap_or_else(|e| e.into_inner());
    let u = Usuario { token, proyecto };
    let g = Gdrive::de_url(&url).unwrap();

    // El primer recorrido arma el árbol; el token queda guardado.
    let (antes, _) = g.recorrer("").unwrap();
    let t0 = g.token_de_seguimiento().expect("hay índice");
    let n0 = antes.len();

    let mut limpieza = Limpieza(&u, vec![]);
    let carpeta = u.carpeta("x0-cambios", Some(&raiz));
    limpieza.1.push(carpeta.clone());

    // Un fichero nuevo en una carpeta nueva.
    let f = u.fichero("n1.csv", &carpeta, "k\n1\n");
    esperar(&g, "x0-cambios/n1.csv", |i| {
        i.iter().any(|x| x.ruta == "x0-cambios/n1.csv")
    });
    assert_ne!(g.token_de_seguimiento().unwrap(), t0, "el token avanzó");
    let n1 = g
        .recorrer("")
        .unwrap()
        .0
        .into_iter()
        .find(|x| x.ruta == "x0-cambios/n1.csv")
        .unwrap();

    // Su contenido cambia: otra revisión, otro validador.
    u.pedir(
        "PATCH",
        &format!("https://www.googleapis.com/upload/drive/v3/files/{f}?uploadType=media&fields=id"),
        Some(("text/csv", b"k\n1\n2\n")),
    );
    esperar(&g, "otra revisión de n1.csv", |i| {
        i.iter()
            .any(|x| x.ruta == "x0-cambios/n1.csv" && x.validador() != n1.validador())
    });

    // Se renombra.
    u.parchear(&f, "{\"name\":\"n2.csv\"}", "");
    esperar(&g, "x0-cambios/n2.csv", |i| {
        i.iter().any(|x| x.ruta == "x0-cambios/n2.csv")
            && !i.iter().any(|x| x.ruta == "x0-cambios/n1.csv")
    });

    // Una carpeta con un fichero dentro, creada FUERA y movida después: el feed
    // sólo dice que la carpeta entró, y su fichero se encuentra recorriéndola.
    let fuera = u.carpeta("x0-entra", None);
    limpieza.1.push(fuera.clone());
    u.fichero("dentro.csv", &fuera, "z\n9\n");
    u.parchear(
        &fuera,
        "{}",
        &format!("&addParents={raiz}&removeParents=root"),
    );
    esperar(&g, "x0-entra/dentro.csv", |i| {
        i.iter().any(|x| x.ruta == "x0-entra/dentro.csv")
    });

    // A la papelera.
    u.parchear(&f, "{\"trashed\":true}", "");
    esperar(&g, "n2.csv fuera", |i| {
        !i.iter().any(|x| x.ruta.starts_with("x0-cambios/n"))
    });

    // Y la carpeta entera, con lo que colgaba de ella.
    u.parchear(&fuera, "{\"trashed\":true}", "");
    esperar(&g, "x0-entra fuera", |i| {
        !i.iter().any(|x| x.ruta.starts_with("x0-entra/"))
    });
    assert_eq!(g.recorrer("").unwrap().0.len(), n0, "vuelve a lo de antes");

    // Una carpeta que no está sigue siendo un error, también con el índice.
    let e = g.recorrer("no/esta/").unwrap_err();
    assert!(
        e.motivo().contains("is not in the Drive folder"),
        "{}",
        e.motivo()
    );
}
