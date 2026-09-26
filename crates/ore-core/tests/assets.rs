//! El índice de assets (0034 ⑤) sobre un árbol de fuego con un ejemplar de cada
//! kind y de cada relación, una carpeta del cliente, una vista inducida y un
//! enlace roto. Los oráculos de los árboles reales (demo, victor) los da
//! `pruebas-de-fuego/medida-assets-indice.py` y los cuadra `ore assets`.
use ore_core::assets::{Cabeza, indice};
use ore_core::json::Json;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

fn escribe(raiz: &Path, rel: &str, texto: &str) {
    let p = raiz.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, texto).unwrap();
}

struct Arbol(std::path::PathBuf);
impl Arbol {
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for Arbol {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn arbol() -> Arbol {
    arbol_en("uno")
}

fn arbol_en(caso: &str) -> Arbol {
    let t = Arbol(std::env::temp_dir().join(format!("ore-assets-{}-{caso}", std::process::id())));
    let _ = fs::remove_dir_all(&t.0);
    let r = t.path();
    escribe(
        r,
        "ontology.config.yaml",
        "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\nmetadata: { name: fuego, version: 0.1.0 }\ndatasources:\n  - { name: pg, type: postgres, connectionEnv: PG_URL, labels: { gdpr.sensitivity: low } }\n",
    );
    escribe(
        r,
        "lattice.yaml",
        "apiVersion: oos.dev/v1alpha3
kind: Lattice
metadata: { name: sensitivity, namespace: gdpr }
spec:
  levels: [none, low, high]
",
    );
    escribe(
        r,
        "conduits.yaml",
        "apiVersion: oos.dev/v1alpha1\nkind: ConduitPolicy\nmetadata: { name: fuego }\nspec:\n  owner: team:fuego\n  conduits:\n    materialization.payload: { gdpr.sensitivity: high, oos.maturity: DRAFT }\n",
    );
    escribe(
        r,
        "packages/ventas/package.yaml",
        "apiVersion: oos.dev/v1alpha1\nkind: Package\nmetadata: { name: ventas, version: 0.1.0, status: active, domain: ventas }\nspec: { owner: team:ventas }\n",
    );
    escribe(
        r,
        "packages/ventas/discover.scope.json",
        "{\"source\": \"pg\", \"type\": \"standard\", \"only\": [\"public.pedidos\"]}\n",
    );
    escribe(
        r,
        "packages/ventas/tables/pedidos_t.yaml",
        "apiVersion: oos.dev/v1alpha8\nkind: Table\nmetadata: { name: pedidos_t, namespace: ventas }\nspec:\n  datasource: pg\n  object: \"public.pedidos\"\n  columns:\n    id: { type: Integer }\n    pais: { type: String }\n    email: { type: String, labels: { gdpr.sensitivity: high } }\n  reads: { fullScan: cheap }\n  changes: { mode: append, witness: snapshot }\n",
    );
    escribe(
        r,
        "packages/ventas/tables/clientes_t.yaml",
        "apiVersion: oos.dev/v1alpha8\nkind: Table\nmetadata: { name: clientes_t, namespace: ventas }\nspec:\n  datasource: pg\n  object: \"public.clientes\"\n  columns:\n    id: { type: Integer }\n    nombre: { type: String }\n  reads: { fullScan: cheap }\n  changes: { mode: append, witness: snapshot }\n",
    );
    // La vista inducida sobre clientes_t (`__` en el fichero, identidad): detalle de la tabla.
    escribe(
        r,
        "packages/ventas/views/Clientes__public_clientes.yaml",
        "apiVersion: oos.dev/v1alpha12\nkind: View\nmetadata: { name: clientes, namespace: ventas }\nspec:\n  owner: team:ventas\n  from: { table: ventas.clientes_t }\n  fields: { id: id, nombre: nombre }\n",
    );
    // El dataset identidad sobre pedidos_t.
    escribe(
        r,
        "packages/ventas/datasets/pedidos.yaml",
        "apiVersion: oos.dev/v1alpha12\nkind: Dataset\nmetadata: { name: pedidos, namespace: ventas }\nspec:\n  owner: team:ventas\n  from: { table: ventas.pedidos_t }\n  freshness: 1h\n",
    );
    // Una vista propia con filtro, en una CARPETA del cliente.
    escribe(
        r,
        "packages/ventas/espana/views/pedidosEs.yaml",
        "apiVersion: oos.dev/v1alpha12\nkind: View\nmetadata: { name: pedidosEs, namespace: ventas, description: los de España }\nspec:\n  owner: team:ventas\n  from: { dataset: ventas.pedidos }\n  where: { pais: ES }\n  fields: { id: id, pais: pais }\n",
    );
    // Un dataset escrito, con puntero y procedencia.
    escribe(
        r,
        "packages/ventas/datasets/resumen.yaml",
        "apiVersion: oos.dev/v1alpha12\nkind: Dataset\nmetadata: { name: resumen, namespace: ventas }\nspec:\n  owner: team:ventas\n  columns:\n    pais: { type: String }\n    n: { type: Integer }\n  changes: { mode: append }\n",
    );
    escribe(
        r,
        "datasets/ventas_resumen.json",
        "{\"estado\": \"copiada\", \"filas\": 3, \"snapshot\": \"7\", \"metadata_location\": \"gs://b/datasets/ventas_resumen/metadata/1.json\", \"dataset\": \"datasets/ventas_resumen\", \"procedencia\": { \"puesto\": \"p1\", \"leidas\": [\"ventas.pedidosEs\"] }}\n",
    );
    escribe(
        r,
        "datasets/ventas/default/pedidos.json",
        "{\"estado\": \"error\", \"motivo\": \"el origen no contesta\", \"vista\": \"ventas.pedidos\"}\n",
    );
    // La entidad: respaldada en el dataset, satisface una interfaz, una propiedad nombra un concepto.
    escribe(
        r,
        "packages/ventas/entities/Pedido.yaml",
        "apiVersion: oos.dev/v1alpha4\nkind: Entity\nmetadata: { name: Pedido, namespace: ventas }\nspec:\n  nature: event\n  primaryKey: [id]\n  backedBy: ventas.pedidos\n  implements: [ventas.Hecho]\n  properties:\n    id: { type: Integer }\n    pais: { type: String }\n    email: { is: ventas.correo }\n",
    );
    escribe(
        r,
        "packages/ventas/interfaces/Hecho.yaml",
        "apiVersion: oos.dev/v1alpha4\nkind: Interface\nmetadata: { name: Hecho, namespace: ventas }\nspec:\n  requires: [ventas.id]\n",
    );
    escribe(
        r,
        "packages/ventas/concepts/correo.yaml",
        "apiVersion: oos.dev/v1alpha4\nkind: Concept\nmetadata: { name: correo, namespace: ventas, labels: { gdpr.sensitivity: high } }\nspec:\n  type: String\n",
    );
    // Una función que lee la vista, escribe la entidad y usa un modelo; y un enlace roto.
    escribe(
        r,
        "packages/ventas/functions/clasificar.yaml",
        "apiVersion: oos.dev/v1alpha10\nkind: Function\nmetadata: { name: clasificar, namespace: ventas }\nspec:\n  runtime: model\n  model: modelo/v2-lite\n  over: ventas.pedidosEs\n  reads: [ventas.nadie]\n  prompt: clasifica\n  output:\n    clase: { type: String }\n  effects:\n    - writes: ventas.Pedido.clase\n",
    );
    escribe(
        r,
        "packages/ventas/actions/segmentar.yaml",
        "apiVersion: oos.dev/v1alpha10\nkind: Action\nmetadata: { name: segmentar, namespace: ventas }\nspec:\n  over: ventas.pedidosEs\n  input:\n    segmento: { type: String, required: true }\n  sets:\n    - writes: ventas.Pedido.segmento\n      from: input.segmento\n",
    );
    escribe(
        r,
        "packages/ventas/models/prevision.yaml",
        "apiVersion: oos.dev/v1alpha11\nkind: TrainedModel\nmetadata: { name: prevision, namespace: ventas }\nspec:\n  owner: team:ventas\n  framework: sklearn\n  task: forecast\n  version: 1\n  artifacts: models/ventas_prevision/v1\n  digest: sha256:abababababababababababababababababababababababababababababababab\n  trainedFrom: [ventas.pedidosEs]\n",
    );
    escribe(
        r,
        "modelos/v2-lite.yaml",
        "apiVersion: oos.dev/v1alpha9\nkind: Model\nmetadata: { name: v2-lite }\nspec:\n  profile: g1/deepseek-v2-lite\n  tier: shared\n  task: chat\n",
    );
    t
}

/// Los punteros del árbol, como los lee quien llama: uno en su sitio
/// (`datasets/ventas/default/pedidos.json`) y otro de antes (`ventas_resumen`).
fn punteros(raiz: &Path) -> BTreeMap<String, Json> {
    ore_core::punteros::del_arbol(raiz)
}

fn item<'a>(j: &'a Json, r: &str) -> &'a BTreeMap<String, Json> {
    let Json::Obj(m) = j else { panic!() };
    let Json::Obj(items) = &m["items"] else {
        panic!()
    };
    let Json::Obj(it) = items
        .get(r)
        .unwrap_or_else(|| panic!("no está `{r}`; hay: {:?}", items.keys().collect::<Vec<_>>()))
    else {
        panic!()
    };
    it
}

