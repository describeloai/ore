//! **La re-inducción retira sólo lo que escribió el inductor** (0038).
//!
//! Medido antes: `review --reinducir` —lo mismo que corren `model` y `copy`
//! desde el catálogo— borraba todo lo que la inducción nueva no producía en
//! las carpetas que gobierna. Sobre una base descubierta se llevaba una vista
//! escrita a mano en `views/` (lo que abre «Create › View»), un schema creado
//! desde el catálogo con su vista (P6a) y una vista a mano dentro del schema
//! del origen. Ahora lo inducido lleva su marca en la primera línea, y sólo lo
//! marcado se retira.

use std::path::{Path, PathBuf};
use std::process::Command;

fn ore(dir: &Path, args: &[&str]) -> (Option<i32>, String) {
    let s = Command::new(env!("CARGO_BIN_EXE_ore"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("no se pudo invocar `ore`");
    (
        s.status.code(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&s.stdout),
            String::from_utf8_lossy(&s.stderr)
        ),
    )
}

fn escribir(p: PathBuf, t: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, t).unwrap();
}

/// Una base descubierta con dos tablas del schema del origen `rubix_demo_ventas`.
fn base(nombre: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ore-reinduccion-{}-{nombre}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    ore(&dir, &["init", ".", "--name", "demo"]);
    let cat = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/catalogos/bigquery-rubix-demo-ventas.json");
    std::fs::copy(&cat, dir.join("cat.json")).unwrap();
    let (c, dicho) = ore(
        &dir,
        &[
            "discover",
            "--from",
            "cat.json",
            "--out",
            "packages/ventas",
            "--only",
            "rubix_demo_ventas.clientes",
            "--only",
            "rubix_demo_ventas.Pedidos",
            "--owner",
            "team:ventas",
            "--no-model",
            "--type",
            "foreign",
        ],
    );
    assert_eq!(c, Some(0), "{dicho}");
    dir
}

const VISTA: &str = "apiVersion: oos.dev/v1alpha13\nkind: View\nmetadata: { name: NOMBRE, namespace: ventas SCHEMA}\nspec:\n  owner: \"team:ventas\"\n  from: { table: ventas.rubix_demo_ventas.clientes }\n  fields: { id: id }\n";

fn vista(nombre: &str, schema: Option<&str>) -> String {
    VISTA.replace("NOMBRE", nombre).replace(
        "SCHEMA",
        &schema
            .map(|s| format!(", schema: {s} "))
            .unwrap_or_default(),
    )
}

/// Lo escrito a mano en una base descubierta: una vista en `default`, un
/// schema creado desde el catálogo con su vista, y una vista dentro del schema
/// del origen.
fn a_mano(dir: &Path) -> Vec<PathBuf> {
    let (c, dicho) = ore(dir, &["package", "schema", "new", "ventas", "espana"]);
    assert_eq!(c, Some(0), "{dicho}");
    let p = |r: &str| dir.join("packages/ventas").join(r);
    escribir(p("views/resumen.yaml"), &vista("resumen", None));
    escribir(p("espana/views/mia.yaml"), &vista("mia", Some("espana")));
    escribir(
        p("rubix_demo_ventas/views/otra.yaml"),
        &vista("otra", Some("rubix_demo_ventas")),
    );
    vec![
        p("views/resumen.yaml"),
        p("espana/schema.yaml"),
        p("espana/views/mia.yaml"),
        p("rubix_demo_ventas/views/otra.yaml"),
    ]
}

