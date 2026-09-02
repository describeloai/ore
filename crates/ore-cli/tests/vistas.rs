//! `ore view` — el motor de vistas alimentado por un paquete de verdad.
//!
//! Por la CLI pública y sin enlazar `ore-core` ni `ore-view`, como el resto: lo
//! que se afirma es lo que un usuario ve.
//!
//! # Lo que de verdad se afirma aquí
//!
//! Que **la absorción no cambió al motor**: las doce piezas contestan sobre un
//! paquete OOS lo mismo que contestaban sobre planes escritos a mano — con la
//! raíz compuesta por la cadena, el linaje por columna, el modo de refresco y
//! el reparto. Y una cosa que `ore validate` no puede decir y `ore view` sí:
//! **una vista que recorta por una columna clasificada la revela aunque no la
//! exponga**, y se niega a materializarla.

use std::path::{Path, PathBuf};
use std::process::Command;

fn conformidad(nombre: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/oos/conformance/v1alpha7")
        .join(nombre)
        .join("input")
}

fn ver(dir: &Path) -> (bool, String, String) {
    let s = Command::new(env!("CARGO_BIN_EXE_ore"))
        .arg("view")
        .arg(dir)
        .output()
        .expect("no se pudo invocar `ore`");
    (
        s.status.success(),
        String::from_utf8_lossy(&s.stdout).to_string(),
        String::from_utf8_lossy(&s.stderr).to_string(),
    )
}

#[test]
fn la_cadena_se_expande_a_su_raiz_y_el_linaje_llega_a_la_columna_fisica() {
    let (ok, out, err) = ver(&conformidad("valid/view-over-view"));
    assert!(ok, "{err}\n{out}");

    // Dos vistas, y `iberia` expandida hasta `erp`.
    assert!(out.contains("hr.empleados\n"), "{out}");
    assert!(out.contains("hr.iberia\n"), "{out}");
    assert!(out.contains("2 vistas encadenadas"), "{out}");
    assert!(out.contains("raíz      erp · public.employees"), "{out}");

    // El linaje llega a la columna FÍSICA, compuesto por la cadena: `dni` es
    // `nationalId` en `iberia` y `national_id` en la raíz.
    assert!(
        out.contains("linaje    dni ← erp·public.employees.national_id  DIRECT · Identidad"),
        "{out}"
    );
    // Y el `where: { pais: [ES, PT] }` de `iberia` deja una arista INDIRECT
    // sobre cada salida: `pais` decide qué filas salen.
    assert!(
        out.contains("linaje    id ← erp·public.employees.country  INDIRECT · Filtro"),
        "{out}"
    );

    // El tipo baja de la entidad: `Employee.id` es `String`.
    assert!(out.contains("esquema   dni: String · id: String"), "{out}");

    // Seleccionar, renombrar y recortar se mantienen incrementalmente.
    assert!(out.contains("REFRESH_MODE = INCREMENTAL"), "{out}");

    // Y el reparto: `erp` sabe `eq` e `in`, así que los dos filtros bajan.
    assert!(
        out.contains("empuje    erp·public.employees recibe 2 filtros"),
        "{out}"
    );
    // Virtual: nada que copiar, nada que autorizar.
    assert!(out.contains("flujo     virtual"), "{out}");
}

#[test]
fn una_copia_que_cabe_en_su_conducto_compila_y_viaja_sellada() {
    let (ok, out, err) = ver(&conformidad("valid/materialized-view-within-clearance"));
    assert!(ok, "{err}\n{out}");
    // El sello: la clasificación de la copia se hereda por el linaje. `dni` es
    // `high` porque la entidad, dos eslabones arriba, lo dijo.
    assert!(
        out.contains("`materialization.payload` compila · sellada:")
            && out.contains("nationalId {gdpr.sensitivity:high}"),
        "{out}"
    );
}

