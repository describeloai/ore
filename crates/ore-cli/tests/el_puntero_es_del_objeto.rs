//! **El puntero es del objeto, no de quien lo lee** (0045 P1′).
//!
//! Medido antes en `victor`: la misma tabla `products` de una fuente era
//! `upsert · key: [id]` en la base estándar y `mode: none` en la foránea: el
//! inductor escribía en la tabla lo que la COPIA necesitaba, y cada base
//! guardaba sus respuestas de clave y de tipo. Dos punteros del mismo objeto.
//!
//! Ahora la cara `D` de la tabla es la del origen, tal cual la sondeó el
//! driver, con la clave si se conoce; y `clave/*` y `tipo/*` se contestan una
//! vez, en el paquete de la fuente. Y lo que el origen no deja copiar —una
//! entidad sobre un origen que solo anexa— no se copia, y se dice.

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

/// El catálogo de prueba; con `pedidos_anexa`, `Pedidos` sale como un origen
/// que solo anexa (`{ append, log }`), que es lo que dice Postgres de una tabla
/// sin clave primaria con WAL lógico.
fn catalogo(pedidos_anexa: bool) -> String {
    let t = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/catalogos/bigquery-rubix-demo-ventas.json"),
    )
    .unwrap()
    .replace("\r\n", "\n");
    if !pedidos_anexa {
        return t;
    }
    let muda = "\"mode\": \"none\",\n        \"witness\": \"none\"";
    assert!(t.find(muda).is_some() && t.find(muda) < t.find("rubix_demo_ventas.Pedidos"));
    t.replacen(
        muda,
        "\"mode\": \"append\",\n        \"witness\": \"log\"",
        1,
    )
}

/// Un repositorio con el paquete de la fuente —como lo deja el Job de catálogo:
/// `package.yaml` y `discover.catalog.json`— y dos bases de ella, una foránea
/// y una estándar, con las mismas dos tablas.
fn arbol(nombre: &str, pedidos_anexa: bool) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ore-puntero-{}-{nombre}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    ore(&dir, &["init", ".", "--name", "demo"]);
    // La fuente declarada, como la deja `ore source add`: sin ella el árbol
    // para en `OOS2004` y nunca llega a las reglas de mantenimiento.
    let manifiesto = dir.join("ontology.config.yaml");
    let mut m = std::fs::read_to_string(&manifiesto).unwrap();
    m.push_str(&format!(
        "\ndatasources:\n  - {{ name: {FUENTE}, type: bigquery, connectionEnv: BQ_VENTAS_URL }}\n"
    ));
    std::fs::write(&manifiesto, m).unwrap();
    let cat = catalogo(pedidos_anexa);
    std::fs::write(dir.join("cat.json"), &cat).unwrap();
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

/// Los ficheros de `base` cuyo texto cumple `f`.
fn ficheros(dir: &Path, base: &str, f: impl Fn(&str) -> bool) -> Vec<String> {
    let mut out = Vec::new();
    let mut pila = vec![dir.join("packages").join(base)];
    while let Some(d) = pila.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                pila.push(p);
            } else if let Ok(t) = std::fs::read_to_string(&p)
                && f(&t)
            {
                out.push(t);
            }
        }
    }
    out
}

