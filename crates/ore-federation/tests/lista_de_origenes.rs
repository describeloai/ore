//! **Cada conector del espacio de trabajo llega a la imagen.**
//!
//! Un `ore-read-<tipo>` hay que nombrarlo en tres sitios a mano: el `cargo build`
//! de `ci/compilar-binarios.sh` (`-p`), la lista `TODOS` que ese script copia, y
//! el `COPY` del `Dockerfile` a `/opt/ore/conectores`. Olvidar uno compila, pasa
//! el CI y **no está en producción**: la pasarela contesta que no tiene el tipo
//! (el 400 de MongoDB, 2026-10-20). La pasarela ya no lleva lista (descubre lo que
//! hay en su carpeta); esto cierra el resto.
//!
//! «Un conector» es un crate `crates/ore-read-*` con `src/main.rs`.

use std::path::{Path, PathBuf};

fn raiz() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("raíz del espacio de trabajo")
}

fn conectores() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(raiz().join("crates"))
        .expect("crates/")
        .flatten()
        .filter(|e| e.path().join("src/main.rs").is_file())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.starts_with("ore-read-"))
        .collect();
    v.sort();
    v
}

fn leer(rel: &str) -> String {
    std::fs::read_to_string(raiz().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// Si `texto` nombra `ore-read-x` como palabra entera (no como prefijo de otro).
fn nombra(texto: &str, nombre: &str) -> bool {
    texto.match_indices(nombre).any(|(i, _)| {
        let sigue = texto[i + nombre.len()..].chars().next();
        !sigue.is_some_and(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    })
}

#[test]
fn todo_conector_se_compila_se_empaqueta_y_se_copia() {
    let todos = conectores();
    assert!(todos.len() >= 9, "no veo los conectores: {todos:?}");
    let script = leer("ci/compilar-binarios.sh");
    let imagen = leer("Dockerfile");
    let paquetes = script
        .split("TODOS=")
        .next()
        .expect("PAQUETES va antes de TODOS");
    let lista = script.split("TODOS=").nth(1).expect("TODOS");

    let mut faltan = Vec::new();
    for c in &todos {
        if !nombra(paquetes, &format!("-p {c}")) {
            faltan.push(format!("{c}: sin `-p {c}` en ci/compilar-binarios.sh"));
        }
        if !nombra(lista, c) {
            faltan.push(format!("{c}: fuera de TODOS en ci/compilar-binarios.sh"));
        }
        if !imagen.contains(&format!("/opt/ore/conectores/{c}")) {
            faltan.push(format!(
                "{c}: sin `COPY … /opt/ore/conectores/{c}` en el Dockerfile"
            ));
        }
    }
    assert!(
        faltan.is_empty(),
        "un conector que no llega a la imagen no está en producción:\n  {}",
        faltan.join("\n  ")
    );
}
