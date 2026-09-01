//! `ore pack` — las propiedades que no caben en un `contains`.
//!
//! El caso de conformidad afirma la **forma** del sobre. Lo que no puede afirmar
//! con una subcadena es lo que hace útil al formato: que dos ejecuciones den los
//! mismos bytes, y que **el digest sea el del paquete sin empaquetar**. Las dos
//! se comprueban comparando dos cómputos, y las dos son del motor.
//!
//! Por la CLI pública y sin enlazar `ore-core`, como el resto.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn escenario(nombre: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ore-pack-{nombre}"));
    let _ = std::fs::remove_dir_all(&dir);
    let escribir = |rel: &str, texto: &str| {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, texto).unwrap();
    };
    escribir(
        "ontology.config.yaml",
        "apiVersion: oos.dev/v1alpha1\n\
         kind: OntologyConfig\n\
         metadata: { name: publicador, version: 0.1.0 }\n",
    );
    escribir(
        "package.yaml",
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
        "concepts/personalEmail.yaml",
        "apiVersion: oos.dev/v1alpha4\n\
         kind: Property\n\
         metadata: { name: personalEmail, namespace: gdpr }\n\
         spec:\n  \
           type: String\n  \
           description: La direccion de correo de una persona fisica.\n",
    );
    dir
}

fn ore(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ore"))
        .args(args)
        .output()
        .expect("no se pudo invocar `ore`")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).to_string()
}

fn todo(o: &Output) -> String {
    format!("{}{}", stdout(o), String::from_utf8_lossy(&o.stderr))
}

/// El sha que sale de una línea `· sha256:…`.
fn digest(texto: &str) -> String {
    texto
        .split_whitespace()
        .find(|t| t.starts_with("sha256:"))
        .unwrap_or_else(|| panic!("no hay digest en:\n{texto}"))
        .to_string()
}

/// Peldaño 1, primera mitad. Un archivo comprimido no lo cumpliría: marcas de
/// tiempo, orden de entradas y nivel de compresión hacen que el mismo paquete
/// dé bytes distintos.
#[test]
fn dos_ejecuciones_dan_los_mismos_bytes() {
    let dir = escenario("determinista");
    let a = ore(&["pack", dir.to_str().unwrap()]);
    let b = ore(&["pack", dir.to_str().unwrap()]);
    assert!(a.status.success(), "{}", todo(&a));
    assert_eq!(stdout(&a), stdout(&b), "dos empaquetados difieren");
    assert!(!stdout(&a).is_empty());
}

/// Peldaño 1, segunda mitad, **y es la que decide el formato**: si el digest
/// cambiara al empaquetar, el contenedor sería parte de la identidad y cambiar
/// de contenedor sería indistinguible de cambiar de paquete.
#[test]
fn el_contenedor_no_cambia_la_identidad() {
    let dir = escenario("identidad");
    let empaquetado = ore(&["pack", dir.to_str().unwrap()]);
    assert!(empaquetado.status.success(), "{}", todo(&empaquetado));

    // El mismo paquete, sin empaquetar, visto por el resolutor.
    let consumidor = std::env::temp_dir().join("ore-pack-identidad-consumidor");
    let _ = std::fs::remove_dir_all(&consumidor);
    copiar(&dir, &consumidor.join("packages/gdpr"));
    std::fs::remove_file(consumidor.join("packages/gdpr/ontology.config.yaml")).unwrap();
    std::fs::write(
        consumidor.join("ontology.config.yaml"),
        "apiVersion: oos.dev/v1alpha1\n\
         kind: OntologyConfig\n\
         metadata: { name: consumidor, version: 0.1.0 }\n\
         dependencies:\n  \
           - { package: oos.dev/regulatory/gdpr, version: \"^0.1\" }\n",
    )
    .unwrap();
    let resuelto = ore(&["lock", consumidor.to_str().unwrap()]);
    assert!(resuelto.status.success(), "{}", todo(&resuelto));
    let lock = std::fs::read_to_string(consumidor.join("ontology.lock")).unwrap();

    let del_paquete = digest(&todo(&empaquetado));
    assert!(
        lock.contains(&del_paquete),
        "el `.oob` digiere `{del_paquete}` y el lock resolvió otra cosa:\n{lock}"
    );
}

