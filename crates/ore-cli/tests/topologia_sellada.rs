//! **El sello del índice de topología**, por la CLI pública.
//!
//! Los casos de `conformance/v1alpha8` cubren lo normativo: acepta, `OOS4011`,
//! `OOS4002`. Lo que no cubren son las tres consecuencias de haber elegido
//! sellar **la copia derivada** y no la declaración:
//!
//! 1. el sello mira **dos columnas**, no la entidad — y por eso una entidad con
//!    campos que no pueden salir se atraviesa igual;
//! 2. una entidad **sin `via`** no ve la regla, ni para bien ni para mal: sin
//!    aristas no hay copia, y sin copia no hay conducto que autorizar;
//! 3. un **binding** con su eje declarado sigue sellándose por donde siempre, y
//!    esta regla no lo toca dos veces.
//!
//! La tercera es la que protege la invariante: `v1alpha1` no cambia ni un
//! resultado por haber encendido esto.

use std::path::{Path, PathBuf};
use std::process::Command;

fn paquete(etiqueta: &str, ficheros: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ore-topo-{etiqueta}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (rel, txt) in ficheros {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, txt).unwrap();
    }
    dir
}

fn validar(dir: &Path) -> String {
    let s = Command::new(env!("CARGO_BIN_EXE_ore"))
        .arg("validate")
        .arg(dir)
        .output()
        .expect("no se pudo invocar `ore`");
    String::from_utf8_lossy(&s.stdout).to_string() + String::from_utf8_lossy(&s.stderr).as_ref()
}

const CONFIG: &str = "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\n\
     metadata: { name: x, version: 0.1.0 }\ndatasources:\n  \
     - { name: erp, type: postgres, connectionEnv: ERP_URL }\n";

const PAQUETE: &str = "apiVersion: oos.dev/v1alpha1\nkind: Package\n\
     metadata: { name: hr, version: 1.0.0, status: active, domain: people }\n\
     spec: { owner: team:data }\n";

const RETICULO: &str = "apiVersion: oos.dev/v1alpha3\nkind: Lattice\n\
     metadata: { name: sensitivity, namespace: gdpr }\n\
     spec:\n  levels: [none, low, high]\n";

const TABLA: &str = "apiVersion: oos.dev/v1alpha8\nkind: Table\n\
     metadata: { name: employees, namespace: erp }\nspec:\n  datasource: erp\n  \
     object: public.employees\n  columns:\n    employee_id: {}\n    manager_id: {}\n    \
     national_id: {}\n  reads: { predicatePushdown: [eq], fullScan: cheap }\n  \
     changes: { mode: retract, witness: log }\n";

const VISTA: &str = "apiVersion: oos.dev/v1alpha8\nkind: View\n\
     metadata: { name: empleados, namespace: hr }\nspec:\n  owner: team:hr\n  \
     from: { table: erp.employees }\n  fields:\n    employeeId: employee_id\n    \
     managerId: manager_id\n    nationalId: national_id\n";

/// Autoriza el conducto del índice hasta `low`.
const CONDUCTOS: &str = "apiVersion: oos.dev/v1alpha1\nkind: ConduitPolicy\n\
     metadata: { name: p }\nspec:\n  owner: team:security\n  conduits:\n    \
     materialization.topology: { gdpr.sensitivity: low }\n";

/// La entidad, con o sin la relación que la hace atravesable.
fn entidad(con_via: bool) -> String {
    let relaciones = if con_via {
        "  relations:\n    manager:\n      target: hr.Employee\n      \
         cardinality: many_to_one\n      via: [managerId]\n"
    } else {
        ""
    };
    format!(
        "apiVersion: oos.dev/v1alpha8\nkind: Entity\n\
         metadata: {{ name: Employee, namespace: hr }}\nspec:\n  nature: entity\n  \
         primaryKey: [employeeId]\n  backedBy: empleados\n  properties:\n    \
         employeeId: {{ type: String }}\n    managerId: {{ type: String }}\n    \
         nationalId:\n      type: String\n      labels: {{ gdpr.sensitivity: high }}\n\
         {relaciones}"
    )
}

