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
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::rc::Rc;

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
/// La referencia de [`Indice::es_fuente`], que es lo que se usa: la prueba
/// `el_indice_dice_lo_mismo` las compara.
#[cfg(test)]
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

/// v1alpha16. **Los `ObjectTable` que lee un documento**: el origen de una
/// colección mantenida, o los que nombra la consulta de una vista SQL. Son
/// punteros de la fuente como las `Table` (0045), y retirarla los retira.
pub fn objetos_que_lee<'a>(pkg: &'a Package, d: &'a Loaded) -> Vec<&'a Loaded> {
    if d.kind == Kind::MediaCollection {
        return d
            .section("from")
            .and_then(|f| f.get("objectTable"))
            .and_then(|(_, r)| r.as_str())
            .and_then(|r| pkg.resolve_object_table(r, d))
            .into_iter()
            .collect();
    }
    let mut out: Vec<&Loaded> = Vec::new();
    if vistas::es_sql(d) {
        for n in ore_core::servir::nombrados(pkg, d) {
            if n.doc.kind == Kind::ObjectTable && !out.iter().any(|x| x.path == n.doc.path) {
                out.push(n.doc);
            }
        }
    }
    out
}

/// **La** tabla que lee: la de `from.table`, o la de una vista SQL que lee UNA
/// sola cosa y es una tabla. Es lo que el catálogo enseña como «su tabla».
/// La referencia de [`Indice::tabla_que_lee`].
#[cfg(test)]
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
    // (b) · v1alpha16: un `ObjectTable` es un puntero de la fuente como una
    // tabla, y cuenta igual.
    for t in pkg
        .docs
        .iter()
        .filter(|d| matches!(d.kind, Kind::Table | Kind::ObjectTable))
    {
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
            .into_iter()
            .chain(objetos_que_lee(pkg, d))
            .any(|t| de(t).as_deref() == Some(n))
        {
            out.insert(p);
        }
    }
    out.into_iter().collect()
}

// ── El índice de una petición (0046 E5b) ────────────────────────────────────
//
// Medido con un origen de 2.000 tablas (0046 E5b): el esquema de la fuente
// tardaba 98 s y `GET /paquetes` 105 s, y 85–91 s de ellos eran UN bucle —por
// cada fila del catálogo, todos los documentos del árbol, y en cada uno otra
// vez lo que lee (una vista SQL: su consulta, analizada de nuevo)—. Y cada
// fuente lo repetía, y analizaba su catálogo tres veces. Esto recorre el
// árbol UNA vez por petición y lo deja en mapas; quien pinta pregunta al
// mapa. Lo que responde es lo mismo que antes: solo cambia cuántas veces se
// calcula.

/// Lo que una petición pregunta del árbol, calculado una vez.
pub struct Indice<'a> {
    raiz: PathBuf,
    pkg: &'a Package,
    /// documento → su paquete (la carpeta bajo `packages/`).
    paquete: HashMap<&'a Path, String>,
    /// documento por su ruta.
    por_ruta: HashMap<&'a Path, &'a Loaded>,
    /// (paquete, `object`) → su `Table`.
    tablas: HashMap<(String, String), &'a Loaded>,
    /// (paquete, `prefix`, `match`) → su `ObjectTable`.
    objetos: HashMap<(String, String, Option<String>), &'a Loaded>,
    /// puntero → los paquetes cuyos documentos lo leen (vistas, datasets,
    /// colecciones), el suyo incluido.
    lectores: HashMap<&'a Path, BTreeSet<String>>,
    /// documento → la única tabla que lee, si lee una (`tabla_que_lee`).
    unica: HashMap<&'a Path, &'a Loaded>,
    /// (`datasource`, `object`) → los paquetes que tienen una `Table` de él.
    con_tabla: HashMap<(String, String), BTreeSet<String>>,
    /// Los catálogos y alcances ya leídos, por su ruta: un catálogo de 2.000
    /// tablas son 0,3 s de análisis, y una petición lo pedía tres veces.
    leidos: RefCell<HashMap<PathBuf, Option<Rc<parse::Node>>>>,
}