#[test]
fn lo_inducido_lleva_su_marca_y_lo_escrito_a_mano_sobrevive() {
    let dir = base("a-mano");
    let inducida = dir.join("packages/ventas/rubix_demo_ventas/views/Clientes__clientes.yaml");
    let t = std::fs::read_to_string(&inducida).unwrap();
    assert!(t.starts_with("# ore discover:"), "{t}");
    let manifiesto = std::fs::read_to_string(dir.join("packages/ventas/package.yaml")).unwrap();
    assert!(!manifiesto.contains("# ore discover:"), "{manifiesto}");

    let suyos = a_mano(&dir);
    let (c, dicho) = ore(&dir, &["review", "packages/ventas", "--reinducir"]);
    assert_eq!(c, Some(0), "{dicho}");
    for f in &suyos {
        assert!(
            f.is_file(),
            "la re-inducción se llevó {}:\n{dicho}",
            f.display()
        );
    }
    assert!(inducida.is_file(), "{dicho}");
    assert!(!dicho.contains("retirado"), "{dicho}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lo_inducido_que_ya_no_sale_se_retira() {
    let dir = base("sobra");
    let suyos = a_mano(&dir);
    // `Pedidos` sale del alcance: lo suyo, que era del inductor, se va.
    let alcance = dir.join("packages/ventas/discover.scope.json");
    let t = std::fs::read_to_string(&alcance).unwrap();
    std::fs::write(
        &alcance,
        t.replace(",\n    \"rubix_demo_ventas.Pedidos\"", "")
            .replace("\"rubix_demo_ventas.Pedidos\",", ""),
    )
    .unwrap();
    assert!(
        !std::fs::read_to_string(&alcance)
            .unwrap()
            .contains("Pedidos")
    );
    let (c, dicho) = ore(&dir, &["review", "packages/ventas", "--reinducir"]);
    assert_eq!(c, Some(0), "{dicho}");
    let s = dir.join("packages/ventas/rubix_demo_ventas");
    assert!(!s.join("tables/Pedidos__Pedidos.yaml").exists(), "{dicho}");
    assert!(!s.join("views/Pedidos__Pedidos.yaml").exists(), "{dicho}");
    assert!(
        s.join("tables/Clientes__clientes.yaml").is_file(),
        "{dicho}"
    );
    for f in &suyos {
        assert!(f.is_file(), "se llevó {}:\n{dicho}", f.display());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Un árbol inducido antes de la marca: la regla de antes, con sus dos
/// límites (ni un schema que la inducción no produce, ni un nombre que no es
/// del inductor en tables/views/datasets).
#[test]
fn un_arbol_de_antes_sin_marca() {
    let dir = base("antes");
    // Sin marcas, como lo dejaba un `discover` de antes.
    for e in walk(&dir.join("packages/ventas")) {
        if e.extension().is_some_and(|x| x == "yaml") {
            let t = std::fs::read_to_string(&e).unwrap();
            if let Some(r) = t.strip_prefix(&format!("{}\n", marca(&t))) {
                std::fs::write(&e, r).unwrap();
            }
        }
    }
    let suyos = a_mano(&dir);
    // y un resto del inductor que ya no sale: se retira
    let resto = dir.join("packages/ventas/rubix_demo_ventas/views/Viejo__viejo.yaml");
    escribir(resto.clone(), &vista("viejo", Some("rubix_demo_ventas")));
    let (c, dicho) = ore(&dir, &["review", "packages/ventas", "--reinducir"]);
    assert_eq!(c, Some(0), "{dicho}");
    for f in &suyos {
        assert!(f.is_file(), "se llevó {}:\n{dicho}", f.display());
    }
    assert!(!resto.exists(), "el resto del inductor sigue:\n{dicho}");
    // y tras esta pasada, lo inducido ya lleva marca
    let t = std::fs::read_to_string(
        dir.join("packages/ventas/rubix_demo_ventas/views/Clientes__clientes.yaml"),
    )
    .unwrap();
    assert!(t.starts_with("# ore discover:"), "{t}");
    let _ = std::fs::remove_dir_all(&dir);
}

fn marca(t: &str) -> String {
    t.lines()
        .next()
        .filter(|l| l.starts_with("# ore discover:"))
        .unwrap_or("\u{0}")
        .to_string()
}

fn walk(d: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(d).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}
