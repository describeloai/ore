//! `ore lock` — resolver contra el árbol.
//!
//! Por la CLI pública y sin enlazar `ore-core`, como el resto: la especificación
//! exige que la implementación de referencia se ejerza **sin conocimiento
//! privilegiado de sus propias estructuras** (`00-overview` §3.3).
//!
//! Y el escenario se construye aquí en vez de vivir en `examples/`, porque lo
//! que se comprueba no es una ontología: es que **dos ejecuciones dan los mismos
//! bytes** y que un lock que quedó atrás se nota sin tocar el árbol. Las dos son
//! propiedades del comando, no del documento.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Un workspace con un vocabulario vendorizado dentro, que es el único caso que
/// se puede resolver hoy — y el que se usa.
fn escenario(nombre: &str, rango: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ore-lock-{nombre}"));
    let _ = std::fs::remove_dir_all(&dir);
    let escribir = |rel: &str, texto: &str| {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, texto).unwrap();
    };

    escribir(
        "ontology.config.yaml",
        &format!(
            "apiVersion: oos.dev/v1alpha1\n\
             kind: OntologyConfig\n\
             metadata: {{ name: consumidor, version: 0.1.0 }}\n\
             dependencies:\n  \
               - {{ package: oos.dev/regulatory/gdpr, version: \"{rango}\" }}\n"
        ),
    );
    // El vocabulario: un miembro sin entidades, que se llama por SU COORDENADA.
    // Que el nombre y la referencia sean la misma cosa es lo que permite
    // resolver sin inventar una convención de rutas.
    escribir(
        "packages/gdpr/package.yaml",
        "apiVersion: oos.dev/v1alpha1\n\
         kind: Package\n\
         metadata:\n  \
           name: oos.dev/regulatory/gdpr\n  \
           version: 0.1.0\n  \
           status: draft\n  \
           domain: compliance\n\
         spec: { owner: \"team:compliance\" }\n",
    );
    escribir(
        "packages/gdpr/concepts/personalEmail.yaml",
        "apiVersion: oos.dev/v1alpha4\n\
         kind: Property\n\
         metadata: { name: personalEmail, namespace: gdpr }\n\
         spec:\n  \
           type: String\n  \
           description: La direccion de correo de una persona fisica.\n",
    );
    dir
}

fn ore(dir: &Path, args: &[&str]) -> Output {
    con_registro(dir, args, &vacio())
}

