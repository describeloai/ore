//! ⭐ **LA PUERTA HEREDADA** (0079 §19.2 P1, ·5c·3): `ore` recibe la puerta de los
//! punteros de ore-serve, sin proceso de más ni conexión por llamada.
//!
//! Medido en ·5c P1: conectar a `catalogo-db` cuesta p50 24 ms · p99 66, la
//! mitad de un `loadTable` (~110 ms). Así que `ore` no conecta: ore-serve lo
//! lanza con **dos tuberías heredadas** y le sirve el protocolo de
//! `ore_core::punteros::protocolo` desde su lado, con su puerta (y, con el
//! catálogo, su pool).
//!
//! # Cómo
//!
//! ```text
//!   ore-serve                                   ore (hijo)
//!   ─────────                                   ──────────
//!   hilo `ore-punteros`:                        ORE_PUNTEROS=fd:<lee>,<escribe>
//!     servir(peticiones ← , → respuestas)       ORE_REF=<rama> (si no es main)
//!        ▲ lee_padre        escribe_padre │        │ escribe        lee ▲
//!        └──── tubería 1 (peticiones) ◄───┼────────┘                    │
//!                                         └─── tubería 2 (respuestas) ──┘
//! ```
//!
//! - **Los descriptores:** las dos tuberías nacen con `O_CLOEXEC` (`std::io::pipe`),
//!   así que ningún otro hijo que se lance a la vez las hereda. Sólo en ESTE
//!   hijo, entre `fork` y `exec` (`pre_exec`), se les quita la marca a los dos
//!   extremos del hijo. Van con su número de siempre —el que tienen en
//!   ore-serve, que el `fork` conserva— y no en 3 y 4 fijos: mover un
//!   descriptor a 3 podría pisar el que `std` usa para contar un `exec` fallido.
//!   `ore` vuelve a ponerles `O_CLOEXEC` al recibirlos (`ore_cli::puerta`): sus
//!   propios hijos (`ore-store-<gcs>`) no se los llevan.
//! - **El hilo:** uno por `ore`, con un `Servidor` cuyo abridor es
//!   [`crate::lugar::heredada`] para el directorio de `ore` y **la ref de la
//!   costura** ([`crate::lugar::ref_en_curso`], tomada en el hilo de la
//!   petición antes de lanzar). Si `ore` no abre la puerta (un verbo sin
//!   punteros), el hilo sólo ve el fin de la tubería.
//! - **El final:** ore-serve suelta sus copias de los extremos del hijo en
//!   cuanto `ore` termina; el hilo lee el fin de la tubería y acaba, y se le
//!   espera (hasta [`ESPERA`]: si un nieto se quedara con un extremo —no debe—,
//!   el hilo se suelta y lo dice por stderr, sin colgar la petición).
//! - **Si `ore` muere a mitad:** lo que estuviera escribiendo la puerta acaba
//!   entero o no empieza (es atómica); la respuesta no llega a nadie (`EPIPE`,
//!   `SIGPIPE` lo ignora Rust) y el hilo acaba. `ore` sale con su código, y la
//!   ruta contesta como contesta hoy a un `ore` que falla.
//! - **Si la puerta no abre** (el catálogo no contesta): el `abrir` de `ore`
//!   recibe el fallo (503) y `ore` lo dice como cualquier fallo del almacén.
//!
//! Sólo en unix: en Windows (el puesto de quien programa) `mando` lanza `ore`
//! como siempre, y el catálogo no existe ahí.

use std::io::BufReader;
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use ore_core::punteros::protocolo::{self, Apertura, Servidor, VARIABLE};

use crate::mando::{Negado, Salida};

/// Lo que se espera al hilo tras acabar `ore`.
pub(crate) const ESPERA: Duration = Duration::from_secs(2);

/// La variable de la ref que `ore` le pasa en su `abrir` (`ore_cli::puerta`).
pub(crate) const VARIABLE_REF: &str = "ORE_REF";

mod c {
    use std::ffi::c_int;
    pub(super) const F_GETFD: c_int = 1;
    pub(super) const F_SETFD: c_int = 2;
    pub(super) const FD_CLOEXEC: c_int = 1;
    unsafe extern "C" {
        /// `fcntl(2)`, de la libc con la que `std` ya enlaza (sin crate `libc`).
        pub(super) fn fcntl(fd: c_int, cmd: c_int, ...) -> c_int;
    }
}

