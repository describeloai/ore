//! `ore diff` sobre el sustrato — las dos propiedades que deciden el diseño.
//!
//! Los cuatro casos de `conformance/v1alpha8/diff` cubren lo normativo: un
//! código por dirección. Lo que no cubren son las dos consecuencias de haber
//! elegido comparar **el efecto y no la sintaxis**:
//!
//! 1. un recorte **incomparable** enciende los dos códigos, y por eso no hace
//!    falta un tercero;
//! 2. un **renombre** en un eslabón intermedio no inventa un cambio, porque el
//!    recorte se acumula en columnas físicas de la raíz.
//!
//! La segunda es la que justifica resolver la cadena en vez de comparar lo
//! declarado, y sin ella cada renombre produciría un falso rompedor.

use std::path::{Path, PathBuf};
use std::process::Command;

fn escribir(dir: &Path, ficheros: &[(&str, &str)]) {
    for (rel, txt) in ficheros {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, txt).unwrap();
    }
}

fn par(etiqueta: &str, antes: &[(&str, &str)], despues: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ore-{etiqueta}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    escribir(&dir.join("before"), antes);
    escribir(&dir.join("after"), despues);
    dir
}

fn diferencia(dir: &Path) -> String {
    let s = Command::new(env!("CARGO_BIN_EXE_ore"))
        .arg("diff")
        .arg(dir.join("before"))
        .arg(dir.join("after"))
        .output()
        .expect("no se pudo invocar `ore`");
    String::from_utf8_lossy(&s.stdout).to_string() + String::from_utf8_lossy(&s.stderr).as_ref()
}

const CONFIG: &str = "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\n\
     metadata: { name: x, version: 0.1.0 }\ndatasources:\n  \
     - { name: erp, type: postgres, connectionEnv: ERP_URL }\n";

fn manifiesto(v: &str) -> String {
    format!(
        "apiVersion: oos.dev/v1alpha1\nkind: Package\n\
         metadata: {{ name: hr, version: {v}, status: active, domain: d }}\n\
         spec: {{ owner: team:data }}\n"
    )
}

const TABLA: &str = "apiVersion: oos.dev/v1alpha8\nkind: Table\n\
     metadata: { name: employees, namespace: erp }\nspec:\n  datasource: erp\n  \
     object: public.employees\n  columns:\n    employee_id: {}\n    country: {}\n    \
     deleted: {}\n  reads: { predicatePushdown: [eq], fullScan: cheap }\n  \
     changes: { mode: retract, witness: log }\n";

/// La base con un `where` sobre `deleted`, y el campo del país con el nombre
/// que se le pase — que es lo que la segunda prueba mueve.
fn vista(campo_pais: &str, borrado: &str) -> String {
    format!(
        "apiVersion: oos.dev/v1alpha8\nkind: View\n\
         metadata: {{ name: empleados, namespace: hr }}\nspec:\n  owner: team:hr\n  \
         from: {{ table: erp.employees }}\n  fields:\n    employeeId: employee_id\n    \
         {campo_pais}: country\n  where:\n    deleted: \"{borrado}\"\n"
    )
}

fn arriba(campo_pais: &str) -> String {
    format!(
        "apiVersion: oos.dev/v1alpha8\nkind: View\n\
         metadata: {{ name: iberia, namespace: hr }}\nspec:\n  owner: team:hr\n  \
         from: {{ view: empleados }}\n  fields:\n    id: employeeId\n  \
         where:\n    {campo_pais}: [ES, PT]\n"
    )
}

/// Un recorte **incomparable** enciende los dos códigos.
///
/// `deleted: "false"` por `"true"` no es más estrecho ni más ancho: es otro
/// conjunto. Pierde filas **y** gana filas, así que emite `OOS5028` y
/// `OOS5029` — y por eso no hace falta un tercer código para el caso disjunto.
#[test]
fn un_recorte_incomparable_enciende_los_dos() {
    let dir = par(
        "diff-incomparable",
        &[
            ("ontology.config.yaml", CONFIG),
            ("package.yaml", &manifiesto("1.0.0")),
            ("tables/employees.yaml", TABLA),
            ("views/empleados.yaml", &vista("pais", "false")),
        ],
        &[
            ("ontology.config.yaml", CONFIG),
            ("package.yaml", &manifiesto("2.0.0")),
            ("tables/employees.yaml", TABLA),
            ("views/empleados.yaml", &vista("pais", "true")),
        ],
    );
    let out = diferencia(&dir);
    assert!(out.contains("OOS5028"), "pierde filas y no lo dice:\n{out}");
    assert!(out.contains("OOS5029"), "gana filas y no lo dice:\n{out}");
    // Y cada uno en su eje: el que pierde le duele a quien lee, el que gana a
    // quien responde del riesgo.
    assert!(out.contains("CONSUMER") && out.contains("POLICY"), "{out}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Un renombre en la cadena **no** inventa un cambio.
///
/// `pais` pasa a llamarse `region` en la vista de abajo y en el `where` de la
/// de arriba. Lo que se sirve es exactamente lo mismo, y el recorte acumulado
/// sigue siendo `country ∈ {ES, PT}` porque se compara en **columnas físicas
/// de la raíz**.
///
/// Comparar lo declarado diría que el recorte cambió de columna, que es un
/// falso rompedor — y con `OOS5022` detrás, un falso rompedor bloquea una
/// publicación legítima.
#[test]
fn un_renombre_en_la_cadena_no_inventa_un_cambio() {
    let arbol = |campo: &str, v: &str| {
        vec![
            ("ontology.config.yaml".to_string(), CONFIG.to_string()),
            ("package.yaml".to_string(), manifiesto(v)),
            ("tables/employees.yaml".to_string(), TABLA.to_string()),
            ("views/empleados.yaml".to_string(), vista(campo, "false")),
            ("views/iberia.yaml".to_string(), arriba(campo)),
        ]
    };
    fn presta(v: &[(String, String)]) -> Vec<(&str, &str)> {
        v.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect()
    }
    let (a, b) = (arbol("pais", "1.0.0"), arbol("region", "1.0.1"));
    let dir = par("diff-renombre", &presta(&a), &presta(&b));

    let out = diferencia(&dir);
    assert!(
        !out.contains("OOS5028") && !out.contains("OOS5029"),
        "un renombre no cambia qué filas se sirven:\n{out}"
    );
    assert!(
        !out.contains("OOS5019") && !out.contains("OOS5020"),
        "ni de dónde salen:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
