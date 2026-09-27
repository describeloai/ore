//! **El puntero es del objeto, no de quien lo lee** (0045 P1).
//!
//! Medido antes en `victor`: la misma tabla `products` de una fuente era
//! `upsert · key: [id]` en la base estándar y `mode: none` en la foránea, porque
//! el inductor solo escribía la clave si la base copiaba, y cada base guardaba
//! sus respuestas de clave y de tipo. Dos punteros distintos del mismo objeto.
//!
//! Ahora la tabla lleva la clave que se conoce, se copie o no, y `clave/*` y
//! `tipo/*` se contestan una vez, en el paquete de la fuente.

use std::path::{Path, PathBuf};
use std::process::Command;

const FUENTE: &str = "bq_ventas";

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

/// Un repositorio con el paquete de la fuente —como lo deja el Job de catálogo:
/// `package.yaml` y `discover.catalog.json`— y dos bases de ella, una foránea
/// y una estándar, con las mismas dos tablas.
fn arbol(nombre: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ore-puntero-{}-{nombre}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    ore(&dir, &["init", ".", "--name", "demo"]);
    let cat = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/catalogos/bigquery-rubix-demo-ventas.json");
    std::fs::copy(&cat, dir.join("cat.json")).unwrap();
    let (c, dicho) = ore(
        &dir,
        &[
            "package",
            "new",
            FUENTE,
            "--owner",
            "team:demo",
            "--path",
            ".",
        ],
    );
    assert_eq!(c, Some(0), "{dicho}");
    std::fs::copy(
        &cat,
        dir.join(format!("packages/{FUENTE}/discover.catalog.json")),
    )
    .unwrap();
    for (base, tipo) in [("fdb", "foreign"), ("sdb", "standard")] {
        let out = format!("packages/{base}");
        let (c, dicho) = ore(
            &dir,
            &[
                "discover",
                "--from",
                "cat.json",
                "--out",
                &out,
                "--only",
                "rubix_demo_ventas.clientes",
                "--only",
                "rubix_demo_ventas.Pedidos",
                "--owner",
                "team:demo",
                "--no-model",
                "--type",
                tipo,
            ],
        );
        assert_eq!(c, Some(0), "{base}: {dicho}");
    }
    dir
}

/// La `Table` de `objeto` en la base, sin la línea que dice de quién es: lo que
/// queda es lo que dice del objeto.
fn tabla(dir: &Path, base: &str, objeto: &str) -> String {
    let raiz = dir.join("packages").join(base);
    let mut pila = vec![raiz];
    while let Some(d) = pila.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                pila.push(p);
                continue;
            }
            let Ok(t) = std::fs::read_to_string(&p) else {
                continue;
            };
            if t.contains("kind: Table") && t.contains(&format!("object: \"{objeto}\"")) {
                return t
                    .lines()
                    .filter(|l| !l.contains("namespace:"))
                    .collect::<Vec<_>>()
                    .join("\n");
            }
        }
    }
    panic!("no hay Table de `{objeto}` en `{base}`");
}

fn respuestas(dir: &Path, paquete: &str) -> String {
    std::fs::read_to_string(dir.join(format!("packages/{paquete}/discover.answers.json")))
        .unwrap_or_default()
}

