//! **La puerta heredada** (0079 §19.2 P1, ·5c·3): `ore` con
//! `ORE_PUNTEROS=fd:<lee>,<escribe>` habla el protocolo de los punteros por dos
//! tuberías que hereda, y quien lo lanzó le sirve la puerta desde su lado —aquí,
//! como hará ore-serve, un hilo con `Servidor` sobre `Ficheros`—. Lee y escribe
//! lo mismo que sin ella.
//!
//! El árbol es el de `la_copia_de_una_fuente_cerrada.rs`
//! (`v1alpha12/valid/a-copy-with-its-plan`): `materialize --vista` sobre una
//! fuente cerrada escribe un puntero `error` que conserva la copia.
#![cfg(unix)]

use std::io::BufReader;
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use ore_core::punteros::Ficheros;
use ore_core::punteros::protocolo::{self, Apertura, Puerta, Servidor};

fn copiar(de: &Path, a: &Path) {
    std::fs::create_dir_all(a).unwrap();
    for e in std::fs::read_dir(de).unwrap().flatten() {
        let (p, q) = (e.path(), a.join(e.file_name()));
        if p.is_dir() {
            copiar(&p, &q);
        } else {
            std::fs::copy(&p, &q).unwrap();
        }
    }
}

const PUNTERO: &str = r#"{
  "dataset": "catalogo/hr/empleados",
  "estado": "copiada",
  "huella": "sha256:0000",
  "metadata_location": "gs://copia/ore/v2/catalogo/hr/empleados/metadata/00000-x.metadata.json",
  "snapshot": "1",
  "vista": "hr.empleados"
}
"#;

