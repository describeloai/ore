//! **Lo que una base tiene de un bucket** (ADR 0046 E5).
//!
//! El catálogo es el de F1, leído del bucket de verdad por `ore-read-s3`
//! (`tests/catalogos/s3-f1.json`): doce tablas de ficheros y cinco conjuntos
//! de objetos. De ahí, `discover` con la fuente aparte:
//!
//! - la fuente escribe los punteros —la `Table` de ficheros con dónde está y
//!   cómo se lee (`object` + `format`, v1alpha16), y el `ObjectTable` de cada
//!   conjunto—, una vez y exportados;
//! - **la clase de la base decide la colección**: la estándar copia todo lo
//!   que entra, así que su `MediaCollection` es mantenida; la foránea es un
//!   espejo, así que la suya es `virtual`;
//! - un contenedor (`archive`) no es una colección: su puntero sí, y se dice.

use std::path::{Path, PathBuf};
use std::process::Command;

const FUENTE: &str = "s3_ventas";

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

const ELEGIDOS: [&str; 4] = [
    "nueva_carpeta.pedidos",
    "nueva_carpeta.contratos",
    "nueva_carpeta.fotos",
    "nueva_carpeta.nueva_carpeta_zip",
];

/// El paquete de la fuente como lo deja el Job de catálogo, el conducto de la
/// copia como lo deja ore-serve al crear una estándar, y dos bases que eligen
/// lo mismo: una foránea y una estándar.
fn arbol(nombre: &str) -> (PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("ore-bucket-{}-{nombre}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    ore(&dir, &["init", ".", "--name", "demo"]);
    let manifiesto = dir.join("ontology.config.yaml");
    let mut m = std::fs::read_to_string(&manifiesto).unwrap();
    m.push_str(&format!(
        "\ndatasources:\n  - {{ name: {FUENTE}, type: s3, connectionEnv: S3_VENTAS_URL }}\n"
    ));
    std::fs::write(&manifiesto, m).unwrap();
    std::fs::write(
        dir.join("conduits.yaml"),
        "apiVersion: oos.dev/v1alpha1\nkind: ConduitPolicy\nmetadata: { name: demo }\nspec:\n  owner: team:demo\n  conduits:\n    materialization.payload: { oos.maturity: DRAFT }\n",
    )
    .unwrap();
    let cat = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/catalogos/s3-f1.json"),
    )
    .unwrap()
    .replace("\r\n", "\n");
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
    let mut todo = String::new();
    for (base, tipo) in [("fdb", "foreign"), ("sdb", "standard")] {
        let out = format!("packages/{base}");
        let mut args = vec!["discover", "--from", "cat.json", "--out", &out];
        for o in ELEGIDOS {
            args.extend(["--only", o]);
        }
        args.extend(["--owner", "team:demo", "--no-model", "--type", tipo]);
        let (c, dicho) = ore(&dir, &args);
        assert_eq!(c, Some(0), "{base}: {dicho}");
        todo.push_str(&dicho);
    }
    (dir, todo)
}

fn leer(dir: &Path, rel: &str) -> String {
    std::fs::read_to_string(dir.join("packages").join(rel))
        .unwrap_or_else(|e| panic!("`{rel}`: {e}"))
}

