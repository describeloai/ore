//! El SQL del árbol contra un árbol compilado: lo que se lee se puede leer y
//! lo que se escribe se puede escribir, con las reglas de `datos_de`.

use ore_core::sql_del_arbol::guion::{cotejar_guion, guion};
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
    // 0038: el schema `espana`, declarado, con un dataset escrito
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
            "create or replace dataset ventas.resumen as select pais, count(*) as n from ventas.pedidosEs group by all"
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
        "create or replace dataset ventas.pedidosEs as select * from ventas.pedidos",
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
    // tres partes (0038): en su forma corta; `default` es la de dos
    assert_eq!(
        n("select * from ventas.default.pedidos join ventas.espana.clientes using (id)"),
        ["ventas.espana.clientes", "ventas.pedidos"]
    );
    assert_eq!(
        n("select * from ventas.espana.nadie"),
        ["ventas.espana.nadie"]
    );
    // una columna cualificada con su tabla no es un nombre que leer
    assert_eq!(
        n("select ventas.pedidos.id from ventas.pedidos"),
        ["ventas.pedidos"]
    );
    assert_eq!(n("select * from a.b.c.d"), Vec::<String>::new());
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

/// Qué celda de la sesión escribe en el árbol (medida-el-sql-que-escribe.sh
/// §3): lo decide el destino —un `paquete.nombre` de un paquete del árbol—, y
/// con el tokenizador, que también ve lo que el parser no analiza.
#[test]
fn una_celda_escribe_en_el_arbol_si_su_destino_es_de_un_paquete() {
    use ore_core::sql_del_arbol::{EscribeEnElArbol as E, escribe_en_el_arbol};
    let a = arbol("escribe");
    let (pkg, _) = ore_core::validate::cargar_paquete(&a.0);
    let e = |q: &str| escribe_en_el_arbol(q, &pkg);
    let t = |n: &str| Some(E::Tabla(n.to_string()));
    assert_eq!(
        e("create or replace table ventas.x as select * from ventas.pedidos"),
        t("ventas.x")
    );
    assert_eq!(
        e("CREATE TABLE IF NOT EXISTS ventas.x AS SELECT 1"),
        t("ventas.x")
    );
    assert_eq!(
        e("insert into ventas.resumen select 'ES', 1"),
        t("ventas.resumen")
    );
    assert_eq!(
        e("insert or replace into ventas.resumen select 'ES', 1"),
        t("ventas.resumen")
    );
    assert_eq!(
        e("insert into \"ventas\".\"x\" by name select 1 as a"),
        t("ventas.x")
    );
    assert_eq!(
        e("create or replace table ventas.x as pivot ventas.pedidos on pais using count(*)"),
        t("ventas.x")
    );
    assert_eq!(e("create temp table ventas.x as select 1"), t("ventas.x"));
    // lo que se escribe es un dataset; `table` también llega aquí, y el
    // análisis dice que una Table no se crea desde SQL
    assert_eq!(
        e("create or replace dataset ventas.x as select 1"),
        t("ventas.x")
    );
    // varias sentencias: se ve igual (y será «una sentencia» al analizar)
    assert_eq!(
        e("select 1; create or replace table ventas.x as select 1"),
        t("ventas.x")
    );
    assert_eq!(
        e("create or replace view ventas.v as select 1"),
        Some(E::Vista("ventas.v".into()))
    );
    // ADR 0040 paso 5: quitarla también es del árbol; `drop view tmp` no
    assert_eq!(
        e("drop view if exists ventas.espana.v"),
        Some(E::Vista("ventas.espana.v".into()))
    );
    assert_eq!(e("drop view v"), None);
    // 0039: lo que crea en el catálogo; `create schema tmp` sigue siendo de DuckDB
    let c = |n: &str| Some(E::Crea(n.to_string()));
    assert_eq!(
        e("CREATE SCHEMA IF NOT EXISTS ventas.demo"),
        c("schema ventas.demo")
    );
    assert_eq!(e("create standard database mi_base"), c("database mi_base"));
    assert_eq!(
        e("create foreign database if not exists espejo from origin erp include (s.*)"),
        c("database espejo")
    );
    assert_eq!(e("create database nueva"), c("database nueva"));
    assert_eq!(e("create schema nada.demo"), None);
    // lo que es de la sesión, de DuckDB
    assert_eq!(
        e("create schema tmp; create table tmp.t as select 1; select * from tmp.t"),
        None
    );
    assert_eq!(e("create table x as select 1"), None);
    assert_eq!(e("create or replace table nada.x as select 1"), None);
    assert_eq!(e("create table lago.ventas.x as select 1"), None);
    // tres partes (0038), en su forma corta
    assert_eq!(
        e("create or replace table ventas.espana.x as select 1"),
        t("ventas.espana.x")
    );
    assert_eq!(
        e("insert into ventas.default.resumen select 'ES', 1"),
        t("ventas.resumen")
    );
    assert_eq!(e("create table ventas.a.b.c as select 1"), None);
    assert_eq!(e("select * from ventas.pedidos"), None);
    assert_eq!(e("-- create table ventas.x as select 1\nselect 1"), None);
    assert_eq!(e("select 'insert into ventas.x' as s"), None);
}