fn arbol(nombre: &str) -> PathBuf {
    let caso = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/oos/conformance/v1alpha12/valid/a-copy-with-its-plan/input");
    let dir = std::env::temp_dir().join(format!(
        "ore-puerta-heredada-{}-{nombre}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    copiar(&caso, &dir);
    let p = dir.join("datasets/hr/default/empleados.json");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, PUNTERO).unwrap();
    dir
}

mod c {
    use std::ffi::c_int;
    pub const F_GETFD: c_int = 1;
    pub const F_SETFD: c_int = 2;
    pub const FD_CLOEXEC: c_int = 1;
    unsafe extern "C" {
        pub fn fcntl(fd: c_int, cmd: c_int, ...) -> c_int;
    }
}

/// Lo que se le preguntó a la puerta servida: las `op`, en orden.
type Oido = Arc<Mutex<Vec<String>>>;

/// `ore` con la puerta heredada, servida por un hilo sobre `Ficheros` (lo que
/// hace ore-serve), o directo. Devuelve el código, la salida y lo oído.
fn ore(dir: &Path, args: &[&str], heredada: bool) -> (Option<i32>, String, Vec<String>) {
    let mut orden = Command::new(env!("CARGO_BIN_EXE_ore"));
    orden
        .args(args)
        .current_dir(dir)
        .env_remove("ERP_URL")
        .env_remove("ORE_PUNTEROS")
        .env("ORE_REF", "propuesta-1");
    if !heredada {
        let s = orden.output().expect("no se pudo invocar `ore`");
        return (
            s.status.code(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&s.stdout),
                String::from_utf8_lossy(&s.stderr)
            ),
            vec![],
        );
    }
    let (lee_padre, escribe_hijo) = std::io::pipe().unwrap();
    let (lee_hijo, escribe_padre) = std::io::pipe().unwrap();
    let (r, w) = (lee_hijo.as_raw_fd(), escribe_hijo.as_raw_fd());
    orden.env("ORE_PUNTEROS", format!("fd:{r},{w}"));
    // SAFETY: sólo `fcntl` sobre dos enteros, entre `fork` y `exec`.
    unsafe {
        orden.pre_exec(move || {
            for fd in [r, w] {
                let m = c::fcntl(fd, c::F_GETFD);
                if m < 0 || c::fcntl(fd, c::F_SETFD, m & !c::FD_CLOEXEC) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    let oido: Oido = Arc::default();
    let (o, cwd) = (Arc::clone(&oido), dir.to_path_buf());
    let hilo = std::thread::spawn(move || {
        let o2 = Arc::clone(&o);
        let mut s = Servidor::nuevo(Box::new(move |a: &Apertura| {
            o2.lock()
                .unwrap()
                .push(format!("abrir {}", a.rama.clone().unwrap_or_default()));
            let f = Ficheros::en(&cwd.join(&a.raiz))
                .con_ajenos(a.ajenos.as_deref().map(|d| cwd.join(d)).as_deref());
            Ok(Box::new(match &a.carpeta {
                Some(c) => f.con_carpeta(&cwd.join(c)),
                None => f,
            }) as Box<dyn Puerta>)
        }));
        // Lo que pasa por la tubería, apuntado.
        let lector = BufReader::new(Apuntado(lee_padre, o));
        protocolo::servir(lector, escribe_padre, &mut s)
    });
    let s = orden.output().expect("no se pudo invocar `ore`");
    drop((lee_hijo, escribe_hijo));
    hilo.join().unwrap().unwrap();
    let oido = oido.lock().unwrap().clone();
    (
        s.status.code(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&s.stdout),
            String::from_utf8_lossy(&s.stderr)
        ),
        oido,
    )
}

/// Un lector que apunta la `op` de cada línea que pasa.
struct Apuntado(std::io::PipeReader, Oido);

impl std::io::Read for Apuntado {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.0.read(buf)?;
        for linea in String::from_utf8_lossy(&buf[..n]).lines() {
            if let Some(i) = linea.find("\"op\":\"") {
                let op = &linea[i + 6..];
                if let Some(f) = op.find('"') {
                    self.1.lock().unwrap().push(op[..f].to_string());
                }
            }
        }
        Ok(n)
    }
}

/// ⭐ Leer: el listado, la ficha y un nombre que no está, igual por las dos.
#[test]
fn ore_lee_igual_por_la_puerta_heredada() {
    let (directo, heredado) = (arbol("lee-d"), arbol("lee-h"));
    for args in [
        vec!["datasets", "."],
        vec!["datasets", ".", "--ficha", "hr.empleados"],
        vec!["datasets", ".", "--ficha", "hr.nadie"],
    ] {
        let (ca, sa, _) = ore(&directo, &args, false);
        let (cb, sb, oido) = ore(&heredado, &args, true);
        let quitar = |s: &str, d: &Path| s.replace(&d.to_string_lossy().into_owned(), "<d>");
        assert_eq!(ca, cb, "{args:?}\n{sa}\n{sb}");
        assert_eq!(quitar(&sa, &directo), quitar(&sb, &heredado), "{args:?}");
        assert!(
            oido.first().map(String::as_str) == Some("abrir"),
            "la primera pregunta abre: {oido:?}"
        );
        assert!(
            oido.iter().any(|o| o == "abrir propuesta-1"),
            "la ref llega en el `abrir`: {oido:?}"
        );
        assert!(oido.len() > 2, "pasó por la puerta: {oido:?}");
    }
    let _ = std::fs::remove_dir_all(&directo);
    let _ = std::fs::remove_dir_all(&heredado);
}

/// ⭐ Escribir: la pasada que no puede abrir la fuente deja el mismo puntero
/// `error` por las dos, y el de la heredada lo escribió quien sirve.
#[test]
fn ore_escribe_igual_por_la_puerta_heredada() {
    let (directo, heredado) = (arbol("escribe-d"), arbol("escribe-h"));
    let args = [
        "materialize",
        ".",
        "--informe",
        "datasets",
        "--vista",
        "hr.empleados",
    ];
    let (ca, sa, _) = ore(&directo, &args, false);
    let (cb, sb, oido) = ore(&heredado, &args, true);
    assert_eq!(ca, cb, "{sa}\n{sb}");
    assert_ne!(cb, Some(0), "{sb}");
    assert!(oido.iter().any(|o| o == "escribir"), "{oido:?}");
    let leer =
        |d: &Path| std::fs::read_to_string(d.join("datasets/hr/default/empleados.json")).unwrap();
    assert!(
        leer(&heredado).contains(r#""estado": "error""#),
        "{}",
        leer(&heredado)
    );
    assert!(leer(&heredado).contains("metadata_location"));
    // Lo mismo, salvo el segundo del intento: las dos pasadas pueden caer a un lado y otro de un segundo.
    let sin_cuando = |t: String| -> String {
        t.lines()
            .filter(|l| !l.trim_start().starts_with(r#""cuando""#))
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(sin_cuando(leer(&directo)), sin_cuando(leer(&heredado)));
    let _ = std::fs::remove_dir_all(&directo);
    let _ = std::fs::remove_dir_all(&heredado);
}

/// Un `fd:` que no sirve es un fallo del almacén, dicho, no un cuelgue.
#[test]
fn un_fd_que_no_sirve_es_un_fallo_dicho() {
    let dir = arbol("roto");
    for valor in ["fd:3", "fd:9999,9998", "fd:a,b"] {
        let s = Command::new(env!("CARGO_BIN_EXE_ore"))
            .args(["datasets", "."])
            .current_dir(&dir)
            .env("ORE_PUNTEROS", valor)
            .output()
            .unwrap();
        let salida = String::from_utf8_lossy(&s.stderr);
        assert_ne!(s.status.code(), Some(0), "{valor}: {salida}");
        assert!(salida.contains("ORE_PUNTEROS"), "{valor}: {salida}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