/// **El sello mira dos columnas, no la entidad.**
///
/// `nationalId` es `high` y el conducto admite `low`. Compila — y compila
/// *porque* `nationalId` no viaja: por la arista van `employeeId` y `managerId`.
///
/// Es la afirmación que impide leer esto como *«una entidad con datos sensibles
/// no se atraviesa»*, que es lo contrario de lo que dice. Y se comprueba aquí
/// además de en conformidad porque el caso válido de allí demuestra que compila,
/// no **por qué**: aquí se mueve la etiqueta a la clave y se ve cambiar la
/// respuesta sin tocar nada más.
#[test]
fn lo_critico_no_viaja_y_por_eso_la_travesia_compila() {
    let base = |e: &str| {
        let dir = paquete(
            "solo-dos-columnas",
            &[
                ("ontology.config.yaml", CONFIG),
                ("package.yaml", PAQUETE),
                ("lattices/s.yaml", RETICULO),
                ("conduits.yaml", CONDUCTOS),
                ("tables/employees.yaml", TABLA),
                ("views/empleados.yaml", VISTA),
                ("entities/Employee.yaml", e),
            ],
        );
        let out = validar(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        out
    };

    // La etiqueta está en una propiedad que NO es clave ni enlace: no viaja.
    let out = base(&entidad(true));
    assert!(
        !out.contains("OOS4002"),
        "`nationalId` no viaja en la arista:\n{out}"
    );

    // Y el control, que es lo que le da valor: la misma etiqueta sobre la CLAVE
    // sí viaja, y el sello la ve.
    let sobre_la_clave = entidad(true).replace(
        "employeeId: { type: String }",
        "employeeId:\n      type: String\n      labels: { gdpr.sensitivity: high }",
    );
    let out = base(&sobre_la_clave);
    assert!(out.contains("OOS4002"), "la clave sí viaja:\n{out}");
    assert!(
        out.contains("manager"),
        "y el mensaje nombra la relación que lo provoca:\n{out}"
    );
}

/// **Una entidad sin `via` no ve la regla.**
///
/// Ni para bien ni para mal: sin aristas no hay copia, y sin copia no hay
/// conducto que autorizar. Sin esta prueba, `OOS4011` podría pedirle la política
/// a cualquier paquete con entidades — que es la forma más fácil de que una
/// regla derivada se convierta en un peaje.
#[test]
fn sin_relaciones_no_hay_conducto_que_pedir() {
    let dir = paquete(
        "sin-via",
        &[
            ("ontology.config.yaml", CONFIG),
            ("package.yaml", PAQUETE),
            ("lattices/s.yaml", RETICULO),
            // Sin `conduits.yaml` a propósito.
            ("tables/employees.yaml", TABLA),
            ("views/empleados.yaml", VISTA),
            ("entities/Employee.yaml", &entidad(false)),
        ],
    );
    let out = validar(&dir);
    assert!(
        !out.contains("OOS4011"),
        "sin `via` no se atraviesa nada:\n{out}"
    );

    // Y el control: la misma pareja con `via` sí lo pide.
    std::fs::write(dir.join("entities/Employee.yaml"), entidad(true)).unwrap();
    let out = validar(&dir);
    assert!(out.contains("OOS4011"), "con `via`, sí:\n{out}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// **El camino viejo no se sella dos veces.**
///
/// Un `Binding` declara su eje `topology`, y el sello de la declaración ya
/// existía. Si esta regla corriera también sobre las aristas que salen de un
/// binding, un paquete v1alpha1 que **no** declara el eje pasaría a fallar — y
/// eso cambiaría un resultado de un borrador normativo.
///
/// Aquí un binding sin `materialization` y sin política de conductos: no hay
/// copia declarada, así que no hay nada que sellar, y sigue compilando como
/// siempre.
#[test]
fn un_binding_sin_eje_declarado_sigue_compilando() {
    let entidad = "apiVersion: oos.dev/v1alpha1\nkind: Entity\n\
         metadata: { name: Employee, namespace: hr }\nspec:\n  nature: entity\n  \
         primaryKey: [employeeId]\n  properties:\n    employeeId: { type: String }\n    \
         managerId: { type: String }\n  relations:\n    manager:\n      \
         target: hr.Employee\n      cardinality: many_to_one\n      via: [managerId]\n";
    let binding = "apiVersion: oos.dev/v1alpha1\nkind: Binding\n\
         metadata: { name: b, namespace: hr }\nspec:\n  targetEntity: hr.Employee\n  \
         datasourceRef: erp\n  source: public.employees\n  properties:\n    \
         employeeId: employee_id\n    managerId: manager_id\n";
    let dir = paquete(
        "binding-viejo",
        &[
            ("ontology.config.yaml", CONFIG),
            ("package.yaml", PAQUETE),
            ("lattices/s.yaml", RETICULO),
            ("entities/Employee.yaml", entidad),
            ("bindings/b.yaml", binding),
        ],
    );
    let out = validar(&dir);
    assert!(
        !out.contains("OOS4011") && !out.contains("OOS4002"),
        "el camino de bindings se sella por su declaración, no por aquí:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
