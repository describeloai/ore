//! ⛔ **LA PUERTA DE LOS PUNTEROS, POR FUERZA MAYOR** (0079 §19.1, E1a·5b·4).
//!
//! Un puntero se lee y se escribe por `ore_core::punteros::Punteros`, y nada más
//! sabe dónde vive: así el corte (E1a·5) cambia la implementación y no a quien
//! llama. Esta guarda recorre el código de todas las crates y falla si algo
//! fuera de la puerta toca los punteros como ficheros:
//!
//! - `punteros::crudo` (las rutas, las carpetas, leer y escribir el fichero);
//! - `.join("datasets")` o `.join("copias")`: la carpeta a mano.
//!
//! Lo que sigue tocándolos lleva su razón en [`PERMITIDOS`]; son el mundo de git
//! (la superposición de una rama, sus fusiones, las migraciones) y mueren con el
//! corte del último inquilino. Una línea que nombra `datasets` y no es un
//! puntero (la carpeta de los documentos) lo dice: `// no es un puntero`.
//!
//! Lo de las pruebas (`#[cfg(test)] mod …` y `tests/`) no cuenta: monta árboles
//! a mano a propósito.

use std::path::{Path, PathBuf};

/// Quién puede, y por qué. Una entrada que ya no hace falta también falla: la
/// lista no se pudre.
const PERMITIDOS: &[(&str, &str)] = &[
    (
        "ore-catalogo/src/git.rs",
        "el andamio de la mudanza lee los punteros de git (sus carpetas)",
    ),
    (
        "ore-catalogo/src/puerta.rs",
        "`Catalogo`: el catálogo guarda cada puntero por su ruta en el árbol; la puerta resuelve el nombre en sus rutas, y la siembra lee el directorio",
    ),
    (
        "ore-cli/src/al_dia.rs",
        "`ore overlay`: la superposición de una rama en el clon de un Job (mundo git)",
    ),
    (
        "ore-cli/src/migrar.rs",
        "`ore migrate v1alpha12`: mueve los ficheros de punteros del árbol",
    ),
    (
        "ore-cli/src/migrar_v14.rs",
        "`ore migrate v1alpha14`: renombra ficheros de punteros del árbol",
    ),
    (
        "ore-serve/src/git.rs",
        "la superposición de una rama y la fusión de punteros en git",
    ),
    (
        "ore-serve/src/arbol.rs",
        "el editor: cierra `datasets/` y `copias/` a mano (`ruta_editable`)",
    ),
    (
        "ore-serve/src/cambios.rs",
        "los anexos de una propuesta: el puntero de tres árboles, por su ruta",
    ),
    (
        "ore-serve/src/datasets.rs",
        "`de_main` del listado en una rama: sin volver a leer cada puntero (F7b·1)",
    ),
];

const VETADOS: &[&str] = &["crudo::", ".join(\"datasets\")", ".join(\"copias\")"];
const NO_ES_UN_PUNTERO: &str = "// no es un puntero";

fn crates() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(es) = std::fs::read_dir(dir) else {
        return;
    };
    for e in es.flatten() {
        let p = e.path();
        if p.is_dir() {
            rs(&p, out);
        } else if p.extension().and_then(|x| x.to_str()) == Some("rs") {
            out.push(p);
        }
    }
}

/// Las líneas de `texto` que tocan un puntero como fichero, sin las pruebas
/// (desde un `#[cfg(test)]` seguido de `mod`) ni los comentarios.
fn vetadas(texto: &str) -> Vec<(usize, String)> {
    let lineas: Vec<&str> = texto.lines().collect();
    let mut out = Vec::new();
    for (i, l) in lineas.iter().enumerate() {
        let t = l.trim_start();
        if t.starts_with("#[cfg(test)]")
            && lineas
                .get(i + 1)
                .is_some_and(|s| s.trim_start().starts_with("mod "))
        {
            break;
        }
        if t.starts_with("//") || l.contains(NO_ES_UN_PUNTERO) {
            continue;
        }
        if VETADOS.iter().any(|v| l.contains(v)) {
            out.push((i + 1, t.to_string()));
        }
    }
    out
}

#[test]
fn nada_toca_un_puntero_sin_pasar_por_la_puerta() {
    let raiz = crates();
    let mut ficheros = Vec::new();
    for c in std::fs::read_dir(&raiz).expect("crates/").flatten() {
        rs(&c.path().join("src"), &mut ficheros);
    }
    assert!(
        ficheros.len() > 100,
        "no veo el código de las crates: la guarda no mide nada"
    );
    let mut de_mas = Vec::new();
    let mut usados = std::collections::BTreeSet::new();
    for f in &ficheros {
        let rel = f
            .strip_prefix(&raiz)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        // La puerta misma.
        if rel.starts_with("ore-core/src/punteros") {
            continue;
        }
        let v = vetadas(&std::fs::read_to_string(f).unwrap());
        if v.is_empty() {
            continue;
        }
        if PERMITIDOS.iter().any(|(p, _)| *p == rel) {
            usados.insert(rel);
            continue;
        }
        for (n, l) in v {
            de_mas.push(format!("  {rel}:{n}  {l}"));
        }
    }
    assert!(
        de_mas.is_empty(),
        "tocan un puntero sin pasar por `ore_core::punteros::Punteros` (0079 §19.1):\n{}\n\n\
         Léelo o escríbelo por la puerta. Si de verdad es del mundo de git, va en PERMITIDOS con \
         su razón; si no es un puntero, la línea lo dice: `{NO_ES_UN_PUNTERO}`.",
        de_mas.join("\n")
    );
    let sobran: Vec<&str> = PERMITIDOS
        .iter()
        .map(|(p, _)| *p)
        .filter(|p| !usados.contains(*p))
        .collect();
    assert!(
        sobran.is_empty(),
        "ya no tocan punteros y siguen en PERMITIDOS: {sobran:?}. Quítalos de la lista."
    );
}

#[test]
fn la_guarda_ve_lo_que_veta() {
    let malo = "fn f(r: &Path) {\n    let d = r.join(\"datasets\");\n    ore_core::punteros::crudo::leer_en(&d, \"a.b\");\n}\n";
    assert_eq!(vetadas(malo).len(), 2);
    let comentado = "// r.join(\"datasets\")\nlet d = pkg.join(\"datasets\"); // no es un puntero: documentos\n";
    assert!(vetadas(comentado).is_empty());
    let pruebas = "fn f() {}\n#[cfg(test)]\nmod tests {\n    let d = r.join(\"datasets\");\n}\n";
    assert!(vetadas(pruebas).is_empty());
    // Un `#[cfg(test)]` suelto (una función) no corta: lo de después cuenta.
    let suelto = "#[cfg(test)]\nfn solo_en_pruebas() {}\nlet d = r.join(\"copias\");\n";
    assert_eq!(vetadas(suelto).len(), 1);
}
