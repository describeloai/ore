//! Las ontologías de referencia de `vendor/oos/examples/` validan sin un solo
//! diagnóstico.
//!
//! # La regla
//!
//! **Todo lo que entra en `examples/` valida.** No hay excepciones, listas de
//! exclusión ni ejemplos «casi». Un directorio nuevo bajo `examples/` entra en
//! esta comprobación por existir, y si no valida, este test se pone rojo.
//!
//! El corolario importa tanto como la regla: una ontología escrita contra un
//! lenguaje que la especificación **no** define —construcciones aplazadas,
//! vocabulario anterior a que los perfiles se fijaran— no se arregla ni se
//! exceptúa. **No va en `examples/`.** Su sitio es `docs/vision/`, que es no
//! normativo y que este test no recorre, y allí dice de sí misma que no valida.
//! `docs/vision/acme-global` es exactamente ese caso.
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

/// Y el artefacto generado que el ejemplo compromete tiene que ser **el que hoy
/// emite el motor**, byte a byte.
///
/// `OOS2013` ya vigila que el esquema comprometido conozca cada nivel de cada
/// retículo, y su razonamiento para no comparar texto es correcto: dos
/// implementaciones pueden formatear distinto. Pero eso cubre **una** dimensión
/// del artefacto, y el artefacto tiene muchas.
///
/// Se midió al cerrar el contexto en `purpose`: el esquema comprometido del
/// ejemplo llevaba **una decisión entera de retraso** —`context: { purpose:
/// String }` no estaba— y nada se puso rojo, porque los niveles seguían todos
/// ahí.
///
/// > **Un artefacto generado que está obsoleto tiene exactamente el mismo
/// > aspecto que uno al día.**
///
/// Aquí sí se comparan bytes, y se puede: no es una afirmación de conformidad
/// sobre implementaciones ajenas, es este repositorio comprobando que lo que
/// enseña es lo que produce. El fichero lo dice en su primera línea — NO EDITAR.
#[test]
fn lo_generado_en_los_ejemplos_esta_al_dia() {
    let raiz = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/oos/examples")
        .canonicalize()
        .expect("no se encuentra vendor/oos/examples");
    let ore = env!("CARGO_BIN_EXE_ore");
    let mut fallos = Vec::new();

    for dir in &ontologias(&raiz) {
        for fichero in std::fs::read_dir(dir.join("policies")).into_iter().flatten().flatten() {
            let p = fichero.path();
            if p.extension().and_then(|e| e.to_str()) != Some("cedarschema") {
                continue;
            }
            let comprometido = std::fs::read_to_string(&p).unwrap_or_default();
            let salida = Command::new(ore)
                .args(["export"])
                .arg(dir)
                .args(["--format", "cedarschema"])
                .output()
                .expect("no se pudo invocar `ore`");
            let emitido = String::from_utf8_lossy(&salida.stdout).replace("\r\n", "\n");
            if comprometido.replace("\r\n", "\n") != emitido {
                fallos.push(p.display().to_string());
            }
        }
    }

    assert!(
        fallos.is_empty(),
        "el artefacto generado que el ejemplo compromete no es el que el motor emite \
         hoy:\n  {}\n\nRegenéralo con `ore export <dir> --format cedarschema`. Un \
         esquema obsoleto no falla: deja de casar, y el dato queda sin gobernar.",
        fallos.join("\n  ")
    );
}
