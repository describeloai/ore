//! **Una caché escrita bajo otra regla no se distingue de una buena mirándola.**
//!
//! Es el fallo que el [ADR 0006](../../../docs/decisions/0006-el-artefacto-de-topologia.md)
//! nombra al final y que nada comprobaba:
//!
//! > *Refrescar responde a que el dato cambió; reconstruir, a que la REGLA
//! > cambió. Un efecto computado bajo una regla nueva sobre datos enmascarados
//! > con la vieja es la clase de fallo que no tiene aspecto de fallo.*
//!
//! `ore_core::cache` prueba la decisión sobre estructuras. Esto prueba la otra
//! mitad, que es la que se usa: que el digest **no se teclea sino que sale del
//! árbol**, y que el código de salida distingue *«hoy no sirve»* de *«esta caché
//! no vale para esta pregunta»* — porque el segundo no se arregla esperando.
//!
//! Se ejerce por la CLI pública y sin enlazar `ore-core`, como `informe.rs` y por
//! la misma razón: `00-overview` §3.3 exige que la implementación de referencia
//! se pruebe sin conocimiento privilegiado de sus propias estructuras.

use std::path::{Path, PathBuf};
use std::process::Command;

fn ejemplo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("vendor/oos/examples/acme-retail")
}

/// El digest del bundle, por la misma puerta por la que lo daría cualquiera.
fn bundle() -> String {
    let s = Command::new(env!("CARGO_BIN_EXE_ore"))
        .arg("compile")
        .arg(ejemplo())
        .output()
        .expect("no se pudo invocar `ore`");
    assert!(s.status.success(), "`ore compile` falló");
    let salida = String::from_utf8_lossy(&s.stdout).to_string();
    let (_, resto) = salida.split_once("\"bundle\": \"").expect("hay bundle");
    resto.split('"').next().expect("cierra").to_string()
}

fn manifiesto(dir: &Path, bundle: &str, marca: &str) -> PathBuf {
    let ruta = dir.join("cache.json");
    std::fs::write(
        &ruta,
        format!(
            r#"{{"oreCache":1,"entries":[{{"bundle":"{bundle}","entity":"hr.Employee",
               "properties":["employeeId","nationalId"],"table":"lago.cache.hr_employee","datasource":"lago",
               "watermark":"{marca}"}}]}}"#
        ),
    )
    .expect("se escribe");
    ruta
}

/// `(codigo, stdout)`.
fn check(manifiesto: &Path, extra: &[&str]) -> (i32, String) {
    let s = Command::new(env!("CARGO_BIN_EXE_ore"))
        .args(["cache", "check", "--manifest"])
        .arg(manifiesto)
        .args(["--entity", "hr.Employee", "--props", "employeeId"])
        .args(extra)
        .arg(ejemplo())
        .output()
        .expect("no se pudo invocar `ore`");
    (
        s.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&s.stdout).to_string(),
    )
}

fn temporal(nombre: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ore-cache-{}-{nombre}", std::process::id()));
    std::fs::create_dir_all(&d).expect("se crea");
    d
}

/// La línea de base: sin ella, un fallo del binario pasaría por un veredicto.
#[test]
fn bajo_el_mismo_bundle_y_sin_sla_sirve() {
    let d = temporal("sirve");
    let m = manifiesto(&d, &bundle(), "2026-08-31T10:00:00Z");
    let (codigo, salida) = check(&m, &[]);
    assert_eq!(codigo, 0, "{salida}");
    assert!(
        salida.contains("sirve · lago.cache.hr_employee"),
        "{salida}"
    );
}

/// **El caso.** El manifiesto dice un bundle que no es el del árbol, así que las
/// filas se escribieron bajo una clasificación que ya no rige — y la tabla tiene
/// exactamente el mismo aspecto que antes.
///
/// Dos cosas se comprueban aquí, y la segunda es la que salva a alguien: que se
/// detecta, y que **el consejo no es refrescar**. Refrescar reescribe las filas
/// bajo la misma pregunta, y lo que cambió es la pregunta.
#[test]
fn una_cache_de_otro_bundle_no_sirve_y_no_se_arregla_refrescando() {
    let d = temporal("regla");
    let m = manifiesto(
        &d,
        "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        "2026-08-31T10:00:00Z",
    );
    let (codigo, salida) = check(&m, &[]);
    assert_eq!(codigo, 65, "{salida}");
    assert!(salida.contains("regla distinta"), "{salida}");
    assert!(salida.contains("reconstruir"), "{salida}");
    assert!(
        !salida.contains("remedio: refrescar"),
        "el consejo equivocado:\n{salida}"
    );
}

/// Y el código de salida separa lo que el texto separa. Un guion que colapse los
/// dos casos tendría que leer el mensaje para saber si hay que reconstruir.
#[test]
fn rancia_y_regla_distinta_no_salen_por_el_mismo_codigo() {
    let d = temporal("codigos");
    let m = manifiesto(&d, &bundle(), "2026-08-31T10:00:00Z");
    let (rancia, salida) = check(&m, &["--at", "2026-08-31T12:00:00Z", "--sla", "1h"]);
    assert_eq!(rancia, 1, "{salida}");
    assert!(salida.contains("rancia"), "{salida}");
    assert!(salida.contains("remedio: refrescar"), "{salida}");
}

/// El digest **no se teclea**: sale del árbol. Es lo que impide que quien
/// pregunta conteste por su cuenta la única pregunta que la caché no puede
/// contestar sola.
#[test]
fn el_bundle_no_es_una_bandera() {
    let s = Command::new(env!("CARGO_BIN_EXE_ore"))
        .args(["cache", "check", "--help"])
        .output()
        .expect("no se pudo invocar `ore`");
    let ayuda = String::from_utf8_lossy(&s.stdout).to_string();
    assert!(!ayuda.contains("--bundle"), "{ayuda}");
    assert!(ayuda.contains("--manifest"), "{ayuda}");
}