impl<'a> Indice<'a> {
    pub fn nuevo(raiz: &Path, pkg: &'a Package) -> Self {
        let mut i = Indice {
            raiz: raiz.to_path_buf(),
            pkg,
            paquete: HashMap::new(),
            por_ruta: HashMap::new(),
            tablas: HashMap::new(),
            objetos: HashMap::new(),
            lectores: HashMap::new(),
            unica: HashMap::new(),
            con_tabla: HashMap::new(),
            leidos: RefCell::new(HashMap::new()),
        };
        // Las tablas por su nombre cualificado: `pkg.table` es una búsqueda
        // lineal, y se pedía una por documento.
        let mut por_nombre: HashMap<String, &'a Loaded> = HashMap::new();
        for d in &pkg.docs {
            i.por_ruta.insert(d.path.as_path(), d);
            if let Some(p) = paquete_de(raiz, d) {
                i.paquete.insert(d.path.as_path(), p);
            }
            let texto = |k: &str| d.section(k).and_then(|v| v.as_str()).map(String::from);
            let suyo = i.paquete.get(d.path.as_path()).cloned();
            match d.kind {
                Kind::Table => {
                    if let Some(q) = d.qname() {
                        por_nombre.entry(q).or_insert(d);
                    }
                    if let (Some(o), Some(p)) = (texto("object"), suyo) {
                        i.tablas.entry((p.clone(), o.clone())).or_insert(d);
                        if let Some(f) = texto("datasource") {
                            i.con_tabla.entry((f, o)).or_default().insert(p);
                        }
                    }
                }
                Kind::ObjectTable => {
                    if let (Some(p), Some(pre)) = (suyo, texto("prefix")) {
                        i.objetos.entry((p, pre, texto("match"))).or_insert(d);
                    }
                }
                _ => {}
            }
        }
        // Lo que lee cada documento, UNA vez: la consulta de una vista SQL se
        // analiza aquí y no por cada fila que pregunte.
        for d in &pkg.docs {
            let leidos = lee(pkg, d, &por_nombre);
            if let Some(p) = i.paquete.get(d.path.as_path()) {
                for x in leidos
                    .iter()
                    .filter(|x| matches!(x.kind, Kind::Table | Kind::ObjectTable))
                {
                    i.lectores
                        .entry(x.path.as_path())
                        .or_default()
                        .insert(p.clone());
                }
            }
            let mut distintos: Vec<&Loaded> = Vec::new();
            for x in &leidos {
                if !distintos.iter().any(|y| y.path == x.path) {
                    distintos.push(x);
                }
            }
            if let [uno] = distintos.as_slice()
                && uno.kind == Kind::Table
            {
                i.unica.insert(d.path.as_path(), uno);
            }
        }
        i
    }

