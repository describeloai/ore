//! El guardián del árbol de dependencias del binario que se distribuye.
//!
//! # Qué protege
//!
//! `main.rs` rotula el compilador *«hermético: sin red, sin credenciales, sin
//! reloj»*, y `lector.rs` explica por qué esa frase solo vale si es una propiedad
//! del **artefacto** y no una promesa del código: un binario sin código de red no
//! puede hacer una llamada; uno con una pila TLS enlazada que promete no usarla
//! tiene una política.
//!
//! Desde que existe `ore-read-postgres` esa propiedad está a un `cargo add` de
//! distancia de romperse **sin que nadie lo note**: el driver trae 65 crates y FFI
//! de plataforma, y añadirlas al crate equivocado no rompe ninguna prueba, no
//! cambia ninguna salida y no sale en ninguna revisión.
//!
//! # Por qué se lee `Cargo.lock` y no se ejecuta `cargo tree`
//!
//! Porque `Cargo.lock` está en el repositorio, tiene las aristas, y leerlo no
//! necesita red, ni toolchain, ni un cargo anidado dentro de otro cargo. La
//! medición es el cierre transitivo real, no una lista escrita a mano que
//! envejece.
//!
//! Y **el reparto en directorios no demuestra nada**: `cargo` no mira los
//! directorios. Que el driver esté en `crates/ore-read-postgres/` es una
//! convención; que sus dependencias no alcancen a `ore-cli` es un hecho, y este
//! fichero es el que lo convierte en uno.

use std::collections::{BTreeMap, BTreeSet};

/// Lo que jamás debe entrar en el binario `ore`, con el motivo de cada veto.
///
/// Es una lista de **capacidades**, no de crates concretas: se compara por
/// prefijo para que la sustituta de mañana caiga en la misma red. Un veto sin
/// motivo escrito es un veto que alguien levantará sin saber qué rompía.
const VETADAS: &[(&str, &str)] = &[
    (
        "tokio",
        "un planificador asíncrono solo hace falta para hablar con algo",
    ),
    (
        "postgres",
        "un driver de base de datos vive fuera: es la costura entera",
    ),
    ("native-tls", "TLS es red, y el compilador no tiene red"),
    ("rustls", "TLS es red, y el compilador no tiene red"),
    ("openssl", "FFI contra una biblioteca del sistema"),
    ("schannel", "FFI contra la pila TLS de Windows"),
    ("security-framework", "FFI contra el llavero de macOS"),
    (
        "aws-lc",
        "C y ensamblador, con cmake y nasm para construirlo",
    ),
    (
        "ring",
        "ensamblador; `sha2` cubre lo que el digest necesita",
    ),
    (
        "reqwest",
        "un cliente HTTP es exactamente lo que no debe existir aquí",
    ),
    (
        "hyper",
        "un cliente HTTP es exactamente lo que no debe existir aquí",
    ),
    ("mio", "entrada/salida por eventos: solo sirve para sockets"),
    ("socket2", "sockets"),
    (
        "getrandom",
        "aleatoriedad: la compilación es una función pura",
    ),
    ("chrono", "el reloj; la compilación no lo lee"),
    ("time", "el reloj; la compilación no lo lee"),
];

/// El cierre transitivo medido hoy. No es un objetivo: es un **testigo**.
///
/// Un número exacto obliga a que cualquier dependencia nueva —buena o mala— pase
/// por aquí y se justifique. Un techo holgado dejaría entrar cinco crates de
/// tapadillo, que es como crecen los árboles de dependencias.
const CIERRE: usize = 32;

#[test]
fn el_binario_que_se_distribuye_no_sabe_hablar_por_la_red() {
    let cierre = cierre_de("ore-cli");
    let mut culpables: Vec<String> = Vec::new();
    for nombre in &cierre {
        if let Some((veto, motivo)) = VETADAS
            .iter()
            .find(|(v, _)| nombre == v || nombre.starts_with(&format!("{v}-")))
        {
            culpables.push(format!("  {nombre} — vetada como `{veto}`: {motivo}"));
        }
    }
    assert!(
        culpables.is_empty(),
        "el binario `ore` ha ganado capacidades que su documentación dice que no tiene:\n{}\n\n\
         Si de verdad hace falta, el sitio es un crate aparte —como \
         `ore-read-postgres`— y la razón va en `docs/decisions/`.",
        culpables.join("\n")
    );
}

#[test]
fn el_arbol_no_crece_sin_que_nadie_lo_diga() {
    let cierre = cierre_de("ore-cli");
    assert_eq!(
        cierre.len(),
        CIERRE,
        "el cierre de `ore-cli` pasó de {CIERRE} a {} crates.\n  \
         No es un fallo: es una decisión que tiene que ser deliberada. Si la \
         dependencia nueva se justifica, actualiza `CIERRE` y di por qué en el \
         mensaje del commit.\n  {}",
        cierre.len(),
        cierre.iter().cloned().collect::<Vec<_>>().join(", ")
    );
}

