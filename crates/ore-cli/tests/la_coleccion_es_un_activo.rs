//! 0046 E8·1d · **La colección es un activo de primera clase**, como el
//! dataset: `ore view` dice su raíz —lo que el Job de la copia busca para
//! abrir su fuente—, `ore collections` la lista con su forma y su puntero,
//! `ore datasets` no la cuenta como dataset aunque su puntero viva junto a los
//! suyos, y el índice de activos lleva su puntero.
//!
//! El árbol es el caso de conformidad `v1alpha16/valid/a-virtual-collection`:
//! la colección virtual `legal.archivo.contratos` sobre el `ObjectTable`
//! `s3_ventas.docs.contratos`. Sus punteros, en el catálogo de la prueba.
#![cfg(unix)]

#[path = "comun/catalogo.rs"]
mod catalogo;

use std::path::{Path, PathBuf};
use std::process::Command;

use catalogo::Catalogo;

const CONTRATOS: &str = "legal.archivo.contratos";

fn ore(cat: &Catalogo, dir: &Path, args: &[&str]) -> (Option<i32>, String) {
    let s = cat
        .en(Command::new(env!("CARGO_BIN_EXE_ore"))
            .args(args)
            .current_dir(dir)
            .env_remove("ORE_REF"))
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
    let cat = Catalogo::nuevo("coleccion-view");
    let (codigo, salida) = ore(&cat, &dir, &["view", "."]);
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
    let cat = Catalogo::nuevo("coleccion-lista");
    let (c, sin) = ore(&cat, &dir, &["collections", ".", "--json"]);
    assert_eq!(c, Some(0), "{sin}");
    assert!(sin.contains(r#""forma":"virtual""#), "{sin}");
    assert!(
        sin.contains(r#""puntero":null"#),
        "sin transacción todavía: {sin}"
    );

    cat.poner(CONTRATOS, PUNTERO, None);
    let (_, con) = ore(&cat, &dir, &["collections", ".", "--json"]);
    assert!(con.contains(r#""transaccion":1"#), "{con}");
    assert!(con.contains(r#""actuales":4"#), "{con}");
    assert!(
        con.contains(r#""objectTable":"s3_ventas.docs.contratos""#),
        "{con}"
    );

    // Vive con los punteros de los datasets, y no es uno.
    let (c, ds) = ore(&cat, &dir, &["datasets", ".", "--json"]);
    assert_eq!(c, Some(0), "{ds}");
    assert!(
        !ds.contains("contratos"),
        "una colección no es un dataset: {ds}"
    );
    let (c, f) = ore(&cat, &dir, &["datasets", ".", "--ficha", CONTRATOS]);
    assert_eq!(c, Some(65), "{f}");

    // Una que no está: 65, lo que ore-serve dice como 404.
    let (c, _) = ore(
        &cat,
        &dir,
        &["collections", ".", "--ficha", "legal.archivo.no"],
    );
    assert_eq!(c, Some(65));
    let (c, e) = ore(
        &cat,
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

/// 0046 E8·3c · pasado el techo, la colección lo dice donde se mira.
#[test]
fn el_techo_se_ve_en_la_lista() {
    let dir = arbol("techo");
    let cat = Catalogo::nuevo("coleccion-techo");
    let con_techo = PUNTERO.replacen(
        "\"transaccion\": 1,",
        "\"transaccion\": 1,\n  \"techo\": { \"filas\": 600000, \"limite\": 500000, \"aviso\": \"600000 filas en el manifiesto: toca merge-on-read\" },",
        1,
    );
    cat.poner(CONTRATOS, &con_techo, None);
    let (c, s) = ore(&cat, &dir, &["collections", "."]);
    assert_eq!(c, Some(0), "{s}");
    assert!(s.contains("⚠ techo · 600000 filas"), "{s}");
    let (_, j) = ore(&cat, &dir, &["collections", ".", "--json"]);
    assert!(j.contains(r#""limite":500000"#), "{j}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn el_indice_de_activos_lleva_su_puntero() {
    let dir = arbol("activos");
    let cat = Catalogo::nuevo("coleccion-activos");
    cat.poner(CONTRATOS, PUNTERO, None);
    let (c, s) = ore(&cat, &dir, &["assets", ".", "--json"]);
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

/// La pasada de `main` recorre todas las colecciones, y el Job abre sólo las
/// fuentes de lo que se le pide: una colección con su copia y su fuente sin
/// abrir se queda como estaba —no pasa a `error`—. Pedida con `--vista`, la
/// falta de credencial sí es su fallo.
#[test]
fn una_coleccion_cuya_fuente_no_se_abrio_se_queda_como_estaba() {
    let dir = arbol("sin-abrir");
    let cat = Catalogo::nuevo("coleccion-sin-abrir");
    cat.poner(CONTRATOS, PUNTERO, None);
    let antes = cat.leer(CONTRATOS, None);
    let materializar = |extra: &[&str]| {
        let s = cat
            .en(Command::new(env!("CARGO_BIN_EXE_ore"))
                .args(["materialize", "."])
                .args(extra)
                .current_dir(&dir)
                .env_remove("S3_VENTAS_URL")
                .env_remove("ORE_REF"))
            .output()
            .expect("no se pudo invocar `ore`");
        format!(
            "{}{}",
            String::from_utf8_lossy(&s.stdout),
            String::from_utf8_lossy(&s.stderr)
        )
    };
    let salida = materializar(&[]);
    assert!(
        salida.contains("la fuente `s3_ventas` no se abrió en esta pasada: se queda como estaba"),
        "{salida}"
    );
    assert_eq!(cat.leer(CONTRATOS, None), antes, "{salida}");
    let salida = materializar(&["--vista", CONTRATOS]);
    let puntero = cat.leer(CONTRATOS, None).unwrap_or_default();
    assert_eq!(
        cat.campo(CONTRATOS, "estado").as_deref(),
        Some("error"),
        "{salida}\n{puntero}"
    );
    // y lo que tenía sigue: un fallo no borra la copia
    assert_eq!(
        cat.campo(CONTRATOS, "transaccion").as_deref(),
        Some("1"),
        "{puntero}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
