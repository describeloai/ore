//! ⭐⭐ 0045 P2 · **EL PUNTERO, ESTÉ DONDE ESTÉ.**
//!
//! La `Table` que apunta al origen vivía en la carpeta `tables/` de cada
//! database, y ore-serve la buscaba ahí: la hermana de la vista o del dataset
//! que la lee. Con 0045 pasa a vivir UNA vez, en el paquete de su fuente
//! (`packages/<fuente>/<schema>/tables/`), y la database solo la nombra.
//!
//! ⇒ Aquí se resuelve por el árbol —lo que el compilador ya hace—, y así cada
//!   lector funciona en las dos disposiciones: la de hoy (la tabla en la
//!   database, `<obj>_t`) y la de 0045.
//!
//! Medido antes de escribir esto (0045 P2): con los punteros movidos a la
//! fuente, el esquema de una database caía al catálogo entero (todo sin copiar),
//! la clave de sus copias desaparecía (19 → 0) y `retirar_fuente` no veía a
//! nadie y dejaba subir un árbol roto.
use ore_core::document::Kind;
use ore_core::link::{Loaded, Package};
use ore_core::parse;
use ore_core::vistas::{self, Fuente};
use std::collections::BTreeSet;
use std::path::Path;

/// La carpeta de `packages/` a la que pertenece un documento del árbol.
pub fn paquete_de(raiz: &Path, d: &Loaded) -> Option<String> {
    let rel = d.path.strip_prefix(raiz).unwrap_or(&d.path);
    let mut partes = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .filter(|c| c != ".");
    (partes.next()? == "packages")
        .then(|| partes.next())
        .flatten()
}

/// **¿Es el paquete de una fuente?** El que deja el Job de catálogo:
/// `discover.catalog.json` con `source` igual a su propio nombre, y sin
/// alcance. Una database lleva `discover.scope.json`; un paquete escrito a mano
/// sobre una fuente (`olist` en `la-copia-se-decide.sh`) lleva un catálogo de
/// OTRA fuente, y no es la fuente.
pub fn es_paquete_de_fuente(dir: &Path) -> bool {
    if dir.join("discover.scope.json").is_file() {
        return false;
    }
    let Some(n) = dir.file_name().and_then(|x| x.to_str()) else {
        return false;
    };
    std::fs::read_to_string(dir.join("discover.catalog.json"))
        .ok()
        .and_then(|t| parse::parse(&t).ok())
        .and_then(|c| {
            c.get("source")
                .and_then(|(_, v)| v.as_str())
                .map(|s| s == n)
        })
        .unwrap_or(false)
}

/// Las `Table` que lee un documento: la de `from.table`, o las que nombra la
/// consulta de una vista SQL (v1alpha14), resueltas por el árbol.
pub fn tablas_que_lee<'a>(pkg: &'a Package, d: &'a Loaded) -> Vec<&'a Loaded> {
    if let Some(Fuente::Tabla(qn)) = vistas::fuente(d) {
        return pkg.table(&qn).into_iter().collect();
    }
    if vistas::es_sql(d) {
        let mut vistas_: Vec<&Loaded> = Vec::new();
        for n in ore_core::servir::nombrados(pkg, d) {
            if n.doc.kind == Kind::Table && !vistas_.iter().any(|x| x.path == n.doc.path) {
                vistas_.push(n.doc);
            }
        }
        return vistas_;
    }
    Vec::new()
}

/// **La** tabla que lee: la de `from.table`, o la de una vista SQL que lee UNA
/// sola cosa y es una tabla. Es lo que el catálogo enseña como «su tabla».
pub fn tabla_que_lee<'a>(pkg: &'a Package, d: &'a Loaded) -> Option<&'a Loaded> {
    if let Some(Fuente::Tabla(qn)) = vistas::fuente(d) {
        return pkg.table(&qn);
    }
    if !vistas::es_sql(d) {
        return None;
    }
    let nombrados = ore_core::servir::nombrados(pkg, d);
    let mut distintos: Vec<&Loaded> = Vec::new();
    for n in &nombrados {
        if !distintos.iter().any(|x| x.path == n.doc.path) {
            distintos.push(n.doc);
        }
    }
    match distintos.as_slice() {
        [uno] if uno.kind == Kind::Table => Some(uno),
        _ => None,
    }
}

