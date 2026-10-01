//! Las **instantáneas**: lo que `ore-code` dice de cada fichero del corpus —el
//! documento que genera, o cada fallo con su línea, su columna y su ayuda—,
//! comprometido en `corpus/instantaneas/`.
//!
//! Un cambio de mensaje, de sitio o de documento sale en el diff de la
//! revisión, no en producción. Para aceptar el cambio:
//! `ORE_CODE_ACTUALIZAR=1 cargo test -p ore-code --test instantaneas`.

use ore_code::lineas::Lineas;
use ore_code::{Fallo, emitir, python};
use std::fmt::Write;
use std::path::Path;

fn corpus() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/corpus"))
}

fn fallo(s: &mut String, l: &Lineas, f: &Fallo) {
    let (linea, columna) = l.de(f.rango);
    let _ = writeln!(s, "  {linea}:{columna} {}", f.mensaje);
    if let Some(a) = &f.ayuda {
        let _ = writeln!(s, "      ayuda: {a}");
    }
}

fn instantanea(texto: &str, ruta: &str) -> String {
    let d = python::derivar(texto, ruta);
    let l = Lineas::new(texto);
    let mut s = String::new();
    for (titulo, fs) in [
        ("sintaxis", &d.sintaxis),
        ("versión", &d.version),
        ("avisos", &d.avisos),
    ] {
        if !fs.is_empty() {
            let _ = writeln!(s, "── {titulo}");
            fs.iter().for_each(|f| fallo(&mut s, &l, f));
        }
    }
    for f in &d.funciones {
        match &f.resultado {
            Ok(x) => {
                let _ = writeln!(s, "── {} → {}", f.nombre, emitir::ruta_del_documento(x));
                s.push_str(&emitir::documento(x, "ventas"));
            }
            Err(fs) => {
                let _ = writeln!(s, "── {} no se deriva (OOS2043)", f.nombre);
                fs.iter().for_each(|x| fallo(&mut s, &l, x));
            }
        }
    }
    s
}

#[test]
fn cada_fichero_dice_lo_mismo_que_su_instantanea() {
    let actualizar = std::env::var_os("ORE_CODE_ACTUALIZAR").is_some();
    let dir = corpus().join("instantaneas");
    std::fs::create_dir_all(&dir).unwrap();
    let mut distintas = Vec::new();
    let mut fuentes: Vec<_> = std::fs::read_dir(corpus().join("funciones"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    fuentes.sort();
    for p in fuentes {
        let nombre = p.file_name().unwrap().to_str().unwrap();
        let dicho = instantanea(
            &std::fs::read_to_string(&p).unwrap(),
            &format!("funciones/{nombre}"),
        );
        let fichero = dir.join(nombre.replace(".py", ".txt"));
        let guardado = std::fs::read_to_string(&fichero)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        if dicho != guardado {
            if actualizar {
                std::fs::write(&fichero, &dicho).unwrap();
            } else {
                distintas.push(format!("── {nombre}\n{dicho}"));
            }
        }
    }
    assert!(
        distintas.is_empty(),
        "lo que ore-code dice ha cambiado (ORE_CODE_ACTUALIZAR=1 para aceptarlo):\n{}",
        distintas.join("\n")
    );
}

#[test]
fn generar_dos_veces_da_los_mismos_bytes() {
    for e in std::fs::read_dir(corpus().join("funciones")).unwrap() {
        let p = e.unwrap().path();
        let t = std::fs::read_to_string(&p).unwrap();
        assert_eq!(
            instantanea(&t, "f.py"),
            instantanea(&t, "f.py"),
            "{}",
            p.display()
        );
    }
}
