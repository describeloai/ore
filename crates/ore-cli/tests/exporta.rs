//! `exports` dentro de un árbol — desde v1alpha28 (ORE 0059) no es frontera.
//!
//! Un árbol es un catálogo: sus bases se leen por su nombre, en cualquier
//! versión de sus documentos y de su config, con `exports` o sin él. Lo que se
//! afirma aquí son los dos árboles que antes eran `OOS2028`:
//!
//! 1. un documento v1alpha8 que cruza a un paquete sin `exports`;
//! 2. el peldaño que `exports` no nombraba.
//!
//! Por la CLI pública, como el resto: lo que se afirma es lo que un usuario ve.

use std::path::{Path, PathBuf};
use std::process::Command;

fn paquete(etiqueta: &str, ficheros: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ore-{etiqueta}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (rel, txt) in ficheros {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, txt).unwrap();
    }
    dir
}

fn validar(dir: &Path) -> (bool, String) {
    let s = Command::new(env!("CARGO_BIN_EXE_ore"))
        .arg("validate")
        .arg(dir)
        .output()
        .expect("no se pudo invocar `ore`");
    (
        s.status.success(),
        String::from_utf8_lossy(&s.stdout).to_string()
            + String::from_utf8_lossy(&s.stderr).as_ref(),
    )
}

const CONFIG: &str = "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\n\
     metadata: { name: x, version: 0.1.0 }\ndatasources:\n  \
     - { name: erp, type: postgres, connectionEnv: ERP_URL }\n";

fn manifiesto(nombre: &str, exports: Option<&str>) -> String {
    let e = exports.map_or(String::new(), |x| format!(", exports: [{x}]"));
    format!(
        "apiVersion: oos.dev/v1alpha1\nkind: Package\n\
         metadata: {{ name: {nombre}, version: 1.0.0, status: active, domain: d }}\n\
         spec: {{ owner: team:data{e} }}\n"
    )
}

const TABLA: &str = "apiVersion: oos.dev/v1alpha8\nkind: Table\n\
     metadata: { name: employees, namespace: infra }\nspec:\n  datasource: erp\n  \
     object: public.employees\n  columns:\n    employee_id: {}\n    country: {}\n  \
     reads: { predicatePushdown: [eq], fullScan: cheap }\n  \
     changes: { mode: retract, witness: log }\n";

/// `empleados` es el peldaño; `iberia` se apoya en él y es lo que se expone.
const PELDANO: &str = "apiVersion: oos.dev/v1alpha8\nkind: View\n\
     metadata: { name: empleados, namespace: infra }\nspec:\n  owner: team:hr\n  \
     from: { table: infra.employees }\n  fields:\n    employeeId: employee_id\n    \
     pais: country\n";

const EXPUESTA: &str = "apiVersion: oos.dev/v1alpha8\nkind: View\n\
     metadata: { name: iberia, namespace: infra }\nspec:\n  owner: team:hr\n  \
     from: { view: empleados }\n  fields:\n    id: employeeId\n  \
     where:\n    pais: [ES, PT]\n";

fn entidad(nombre: &str, vista: &str, campo: &str, version: &str) -> String {
    format!(
        "apiVersion: oos.dev/{version}\nkind: Entity\n\
         metadata: {{ name: {nombre}, namespace: rrhh }}\nspec:\n  nature: entity\n  \
         primaryKey: [{campo}]\n  backedBy: {vista}\n  properties:\n    \
         {campo}: {{ type: String }}\n"
    )
}

/// Un documento cruza a otro paquete sin que este exporte nada, sea de
/// v1alpha1 o de v1alpha8: compila (antes, el de v1alpha8 era `OOS2028`).
#[test]
fn cruzar_de_paquete_compila_sin_exports() {
    for version in ["v1alpha1", "v1alpha8"] {
        let dir = paquete(
            &format!("exporta-cruce-{version}"),
            &[
                ("ontology.config.yaml", CONFIG),
                ("packages/infra/package.yaml", &manifiesto("infra", None)),
                ("packages/infra/tables/employees.yaml", TABLA),
                ("packages/infra/views/empleados.yaml", PELDANO),
                ("packages/rrhh/package.yaml", &manifiesto("rrhh", None)),
                (
                    "packages/rrhh/entities/Employee.yaml",
                    &entidad("Employee", "infra.empleados", "employeeId", version),
                ),
            ],
        );
        let (ok, out) = validar(&dir);
        assert!(
            ok,
            "{version}: el árbol es un catálogo:
{out}"
        );
        assert!(!out.contains("OOS2028"), "{out}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Con un `exports` que nombra la vista de arriba, el peldaño sobre el que se
/// apoya también se lee: la lista no cierra lo que no nombra.
#[test]
fn exports_no_cierra_el_peldano() {
    let dir = paquete(
        "exporta-peldano",
        &[
            ("ontology.config.yaml", CONFIG),
            (
                "packages/infra/package.yaml",
                &manifiesto("infra", Some("infra.iberia")),
            ),
            ("packages/infra/tables/employees.yaml", TABLA),
            ("packages/infra/views/empleados.yaml", PELDANO),
            ("packages/infra/views/iberia.yaml", EXPUESTA),
            ("packages/rrhh/package.yaml", &manifiesto("rrhh", None)),
            (
                "packages/rrhh/entities/Iberico.yaml",
                &entidad("Iberico", "infra.iberia", "id", "v1alpha8"),
            ),
            (
                "packages/rrhh/entities/Otra.yaml",
                &entidad("Otra", "infra.empleados", "employeeId", "v1alpha8"),
            ),
        ],
    );
    let (ok, out) = validar(&dir);
    assert!(
        ok,
        "el peldaño se lee por su nombre:
{out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
