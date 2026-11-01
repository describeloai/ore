//! **`ore puntero <nombre>`** (0079 ·5d): lo que los guiones de los Jobs le preguntan a `ore` en
//! vez de abrir el fichero del puntero, que con el catálogo no existe. Por la puerta de
//! `ORE_PUNTEROS`; sin ella, no hay a quién preguntar.
#![cfg(unix)]

#[path = "comun/catalogo.rs"]
mod catalogo;

use std::path::Path;
use std::process::Command;

use catalogo::Catalogo;

fn puntero(cat: Option<&Catalogo>, nombre: &str) -> (i32, String, String) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_ore"));
    c.args(["puntero", nombre, "--path"])
        .arg(Path::new("."))
        .env_remove("ORE_PUNTEROS")
        .env_remove("ORE_REF");
    if let Some(cat) = cat {
        cat.en(&mut c);
    }
    let s = c.output().unwrap();
    (
        s.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&s.stdout).into_owned(),
        String::from_utf8_lossy(&s.stderr).into_owned(),
    )
}

#[test]
fn el_puntero_de_un_nombre_por_la_puerta() {
    let cat = Catalogo::nuevo("puntero");
    cat.poner(
        "ventas.pedidos",
        r#"{"estado": "copiada", "filas": "3"}"#,
        None,
    );
    cat.poner(
        "ventas.eu.clientes",
        r#"{"estado": "error", "motivo": "la fuente no contesta"}"#,
        None,
    );
    let (c, t, e) = puntero(Some(&cat), "ventas.pedidos");
    assert_eq!(c, 0, "{e}");
    assert!(t.contains("\"filas\"") && t.contains("\"3\""), "{t}");
    let (c, t, e) = puntero(Some(&cat), "ventas.eu.clientes");
    assert_eq!(c, 0, "{e}");
    assert!(t.contains("la fuente no contesta"), "{t}");
    assert_eq!(puntero(Some(&cat), "ventas.nunca").0, 66, "sin puntero, 66");
    assert_eq!(puntero(Some(&cat), "sin_base").0, 65, "un no-nombre, 65");
    // Sin puerta, el almacén no contesta (69) y se dice por qué.
    let (c, _, e) = puntero(None, "ventas.pedidos");
    assert_eq!(c, 69, "{e}");
    assert!(e.contains("ORE_PUNTEROS"), "{e}");
}