fn relaciones(it: &BTreeMap<String, Json>) -> Vec<(String, String, bool)> {
    let Json::Arr(r) = &it["relaciones"] else {
        panic!()
    };
    r.iter()
        .map(|x| {
            let Json::Obj(o) = x else { panic!() };
            let s = |k: &str| match o.get(k) {
                Some(Json::Str(v)) => v.clone(),
                _ => String::new(),
            };
            (
                s("tipo"),
                s("ref"),
                o.get("rota") == Some(&Json::Bool(true)),
            )
        })
        .collect()
}

fn tiene(it: &BTreeMap<String, Json>, tipo: &str, r: &str) -> bool {
    relaciones(it).iter().any(|(t, x, _)| t == tipo && x == r)
}

#[test]
fn el_indice_proyecta_cada_kind_con_su_carpeta_su_define_y_sus_relaciones() {
    let t = arbol();
    let (pkg, _) = ore_core::validate::cargar_paquete(t.path());
    let j = indice(
        &pkg,
        &punteros(t.path()),
        &Cabeza {
            cabeza: Some("abc".into()),
            rama: Some("main".into()),
            generado: None,
        },
    );
    let Json::Obj(m) = &j else { panic!() };
    assert_eq!(m["cabeza"], Json::s("abc"));
    let Json::Obj(items) = &m["items"] else {
        panic!()
    };

    // Los ítems: cada kind. La vista que el inductor deja sobre una tabla es
    // uno más (0040 paso 6: una vista de pleno derecho, sin trato aparte).
    let refs: Vec<&String> = items.keys().collect();
    for r in [
        "view:ventas.clientes",
        "table:ventas.pedidos_t",
        "table:ventas.clientes_t",
        "dataset:ventas.pedidos",
        "dataset:ventas.resumen",
        "view:ventas.pedidosEs",
        "entity:ventas.Pedido",
        "interface:ventas.Hecho",
        "concept:ventas.correo",
        "function:ventas.clasificar",
        "action:ventas.segmentar",
        "trainedmodel:ventas.prevision",
        "model:v2-lite",
    ] {
        assert!(items.contains_key(r), "falta `{r}`: {refs:?}");
    }
    assert_eq!(items.len(), 13);

    // La carpeta del cliente, y la del kind que no cuenta.
    assert_eq!(
        item(&j, "view:ventas.pedidosEs")["carpeta"],
        Json::s("espana")
    );
    assert_eq!(item(&j, "dataset:ventas.pedidos")["carpeta"], Json::s(""));
    assert_eq!(
        item(&j, "view:ventas.pedidosEs")["description"],
        Json::s("los de España")
    );
    assert_eq!(
        item(&j, "model:v2-lite")["paquete"],
        Json::Crudo("null".into())
    );

    // define: el dataset identidad; la vista con filtro no.
    let Json::Obj(def) = &item(&j, "dataset:ventas.pedidos")["define"] else {
        panic!()
    };
    assert_eq!(def["identidad"], Json::Bool(true));
    assert_eq!(def["from"], Json::s("table:ventas.pedidos_t"));
    assert_eq!(def["freshness"], Json::s("1h"));
    let Json::Obj(def) = &item(&j, "view:ventas.pedidosEs")["define"] else {
        panic!()
    };
    assert_eq!(def["identidad"], Json::Bool(false));
    assert_eq!(def["where"], Json::Bool(true));
    let Json::Obj(def) = &item(&j, "dataset:ventas.resumen")["define"] else {
        panic!()
    };
    assert_eq!(def["columns"], Json::Int(2));

    // expone: con el tipo de la raíz.
    let Json::Arr(ex) = &item(&j, "dataset:ventas.pedidos")["expone"] else {
        panic!()
    };
    assert_eq!(ex.len(), 3);
    assert!(ex.iter().any(|c| matches!(c, Json::Obj(o) if o.get("name") == Some(&Json::s("email")) && o.get("type") == Some(&Json::s("String")))));

    // detalle: la tabla con lo del origen, y nada de la vista que la lee.
    let Json::Obj(det) = &item(&j, "table:ventas.clientes_t")["detalle"] else {
        panic!()
    };
    assert_eq!(det["object"], Json::s("public.clientes"));
    assert!(!det.contains_key("vistaInducida"));

    // puntero: el escrito con su procedencia; el mantenido en error.
    let Json::Obj(p) = &item(&j, "dataset:ventas.resumen")["puntero"] else {
        panic!()
    };
    assert_eq!(p["estado"], Json::s("copiada"));
    assert_eq!(p["filas"], Json::Int(3));
    assert_eq!(p["ubicacion"], Json::s("datasets/ventas_resumen"));
    let Json::Obj(p) = &item(&j, "dataset:ventas.pedidos")["puntero"] else {
        panic!()
    };
    assert_eq!(p["estado"], Json::s("error"));
    assert_eq!(p["motivo"], Json::s("el origen no contesta"));

    // relaciones, en las dos direcciones.
    let ds = item(&j, "dataset:ventas.pedidos");
    assert!(tiene(ds, "sale_de", "table:ventas.pedidos_t"));
    assert!(tiene(ds, "produce", "view:ventas.pedidosEs"));
    assert!(tiene(ds, "respalda", "entity:ventas.Pedido"));
    assert!(tiene(
        item(&j, "table:ventas.pedidos_t"),
        "produce",
        "dataset:ventas.pedidos"
    ));
    let v = item(&j, "view:ventas.pedidosEs");
    assert!(tiene(v, "sale_de", "dataset:ventas.pedidos"));
    assert!(tiene(v, "leido_por", "function:ventas.clasificar"));
    assert!(tiene(v, "leido_por", "action:ventas.segmentar"));
    assert!(tiene(v, "produce", "trainedmodel:ventas.prevision"));
    assert!(
        tiene(v, "produce", "dataset:ventas.resumen"),
        "la procedencia del escrito: {:?}",
        relaciones(v)
    );
    let e = item(&j, "entity:ventas.Pedido");
    assert!(tiene(e, "respaldada_por", "dataset:ventas.pedidos"));
    assert!(tiene(e, "satisface", "interface:ventas.Hecho"));
    assert!(tiene(e, "nombra", "concept:ventas.correo"));
    assert!(tiene(e, "escrito_por", "function:ventas.clasificar"));
    assert!(tiene(e, "escrito_por", "action:ventas.segmentar"));
    assert!(tiene(
        item(&j, "interface:ventas.Hecho"),
        "satisfecha_por",
        "entity:ventas.Pedido"
    ));
    assert!(tiene(
        item(&j, "concept:ventas.correo"),
        "nombrado_por",
        "entity:ventas.Pedido"
    ));
    let f = item(&j, "function:ventas.clasificar");
    assert!(tiene(f, "lee", "view:ventas.pedidosEs"));
    assert!(tiene(f, "escribe", "entity:ventas.Pedido"));
    assert!(tiene(f, "usa", "model:v2-lite"));
    assert!(tiene(
        item(&j, "model:v2-lite"),
        "usado_por",
        "function:ventas.clasificar"
    ));
    // el enlace roto se enseña, y no tiene inversa
    assert!(
        relaciones(f)
            .iter()
            .any(|(t, r, rota)| t == "lee" && r == "view:ventas.nadie" && *rota)
    );
    assert!(!items.contains_key("view:ventas.nadie"));

    // acceso: la clasificación sube por la columna que usa; el conducto compila.
    let Json::Obj(ac) = &item(&j, "dataset:ventas.pedidos")["acceso"] else {
        panic!()
    };
    let Json::Obj(cl) = &ac["clasificacion"] else {
        panic!()
    };
    assert_eq!(cl["gdpr.sensitivity"], Json::s("high"), "{cl:?}");
    let Json::Obj(co) = &ac["conductos"] else {
        panic!()
    };
    assert_eq!(co["materialization.payload"], Json::s("compila"));
    let Json::Obj(ac) = &item(&j, "concept:ventas.correo")["acceso"] else {
        panic!()
    };
    let Json::Obj(cl) = &ac["clasificacion"] else {
        panic!()
    };
    assert_eq!(cl["gdpr.sensitivity"], Json::s("high"));

    // paquetes
    let Json::Arr(ps) = &m["paquetes"] else {
        panic!()
    };
    let Json::Obj(p) = &ps[0] else { panic!() };
    assert_eq!(p["name"], Json::s("ventas"));
    assert_eq!(p["type"], Json::s("standard"));
    assert_eq!(p["scoped"], Json::Bool(true));
    assert_eq!(p["source"], Json::s("pg"));
    // Once más la vista que el inductor deja sobre su tabla: un ítem más (0040 paso 6).
    assert_eq!(p["items"], Json::Int(12));
    assert_eq!(
        p["carpetas"],
        Json::Arr(vec![Json::s(""), Json::s("espana")])
    );
}

