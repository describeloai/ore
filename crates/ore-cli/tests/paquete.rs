//! `ore package new`, y lo que ata con `OOS2030`.
//!
//! Hasta ahora **un paquete solo nacía descubriendo una fuente**: `ore init`
//! deja `packages/` vacío y el único que escribía un `package.yaml` era el
//! inductor. Sin destino no hay a dónde mover nada, así que este verbo va antes
//! que `ore package move`.
//!
//! La prueba que de verdad importa es la última: lo que este mando crea tiene
//! que poder **contener** contenido gobernado. Un mando que creara un paquete
//! donde `OOS2030` no se puede satisfacer sería peor que no tenerlo.

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

fn taller(nombre: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ore-paquete-{}-{nombre}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (_, dicho) = ore(&dir, &["init", ".", "--name", "demo"]);
    assert!(
        dir.join("ontology.config.yaml").is_file(),
        "el taller no arrancó:\n{dicho}"
    );
    dir
}

#[test]
fn crea_el_manifiesto_y_nada_mas() {
    let dir = taller("crea");
    let (c, dicho) = ore(&dir, &["package", "new", "ventas", "--owner", "team:datos"]);
    assert_eq!(c, Some(0), "{dicho}");

    let m = dir.join("packages/ventas/package.yaml");
    let t = std::fs::read_to_string(&m).expect("no escribió el manifiesto");
    assert!(t.contains("name: ventas"), "{t}");
    assert!(t.contains("owner: \"team:datos\""), "{t}");
    // `draft` y no `active`: de `status` sale la madurez por defecto de lo que
    // el paquete contenga, y uno recién creado no contiene nada.
    assert!(t.contains("status: draft"), "{t}");
    // `domain` es obligatorio en el esquema, así que no se puede omitir; el
    // nombre es la conjetura más pequeña.
    assert!(t.contains("domain: ventas"), "{t}");

    // Y nada más: un directorio vacío no viaja en git y no significa nada.
    let dentro: Vec<String> = std::fs::read_dir(dir.join("packages/ventas"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(dentro, vec!["package.yaml".to_string()], "{dentro:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// El emisor es el del inductor, así que un manifiesto escrito a mano y uno
/// inducido son el mismo texto salvo en lo que cada uno DECIDE.
#[test]
fn el_manifiesto_lo_escribe_el_mismo_emisor_que_la_induccion() {
    let dir = taller("emisor");
    ore(&dir, &["package", "new", "ventas", "--owner", "team:datos"]);
    let a_mano = std::fs::read_to_string(dir.join("packages/ventas/package.yaml")).unwrap();

    // Lo que la inducción escribe, por la misma puerta.
    std::fs::write(
        dir.join("cat.json"),
        r#"{"source":"f","tables":[{"name":"t","columns":[{"name":"id","type":"String"}]}]}"#,
    )
    .unwrap();
    ore(
        &dir,
        &["discover", "--from", "cat.json", "--out", "packages/otro"],
    );
    let inducido = std::fs::read_to_string(dir.join("packages/otro/package.yaml")).unwrap();

    // Misma forma, línea a línea; lo único que cambia es lo que cada uno decide.
    assert_eq!(a_mano.lines().count(), inducido.lines().count(), "{a_mano}");
    assert!(a_mano.contains("status: draft") && inducido.contains("status: active"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn no_sobrescribe_y_no_acepta_un_nombre_imposible() {
    let dir = taller("negativas");
    ore(&dir, &["package", "new", "ventas", "--owner", "team:datos"]);

    let (c, dicho) = ore(&dir, &["package", "new", "ventas", "--owner", "team:otro"]);
    assert_eq!(c, Some(65), "{dicho}");
    assert!(dicho.contains("ya existe"), "{dicho}");

    // Legal como nombre de paquete —es la coordenada con la que otro lo
    // importa— e imposible como `namespace`. Es el hueco entre los dos
    // vocabularios del esquema, y salió construyendo esto.
    for malo in ["mi-paquete", "oos.dev", "2ventas"] {
        let (c, dicho) = ore(&dir, &["package", "new", malo, "--owner", "team:d"]);
        assert_eq!(c, Some(64), "`{malo}`:\n{dicho}");
        assert!(dicho.contains("espacio de nombres"), "{dicho}");
        assert!(
            !dir.join("packages").join(malo).exists(),
            "se creó igualmente"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Sin dueño escribe `cambiame`, **que no valida**, y lo dice en vez de
/// inventar un handle. Es la misma figura que el inductor.
#[test]
fn sin_dueno_lo_dice_y_el_arbol_lo_cobra() {
    let dir = taller("dueno");
    let (c, dicho) = ore(&dir, &["package", "new", "ventas"]);
    assert_eq!(c, Some(0), "{dicho}");
    assert!(dicho.contains("NO valida"), "{dicho}");
    assert!(
        dicho.contains("OOS2009"),
        "el árbol tiene que cobrarlo:\n{dicho}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// **La que ata el paso 1 con el paso 2.**
///
/// Lo que este mando crea tiene que poder CONTENER contenido gobernado: un
/// documento con el `namespace` del paquete compila, y uno con otro no. Si el
/// mando creara paquetes donde `OOS2030` no se puede satisfacer, sería peor que
/// no tenerlo.
#[test]
fn lo_que_crea_puede_contener_contenido_gobernado() {
    let dir = taller("contiene");
    ore(&dir, &["package", "new", "ventas", "--owner", "team:datos"]);
    let tablas = dir.join("packages/ventas/tables");
    std::fs::create_dir_all(&tablas).unwrap();

    let tabla = |ns: &str| {
        format!(
            "apiVersion: oos.dev/v1alpha8\nkind: Table\n\
             metadata: {{ name: t, namespace: {ns} }}\nspec:\n  \
             datasource: d\n  object: \"public.t\"\n  columns:\n    id: {{}}\n  \
             reads: {{ predicatePushdown: [eq], fullScan: cheap }}\n  \
             changes: {{ mode: none, witness: none }}\n"
        )
    };

    std::fs::write(tablas.join("t.yaml"), tabla("ventas")).unwrap();
    let (_, dicho) = ore(&dir, &["validate", "."]);
    assert!(
        !dicho.contains("OOS2030"),
        "un documento del paquete no puede dar OOS2030:\n{dicho}"
    );

    std::fs::write(tablas.join("t.yaml"), tabla("otro")).unwrap();
    let (_, dicho) = ore(&dir, &["validate", "."]);
    assert!(dicho.contains("OOS2030"), "y uno de fuera sí:\n{dicho}");
    let _ = std::fs::remove_dir_all(&dir);
}
