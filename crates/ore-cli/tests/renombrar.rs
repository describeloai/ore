//! `moved` y `reserved` en sus dos alcances nuevos.
//!
//! Los tres casos de conformidad cubren lo normativo: el renombrado anunciado
//! de un documento, el campo que desaparece sin anuncio y el nombre de
//! documento que vuelve. Lo que no cubren son las dos simetrías que hacen que
//! esto sea **un** mecanismo y no tres reglas parecidas:
//!
//! 1. `OOS2006` vale igual en los tres alcances, y **reservar lo que nunca
//!    existió es legal en los tres** — reservar mira hacia delante;
//! 2. un campo **anunciado** no cuenta como supresión, igual que una propiedad
//!    anunciada no lo contaba desde v1alpha1.
//!
//! Por la CLI pública, como el resto.

use std::path::{Path, PathBuf};
use std::process::Command;

fn escribir(dir: &Path, ficheros: &[(&str, &str)]) {
    for (rel, txt) in ficheros {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, txt).unwrap();
    }
}

fn arbol(etiqueta: &str, ficheros: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ore-{etiqueta}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    escribir(&dir, ficheros);
    dir
}

fn correr(verbo: &str, args: &[&Path]) -> (bool, String) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_ore"));
    c.arg(verbo);
    for a in args {
        c.arg(a);
    }
    let s = c.output().expect("no se pudo invocar `ore`");
    (
        s.status.success(),
        String::from_utf8_lossy(&s.stdout).to_string()
            + String::from_utf8_lossy(&s.stderr).as_ref(),
    )
}

const CONFIG: &str = "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\n\
     metadata: { name: x, version: 0.1.0 }\ndatasources:\n  \
     - { name: erp, type: postgres, connectionEnv: ERP_URL }\n";

const TABLA: &str = "apiVersion: oos.dev/v1alpha8\nkind: Table\n\
     metadata: { name: employees, namespace: hr }\nspec:\n  datasource: erp\n  \
     object: public.employees\n  columns:\n    employee_id: {}\n    country: {}\n  \
     reads: { predicatePushdown: [eq], fullScan: cheap }\n  \
     changes: { mode: retract, witness: log }\n";

fn manifiesto(version: &str, extra: &str) -> String {
    format!(
        "apiVersion: oos.dev/v1alpha1\nkind: Package\n\
         metadata: {{ name: hr, version: {version}, status: active, domain: d }}\n\
         spec:\n  owner: team:data\n{extra}"
    )
}

fn vista(campos: &str, extra: &str) -> String {
    format!(
        "apiVersion: oos.dev/v1alpha8\nkind: View\n\
         metadata: {{ name: empleados, namespace: hr }}\nspec:\n  owner: team:hr\n  \
         from: {{ table: hr.employees }}\n  fields:\n{campos}{extra}"
    )
}

/// `OOS2006` vale en los tres alcances, y reservar lo inexistente es legal en
/// todos.
///
/// La simetría es la afirmación: **es un mecanismo, no tres reglas parecidas.**
/// Y la segunda mitad importa tanto como la primera — reservar mira hacia
/// delante, así que retirar un nombre que aún no existe es exactamente para lo
/// que sirve.
#[test]
fn un_nombre_retirado_no_vuelve_en_ninguno_de_los_dos_alcances() {
    let caso = |etiqueta: &str, en_vista: &str, en_manifiesto: &str| {
        let dir = arbol(
            etiqueta,
            &[
                ("ontology.config.yaml", CONFIG),
                ("package.yaml", &manifiesto("1.0.0", en_manifiesto)),
                ("tables/employees.yaml", TABLA),
                (
                    "views/empleados.yaml",
                    &vista("    employeeId: employee_id\n    pais: country\n", en_vista),
                ),
            ],
        );
        let (ok, out) = correr("validate", &[&dir]);
        let _ = std::fs::remove_dir_all(&dir);
        (ok, out)
    };

    // El campo, contra el `reserved` de su propia vista.
    let (ok, out) = caso(
        "ren-campo-vivo",
        "  reserved:\n    - { name: pais, reason: retirado }\n",
        "",
    );
    assert!(!ok && out.contains("OOS2006"), "{out}");

    // El documento, contra el `reserved` de su manifiesto.
    let (ok, out) = caso(
        "ren-doc-vivo",
        "",
        "  reserved:\n    - { name: hr.empleados, reason: retirada }\n",
    );
    assert!(!ok && out.contains("OOS2006"), "{out}");

    // Y reservar lo que no existe es legal en los dos: reservar mira hacia
    // delante, y esa es la mitad que impide que el nombre vuelva.
    let (ok, out) = caso(
        "ren-fantasmas",
        "  reserved:\n    - { name: fantasma, reason: nunca existio }\n",
        "  reserved:\n    - { name: hr.fantasma, reason: nunca existio }\n",
    );
    assert!(ok, "reservar hacia delante tiene que valer:\n{out}");
}

/// Un campo anunciado no cuenta como supresión.
///
/// Es la misma válvula que una propiedad anunciada tiene desde v1alpha1, y es
/// la razón por la que `OOS5001` no se pudo extender a los campos hasta que la
/// vista tuvo dónde anunciar: sin ella, todo renombrado de campo habría sido
/// rompedor para siempre.
#[test]
fn un_campo_anunciado_no_cuenta_como_supresion() {
    let par = |etiqueta: &str, extra_despues: &str| {
        let dir = std::env::temp_dir().join(format!("ore-{etiqueta}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        escribir(
            &dir.join("before"),
            &[
                ("ontology.config.yaml", CONFIG),
                ("package.yaml", &manifiesto("1.0.0", "")),
                ("tables/employees.yaml", TABLA),
                (
                    "views/empleados.yaml",
                    &vista("    employeeId: employee_id\n    pais: country\n", ""),
                ),
            ],
        );
        escribir(
            &dir.join("after"),
            &[
                ("ontology.config.yaml", CONFIG),
                ("package.yaml", &manifiesto("2.0.0", "")),
                ("tables/employees.yaml", TABLA),
                (
                    "views/empleados.yaml",
                    &vista("    employeeId: employee_id\n", extra_despues),
                ),
            ],
        );
        let (_, out) = correr("diff", &[&dir.join("before"), &dir.join("after")]);
        let _ = std::fs::remove_dir_all(&dir);
        out
    };

    // Sin anunciar: es una supresión y se dice.
    let out = par("ren-campo-mudo", "");
    assert!(out.contains("OOS5001"), "{out}");
    assert!(out.contains("hr.empleados.pais"), "{out}");

    // Anunciado: no lo es.
    let out = par(
        "ren-campo-dicho",
        "  moved:\n    - { from: pais, to: region, since: 2.0.0 }\n",
    );
    assert!(
        !out.contains("OOS5001"),
        "un campo anunciado no es una supresión:\n{out}"
    );
}
