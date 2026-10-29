//! **`ore puntero <nombre>`** (0079 ·5d): lo que los guiones de los Jobs le preguntan a `ore` en
//! vez de abrir el fichero del puntero, que con el catálogo no existe. Por la puerta: con ficheros,
//! el del árbol (el de su sitio, o el de antes); con `ORE_PUNTEROS`, el de quien la sirva.

use std::path::{Path, PathBuf};
use std::process::Command;

fn arbol(nombre: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ore-puntero-{nombre}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("datasets/ventas/default")).unwrap();
    std::fs::create_dir_all(d.join("datasets/ventas/eu")).unwrap();
    std::fs::write(
        d.join("datasets/ventas/default/pedidos.json"),
        "{\"estado\": \"copiada\", \"filas\": \"3\"}\n",
    )
    .unwrap();
    std::fs::write(
        d.join("datasets/ventas/eu/clientes.json"),
        "{\"estado\": \"error\", \"motivo\": \"la fuente no contesta\"}\n",
    )
    .unwrap();
    // Uno de antes: `datasets/<p>_<n>.json`.
    std::fs::write(
        d.join("datasets/ventas_viejo.json"),
        "{\"estado\": \"copiada\"}\n",
    )
    .unwrap();
    d
}

fn puntero(dir: &Path, nombre: &str, entorno: &[(&str, &str)]) -> (i32, String) {
    let s = Command::new(env!("CARGO_BIN_EXE_ore"))
        .args(["puntero", nombre, "--path"])
        .arg(dir)
        .env_remove("ORE_PUNTEROS")
        .envs(entorno.iter().copied())
        .output()
        .unwrap();
    (
        s.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&s.stdout).into_owned(),
    )
}

#[test]
fn el_puntero_de_un_nombre_por_la_puerta() {
    let d = arbol("ficheros");
    let (c, t) = puntero(&d, "ventas.pedidos", &[]);
    assert_eq!(c, 0);
    assert!(t.contains("\"filas\": \"3\""), "{t}");
    let (c, t) = puntero(&d, "ventas.eu.clientes", &[]);
    assert_eq!(c, 0);
    assert!(t.contains("la fuente no contesta"), "{t}");
    let (c, t) = puntero(&d, "ventas.viejo", &[]);
    assert_eq!(c, 0, "el de antes también se lee: {t}");
    assert_eq!(puntero(&d, "ventas.nunca", &[]).0, 66, "sin puntero, 66");
    assert_eq!(puntero(&d, "sin_base", &[]).0, 65, "un no-nombre, 65");
    // Por el proceso (la puerta que usa un Job con el catálogo): lo mismo.
    let orden = format!("{} punteros-servir", env!("CARGO_BIN_EXE_ore"));
    let (c, t) = puntero(&d, "ventas.pedidos", &[("ORE_PUNTEROS", orden.as_str())]);
    assert_eq!(c, 0);
    assert!(t.contains("\"filas\": \"3\""), "{t}");
    assert_eq!(
        puntero(&d, "ventas.nunca", &[("ORE_PUNTEROS", orden.as_str())]).0,
        66
    );
    let _ = std::fs::remove_dir_all(&d);
}
