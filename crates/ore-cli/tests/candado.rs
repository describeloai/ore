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

/// Empaqueta un paquete del árbol y lo deja en el registro.
fn publicar(paquete: &Path, registro: &Path, como: &str) {
    let o = Command::new(env!("CARGO_BIN_EXE_ore"))
        .args(["pack", paquete.to_str().unwrap()])
        .output()
        .expect("no se pudo invocar `ore`");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    std::fs::write(registro.join(como), &o.stdout).unwrap();
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

/// Un obtenedor **que no es el nuestro**, escrito aquí mismo.
///
/// Empezó siendo `ore-fetch`, el de referencia, y falló en la CI por un motivo
/// que no tenía nada que ver con lo que mide: `cargo test --workspace` no
/// garantiza que el binario de OTRO miembro esté enlazado cuando corren estas
/// pruebas. El arreglo mejora la prueba — lo que el contrato dice es *«un
/// programa del usuario en el PATH»*, y un guion de tres líneas lo demuestra
/// mejor que un binario nuestro.
///
/// El de referencia tiene su propia prueba, en su propia crate, que es donde
/// `cargo` sí garantiza que exista.
fn obtenedor(registro: &Path) -> PathBuf {
    // Uno por LLAMADA, no por registro: las pruebas corren en paralelo y varias
    // comparten el registro vacío, así que escribían el mismo fichero a la vez.
    // Es un fallo del andamio disfrazado de fallo del producto, y de los que
    // solo aparecen a veces — que es la peor clase.
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let bin = std::env::temp_dir().join(format!(
        "ore-obtenedor-{}-{}",
        std::process::id(),
        N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&bin).unwrap();
    // El de mayor version que haya, como haria un registro. Se resuelve al
    // escribir el guion porque un `.bat` no sabe ordenar.
    let mut ficheros: Vec<std::path::PathBuf> = std::fs::read_dir(registro)
        .map(|e| e.flatten().map(|x| x.path()).collect())
        .unwrap_or_default();
    ficheros.sort();
    let oob = ficheros
        .pop()
        .unwrap_or_else(|| registro.join("ninguno.oob"));
    if cfg!(windows) {
        let p = bin.join("ore-fetch.bat");
        std::fs::write(
            &p,
            format!(
                "@echo off\r\nif not exist \"{0}\" exit /b 1\r\ntype \"{0}\"\r\n",
                oob.display()
            ),
        )
        .unwrap();
    } else {
        let p = bin.join("ore-fetch");
        std::fs::write(
            &p,
            format!(
                "#!/bin/sh\n[ -f '{0}' ] || exit 1\ncat '{0}'\n",
                oob.display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    bin
}

fn con_registro(dir: &Path, args: &[&str], registro: &Path) -> Output {
    let bin = obtenedor(registro);
    let path = match std::env::var_os("PATH") {
        Some(p) => {
            let mut dirs = vec![bin];
            dirs.extend(std::env::split_paths(&p));
            std::env::join_paths(dirs).unwrap()
        }
        None => bin.into_os_string(),
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
fn una_coordenada_que_no_esta_en_ninguna_parte_no_se_resuelve() {
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
fn una_dependencia_ausente_se_obtiene_y_se_vendoriza() {
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
    assert!(salida(&o).contains("vendor/"), "{}", salida(&o));

    // Y ya no hace falta nadie: el registro se vacía y sigue compilando.
    std::fs::remove_dir_all(&registro).unwrap();
    let v = ore(&dir, &["validate"]);
    assert!(v.status.success(), "{}", salida(&v));
}

/// Actualizar es el resto del ciclo de vida, y no existía: con la vieja
/// vendorizada y el rango subido, `ore lock` fallaba en vez de traer la nueva.
/// Un ciclo que solo cubre la primera vez no es un ciclo.
#[test]
fn subir_el_rango_trae_la_nueva_y_retira_la_vieja() {
    let dir = escenario("subir", "^0.1");
    let registro = std::env::temp_dir().join("ore-registro-subir");
    let _ = std::fs::remove_dir_all(&registro);
    std::fs::create_dir_all(&registro).unwrap();
    publicar(&dir.join("packages/gdpr"), &registro, "gdpr-0.1.0.oob");
    std::fs::remove_dir_all(dir.join("packages/gdpr")).unwrap();
    assert!(con_registro(&dir, &["lock"], &registro).status.success());
    assert!(dir.join("vendor/gdpr-0.1.0.oob").exists());

    // Se publica la 0.2.0 y se sube el rango.
    let nueva = escenario("subir-v2", "^0.1");
    let p = nueva.join("packages/gdpr/package.yaml");
    let t = std::fs::read_to_string(&p)
        .unwrap()
        .replace("0.1.0", "0.2.0");
    std::fs::write(&p, t).unwrap();
    publicar(&nueva.join("packages/gdpr"), &registro, "gdpr-0.2.0.oob");
    let c = dir.join("ontology.config.yaml");
    let t = std::fs::read_to_string(&c).unwrap().replace("^0.1", "^0.2");
    std::fs::write(&c, t).unwrap();

    let o = con_registro(&dir, &["lock"], &registro);
    assert!(o.status.success(), "{}", salida(&o));
    assert!(
        dir.join("vendor/gdpr-0.2.0.oob").exists(),
        "no trajo la nueva"
    );
    // Y la vieja se retira: dos `.oob` del mismo paquete son dos verdades, y el
    // cargador metería las dos.
    assert!(
        !dir.join("vendor/gdpr-0.1.0.oob").exists(),
        "dejó las dos versiones en el árbol"
    );
    assert!(
        con_registro(&dir, &["validate"], &registro)
            .status
            .success()
    );
}

/// Un rango que el lock no satisface **no compila**. El manifiesto pedía una
/// cosa, el lock resolvía otra, y lo que gobernaba era la clasificación vieja —
/// en verde.
#[test]
fn un_rango_que_el_lock_no_resuelve_no_compila() {
    let dir = escenario("desfasado", "^0.1");
    let registro = std::env::temp_dir().join("ore-registro-desfasado");
    let _ = std::fs::remove_dir_all(&registro);
    std::fs::create_dir_all(&registro).unwrap();
    publicar(&dir.join("packages/gdpr"), &registro, "gdpr-0.1.0.oob");
    assert!(con_registro(&dir, &["lock"], &registro).status.success());
    assert!(
        con_registro(&dir, &["validate"], &registro)
            .status
            .success()
    );

    let c = dir.join("ontology.config.yaml");
    let t = std::fs::read_to_string(&c).unwrap().replace("^0.1", "^0.9");
    std::fs::write(&c, t).unwrap();

    let o = con_registro(&dir, &["validate"], &registro);
    assert!(
        !o.status.success(),
        "el manifiesto y el lock discrepan y pasó"
    );
    assert!(salida(&o).contains("OOS2013"), "{}", salida(&o));
}

/// Nada de lo que llega se cree. Un `.oob` que diga otro paquete no se escribe,
/// venga de donde venga — que es lo que permite que el origen no tenga que ser
/// de confianza.
#[test]
fn un_oob_que_no_es_el_que_se_pidio_no_se_escribe() {
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

/// Un rango que la versión del árbol no satisface **se pide**, y si nadie la
/// tiene, no se escribe nada.
///
/// Que se pida es lo que cambió al existir el ciclo de actualización: una
/// versión corta ya no es un callejón, es una dependencia por resolver. Lo que
/// no cambia es lo de siempre — escribir en el lock que se cumple un rango que
/// no se cumple sería afirmar en el único sitio donde eso se afirma.
#[test]
fn un_rango_que_no_se_satisface_no_se_escribe() {
    let dir = escenario("rango", "^0.2");
    let o = ore(&dir, &["lock"]); // el registro vacío: nadie tiene la 0.2
    assert!(!o.status.success(), "resolvió un rango que no se cumple");
    assert!(
        !dir.join("ontology.lock").exists(),
        "escribió un lock que no cumple el rango"
    );
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
