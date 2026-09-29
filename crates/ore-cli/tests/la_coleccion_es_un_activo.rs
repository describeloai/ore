//! 0046 E8·1d · **La colección es un activo de primera clase**, como el
//! dataset: `ore view` dice su raíz —lo que el Job de la copia busca para
//! abrir su fuente—, `ore collections` la lista con su forma y su puntero,
//! `ore datasets` no la cuenta como dataset aunque su puntero viva junto a los
//! suyos, y el índice de activos lleva su puntero.
//!
//! El árbol es el caso de conformidad `v1alpha16/valid/a-virtual-collection`:
//! la colección virtual `legal.archivo.contratos` sobre el `ObjectTable`
//! `s3_ventas.docs.contratos`.

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

fn copiar(de: &Path, a: &Path) {
    std::fs::create_dir_all(a).unwrap();
    for e in std::fs::read_dir(de).unwrap().flatten() {
        let p = e.path();
        let q = a.join(e.file_name());
        if p.is_dir() {
            copiar(&p, &q);
        } else {
            std::fs::copy(&p, &q).unwrap();
        }
    }
}

fn arbol(nombre: &str) -> PathBuf {
    let caso = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/oos/conformance/v1alpha16/valid/a-virtual-collection/input");
    let dir = std::env::temp_dir().join(format!("ore-coleccion-{}-{nombre}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    copiar(&caso, &dir);
    dir
}

/// El puntero de su primera transacción, como lo deja `ore materialize`.
const PUNTERO: &str = r#"{
  "cambios": { "entran": 4, "fuera_de_formato": 0, "pierden": 0, "retiran": 0 },
  "dataset": "colecciones/legal/archivo/contratos",
  "estado": "transaccion",
  "items": { "actuales": 4, "perdidos": 0, "retirados": 0 },
  "kind": "MediaCollection",
  "metadata_location": "s3://copia/ore/v2/colecciones/legal/archivo/contratos/metadata/00000-x.metadata.json",
  "snapshot": "1",
  "testigo": { "modo": "listing", "valor": "sha256:abc" },
  "transaccion": 1,
  "virtual": true,
  "vista": "legal.archivo.contratos"
}
"#;

#[test]
fn ore_view_dice_la_raiz_de_la_coleccion() {
    let dir = arbol("view");
    let (codigo, salida) = ore(&dir, &["view", "."]);
    assert_eq!(codigo, Some(0), "{salida}");
    // Las dos líneas que el Job de la copia lee: el nombre y su `raíz`.
    let mut lineas = salida
        .lines()
        .skip_while(|l| *l != "legal.archivo.contratos");
    assert!(lineas.next().is_some(), "{salida}");
    let raiz = lineas
        .find(|l| l.starts_with("  raíz"))
        .expect("una línea `raíz`");
    assert!(
        raiz.contains("s3_ventas · el listado de `Nueva carpeta/contratos/`"),
        "{raiz}"
    );
    assert!(salida.contains("colección document · virtual"), "{salida}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ore_collections_la_lista_y_ore_datasets_no() {
    let dir = arbol("lista");
    let (c, sin) = ore(&dir, &["collections", ".", "--json"]);
    assert_eq!(c, Some(0), "{sin}");
    assert!(sin.contains(r#""forma":"virtual""#), "{sin}");
    assert!(
        sin.contains(r#""puntero":null"#),
        "sin transacción todavía: {sin}"
    );

    let p = dir.join("datasets/legal/archivo/contratos.json");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, PUNTERO).unwrap();
    let (_, con) = ore(&dir, &["collections", ".", "--json"]);
    assert!(con.contains(r#""transaccion":1"#), "{con}");
    assert!(con.contains(r#""actuales":4"#), "{con}");
    assert!(
        con.contains(r#""objectTable":"s3_ventas.docs.contratos""#),
        "{con}"
    );

    // Vive con los punteros de los datasets, y no es uno.
    let (c, ds) = ore(&dir, &["datasets", ".", "--json"]);
    assert_eq!(c, Some(0), "{ds}");
    assert!(
        !ds.contains("contratos"),
        "una colección no es un dataset: {ds}"
    );
    let (c, f) = ore(
        &dir,
        &["datasets", ".", "--ficha", "legal.archivo.contratos"],
    );
    assert_eq!(c, Some(65), "{f}");

    // Una que no está: 65, lo que ore-serve dice como 404.
    let (c, _) = ore(&dir, &["collections", ".", "--ficha", "legal.archivo.no"]);
    assert_eq!(c, Some(65));
    let (c, e) = ore(
        &dir,
        &[
            "collections",
            ".",
            "--items",
            "legal.archivo.contratos",
            "--estado",
            "otro",
        ],
    );
    assert_eq!(c, Some(64), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn el_indice_de_activos_lleva_su_puntero() {
    let dir = arbol("activos");
    let p = dir.join("datasets/legal/archivo/contratos.json");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, PUNTERO).unwrap();
    let (c, s) = ore(&dir, &["assets", ".", "--json"]);
    assert_eq!(c, Some(0), "{s}");
    let n = ore_core::parse::parse(s.lines().last().unwrap()).expect("el índice es JSON");
    let (_, items) = n.get("items").expect("items");
    let (_, item) = items
        .entries()
        .iter()
        .find(|(k, _)| {
            k.as_str()
                .is_some_and(|k| k.starts_with("collection:") && k.ends_with("contratos"))
        })
        .expect("la colección es un ítem");
    let (_, p) = item.get("puntero").expect("con puntero");
    let valor = |n: &ore_core::parse::Node, k: &str| {
        n.get(k)
            .and_then(|(_, v)| v.as_str().map(String::from))
            .unwrap_or_default()
    };
    assert_eq!(valor(p, "transaccion"), "1");
    assert_eq!(valor(p, "virtual"), "true");
    assert_eq!(valor(p.get("items").unwrap().1, "actuales"), "4");
    let _ = std::fs::remove_dir_all(&dir);
}