/// 0035 ① · el proyecto en el índice: una lente sobre el mismo árbol.
///
/// Cuatro manifiestos sobre el árbol de fuego —uno que nombra el paquete
/// entero, otro la carpeta del cliente, otro que **se solapa** con ése, uno
/// vacío y uno roto— y lo que el índice tiene que decir de ellos: cuántos
/// ítems toca cada uno, que un ítem lleva `proyectos` **en plural**, que lo
/// roto se lista igual sin alcanzar nada, y que el resto del índice **no
/// cambia** (el proyecto organiza; no compila ni gobierna).
#[test]
fn el_indice_reparte_los_items_por_proyecto_y_se_solapan() {
    let t = arbol_en("proyectos");
    let r = t.path();
    let sin = {
        let (pkg, _) = ore_core::validate::cargar_paquete(r);
        indice(&pkg, &punteros(r), &Cabeza::default())
    };
    for (n, texto) in [
        (
            "todo-ventas",
            "---\nnombre: Todo Ventas\ndescripcion: El paquete entero.\ncontiene: [ventas]\n---\nProsa.\n",
        ),
        (
            "espana",
            "---\nnombre: España\ncontiene: [ventas/espana]\n---\n",
        ),
        (
            "espana-bis",
            "---\nnombre: España, otra vez\ncontiene: [ventas/espana]\n---\n",
        ),
        (
            "nuevo",
            "---\nnombre: Nuevo\n---\nUn propósito sin nada todavía.\n",
        ),
        ("roto", "---\ndescripcion: sin nombre\n---\n"),
    ] {
        escribe(r, &format!("proyectos/{n}/README.md"), texto);
    }
    let (pkg, _) = ore_core::validate::cargar_paquete(r);
    let j = indice(&pkg, &punteros(r), &Cabeza::default());
    let Json::Obj(m) = &j else { panic!() };

    // El árbol no se entera: los mismos ítems, las mismas relaciones.
    let Json::Obj(a) = &sin else { panic!() };
    let Json::Obj(antes) = &a["items"] else {
        panic!()
    };
    let Json::Obj(ahora) = &m["items"] else {
        panic!()
    };
    assert_eq!(antes.len(), ahora.len(), "el manifiesto no es un ítem");

    // Cada proyecto, con lo que nombra y cuántos ítems le tocan.
    let Json::Arr(ps) = &m["proyectos"] else {
        panic!()
    };
    let dicho: Vec<(String, String, String)> = ps
        .iter()
        .map(|p| {
            let Json::Obj(p) = p else { panic!() };
            let s = |k: &str| match p.get(k) {
                Some(Json::Str(v)) => v.clone(),
                Some(Json::Int(v)) => v.to_string(),
                _ => "-".into(),
            };
            (s("nombre"), s("items"), s("roto"))
        })
        .collect();
    assert_eq!(
        dicho,
        vec![
            ("espana".into(), "1".into(), "-".into()),
            ("espana-bis".into(), "1".into(), "-".into()),
            ("nuevo".into(), "0".into(), "-".into()),
            ("roto".into(), "0".into(), "sin `nombre`".into()),
            ("todo-ventas".into(), "12".into(), "-".into()),
        ],
        "los proyectos, por nombre de carpeta"
    );

    // Un ítem está en VARIOS: `proyectos` es plural, y el solape se dice.
    assert_eq!(
        item(&j, "view:ventas.pedidosEs")["proyectos"],
        Json::Arr(vec![
            Json::s("espana"),
            Json::s("espana-bis"),
            Json::s("todo-ventas"),
        ])
    );
    // Lo que sólo alcanza el paquete entero.
    assert_eq!(
        item(&j, "dataset:ventas.pedidos")["proyectos"],
        Json::Arr(vec![Json::s("todo-ventas")])
    );
    // Y lo que queda FUERA de todos: el modelo de la raíz, que no es de nadie.
    assert_eq!(
        item(&j, "model:v2-lite")["proyectos"],
        Json::Arr(Vec::new()),
        "un ítem sin paquete no cae en ningún proyecto"
    );
}

