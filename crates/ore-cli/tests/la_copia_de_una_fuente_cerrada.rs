//! **Una copia cuya fuente la pasada no abrió se queda como estaba.** El Job
//! de la copia abre sólo las fuentes de lo que se le pide (`VISTAS`) y, sin
//! `--vista`, recorre todo: un dataset de otra fuente al que le tocaba leer el
//! origen salía en `error` (medido el 2026-10-12:
//! `standard_test.public.brain_embeddings`, tras la copia de una colección de
//! Azure). Con su copia y sin pedirse, no es su fallo; pedido con `--vista`, sí.
//!
//! El árbol es el caso de conformidad `v1alpha12/valid/a-copy-with-its-plan`:
//! `hr.empleados`, copia de `hr.employees` en el Postgres `erp` (`ERP_URL`). Sus punteros, en el
//! catálogo de la prueba.
#![cfg(unix)]

#[path = "comun/catalogo.rs"]
mod catalogo;

use std::path::{Path, PathBuf};
use std::process::Command;

use catalogo::Catalogo;

fn copiar(de: &Path, a: &Path) {
    std::fs::create_dir_all(a).unwrap();
    for e in std::fs::read_dir(de).unwrap().flatten() {
        let p = e.path();
        let q = a.join(e.file_name());
        if p.is_dir() {
            copiar(&p, &q);
        } else {
            std::fs::copy(&p, &q).unwrap();
        }
    }
}

fn arbol(nombre: &str) -> PathBuf {
    let caso = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/oos/conformance/v1alpha12/valid/a-copy-with-its-plan/input");
    let dir = std::env::temp_dir().join(format!(
        "ore-fuente-cerrada-{}-{nombre}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    copiar(&caso, &dir);
    dir
}

/// Lo que dejó una pasada anterior: una copia hecha, con otra cabecera.
const PUNTERO: &str = r#"{
  "dataset": "catalogo/hr/empleados",
  "estado": "copiada",
  "huella": "sha256:0000",
  "metadata_location": "gs://copia/ore/v2/catalogo/hr/empleados/metadata/00000-x.metadata.json",
  "snapshot": "1",
  "vista": "hr.empleados"
}
"#;

fn materializar(cat: &Catalogo, dir: &Path, extra: &[&str]) -> (Option<i32>, String) {
    let s = cat
        .en(Command::new(env!("CARGO_BIN_EXE_ore"))
            .args(["materialize", "."])
            .args(extra)
            .current_dir(dir)
            .env_remove("ERP_URL")
            .env_remove("ORE_REF"))
        .output()
        .expect("no se pudo invocar `ore`");
    (
        s.status.code(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&s.stdout),
            String::from_utf8_lossy(&s.stderr)
        ),
    )
}

#[test]
fn un_dataset_cuya_fuente_no_se_abrio_se_queda_como_estaba() {
    let dir = arbol("dataset");
    let cat = Catalogo::nuevo("fuente-cerrada");
    cat.poner("hr.empleados", PUNTERO, None);
    let antes = cat.leer("hr.empleados", None);

    let (codigo, salida) = materializar(&cat, &dir, &[]);
    assert!(
        salida.contains("la fuente `erp` no se abrió en esta pasada: se queda como estaba"),
        "{salida}"
    );
    assert_eq!(codigo, Some(0), "no es un fallo de la pasada:\n{salida}");
    assert_eq!(cat.leer("hr.empleados", None), antes, "{salida}");

    // Pedido con `--vista`, la falta de credencial sí es su fallo, y lo dice.
    let (codigo, salida) = materializar(&cat, &dir, &["--vista", "hr.empleados"]);
    let puntero = cat.leer("hr.empleados", None).unwrap_or_default();
    assert_ne!(codigo, Some(0), "{salida}");
    assert_eq!(
        cat.campo("hr.empleados", "estado").as_deref(),
        Some("error"),
        "{salida}\n{puntero}"
    );
    assert!(
        cat.campo("hr.empleados", "metadata_location").is_some(),
        "un fallo no borra la copia: {puntero}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Sin copia todavía no hay nada que conservar: es su fallo, y lo dice.
#[test]
fn sin_copia_la_fuente_cerrada_es_su_fallo() {
    let dir = arbol("sin-copia");
    let cat = Catalogo::nuevo("sin-copia");
    let (codigo, salida) = materializar(&cat, &dir, &[]);
    assert!(!salida.contains("no se abrió en esta pasada"), "{salida}");
    assert_ne!(codigo, Some(0), "{salida}");
    assert!(salida.contains("`ERP_URL`"), "{salida}");
    let _ = std::fs::remove_dir_all(&dir);
}
