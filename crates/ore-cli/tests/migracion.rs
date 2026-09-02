//! **Qué queda del binding en el árbol, y por qué.**
//!
//! El criterio de la migración a v1alpha8 se escribió como un recuento: cero
//! `kind: Binding` y cero `from: { datasource }` fuera de las suites de
//! conformidad. Un recuento a cero es fácil de conseguir y fácil de conseguir
//! **mal**: basta borrar el documento que estorba, y con él la prueba de lo que
//! afirmaba.
//!
//! Así que aquí no hay un número: hay un **inventario con motivos**. Lo que
//! queda en la forma vieja está enumerado uno a uno con la razón por la que
//! sigue ahí, y cualquier otro pone el test en rojo. Añadir uno cuesta escribir
//! por qué, que es exactamente lo que debe costar.
//!
//! Y las dos razones que hay son de clases distintas, que es lo que importa:
//!
//! - **`dos-familias` no se puede migrar.** Una entidad servida desde dos
//!   objetos de dos fuentes es algo que el binding decía sin esfuerzo y que
//!   v1alpha8 **no sabe escribir**: una entidad tiene un `backedBy`, una vista
//!   sale de un sitio, y el vocabulario no tiene junta. Es un hueco real, y
//!   borrarlo para que el recuento diera cero habría convertido una limitación
//!   en un número bonito.
//! - **`con-vista` sí se podría, y se queda a propósito.** Es el testigo de que
//!   el ejecutor sigue sirviendo un documento de una versión anterior, que es lo
//!   que `00-scope` §5.4 promete: *«un documento que declare `apiVersion:
//!   oos.dev/v1alpha1` sigue compilando»*. Su gemelo `con-tabla` mide el camino
//!   nuevo, y los dos juntos son, en el ejecutor, lo que `valid/mixed-versions`
//!   es en la conformidad.
//!
//! Por la CLI no: esto se lee del árbol de ficheros, que es lo que se afirma.

use std::path::{Path, PathBuf};

fn raiz() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Lo que **no** se recorre, y por qué no cuenta.
const FUERA: &[(&str, &str)] = &[
    (
        "target",
        "artefactos de compilación: nada de aquí es fuente",
    ),
    (".git", "historia, no árbol"),
    (
        "vendor/oos/conformance",
        "las suites certifican las versiones que EXISTIERON, incluida la que \
         retira el binding. `conformance/v1alpha1` tiene que seguir teniendo \
         bindings, y `conformance/v1alpha8/valid/mixed-versions` tiene uno a \
         propósito: es el caso que prueba que conviven",
    ),
    (
        "vendor/oos/docs/vision",
        "ontologías escritas contra el lenguaje COMPLETO, que dicen de sí mismas \
         que no validan. No son documentación ejecutable: son el destino, y el \
         destino se escribe con la gramática que todavía no hay",
    ),
];

/// Lo que sigue en la forma vieja **dentro** de lo recorrido, con su motivo.
const TESTIGOS: &[(&str, &str)] = &[
    (
        "crates/ore-exec/casos/dos-familias",
        "una entidad servida desde DOS fuentes. v1alpha8 no sabe escribirlo, y \
         el ejecutor sí sabe federarlo — ver el README del caso",
    ),
    (
        "crates/ore-exec/casos/con-vista",
        "el testigo v1alpha7 del ejecutor: su gemelo `con-tabla` mide el camino \
         nuevo, y este mide que el viejo sigue sirviéndose",
    ),
];

fn documentos(dir: &Path, raiz: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        let rel = p
            .strip_prefix(raiz)
            .unwrap_or(&p)
            .to_string_lossy()
            .replace('\\', "/");
        if FUERA.iter().any(|(d, _)| rel == *d) {
            continue;
        }
        if p.is_dir() {
            documentos(&p, raiz, out);
        } else if p.extension().and_then(|x| x.to_str()) == Some("yaml") {
            out.push(p);
        }
    }
}

/// Qué documentos siguen en la forma vieja: `(ruta relativa, qué forma)`.
fn en_la_forma_vieja() -> Vec<(String, &'static str)> {
    let raiz = raiz();
    let mut ficheros = Vec::new();
    documentos(&raiz, &raiz, &mut ficheros);
    assert!(
        ficheros.len() > 50,
        "recorrió {} documentos: el árbol no puede ser tan pequeño, algo filtró de más",
        ficheros.len()
    );

    let mut out = Vec::new();
    for f in ficheros {
        let Ok(txt) = std::fs::read_to_string(&f) else {
            continue;
        };
        let rel = f
            .strip_prefix(&raiz)
            .unwrap_or(&f)
            .to_string_lossy()
            .replace('\\', "/");
        if txt.contains("kind: Binding") {
            out.push((rel.clone(), "kind: Binding"));
        }
        // El puntero dentro de la vista. Se mira `from` con `datasource` en la
        // misma línea o en la siguiente: `materialized.datasource` sigue siendo
        // legítimo y no puede contarse.
        if txt.contains("from: { datasource")
            || txt.contains("from:\n    datasource")
            || txt.contains("from:\n  datasource")
        {
            out.push((rel, "from: { datasource }"));
        }
    }
    out
}

/// **El árbol usa la forma nueva, y lo que no la usa dice por qué.**
#[test]
fn nada_sigue_en_la_forma_vieja_sin_un_motivo_escrito() {
    let sobra: Vec<String> = en_la_forma_vieja()
        .into_iter()
        .filter(|(rel, _)| !TESTIGOS.iter().any(|(t, _)| rel.starts_with(t)))
        .map(|(rel, forma)| format!("  {rel}  —  {forma}"))
        .collect();

    assert!(
        sobra.is_empty(),
        "quedan documentos en la forma anterior a v1alpha8 sin motivo escrito:\n{}\n\n\
         Migrarlos, o —si NO se pueden migrar— añadirlos a `TESTIGOS` con la razón. \
         Un recuento a cero conseguido borrando la prueba de algo es peor que un \
         recuento que no llega.",
        sobra.join("\n")
    );
}

/// Y al revés: **un motivo que ya no describe nada se retira.**
///
/// Una lista de excepciones solo crece si nadie la mira, y una excepción muerta
/// es peor que una viva: parece que algo sigue pendiente cuando ya no lo está.
/// El día que v1alpha8 sepa decir «esta entidad sale de estos dos objetos»,
/// este test se pone rojo y pide que se borre la entrada.
#[test]
fn ningun_testigo_sobra() {
    let vivos = en_la_forma_vieja();
    let muertos: Vec<&str> = TESTIGOS
        .iter()
        .map(|(t, _)| *t)
        .filter(|t| !vivos.iter().any(|(rel, _)| rel.starts_with(t)))
        .collect();
    assert!(
        muertos.is_empty(),
        "estos testigos ya no tienen nada en la forma vieja, así que sobran de \
         `TESTIGOS`: {muertos:?}"
    );
}

/// El escaparate no tiene excepciones, y esa es media tesis del proyecto.
///
/// `examples/` es lo primero que lee alguien que llega, y un ejemplo que
/// enseñara la gramática retirada enseñaría mal. No se exceptúa: se migra.
#[test]
fn el_escaparate_esta_entero_en_la_forma_nueva() {
    let sobra: Vec<String> = en_la_forma_vieja()
        .into_iter()
        .filter(|(rel, _)| rel.starts_with("vendor/oos/examples"))
        .map(|(rel, forma)| format!("{rel} — {forma}"))
        .collect();
    assert!(sobra.is_empty(), "{sobra:?}");
}