    /// El árbol del que es el índice.
    pub fn arbol(&self) -> &'a Package {
        self.pkg
    }

    pub fn raiz(&self) -> &Path {
        &self.raiz
    }

    /// El paquete de un documento.
    pub fn paquete(&self, d: &Loaded) -> Option<&str> {
        self.paquete.get(d.path.as_path()).map(String::as_str)
    }

    pub fn por_ruta(&self, p: &Path) -> Option<&'a Loaded> {
        self.por_ruta.get(p).copied()
    }

    /// La `Table` de `paquete` que apunta a `object`.
    pub fn tabla(&self, paquete: &str, object: &str) -> Option<&'a Loaded> {
        self.tablas
            .get(&(paquete.to_string(), object.to_string()))
            .copied()
    }

    /// El `ObjectTable` de `paquete` con ese prefijo y patrón.
    pub fn objetos(
        &self,
        paquete: &str,
        prefijo: &str,
        patron: Option<&str>,
    ) -> Option<&'a Loaded> {
        self.objetos
            .get(&(
                paquete.to_string(),
                prefijo.to_string(),
                patron.map(String::from),
            ))
            .copied()
    }

    /// Los paquetes que leen este puntero.
    pub fn lectores(&self, puntero: &Loaded) -> impl Iterator<Item = &str> {
        self.lectores
            .get(puntero.path.as_path())
            .into_iter()
            .flatten()
            .map(String::as_str)
    }

    /// Los paquetes con una `Table` de (`datasource`, `object`).
    pub fn con_tabla(&self, fuente: &str, object: &str) -> impl Iterator<Item = &str> {
        self.con_tabla
            .get(&(fuente.to_string(), object.to_string()))
            .into_iter()
            .flatten()
            .map(String::as_str)
    }

    /// La única tabla que lee un documento (`tabla_que_lee`), ya calculada.
    pub fn tabla_que_lee(&self, d: &Loaded) -> Option<&'a Loaded> {
        self.unica.get(d.path.as_path()).copied()
    }

    /// Un JSON del árbol analizado, una vez por petición (catálogos y
    /// alcances). `None` si no está o no analiza.
    pub fn leer(&self, ruta: &Path) -> Option<Rc<parse::Node>> {
        if let Some(n) = self.leidos.borrow().get(ruta) {
            return n.clone();
        }
        let n = std::fs::read_to_string(ruta)
            .ok()
            .and_then(|t| parse::parse(&t).ok())
            .map(Rc::new);
        self.leidos
            .borrow_mut()
            .insert(ruta.to_path_buf(), n.clone());
        n
    }

    /// `es_paquete_de_fuente`, con el catálogo leído una vez.
    pub fn es_fuente(&self, dir: &Path) -> bool {
        if dir.join("discover.scope.json").is_file() {
            return false;
        }
        let Some(n) = dir.file_name().and_then(|x| x.to_str()) else {
            return false;
        };
        self.leer(&dir.join("discover.catalog.json"))
            .and_then(|c| {
                c.get("source")
                    .and_then(|(_, v)| v.as_str().map(|s| s == n))
            })
            .unwrap_or(false)
    }

    /// De qué fuente sale un paquete y si se eligió: `source` del alcance, o
    /// del catálogo si no lo tiene (un paquete escrito a mano, ninguna).
    pub fn origen(&self, dir: &Path) -> (Option<String>, bool) {
        let fuente = |f: &str| {
            self.leer(&dir.join(f)).and_then(|n| {
                n.get("source")
                    .and_then(|(_, v)| v.as_str().map(String::from))
            })
        };
        if let Some(f) = fuente("discover.scope.json") {
            return (Some(f), true);
        }
        (fuente("discover.catalog.json"), false)
    }

    /// Las bases de `fuente` que eligieron `objeto` (su `only`).
    pub fn eligen(&self, fuente: &str, objeto: &str) -> Vec<String> {
        let mut out = Vec::new();
        let Ok(es) = std::fs::read_dir(self.raiz.join("packages")) else {
            return out;
        };
        for e in es.flatten() {
            let Some(a) = self.leer(&e.path().join("discover.scope.json")) else {
                continue;
            };
            let de_ella = a.get("source").and_then(|(_, v)| v.as_str()) == Some(fuente);
            let la_elige = a
                .get("only")
                .map(|(_, v)| v.items())
                .unwrap_or(&[])
                .iter()
                .any(|x| x.as_str() == Some(objeto));
            if de_ella && la_elige {
                out.push(e.file_name().to_string_lossy().into_owned());
            }
        }
        out
    }
}

