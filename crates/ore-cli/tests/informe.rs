//! `ore report` — el registro de qué gobierna qué, y quién responde.
//!
//! Se prueba por la CLI pública y sin enlazar `ore-core`, como `conformance.rs`
//! y por la misma razón: la especificación exige que la implementación de
//! referencia se ejerza **sin conocimiento privilegiado de sus propias
//! estructuras** (`00-overview` §3.3).
//!
//! Y existe porque el informe se escribió y se ejecutó **a mano**, que es la
//! misma crítica que este repositorio le hace a todo lo demás:
//!
//! > **Una prueba que no corre tiene exactamente el mismo aspecto que una que
//! > pasa.**

use std::path::Path;
use std::process::Command;

fn informe(dir: &str) -> String {
    let raiz = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(dir);
    let s = Command::new(env!("CARGO_BIN_EXE_ore"))
        .arg("report")
        .arg(&raiz)
        .output()
        .expect("no se pudo invocar `ore`");
    assert!(
        s.status.success(),
        "`ore report` falló:\n{}",
        String::from_utf8_lossy(&s.stderr)
    );
    String::from_utf8_lossy(&s.stdout).to_string()
}

#[test]
fn una_fila_por_propiedad_y_clase_exigida_con_quien_responde() {
    let out = informe("vendor/oos/examples/acme-retail");

    // La `constraint` la descarga un `Ruleset`, **y su dueño viaja**. Eso es lo
    // que el informe existe para contestar: no «¿está gobernado?» —eso lo
    // contesta que compile— sino «¿quién responde?».
    assert!(
        out.contains("hr.Employee.nationalId")
            && out.contains("eu.gdpr-minimization (team:compliance)"),
        "falta la atribución con dueño:\n{out}"
    );

    // Y la `authorization` la descarga una política que nombra la propiedad
    // **directamente**. Es la mitad de la proyección que la cobertura no leía, y
    // por la que un paquete gobernado por enumeración no compilaba.
    assert!(
        out.contains("forbid-national-id-egress"),
        "una política que nombra la propiedad tiene que aparecer descargando \
         `authorization`:\n{out}"
    );
}

/// **No lista lo que no exige nada.** De las 40 propiedades clasificadas del
/// ejemplo, 29 no exigen gobierno: un informe que las listara sería el 72% de
/// filas diciendo *«nada que gobernar»*, y el ruido esconde lo que se viene a
/// mirar.
///
/// Lo que exige lo decide `requiresGovernance`, **no** la clasificación: una
/// propiedad `low` está clasificada y no exige nada.
#[test]
fn lo_que_no_exige_gobierno_no_sale() {
    let out = informe("vendor/oos/examples/acme-retail");
    assert!(
        !out.contains("supply.Sku."),
        "`supply.Sku` no exige gobierno y no debería salir:\n{out}"
    );
    assert!(
        out.contains("11 propiedad(es) exigen gobierno"),
        "el recuento tiene que decir cuántas exigen, no cuántas hay:\n{out}"
    );
}

/// **No puede tener filas rojas, y lo dice.** Una propiedad sin la clase que
/// exige su clasificación no compila (`OOS8001`), así que un paquete que llega
/// al informe ya está cubierto entero. Eso es lo que lo separa de un cuadro de
/// mando con un porcentaje.
#[test]
fn el_informe_no_puede_tener_filas_rojas_y_lo_dice() {
    let out = informe("vendor/oos/examples/acme-retail");
    assert!(
        out.contains("si alguna no lo tuviera, esto no habría compilado"),
        "el informe tiene que decir POR QUÉ no puede tener rojas:\n{out}"
    );
}

/// Un paquete donde ninguna clasificación exige nada **no produce una tabla
/// vacía**: dice por qué está vacía. Una tabla sin filas y un modelo sin
/// exigencias tienen el mismo aspecto.
#[test]
fn sin_exigencias_lo_dice_en_vez_de_no_decir_nada() {
    let out = informe("crates/ore-exec/casos/dos-familias");
    assert!(
        out.contains("Ninguna propiedad de este paquete exige gobierno"),
        "una tabla vacía y un modelo sin exigencias no pueden tener el mismo \
         aspecto:\n{out}"
    );
}

/// **De una política de Cedar responde el dueño del `ConduitPolicy`.**
///
/// No es una inferencia cómoda: quien eleva la autorización de un conducto y
/// quien escribe un `permit` toman la misma clase de decisión —*quién ve qué*— y
/// son la misma persona. El ejemplo de referencia lo llevaba escrito **en un
/// comentario** antes de que existiera el campo, y un dueño en prosa no viaja en
/// el bundle.
#[test]
fn una_politica_hereda_el_dueno_de_la_superficie_de_seguridad() {
    let out = informe("vendor/oos/examples/acme-retail");
    assert!(
        !out.contains("sin dueño declarado"),
        "ninguna regla puede quedarse sin quien responda:\n{out}"
    );
    assert!(
        out.contains("forbid-national-id-egress (team:acme-security)"),
        "la política tiene que heredar el dueño del `ConduitPolicy`:\n{out}"
    );
    // Y el `Ruleset` conserva el suyo: son decisiones distintas, de partes
    // distintas, y colapsarlas sería perder justo lo que el informe enseña.
    assert!(
        out.contains("eu.gdpr-minimization (team:compliance)"),
        "el `Ruleset` responde por su cuenta:\n{out}"
    );
}
