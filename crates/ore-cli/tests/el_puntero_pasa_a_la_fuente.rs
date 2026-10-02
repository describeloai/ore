//! **`ore migrate punteros`** (0045 P4): un árbol de antes de P3′ —la `Table`
//! de cada objeto en cada base que lo lee— pasa a tener el puntero una vez, en
//! la fuente, sin re-inducir las bases: se reapuntan.

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

/// Un árbol de antes: dos bases de la fuente —foránea y estándar— con sus
/// propios punteros (la fuente aún sin paquete, como antes de P3′), una clave
/// contestada en la estándar, y DESPUÉS el paquete de la fuente, como lo deja
/// el Job de catálogo.
fn arbol_de_antes(nombre: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("ore-migra-puntero-{}-{nombre}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    ore(&dir, &["init", ".", "--name", "demo"]);
    let manifiesto = dir.join("ontology.config.yaml");
    let mut m = std::fs::read_to_string(&manifiesto).unwrap();
    m.push_str(&format!(
        "\ndatasources:\n  - {{ name: {FUENTE}, type: bigquery, connectionEnv: BQ_VENTAS_URL }}\n"
    ));
    std::fs::write(&manifiesto, m).unwrap();
    let cat = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/catalogos/bigquery-rubix-demo-ventas.json"),
    )
    .unwrap();
    std::fs::write(dir.join("cat.json"), &cat).unwrap();
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
    std::fs::write(
        dir.join("r.json"),
        "{\"answers\":{\"clave/rubix_demo_ventas.Pedidos\":[\"Id\"]}}",
    )
    .unwrap();
    let (c, dicho) = ore(&dir, &["review", "packages/sdb", "--answers", "r.json"]);
    assert_eq!(c, Some(0), "{dicho}");
    std::fs::remove_file(dir.join("r.json")).unwrap();
    assert!(
        std::fs::read_to_string(dir.join("packages/sdb/discover.answers.json"))
            .unwrap()
            .contains("clave/"),
        "sin fuente, la clave es de la base (el árbol de antes)"
    );
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
    std::fs::write(
        dir.join(format!("packages/{FUENTE}/discover.catalog.json")),
        &cat,
    )
    .unwrap();
    std::fs::remove_file(dir.join("cat.json")).unwrap();
    dir
}

/// Los ficheros de un paquete cuyo texto cumple `f`, con su ruta.
fn ficheros(dir: &Path, base: &str, f: impl Fn(&str) -> bool) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    let mut pila = vec![dir.join("packages").join(base)];
    while let Some(d) = pila.pop() {
        let Ok(es) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in es.flatten() {
            let p = e.path();
            if p.is_dir() {
                pila.push(p);
            } else if let Ok(t) = std::fs::read_to_string(&p)
                && f(&t)
            {
                out.push((p, t));
            }
        }
    }
    out
}

fn tablas(dir: &Path, base: &str) -> usize {
    ficheros(dir, base, |t| t.contains("kind: Table")).len()
}