/// 0035 ⑥ · el repositorio en el índice: dónde se trabaja.
///
/// Cuatro READMEs sobre el árbol de fuego —uno que es repositorio, otro
/// anidado dentro de él, uno roto y uno que NO lo es porque no dice
/// `plantilla`— y lo que el índice tiene que decir: la lista con su clase y su
/// versión, cada ítem con **su** repositorio (singular, el más hondo), lo roto
/// listado sin quedarse nada, y **los ítems sin cambiar**.
#[test]
fn el_indice_dice_en_que_repositorio_vive_cada_item() {
    let t = arbol_en("repositorios");
    let r = t.path();
    let sin = {
        let (pkg, _) = ore_core::validate::cargar_paquete(r);
        indice(&pkg, &punteros(r), &Cabeza::default())
    };
    for (ruta, texto) in [
        (
            "packages/ventas/espana/README.md",
            "---\nnombre: New Pipelines Java Transform\nplantilla: transforms\nplantillaVersion: 2\n---\nProsa.\n",
        ),
        (
            "packages/ventas/espana/modelo/README.md",
            "---\nnombre: Churn Model\nplantilla: models\n---\n",
        ),
        (
            "packages/ventas/roto/README.md",
            "---\nplantilla: functions\n---\n",
        ),
        ("packages/ventas/notas/README.md", "# Sólo una carpeta\n"),
    ] {
        escribe(r, ruta, texto);
    }
    let (pkg, _) = ore_core::validate::cargar_paquete(r);
    let j = indice(&pkg, &punteros(r), &Cabeza::default());
    let Json::Obj(m) = &j else { panic!() };

    // El árbol no se entera: los mismos ítems que antes.
    let Json::Obj(a) = &sin else { panic!() };
    let Json::Obj(antes) = &a["items"] else {
        panic!()
    };
    let Json::Obj(ahora) = &m["items"] else {
        panic!()
    };
    assert_eq!(antes.len(), ahora.len(), "un manifiesto no es un ítem");

    let Json::Arr(rs) = &m["repositorios"] else {
        panic!()
    };
    let dicho: Vec<(String, String, String, String)> = rs
        .iter()
        .map(|x| {
            let Json::Obj(x) = x else { panic!() };
            let s = |k: &str| match x.get(k) {
                Some(Json::Str(v)) => v.clone(),
                Some(Json::Int(v)) => v.to_string(),
                _ => "-".into(),
            };
            (s("ruta"), s("plantilla"), s("items"), s("roto"))
        })
        .collect();
    assert_eq!(
        dicho,
        vec![
            (
                "packages/ventas/espana".into(),
                "transforms".into(),
                "1".into(),
                "-".into()
            ),
            (
                "packages/ventas/espana/modelo".into(),
                "models".into(),
                "0".into(),
                "-".into()
            ),
            (
                "packages/ventas/roto".into(),
                "functions".into(),
                "0".into(),
                "sin `nombre`".into()
            ),
        ],
        "un README sin `plantilla` no es un repositorio, y lo roto se lista igual"
    );

    // 0036 ⑤: la versión de la clase, comparada con la del producto.
    let Json::Obj(uno) = &rs[0] else { panic!() };
    assert_eq!(
        uno["plantillaActual"],
        Json::Int(ore_core::clases::de("transforms").unwrap().version)
    );
    assert_eq!(
        uno["actualizable"],
        Json::Bool(2 < ore_core::clases::de("transforms").unwrap().version),
        "el manifiesto dice 2: actualizable si el producto va por más"
    );
    assert_eq!(uno["escribe"], Json::Bool(true), "un transforms escribe");
    let Json::Obj(dos) = &rs[1] else { panic!() };
    assert_eq!(
        dos["actualizable"],
        Json::Bool(true),
        "sin `plantillaVersion`, se puede actualizar"
    );

    // El ítem de la carpeta del cliente vive en SU repositorio; los demás, en ninguno.
    assert_eq!(
        item(&j, "view:ventas.pedidosEs")["repositorio"],
        Json::s("packages/ventas/espana")
    );
    assert_eq!(
        item(&j, "dataset:ventas.pedidos")["repositorio"],
        Json::Crudo("null".into())
    );
}

