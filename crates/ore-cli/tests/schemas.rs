//! `ore package schema new|rename` (0038 P6), de punta a punta sobre un árbol
//! descubierto: una base con alcance —la que crea la consola— y lo que la
//! nombra desde fuera en tres partes.
//!
//! La medida que lo sostiene es `pruebas-de-fuego/medida-renombrar-schema.py`:
//! con la carpeta, la metadata, las referencias de tres partes, los punteros y
//! el `moved`, el árbol compila igual que antes.

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

fn leer(p: PathBuf) -> String {
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn escribir(p: PathBuf, t: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, t).unwrap();
}

/// Cuántos diagnósticos da el árbol: el renombrado no puede cambiarlo.
fn errores(dir: &Path) -> String {
    let (_, dicho) = ore(dir, &["validate", "."]);
    dicho
        .lines()
        .rev()
        .find(|l| l.contains("error"))
        .unwrap_or("0 errores")
        .trim()
        .to_string()
}

/// Una base `ventas` con dos tablas del origen (schema `rubix_demo_ventas`),
/// una View en `default` y un paquete `eu` que la leen en tres partes, un
/// `.sql`, un programa y el puntero de una copia.
fn taller(nombre: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ore-schemas-{}-{nombre}", std::process::id()));
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
        ],
    );
    assert_eq!(c, Some(0), "{dicho}");
    assert!(
        dir.join("packages/ventas/rubix_demo_ventas/schema.yaml")
            .is_file(),
        "discover no dejó el schema del origen:\n{dicho}"
    );
    escribir(
        dir.join("packages/ventas/views/resumen.yaml"),
        "apiVersion: oos.dev/v1alpha13\nkind: View\nmetadata: { name: resumen, namespace: ventas }\nspec:\n  owner: \"team:ventas\"\n  from: { table: ventas.rubix_demo_ventas.clientes_t }\n  fields: { id: id }\n",
    );
    ore(&dir, &["package", "new", "eu", "--owner", "team:eu"]);
    escribir(
        dir.join("packages/eu/views/copia.yaml"),
        "apiVersion: oos.dev/v1alpha13\nkind: View\nmetadata: { name: copia, namespace: eu }\nspec:\n  owner: \"team:eu\"\n  from: { table: ventas.rubix_demo_ventas.clientes_t }\n  fields: { id: id }\n",
    );
    escribir(
        dir.join("packages/ventas/transforms/cuenta.sql"),
        "create or replace dataset ventas.cuenta as\nselect count(*) as n from ventas.rubix_demo_ventas.clientes\n",
    );
    escribir(
        dir.join("packages/ventas/transforms/lee.py"),
        "df = ore.read(\"ventas.rubix_demo_ventas.clientes\")\n",
    );
    escribir(
        dir.join("datasets/ventas/rubix_demo_ventas/clientes.json"),
        "{\"metadata_location\":\"s3://b/ore/v2/catalogo/ventas/rubix_demo_ventas/clientes/metadata/00001-x.metadata.json\"}\n",
    );
    dir
}

