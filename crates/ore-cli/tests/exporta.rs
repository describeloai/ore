//! `exports` — las dos propiedades que ningún caso de conformidad ejerce.
//!
//! Los tres casos de `conformance/v1alpha8` cubren lo normativo: la lista
//! puesta, la lista ausente y la lista con un nombre de más. Lo que no cubren
//! —porque un caso afirma una regla y estas dos son consecuencias— es:
//!
//! 1. **la puerta de versión**, que es lo que hace que esta regla no cambie el
//!    resultado de un documento anterior;
//! 2. **que exportar la vista de arriba no arrastra el peldaño**, que es la
//!    diferencia entre exponer y contener dicha en un árbol.
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

/// Un documento anterior a v1alpha8 cruza sin permiso, y eso es la regla.
///
/// El invariante que esta línea de trabajo sostiene es que **no cambia un solo
/// resultado de v1alpha1 a v1alpha7**. Se midió sin la puerta y caían dos casos
/// de `conformance/v1alpha4` —`concept-from-another-package` y
/// `vocabulary-member-has-no-entities`—, que son exactamente los dos únicos
/// cruces del corpus y son el mismo: un vocabulario del que otros toman
/// autoridad.
///
/// Decide la versión del documento que **escribe** la referencia: quien se
/// acopló lo hizo bajo unas reglas, y son las suyas las que valen.
#[test]
fn un_documento_anterior_a_v1alpha8_cruza_sin_permiso() {
    let arbol = |version: &str| {
        paquete(
            &format!("exporta-puerta-{version}"),
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
        )
    };

    let dir = arbol("v1alpha1");
    let (ok, out) = validar(&dir);
    assert!(
        ok,
        "un documento v1alpha1 no puede cambiar de resultado:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&dir);

    let dir = arbol("v1alpha8");
    let (ok, out) = validar(&dir);
    assert!(!ok, "tenía que negarse:\n{out}");
    assert!(out.contains("OOS2028"), "{out}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Exportar la vista de arriba **no** arrastra el peldaño sobre el que se apoya.
///
/// Es la diferencia entre exponer y contener dicha en un árbol: `infra`
/// contiene dos vistas y expone una. El consumidor no nombra `empleados`, así
/// que no se acopla a ella, así que puede cambiar — y el compilador lo hace
/// cumplir en vez de confiar en que nadie mire.
#[test]
fn exportar_la_de_arriba_no_arrastra_el_peldano() {
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
        ],
    );
    let (ok, out) = validar(&dir);
    assert!(ok, "lo exportado tiene que compilar:\n{out}");

    // Y ahora una segunda entidad que se apoya en el PELDAÑO, que no se exporta.
    std::fs::write(
        dir.join("packages/rrhh/entities/Otra.yaml"),
        entidad("Otra", "infra.empleados", "employeeId", "v1alpha8"),
    )
    .unwrap();
    let (ok, out) = validar(&dir);
    assert!(!ok, "tenía que negarse:\n{out}");
    assert!(out.contains("OOS2028"), "{out}");
    assert!(
        out.contains("infra.empleados") && !out.contains("`backedBy: infra.iberia` cruza"),
        "solo el peldaño se rechaza, y la expuesta sigue valiendo:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
