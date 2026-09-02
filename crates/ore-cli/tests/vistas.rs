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

fn conformidad8(nombre: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/oos/conformance/v1alpha8")
        .join(nombre)
        .join("input")
}

/// Escribe un paquete en un directorio propio de esta prueba.
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

const CONFIG: &str = "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\n\
     metadata: { name: x, version: 0.1.0 }\ndatasources:\n  \
     - { name: erp, type: postgres, connectionEnv: ERP_URL }\n  \
     - { name: lago, type: postgres, connectionEnv: LAGO_URL }\n";

const PAQUETE: &str = "apiVersion: oos.dev/v1alpha1\nkind: Package\n\
     metadata: { name: hr, version: 1.0.0, status: active, domain: people }\n\
     spec: { owner: team:data }\n";

const RETICULO: &str = "apiVersion: oos.dev/v1alpha3\nkind: Lattice\n\
     metadata: { name: sensitivity, namespace: gdpr }\nspec:\n  levels: [none, low, high]\n";

fn conducto(nivel: &str) -> String {
    format!(
        "apiVersion: oos.dev/v1alpha1\nkind: ConduitPolicy\nmetadata: {{ name: hr }}\n\
         spec:\n  owner: team:security\n  conduits:\n    materialization.payload:\n      \
         gdpr.sensitivity: {nivel}\n"
    )
}

/// La tabla de v1alpha8 que las pruebas de abajo comparten: las mismas columnas
/// y las mismas capacidades que la vista de v1alpha7 declaraba dentro.
const TABLA: &str = "apiVersion: oos.dev/v1alpha8\nkind: Table\n\
     metadata: { name: employees, namespace: erp }\nspec:\n  datasource: erp\n  \
     object: public.employees\n  columns:\n    employee_id: {}\n    national_id: {}\n    \
     country: {}\n  reads:\n    predicatePushdown: [eq, in]\n    fullScan: cheap\n  \
     changes: { mode: retract, witness: log }\n";

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

// ── v1alpha8 · lo mismo, con el puntero fuera de la vista ────────────────────
//
// Los tres gemelos afirman lo mismo que los tres de arriba, y por eso los tres
// de arriba NO se tocan: un documento v1alpha7 sigue compilando mientras
// v1alpha1 sea normativo, y borrar su prueba sería dejar de mirar un camino que
// sigue vivo. Lo que se afirma junto es que **el motor no se entera de por cuál
// de los dos vino el puntero**.

#[test]
fn la_cadena_sobre_una_tabla_da_el_mismo_linaje_y_ensena_las_dos_caras() {
    let dir = paquete(
        "vista-tabla-cadena",
        &[
            ("ontology.config.yaml", CONFIG),
            ("package.yaml", PAQUETE),
            ("lattices/sensitivity.yaml", RETICULO),
            ("tables/employees.yaml", TABLA),
            (
                "views/empleados.yaml",
                "apiVersion: oos.dev/v1alpha8\nkind: View\n\
                 metadata: { name: empleados, namespace: hr }\nspec:\n  owner: team:hr\n  \
                 from: { table: erp.employees }\n  fields:\n    employeeId: employee_id\n    \
                 nationalId: national_id\n    pais: country\n",
            ),
            (
                "views/iberia.yaml",
                "apiVersion: oos.dev/v1alpha8\nkind: View\n\
                 metadata: { name: iberia, namespace: hr }\nspec:\n  owner: team:hr\n  \
                 from: { view: empleados }\n  fields:\n    id: employeeId\n    dni: nationalId\n  \
                 where:\n    pais: [ES, PT]\n",
            ),
            (
                "entities/Employee.yaml",
                "apiVersion: oos.dev/v1alpha8\nkind: Entity\n\
                 metadata: { name: Employee, namespace: hr }\nspec:\n  nature: entity\n  \
                 primaryKey: [id]\n  backedBy: iberia\n  properties:\n    id: { type: String }\n    \
                 dni: { type: String, labels: { gdpr.sensitivity: high } }\n",
            ),
        ],
    );
    let (ok, out, err) = ver(&dir);
    assert!(ok, "{err}\n{out}");

    // Lo mismo que sobre una vista de v1alpha7, palabra por palabra.
    assert!(out.contains("2 vistas encadenadas"), "{out}");
    assert!(out.contains("raíz      erp · public.employees"), "{out}");
    assert!(
        out.contains("linaje    dni ← erp·public.employees.national_id  DIRECT · Identidad"),
        "{out}"
    );
    assert!(
        out.contains("linaje    id ← erp·public.employees.country  INDIRECT · Filtro"),
        "{out}"
    );
    assert!(out.contains("esquema   dni: String · id: String"), "{out}");
    assert!(out.contains("REFRESH_MODE = INCREMENTAL"), "{out}");

    // Y las capacidades salen de `reads` de la TABLA: `eq` e `in` bajan el
    // filtro. En v1alpha7 esto se leía de `capabilities` de la vista raíz.
    assert!(
        out.contains("empuje    erp·public.employees recibe 1 filtro"),
        "{out}"
    );

    // Lo que v1alpha7 no podía enseñar: las dos caras del objeto.
    assert!(
        out.contains("caras     reads: eq, in · fullScan: cheap · changes: retract · witness: log"),
        "{out}"
    );
    assert!(out.contains("raíz de lectura: la tabla"), "{out}");
}