/// Le quita `FD_CLOEXEC` a `fd`: que pase el `exec`.
///
/// Corre entre `fork` y `exec`: sólo `fcntl`, que es seguro ahí
/// (*async-signal-safe*), y nada que reserve memoria.
fn heredable(fd: i32) -> std::io::Result<()> {
    // SAFETY: `fcntl` sobre un descriptor abierto que es nuestro; F_GETFD y
    // F_SETFD no tocan memoria.
    unsafe {
        let marcas = c::fcntl(fd, c::F_GETFD);
        if marcas < 0 || c::fcntl(fd, c::F_SETFD, marcas & !c::FD_CLOEXEC) < 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

/// **Corre `binario` en `raiz` con la puerta heredada.** Lo mismo que
/// `mando::correr_con` (que ya comprobó la lista), con las dos tuberías y el
/// hilo que sirve. Ver el módulo.
pub(crate) fn correr(
    binario: &Path,
    raiz: &Path,
    args: &[String],
    entorno: &[(String, String)],
) -> Result<Salida, Negado> {
    let imposible = |e: std::io::Error| {
        Negado::ArgumentoImposible(format!("no se pudo abrir la puerta heredada: {e}"))
    };
    let rama = crate::lugar::ref_en_curso();
    // ·5c·5c·2: si corre en una definición, lo que `ore` escriba se graba en su cinta.
    let cinta = crate::lugar::cinta_en_curso();
    // Las peticiones: escribe `ore`, lee ore-serve. Las respuestas, al revés.
    let (lee_padre, escribe_hijo) = std::io::pipe().map_err(imposible)?;
    let (lee_hijo, escribe_padre) = std::io::pipe().map_err(imposible)?;
    let (lee, escribe) = (lee_hijo.as_raw_fd(), escribe_hijo.as_raw_fd());

    let mut orden = Command::new(binario);
    orden
        .args(args)
        .envs(entorno.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .current_dir(raiz)
        .env(VARIABLE, format!("fd:{lee},{escribe}"));
    match &rama {
        Some(r) => orden.env(VARIABLE_REF, r),
        None => orden.env_remove(VARIABLE_REF),
    };
    // SAFETY: el cierre sólo llama a `fcntl` sobre dos enteros copiados.
    unsafe {
        orden.pre_exec(move || {
            heredable(lee)?;
            heredable(escribe)
        });
    }

    let cwd = raiz.to_path_buf();
    let (acabado, al_acabar) = std::sync::mpsc::channel::<()>();
    let hilo = std::thread::Builder::new()
        .name("ore-punteros".into())
        .spawn(move || {
            let mut s = Servidor::nuevo(Box::new(move |a: &Apertura| {
                crate::lugar::heredada(a, &cwd, rama.as_deref(), cinta.as_ref())
            }));
            let r = protocolo::servir(BufReader::new(lee_padre), escribe_padre, &mut s);
            let _ = acabado.send(());
            r
        })
        .map_err(imposible)?;

    let t = std::time::Instant::now();
    let salida = orden.output();
    crate::fases::ore(t.elapsed());
    // `ore` acabó (o no arrancó): sus extremos, fuera, y el hilo ve el final.
    drop(lee_hijo);
    drop(escribe_hijo);
    match al_acabar.recv_timeout(ESPERA) {
        Ok(()) => {
            let _ = hilo.join();
        }
        Err(_) => eprintln!(
            "ore-serve · la puerta heredada de `{}` no acabó en {ESPERA:?}: se suelta su hilo",
            binario.display()
        ),
    }
    let salida = salida
        .map_err(|e| Negado::ArgumentoImposible(format!("no se pudo ejecutar `ore`: {e}")))?;
    Ok(Salida {
        codigo: salida.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&salida.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&salida.stderr).into_owned(),
    })
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use std::path::PathBuf;

    fn tmp(nombre: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "ore-serve-heredada-{nombre}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(d.join("datasets/ventas/default")).unwrap();
        std::fs::write(
            d.join("datasets/ventas/default/pedidos.json"),
            "{\"estado\": \"copiada\", \"vista\": \"ventas.pedidos\"}\n",
        )
        .unwrap();
        d
    }

    fn sh(raiz: &Path, guion: &str) -> Salida {
        correr(
            Path::new("/bin/bash"),
            raiz,
            &["-c".into(), guion.into()],
            &[],
        )
        .unwrap()
    }

    /// Sin `ore`: un `bash` (`dash` no redirige a descriptores de dos cifras) que habla el protocolo a mano por los descriptores
    /// que dice `ORE_PUNTEROS`. Abre, lee un puntero y escribe otro.
    #[test]
    fn un_hijo_habla_por_las_tuberias_que_hereda() {
        let d = tmp("sh");
        let s = sh(
            &d,
            r#"set -e
L=${ORE_PUNTEROS#fd:}; R=${L%,*}; W=${L#*,}
pide() { eval "printf '%s\n' \"\$1\" >&$W"; eval "read -r linea <&$R"; printf '%s\n' "$linea"; }
pide '{"op":"abrir","raiz":".","carpeta":null,"ajenos":null,"ref":null}'
pide '{"op":"uno","nombre":"ventas.pedidos"}'
pide '{"op":"escribir","cambios":[{"nombre":"ventas.otro","que":{"poner":{"campos":[["estado","copiada"]],"quitar":[]}},"esperado":"nada"}],"contexto":{"autor":"ana","operacion":"copia","idempotencia":null}}'
pide '{"op":"escribir","cambios":[{"nombre":"ventas.otro","que":"retirar","esperado":"nada"}],"contexto":{"autor":"ana","operacion":"copia","idempotencia":null}}'
echo "ref=${ORE_REF:-}"
"#,
        );
        assert!(s.bien(), "{}{}", s.stdout, s.stderr);
        let lineas: Vec<&str> = s.stdout.lines().collect();
        assert_eq!(lineas[0], r#"{"ok":true}"#);
        assert!(lineas[1].contains(r#""de":"aqui""#), "{}", lineas[1]);
        assert!(lineas[1].contains("copiada"), "{}", lineas[1]);
        assert_eq!(lineas[2], r#"{"ok":true}"#);
        assert!(
            lineas[3].contains(r#""codigo":409"#),
            "el CAS cruza: {}",
            lineas[3]
        );
        assert_eq!(lineas[4], "ref=", "en `main`, sin `ORE_REF`");
        assert!(d.join("datasets/ventas/default/otro.json").is_file());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// La ref de la costura llega al hijo; un hijo que no abre la puerta acaba
    /// igual (el hilo ve el final); y uno que muere a mitad no cuelga a nadie.
    #[test]
    fn la_ref_llega_y_el_hilo_acaba_aunque_el_hijo_no_hable_o_muera() {
        let d = tmp("ref");
        {
            let _en = crate::lugar::EnRef::poner(Some("propuesta-1"));
            let s = sh(&d, "echo \"ref=$ORE_REF\"");
            assert_eq!(s.stdout.trim(), "ref=propuesta-1", "{}", s.stderr);
        }
        let t = std::time::Instant::now();
        let s = sh(
            &d,
            r#"L=${ORE_PUNTEROS#fd:}; W=${L#*,}
eval "echo '{\"op\":\"abrir\",\"raiz\":\".\"}' >&$W"; exit 3"#,
        );
        assert_eq!(s.codigo, 3);
        assert!(
            t.elapsed() < ESPERA,
            "el hilo acabó sin esperar: {:?}",
            t.elapsed()
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Lo que no arranca es un error, como en `mando`, y no deja hilos colgados.
    #[test]
    fn un_binario_que_no_existe_es_un_error() {
        let d = tmp("nada");
        let r = correr(Path::new("/no/existe/ore"), &d, &[], &[]);
        assert!(matches!(r, Err(Negado::ArgumentoImposible(_))));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// El `ore` de este target, si está compilado (`cargo build -p ore-cli`).
    fn ore() -> Option<PathBuf> {
        let p = std::env::current_exe()
            .ok()?
            .parent()?
            .parent()?
            .join("ore");
        p.is_file().then_some(p)
    }

    /// ⭐ **(a) Un `ore` de verdad, por la puerta heredada y directo, ve y escribe
    /// lo mismo** (con `Ficheros` detrás): el listado, la ficha, y una pasada
    /// que escribe un puntero (`materialize --vista` sobre una fuente cerrada:
    /// el `error` que conserva la copia).
    #[test]
    fn un_ore_de_verdad_lee_y_escribe_igual_por_la_puerta_heredada() {
        let Some(ore) = ore() else {
            eprintln!("(sin `ore` compilado en este target: `cargo build -p ore-cli` y otra vez)");
            return;
        };
        let caso = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../vendor/oos/conformance/v1alpha12/valid/a-copy-with-its-plan/input");
        let puntero = r#"{
  "dataset": "catalogo/hr/empleados",
  "estado": "copiada",
  "huella": "sha256:0000",
  "metadata_location": "gs://copia/ore/v2/catalogo/hr/empleados/metadata/00000-x.metadata.json",
  "snapshot": "1",
  "vista": "hr.empleados"
}
"#;
        let preparar = |n: &str| {
            let d = tmp(n);
            copiar(&caso, &d);
            let p = d.join("datasets/hr/default/empleados.json");
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, puntero).unwrap();
            d
        };
        let (directo, heredado) = (preparar("directo"), preparar("heredado"));
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        for args in [
            v(&["datasets", "."]),
            v(&["datasets", ".", "--ficha", "hr.empleados"]),
            v(&["datasets", ".", "--ficha", "hr.nadie"]),
            v(&[
                "materialize",
                ".",
                "--informe",
                "datasets",
                "--vista",
                "hr.empleados",
            ]),
            v(&["datasets", ".", "--informe", "datasets"]),
        ] {
            let a = directo_sin_puerta(&ore, &directo, &args);
            let b = correr(&ore, &heredado, &args, &[]).unwrap();
            let quitar = |s: &str, d: &Path| s.replace(&d.to_string_lossy().into_owned(), "<d>");
            assert_eq!(a.codigo, b.codigo, "{args:?}\n{}\n{}", a.stderr, b.stderr);
            assert_eq!(
                quitar(&a.stdout, &directo),
                quitar(&b.stdout, &heredado),
                "{args:?}"
            );
        }
        let escrito = |d: &Path| {
            std::fs::read_to_string(d.join("datasets/hr/default/empleados.json")).unwrap()
        };
        assert!(
            escrito(&heredado).contains(r#""estado": "error""#),
            "{}",
            escrito(&heredado)
        );
        assert_eq!(escrito(&directo), escrito(&heredado));
        let _ = std::fs::remove_dir_all(&directo);
        let _ = std::fs::remove_dir_all(&heredado);
    }

    /// `ore` como lo lanza `mando` sin la puerta heredada (sin su lista: aquí
    /// también corre `materialize`).
    fn directo_sin_puerta(ore: &Path, raiz: &Path, args: &[String]) -> Salida {
        let s = Command::new(ore)
            .args(args)
            .current_dir(raiz)
            .env_remove(VARIABLE)
            .output()
            .unwrap();
        Salida {
            codigo: s.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&s.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&s.stderr).into_owned(),
        }
    }

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

    /// **La medida** (·5c·3): `ore datasets . --cargar` 100 veces, directo y por
    /// la puerta heredada. `cargo test -p ore-serve --bin ore-serve -- --ignored
    /// --nocapture lo_que_cuesta`.
    #[test]
    #[ignore]
    fn lo_que_cuesta_la_puerta_heredada() {
        let Some(ore) = ore() else {
            panic!("sin `ore` compilado en este target");
        };
        let d = tmp("medida");
        for i in 0..150 {
            std::fs::write(
                d.join(format!("datasets/ventas/default/t{i}.json")),
                format!("{{\"estado\": \"copiada\", \"vista\": \"ventas.t{i}\"}}\n"),
            )
            .unwrap();
        }
        let veces = 100;
        for args in [
            vec![
                "datasets".to_string(),
                ".".into(),
                "--cargar".into(),
                "ventas.t7".into(),
            ],
            vec!["datasets".to_string(), ".".into()],
        ] {
            let medir = |heredada: bool| {
                let mut ms: Vec<f64> = (0..veces)
                    .map(|_| {
                        let t = std::time::Instant::now();
                        let s = if heredada {
                            correr(&ore, &d, &args, &[])
                        } else {
                            crate::mando::correr_con(&ore, &d, &args, &[])
                        }
                        .unwrap();
                        let e = t.elapsed().as_secs_f64() * 1000.0;
                        std::hint::black_box(s.codigo);
                        e
                    })
                    .collect();
                ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
                (ms[veces / 2], ms[veces * 9 / 10], ms[veces * 99 / 100])
            };
            let _ = medir(false); // en caliente
            let (d50, d90, d99) = medir(false);
            let (h50, h90, h99) = medir(true);
            eprintln!(
                "{args:?} ×{veces}: directo p50 {d50:.2} p90 {d90:.2} p99 {d99:.2} ms · \
                 heredada p50 {h50:.2} p90 {h90:.2} p99 {h99:.2} ms"
            );
        }
        let _ = std::fs::remove_dir_all(&d);
    }
}