/// La foránea se re-induce después de que la estándar exista: el objeto se
/// copia en la fuente, y su tabla es la misma en las dos.
#[test]
fn si_alguna_base_copia_el_objeto_su_tabla_es_la_misma_en_todas() {
    let dir = arbol("origen");
    let (c, dicho) = ore(&dir, &["review", "packages/fdb", "--reinducir"]);
    assert_eq!(c, Some(0), "{dicho}");
    let f = tabla(&dir, "fdb", "rubix_demo_ventas.clientes");
    let s = tabla(&dir, "sdb", "rubix_demo_ventas.clientes");
    assert!(
        f.contains("key: [id, cod_pais]"),
        "la foránea no lleva la clave:\n{f}"
    );
    assert_eq!(f, s, "dos punteros distintos del mismo objeto");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Y si nadie lo copia, la tabla es lo que el driver sondeó, tal cual: ni
/// `drift-detect` ni el motor ven una cara que el origen no dijo.
#[test]
fn si_nadie_copia_el_objeto_su_tabla_es_la_que_sondeo_el_driver() {
    let dir = arbol("nadie");
    std::fs::remove_dir_all(dir.join("packages/sdb")).unwrap();
    let (c, dicho) = ore(&dir, &["review", "packages/fdb", "--reinducir"]);
    assert_eq!(c, Some(0), "{dicho}");
    let f = tabla(&dir, "fdb", "rubix_demo_ventas.clientes");
    assert!(f.contains("mode: none"), "{f}");
    assert!(!f.contains("key:"), "{f}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn la_clave_contestada_es_de_la_fuente_y_la_ven_todas_sus_bases() {
    let dir = arbol("contestada");
    std::fs::write(
        dir.join("r.yaml"),
        "answers:\n  clave/rubix_demo_ventas.Pedidos: [Id]\n",
    )
    .unwrap();
    let (c, dicho) = ore(&dir, &["review", "packages/sdb", "--answers", "r.yaml"]);
    assert_eq!(c, Some(0), "{dicho}");

    // Guardada en la fuente, y no en la base que la contestó.
    assert!(
        respuestas(&dir, FUENTE).contains("clave/rubix_demo_ventas.Pedidos"),
        "la fuente no la guarda: {}",
        respuestas(&dir, FUENTE)
    );
    assert!(
        !respuestas(&dir, "sdb").contains("clave/"),
        "la base se la quedó: {}",
        respuestas(&dir, "sdb")
    );

    // La otra base la ve en cuanto vuelve a inducir, sin contestarla.
    let (c, dicho) = ore(&dir, &["review", "packages/fdb", "--reinducir"]);
    assert_eq!(c, Some(0), "{dicho}");
    let f = tabla(&dir, "fdb", "rubix_demo_ventas.Pedidos");
    assert!(f.contains("key: [Id]"), "{f}");
    assert_eq!(f, tabla(&dir, "sdb", "rubix_demo_ventas.Pedidos"));

    // Y una base nueva de la misma fuente nace con ella.
    let (c, dicho) = ore(
        &dir,
        &[
            "discover",
            "--from",
            "cat.json",
            "--out",
            "packages/otra",
            "--only",
            "rubix_demo_ventas.Pedidos",
            "--owner",
            "team:demo",
            "--no-model",
            "--type",
            "foreign",
        ],
    );
    assert_eq!(c, Some(0), "{dicho}");
    assert!(tabla(&dir, "otra", "rubix_demo_ventas.Pedidos").contains("key: [Id]"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn una_base_que_contesto_distinto_lo_dice_y_no_reinduce() {
    let dir = arbol("choque");
    std::fs::write(
        dir.join("r.yaml"),
        "answers:\n  clave/rubix_demo_ventas.Pedidos: [Id]\n",
    )
    .unwrap();
    let (c, dicho) = ore(&dir, &["review", "packages/sdb", "--answers", "r.yaml"]);
    assert_eq!(c, Some(0), "{dicho}");

    // Lo de antes de 0045: la base guardaba su propia respuesta, y otra.
    let antes = tabla(&dir, "fdb", "rubix_demo_ventas.Pedidos");
    std::fs::write(
        dir.join("packages/fdb/discover.answers.json"),
        "{\"answers\":{\"clave/rubix_demo_ventas.Pedidos\":[\"Total\"]}}",
    )
    .unwrap();
    let (c, dicho) = ore(&dir, &["review", "packages/fdb", "--reinducir"]);
    assert_eq!(c, Some(65), "{dicho}");
    assert!(dicho.contains("contestó distinto de su fuente"), "{dicho}");
    assert!(dicho.contains("clave/rubix_demo_ventas.Pedidos"), "{dicho}");
    assert_eq!(
        antes,
        tabla(&dir, "fdb", "rubix_demo_ventas.Pedidos"),
        "re-indujo igual"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
