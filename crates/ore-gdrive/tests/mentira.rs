//! **Una carpeta de Drive como origen, contra el Drive de mentira** (ADR 0082
//! paso 5, `pruebas-de-fuego/drive-de-mentira.py`): lo mismo que
//! `laboratorio.rs` pero sin cuenta de Google, sin red y con el servidor
//! cambiándose a voluntad, que es lo que permite probar en el CI la lectura
//! fijada, los nativos, el *feed* de cambios y su respaldo.
//!
//! El servidor llega en `ORE_LAB_GDRIVE_MENTIRA` (`http://127.0.0.1:8792`); y
//! `ORE_GCP_TOKEN`, `ORE_GDRIVE_CUENTA`, `ORE_GCP_IAM` y `ORE_GDRIVE_LABORATORIO`
//! los pone el script (la autosuplantación contra el IAM de mentira). Sin la
//! variable no hay laboratorio, y el test lo dice.
//!
//! Cada test siembra bajo su propia carpeta (`t1/`, `t2/`) y su `Gdrive` tiene
//! esa carpeta de prefijo; corren de uno en uno porque `recorridos()` es global.

use ore_gdrive::{Gdrive, recorridos};
use ore_objetos::huella::{Calculo, Md5};
use ore_objetos::{Origen, Rechazo};
use std::io::Read;

static UNO_A_UNO: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn md5(b: &[u8]) -> String {
    let mut c = Md5::default();
    c.sumar(b);
    c.texto()
}

struct Mentira(String);

impl Mentira {
    fn de_entorno() -> Option<Mentira> {
        std::env::var("ORE_LAB_GDRIVE_MENTIRA").ok().map(Mentira)
    }

    fn pedir(&self, metodo: &str, que: &str, mas: &str, cuerpo: &[u8]) {
        let url = format!("{}/_mentira{que}?{mas}", self.0);
        ore_gcp::cliente()
            .unwrap()
            .request(metodo, &url)
            .send_bytes(cuerpo)
            .unwrap_or_else(|e| panic!("{metodo} {url}: {e}"));
    }

    fn subir(&self, ruta: &str, datos: &str) {
        self.pedir("PUT", "", &format!("ruta={ruta}"), datos.as_bytes());
    }

    fn subir_otra_vez(&self, ruta: &str, datos: &str) {
        self.pedir("PUT", "", &format!("ruta={ruta}&dup=1"), datos.as_bytes());
    }

    fn nativo(&self, ruta: &str, clase: &str, texto: &str) {
        self.pedir(
            "POST",
            "/nativo",
            &format!("ruta={ruta}&clase={clase}"),
            texto.as_bytes(),
        );
    }

    fn accion(&self, que: &str, mas: &str) {
        self.pedir("POST", que, mas, b"");
    }

    fn id(&self, ruta: &str) -> String {
        let t = ore_gcp::cliente()
            .unwrap()
            .get(&format!("{}/_mentira/id?ruta={ruta}", self.0))
            .call()
            .unwrap()
            .into_string()
            .unwrap();
        t.split('"').nth(3).expect("el id").to_string()
    }

    fn cuentas(&self) -> String {
        ore_gcp::cliente()
            .unwrap()
            .get(&format!("{}/_mentira/cuentas", self.0))
            .call()
            .unwrap()
            .into_string()
            .unwrap()
    }

    fn gdrive(&self, prefijo: &str) -> Gdrive {
        Gdrive::de_url(&format!("gdrive://RAIZ/{prefijo}/?endpoint={}", self.0)).unwrap()
    }
}

fn claves(g: &Gdrive, prefijo: &str) -> Vec<String> {
    Origen::listar(g, prefijo)
        .unwrap()
        .into_iter()
        .map(|o| o.clave)
        .collect()
}

fn leer(g: &Gdrive, clave: &str, etag: &str) -> Vec<u8> {
    let mut b = Vec::new();
    Origen::abrir(g, clave, etag)
        .unwrap()
        .read_to_end(&mut b)
        .unwrap();
    b
}