/// ⭐⭐ ORE 0041 · v1alpha15: el modelo vive en un paquete y un schema.
///
/// Uno en `ventas.espana` y otro con el mismo nombre en `ventas` (`default`)
/// son dos modelos; el índice les da paquete y schema (el catálogo los pinta);
/// una función de `espana` que dice `modelo/chat` es el de su schema; uno de
/// antes en la raíz sigue resolviendo; y dos en el mismo schema es OOS2035.
#[test]
fn el_modelo_de_v1alpha15_vive_en_su_paquete_y_su_schema() {
    let t = arbol_en("modelo15");
    let r = t.path();
    escribe(
        r,
        "packages/ventas/espana/schema.yaml",
        "apiVersion: oos.dev/v1alpha13\nkind: Schema\nmetadata:\n  name: espana\n  namespace: ventas\n",
    );
    escribe(
        r,
        "packages/ventas/espana/modelos/chat.yaml",
        "apiVersion: oos.dev/v1alpha15\nkind: Model\nmetadata:\n  name: chat\n  namespace: ventas\n  schema: espana\nspec:\n  profile: g1/llama-3.1-8b\n  tier: shared\n  task: chat\n",
    );
    escribe(
        r,
        "packages/ventas/modelos/chat.yaml",
        "apiVersion: oos.dev/v1alpha15\nkind: Model\nmetadata:\n  name: chat\n  namespace: ventas\nspec:\n  profile: g1/qwen3-8b\n  tier: shared\n  task: chat\n",
    );
    escribe(
        r,
        "packages/ventas/espana/functions/resumir.yaml",
        "apiVersion: oos.dev/v1alpha13\nkind: Function\nmetadata: { name: resumir, namespace: ventas, schema: espana }\nspec:\n  runtime: model\n  model: modelo/chat\n  over: ventas.pedidosEs\n  reads: [ventas.pedidosEs]\n  prompt: resume\n  output:\n    resumen: { type: String }\n",
    );

    let d = ore_core::validate::validate_package(r);
    let nuevos: Vec<String> = d
        .iter()
        .filter(|x| {
            let rel = x
                .file
                .strip_prefix(r)
                .unwrap_or(&x.file)
                .to_string_lossy()
                .replace('\\', "/");
            rel.contains("modelos/") || rel.contains("resumir")
        })
        .map(|x| format!("{:?} {} {}", x.code, x.file.display(), x.message))
        .collect();
    assert!(nuevos.is_empty(), "{nuevos:#?}");

    let (pkg, _) = ore_core::validate::cargar_paquete(r);
    let j = indice(&pkg, &punteros(r), &Cabeza::default());
    let es = item(&j, "model:ventas.espana.chat");
    assert_eq!(es["paquete"], Json::s("ventas"));
    assert_eq!(es["schema"], Json::s("espana"));
    assert_eq!(es["carpeta"], Json::s("espana"));
    let def = item(&j, "model:ventas.chat");
    assert_eq!(def["schema"], Json::s("default"));
    assert_eq!(def["carpeta"], Json::s(""));
    // la de antes, en la raíz, sigue ahí y sin paquete
    assert_eq!(
        item(&j, "model:v2-lite")["paquete"],
        Json::Crudo("null".into())
    );
    // la función de `espana` usa el de su schema, no el de `default`
    let f = item(&j, "function:ventas.espana.resumir");
    assert!(
        tiene(f, "usa", "model:ventas.espana.chat"),
        "{:?}",
        relaciones(f)
    );

    // dos en el mismo schema: OOS2035
    escribe(
        r,
        "packages/ventas/espana/modelos/otro.yaml",
        "apiVersion: oos.dev/v1alpha15\nkind: Model\nmetadata:\n  name: chat\n  namespace: ventas\n  schema: espana\nspec:\n  profile: g1/llama-3.1-8b\n  tier: shared\n  task: chat\n",
    );
    let d = ore_core::validate::validate_package(r);
    assert!(
        d.iter().any(|x| x.code == ore_core::code::Code::Oos2035),
        "{d:#?}"
    );
}

/// Un `Model` de antes con `namespace` es OOS1005: la clave es de v1alpha15.
#[test]
fn un_modelo_de_antes_no_lleva_namespace() {
    let d = ore_core::validate::validate_document(
        Path::new("modelos/x.yaml"),
        "apiVersion: oos.dev/v1alpha9\nkind: Model\nmetadata:\n  name: x\n  namespace: ventas\nspec:\n  profile: g1/x\n  tier: shared\n  task: chat\n",
    );
    assert!(
        d.iter().any(|x| x.code == ore_core::code::Code::Oos1005),
        "{d:#?}"
    );
}
