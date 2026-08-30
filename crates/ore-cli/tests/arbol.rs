//! Guardián del árbol de trabajo: **un fin de línea es contenido**.
//!
//! # De qué protege, exactamente
//!
//! `90-canonical-form.md` §N1 computa el digest de un paquete sobre su
//! contenido, y un fin de línea forma parte de él. Windows pone
//! `core.autocrlf = true` en el gitconfig del sistema, así que sin una política
//! escrita en el repositorio el fin de línea de cada árbol lo decide la máquina
//! que lo extrajo. Dos copias de trabajo del mismo commit acaban difiriendo byte
//! a byte y **git declara limpias a las dos**, porque compara `stat` y no
//! contenido.
//!
//! Los dos árboles —este y `vendor/oos`— fijan la política en su
//! `.gitattributes`. Este test la hace cumplir, que no es lo mismo que
//! declararla:
//!
//! 1. **La política existe y sigue diciendo lo que debe.** Es la afirmación que
//!    más pesa. Si `* text=auto eol=lf` desapareciera de `vendor/oos`, un
//!    `git add` en Windows reescribiría los 1.288 ficheros de la suite de
//!    conformidad, **cuyos bytes son precisamente lo que afirman**. Ya pasó una
//!    vez, y es por lo que ese fichero existe.
//! 2. **Ningún fichero gobernado por `eol=lf` lleva un CR.** Para esos ficheros
//!    disco y blob son la misma afirmación: el filtro de entrada convierte CRLF
//!    en LF al commitear, y `eol=lf` no reintroduce nada al extraer. Así que un
//!    CR en disco es *exactamente* la deriva que git no puede ver, venga de una
//!    edición local o de un blob que se commiteó antes de que hubiera política.
//!
//! La excepción se lee del propio `.gitattributes` en vez de repetirse aquí.
//! `conformance/**` está en `-text` a propósito —hay casos que codifican una
//! diferencia de bytes cuya irrelevancia es justo lo que afirman— y ahí git sí
//! detecta cualquier cambio, porque guarda los bytes tal cual. Repetir esa lista
//! en dos sitios sería reintroducir el problema que este test cierra.
//!
//! # Lo que encontró la primera vez
//!
//! `crates/ore-cli/Cargo.toml`, con CRLF **en el blob**, en un árbol donde los
//! otros treinta y seis ficheros estaban en LF. Este árbol no tenía
//! `.gitattributes`: estaba bien por un ajuste local de una máquina, y no del
//! todo.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Un árbol con `.gitattributes` propio, relativo a la raíz del repositorio.
struct Arbol {
    nombre: &'static str,
    ruta: &'static str,
}

const ARBOLES: &[Arbol] = &[
    Arbol {
        nombre: "ORE",
        ruta: ".",
    },
    Arbol {
        nombre: "vendor/oos",
        ruta: "vendor/oos",
    },
];

fn raiz() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Lee la política de un árbol y devuelve **lo que está exento a propósito**.
///
/// Falla si el `.gitattributes` no está, si la regla universal ya no fija
/// `eol=lf`, o si trae un patrón que este guardián no sabe leer.
fn politica(base: &Path, arbol: &Arbol) -> Vec<String> {
    let texto = std::fs::read_to_string(base.join(".gitattributes")).unwrap_or_else(|_| {
        panic!(
            "{} no tiene `.gitattributes`.\n\
             Sin él, el fin de línea de este árbol lo decide `core.autocrlf` de cada \
             máquina —que en Windows viene a `true` en el gitconfig del sistema—, y este \
             guardián no tiene ninguna política que hacer cumplir.",
            arbol.nombre
        )
    });

    let mut universal = false;
    let mut exentos = Vec::new();

    for linea in texto.lines() {
        let linea = linea.trim();
        if linea.is_empty() || linea.starts_with('#') {
            continue;
        }
        let mut campos = linea.split_whitespace();
        let patron = campos
            .next()
            .expect("una línea no vacía tiene al menos un campo");
        let atributos: Vec<&str> = campos.collect();

        if patron == "*" {
            assert!(
                atributos.contains(&"text=auto") && atributos.contains(&"eol=lf"),
                "la regla `*` de {}/.gitattributes ya no dice `text=auto eol=lf`, sino \
                 `{linea}`. Esa regla es la política entera del árbol.",
                arbol.nombre
            );
            universal = true;
        } else if atributos.contains(&"-text") {
            exentos.push(prefijo(patron, arbol.nombre));
        } else {
            panic!(
                "{}/.gitattributes trae una regla que este guardián no sabe leer: `{linea}`.\n\
                 Enséñasela antes de confiar en él: un patrón mal leído exceptúa de más, y un \
                 guardián que exceptúa de más tiene el mismo aspecto que uno que funciona.",
                arbol.nombre
            );
        }
    }

    assert!(
        universal,
        "{}/.gitattributes no fija `* text=auto eol=lf`, que es su política entera.",
        arbol.nombre
    );
    exentos
}