/// **Lo que solo el motor ve.** `iberia` expone `id` y recorta por
/// `nationalId`, que la entidad clasifica `high`. No copia el DNI — y revela
/// quién lo tiene, porque qué filas aparecen es observable. `ore validate`
/// acepta el paquete: el núcleo comprueba lo que se copia, no lo que decide.
/// `ore view` se niega, por la arista INDIRECT, y dice por dónde.
#[test]
fn recortar_por_una_columna_clasificada_la_revela_y_el_motor_se_niega() {
    let dir = std::env::temp_dir().join(format!("ore-vista-indirecta-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let w = |rel: &str, txt: &str| {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, txt).unwrap();
    };
    w(
        "ontology.config.yaml",
        "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\nmetadata: { name: x, version: 0.1.0 }\n\
         datasources:\n  - { name: erp, type: postgres, connectionEnv: ERP_URL }\n  \
         - { name: lago, type: postgres, connectionEnv: LAGO_URL }\n",
    );
    w(
        "package.yaml",
        "apiVersion: oos.dev/v1alpha1\nkind: Package\n\
         metadata: { name: hr, version: 1.0.0, status: active, domain: people }\nspec: { owner: team:data }\n",
    );
    w(
        "lattices/sensitivity.yaml",
        "apiVersion: oos.dev/v1alpha3\nkind: Lattice\nmetadata: { name: sensitivity, namespace: gdpr }\n\
         spec:\n  levels: [none, low, high]\n",
    );
    w(
        "conduits.yaml",
        "apiVersion: oos.dev/v1alpha1\nkind: ConduitPolicy\nmetadata: { name: hr }\n\
         spec:\n  owner: team:security\n  conduits:\n    materialization.payload:\n      gdpr.sensitivity: low\n",
    );
    w(
        "views/empleados.yaml",
        "apiVersion: oos.dev/v1alpha7\nkind: View\nmetadata: { name: empleados, namespace: hr }\n\
         spec:\n  owner: team:hr\n  from: { datasource: erp, object: public.employees }\n  \
         version: { witness: none }\n  fields:\n    employeeId: employee_id\n    nationalId: national_id\n",
    );
    w(
        "views/iberia.yaml",
        "apiVersion: oos.dev/v1alpha7\nkind: View\nmetadata: { name: iberia, namespace: hr }\n\
         spec:\n  owner: team:hr\n  from: { view: empleados }\n  version: { witness: none }\n  \
         fields:\n    id: employeeId\n  where:\n    nationalId: [12345678A]\n  \
         materialized: { datasource: lago, table: cache.iberia }\n",
    );
    w(
        "entities/Employee.yaml",
        "apiVersion: oos.dev/v1alpha7\nkind: Entity\nmetadata: { name: Employee, namespace: hr }\n\
         spec:\n  nature: entity\n  primaryKey: [employeeId]\n  backedBy: empleados\n  properties:\n    \
         employeeId: { type: String }\n    nationalId: { type: String, labels: { gdpr.sensitivity: high } }\n",
    );

    // El núcleo lo acepta: `iberia` no copia `nationalId`.
    let v = Command::new(env!("CARGO_BIN_EXE_ore"))
        .arg("validate")
        .arg(&dir)
        .output()
        .unwrap();
    assert!(
        v.status.success(),
        "`ore validate` tenía que aceptar: la copia no lleva la columna\n{}",
        String::from_utf8_lossy(&v.stderr)
    );

    // El motor no: qué filas salen lo decide una columna `high`.
    let (ok, out, err) = ver(&dir);
    assert!(!ok, "tenía que negarse:\n{out}");
    assert!(
        out.contains("`materialization.payload` NO compila"),
        "{out}"
    );
    assert!(out.contains("`id` no compila"), "{out}");
    assert!(
        out.contains("erp·public.employees.national_id  por INFLUENCIA (Filtro)"),
        "{out}"
    );
    assert!(err.contains("se niega a compilar"), "{err}");

    let _ = std::fs::remove_dir_all(&dir);
}