/// 0038: tres partes contra el árbol. El schema tiene que estar declarado; lo
/// que hay en él se lee por su nombre de tres partes; y dos partes es `default`.
#[test]
fn tres_partes_contra_el_arbol() {
    let a = arbol("tres");
    let (pkg, _) = ore_core::validate::cargar_paquete(&a.0);
    let coteja = |q: &str| cotejar(&pkg, &analizar(q).unwrap_or_else(|f| panic!("{q}: {f:?}")));
    assert_eq!(
        coteja(
            "create or replace dataset ventas.espana.r as select * from ventas.espana.clientes join ventas.default.pedidos using (id)"
        ),
        Vec::<Fallo>::new()
    );
    let f = coteja("select * from ventas.francia.clientes");
    assert!(
        f.len() == 1
            && f[0]
                .mensaje
                .contains("no hay ningún schema `francia` en la base `ventas`"),
        "{f:?}"
    );
    let f = coteja("create or replace dataset ventas.francia.x as select 1");
    assert!(f[0].mensaje.contains("schema `francia`"), "{f:?}");
    // `ventas.clientes` es `ventas.default.clientes`, que no está: el de `espana` no se adivina
    let f = coteja("select * from ventas.clientes");
    assert!(f[0].mensaje.contains("`ventas.clientes`"), "{f:?}");
    let f = coteja("select * from ventas.espana.nadie");
    assert!(f[0].mensaje.contains("`ventas.espana.nadie`"), "{f:?}");
}

/// Los avisos de una celda `sql` (0038): un `ORE-SQL-2P` por cada nombre del
/// árbol con dos partes —lo que se lee y el destino—, en su sitio, una vez.
#[test]
fn los_avisos_de_una_celda() {
    use ore_core::sql_del_arbol::{DOS_PARTES, avisos_de_celda};
    let a = arbol("avisos");
    let (pkg, _) = ore_core::validate::cargar_paquete(&a.0);
    let av = |q: &str| {
        avisos_de_celda(q, &pkg)
            .into_iter()
            .map(|f| (f.codigo, f.pos.map(|p| (p.line, p.col)), f.mensaje))
            .collect::<Vec<_>>()
    };
    let r =
        av("insert into ventas.nuevo\nselect * from ventas.pedidos join ventas.pedidos using (id)");
    assert_eq!(r.len(), 2, "{r:?}");
    assert_eq!((r[0].0, r[0].1), (Some(DOS_PARTES), Some((1, 13))));
    assert!(r[0].2.contains("`ventas.default.nuevo`"), "{r:?}");
    assert_eq!(r[1].1, Some((2, 15)));
    // tres partes, un esquema de la sesión, una columna: nada
    assert!(
        av("select * from ventas.default.pedidos join ventas.espana.clientes using (id)")
            .is_empty()
    );
    assert!(av("create table tmp.t as select 1; select * from tmp.t").is_empty());
    assert!(av("select ventas.pais from ventas.default.pedidos as ventas").is_empty());
}