#[test]
fn la_lectura_fijada_y_los_nativos() {
    let Some(m) = Mentira::de_entorno() else {
        eprintln!("sin ORE_LAB_GDRIVE_MENTIRA no hay laboratorio: nada que probar");
        return;
    };
    let _uno = UNO_A_UNO.lock().unwrap_or_else(|e| e.into_inner());
    m.subir("t1/a.csv", "x,y\n1,2\n");
    m.subir_otra_vez("t1/a.csv", "x,y\n9,9\n");
    m.subir("t1/sub/c.csv", "k\nv\n");
    m.nativo("t1/doc", "doc", "Hola Drive\n");
    m.nativo("t1/hoja", "hoja", "a,b\n1,2\n3,4\n");
    m.accion(
        "/atajo",
        &format!("ruta=t1/atajo&destino={}", m.id("t1/sub/c.csv")),
    );
    let g = m.gdrive("t1");

    // El árbol: los dos `a.csv` llevan su ID, el acceso directo no entra.
    let cs = claves(&g, "t1/");
    assert_eq!(cs.len(), 5, "{cs:?}");
    assert_eq!(cs.iter().filter(|c| c.starts_with("t1/a~")).count(), 2);
    for c in ["t1/sub/c.csv", "t1/doc.txt", "t1/hoja.csv"] {
        assert!(cs.contains(&c.to_string()), "{c}: {cs:?}");
    }

    // Un binario: sus bytes, el md5 de Drive = el suyo, el rango por el final.
    let objetos = Origen::listar(&g, "t1/").unwrap();
    let c = objetos.iter().find(|o| o.clave == "t1/sub/c.csv").unwrap();
    assert_eq!(leer(&g, &c.clave, &c.etag), b"k\nv\n");
    let h = Origen::huella_de(&g, &c.clave, &c.etag).unwrap();
    assert_eq!(h.as_deref(), Some(md5(b"k\nv\n").as_str()));
    assert_eq!(Origen::rango(&g, &c.clave, "-2").unwrap(), b"v\n");
    assert_eq!(
        Origen::rango_de(&g, &c.clave, "0-0", &c.etag).unwrap(),
        b"k"
    );

    // El fichero cambia entre `versiones` y `bajar`: la revisión fijada ya no
    // está (`404`), y no se leen otros bytes.
    m.subir("t1/sub/c.csv", "k\nOTRO\n");
    assert!(Origen::abrir(&g, &c.clave, &c.etag).is_err());
    assert!(matches!(
        Origen::leer_fijado(&g, &c.clave, &c.etag, "", None),
        Err(Rechazo::Cambiado(_))
    ));
    let (dir, _) = (Origen::listar(&g, "t1/").unwrap(), ());
    let nuevo = dir.iter().find(|o| o.clave == "t1/sub/c.csv").unwrap();
    assert_ne!(nuevo.etag, c.etag);
    assert_eq!(leer(&g, &nuevo.clave, &nuevo.etag), b"k\nOTRO\n");

    // Un nativo se exporta: sin el BOM del Doc, con el CRLF de la Hoja, el
    // rango cortado de lo exportado y la huella de lo exportado.
    assert_eq!(Origen::rango(&g, "t1/doc.txt", "0-3").unwrap(), b"Hola");
    assert_eq!(Origen::rango(&g, "t1/doc.txt", "-5").unwrap(), b"Drive");
    let hoja = dir.iter().find(|o| o.clave == "t1/hoja.csv").unwrap();
    assert!(hoja.etag.contains("#v"), "{}", hoja.etag);
    assert_eq!(leer(&g, &hoja.clave, &hoja.etag), b"a,b\r\n1,2\r\n3,4");
    let h = Origen::huella_de(&g, &hoja.clave, &hoja.etag).unwrap();
    assert_eq!(h.as_deref(), Some(md5(b"a,b\r\n1,2\r\n3,4").as_str()));
    let l = Origen::leer_fijado(&g, &hoja.clave, &hoja.etag, "", Some("bytes=0-2")).unwrap();
    assert_eq!(
        (l.estado, l.cabecera("content-range")),
        (206, Some("bytes 0-2/13"))
    );
    assert!(matches!(
        Origen::leer_fijado(&g, &hoja.clave, &hoja.etag, "", Some("bytes=99-100")),
        Err(Rechazo::Rango(_))
    ));

    // Un nativo cambia, o sólo se toca su metadato: cambia su validador y el
    // fijado ya no se lee (el `version` de Drive sube con los dos, X0 B5).
    m.nativo("t1/hoja", "hoja", "a,b\n1,2\n3,4\n5,6\n");
    assert!(matches!(
        Origen::leer_fijado(&g, &hoja.clave, &hoja.etag, "", None),
        Err(Rechazo::Cambiado(_))
    ));
    let otro = Origen::listar(&g, "t1/")
        .unwrap()
        .into_iter()
        .find(|o| o.clave == "t1/hoja.csv")
        .unwrap();
    m.accion("/tocar", "ruta=t1/hoja");
    let tocado = Origen::listar(&g, "t1/")
        .unwrap()
        .into_iter()
        .find(|o| o.clave == "t1/hoja.csv")
        .unwrap();
    assert_ne!(otro.etag, tocado.etag, "un cambio de metadatos también");
    assert_eq!(
        leer(&g, &tocado.clave, &tocado.etag),
        b"a,b\r\n1,2\r\n3,4\r\n5,6"
    );

    // Nadie mandó una petición sin su token.
    assert!(m.cuentas().contains("\"sin_token\": 0"), "{}", m.cuentas());
}

