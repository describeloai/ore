//! ORE 0051 P7 · lo que nunca es nulo, visto desde fuera (OOS v1alpha22 `01`
//! §7 y §8), sobre el caso de conformidad
//! `v1alpha22/emit/a-guaranteed-column-is-non-null`: una tabla cuyo origen
//! garantiza `id` y `email`, una vista que las lee tal cual, y una entidad que
//! exige `apodo` sin que su columna lo garantice.

use std::path::PathBuf;
use std::process::Command;

fn caso() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/oos/conformance/v1alpha22/emit/a-guaranteed-column-is-non-null/input")
}

fn ore(args: &[&str]) -> (Option<i32>, String, String) {
    let s = Command::new(env!("CARGO_BIN_EXE_ore"))
        .args(args)
        .output()
        .expect("ore arranca");
    (
        s.status.code(),
        String::from_utf8_lossy(&s.stdout).into_owned(),
        String::from_utf8_lossy(&s.stderr).into_owned(),
    )
}

/// §8: el documento es válido —sale con 0— y `ore validate` avisa de la
/// propiedad que la entidad exige y su columna no garantiza. Sólo de esa: `id`
/// y `email` las garantiza el origen, y `nota` no la exige nadie.
#[test]
fn validate_avisa_de_lo_que_la_entidad_exige_y_la_columna_no_garantiza() {
    let c = caso();
    let (codigo, out, err) = ore(&["validate", c.to_str().unwrap()]);
    assert_eq!(codigo, Some(0), "{out}{err}");
    assert!(out.contains("ok · sin errores"), "{out}");
    assert!(
        out.contains(
            "aviso: `ventas.Cliente.apodo` es `required` y la columna `apodo` de `ventas.clientes` puede ser nula"
        ),
        "{out}"
    );
    assert_eq!(out.matches("aviso:").count(), 1, "{out}");
}

/// §7: el `!` de un campo sale de su columna, y el `required` de la propiedad
/// no lo pone.
#[test]
fn graphql_pone_el_signo_donde_la_columna_nunca_es_nula() {
    let c = caso();
    let (codigo, sdl, err) = ore(&["export", c.to_str().unwrap(), "--format", "graphql"]);
    assert_eq!(codigo, Some(0), "{err}");
    assert!(sdl.contains("id: ID!"), "{sdl}");
    assert!(sdl.contains("email: String!"), "{sdl}");
    assert!(
        sdl.contains("nota: String") && !sdl.contains("nota: String!"),
        "{sdl}"
    );
    assert!(
        sdl.contains("apodo: String") && !sdl.contains("apodo: String!"),
        "{sdl}"
    );
}