/// **Un guion se coteja en orden**: lo que crea la sentencia 1 —una base, un
/// schema, un dataset— existe para la 2; lo que ya había, se dice.
#[test]
fn un_guion_se_coteja_en_orden() {
    let a = arbol("guion");
    let (pkg, _) = ore_core::validate::cargar_paquete(&a.0);
    let coteja = |q: &str| cotejar_guion(&pkg, &guion(q).unwrap_or_else(|f| panic!("{q}: {f:?}")));
    // el ejemplo: el schema, el dataset vacío, el insert y el select
    assert_eq!(
        coteja(
            "create schema if not exists ventas.demo;
             create dataset if not exists ventas.demo.clientes (id bigint, nombre string);
             insert into ventas.demo.clientes (id, nombre) values (1, 'Ana');
             select * from ventas.demo.clientes"
        ),
        Vec::<Fallo>::new()
    );
    // y una base nueva entera, con lo que se escribe en ella y se lee luego
    assert_eq!(
        coteja(
            "create database mi_base;
             create schema mi_base.s;
             create or replace dataset mi_base.s.r as select * from ventas.pedidos;
             select count(*) from mi_base.s.r"
        ),
        Vec::<Fallo>::new()
    );
    // sin las sentencias de antes, lo mismo no existe
    let f = coteja("select * from ventas.demo.clientes");
    assert!(f[0].mensaje.contains("schema `demo`"), "{f:?}");
    let f = coteja("create dataset ventas.demo.x (a int)");
    assert!(f[0].mensaje.contains("schema `demo`"), "{f:?}");
    assert!(
        f[0].ayuda
            .as_deref()
            .unwrap()
            .contains("create schema ventas.demo")
    );
    let f = coteja("create schema otra.s");
    assert!(f[0].mensaje.contains("ninguna base `otra`"), "{f:?}");

    // lo que ya hay, en su línea; con `if not exists`, nada
    let f = coteja(
        "select 1;
create schema ventas.espana",
    );
    assert!(
        f.len() == 1 && f[0].mensaje.contains("ya hay un schema `ventas.espana`"),
        "{f:?}"
    );
    assert_eq!(f[0].pos.map(|p| p.line), Some(2));
    assert!(coteja("create schema if not exists ventas.espana").is_empty());
    let f = coteja("create database ventas");
    assert!(f[0].mensaje.contains("ya hay una base `ventas`"), "{f:?}");
    let f = coteja("create dataset ventas.espana.clientes (id int)");
    assert!(f[0].mensaje.contains("ya hay un dataset"), "{f:?}");
    assert!(coteja("create dataset if not exists ventas.espana.clientes (id int)").is_empty());
    let f = coteja("create dataset if not exists ventas.pedidos (id int)");
    assert!(f[0].mensaje.contains("mantenido"), "{f:?}");
    let f = coteja(
        "create dataset ventas.x (a int);
create dataset ventas.x (a int)",
    );
    assert!(
        f.len() == 1 && f[0].mensaje.contains("ya hay un dataset `ventas.x`"),
        "{f:?}"
    );

    // una base sobre un origen: el origen tiene que estar en el árbol
    assert!(
        coteja("create foreign database espejo from origin ventas include (public.*)").is_empty()
    );
    let f = coteja("create foreign database espejo from origin nadie include (public.*)");
    assert!(f[0].mensaje.contains("ningún origen `nadie`"), "{f:?}");
}