#[test]
fn el_arbol_sigue_los_cambios_y_se_recupera_si_el_feed_falla() {
    let Some(m) = Mentira::de_entorno() else {
        eprintln!("sin ORE_LAB_GDRIVE_MENTIRA no hay laboratorio: nada que probar");
        return;
    };
    let _uno = UNO_A_UNO.lock().unwrap_or_else(|e| e.into_inner());
    m.subir("t2/a.csv", "1\n");
    m.subir("t2/sub/b.csv", "2\n");
    let g = m.gdrive("t2");
    let (c0, i0) = recorridos();
    assert_eq!(claves(&g, "t2/"), ["t2/a.csv", "t2/sub/b.csv"]);
    let t0 = g.token_de_seguimiento().expect("hay índice");
    assert_eq!(recorridos(), (c0 + 1, i0), "la primera vez recorre entero");

    // Un fichero nuevo, uno renombrado, una carpeta movida con lo que lleva.
    m.subir("t2/nuevo.csv", "3\n");
    m.accion("/renombrar", "ruta=t2/nuevo.csv&a=n2.csv");
    m.accion("/mover", "ruta=t2/sub&a=t2/otra/");
    assert_eq!(
        claves(&g, "t2/"),
        ["t2/a.csv", "t2/n2.csv", "t2/otra/sub/b.csv"]
    );
    assert_eq!(
        recorridos(),
        (c0 + 1, i0 + 1),
        "una puesta al día, sin recorrer"
    );
    assert_ne!(g.token_de_seguimiento().unwrap(), t0, "el token avanzó");

    // Lo que cambia fuera de la carpeta se mira y se ignora.
    m.pedir("PUT", "", "ruta=ajeno.csv&raiz=OTRA", b"x\n");
    assert_eq!(claves(&g, "t2/").len(), 3);

    // Una carpeta que ENTRA desde fuera: el feed sólo dice que entró, y su
    // fichero se encuentra recorriéndola.
    m.pedir("PUT", "", "ruta=entra/d.csv&raiz=OTRA", b"d\n");
    m.pedir("POST", "/mover", "ruta=entra&raiz=OTRA&a=t2/", b"");
    assert!(claves(&g, "t2/").contains(&"t2/entra/d.csv".to_string()));

    // A la papelera, y borrada de verdad con lo que colgaba de ella.
    m.accion("/papelera", "ruta=t2/a.csv");
    m.pedir("DELETE", "", "ruta=t2/otra", b"");
    assert_eq!(claves(&g, "t2/"), ["t2/entra/d.csv", "t2/n2.csv"]);
    let (c1, i1) = recorridos();
    assert_eq!(c1, c0 + 1, "ninguno de estos cambios pidió recorrer entero");
    assert_eq!(
        i1,
        i0 + 4,
        "cuatro puestas al día, una por cada vez que se miró"
    );

    // Un token que Drive ya no acepta: se tira el índice y se recorre entero;
    // el árbol es el mismo.
    m.accion("/caducar", "");
    m.subir("t2/despues.csv", "5\n");
    assert_eq!(
        claves(&g, "t2/"),
        ["t2/despues.csv", "t2/entra/d.csv", "t2/n2.csv"]
    );
    assert_eq!(recorridos().0, c1 + 1, "el respaldo recorrió entero");
    // Y vuelve a seguir los cambios desde el token nuevo.
    m.subir("t2/ultimo.csv", "6\n");
    assert_eq!(claves(&g, "t2/").len(), 4);
    assert_eq!(recorridos().0, c1 + 1, "otra vez sin recorrer");

    // Una carpeta que no está es un error, no una lista vacía.
    let e = g.recorrer("t2/no/esta").unwrap_err();
    assert!(
        e.motivo().contains("is not in the Drive folder"),
        "{}",
        e.motivo()
    );
    assert!(m.cuentas().contains("\"sin_token\": 0"), "{}", m.cuentas());
}
