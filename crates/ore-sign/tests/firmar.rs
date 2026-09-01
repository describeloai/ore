//! El contrato del firmador, y la delegación entera vista desde fuera.
//!
//! Vive en la crate del firmador porque es donde `cargo` garantiza que su
//! binario exista — la misma razón por la que la prueba del obtenedor vive en la
//! suya. La **verificación** se prueba aparte, en `ore-core`, y sin depender de
//! ningún binario: es la mitad que tiene que funcionar aunque nadie tenga un
//! firmador instalado.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const SEMILLA: &str = "7777777777777777777777777777777777777777777777777777777777777777";

fn claves(nombre: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ore-sign-claves-{nombre}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("oos.dev.key"), SEMILLA).unwrap();
    d
}

fn firmador(dir: &Path, args: &[&str], entrada: Option<&str>) -> Output {
    let mut hijo = Command::new(env!("CARGO_BIN_EXE_ore-sign"))
        .args(args)
        .env("ORE_SIGN_DIR", dir)
        .stdin(if entrada.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("no se pudo invocar `ore-sign`");
    if let Some(t) = entrada {
        hijo.stdin.take().unwrap().write_all(t.as_bytes()).unwrap();
    }
    hijo.wait_with_output().unwrap()
}

fn salida(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

/// La delegación completa: se pide por stdin, sale la firma por stdout, y lo que
/// sale **verifica** con la pública que el mismo programa publica.
#[test]
fn firma_lo_que_le_llega_por_stdin_y_su_publica_lo_verifica() {
    let dir = claves("contrato");
    let enunciado = ore_core::firma::enunciado("oos.dev/regulatory/gdpr", "0.1.0", "sha256:abc");
    let peticion = format!(
        "{{\"keyId\":\"oos.dev\",\"statement\":{}}}",
        ore_core::json::Json::s(&enunciado).jcs()
    );

    let o = firmador(&dir, &[], Some(&peticion));
    assert!(o.status.success(), "{}", salida(&o));
    let firma = String::from_utf8_lossy(&o.stdout).trim().to_string();

    let p = firmador(&dir, &["--public", "oos.dev"], None);
    assert!(p.status.success(), "{}", salida(&p));
    let publica = String::from_utf8_lossy(&p.stdout).trim().to_string();

    assert_eq!(
        ore_core::firma::verificar(ore_core::firma::ED25519, &publica, &firma, &enunciado),
        Ok(())
    );
}

/// Por **stdin** y no por `argv`, y no es una preferencia de estilo: `argv` lo
/// lee cualquier proceso de la máquina, y aquí lo que pasaría por ahí es qué se
/// está firmando y con qué clave.
#[test]
fn sin_peticion_por_stdin_no_firma() {
    let o = firmador(&claves("vacio"), &[], Some(""));
    assert!(!o.status.success(), "firmó sin que nadie le pidiera nada");
    assert!(salida(&o).contains("stdin"), "{}", salida(&o));
}

/// El `keyId` es un nombre de fichero, así que un `..` leería una clave que no
/// es la que se pidió — y el fallo no lo vería nadie, porque la firma saldría
/// bien.
#[test]
fn un_key_id_que_atraviesa_directorios_se_rechaza() {
    let dir = claves("travesia");
    for id in ["../otra", "sub/otra", "..", "C:otra"] {
        let peticion = format!("{{\"keyId\":\"{id}\",\"statement\":\"x\"}}");
        let o = firmador(&dir, &[], Some(&peticion));
        assert!(!o.status.success(), "aceptó el `keyId` `{id}`");
    }
}

/// Y sin `ORE_SIGN_DIR` lo dice, en vez de firmar con algo que encontrara por
/// ahí.
#[test]
fn sin_directorio_de_claves_lo_dice() {
    let o = Command::new(env!("CARGO_BIN_EXE_ore-sign"))
        .args(["--public", "oos.dev"])
        .env_remove("ORE_SIGN_DIR")
        .output()
        .unwrap();
    assert!(!o.status.success());
    assert!(salida(&o).contains("ORE_SIGN_DIR"), "{}", salida(&o));
}

// ── Y la delegación desde `ore pack` ────────────────────────────────────────

/// El binario que empaqueta, al lado del que firma.
///
/// `cargo` no define `CARGO_BIN_EXE_ore` fuera de su propia crate, así que se
/// busca donde `cargo` los deja a los dos. Si falta, se dice en vez de saltarse
/// la prueba: una prueba que se salta sola en CI no prueba nada, y descubrirlo
/// más tarde cuesta más que leer este mensaje.
fn ore() -> PathBuf {
    let p = Path::new(env!("CARGO_BIN_EXE_ore-sign"))
        .with_file_name(format!("ore{}", std::env::consts::EXE_SUFFIX));
    assert!(
        p.is_file(),
        "falta `{}`. Esta prueba mide la delegación entre los dos binarios, así que \
         hacen falta los dos: `cargo test --workspace` los construye.",
        p.display()
    );
    p
}

/// `ore pack --sign` delega, y **comprueba lo que le devuelven**.
///
/// Que el paquete salga firmado es la mitad; la otra es que `ore` no se crea la
/// firma que recibe. Publicar un `.oob` con una firma rota repartiría un paquete
/// que nadie puede usar, y el fallo saldría en el árbol de otro.
#[test]
fn empaquetar_firmando_delega_y_el_oob_sale_con_la_firma() {
    let dir = claves("pack");
    let raiz = std::env::temp_dir().join("ore-sign-pack");
    let _ = std::fs::remove_dir_all(&raiz);
    let escribir = |rel: &str, texto: &str| {
        let p = raiz.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, texto).unwrap();
    };
    escribir(
        "package.yaml",
        r#"apiVersion: oos.dev/v1alpha1
kind: Package
metadata:
  name: oos.dev/regulatory/gdpr
  version: 0.1.0
  status: draft
  domain: compliance
spec: { owner: "team:compliance" }
"#,
    );
    escribir(
        "concepts/dateOfBirth.yaml",
        r#"apiVersion: oos.dev/v1alpha4
kind: Property
metadata: { name: dateOfBirth, namespace: gdpr }
spec:
  type: Date
  description: La fecha de nacimiento de una persona fisica.
"#,
    );

    // El PATH con el firmador delante: es como `ore` lo encuentra, y el contrato
    // dice «un programa del usuario en el PATH».
    let bin = Path::new(env!("CARGO_BIN_EXE_ore-sign")).parent().unwrap();
    let path = match std::env::var_os("PATH") {
        Some(p) => {
            let mut dirs = vec![bin.to_path_buf()];
            dirs.extend(std::env::split_paths(&p));
            std::env::join_paths(dirs).unwrap()
        }
        None => bin.as_os_str().to_owned(),
    };
    let empaquetar = |firmar: bool| {
        let mut args = vec!["pack".to_string(), raiz.to_string_lossy().to_string()];
        if firmar {
            args.extend(["--sign".to_string(), "oos.dev".to_string()]);
        }
        Command::new(ore())
            .args(&args)
            .env("ORE_SIGN_DIR", &dir)
            .env("PATH", &path)
            .output()
            .expect("no se pudo invocar `ore`")
    };

    let o = empaquetar(true);
    assert!(o.status.success(), "{}", salida(&o));
    let oob = String::from_utf8_lossy(&o.stdout).to_string();
    assert!(oob.contains("\"signatures\""), "el `.oob` salió sin firma");
    assert!(oob.contains("\"keyId\":\"oos.dev\""), "{oob}");

    // Y el digest no se mueve: firmar no cambia la identidad del paquete, así
    // que un lock resuelto antes de firmar sigue valiendo después.
    let digest = |o: &Output| {
        salida(o)
            .split_whitespace()
            .find(|t| t.starts_with("sha256:"))
            .map(String::from)
            .expect("no hay digest")
    };
    assert_eq!(digest(&o), digest(&empaquetar(false)));
}

/// Sin firmador en el PATH, `--sign` **falla**. No se publica sin firma lo que
/// se pidió firmado: un `.oob` que sale distinto de lo que se pidió y con
/// código de éxito es peor que un error.
#[test]
fn pedir_firma_sin_firmador_no_publica_sin_firma() {
    let raiz = std::env::temp_dir().join("ore-sign-sin-firmador");
    let _ = std::fs::remove_dir_all(&raiz);
    std::fs::create_dir_all(&raiz).unwrap();
    std::fs::write(
        raiz.join("package.yaml"),
        r#"apiVersion: oos.dev/v1alpha1
kind: Package
metadata: { name: oos.dev/x, version: 0.1.0, status: draft, domain: x }
spec: { owner: "team:x" }
"#,
    )
    .unwrap();
    let vacio = std::env::temp_dir().join("ore-sign-path-vacio");
    std::fs::create_dir_all(&vacio).unwrap();

    let o = Command::new(ore())
        .args(["pack", &raiz.to_string_lossy(), "--sign", "oos.dev"])
        .env("PATH", &vacio)
        .output()
        .expect("no se pudo invocar `ore`");
    assert!(!o.status.success(), "publicó sin la firma que se le pidió");
    assert!(salida(&o).contains("ore-sign"), "{}", salida(&o));
}
