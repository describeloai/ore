//! Las ontologías de referencia de `vendor/oos/examples/` validan sin un solo
//! diagnóstico.
//!
//! # Por qué existe este test
//!
//! Un ejemplo es documentación ejecutable, y la documentación que nadie ejecuta
//! deriva. Estos dos ficheros son lo primero que lee alguien que llega a OOS:
//! si ilustran una gramática que la especificación no define —o que define de
//! otra forma— enseñan mal, y nadie se entera hasta que un tercero intenta
//! escribir su propia ontología copiándolos.
//!
//! Que el ejemplo esté en verde no es cosmética: es la única prueba de que la
//! especificación, la implementación y lo que se le enseña a un recién llegado
//! dicen lo mismo.
//!
//! # Como consumidor externo, igual que la suite
//!
//! Este runner invoca el binario `ore` por su CLI pública y **no enlaza contra
//! `ore-core`**, por la misma razón que `conformance.rs`: la especificación
//! exige que la implementación de referencia se ejerza sin conocimiento
//! privilegiado de sus propias estructuras (`00-overview.md` §3.3). Un ejemplo
//! validado por una API interna no demostraría que un tercero puede validarlo.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Los directorios de primer nivel bajo `examples/`. Se descubren, no se
/// enumeran: un ejemplo nuevo entra en CI por existir, que es exactamente la
/// propiedad que se quiere.
fn ontologias(raiz: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(raiz)
        .expect("no se puede leer vendor/oos/examples")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    v.sort();
    v
}

#[test]
fn los_ejemplos_validan() {
    let raiz = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/oos/examples")
        .canonicalize()
        .expect(
            "no se encuentra vendor/oos/examples — \
             ejecuta `git submodule update --init`",
        );

    let ontologias = ontologias(&raiz);
    assert!(
        !ontologias.is_empty(),
        "no hay ninguna ontología de ejemplo en vendor/oos/examples"
    );

    let ore = env!("CARGO_BIN_EXE_ore");
    let mut fallos = Vec::new();

    for dir in &ontologias {
        let nombre = dir.file_name().unwrap().to_string_lossy().to_string();
        let salida = Command::new(ore)
            .arg("validate")
            .arg(dir)
            .output()
            .unwrap_or_else(|e| panic!("no se pudo invocar `ore`: {e}"));

        let texto = format!(
            "{}{}",
            String::from_utf8_lossy(&salida.stdout),
            String::from_utf8_lossy(&salida.stderr)
        );

        // El criterio es CERO diagnósticos, no «el proceso terminó bien». Un
        // aviso es una divergencia igual que un error: si el ejemplo necesita
        // que se le perdone algo, ya no ilustra la especificación.
        let diagnosticos = texto
            .lines()
            .filter(|l| l.starts_with("error[") || l.starts_with("warning["))
            .count();

        if diagnosticos > 0 || !salida.status.success() {
            fallos.push((nombre, texto));
        }
    }

    if !fallos.is_empty() {
        let mut msg = String::from("\n");
        for (nombre, texto) in &fallos {
            msg.push_str(&format!("── examples/{nombre} ──\n{texto}\n"));
        }
        msg.push_str(&format!(
            "\n{} de {} ontologías de ejemplo no validan.\n\
             Un ejemplo que no valida enseña una gramática que no existe.\n",
            fallos.len(),
            ontologias.len()
        ));
        panic!("{msg}");
    }
}