/// **Las databases que salen de la fuente `n`**: lo que impide retirarla.
///
/// Tres maneras de salir de ella, y las tres cuentan:
///
/// - (a) un alcance que la nombra (`discover.scope.json`, `source`);
/// - (b) una `Table` fuera de su paquete con `datasource: <n>` — la
///   disposición de hoy, y un paquete escrito a mano (`olist`);
/// - (c) un documento fuera de su paquete que lee una `Table` de él —la de 0045:
///   un Dataset o una View que nombra `<n>.<schema>.<obj>`—.
///
/// ⛔ Antes solo contaba (b), buscando carpetas `tables/`. Con los punteros en
///   la fuente no veía a nadie, la fuente se retiraba con sus Tables, y el
///   filtro de diagnósticos —que ignora los que mencionan la fuente— dejaba
///   subir los `OOS2018` de todas las databases que la leían.
pub fn bases_que_salen_de(raiz: &Path, pkg: &Package, n: &str) -> Vec<String> {
    let mut out = BTreeSet::new();
    // (a)
    if let Ok(es) = std::fs::read_dir(raiz.join("packages")) {
        for e in es.flatten() {
            let Some(p) = e.file_name().to_str().map(String::from) else {
                continue;
            };
            if p == n {
                continue;
            }
            let de_ella = std::fs::read_to_string(e.path().join("discover.scope.json"))
                .ok()
                .and_then(|t| parse::parse(&t).ok())
                .and_then(|a| {
                    a.get("source")
                        .and_then(|(_, v)| v.as_str())
                        .map(|s| s == n)
                })
                .unwrap_or(false);
            if de_ella {
                out.insert(p);
            }
        }
    }
    let de = |d: &Loaded| paquete_de(raiz, d);
    // (b)
    for t in pkg.docs.iter().filter(|d| d.kind == Kind::Table) {
        let nombra = t.section("datasource").and_then(|v| v.as_str()) == Some(n);
        if let Some(p) = de(t)
            && nombra
            && p != n
        {
            out.insert(p);
        }
    }
    // (c)
    for d in &pkg.docs {
        let Some(p) = de(d) else { continue };
        if p == n {
            continue;
        }
        if tablas_que_lee(pkg, d)
            .iter()
            .any(|t| de(t).as_deref() == Some(n))
        {
            out.insert(p);
        }
    }
    out.into_iter().collect()
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use std::path::PathBuf;

    fn escribir(d: &Path, rel: &str, t: &str) {
        let p = d.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, t).unwrap();
    }

    const MANIFIESTO: &str = "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\nmetadata: { name: t, version: 0.1.0 }\ndatasources:\n  - { name: pg, type: postgres, connectionEnv: PG_URL }\n";
    /// El conducto de la copia, autorizado como lo deja ore-serve al inducir:
    /// sin él, una database estándar es `OOS4011`.
    const CONDUCTOS: &str = "apiVersion: oos.dev/v1alpha1
kind: ConduitPolicy
metadata: { name: t }
spec:
  owner: team:t
  conduits:
    materialization.payload: {}