/// El criterio de T2, medido sobre el caso de conformidad que lo nombra:
/// **`reads: none` y la raíz de lectura en la copia.**
#[test]
fn un_topico_ensena_que_no_se_lee_y_que_se_lee_de_la_copia() {
    let (ok, out, err) = ver(&conformidad8("valid/stream-table-materialized"));
    assert!(ok, "{err}\n{out}");
    assert!(out.contains("caras     reads: none"), "{out}");
    assert!(out.contains("raíz de lectura: la copia"), "{out}");
    // Y la regla que lo decidió, por su nombre.
    assert!(out.contains("`OOS2020` la exige"), "{out}");
    // La cara `D` de un tópico compactado: `upsert`, con su clave.
    assert!(
        out.contains("changes: upsert · witness: log · key: order_id"),
        "{out}"
    );
}

#[test]
fn una_copia_sobre_una_tabla_que_cabe_en_su_conducto_viaja_sellada() {
    let dir = paquete(
        "vista-tabla-sellada",
        &[
            ("ontology.config.yaml", CONFIG),
            ("package.yaml", PAQUETE),
            ("lattices/sensitivity.yaml", RETICULO),
            ("conduits.yaml", &conducto("high")),
            ("tables/employees.yaml", TABLA),
            (
                "views/empleados.yaml",
                "apiVersion: oos.dev/v1alpha8\nkind: View\n\
                 metadata: { name: empleados, namespace: hr }\nspec:\n  owner: team:hr\n  \
                 from: { table: erp.employees }\n  fields:\n    employeeId: employee_id\n    \
                 nationalId: national_id\n  \
                 materialized: { datasource: lago, table: cache.hr_employees }\n",
            ),
            (
                "views/iberia.yaml",
                "apiVersion: oos.dev/v1alpha8\nkind: View\n\
                 metadata: { name: iberia, namespace: hr }\nspec:\n  owner: team:hr\n  \
                 from: { view: empleados }\n  fields:\n    id: employeeId\n    dni: nationalId\n",
            ),
            (
                "entities/Employee.yaml",
                "apiVersion: oos.dev/v1alpha8\nkind: Entity\n\
                 metadata: { name: Employee, namespace: hr }\nspec:\n  nature: entity\n  \
                 primaryKey: [id]\n  backedBy: iberia\n  properties:\n    id: { type: String }\n    \
                 dni: { type: String, labels: { gdpr.sensitivity: high } }\n",
            ),
        ],
    );
    let (ok, out, err) = ver(&dir);
    assert!(ok, "{err}\n{out}");
    // El sello sigue heredándose por el linaje con una tabla debajo: `dni` es
    // `high` porque la entidad, dos eslabones arriba, lo dijo.
    assert!(
        out.contains("`materialization.payload` compila · sellada:")
            && out.contains("nationalId {gdpr.sensitivity:high}"),
        "{out}"
    );
}

/// **El que el motor ve y el núcleo no, con una tabla debajo.** El gemelo
/// v1alpha8 del de abajo, y el que de verdad importa de los tres: la arista
/// `INDIRECT` no sale de la gramática ni de la versión — sale de que **qué
/// filas aparecen es observable**. Cambiar dónde vive el puntero no la mueve.
///
/// `ore validate` acepta el paquete: la copia no lleva `nationalId`. `ore view`
/// se niega, y ahora además enseña de qué objeto salió la columna que delata.
#[test]
fn recortar_por_una_columna_clasificada_de_una_tabla_tambien_se_niega() {
    let dir = paquete(
        "vista-tabla-indirecta",
        &[
            ("ontology.config.yaml", CONFIG),
            ("package.yaml", PAQUETE),
            ("lattices/sensitivity.yaml", RETICULO),
            ("conduits.yaml", &conducto("low")),
            ("tables/employees.yaml", TABLA),
            (
                "views/empleados.yaml",
                "apiVersion: oos.dev/v1alpha8\nkind: View\n\
                 metadata: { name: empleados, namespace: hr }\nspec:\n  owner: team:hr\n  \
                 from: { table: erp.employees }\n  fields:\n    employeeId: employee_id\n    \
                 nationalId: national_id\n",
            ),
            (
                "views/iberia.yaml",
                "apiVersion: oos.dev/v1alpha8\nkind: View\n\
                 metadata: { name: iberia, namespace: hr }\nspec:\n  owner: team:hr\n  \
                 from: { view: empleados }\n  fields:\n    id: employeeId\n  where:\n    \
                 nationalId: [12345678A]\n  \
                 materialized: { datasource: lago, table: cache.iberia }\n",
            ),
            (
                "entities/Employee.yaml",
                "apiVersion: oos.dev/v1alpha8\nkind: Entity\n\
                 metadata: { name: Employee, namespace: hr }\nspec:\n  nature: entity\n  \
                 primaryKey: [employeeId]\n  backedBy: empleados\n  properties:\n    \
                 employeeId: { type: String }\n    \
                 nationalId: { type: String, labels: { gdpr.sensitivity: high } }\n",
            ),
        ],
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
        out.contains("NO compila") && err.contains("el motor de vistas se niega"),
        "{out}\n{err}"
    );
    // Y la columna que delata es la de la TABLA, nombrada por su nombre físico.
    assert!(out.contains("national_id"), "{out}");
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