/// Los únicos patrones exentos que hay son prefijos de directorio
/// (`conformance/**`). Cualquier otro se rechaza en voz alta en vez de
/// interpretarse a medias.
fn prefijo(patron: &str, arbol: &str) -> String {
    let base = patron
        .strip_suffix("/**")
        .or_else(|| patron.strip_suffix("/*"))
        .unwrap_or(patron);
    assert!(
        !base.contains(['*', '?', '[', ']']),
        "{arbol}/.gitattributes exceptúa `{patron}`, y este guardián solo sabe leer \
         prefijos de directorio."
    );
    format!("{}/", base.trim_end_matches('/'))
}

/// Los ficheros que git sigue en ese árbol. Se le pregunta a git en vez de
/// recorrer el disco porque el conjunto gobernado por `.gitattributes` es
/// exactamente ese, y porque así no hay que reimplementar `.gitignore`.
fn seguidos(base: &Path, arbol: &Arbol) -> Vec<String> {
    let salida = Command::new("git")
        .arg("-C")
        .arg(base)
        .args(["ls-files", "-z"])
        .output()
        .unwrap_or_else(|e| panic!("no se pudo ejecutar git sobre {}: {e}", arbol.nombre));

    assert!(
        salida.status.success(),
        "`git ls-files` falló sobre {}: {}",
        arbol.nombre,
        String::from_utf8_lossy(&salida.stderr).trim()
    );

    salida
        .stdout
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect()
}

/// La afirmación que más pesa: que la política sigue ahí y sigue diciendo lo que
/// debe. Todo lo demás depende de ella.
#[test]
fn cada_arbol_fija_su_politica_de_fin_de_linea() {
    let raiz = raiz();
    for arbol in ARBOLES {
        politica(&raiz.join(arbol.ruta), arbol);
    }
}

#[test]
fn ningun_fichero_gobernado_por_eol_lf_lleva_cr() {
    let raiz = raiz();
    let mut hallazgos = Vec::new();

    for arbol in ARBOLES {
        let base = raiz.join(arbol.ruta);
        let exentos = politica(&base, arbol);

        for rel in seguidos(&base, arbol) {
            if exentos.iter().any(|p| rel.starts_with(p.as_str())) {
                continue;
            }
            let ruta = base.join(&rel);
            // Un gitlink aparece como ruta seguida y es un directorio: es otro
            // árbol, con su propia política, y se recorre por su cuenta.
            if !ruta.is_file() {
                continue;
            }
            let bytes = std::fs::read(&ruta)
                .unwrap_or_else(|e| panic!("no se pudo leer {}/{rel}: {e}", arbol.nombre));
            let cr = bytes.iter().filter(|b| **b == b'\r').count();
            if cr > 0 {
                hallazgos.push(format!("  {}/{rel} — {cr} CR", arbol.nombre));
            }
        }
    }

    assert!(
        hallazgos.is_empty(),
        "estos ficheros llevan CR y su árbol los declara `eol=lf`:\n\n{}\n\n\
         git los dará por limpios —compara `stat`, no contenido—, así que esta deriva no \
         sale en ningún `status` ni en ningún `diff`. `git add --renormalize .` la corrige, \
         y conviene hacerlo antes de que dos copias de trabajo del mismo commit dejen de \
         ser el mismo árbol.",
        hallazgos.join("\n")
    );
}