#[test]
fn crear_un_schema_y_lo_que_se_niega() {
    let dir = taller("crear");
    let antes = errores(&dir);
    let (c, dicho) = ore(
        &dir,
        &[
            "package",
            "schema",
            "new",
            "ventas",
            "espana",
            "--description",
            "Lo de España",
            "--json",
        ],
    );
    assert_eq!(c, Some(0), "{dicho}");
    let t = leer(dir.join("packages/ventas/espana/schema.yaml"));
    assert!(
        t.contains("kind: Schema") && t.contains("name: espana"),
        "{t}"
    );
    assert!(t.contains("description: \"Lo de España\""), "{t}");
    assert!(dicho.contains("\"schema\": \"espana\""), "{dicho}");
    assert_eq!(errores(&dir), antes, "crear un schema vacío no cambia nada");

    // otra vez, o con otras mayúsculas: 73
    let (c, dicho) = ore(&dir, &["package", "schema", "new", "ventas", "Espana"]);
    assert_eq!(c, Some(73), "{dicho}");
    // los nombres que no pueden ser: 65
    for n in ["default", "information_schema", "tables", "1x", "a-b"] {
        let (c, dicho) = ore(&dir, &["package", "schema", "new", "ventas", n]);
        assert_eq!(c, Some(65), "{n}: {dicho}");
    }
    let (c, dicho) = ore(
        &dir,
        &[
            "package", "schema", "new", "ventas", "francia", "--owner", "cambiame",
        ],
    );
    assert_eq!(c, Some(65), "{dicho}");
    // en un paquete que no hay: 66
    let (c, dicho) = ore(&dir, &["package", "schema", "new", "nadie", "espana"]);
    assert_eq!(c, Some(66), "{dicho}");

    // ⛔ la puerta: una carpeta con algo de `default` dentro no se hace schema
    //   (OOS2036), y no queda nada escrito
    escribir(
        dir.join("packages/ventas/suelta/views/x.yaml"),
        "apiVersion: oos.dev/v1alpha13\nkind: View\nmetadata: { name: x, namespace: ventas }\nspec:\n  owner: \"team:ventas\"\n  from: { table: ventas.rubix_demo_ventas.clientes_t }\n  fields: { id: id }\n",
    );
    let antes = errores(&dir);
    let (c, dicho) = ore(&dir, &["package", "schema", "new", "ventas", "suelta"]);
    assert_eq!(c, Some(65), "{dicho}");
    assert!(dicho.contains("OOS2036"), "{dicho}");
    assert!(!dir.join("packages/ventas/suelta/schema.yaml").exists());
    assert!(
        dir.join("packages/ventas/suelta/views/x.yaml").is_file(),
        "la carpeta sigue"
    );
    assert_eq!(errores(&dir), antes);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn renombrar_un_schema_descubierto() {
    let dir = taller("renombrar");
    let antes = errores(&dir);
    let (c, dicho) = ore(
        &dir,
        &[
            "package",
            "schema",
            "rename",
            "ventas",
            "rubix_demo_ventas",
            "ventas_es",
            "--json",
        ],
    );
    assert_eq!(c, Some(0), "{dicho}");
    let p = |r: &str| dir.join(r);

    // ① la carpeta, entera
    assert!(!p("packages/ventas/rubix_demo_ventas").exists(), "{dicho}");
    let s = leer(p("packages/ventas/ventas_es/schema.yaml"));
    assert!(s.contains("name: ventas_es"), "{s}");
    let vista = leer(p("packages/ventas/ventas_es/views/Clientes__clientes.yaml"));
    // ② su metadata; lo suyo en una parte, tal cual
    assert!(vista.contains("schema: ventas_es"), "{vista}");
    // La vista inducida es una consulta (0040 paso 6), y nombra su tabla en
    // tres partes entre comillas: se reapunta como las demás.
    assert!(
        vista.contains(r#"FROM "ventas"."ventas_es"."clientes_t""#),
        "{vista}"
    );
    // ③ lo que lo nombra en tres partes: yaml y sql; el programa, no
    assert!(
        leer(p("packages/ventas/views/resumen.yaml"))
            .contains("table: ventas.ventas_es.clientes_t")
    );
    assert!(leer(p("packages/eu/views/copia.yaml")).contains("table: ventas.ventas_es.clientes_t"));
    assert!(
        leer(p("packages/ventas/transforms/cuenta.sql")).contains("from ventas.ventas_es.clientes")
    );
    assert!(
        leer(p("packages/ventas/transforms/lee.py")).contains("ventas.rubix_demo_ventas.clientes")
    );
    assert!(
        dicho.contains("packages/ventas/transforms/lee.py"),
        "se dice:\n{dicho}"
    );
    // ④ el puntero, en su sitio nuevo; los bytes del lago, donde estaban
    assert!(!p("datasets/ventas/rubix_demo_ventas").exists());
    let puntero = leer(p("datasets/ventas/ventas_es/clientes.json"));
    assert!(
        puntero.contains("catalogo/ventas/rubix_demo_ventas/clientes"),
        "{puntero}"
    );
    // ⑤ el anuncio
    let m = leer(p("packages/ventas/package.yaml"));
    assert!(
        m.contains("from: ventas.rubix_demo_ventas.Clientes, to: ventas.ventas_es.Clientes"),
        "{m}"
    );
    assert!(
        m.contains("from: ventas.rubix_demo_ventas.clientes, to: ventas.ventas_es.clientes"),
        "{m}"
    );
    // ⑥ la regla del alcance
    let a = leer(p("packages/ventas/discover.scope.json"));
    assert!(a.contains("\"rubix_demo_ventas\": \"ventas_es\""), "{a}");
    // y el árbol, igual que antes
    assert_eq!(errores(&dir), antes, "{dicho}");

    // La siguiente inducción NO lo deshace: sale en `ventas_es/`.
    let (c, dicho) = ore(&dir, &["review", "packages/ventas", "--reinducir"]);
    assert_eq!(c, Some(0), "{dicho}");
    assert!(
        !p("packages/ventas/rubix_demo_ventas").exists(),
        "la re-inducción volvió a la carpeta del origen:\n{dicho}"
    );
    assert!(p("packages/ventas/ventas_es/views/Clientes__clientes.yaml").is_file());
    assert!(
        leer(p("packages/ventas/ventas_es/views/Clientes__clientes.yaml"))
            .contains("schema: ventas_es")
    );
    assert_eq!(errores(&dir), antes);

    // Lo que se niega
    for (args, codigo) in [
        (["ventas", "default", "otro"], 65),
        (["ventas", "ventas_es", "VENTAS_ES"], 65),
        (["ventas", "ventas_es", "tables"], 65),
        (["ventas", "no_hay", "otro"], 66),
        (["nadie", "ventas_es", "otro"], 66),
    ] {
        let mut a = vec!["package", "schema", "rename"];
        a.extend(args);
        let (c, dicho) = ore(&dir, &a);
        assert_eq!(c, Some(codigo), "{args:?}: {dicho}");
    }
    ore(&dir, &["package", "schema", "new", "ventas", "ocupado"]);
    let (c, dicho) = ore(
        &dir,
        &[
            "package",
            "schema",
            "rename",
            "ventas",
            "ventas_es",
            "ocupado",
        ],
    );
    assert_eq!(c, Some(73), "{dicho}");

    // Y de vuelta al del origen: la regla sobra y se va.
    let (c, dicho) = ore(
        &dir,
        &[
            "package",
            "schema",
            "rename",
            "ventas",
            "ventas_es",
            "rubix_demo_ventas",
        ],
    );
    assert_eq!(c, Some(0), "{dicho}");
    let a = leer(p("packages/ventas/discover.scope.json"));
    assert!(!a.contains("schemas"), "{a}");
    assert!(
        leer(p("packages/eu/views/copia.yaml"))
            .contains("table: ventas.rubix_demo_ventas.clientes")
    );
    let _ = std::fs::remove_dir_all(&dir);
}
