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
    // 0057 B4·1: una vista materializada también es del árbol (caía a DuckDB).
    assert_eq!(
        e("create or replace materialized view ventas.v as select 1 as a"),
        Some(E::Vista("ventas.v".to_string()))
    );
    assert_eq!(
        e("create materialized view ventas.v as select 1 as a"),
        Some(E::Vista("ventas.v".to_string()))
    );
    // ADR 0049 B4·4: una colección es del árbol (DuckDB no la conoce), resuelva o no
    assert_eq!(
        e("create media collection if not exists ventas.s.c media document formats (pdf)"),
        Some(E::Crea("media collection ventas.s.c".into()))
    );
    assert_eq!(
        e("create media collection nada.s.c media image formats (png)"),
        Some(E::Crea("media collection ".into()))
    );
    // 0049 B8: and `alter media collection`, which DuckDB does not know either
    assert_eq!(
        e("alter media collection ventas.s.c set managed"),
        Some(E::Crea("media collection ventas.s.c".into()))
    );
    // 0049 B8·3: and `describe <kind>` of an asset of the tree; of what is
    // not the tree's (`describe tmp`, a DuckDB table), DuckDB's
    assert_eq!(
        e("describe media collection ventas.s.c"),
        Some(E::Crea("media collection ventas.s.c".into()))
    );
    assert_eq!(
        e("describe view ventas.s.v"),
        Some(E::Crea("view ventas.s.v".into()))
    );
    assert_eq!(
        e("describe object table ventas.s.o"),
        Some(E::Crea("object table ventas.s.o".into()))
    );
    assert_eq!(e("describe table tmp"), None);
    assert_eq!(e("describe tmp"), None);
    assert_eq!(e("describe table nada.s.t"), None);
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
            materializada,
        } => {
            assert!(!*materializada);
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

/// ADR 0040 paso 7: `create materialized view` es la vista y su copia —el
/// dataset `<vista>_copia`—. La copia de una consulta se calcula en un puesto,
/// que lee el lago: una vista materializada no lee una tabla de un origen, ni
/// directamente ni por otra vista; y el nombre de su copia tiene que estar libre.
#[test]
fn una_vista_materializada_lee_el_lago_o_el_origen_y_su_copia_tiene_sitio() {
    use ore_core::sql_del_arbol::guion::Sentencia;
    let t = guion(
        "create or replace materialized view ventas.grandes as select id from ventas.pedidos",
    )
    .unwrap_or_else(|f| panic!("{f:?}"));
    assert_eq!(t[0].sentencia.que(), "create or replace materialized view");
    assert!(matches!(
        &t[0].sentencia,
        Sentencia::CrearVista {
            materializada: true,
            ..
        }
    ));
    let a = arbol("materializada");
    escribe(
        &a.0,
        "packages/ventas/datasets/ocupado.yaml",
        "apiVersion: oos.dev/v1alpha12
kind: Dataset
metadata: { name: nueva_copia, namespace: ventas }
spec:
  owner: team:ventas
  columns:
    id: { type: Integer }
  changes: { mode: append }
",
    );
    escribe(
        &a.0,
        "packages/ventas/views/deLaTabla.yaml",
        "apiVersion: oos.dev/v1alpha12
kind: View
metadata: { name: deLaTabla, namespace: ventas }
spec:
  owner: team:ventas
  from: { table: ventas.pedidos_t }
  fields: { id: id }
",
    );
    let (pkg, diags) = ore_core::validate::cargar_paquete(&a.0);
    assert!(diags.is_empty(), "{diags:?}");
    let f = |q: &str| cotejar_guion(&pkg, &guion(q).unwrap_or_else(|f| panic!("{q}: {f:?}")));
    // sobre un dataset, o una vista sobre un dataset (mantenido: la tabla la
    // copia él): se puede
    assert!(
        f("create materialized view ventas.grande as select id from ventas.pedidos").is_empty()
    );
    assert!(
        f("create materialized view ventas.grande as select id from ventas.pedidosEs").is_empty()
    );
    // 0053 F7·2: sobre una tabla de un origen, o una vista que acaba en una,
    // también: la copia la calcula el Job, que lee esas tablas con el reparto
    assert!(
        f("create materialized view ventas.grande as select id from ventas.pedidos_t").is_empty()
    );
    assert!(
        f("create materialized view ventas.grande as select id from ventas.deLaTabla").is_empty()
    );
    // 0053 F7·3: `create or replace dataset … as select` desde el origen es
    // una copia (la vista `<d>_consulta` y `d`); `insert` desde el origen, no
    assert!(
        f("create or replace dataset ventas.desde_el_origen as select id from ventas.pedidos_t")
            .is_empty()
    );
    let u = guion(
        "create or replace dataset ventas.desde_el_origen as select id from ventas.deLaTabla",
    )
    .unwrap();
    let t = &u[0];
    if let ore_core::sql_del_arbol::guion::Sentencia::Unidad(un) = &t.sentencia {
        assert_eq!(
            ore_core::sql_del_arbol::copia_desde_el_origen(&pkg, un).as_deref(),
            Some("ventas.desde_el_origen_consulta")
        );
    } else {
        panic!("no es una unidad");
    }
    let x = f("insert into ventas.pedidos select id from ventas.pedidos_t");
    assert!(
        x.iter()
            .any(|x| x.mensaje.contains("insert` desde un origen todavía no")),
        "{x:?}"
    );
    // su copia se llamaría `ventas.nueva_copia`, que ya es otra cosa
    let x = f("create materialized view ventas.nueva as select id from ventas.pedidos");
    assert!(
        x.iter().any(|x| x.mensaje.contains("`ventas.nueva_copia`")),
        "{x:?}"
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

/// 0049 B7·1 and B7·3: a `MediaCollection` is a relation in `FROM` —one row per item—,
/// a name the cell resolves; a dataset written from it is anchored to it and
/// computed item by item, with its limits (B7·3), each one said.
#[test]
fn a_collection_is_read_in_from_and_a_dataset_written_from_it_is_anchored() {
    let t = arbol("coleccion");
    escribe(
        &t.0,
        "packages/ventas/collections/contratos.yaml",
        "apiVersion: oos.dev/v1alpha19\nkind: MediaCollection\nmetadata: { name: contratos, namespace: ventas }\nspec:\n  owner: team:ventas\n  media: document\n  formats: [pdf]\n",
    );
    let r = &t.0;
    assert!(
        fallos(
            r,
            "select path, size from ventas.default.contratos where content_type = 'application/pdf'"
        )
        .is_empty()
    );
    assert!(
        fallos(
            r,
            "select c._item, p.id from ventas.contratos c join ventas.pedidos p on true"
        )
        .is_empty()
    );
    // B7·3: written from it, the dataset is anchored to it
    let q = "create or replace dataset ventas.paginas as select path from ventas.contratos";
    assert!(fallos(r, q).is_empty(), "{:?}", fallos(r, q));
    let (pkg, _) = ore_core::validate::cargar_paquete(r);
    assert_eq!(
        ore_core::sql_del_arbol::anchored_to(&pkg, &analizar(q).unwrap()).as_deref(),
        Some("ventas.contratos")
    );
    let otra = "create or replace dataset ventas.otra as select 1 as a from ventas.pedidos";
    assert_eq!(
        ore_core::sql_del_arbol::anchored_to(&pkg, &analizar(otra).unwrap()),
        None
    );
    // and its limits, one error each, saying what to do instead
    let uno = |q: &str, que: &str| {
        let f = fallos(r, q);
        assert!(f.len() == 1 && f[0].mensaje.contains(que), "{q}: {f:?}");
    };
    uno(
        "insert into ventas.resumen select path as pais, 1 as n from ventas.contratos",
        "written whole",
    );
    uno(
        "create or replace dataset ventas.p2 as select c.path, p.id from ventas.contratos c join ventas.pedidos p on true",
        "and nothing else",
    );
    uno(
        "create or replace dataset ventas.p3 as select content_type, count(*) as n from ventas.contratos group by 1",
        "group by",
    );
    uno(
        "create or replace dataset ventas.p4 as select path from ventas.contratos order by path limit 3",
        "order by",
    );
    uno(
        "create or replace dataset ventas.resumen as select path as pais from ventas.contratos",
        "is not anchored",
    );
    assert_eq!(
        ore_core::sql_del_arbol::nombres_a_resolver(
            "select path from ventas.default.contratos",
            &pkg
        ),
        ["ventas.contratos"]
    );
    // 0057 C2: a view reads a collection as its listing (v1alpha17 `04` §3);
    // one with a copy does not, and says how instead.
    let guiona = |q: &str| cotejar_guion(&pkg, &guion(q).unwrap_or_else(|f| panic!("{q}: {f:?}")));
    let v = "create or replace view ventas.sin_pdf as select p.id from ventas.pedidos p left join ventas.contratos c on c.path = p.id where c.path is null";
    assert!(guiona(v).is_empty(), "{:?}", guiona(v));
    let m = guiona("create materialized view ventas.listado as select path from ventas.contratos");
    assert!(
        m.len() == 1
            && m[0]
                .mensaje
                .contains("una vista con copia no lee su listado"),
        "{m:?}"
    );
}

/// 0049 B7·2: a tree `Function` is called from SQL by its name —as a value or
/// as rows—; the text is rewritten for DuckDB with the tokenizer (a name in a
/// comment is not a call), and what cannot be called says why.
#[test]
fn a_tree_function_is_called_from_sql() {
    let t = arbol("funciones");
    escribe(
        &t.0,
        "packages/ventas/collections/contratos.yaml",
        "apiVersion: oos.dev/v1alpha19\nkind: MediaCollection\nmetadata: { name: contratos, namespace: ventas }\nspec:\n  owner: team:ventas\n  media: document\n  formats: [pdf]\n",
    );
    escribe(
        &t.0,
        "packages/ventas/functions/paginas.yaml",
        "apiVersion: oos.dev/v1alpha20\nkind: Function\nmetadata: { name: paginas, namespace: ventas }\nspec:\n  runtime: python\n  entrypoint: functions/paginas.py:paginas\n  input:\n    item: { type: 'Media<ventas.default.contratos>', required: true }\n  output: { type: 'list<Struct<page: Integer, texto: String>>' }\n",
    );
    escribe(
        &t.0,
        "packages/ventas/functions/paginas.py",
        "import ore\nfrom dataclasses import dataclass\nfrom ore.tipos import Media\n\n\n@dataclass\nclass Pagina:\n    page: int\n    texto: str\n\n\n@ore.function\ndef paginas(item: Media[\"ventas.default.contratos\"]) -> list[Pagina]:\n    return []\n",
    );
    escribe(
        &t.0,
        "packages/ventas/functions/clasificar.yaml",
        "apiVersion: oos.dev/v1alpha10\nkind: Function\nmetadata: { name: clasificar, namespace: ventas }\nspec:\n  runtime: model\n  model: modelo/v2-lite\n  over: ventas.pedidosEs\n  prompt: clasifica\n  output:\n    clase: { type: String }\n",
    );
    let r = &t.0;
    let (pkg, _) = ore_core::validate::cargar_paquete(r);
    let u = |q: &str| cotejar(&pkg, &analizar(q).unwrap());

    // as rows, in a lateral join over the collection
    let q = "select c.path, p.page from ventas.contratos c cross join lateral ventas.paginas(c.item) as p";
    assert!(u(q).is_empty(), "{:?}", u(q));
    let (sql, calls) = ore_core::sql_del_arbol::sql_calls(q, &pkg);
    assert_eq!(
        sql,
        "select c.path, p.page from ventas.contratos c cross join lateral \
         (select unnest(__ore_fn_1(c.item), max_depth := 2)) as p"
    );
    assert_eq!(calls.len(), 1);
    assert!(
        calls[0].table && calls[0].arity == 1 && calls[0].name == "ventas.paginas",
        "{calls:?}"
    );
    // and the collection is still a name to resolve; the call is not
    assert_eq!(
        ore_core::sql_del_arbol::nombres_a_resolver(q, &pkg),
        ["ventas.contratos"]
    );

    // as a value, three parts, nested parentheses; a comment does not count
    let q = "-- ventas.paginas(x)\nselect ventas.default.paginas(c.item), lower(c.path) from ventas.contratos c";
    let (sql, calls) = ore_core::sql_del_arbol::sql_calls(q, &pkg);
    assert_eq!(
        sql,
        "-- ventas.paginas(x)\nselect __ore_fn_1(c.item), lower(c.path) from ventas.contratos c"
    );
    assert!(!calls[0].table && calls[0].arity == 1, "{calls:?}");
    // no calls: the text as it was
    let q = "select  path  from ventas.contratos";
    assert_eq!(ore_core::sql_del_arbol::sql_calls(q, &pkg).0, q);

    // what cannot be called
    let f = u("select ventas.nadie(c.item) from ventas.contratos c");
    assert!(
        f.len() == 1
            && f[0]
                .mensaje
                .contains("no published function `ventas.nadie`"),
        "{f:?}"
    );
    let f = u("select ventas.clasificar(c.item) from ventas.contratos c");
    assert!(
        f.len() == 1 && f[0].mensaje.contains("runtime: model"),
        "{f:?}"
    );
    // a DuckDB function without dots is still DuckDB's
    assert!(u("select upper(path) from ventas.contratos").is_empty());
}

/// 0049 B8: a collection from an `ObjectTable` —managed or `virtual`— is
/// created from SQL, and `alter … set managed|virtual` is for one with an
/// origin; each failure says why.
#[test]
fn a_collection_with_an_origin_from_sql() {
    let t = arbol("origen");
    escribe(
        &t.0,
        "packages/ventas/objecttables/docs_t.yaml",
        "apiVersion: oos.dev/v1alpha16\nkind: ObjectTable\nmetadata: { name: docs_t, namespace: ventas }\nspec:\n  datasource: pg\n  prefix: \"docs/\"\n  match: \"*.pdf\"\n  media: document\n  changes: { mode: retract, witness: listing }\n",
    );
    escribe(
        &t.0,
        "packages/ventas/collections/vdocs.yaml",
        "apiVersion: oos.dev/v1alpha19\nkind: MediaCollection\nmetadata: { name: vdocs, namespace: ventas }\nspec:\n  owner: team:ventas\n  media: document\n  formats: [pdf]\n  from: { objectTable: ventas.default.docs_t }\n  virtual: true\n",
    );
    escribe(
        &t.0,
        "packages/ventas/collections/escrita.yaml",
        "apiVersion: oos.dev/v1alpha19\nkind: MediaCollection\nmetadata: { name: escrita, namespace: ventas }\nspec:\n  owner: team:ventas\n  media: document\n  formats: [pdf]\n",
    );
    let (pkg, _) = ore_core::validate::cargar_paquete(&t.0);
    let coteja = |q: &str| cotejar_guion(&pkg, &guion(q).unwrap_or_else(|f| panic!("{q}: {f:?}")));
    // created from an object table, managed or virtual
    assert_eq!(
        coteja(
            "create media collection ventas.mdocs media document formats (pdf) from object table ventas.docs_t"
        ),
        Vec::<Fallo>::new()
    );
    assert_eq!(
        coteja(
            "create media collection if not exists ventas.vdocs media document formats (pdf) from object table ventas.docs_t virtual"
        ),
        Vec::<Fallo>::new()
    );
    // and converted
    assert_eq!(
        coteja("alter media collection ventas.vdocs set managed"),
        Vec::<Fallo>::new()
    );
    // what fails, and why
    let uno = |q: &str, que: &str| {
        let f = coteja(q);
        assert!(f.len() == 1 && f[0].mensaje.contains(que), "{q}: {f:?}");
    };
    uno(
        "create media collection ventas.x media document formats (pdf) from object table ventas.nada",
        "no `ObjectTable` `ventas.nada`",
    );
    uno(
        "create media collection ventas.x media document formats (pdf) from object table ventas.pedidos",
        "a collection comes from an `ObjectTable`",
    );
    uno(
        "create media collection ventas.escrita media document formats (pdf) from object table ventas.docs_t",
        "is a written collection",
    );
    uno(
        "alter media collection ventas.escrita set managed",
        "has no origin",
    );
    uno(
        "alter media collection ventas.pedidos set managed",
        "not a collection",
    );
    uno(
        "alter media collection ventas.nada set virtual",
        "no collection `ventas.nada` in this branch",
    );
}

/// 0049 B8·3 · `describe <kind>`: its columns, then `# Detail`, from the tree
/// and the pointer of the branch.
#[test]
fn describe_gives_columns_then_detail_of_each_kind() {
    let t = arbol("describe");
    escribe(
        &t.0,
        "packages/ventas/objecttables/docs_t.yaml",
        "apiVersion: oos.dev/v1alpha16\nkind: ObjectTable\nmetadata: { name: docs_t, namespace: ventas }\nspec:\n  datasource: pg\n  prefix: \"docs/\"\n  match: \"*.pdf\"\n  media: document\n  changes: { mode: retract, witness: listing }\n",
    );
    escribe(
        &t.0,
        "packages/ventas/collections/docs.yaml",
        "apiVersion: oos.dev/v1alpha19\nkind: MediaCollection\nmetadata: { name: docs, namespace: ventas }\nspec:\n  owner: team:ventas\n  media: document\n  formats: [pdf]\n  from: { objectTable: ventas.default.docs_t }\n",
    );
    let (pkg, diags) = ore_core::validate::cargar_paquete(&t.0);
    assert!(diags.is_empty(), "{diags:?}");
    let doc = |n: &str| {
        pkg.docs
            .iter()
            .find(|d| d.qname().as_deref() == Some(n))
            .unwrap_or_else(|| panic!("no `{n}`"))
    };
    let describe = |n: &str, p: Option<&str>, conducto: bool| {
        let p = p.map(|t| ore_core::parse::parse(t).unwrap());
        ore_core::assets::describir(&pkg, doc(n), p.as_ref(), conducto)
    };
    let detalle = |f: &[[String; 3]]| -> std::collections::BTreeMap<String, String> {
        let i = f.iter().position(|x| x[0] == "# Detail").expect("# Detail");
        f[i + 1..]
            .iter()
            .map(|x| (x[0].clone(), x[1].clone()))
            .collect()
    };
    let columnas = |f: &[[String; 3]]| -> Vec<String> {
        f.iter()
            .take_while(|x| !x[0].is_empty())
            .map(|x| format!("{} {}", x[0], x[1]))
            .collect()
    };
    // a table: its columns with their types, where it comes from and who copies it
    let f = describe("ventas.pedidos_t", None, true);
    assert_eq!(columnas(&f), ["id Integer", "pais String"], "{f:?}");
    let d = detalle(&f);
    assert_eq!(d["kind"], "table");
    assert_eq!(
        (d["datasource"].as_str(), d["object"].as_str()),
        ("pg", "public.pedidos")
    );
    assert_eq!(d["copied by"], "ventas.pedidos");
    // a dataset: what it reads and its pointer
    let f = describe(
        "ventas.pedidos",
        Some("{\"filas\": 7, \"transaccion\": 3, \"estado\": \"al-dia\"}"),
        true,
    );
    let d = detalle(&f);
    assert_eq!(
        (
            d["kind"].as_str(),
            d["from"].as_str(),
            d["rows"].as_str(),
            d["transaction"].as_str()
        ),
        ("dataset", "ventas.pedidos_t", "7", "3")
    );
    // a view: virtual, and what it reads
    let d = detalle(&describe("ventas.pedidosEs", None, true));
    assert_eq!(
        (d["kind"].as_str(), d["type"].as_str(), d["reads"].as_str()),
        ("view", "virtual", "ventas.pedidos")
    );
    // an object table: its listing columns, its prefix, and the collections from it
    let f = describe("ventas.docs_t", None, true);
    assert!(columnas(&f).iter().any(|c| c.starts_with("key ")), "{f:?}");
    let d = detalle(&f);
    assert_eq!(
        (
            d["kind"].as_str(),
            d["prefix"].as_str(),
            d["collections"].as_str()
        ),
        ("object table", "docs/", "ventas.docs")
    );
    // a managed collection: its status from the pointer
    let estado = |p: Option<&str>, conducto: bool| {
        let d = detalle(&describe("ventas.docs", p, conducto));
        (
            d["type"].clone(),
            d["status"].clone(),
            d.get("pending").cloned(),
        )
    };
    let n = |s: &str| s.to_string();
    assert_eq!(
        estado(
            Some("{\"virtual\": false, \"por_copiar\": 0, \"items\": {\"actuales\": 4}}"),
            true
        ),
        (n("managed"), n("copied"), Some(n("0")))
    );
    assert_eq!(
        estado(
            Some("{\"virtual\": false, \"por_copiar\": 1, \"items\": {\"actuales\": 4}}"),
            true
        )
        .1,
        "copying (1 of 4 pending)"
    );
    assert_eq!(
        estado(
            Some("{\"virtual\": true, \"items\": {\"actuales\": 4}}"),
            true
        ),
        (n("managed"), n("not copied yet"), Some(n("4")))
    );
    assert_eq!(
        estado(None, false).1,
        "copy waits for the owner's conduit (OOS4011)"
    );
    assert_eq!(
        columnas(&describe("ventas.docs", None, true))[0],
        "_item Media"
    );
}

/// 0056 V2·3: la función propia (v1alpha26) se llama `functions.<def>(…)`, y
/// el nombre de antes —`<paquete>.<def>`— sigue llamándola, por el nombre de
/// verdad: lo escrito antes de migrar no se rompe.
#[test]
fn a_function_of_its_own_is_called_as_functions_dot_name() {
    let t = arbol("funcion-propia");
    escribe(
        &t.0,
        "packages/ventas/collections/contratos.yaml",
        "apiVersion: oos.dev/v1alpha19\nkind: MediaCollection\nmetadata: { name: contratos, namespace: ventas }\nspec:\n  owner: team:ventas\n  media: document\n  formats: [pdf]\n",
    );
    escribe(
        &t.0,
        "functions/paginas.yaml",
        "apiVersion: oos.dev/v1alpha26\nkind: Function\nmetadata:\n  name: paginas\n  version: 0.1.0\nspec:\n  owner: team:ventas\n  runtime: python\n  entrypoint: packages/ventas/repo/funciones/paginas.py:paginas\n  codeDigest: sha256:0000000000000000000000000000000000000000000000000000000000000000\n  input:\n    item: { type: 'Media<ventas.default.contratos>', required: true }\n  output: { type: 'list<Struct<page: Integer, texto: String>>' }\n",
    );
    let r = &t.0;
    let (pkg, _) = ore_core::validate::cargar_paquete(r);
    let u = |q: &str| cotejar(&pkg, &analizar(q).unwrap());

    let q = "select c.path, p.page from ventas.contratos c cross join lateral functions.paginas(c.item) as p";
    assert!(u(q).is_empty(), "{:?}", u(q));
    let (sql, calls) = ore_core::sql_del_arbol::sql_calls(q, &pkg);
    assert!(sql.contains("__ore_fn_1(c.item)"), "{sql}");
    assert_eq!(calls[0].name, "functions.paginas");

    // El nombre de antes: la misma función, por su nombre de verdad.
    let viejo = "select ventas.paginas(c.item) from ventas.contratos c";
    assert!(u(viejo).is_empty(), "{:?}", u(viejo));
    let (_, calls) = ore_core::sql_del_arbol::sql_calls(viejo, &pkg);
    assert_eq!(calls[0].name, "functions.paginas");

    // Otro paquete no la alcanza por el nombre de antes.
    let otro = "select compras.paginas(c.item) from ventas.contratos c";
    assert!(!u(otro).is_empty());
}

/// 0049 B10·1: **ficheros que dan ficheros, en SQL** —`create or replace media
/// collection … as select …`—. La consulta da una fila por fichero (`name`,
/// `data`, y si acaso `content_type` y `anchor`), lee una colección y nada más,
/// todo por ítem; la colección destino es escrita y del mismo medio, o nueva.
/// Cada fallo, uno, y dice qué hacer.
#[test]
fn a_media_collection_is_derived_from_a_query() {
    use ore_core::sql_del_arbol::guion::Sentencia;
    let t = arbol("derivada");
    let col = |nombre: &str, media: &str, formato: &str, resto: &str| {
        escribe(
            &t.0,
            &format!("packages/ventas/collections/{nombre}.yaml"),
            &format!(
                "apiVersion: oos.dev/v1alpha19\nkind: MediaCollection\nmetadata: {{ name: {nombre}, namespace: ventas }}\nspec:\n  owner: team:ventas\n  media: {media}\n  formats: [{formato}]\n{resto}"
            ),
        )
    };
    col("contratos", "document", "pdf", "");
    col("paginas_png", "image", "png", "");
    col("otra_img", "image", "jpg", "");
    escribe(
        &t.0,
        "packages/ventas/objecttables/docs_t.yaml",
        "apiVersion: oos.dev/v1alpha16\nkind: ObjectTable\nmetadata: { name: docs_t, namespace: ventas }\nspec:\n  datasource: pg\n  prefix: \"docs/\"\n  match: \"*.png\"\n  media: image\n  changes: { mode: retract, witness: listing }\n",
    );
    col(
        "mantenida",
        "image",
        "png",
        "  from: { objectTable: ventas.default.docs_t }\n",
    );
    escribe(
        &t.0,
        "functions/pdf_a_png.yaml",
        "apiVersion: oos.dev/v1alpha26\nkind: Function\nmetadata:\n  name: pdf_a_png\n  version: 0.1.0\nspec:\n  owner: team:ventas\n  runtime: python\n  entrypoint: packages/ventas/repo/funciones/pdf_a_png.py:pdf_a_png\n  codeDigest: sha256:0000000000000000000000000000000000000000000000000000000000000000\n  input:\n    item: { type: 'Media<ventas.default.contratos>', required: true }\n  output: { type: 'list<Struct<name: String, data: Opaque, anchor: Struct<kind: String, page: Integer>>>' }\n",
    );
    let (pkg, diags) = ore_core::validate::cargar_paquete(&t.0);
    assert!(diags.is_empty(), "{diags:?}");
    let trozo = |q: &str| {
        let mut v = guion(q).unwrap_or_else(|f| panic!("{q}: {f:?}"));
        assert_eq!(v.len(), 1);
        v.remove(0)
    };
    let coteja = |q: &str| cotejar_guion(&pkg, &[trozo(q)]);

    // una página por fichero, de una Function: lo que lee, lo que llama, lo que da
    let q = "create or replace media collection ventas.paginas media image formats (png) as
select p.name, p.data, p.anchor
from ventas.contratos as c
cross join lateral functions.pdf_a_png(c._item) as p
where c.content_type = 'application/pdf'";
    let t1 = trozo(q);
    assert_eq!(t1.sentencia.que(), "create or replace media collection");
    let Sentencia::ColeccionDerivada {
        destino,
        media,
        formatos,
        consulta,
        columnas,
        ..
    } = &t1.sentencia
    else {
        panic!("{:?}", t1.sentencia)
    };
    assert_eq!(destino.referencia(), "ventas.paginas");
    assert_eq!(
        (media.as_str(), formatos.as_slice()),
        ("image", &["png".to_string()][..])
    );
    assert_eq!(columnas, &["name", "data", "anchor"]);
    assert_eq!(
        consulta
            .lee
            .iter()
            .map(|n| n.referencia())
            .collect::<Vec<_>>(),
        ["ventas.contratos"]
    );
    assert_eq!(consulta.calls.len(), 1);
    assert!(
        consulta.consulta.starts_with("select p.name"),
        "{}",
        consulta.consulta
    );
    assert!(coteja(q).is_empty(), "{:?}", coteja(q));
    // la celda es del árbol (DuckDB no la conoce)
    assert_eq!(
        ore_core::sql_del_arbol::escribe_en_el_arbol(q, &pkg),
        Some(ore_core::sql_del_arbol::EscribeEnElArbol::Crea(
            "media collection ventas.paginas".into()
        ))
    );
    // en una que ya existe, escrita y del mismo medio y formatos
    let ya = "create or replace media collection ventas.paginas_png media image formats (png) as select p.name, p.data from ventas.contratos c cross join lateral functions.pdf_a_png(c._item) p";
    assert!(coteja(ya).is_empty(), "{:?}", coteja(ya));
    // copiar sin función: `data` es el ítem, y `content_type` se puede decir
    let copia = "create or replace media collection ventas.solo_pdf media document formats (pdf) as select c.path as name, c._item as data, c.content_type from ventas.contratos as c where c.size < 1000000";
    assert!(coteja(copia).is_empty(), "{:?}", coteja(copia));

    // lo que no analiza: su fallo
    let mal = |q: &str, que: &str| {
        let f = guion(q).expect_err(q);
        assert!(f.iter().any(|x| x.mensaje.contains(que)), "{q}: {f:?}");
    };
    let sel = "select p.name, p.data from ventas.contratos c cross join lateral functions.pdf_a_png(c._item) p";
    let x = "create or replace media collection ventas.x media image formats (png)";
    mal(
        &format!("create media collection ventas.x media image formats (png) as {sel}"),
        "la segunda vez",
    );
    mal(x, "vaciaría");
    mal(
        &format!(
            "create media collection if not exists ventas.x media image formats (png) as {sel}"
        ),
        "`if not exists` no va",
    );
    mal(
        &format!("{x} from object table ventas.docs_t as {sel}"),
        "no de las dos",
    );
    mal(&format!("{x} as select * from ventas.contratos"), "`*`");
    mal(
        &format!(
            "{x} as select p.name, p.data, p.page from ventas.contratos c cross join lateral functions.pdf_a_png(c._item) p"
        ),
        "`page` no es una columna de un fichero",
    );
    mal(
        &format!("{x} as select lower(c.path), c._item as data from ventas.contratos c"),
        "no tiene nombre",
    );
    mal(
        &format!("{x} as select c.path as name from ventas.contratos c"),
        "falta la columna `data`",
    );
    mal(
        &format!(
            "{x} as select c.path as name, c.path as name, c._item as data from ventas.contratos c"
        ),
        "dos veces",
    );
    mal(
        &format!("{x} as insert into ventas.resumen select 1, 2"),
        "es un `select`",
    );

    // lo que no coteja contra el árbol: un fallo, el suyo
    let uno = |q: &str, que: &str| {
        let f = coteja(q);
        assert!(f.len() == 1 && f[0].mensaje.contains(que), "{q}: {f:?}");
    };
    uno(
        &format!("{x} as select p.pais as name, p.pais as data from ventas.pedidos p"),
        "no lee ninguna",
    );
    uno(
        &format!(
            "{x} as select c.path as name, c._item as data from ventas.contratos c join ventas.pedidos p on true"
        ),
        "y nada más",
    );
    uno(
        &format!(
            "{x} as select c.path as name, c._item as data from ventas.contratos c order by c.path limit 3"
        ),
        "no se calcula ítem a ítem",
    );
    uno(
        &format!(
            "create or replace media collection ventas.otra_img media image formats (png) as {sel}"
        ),
        "ya existe con `media image formats (jpg)`",
    );
    uno(
        &format!(
            "create or replace media collection ventas.mantenida media image formats (png) as {sel}"
        ),
        "colección mantenida",
    );
    uno(
        &format!(
            "create or replace media collection ventas.resumen media image formats (png) as {sel}"
        ),
        "ya es un `Dataset`",
    );
    uno(
        "create or replace media collection ventas.contratos media document formats (pdf) as select c.path as name, c._item as data from ventas.contratos c",
        "se lee a sí misma",
    );
    uno(
        &format!(
            "{x} as select p.name, p.data from ventas.contratos c cross join lateral functions.nadie(c._item) p"
        ),
        "no published function",
    );
}