/// La `Table` de `objeto` en la base, sin la línea que dice de quién es: lo que
/// queda es lo que dice del objeto.
fn tabla(dir: &Path, base: &str, objeto: &str) -> String {
    let t = ficheros(dir, base, |t| {
        t.contains("kind: Table") && t.contains(&format!("object: \"{objeto}\""))
    });
    let t = t
        .first()
        .unwrap_or_else(|| panic!("no hay Table de `{objeto}` en `{base}`"));
    t.lines()
        .filter(|l| !l.contains("namespace:"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn respuestas(dir: &Path, paquete: &str) -> String {
    std::fs::read_to_string(dir.join(format!("packages/{paquete}/discover.answers.json")))
        .unwrap_or_default()
}

/// La foránea y la estándar escriben la misma tabla: la cara del origen tal
/// cual (`mode: none`, que es lo que sondeó el driver) y su clave.
#[test]
fn la_tabla_es_la_misma_la_lea_quien_la_lea() {
    let dir = arbol("misma", false);
    let f = tabla(&dir, "fdb", "rubix_demo_ventas.clientes");
    assert!(
        f.contains("mode: none"),
        "la cara no es la del origen:\n{f}"
    );
    assert!(f.contains("key: [id, cod_pais]"), "sin la clave:\n{f}");
    assert!(!f.contains("upsert"), "la copia escrita en el objeto:\n{f}");
    assert_eq!(
        f,
        tabla(&dir, "sdb", "rubix_demo_ventas.clientes"),
        "dos punteros distintos del mismo objeto"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn la_clave_contestada_es_de_la_fuente_y_la_ven_todas_sus_bases() {
    let dir = arbol("contestada", false);
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

/// Una base de antes de 0045 guardaba su propia clave. Manda la de la fuente,
/// se dice, y la base puede seguir revisándose: lo viejo se va al guardar.
#[test]
fn en_un_choque_manda_la_fuente_y_se_dice() {
    let dir = arbol("choque", false);
    std::fs::write(
        dir.join("r.yaml"),
        "answers:\n  clave/rubix_demo_ventas.Pedidos: [Id]\n",
    )
    .unwrap();
    let (c, dicho) = ore(&dir, &["review", "packages/sdb", "--answers", "r.yaml"]);
    assert_eq!(c, Some(0), "{dicho}");

    std::fs::write(
        dir.join("packages/fdb/discover.answers.json"),
        "{\"answers\":{\"clave/rubix_demo_ventas.Pedidos\":[\"Total\"]}}",
    )
    .unwrap();
    let (c, dicho) = ore(&dir, &["review", "packages/fdb", "--reinducir"]);
    assert_eq!(c, Some(0), "{dicho}");
    assert!(
        dicho.contains("contestada distinto de su fuente"),
        "{dicho}"
    );
    assert!(tabla(&dir, "fdb", "rubix_demo_ventas.Pedidos").contains("key: [Id]"));
    assert!(
        !respuestas(&dir, "fdb").contains("clave/"),
        "la base conserva la vieja: {}",
        respuestas(&dir, "fdb")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// ⛔ El caso que P1 escondía. `Pedidos` solo anexa (Postgres sin clave
/// primaria lo dice así a propósito), se modela en la estándar y alguien
/// contesta una clave: la tabla gana la clave y sigue diciendo `append`, y la
/// copia de la entidad NO se emite —lo borrado en el origen no llegaría—; el
/// informe lo dice y el árbol compila (sin `OOS2021` y sin mentir para
/// evitarlo).
#[test]
fn una_entidad_sobre_un_origen_que_solo_anexa_no_se_copia_y_se_dice() {
    let dir = arbol("anexa", true);
    let (c, dicho) = ore(
        &dir,
        &["model", "packages/sdb", "rubix_demo_ventas.Pedidos"],
    );
    assert_eq!(c, Some(0), "{dicho}");
    // En JSON: un `.yaml` en la raíz lo compilaría `validate`.
    std::fs::write(
        dir.join("r.json"),
        "{\"answers\":{\"clave/rubix_demo_ventas.Pedidos\":[\"Id\"]}}",
    )
    .unwrap();
    let (c, dicho) = ore(&dir, &["review", "packages/sdb", "--answers", "r.json"]);
    assert_eq!(c, Some(0), "{dicho}");
    assert!(dicho.contains("no se copia"), "{dicho}");
    assert!(dicho.contains("OOS2021"), "{dicho}");

    let t = tabla(&dir, "sdb", "rubix_demo_ventas.Pedidos");
    assert!(t.contains("mode: append"), "{t}");
    assert!(t.contains("key: [Id]"), "{t}");
    let copias = ficheros(&dir, "sdb", |t| {
        t.contains("kind: Dataset") && t.to_lowercase().contains("pedidos")
    });
    assert!(copias.is_empty(), "se copió igual:\n{copias:?}");

    // El árbol llega a las reglas de mantenimiento y `OOS2021` no salta: no hay
    // copia que la dispare, ni una tabla que mienta para esquivarla. Lo único
    // que queda es `OOS4011` —el conducto de la copia de `clientes` sin
    // autorizar—, que lo declara ore-serve al inducir, no el CLI.
    let (_, dicho) = ore(&dir, &["validate", "."]);
    assert!(!dicho.contains("OOS2021"), "{dicho}");
    let errores: Vec<&str> = dicho.lines().filter(|l| l.starts_with("error[")).collect();
    assert!(
        errores.iter().all(|l| l.starts_with("error[OOS4011]")),
        "{dicho}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
