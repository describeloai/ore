//! El obtenedor de referencia, por su contrato.
//!
//! Vive en su propia crate porque es donde `cargo` garantiza que el binario
//! exista: `CARGO_BIN_EXE_ore-fetch` solo está definido para las pruebas del
//! paquete que lo declara. Intentarlo desde `ore-cli` salió verde en Windows y
//! rojo en la CI, por un motivo que no tenía nada que ver con lo que medía.

use std::io::Write as _;
use std::process::{Command, Stdio};

fn pedir(dir: Option<&str>, peticion: &str) -> (bool, String, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ore-fetch"));
    match dir {
        Some(d) => cmd.env("ORE_FETCH_DIR", d),
        None => cmd.env_remove("ORE_FETCH_DIR"),
    };
    let mut hijo = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("no se pudo invocar `ore-fetch`");
    hijo.stdin
        .take()
        .unwrap()
        .write_all(peticion.as_bytes())
        .unwrap();
    let s = hijo.wait_with_output().unwrap();
    (
        s.status.success(),
        String::from_utf8_lossy(&s.stdout).to_string(),
        String::from_utf8_lossy(&s.stderr).to_string(),
    )
}

fn registro(nombre: &str, ficheros: &[(&str, &str)]) -> String {
    let d = std::env::temp_dir().join(format!("ore-fetch-{nombre}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    for (n, c) in ficheros {
        std::fs::write(d.join(n), c).unwrap();
    }
    d.to_string_lossy().to_string()
}

/// La petición entra por stdin y el `.oob` sale por stdout. Nada más por stdout:
/// lo que se canaliza es el artefacto.
#[test]
fn la_peticion_entra_por_stdin_y_el_oob_sale_por_stdout() {
    let d = registro("basico", &[("gdpr-0.1.0.oob", "{\"package\":\"x\"}")]);
    let (ok, out, err) = pedir(
        Some(&d),
        "{\"package\":\"oos.dev/regulatory/gdpr\",\"range\":\"^0.1\"}",
    );
    assert!(ok, "{err}");
    assert_eq!(out, "{\"package\":\"x\"}");
}

/// Devuelve la MÁS ALTA que encuentra, y no interpreta el rango: quien pide
/// comprueba de todos modos, y hacerlo aquí solo haría que esa comprobación
/// pareciera de más.
#[test]
fn devuelve_la_mas_alta_y_no_interpreta_el_rango() {
    let d = registro(
        "versiones",
        &[
            ("gdpr-0.1.0.oob", "vieja"),
            ("gdpr-0.9.0.oob", "nueva"),
            ("gdpr-0.10.0.oob", "la mas alta"),
        ],
    );
    let (ok, out, err) = pedir(Some(&d), "{\"package\":\"oos.dev/regulatory/gdpr\"}");
    assert!(ok, "{err}");
    // `0.10` es mayor que `0.9`: se comparan números, no cadenas.
    assert_eq!(out, "la mas alta");
}

/// Sin nada que traer falla, y lo cuenta por **stderr**: `ore` lo muestra
/// literal, y es lo único accionable que existe.
#[test]
fn un_paquete_ausente_se_dice_por_stderr() {
    let d = registro("vacio", &[]);
    let (ok, out, err) = pedir(Some(&d), "{\"package\":\"oos.dev/regulatory/gdpr\"}");
    assert!(!ok);
    assert!(out.is_empty(), "escribió algo por stdout: {out}");
    assert!(err.contains("gdpr-<version>.oob"), "{err}");
}

/// Una petición vacía no se adivina. La coordenada va por stdin y no por la
/// línea de órdenes porque `argv` lo lee cualquier proceso de la máquina.
#[test]
fn sin_peticion_no_se_inventa_una() {
    let d = registro("sin-peticion", &[]);
    let (ok, _, err) = pedir(Some(&d), "");
    assert!(!ok);
    assert!(err.contains("stdin"), "{err}");
}