";
    const CATALOGO: &str = "{\"source\":\"pg\",\"tables\":[{\"name\":\"olist.customers\",\"columns\":[{\"name\":\"customer_id\",\"type\":\"String\"}]},{\"name\":\"olist.orders\",\"columns\":[{\"name\":\"order_id\",\"type\":\"String\"}]}]}";

    fn paquete(n: &str, estado: &str, extra: &str) -> String {
        format!(
            "apiVersion: oos.dev/v1alpha1\nkind: Package\nmetadata: {{ name: {n}, version: 0.1.0, status: {estado}, domain: {n} }}\nspec: {{ owner: team:t{extra} }}\n"
        )
    }

    fn schema(n: &str) -> String {
        format!(
            "apiVersion: oos.dev/v1alpha13\nkind: Schema\nmetadata: {{ name: olist, namespace: {n} }}\nspec: {{ owner: team:t }}\n"
        )
    }

    fn tabla(nombre: &str, ns: &str, objeto: &str, col: &str, clave: bool) -> String {
        let k = if clave {
            format!(", key: [{col}]")
        } else {
            String::new()
        };
        format!(
            "apiVersion: oos.dev/v1alpha13\nkind: Table\nmetadata: {{ name: {nombre}, namespace: {ns}, schema: olist }}\nspec:\n  datasource: pg\n  object: \"{objeto}\"\n  columns: {{ {col}: {{ type: String }} }}\n  reads: {{ fullScan: cheap }}\n  changes: {{ mode: none, witness: none{k} }}\n"
        )
    }

    /// ⭐ 0045: los punteros en la fuente (`pg`), exportados; `tienda` (estándar)
    /// copia `orders`; `espejo` (foránea) lee `customers` y, por SQL, `orders`.
    fn arbol_nuevo(nombre: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ore-punteros-{}-{nombre}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        escribir(&d, "ontology.config.yaml", MANIFIESTO);
        escribir(&d, "conduits.yaml", CONDUCTOS);
        escribir(
            &d,
            "packages/pg/package.yaml",
            &paquete(
                "pg",
                "draft",
                ", exports: [pg.olist.customers, pg.olist.orders]",
            ),
        );
        escribir(&d, "packages/pg/discover.catalog.json", CATALOGO);
        escribir(&d, "packages/pg/olist/schema.yaml", &schema("pg"));
        escribir(
            &d,
            "packages/pg/olist/tables/orders.yaml",
            &tabla("orders", "pg", "olist.orders", "order_id", true),
        );
        escribir(
            &d,
            "packages/pg/olist/tables/customers.yaml",
            &tabla("customers", "pg", "olist.customers", "customer_id", false),
        );
        escribir(
            &d,
            "packages/tienda/package.yaml",
            &paquete("tienda", "active", ""),
        );
        escribir(
            &d,
            "packages/tienda/discover.scope.json",
            "{\"source\":\"pg\",\"objects\":[\"olist.orders\"],\"type\":\"standard\"}",
        );
        escribir(&d, "packages/tienda/olist/schema.yaml", &schema("tienda"));
        escribir(
            &d,
            "packages/tienda/olist/datasets/orders.yaml",
            "apiVersion: oos.dev/v1alpha13\nkind: Dataset\nmetadata: { name: orders, namespace: tienda, schema: olist }\nspec:\n  owner: team:t\n  from: { table: pg.olist.orders }\n  fields: { order_id: order_id }\n",
        );
        escribir(
            &d,
            "packages/espejo/package.yaml",
            &paquete("espejo", "active", ""),
        );
        escribir(
            &d,
            "packages/espejo/discover.scope.json",
            "{\"source\":\"pg\",\"objects\":[\"olist.customers\",\"olist.orders\"]}",
        );
        escribir(&d, "packages/espejo/olist/schema.yaml", &schema("espejo"));
        escribir(
            &d,
            "packages/espejo/olist/views/customers.yaml",
            "apiVersion: oos.dev/v1alpha13\nkind: View\nmetadata: { name: customers, namespace: espejo, schema: olist }\nspec:\n  owner: team:t\n  from: { table: pg.olist.customers }\n  fields: { customer_id: customer_id }\n",
        );
        escribir(
            &d,
            "packages/espejo/olist/views/orders.yaml",
            "apiVersion: oos.dev/v1alpha14\nkind: View\nmetadata: { name: orders, namespace: espejo, schema: olist }\nspec:\n  owner: team:t\n  dialect: duckdb\n  sql: |\n    SELECT order_id FROM pg.olist.orders\n  columns:\n    order_id: { type: String }\n",
        );
        d
    }

    /// La de hoy: el puntero en la database (`orders_t`), y la fuente con solo
    /// su catálogo.
    fn arbol_de_hoy(nombre: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("ore-punteros-hoy-{}-{nombre}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        escribir(&d, "ontology.config.yaml", MANIFIESTO);
        escribir(&d, "conduits.yaml", CONDUCTOS);
        escribir(&d, "packages/pg/package.yaml", &paquete("pg", "draft", ""));
        escribir(&d, "packages/pg/discover.catalog.json", CATALOGO);
        escribir(
            &d,
            "packages/tienda/package.yaml",
            &paquete("tienda", "active", ""),
        );
        escribir(
            &d,
            "packages/tienda/discover.scope.json",
            "{\"source\":\"pg\",\"objects\":[\"olist.orders\"],\"type\":\"standard\"}",
        );
        escribir(&d, "packages/tienda/olist/schema.yaml", &schema("tienda"));
        escribir(
            &d,
            "packages/tienda/olist/tables/orders.yaml",
            &tabla("orders_t", "tienda", "olist.orders", "order_id", true),
        );
        escribir(
            &d,
            "packages/tienda/olist/datasets/orders.yaml",
            "apiVersion: oos.dev/v1alpha13\nkind: Dataset\nmetadata: { name: orders, namespace: tienda, schema: olist }\nspec:\n  owner: team:t\n  from: { table: orders_t }\n  fields: { order_id: order_id }\n",
        );
        d
    }

    /// El árbol, cargado — y comprobado: que compile sin un solo diagnóstico
    /// es lo que hace de él una prueba de la disposición y no un dibujo.
    fn cargar(d: &Path) -> Package {
        let diags: Vec<String> = ore_core::validate::validate_package(d)
            .iter()
            .map(|x| format!("{:?}: {}", x.code, x.message))
            .collect();
        assert!(
            diags.is_empty(),
            "el árbol de prueba no compila: {diags:#?}"
        );
        ore_core::validate::cargar_paquete(d).0
    }

    #[test]
    fn la_fuente_se_reconoce_por_su_catalogo_y_sin_alcance() {
        let d = arbol_nuevo("fuente");
        assert!(es_paquete_de_fuente(&d.join("packages/pg")));
        assert!(!es_paquete_de_fuente(&d.join("packages/tienda")));
        assert!(!es_paquete_de_fuente(&d.join("packages/espejo")));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn la_tabla_que_lee_se_resuelve_en_otro_paquete() {
        let d = arbol_nuevo("resuelve");
        let pkg = cargar(&d);
        let de = |n: &str| {
            pkg.docs
                .iter()
                .find(|x| x.qname().as_deref() == Some(n))
                .unwrap()
        };
        for lector in ["tienda.olist.orders", "espejo.olist.orders"] {
            let l = pkg
                .docs
                .iter()
                .find(|x| x.qname().as_deref() == Some(lector) && x.kind != Kind::Table)
                .unwrap();
            let t =
                tabla_que_lee(&pkg, l).unwrap_or_else(|| panic!("{lector} no resuelve su tabla"));
            assert_eq!(t.qname().as_deref(), Some("pg.olist.orders"), "{lector}");
            assert_eq!(paquete_de(&d, t).as_deref(), Some("pg"));
        }
        assert_eq!(
            tabla_que_lee(&pkg, de("espejo.olist.customers"))
                .and_then(|t| t.qname())
                .as_deref(),
            Some("pg.olist.customers")
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Las tres maneras de salir de una fuente cuentan, en las dos disposiciones.
    #[test]
    fn las_bases_que_salen_de_una_fuente() {
        let d = arbol_nuevo("bases");
        assert_eq!(
            bases_que_salen_de(&d, &cargar(&d), "pg"),
            ["espejo", "tienda"]
        );
        // (c) sola: sin alcances, se ve por lo que leen.
        std::fs::remove_file(d.join("packages/espejo/discover.scope.json")).unwrap();
        std::fs::remove_file(d.join("packages/tienda/discover.scope.json")).unwrap();
        assert_eq!(
            bases_que_salen_de(&d, &cargar(&d), "pg"),
            ["espejo", "tienda"]
        );
        let _ = std::fs::remove_dir_all(&d);

        let h = arbol_de_hoy("bases");
        assert_eq!(bases_que_salen_de(&h, &cargar(&h), "pg"), ["tienda"]);
        // (b) sola: la Table con `datasource: pg`, sin alcance.
        std::fs::remove_file(h.join("packages/tienda/discover.scope.json")).unwrap();
        assert_eq!(bases_que_salen_de(&h, &cargar(&h), "pg"), ["tienda"]);
        let _ = std::fs::remove_dir_all(&h);
    }
}