/// El manifiesto es del workspace de quien publica, y lleva sus fuentes. Que no
/// viaje no es higiene: es que publicarlo sería publicar la infraestructura de
/// otro.
#[test]
fn el_manifiesto_del_workspace_no_viaja() {
    let dir = escenario("manifiesto");
    std::fs::write(
        dir.join("ontology.config.yaml"),
        "apiVersion: oos.dev/v1alpha1\n\
         kind: OntologyConfig\n\
         metadata: { name: publicador, version: 0.1.0 }\n\
         datasources:\n  \
           - { name: interno, type: postgres, connectionEnv: SECRETO_INTERNO_URL }\n",
    )
    .unwrap();
    let o = ore(&["pack", dir.to_str().unwrap()]);
    assert!(o.status.success(), "{}", todo(&o));
    let oob = stdout(&o);
    assert!(!oob.contains("OntologyConfig"), "{oob}");
    assert!(!oob.contains("SECRETO_INTERNO_URL"), "{oob}");
}

/// Un binding dice dónde está el dato DE QUIEN PUBLICA, y viaja hacia alguien
/// que no tiene esa fuente.
#[test]
fn un_paquete_con_bindings_no_se_publica() {
    let dir = escenario("bindings");
    std::fs::write(
        dir.join("ontology.config.yaml"),
        "apiVersion: oos.dev/v1alpha1\n\
         kind: OntologyConfig\n\
         metadata: { name: publicador, version: 0.1.0 }\n\
         datasources: [{ name: db, type: postgres, connectionEnv: U }]\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("entities")).unwrap();
    std::fs::write(
        dir.join("entities/E.yaml"),
        "apiVersion: oos.dev/v1alpha1\n\
         kind: Entity\n\
         metadata: { name: E, namespace: gdpr }\n\
         spec:\n  \
           nature: entity\n  \
           primaryKey: [id]\n  \
           properties: { id: { type: String } }\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("b.yaml"),
        "apiVersion: oos.dev/v1alpha1\n\
         kind: Binding\n\
         metadata: { name: b, namespace: gdpr }\n\
         spec:\n  \
           targetEntity: gdpr.E\n  \
           datasourceRef: db\n  \
           source: \"t\"\n  \
           properties: { id: \"id\" }\n",
    )
    .unwrap();
    let o = ore(&["pack", dir.to_str().unwrap()]);
    assert!(!o.status.success(), "publicó la infraestructura de alguien");
    assert!(todo(&o).contains("binding"), "{}", todo(&o));
}

/// Publicar lo que no compila reparte un problema en vez de un paquete, y quien
/// lo importe lo descubrirá en su árbol, que es donde no puede arreglarlo.
#[test]
fn lo_que_no_valida_no_se_publica() {
    let dir = escenario("invalido");
    std::fs::write(
        dir.join("concepts/personalEmail.yaml"),
        "apiVersion: oos.dev/v1alpha4\n\
         kind: Property\n\
         metadata: { name: personalEmail, namespace: gdpr }\n\
         spec: { clave_inventada: si }\n",
    )
    .unwrap();
    let o = ore(&["pack", dir.to_str().unwrap()]);
    assert!(!o.status.success(), "empaquetó algo que no valida");
    assert!(todo(&o).contains("no valida"), "{}", todo(&o));
}

fn copiar(de: &Path, a: &Path) {
    std::fs::create_dir_all(a).unwrap();
    for e in std::fs::read_dir(de).unwrap().flatten() {
        let destino = a.join(e.file_name());
        if e.path().is_dir() {
            copiar(&e.path(), &destino);
        } else {
            std::fs::copy(e.path(), destino).unwrap();
        }
    }
}
