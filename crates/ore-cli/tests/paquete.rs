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
    let dir = taller("nuevo-negativas");
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

// ── `ore package move` ──────────────────────────────────────────────────────

/// Un taller con dos paquetes: uno inducido de un catálogo real y otro vacío.
fn dos_paquetes(nombre: &str) -> PathBuf {
    let dir = taller(nombre);
    let cat = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/catalogos/bigquery-rubix-demo-ventas.json");
    std::fs::copy(&cat, dir.join("cat.json")).unwrap();
    ore(
        &dir,
        &["discover", "--from", "cat.json", "--out", "packages/ventas"],
    );
    let (_, d) = ore(&dir, &["package", "new", "eu", "--owner", "team:datos"]);
    assert!(
        dir.join("packages/ventas/tables").is_dir(),
        "el taller no tiene tablas:
{d}"
    );
    dir
}

fn codigos(dir: &Path) -> Vec<String> {
    let (_, dicho) = ore(dir, &["validate", "."]);
    dicho
        .lines()
        .filter_map(|l| l.strip_prefix("error[").and_then(|r| r.split(']').next()))
        .map(String::from)
        .collect()
}

/// **Las tres cosas a la vez**, sobre un árbol de verdad.
#[test]
fn mueve_el_fichero_el_nombre_y_lo_anuncia() {
    let dir = dos_paquetes("mueve");
    let antes = codigos(&dir);

    let (c, dicho) = ore(
        &dir,
        &[
            "package",
            "move",
            "ventas.rubix_demo_ventas_clientes",
            "--to",
            "eu",
        ],
    );
    assert_eq!(c, Some(0), "{dicho}");

    // ① el fichero, en el mismo subdirectorio del destino
    let nuevo = dir.join("packages/eu/tables/Clientes__rubix_demo_ventas_clientes.yaml");
    assert!(nuevo.is_file(), "no está en el destino:\n{dicho}");
    assert!(
        !dir.join("packages/ventas/tables/Clientes__rubix_demo_ventas_clientes.yaml")
            .exists(),
        "sigue en el origen"
    );

    // ② el `namespace`, que con `OOS2030` es una sola cosa con lo anterior
    let t = std::fs::read_to_string(&nuevo).unwrap();
    assert!(t.contains("namespace: eu"), "{t}");

    // ③ el anuncio en el manifiesto de ORIGEN
    let m = std::fs::read_to_string(dir.join("packages/ventas/package.yaml")).unwrap();
    assert!(
        m.contains("from: ventas.rubix_demo_ventas_clientes")
            && m.contains("to: eu.rubix_demo_ventas_clientes"),
        "{m}"
    );

    // ④ y lo que lo nombraba, reapuntado
    let v = std::fs::read_to_string(
        dir.join("packages/ventas/views/Clientes__rubix_demo_ventas_clientes.yaml"),
    )
    .unwrap();
    assert!(v.contains("table: eu.rubix_demo_ventas_clientes"), "{v}");

    // Y lo que el movimiento deja: EXACTAMENTE el `OOS2028` que el mando
    // anunció, porque `exports` no lo decide él.
    let despues = codigos(&dir);
    // Lo que sobra respecto de antes, contando repeticiones.
    let mut nuevos = despues.clone();
    for c in &antes {
        if let Some(i) = nuevos.iter().position(|x| x == c) {
            nuevos.remove(i);
        }
    }
    assert_eq!(
        nuevos,
        vec!["OOS2028".to_string()],
        "{despues:?} vs {antes:?}"
    );
    assert!(dicho.contains("cruzan a `eu`"), "y lo dice:\n{dicho}");
    assert!(dicho.contains("exports"), "{dicho}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Y añadiendo el `exports` que el mando dijo, el árbol queda como estaba.
#[test]
fn con_el_export_que_dice_el_arbol_queda_igual() {
    let dir = dos_paquetes("export");
    let antes = codigos(&dir);
    ore(
        &dir,
        &[
            "package",
            "move",
            "ventas.rubix_demo_ventas_clientes",
            "--to",
            "eu",
        ],
    );
    let m = dir.join("packages/eu/package.yaml");
    let t = std::fs::read_to_string(&m).unwrap().replace(
        "spec: { owner: \"team:datos\" }",
        "spec: { owner: \"team:datos\", exports: [eu.rubix_demo_ventas_clientes] }",
    );
    std::fs::write(&m, t).unwrap();

    assert_eq!(codigos(&dir), antes, "el movimiento no deja nada suyo");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Lo que se niega, y **sin tocar un fichero**: o se hacen los tres pasos o
/// ninguno.
#[test]
fn las_negativas_no_mueven_nada() {
    let dir = dos_paquetes("mover-negativas");
    let sitio = dir.join("packages/ventas/tables/Clientes__rubix_demo_ventas_clientes.yaml");
    let original = std::fs::read_to_string(&sitio).unwrap();

    for (args, porque) in [
        (
            vec!["package", "move", "ventas.no_existe", "--to", "eu"],
            "no hay ningún documento",
        ),
        (
            vec![
                "package",
                "move",
                "ventas.rubix_demo_ventas_clientes",
                "--to",
                "no_existe",
            ],
            "no hay ningún paquete",
        ),
        (
            vec![
                "package",
                "move",
                "ventas.rubix_demo_ventas_clientes",
                "--to",
                "ventas",
            ],
            "ya está en",
        ),
    ] {
        let (c, dicho) = ore(&dir, &args);
        assert_eq!(c, Some(65), "{args:?}:\n{dicho}");
        assert!(dicho.contains(porque), "{dicho}");
    }

    assert_eq!(
        std::fs::read_to_string(&sitio).unwrap(),
        original,
        "una negativa movió algo"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ── `ore package split` ─────────────────────────────────────────────────────

/// **Sin `--to` no mueve nada**, y lo que enumera es lo que la medida encontró:
/// un paquete recién descubierto son 30 documentos en **10 componentes de 3**
/// —`Table` + `View` + `Entity` por objeto—.
#[test]
fn enumera_las_componentes_y_no_toca_nada() {
    let dir = dos_paquetes("enumera");
    let antes = codigos(&dir);

    let (c, dicho) = ore(&dir, &["package", "split", "ventas"]);
    assert_eq!(c, Some(0), "{dicho}");
    assert!(
        dicho.contains("30 documento(s), 10 componente(s)"),
        "{dicho}"
    );
    assert!(dicho.contains("── 1 · 3 documento(s)"), "{dicho}");
    assert!(
        dicho.contains("sale entera SIN una sola referencia cruzando"),
        "{dicho}"
    );
    assert_eq!(codigos(&dir), antes, "enumerar movió algo");
    let _ = std::fs::remove_dir_all(&dir);
}

/// **El aserto que decide el verbo**: mover una componente entera deja el árbol
/// EXACTAMENTE como estaba. Cero referencias cruzando, cero diagnósticos nuevos.
///
/// Es lo que una persona no calcula bien: moviendo la tabla sola quedaría un
/// `OOS2028`, y la componente son tres documentos que hay que ver a la vez.
#[test]
fn una_componente_entera_sale_a_cero() {
    let dir = dos_paquetes("componente");
    let antes = codigos(&dir);

    let (c, dicho) = ore(
        &dir,
        &[
            "package",
            "split",
            "ventas",
            "--to",
            "eu",
            "--con",
            "ventas.Clientes",
            "--con",
            "ventas.clientes",
            "--con",
            "ventas.rubix_demo_ventas_clientes",
        ],
    );
    assert_eq!(c, Some(0), "{dicho}");
    assert!(dicho.contains("3 documento(s) → `eu`"), "{dicho}");
    assert!(dicho.contains("el corte sale a cero"), "{dicho}");
    assert_eq!(codigos(&dir), antes, "el corte dejó algo suyo:\n{dicho}");

    // Y los tres están donde tienen que estar, con su espacio de nombres nuevo.
    for f in [
        "packages/eu/entities/Clientes.yaml",
        "packages/eu/views/Clientes__rubix_demo_ventas_clientes.yaml",
        "packages/eu/tables/Clientes__rubix_demo_ventas_clientes.yaml",
    ] {
        let t = std::fs::read_to_string(dir.join(f)).unwrap_or_else(|_| panic!("falta {f}"));
        assert!(t.contains("namespace: eu"), "{f}:\n{t}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Y un corte PARCIAL cuesta, y dice **qué falta para que salga a cero** — que
/// es el aviso por el que este verbo existe.
#[test]
fn un_corte_parcial_dice_lo_que_cuesta_y_lo_que_falta() {
    let dir = dos_paquetes("parcial");
    let (c, dicho) = ore(
        &dir,
        &[
            "package",
            "split",
            "ventas",
            "--to",
            "eu",
            "--con",
            "ventas.rubix_demo_ventas_mov_bak",
        ],
    );
    assert_eq!(c, Some(0), "{dicho}");
    assert!(dicho.contains("1 referencia(s) cruzan"), "{dicho}");
    assert!(
        dicho.contains("no es una componente entera") && dicho.contains("Con 2 más"),
        "tiene que decir qué arrastra:\n{dicho}"
    );
    assert!(dicho.contains("ventas.Mov_bak"), "{dicho}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Un paquete **modelado** es una sola componente: no hay corte gratis, y esto
/// lo dice en vez de fingir que sí.
#[test]
fn un_paquete_conectado_dice_que_no_hay_corte_gratis() {
    let dir = std::env::temp_dir().join(format!("ore-split-conectado-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let origen =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/oos/examples/acme-retail");
    copiar(&origen, &dir);

    let (c, dicho) = ore(&dir, &["package", "split", "hr"]);
    assert_eq!(c, Some(0), "{dicho}");
    assert!(dicho.contains("1 componente(s)"), "{dicho}");
    assert!(dicho.contains("no hay corte gratis"), "{dicho}");
    let _ = std::fs::remove_dir_all(&dir);
}

fn copiar(de: &Path, a: &Path) {
    std::fs::create_dir_all(a).unwrap();
    for e in std::fs::read_dir(de).unwrap().flatten() {
        let d = a.join(e.file_name());
        if e.path().is_dir() {
            copiar(&e.path(), &d);
        } else {
            std::fs::copy(e.path(), d).unwrap();
        }
    }
}

/// Con `--to` y sin `--con` **no se parte por la mitad**: cuál es el corte no lo
/// decide este mando.
#[test]
fn con_destino_y_sin_corte_se_niega() {
    let dir = dos_paquetes("sin-corte");
    let (c, dicho) = ore(&dir, &["package", "split", "ventas", "--to", "eu"]);
    assert_eq!(c, Some(64), "{dicho}");
    assert!(dicho.contains("sería inventar un límite"), "{dicho}");
    let _ = std::fs::remove_dir_all(&dir);
}

// ── `ore package merge` ─────────────────────────────────────────────────────

/// **El viaje de ida y vuelta**: partir y volver a fundir deja el árbol como
/// estaba, y el origen queda como LÁPIDA en vez de desaparecer.
///
/// Que la lápida valga se midió con `ore diff` y su control: con el anuncio sale
/// `changes: []`; sin él, `OOS5007` y salto mayor.
#[test]
fn fundir_deshace_el_corte_y_deja_una_lapida() {
    let dir = dos_paquetes("fundir");
    let antes = codigos(&dir);
    ore(
        &dir,
        &[
            "package",
            "split",
            "ventas",
            "--to",
            "eu",
            "--con",
            "ventas.Clientes",
            "--con",
            "ventas.clientes",
            "--con",
            "ventas.rubix_demo_ventas_clientes",
        ],
    );

    let (c, dicho) = ore(&dir, &["package", "merge", "eu", "--into", "ventas"]);
    assert_eq!(c, Some(0), "{dicho}");
    assert!(dicho.contains("3 documento(s)"), "{dicho}");
    assert!(dicho.contains("LÁPIDA"), "{dicho}");
    assert_eq!(codigos(&dir), antes, "el ida y vuelta dejó algo:\n{dicho}");

    // La lápida: retirada, sin documentos, y con un `moved` por cada uno.
    let m = std::fs::read_to_string(dir.join("packages/eu/package.yaml")).unwrap();
    assert!(m.contains("status: retired"), "{m}");
    assert_eq!(m.matches("from: eu.").count(), 3, "{m}");
    assert!(
        !dir.join("packages/eu/entities").exists()
            || std::fs::read_dir(dir.join("packages/eu/entities"))
                .map(|d| d.flatten().count())
                .unwrap_or(0)
                == 0,
        "quedó contenido en el paquete fundido"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Las colisiones **se niegan y no se resuelven**: perder un documento no lo
/// decide una herramienta. Es la misma elección que `move` al no sobrescribir.
#[test]
fn las_colisiones_se_niegan_sin_mover_nada() {
    let dir = taller("choque");
    for p in ["a", "b"] {
        ore(&dir, &["package", "new", p, "--owner", "team:datos"]);
        let e = dir.join(format!("packages/{p}/entities"));
        std::fs::create_dir_all(&e).unwrap();
        std::fs::write(
            e.join("C.yaml"),
            format!(
                "apiVersion: oos.dev/v1alpha8\nkind: Entity\n\
                 metadata: {{ name: C, namespace: {p} }}\nspec:\n  nature: entity\n  \
                 primaryKey: [id]\n  properties:\n    id: {{ type: String }}\n"
            ),
        )
        .unwrap();
    }

    let (c, dicho) = ore(&dir, &["package", "merge", "b", "--into", "a"]);
    assert_eq!(c, Some(65), "{dicho}");
    assert!(dicho.contains("existen en los dos paquetes"), "{dicho}");
    assert!(
        dir.join("packages/b/entities/C.yaml").is_file(),
        "una negativa movió algo"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// **`OOS2031`**: la lápida le vale a `diff` —que compara dos versiones del
/// mismo paquete— y no le decía nada a quien la importa desde fuera.
#[test]
fn depender_de_una_lapida_se_dice() {
    let dir = taller("lapida");
    ore(&dir, &["package", "new", "a", "--owner", "team:datos"]);
    ore(&dir, &["package", "new", "b", "--owner", "team:datos"]);

    // `b` retirado, con su `moved` hacia `a`.
    let m = dir.join("packages/b/package.yaml");
    let t = std::fs::read_to_string(&m)
        .unwrap()
        .replace("status: draft", "status: retired");
    std::fs::write(
        &m,
        t.replace(
            "spec: { owner: \"team:datos\" }",
            "spec: { owner: \"team:datos\", moved: [{ from: b.C, to: a.C, since: 0.1.0 }] }",
        ),
    )
    .unwrap();

    // Y `a` dependiendo de él.
    let ma = dir.join("packages/a/package.yaml");
    let t = std::fs::read_to_string(&ma).unwrap().replace(
        "spec: { owner: \"team:datos\" }",
        "spec: { owner: \"team:datos\", dependencies: [{ package: b, version: \"^0.1\" }] }",
    );
    std::fs::write(&ma, t).unwrap();

    let (_, dicho) = ore(&dir, &["validate", "."]);
    assert!(dicho.contains("OOS2031"), "{dicho}");
    // Y la ayuda dice A DÓNDE se fue, que es la mitad del valor.
    assert!(
        dicho.contains("`a`"),
        "tiene que nombrar el destino:\n{dicho}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
