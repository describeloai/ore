//! El SQL del árbol contra un árbol compilado: lo que se lee se puede leer y
//! lo que se escribe se puede escribir, con las reglas de `datos_de`.

use ore_core::sql_del_arbol::{Fallo, analizar, cotejar};
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
    let t = Arbol(
        std::env::temp_dir().join(format!("ore-sql-del-arbol-{}-{caso}", std::process::id())),
    );
    let _ = fs::remove_dir_all(&t.0);
    let r = &t.0;
    escribe(
        r,
        "ontology.config.yaml",
        "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\nmetadata: { name: fuego, version: 0.1.0 }\ndatasources:\n  - { name: pg, type: postgres, connectionEnv: PG_URL }\n",
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
        "apiVersion: oos.dev/v1alpha12\nkind: Dataset\nmetadata: { name: pedidos, namespace: ventas }\nspec:\n  owner: team:ventas\n  from: { table: ventas.pedidos_t }\n  freshness: 1h\n",
    );
    escribe(
        r,
        "packages/ventas/views/pedidosEs.yaml",
        "apiVersion: oos.dev/v1alpha12\nkind: View\nmetadata: { name: pedidosEs, namespace: ventas }\nspec:\n  owner: team:ventas\n  from: { dataset: ventas.pedidos }\n  where: { pais: ES }\n  fields: { id: id, pais: pais }\n",
    );
    escribe(
        r,
        "packages/ventas/views/virtual.yaml",
        "apiVersion: oos.dev/v1alpha12\nkind: View\nmetadata: { name: virtual, namespace: ventas }\nspec:\n  owner: team:ventas\n  from: { table: ventas.pedidos_t }\n  fields: { id: id }\n",
    );
    escribe(
        r,
        "packages/ventas/datasets/resumen.yaml",
        "apiVersion: oos.dev/v1alpha12\nkind: Dataset\nmetadata: { name: resumen, namespace: ventas }\nspec:\n  owner: team:ventas\n  columns:\n    pais: { type: String }\n    n: { type: Integer }\n  changes: { mode: append }\n",
    );
    t
}

fn fallos(raiz: &Path, q: &str) -> Vec<Fallo> {
    let (pkg, diags) = ore_core::validate::cargar_paquete(raiz);
    assert!(diags.is_empty(), "el árbol de fuego no compila: {diags:?}");
    cotejar(&pkg, &analizar(q).unwrap())
}

#[test]
fn se_lee_un_dataset_o_una_vista_que_sale_de_uno() {
    let a = arbol("se-lee");
    assert_eq!(
        fallos(
            &a.0,
            "select * from ventas.pedidos join ventas.pedidosEs using (id)"
        ),
        vec![]
    );
    assert_eq!(
        fallos(
            &a.0,
            "create or replace table ventas.resumen as select pais, count(*) as n from ventas.pedidosEs group by all"
        ),
        vec![]
    );
    // lo que se escribe puede no existir todavía: nace al escribirse
    assert_eq!(
        fallos(
            &a.0,
            "insert into ventas.nuevo select * from ventas.pedidos"
        ),
        vec![]
    );
}

#[test]
fn lo_que_no_se_lee_ni_se_escribe_se_dice() {
    let a = arbol("no-se-lee");
    let f = fallos(&a.0, "select * from ventas.pedidos_t");
    assert!(f[0].mensaje.contains("`Table` de una fuente"), "{f:?}");
    let f = fallos(&a.0, "select * from ventas.virtual");
    assert!(f[0].mensaje.contains("`View` virtual"), "{f:?}");
    let f = fallos(&a.0, "select * from ventas.nadie");
    assert!(
        f[0].mensaje
            .contains("no hay ningún `Dataset` ni `View` `ventas.nadie`"),
        "{f:?}"
    );
    let f = fallos(&a.0, "select * from compras.pedidos");
    assert!(f[0].mensaje.contains("ningún paquete `compras`"), "{f:?}");
    let f = fallos(
        &a.0,
        "insert into ventas.pedidos select * from ventas.pedidosEs",
    );
    assert!(f[0].mensaje.contains("mantenido"), "{f:?}");
    let f = fallos(
        &a.0,
        "create or replace table ventas.pedidosEs as select * from ventas.pedidos",
    );
    assert!(
        f[0].mensaje
            .contains("lo que un `.sql` escribe es un `Dataset`"),
        "{f:?}"
    );
}

/// Lo que `sql()` resuelve de una celda: el tokenizador y el árbol como filtro
/// (medida-el-terreno-de-la-regex.py: 52 de 52).
#[test]
fn los_nombres_de_una_celda_los_decide_el_arbol() {
    let a = arbol("celda");
    let (pkg, _) = ore_core::validate::cargar_paquete(&a.0);
    let n = |q: &str| ore_core::sql_del_arbol::nombres_a_resolver(q, &pkg);
    // lo que la regex fallaba: comentario, cadena, `from a, b`, comillas, tres partes
    assert_eq!(
        n("-- de ventas.viejo\nselect * from ventas.pedidos"),
        ["ventas.pedidos"]
    );
    assert_eq!(
        n("select 'from ventas.resumen' from ventas.pedidos"),
        ["ventas.pedidos"]
    );
    assert_eq!(
        n("select * from ventas.pedidos, ventas.resumen"),
        ["ventas.pedidos", "ventas.resumen"]
    );
    assert_eq!(
        n("select * from \"ventas\".\"pedidosEs\""),
        ["ventas.pedidosEs"]
    );
    assert_eq!(n("select * from lago.ventas.pedidos"), Vec::<String>::new());
    // lo que el parser no analiza, el tokenizador sí
    assert_eq!(
        n("pivot ventas.pedidos on pais using count(*)"),
        ["ventas.pedidos"]
    );
    assert_eq!(n("summarize ventas.resumen"), ["ventas.resumen"]);
    // una errata tras FROM se resuelve (y será el 404 de siempre)
    assert_eq!(n("select * from ventas.nadie"), ["ventas.nadie"]);
    // un alias con el nombre de un paquete, en la lista de columnas, no
    assert_eq!(
        n("select ventas.pais from ventas.pedidos as ventas"),
        ["ventas.pedidos"]
    );
    // un esquema de la sesión es del motor
    assert_eq!(
        n("create schema tmp; create table tmp.t as select 1; select * from tmp.t"),
        Vec::<String>::new()
    );
    // la Table se resuelve para que su 409 diga cómo se lee
    assert_eq!(n("select * from ventas.pedidos_t"), ["ventas.pedidos_t"]);
}