#[test]
fn la_estandar_copia_sus_colecciones_y_la_foranea_las_sirve_en_sitio() {
    let (dir, dicho) = arbol("clases");

    // La fuente: la tabla de ficheros dice dónde está y cómo se lee.
    let t = leer(&dir, "s3_ventas/nueva_carpeta/tables/pedidos.yaml");
    for s in [
        "apiVersion: oos.dev/v1alpha16",
        "object: \"Nueva carpeta/ventas/pedidos/\"",
        "format:\n    type: parquet\n    match: \"**/*.parquet\"\n    partitions: [fecha]",
        "witness: listing",
    ] {
        assert!(t.contains(s), "sin `{s}`:\n{t}");
    }
    // Y el puntero de cada conjunto, también el del zip.
    let o = leer(&dir, "s3_ventas/nueva_carpeta/objects/contratos.yaml");
    for s in [
        "kind: ObjectTable",
        "metadata: { name: contratos, namespace: s3_ventas, schema: nueva_carpeta }",
        "prefix: \"Nueva carpeta/contratos/\"",
        "media: document",
    ] {
        assert!(o.contains(s), "sin `{s}`:\n{o}");
    }
    let z = leer(
        &dir,
        "s3_ventas/nueva_carpeta/objects/nueva_carpeta_zip.yaml",
    );
    assert!(
        z.contains("match: \"*.zip\"") && z.contains("media: archive"),
        "{z}"
    );
    let m = leer(&dir, "s3_ventas/package.yaml");
    // ⭐ 0046 E5′: todo lo catalogado —12 tablas y 5 conjuntos— tiene su
    //   puntero, lo elija una base o no.
    for e in [
        "s3_ventas.nueva_carpeta.contratos",
        "s3_ventas.nueva_carpeta.nueva_carpeta_zip",
        "s3_ventas.nueva_carpeta.olist_orders_dataset",
        "s3_ventas.raiz_txt",
    ] {
        assert!(m.contains(e), "sin `{e}`: {m}");
    }
    let cuantos = |kind: &str| {
        walk(&dir.join("packages/s3_ventas"))
            .iter()
            .filter(|p| {
                p.parent()
                    .and_then(|d| d.file_name())
                    .is_some_and(|n| n == kind)
            })
            .count()
    };
    assert_eq!((cuantos("tables"), cuantos("objects")), (12, 5));

    // La foránea sirve en sitio; la estándar copia. Las dos, del mismo puntero.
    let f = leer(&dir, "fdb/nueva_carpeta/collections/fotos.yaml");
    assert!(
        f.contains("virtual: true")
            && f.contains("formats: [jpg, png]")
            && f.contains("from: { objectTable: s3_ventas.nueva_carpeta.fotos }"),
        "{f}"
    );
    let s = leer(&dir, "sdb/nueva_carpeta/collections/fotos.yaml");
    assert!(
        !s.contains("virtual")
            && s.contains("from: { objectTable: s3_ventas.nueva_carpeta.fotos }"),
        "{s}"
    );
    // El zip no es una colección en ninguna, y se dice.
    for base in ["fdb", "sdb"] {
        assert!(
            !dir.join(format!(
                "packages/{base}/nueva_carpeta/collections/nueva_carpeta_zip.yaml"
            ))
            .exists(),
            "{base}"
        );
        assert!(
            !dir.join(format!("packages/{base}/nueva_carpeta/objects"))
                .exists()
        );
    }
    assert!(
        dicho.contains("`nueva_carpeta.nueva_carpeta_zip` no es una colección"),
        "{dicho}"
    );

    let (c, v) = ore(&dir, &["validate", "."]);
    assert_eq!(c, Some(0), "{v}");
    assert!(!v.contains("error["), "{v}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// ⭐ 0046 E5′ · Un conjunto se va de la fuente cuando se va del origen, no
/// cuando nadie lo elige: sin bases, los 17 punteros siguen; sin `contratos`
/// en el catálogo, solo su `ObjectTable` se retira.
#[test]
fn un_conjunto_que_sale_del_origen_se_retira_de_la_fuente() {
    let (dir, _) = arbol("retira");
    std::fs::remove_dir_all(dir.join("packages/fdb")).unwrap();
    std::fs::remove_dir_all(dir.join("packages/sdb")).unwrap();
    let (c, dicho) = ore(&dir, &["source", "induce", FUENTE]);
    assert_eq!(c, Some(0), "{dicho}");
    assert!(
        !dicho.contains("retirado"),
        "sin bases no se retira nada: {dicho}"
    );

    let ruta = dir.join("packages/s3_ventas/discover.catalog.json");
    let cat = std::fs::read_to_string(&ruta).unwrap();
    std::fs::write(&ruta, sin_entrada(&cat, "nueva_carpeta.contratos")).unwrap();
    let (c, dicho) = ore(&dir, &["source", "induce", FUENTE]);
    assert_eq!(c, Some(0), "{dicho}");
    assert!(
        dicho.contains("1 retirado(s), ya no están en el origen")
            && dicho.contains("nueva_carpeta/objects/contratos.yaml"),
        "{dicho}"
    );
    assert!(
        dir.join("packages/s3_ventas/nueva_carpeta/objects/fotos.yaml")
            .exists()
    );
    let (_, v) = ore(&dir, &["validate", "."]);
    assert!(!v.contains("error["), "{v}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// El catálogo sin la entrada `nombre` (un objeto de `tables` u `objects`,
/// sangrado a cuatro espacios como lo escribe el driver).
fn sin_entrada(cat: &str, nombre: &str) -> String {
    let i = cat.find(&format!("\"name\": \"{nombre}\"")).unwrap();
    let ini = cat[..i]
        .rfind(
            "
    {",
        )
        .unwrap();
    let fin = cat[i..]
        .find(
            "
    }",
        )
        .unwrap()
        + i
        + "
    }"
        .len();
    let resto = &cat[fin..];
    match resto.strip_prefix(',') {
        Some(r) => format!("{}{r}", &cat[..ini]),
        // Era la última: la coma sobrante es la de antes.
        None => format!("{}{resto}", cat[..ini].trim_end_matches(',')),
    }
}

fn walk(d: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut pila = vec![d.to_path_buf()];
    while let Some(d) = pila.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                pila.push(p);
            } else if p.extension().is_some_and(|x| x == "yaml") {
                out.push(p);
            }
        }
    }
    out
}

/// Una errata en `--only` se dice con lo que el bucket tiene, conjuntos
/// incluidos: son tan elegibles como las tablas.
#[test]
fn una_errata_se_dice_con_los_conjuntos_del_bucket() {
    let dir = std::env::temp_dir().join(format!("ore-bucket-{}-errata", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let cat = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/catalogos/s3-f1.json");
    let (c, dicho) = ore(
        &dir,
        &[
            "discover",
            "--from",
            cat.to_str().unwrap(),
            "--out",
            "packages/x",
            "--only",
            "nueva_carpeta.contrato",
            "--owner",
            "team:demo",
            "--no-model",
            "--type",
            "foreign",
        ],
    );
    assert_eq!(c, Some(65), "{dicho}");
    assert!(
        dicho.contains("`nueva_carpeta.contrato`") && dicho.contains("El catálogo trae 17 objetos"),
        "{dicho}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