/// Un directorio vacío hace de registro que no tiene nada. Hace falta un valor
/// SIEMPRE: `ore-fetch` está en el PATH de las pruebas —cargo pone ahí los
/// binarios— así que sin esto una prueba dependería de lo que hubiera en el
/// entorno de quien la corre.
fn vacio() -> PathBuf {
    let d = std::env::temp_dir().join("ore-registro-vacio");
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn con_registro(dir: &Path, args: &[&str], registro: &Path) -> Output {
    // El `PATH` se pone a mano, y hace falta: `ore` busca ahí el obtenedor, y
    // que `cargo test` deje el directorio de binarios en el `PATH` es cierto en
    // Windows y no en todas partes. Una prueba que dependa de eso pasa aquí y
    // falla en la CI por un motivo que no tiene nada que ver con lo que mide.
    let bin = Path::new(env!("CARGO_BIN_EXE_ore")).parent().unwrap();
    let path = match std::env::var_os("PATH") {
        Some(p) => {
            let mut dirs = vec![bin.to_path_buf()];
            dirs.extend(std::env::split_paths(&p));
            std::env::join_paths(dirs).unwrap()
        }
        None => bin.as_os_str().to_owned(),
    };
    Command::new(env!("CARGO_BIN_EXE_ore"))
        .args(args)
        .arg(dir)
        .env("ORE_FETCH_DIR", registro)
        .env("PATH", path)
        .output()
        .expect("no se pudo invocar `ore`")
}

fn salida(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

/// Lo que hace posible resolver sin registro: la coordenada **es** el nombre.
#[test]
fn resuelve_una_dependencia_contra_el_arbol() {
    let dir = escenario("resuelve", "^0.1");
    let o = ore(&dir, &["lock"]);
    assert!(o.status.success(), "{}", salida(&o));

    let lock = std::fs::read_to_string(dir.join("ontology.lock")).expect("no escribió el lock");
    assert!(lock.contains("name: oos.dev/regulatory/gdpr"), "{lock}");
    assert!(lock.contains("version: 0.1.0"), "{lock}");
    assert!(lock.contains("resolved: file:packages/gdpr"), "{lock}");
    assert!(lock.contains("digest: sha256:"), "{lock}");
    assert!(lock.contains("requestedBy: root"), "{lock}");
    // `provides` se DERIVA de lo que el paquete tiene dentro. Escribirlo a mano
    // en un artefacto generado sería declarar dos veces lo mismo.
    assert!(lock.contains("concepts: [gdpr.personalEmail]"), "{lock}");

    // Y con el lock puesto, `OOS2013` queda satisfecho: la dependencia declarada
    // está resuelta, que es lo que ese código comprueba.
    let v = ore(&dir, &["validate"]);
    assert!(v.status.success(), "{}", salida(&v));
}

/// Dos ejecuciones, los mismos bytes. Es `G1` aplicado al artefacto que fija de
/// qué depende un bundle: si el lock variara, el digest del bundle variaría con
/// él y el mismo commit dejaría de producir el mismo artefacto.
#[test]
fn dos_ejecuciones_dan_los_mismos_bytes() {
    let dir = escenario("determinista", "^0.1");
    assert!(ore(&dir, &["lock"]).status.success());
    let a = std::fs::read_to_string(dir.join("ontology.lock")).unwrap();
    assert!(ore(&dir, &["lock"]).status.success());
    let b = std::fs::read_to_string(dir.join("ontology.lock")).unwrap();
    assert_eq!(a, b, "dos ejecuciones sobre el mismo árbol difieren");
}

/// `--check` dice que el lock quedó atrás **sin arreglarlo**. En CI hace falta
/// esa distinción: un artefacto obsoleto que se corrige solo al mirarlo no se
/// distingue de uno al día.
#[test]
fn comprobar_no_toca_el_arbol() {
    let dir = escenario("check", "^0.1");
    assert!(ore(&dir, &["lock"]).status.success());
    let al_dia = ore(&dir, &["lock", "--check"]);
    assert!(al_dia.status.success(), "{}", salida(&al_dia));

    std::fs::write(
        dir.join("ontology.lock"),
        "lockfileVersion: 1\npackages: []\n",
    )
    .unwrap();
    let atras = ore(&dir, &["lock", "--check"]);
    assert!(!atras.status.success(), "aceptó un lock que quedó atrás");
    // Y NO lo ha reescrito: comprobar es comprobar.
    let tras = std::fs::read_to_string(dir.join("ontology.lock")).unwrap();
    assert!(
        !tras.contains("oos.dev"),
        "`--check` reescribió el lock:\n{tras}"
    );
}

/// Lo que no está ni en el árbol ni donde se busca **no se inventa**, y lo que
/// cuenta el obtenedor se muestra literal: es lo único accionable que existe.
#[test]
fn lo_que_no_esta_en_ninguna_parte_no_se_inventa() {
    let dir = escenario("ausente", "^0.1");
    std::fs::remove_dir_all(dir.join("packages/gdpr")).unwrap();
    let o = ore(&dir, &["lock"]);
    assert!(!o.status.success(), "resolvió algo que no está");
    assert!(
        !dir.join("ontology.lock").exists(),
        "escribió un lock sin resolver"
    );
}

/// La delegación entera: `ore` no habla por la red, ejecuta un programa que sí,
/// y **comprueba lo que le devuelve**. Después el árbol compila sin nadie, que
/// es el punto de vendorizar en vez de cachear.
#[test]
fn lo_que_falta_se_trae_y_se_queda() {
    let dir = escenario("traer", "^0.1");
    let registro = std::env::temp_dir().join("ore-registro-traer");
    let _ = std::fs::remove_dir_all(&registro);
    std::fs::create_dir_all(&registro).unwrap();

    // Se publica el paquete y se saca del árbol: a partir de aquí solo está
    // «fuera».
    let publicado = Command::new(env!("CARGO_BIN_EXE_ore"))
        .args(["pack", dir.join("packages/gdpr").to_str().unwrap()])
        .output()
        .unwrap();
    assert!(publicado.status.success());
    std::fs::write(registro.join("gdpr-0.1.0.oob"), &publicado.stdout).unwrap();
    std::fs::remove_dir_all(dir.join("packages/gdpr")).unwrap();

    let o = con_registro(&dir, &["lock"], &registro);
    assert!(o.status.success(), "{}", salida(&o));
    assert!(
        dir.join("vendor/gdpr-0.1.0.oob").exists(),
        "no lo vendorizó: {}",
        salida(&o)
    );
    assert!(salida(&o).contains("ore-fetch"), "{}", salida(&o));

    // Y ya no hace falta nadie: el registro se vacía y sigue compilando.
    std::fs::remove_dir_all(&registro).unwrap();
    let v = ore(&dir, &["validate"]);
    assert!(v.status.success(), "{}", salida(&v));
}

/// Nada de lo que llega se cree. Un `.oob` que diga otro paquete no se escribe,
/// venga de donde venga — que es lo que permite que el origen no tenga que ser
/// de confianza.
#[test]
fn lo_que_llega_y_no_es_lo_que_se_pidio_no_se_escribe() {
    let dir = escenario("impostor", "^0.1");
    let registro = std::env::temp_dir().join("ore-registro-impostor");
    let _ = std::fs::remove_dir_all(&registro);
    std::fs::create_dir_all(&registro).unwrap();

    // Se publica OTRO paquete con el nombre de fichero que el obtenedor espera.
    let otro = escenario("impostor-otro", "^0.1");
    std::fs::write(
        otro.join("package.yaml"),
        r#"apiVersion: oos.dev/v1alpha1
kind: Package
metadata:
  name: oos.dev/otro/paquete
  version: 0.1.0
  status: draft
  domain: x
spec: { owner: "team:x" }
"#,
    )
    .unwrap();
    let publicado = Command::new(env!("CARGO_BIN_EXE_ore"))
        .args(["pack", otro.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(publicado.status.success());
    std::fs::write(registro.join("gdpr-0.1.0.oob"), &publicado.stdout).unwrap();
    std::fs::remove_dir_all(dir.join("packages/gdpr")).unwrap();

    let o = con_registro(&dir, &["lock"], &registro);
    assert!(
        !o.status.success(),
        "aceptó un paquete que no era el pedido"
    );
    assert!(salida(&o).contains("llegó"), "{}", salida(&o));
    assert!(!dir.join("vendor").exists(), "escribió lo que no era");
}

/// Un rango que la versión del árbol no satisface no se resuelve. Escribirlo
/// diría que se cumple algo que no se cumple, y el lock es justamente el sitio
/// donde eso se afirma.
#[test]
fn un_rango_que_no_se_satisface_no_se_escribe() {
    let dir = escenario("rango", "^0.2");
    let o = ore(&dir, &["lock"]);
    assert!(!o.status.success(), "resolvió un rango que no se cumple");
    assert!(salida(&o).contains("^0.2"), "{}", salida(&o));
}

/// Sin dependencias no se escribe un lock. Un artefacto generado que no resuelve
/// nada es un fichero que hay que mantener sin que diga nada.
#[test]
fn sin_dependencias_no_hay_lock() {
    let dir = escenario("vacio", "^0.1");
    std::fs::write(
        dir.join("ontology.config.yaml"),
        "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\nmetadata: { name: c, version: 0.1.0 }\n",
    )
    .unwrap();
    let o = ore(&dir, &["lock"]);
    assert!(o.status.success(), "{}", salida(&o));
    assert!(
        !dir.join("ontology.lock").exists(),
        "escribió un lock vacío"
    );
}
