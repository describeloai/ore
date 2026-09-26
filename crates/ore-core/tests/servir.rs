//! Servir una vista es servir su consulta, con cada nombre del árbol resuelto
//! (ADR 0040 paso 4): un dataset por el nombre con que lo registra quien lee,
//! una vista por su consulta como subconsulta, una tabla de un origen nunca.

use ore_core::servir::{Para, servir};
use std::fs;
use std::path::Path;

struct Arbol(std::path::PathBuf);
impl Drop for Arbol {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn escribe(raiz: &Path, rel: &str, texto: &str) {
    let p = raiz.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, texto).unwrap();
}

fn arbol(caso: &str) -> Arbol {
    let t = Arbol(std::env::temp_dir().join(format!("ore-servir-{}-{caso}", std::process::id())));
    let _ = fs::remove_dir_all(&t.0);
    let r = &t.0;
    escribe(
        r,
        "ontology.config.yaml",
        "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\nmetadata: { name: servir, version: 0.1.0 }\ndatasources:\n  - { name: pg, type: postgres, connectionEnv: PG_URL }\n",
    );
    escribe(
        r,
        "packages/ventas/package.yaml",
        "apiVersion: oos.dev/v1alpha1\nkind: Package\nmetadata: { name: ventas, version: 0.1.0, status: active, domain: ventas }\nspec: { owner: team:ventas }\n",
    );
    escribe(
        r,
        "packages/ventas/tables/pedidos_t.yaml",
        "apiVersion: oos.dev/v1alpha8\nkind: Table\nmetadata: { name: pedidos_t, namespace: ventas }\nspec:\n  datasource: pg\n  object: \"public.pedidos\"\n  columns:\n    id: { type: Integer }\n    pais: { type: String }\n  reads: { fullScan: cheap }\n  changes: { mode: append, witness: snapshot }\n",
    );
    escribe(
        r,
        "packages/ventas/datasets/pedidos.yaml",
        "apiVersion: oos.dev/v1alpha12\nkind: Dataset\nmetadata: { name: pedidos, namespace: ventas }\nspec:\n  owner: team:ventas\n  columns:\n    id: { type: Integer }\n    pais: { type: String }\n    total: { type: Decimal }\n  changes: { mode: append }\n",
    );
    escribe(
        r,
        "packages/ventas/espana/schema.yaml",
        "apiVersion: oos.dev/v1alpha13\nkind: Schema\nmetadata: { name: espana, namespace: ventas }\nspec: { owner: team:ventas }\n",
    );
    escribe(
        r,
        "packages/ventas/espana/datasets/clientes.yaml",
        "apiVersion: oos.dev/v1alpha13\nkind: Dataset\nmetadata: { name: clientes, namespace: ventas, schema: espana }\nspec:\n  owner: team:ventas\n  columns:\n    id: { type: Integer }\n  changes: { mode: append }\n",
    );
    // Una vista SQL sobre un dataset, con un `WITH` que no es del árbol.
    escribe(
        r,
        "packages/ventas/views/grandes.yaml",
        "apiVersion: oos.dev/v1alpha14\nkind: View\nmetadata: { name: grandes, namespace: ventas }\nspec:\n  owner: team:ventas\n  dialect: duckdb\n  sql: |\n    WITH g AS (SELECT id, pais FROM ventas.pedidos WHERE total > 100)\n    SELECT id, pais FROM g\n  columns:\n    id: { type: Integer }\n    pais: { type: String }\n",
    );
    // Una estructurada sobre la SQL: se sirve por su traducción, y la de
    // debajo entra como subconsulta.
    escribe(
        r,
        "packages/ventas/views/grandesEs.yaml",
        "apiVersion: oos.dev/v1alpha13\nkind: View\nmetadata: { name: grandesEs, namespace: ventas }\nspec:\n  owner: team:ventas\n  from: { view: ventas.grandes }\n  where: { pais: ES }\n  fields: { id: id }\n",
    );
    // Una SQL que junta un dataset de `default` con uno de `espana`.
    escribe(
        r,
        "packages/ventas/views/cruce.yaml",
        "apiVersion: oos.dev/v1alpha14\nkind: View\nmetadata: { name: cruce, namespace: ventas }\nspec:\n  owner: team:ventas\n  dialect: duckdb\n  sql: |\n    SELECT p.id FROM ventas.pedidos p JOIN ventas.espana.clientes c ON c.id = p.id\n  columns:\n    id: { type: Integer }\n",
    );
    // Una que lee una tabla de un origen: virtual, no se sirve desde lo que se tiene.
    escribe(
        r,
        "packages/ventas/views/virtual.yaml",
        "apiVersion: oos.dev/v1alpha14\nkind: View\nmetadata: { name: virtual, namespace: ventas }\nspec:\n  owner: team:ventas\n  dialect: duckdb\n  sql: |\n    SELECT id FROM ventas.pedidos_t\n  columns:\n    id: { type: Integer }\n",
    );
    t
}