/// Todo lo que nombra un documento como su entrada —una tabla, un
/// `ObjectTable`, o lo que nombre su consulta SQL, analizada una sola vez—.
fn lee<'a>(
    pkg: &'a Package,
    d: &'a Loaded,
    por_nombre: &HashMap<String, &'a Loaded>,
) -> Vec<&'a Loaded> {
    if let Some(Fuente::Tabla(qn)) = vistas::fuente(d) {
        return por_nombre
            .get(ore_core::normalize::a_corto(&qn).as_ref())
            .copied()
            .into_iter()
            .collect();
    }
    if d.kind == Kind::MediaCollection {
        return objetos_que_lee(pkg, d);
    }
    if vistas::es_sql(d) {
        return ore_core::servir::nombrados(pkg, d)
            .into_iter()
            .map(|n| n.doc)
            .collect();
    }
    Vec::new()
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

    /// ⭐ 0046 E5b · **El índice dice lo mismo que las funciones a las que
    /// sustituye**, documento a documento: la tabla que lee cada uno, las
    /// fuentes y quién lee cada puntero (`tablas_que_lee` recorrido entero,
    /// como hacía el bucle de antes).
    #[test]
    fn el_indice_dice_lo_mismo() {
        let d = arbol_nuevo("indice");
        let pkg = cargar(&d);
        let idx = Indice::nuevo(&d, &pkg);
        for x in &pkg.docs {
            assert_eq!(
                idx.tabla_que_lee(x).map(|t| &t.path),
                tabla_que_lee(&pkg, x).map(|t| &t.path),
                "{}",
                x.path.display()
            );
            if x.kind == Kind::Table {
                let antes: BTreeSet<String> = pkg
                    .docs
                    .iter()
                    .filter(|y| tablas_que_lee(&pkg, y).iter().any(|t| t.path == x.path))
                    .filter_map(|y| paquete_de(&d, y))
                    .collect();
                let ahora: BTreeSet<String> = idx.lectores(x).map(String::from).collect();
                assert_eq!(ahora, antes, "{}", x.path.display());
            }
        }
        for p in ["pg", "tienda", "espejo"] {
            let dir = d.join("packages").join(p);
            assert_eq!(idx.es_fuente(&dir), es_paquete_de_fuente(&dir), "{p}");
        }
        let _ = std::fs::remove_dir_all(&d);
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

    /// ⭐ 0046 E3: el `ObjectTable` es un puntero de la fuente como una tabla.
    /// Una colección que sale de él y una vista que lo pregunta salen de la
    /// fuente, y retirarla sin verlas dejaría subir un árbol roto: (b) y (c)
    /// solo miraban `Table`, y sin alcances no veían a ninguna de las dos.
    #[test]
    fn lo_que_sale_de_una_fuente_de_objetos() {
        let d = std::env::temp_dir().join(format!("ore-punteros-s3-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        escribir(
            &d,
            "ontology.config.yaml",
            "apiVersion: oos.dev/v1alpha1
kind: OntologyConfig
metadata: { name: t, version: 0.1.0 }
datasources:
  - { name: s3, type: s3, connectionEnv: S3_URL }
",
        );
        escribir(&d, "conduits.yaml", CONDUCTOS);
        escribir(
            &d,
            "packages/s3/package.yaml",
            &paquete("s3", "draft", ", exports: [s3.docs.contratos]"),
        );
        escribir(
            &d,
            "packages/s3/docs/schema.yaml",
            "apiVersion: oos.dev/v1alpha13
kind: Schema
metadata: { name: docs, namespace: s3 }
",
        );
        escribir(
            &d,
            "packages/s3/docs/objects/contratos.yaml",
            "apiVersion: oos.dev/v1alpha16
kind: ObjectTable
metadata: { name: contratos, namespace: s3, schema: docs }
spec:
  datasource: s3
  prefix: \"contratos/\"
  media: document
  changes: { mode: retract, witness: listing }
",
        );
        escribir(
            &d,
            "packages/legal/package.yaml",
            &paquete("legal", "active", ""),
        );
        escribir(
            &d,
            "packages/legal/collections/contratos.yaml",
            "apiVersion: oos.dev/v1alpha16
kind: MediaCollection
metadata: { name: contratos, namespace: legal }
spec:
  owner: team:t
  media: document
  formats: [pdf]
  from: { objectTable: s3.docs.contratos }
  virtual: true
",
        );
        escribir(
            &d,
            "packages/espejo/package.yaml",
            &paquete("espejo", "active", ""),
        );
        escribir(
            &d,
            "packages/espejo/views/inventario.yaml",
            "apiVersion: oos.dev/v1alpha16
kind: View
metadata: { name: inventario, namespace: espejo }
spec:
  owner: team:t
  dialect: duckdb
  sql: |
    SELECT key, size FROM s3.docs.contratos
  columns:
    key: { type: String }
    size: { type: Integer }
",
        );
        assert_eq!(
            bases_que_salen_de(&d, &cargar(&d), "s3"),
            ["espejo", "legal"]
        );
        let _ = std::fs::remove_dir_all(&d);
    }
}