/// El ejecutor está fuera por un motivo DISTINTO al del driver, y la lista de
/// vetadas lo dice sola: el driver trae `tokio`, `native-tls` y `openssl` —la
/// red—; el evaluador trae `chrono` y `time` —el reloj—. Enlazar `cedar-policy`
/// dentro de `ore` metería un reloj en un binario cuyo invariante III dice que
/// la compilación es pura.
///
/// Y el reloj es la evidencia, no el argumento: el compilador contesta *qué dice
/// este documento* y el evaluador *puede ESTE principal*, que necesita una
/// petición. `ore validate` no tiene peticiones
/// (`docs/decisions/0007-enlazar-el-evaluador-de-cedar.md`).
#[test]
fn el_evaluador_esta_donde_esta_por_algo() {
    let ore = cierre_de("ore-cli");
    let exec = cierre_de("ore-exec");
    assert!(
        exec.len() > ore.len() * 4,
        "`ore-exec` tiene {} crates y `ore-cli` {}. La costura existe para que el          peso caiga fuera; si ya no hay peso, sobra la costura.",
        exec.len(),
        ore.len()
    );
    // Y lo que de verdad no puede pasar: que el reloj cruce la costura.
    let reloj: Vec<&String> = ore
        .iter()
        .filter(|n| n.as_str() == "chrono" || n.as_str() == "time" || n.starts_with("time-"))
        .collect();
    assert!(
        reloj.is_empty(),
        "el binario que compila ha ganado un reloj: {reloj:?}. La compilación es          pura por invariante III, y un digest que dependa del instante deja de          ser una identidad."
    );
}

/// Y el driver tiene que seguir siendo gordo: si adelgazara hasta parecerse al
/// compilador, la costura habría dejado de separar nada y valdría la pena
/// preguntarse si sigue haciendo falta.
#[test]
fn el_driver_esta_donde_esta_por_algo() {
    let ore = cierre_de("ore-cli");
    let pg = cierre_de("ore-read-postgres");
    assert!(
        pg.len() > ore.len() * 2,
        "`ore-read-postgres` tiene {} crates y `ore-cli` {}. La costura existe \
         para que el peso caiga fuera; si ya no hay peso, sobra la costura.",
        pg.len(),
        ore.len()
    );
}

// ── La medición ─────────────────────────────────────────────────────────────

/// Cierre transitivo de un paquete según `Cargo.lock`, sin contarlo a él ni a
/// los demás miembros del espacio de trabajo: lo que se mide es lo que se
/// arrastra de fuera.
fn cierre_de(raiz: &str) -> BTreeSet<String> {
    let lock = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("Cargo.lock"),
    )
    .expect("Cargo.lock: el guardián mide sobre él, y sin él no mide nada");

    let aristas = aristas(&lock);
    assert!(
        aristas.contains_key(raiz),
        "`{raiz}` no está en Cargo.lock: ¿se renombró el paquete?"
    );

    let propios: BTreeSet<&str> = ["ore-core", "ore-cli", "ore-driver", "ore-exec", "ore-read-jsonl", "ore-read-postgres"].into();
    let mut vistos = BTreeSet::new();
    let mut pila = vec![raiz.to_string()];
    while let Some(p) = pila.pop() {
        for d in aristas.get(&p).map(Vec::as_slice).unwrap_or(&[]) {
            if vistos.insert(d.clone()) {
                pila.push(d.clone());
            }
        }
    }
    vistos.retain(|d| !propios.contains(d.as_str()));
    vistos
}

/// `Cargo.lock` es TOML, y ORE no lleva un analizador de TOML — ni lo va a llevar
/// por esto, que sería justo la clase de dependencia que este fichero vigila. La
/// forma que hace falta leer es diminuta y está fijada por cargo: `[[package]]`,
/// `name = "…"`, y una lista `dependencies = [ … ]` de cadenas.
fn aristas(lock: &str) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut nombre = String::new();
    let mut en_lista = false;
    for linea in lock.lines() {
        let l = linea.trim();
        if en_lista {
            if l == "]" {
                en_lista = false;
            } else if let Some(d) = l.trim_end_matches(',').trim_matches('"').split(' ').next() {
                out.entry(nombre.clone()).or_default().push(d.to_string());
            }
            continue;
        }
        if l == "[[package]]" {
            nombre.clear();
        } else if let Some(v) = l.strip_prefix("name = ") {
            nombre = v.trim_matches('"').to_string();
            out.entry(nombre.clone()).or_default();
        } else if l == "dependencies = [" {
            en_lista = true;
        }
    }
    out
}