fn vista<'a>(pkg: &'a ore_core::link::Package, qn: &str) -> &'a ore_core::link::Loaded {
    pkg.view(qn).unwrap_or_else(|| panic!("no está {qn}"))
}

#[test]
fn un_dataset_se_nombra_como_lo_registra_el_puesto() {
    let t = arbol("puesto");
    let (pkg, diags) = ore_core::validate::cargar_paquete(&t.0);
    assert!(diags.is_empty(), "{diags:?}");
    let s = servir(&pkg, vista(&pkg, "ventas.grandes"), Para::Puesto).unwrap();
    assert_eq!(s.datasets, ["ventas.pedidos"]);
    assert!(
        s.consulta.contains(r#""__ore_dataset"."ventas.pedidos""#),
        "{}",
        s.consulta
    );
    // El nombre del `WITH` no es del árbol, y se queda.
    assert!(s.consulta.contains("FROM g"), "{}", s.consulta);
}

#[test]
fn una_vista_que_lee_otra_la_lleva_dentro() {
    let t = arbol("anidada");
    let (pkg, _) = ore_core::validate::cargar_paquete(&t.0);
    let s = servir(&pkg, vista(&pkg, "ventas.grandesEs"), Para::Puesto).unwrap();
    assert_eq!(s.datasets, ["ventas.pedidos"]);
    assert!(
        s.consulta.contains("\"grandes\""),
        "el alias de la de debajo: {}",
        s.consulta
    );
    assert!(
        !s.consulta.contains("\"ventas\".\"grandes\""),
        "{}",
        s.consulta
    );
}

#[test]
fn el_catalogo_nombra_como_el_catalogo() {
    let t = arbol("catalogo");
    let (pkg, _) = ore_core::validate::cargar_paquete(&t.0);
    let v = vista(&pkg, "ventas.cruce");
    let sin = servir(&pkg, v, Para::Catalogo { base: None }).unwrap();
    assert!(
        sin.consulta.contains(r#""ventas"."pedidos""#),
        "{}",
        sin.consulta
    );
    assert!(
        sin.consulta.contains(r#""ventas"."espana"."clientes""#),
        "{}",
        sin.consulta
    );
    let con = servir(
        &pkg,
        v,
        Para::Catalogo {
            base: Some("ventas"),
        },
    )
    .unwrap();
    assert!(
        con.consulta.contains(r#""default"."pedidos""#),
        "{}",
        con.consulta
    );
    assert!(
        con.consulta.contains(r#""espana"."clientes""#),
        "{}",
        con.consulta
    );
    let p = servir(&pkg, v, Para::Puesto).unwrap();
    assert_eq!(p.datasets, ["ventas.pedidos", "ventas.espana.clientes"]);
}

#[test]
fn una_tabla_de_un_origen_no_se_sirve() {
    let t = arbol("virtual");
    let (pkg, _) = ore_core::validate::cargar_paquete(&t.0);
    let e = servir(&pkg, vista(&pkg, "ventas.virtual"), Para::Puesto).unwrap_err();
    assert!(e.contains("ventas.pedidos_t") && e.contains("Table"), "{e}");
}