#[test]
fn el_puntero_pasa_a_la_fuente_y_las_bases_se_reapuntan() {
    let dir = arbol_de_antes("pasa");
    let (_, antes) = ore(&dir, &["validate", "."]);
    assert_eq!((tablas(&dir, "fdb"), tablas(&dir, "sdb")), (2, 2));

    // En seco: lo dice, y no escribe nada.
    let (c, dicho) = ore(&dir, &["migrate", "punteros", ".", "--seco"]);
    assert_eq!(c, Some(0), "{dicho}");
    assert!(dicho.contains("4 puntero(s) fuera de las bases"), "{dicho}");
    assert_eq!(
        (
            tablas(&dir, "fdb"),
            tablas(&dir, "sdb"),
            tablas(&dir, FUENTE)
        ),
        (2, 2, 0)
    );

    let (c, dicho) = ore(&dir, &["migrate", "punteros", "."]);
    assert_eq!(c, Some(0), "{dicho}");
    assert!(dicho.contains("mismos diagnósticos"), "{dicho}");
    assert!(
        dicho.contains("1 respuesta(s) del objeto a la fuente"),
        "{dicho}"
    );
    assert_eq!(
        (
            tablas(&dir, "fdb"),
            tablas(&dir, "sdb"),
            tablas(&dir, FUENTE)
        ),
        // ⭐ 0046 E5′: la fuente escribe todo su catálogo, no solo lo que
        //   leían las dos bases.
        (0, 0, 12)
    );

    // Lo que se contestó en la base es ahora de la fuente, y su puntero lo lleva.
    let resp = |p: &str| {
        std::fs::read_to_string(dir.join(format!("packages/{p}/discover.answers.json")))
            .unwrap_or_default()
    };
    assert!(
        resp(FUENTE).contains("clave/rubix_demo_ventas.Pedidos"),
        "{}",
        resp(FUENTE)
    );
    assert!(!resp("sdb").contains("clave/"), "{}", resp("sdb"));
    let pedidos = ficheros(&dir, FUENTE, |t| {
        t.contains("object: \"rubix_demo_ventas.Pedidos\"")
    });
    assert!(pedidos[0].1.contains("key: [Id]"), "{}", pedidos[0].1);

    // Las bases leen la fuente: el Dataset por `from`, la View por su consulta.
    let d = ficheros(&dir, "sdb", |t| t.contains("kind: Dataset"));
    assert!(
        d.iter().any(|(_, t)| t.contains(&format!(
            "from: {{ table: {FUENTE}.rubix_demo_ventas.Pedidos }}"
        ))),
        "{d:?}"
    );
    let v = ficheros(&dir, "fdb", |t| t.contains("kind: View"));
    assert!(
        v.iter()
            .any(|(_, t)| t.contains(&format!("\"{FUENTE}\".\"rubix_demo_ventas\".\"clientes\""))),
        "{v:?}"
    );
    assert!(
        !v.iter().any(|(_, t)| t.contains("_t\"")),
        "queda un `_t`: {v:?}"
    );

    // El árbol dice lo mismo que antes.
    let (_, despues) = ore(&dir, &["validate", "."]);
    assert_eq!(
        antes.lines().last(),
        despues.lines().last(),
        "antes:\n{antes}\ndespués:\n{despues}"
    );

    // Y una segunda pasada no tiene nada que mover.
    let (c, dicho) = ore(&dir, &["migrate", "punteros", "."]);
    assert_eq!(c, Some(0), "{dicho}");
    assert!(dicho.contains("nada que mover"), "{dicho}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Una etiqueta que alguien puso en un puntero de una base no se pierde en
/// silencio: no se migra, y no se escribe nada.
#[test]
fn una_etiqueta_que_se_perderia_para_la_migracion() {
    let dir = arbol_de_antes("etiqueta");
    // En `Pedidos`, que no tiene columnas garantizadas y sigue en su versión.
    // `clientes` declara `required` y es v1alpha22, donde `labels` en una
    // columna ya es `OOS1005` (`01-nunca-nula` §2): ahí el árbol ni carga.
    let (ruta, t) = ficheros(&dir, "fdb", |t| {
        t.contains("kind: Table") && t.contains("object: \"rubix_demo_ventas.Pedidos\"")
    })
    .remove(0);
    let col = t
        .lines()
        .find(|l| l.trim_start().starts_with("Id:"))
        .expect("la columna Id")
        .to_string();
    let con = col.replacen(" }", ", labels: { nota: revisada } }", 1);
    assert_ne!(col, con, "{col}");
    std::fs::write(&ruta, t.replacen(&col, &con, 1)).unwrap();

    let (c, dicho) = ore(&dir, &["migrate", "punteros", "."]);
    assert_eq!(c, Some(65), "{dicho}");
    assert!(dicho.contains("etiquetas"), "{dicho}");
    assert!(dicho.contains("nota"), "{dicho}");
    assert_eq!(
        (
            tablas(&dir, "fdb"),
            tablas(&dir, "sdb"),
            tablas(&dir, FUENTE)
        ),
        (2, 2, 0)
    );
    let _ = std::fs::remove_dir_all(&dir);
}
