//! **El registro de copias**, por la CLI pública.
//!
//! Lo que se afirma aquí no es que el `FilterTree` funcione —eso lo prueban las
//! 143 pruebas de `ore-view` desde M5— sino que **hay un sitio**: que las copias
//! que un paquete tiene dejaron de ser invisibles, vengan de donde vengan.
//!
//! # La afirmación que importa
//!
//! La topología era una vista materializada escrita a mano en el paradigma
//! anterior: `ore-exec` la construye por su cuenta, la refresca con marca de
//! agua propia y nadie la llama copia. Aquí aparece **en el mismo registro y
//! con las mismas tres caras** que una `materialized`, y su ruta de refresco
//! aparece **al lado y por separado**: *estar registrada y estar mantenida son
//! dos cosas*.
//!
//! El inventario de mecanismos —cuántos hay y por qué existe cada uno— lo
//! guarda `registro.rs` en una prueba propia, porque es una afirmación sobre el
//! árbol y no sobre un paquete.

use std::path::{Path, PathBuf};
use std::process::Command;

fn conformidad8(nombre: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/oos/conformance/v1alpha8")
        .join(nombre)
        .join("input")
}

fn ejemplo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/oos/examples/acme-retail")
}

fn ver(dir: &Path) -> (bool, String) {
    let s = Command::new(env!("CARGO_BIN_EXE_ore"))
        .arg("view")
        .arg(dir)
        .output()
        .expect("no se pudo invocar `ore`");
    (
        s.status.success(),
        String::from_utf8_lossy(&s.stdout).into(),
    )
}

/// El bloque del registro va **por paquete** y no por vista, y no es un detalle
/// de presentación: una copia puede contestar la consulta de otra vista, así que
/// la unidad en la que tiene sentido preguntarse *«qué hay ya calculado»* es el
/// paquete entero.
#[test]
fn una_vista_materializada_entra_en_el_registro_con_sus_tres_caras() {
    let (ok, out) = ver(&conformidad8("valid/append-changes-back-an-event"));
    assert!(ok, "el caso es válido:\n{out}");

    assert!(out.contains("registro · 1 copia · nadie 1"), "{out}");
    assert!(out.contains("  app.clics"), "{out}");
    // Las tres caras: qué contesta, dónde vive, hasta cuándo fue cierta.
    assert!(out.contains("    plan      sha256:"), "el plan:\n{out}");
    assert!(
        out.contains("    destino   lago·cache.clics"),
        "el destino:\n{out}"
    );
    assert!(
        out.contains("    testigo   sin poblar"),
        "el testigo, vacío y dicho:\n{out}"
    );
    // Y la cuarta cosa, que no es de la copia: quién la refresca.
    assert!(out.contains("    refresco  nadie —"), "{out}");
}

/// **El plan del registro es el mismo plan de la vista.** No una reconstrucción
/// parecida: el mismo digest que `ore view` imprime arriba.
///
/// Si divergieran, el View Matcher razonaría sobre un plan que no es el que la
/// vista define, y ofrecería la copia para una consulta que no contesta.
#[test]
fn el_plan_registrado_es_el_mismo_que_el_de_la_vista() {
    let (_, out) = ver(&conformidad8("valid/append-changes-back-an-event"));
    // Solo el digest: la línea de la vista lleva además cuántas se encadenaron.
    let digests: Vec<&str> = out
        .lines()
        .filter(|l| l.trim_start().starts_with("plan      sha256:"))
        .filter_map(|l| l.split_whitespace().find(|w| w.starts_with("sha256:")))
        .collect();
    assert_eq!(digests.len(), 2, "la vista y su copia:\n{out}");
    assert_eq!(
        digests[0], digests[1],
        "el registro guarda OTRO plan que el de la vista:\n{out}"
    );
}

/// **La topología es una copia, y ahora se ve.**
///
/// `acme-retail` no declara ni una `materialized`, y aun así tiene cuatro copias:
/// una por cada relación con `via` de una entidad con clave simple. Las construye
/// `ore-exec` desde siempre; lo nuevo es que estén registradas.
///
/// Y se llega a ellas **por el sustrato**: la fuente física de una entidad es la
/// raíz de la vista que la respalda. Ni un binding de por medio, que es lo que
/// permite que esto siga valiendo con la gramática de v1alpha8.
#[test]
fn la_topologia_entra_en_el_mismo_registro_y_con_su_ruta_aparte() {
    let (ok, out) = ver(&ejemplo());
    assert!(ok, "{out}");

    assert!(
        out.contains("registro · 4 copias · nadie 0 · índice de topología 4"),
        "{out}"
    );
    for esperada in [
        "  hr.Employee.manager",
        "  hr.Employee.department",
        "  supply.Shipment.supplier",
        "  supply.Shipment.sku",
    ] {
        assert!(out.contains(esperada), "falta `{esperada}`:\n{out}");
    }
    // El destino nombra su formato, porque el formato es del destino: el
    // registro no sabe qué es un CSR firmado.
    assert!(
        out.contains("    destino   oretopo·hr.Employee.manager"),
        "{out}"
    );
    // Registrada, y mantenida por otro. Las dos cosas dichas a la vez.
    assert_eq!(
        out.matches("    refresco  índice de topología").count(),
        4,
        "{out}"
    );
    assert!(
        out.contains("`ore index refresh`, con marca de agua propia"),
        "dice por qué tiene ruta propia:\n{out}"
    );
}

/// Un paquete sin copias lo dice, en vez de no decir nada. La diferencia importa:
/// *«no hay copias»* y *«no miré»* se leen igual cuando la salida está vacía.
#[test]
fn un_paquete_sin_copias_lo_dice() {
    let (ok, out) = ver(&conformidad8("valid/view-over-table"));
    assert!(ok, "{out}");
    assert!(
        out.contains("registro · 0 copias · nadie 0 · índice de topología 0"),
        "{out}"
    );
    assert!(
        out.contains("ninguna · el paquete no declara ninguna materialización"),
        "{out}"
    );
}