/// ADR 0040 paso 5: `create view` y `drop view` en el guion. La frase se lee
/// entera —el nombre, la lista de columnas con sus comentarios, `comment`,
/// `with schema evolution`— y la consulta se guarda tal como se escribió.
#[test]
fn una_vista_se_crea_y_se_quita_desde_sql() {
    use ore_core::sql_del_arbol::guion::Sentencia;
    let q = "create or replace view ventas.espana.porPais (pais comment 'el país', n)\n  comment 'pedidos por país'\n  with schema evolution\nas\nselect pais, count(*) as n\nfrom ventas.pedidos\ngroup by pais;";
    let t = guion(q).unwrap_or_else(|f| panic!("{f:?}"));
    assert_eq!(t[0].sentencia.que(), "create or replace view");
    match &t[0].sentencia {
        Sentencia::CrearVista {
            destino,
            consulta,
            lee,
            columnas,
            comentario,
            o_reemplaza,
            si_no_existe,
            evolucion,
        } => {
            assert_eq!(destino.referencia(), "ventas.espana.porPais");
            assert_eq!(
                consulta,
                "select pais, count(*) as n\nfrom ventas.pedidos\ngroup by pais"
            );
            assert_eq!(
                lee.iter().map(|n| n.referencia()).collect::<Vec<_>>(),
                ["ventas.pedidos"]
            );
            assert_eq!(
                columnas
                    .iter()
                    .map(|c| (c.nombre.as_str(), c.comentario.as_deref()))
                    .collect::<Vec<_>>(),
                [("pais", Some("el país")), ("n", None)]
            );
            assert_eq!(comentario.as_deref(), Some("pedidos por país"));
            assert!(*o_reemplaza && !*si_no_existe && *evolucion);
        }
        s => panic!("{s:?}"),
    }
    let t = guion("drop view if exists ventas.v").unwrap();
    assert_eq!(t[0].sentencia.que(), "drop view");

    // lo que no es una vista del árbol se dice, en su sitio
    let f = |q: &str| guion(q).expect_err(q);
    assert!(
        f("create or replace view if not exists ventas.v as select 1")[0]
            .mensaje
            .contains("no van juntas")
    );
    assert!(
        f("create temp view ventas.v as select 1")[0]
            .mensaje
            .contains("temporal")
    );
    assert!(
        f("create view ventas.v as insert into ventas.x select 1")[0]
            .mensaje
            .contains("OOS2038")
    );
    assert!(
        f("create view ventas.v (a, a) as select 1 as a, 2 as b")[0]
            .mensaje
            .contains("dos veces")
    );
    assert!(f("create view ventas.v")[0].mensaje.contains("as select"));
    assert!(
        f("create view v as select 1")[0]
            .mensaje
            .contains("de qué base")
    );
}

#[test]
fn una_vista_se_coteja_con_el_arbol() {
    let t = arbol("vista");
    let (pkg, diags) = ore_core::validate::cargar_paquete(&t.0);
    assert!(diags.is_empty(), "{diags:?}");
    let f = |q: &str| cotejar_guion(&pkg, &guion(q).unwrap_or_else(|f| panic!("{q}: {f:?}")));
    // lee una tabla (virtual), una vista y un dataset: se puede
    assert!(f("create view ventas.nueva as select p.id from ventas.pedidos_t p join ventas.pedidosEs e on e.id = p.id join ventas.pedidos d on d.id = p.id").is_empty());
    // ya hay una: sin `or replace` ni `if not exists` se dice
    let ya = f("create view ventas.pedidosEs as select id from ventas.pedidos");
    assert!(ya[0].mensaje.contains("ya hay una vista"), "{ya:?}");
    assert!(
        f("create or replace view ventas.pedidosEs as select id from ventas.pedidos").is_empty()
    );
    assert!(
        f("create view if not exists ventas.pedidosEs as select id from ventas.pedidos").is_empty()
    );
    // un nombre que ya es un dataset no se reemplaza nunca
    let d = f("create or replace view ventas.resumen as select 1 as n");
    assert!(d[0].mensaje.contains("OOS2035"), "{d:?}");
    // lo que no está, y el schema que no está
    assert!(
        f("create view ventas.v as select * from ventas.nada")[0]
            .mensaje
            .contains("ninguna tabla, vista ni dataset")
    );
    assert!(
        f("create view ventas.fantasma.v as select 1 as n")[0]
            .mensaje
            .contains("ningún schema")
    );
    // en orden: una vista del guion se lee en la siguiente sentencia, y se quita
    assert!(f("create view ventas.v as select id from ventas.pedidos;\nselect * from ventas.v;\ndrop view ventas.v").is_empty());
    assert!(
        f("drop view ventas.nada")[0]
            .mensaje
            .contains("ninguna vista")
    );
    assert!(f("drop view if exists ventas.nada").is_empty());
    assert!(
        f("drop view ventas.resumen")[0]
            .mensaje
            .contains("no una vista")
    );
}
