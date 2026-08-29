//! Runner de la suite de conformidad de OOS.
//!
//! # Por qué esto es un test de integración y no una función de biblioteca
//!
//! La especificación exige que una implementación **no requiera conocimiento
//! privilegiado de estructuras internas**, y que la implementación de
//! referencia ejecute la suite **como un consumidor externo**
//! (`00-overview.md` §3.3).
//!
//! Aquí eso no es una promesa: este runner invoca el binario `ore` por su CLI
//! pública y no enlaza contra `ore-core`. No *puede* hacer trampa. El día que
//! alguien escriba una segunda implementación hará exactamente lo mismo, y esa
//! simetría es lo único que impide que la especificación acabe teniendo la
//! forma de su implementación de referencia.
//!
//! # Qué mide
//!
//! Los 73 casos del submódulo `vendor/oos`, agrupados por la operación que
//! afirman. Hoy están todos en rojo, y ese es el punto de partida: el objetivo
//! de la fase 0 es un número que solo puede subir.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Lo que un caso afirma. Se lee de `expects:` en su `case.yaml`.
#[derive(Debug, PartialEq, Eq)]
enum Expects {
    /// Debe aceptarse.
    Accept,
    /// Debe rechazarse con este código exacto. Fallar antes con otro **no** vale.
    Error(String),
    /// Dos entradas deben producir la misma forma canónica.
    Converge,
    /// Dos entradas **no** deben converger. Vale tanto como lo anterior.
    Diverge,
    /// Afirmación sobre ausencia de campos computados.
    Canonical,
    /// N compilaciones del mismo paquete producen el mismo digest.
    Stable,
    /// Dos entradas producen el mismo digest.
    SameDigest,
    /// Dos entradas producen digests distintos.
    DifferentDigest,
    /// Ida y vuelta por un formato externo es la identidad.
    Roundtrip,
    /// La emisión debe fallar.
    EmitFails,
    /// Propiedades estructurales del artefacto emitido.
    Structure,
}

impl Expects {
    fn parse(raw: &str, dir: &str) -> Self {
        match raw {
            "accept" => Self::Accept,
            "converge" => Self::Converge,
            "diverge" => Self::Diverge,
            "canonical" => Self::Canonical,
            "stable" => Self::Stable,
            "same" => Self::SameDigest,
            "different" => Self::DifferentDigest,
            "roundtrip" => Self::Roundtrip,
            "emit-fails" => Self::EmitFails,
            "structure" => Self::Structure,
            // Un caso de diff puede esperar varios códigos: "OOS5006, OOS5018".
            otro if otro.starts_with("OOS") => Self::Error(otro.to_string()),
            otro => panic!("`expects: {otro}` desconocido en {dir}"),
        }
    }
}

/// Familias de código ya implementadas. Un caso que espera un código de una
/// familia ausente de esta lista está *pendiente*, no *roto*: distinguirlo es
/// lo que permite que el marcador solo suba y que una regresión de verdad
/// destaque.
const IMPLEMENTADAS: &[&str] = &["OOS1"];

fn implementada(codigo: &str) -> bool {
    IMPLEMENTADAS.iter().any(|f| codigo.starts_with(f))
}

struct Case {
    dir: PathBuf,
    grupo: String,
    nombre: String,
    expects: Expects,
}

/// Lector mínimo de `case.yaml`.
///
/// Deliberadamente a mano: `case.yaml` son cuatro claves planas, y **elegir un
/// parser de YAML es una decisión que merece su propia evaluación** contra los
/// casos de `conformance/canonical/`. La forma canónica depende de cómo el
/// parser resuelva tipos implícitos, anclas y alias; no es una dependencia que
/// se escoja de paso para leer un fichero de metadatos.
fn campo(texto: &str, clave: &str) -> Option<String> {
    texto.lines().find_map(|l| {
        let l = l.trim_end();
        l.strip_prefix(clave)
            .and_then(|r| r.strip_prefix(':'))
            .map(|v| v.trim().to_string())
    })
}

/// Recorrido recursivo con `std`. Quince lineas evitan una dependencia que en
/// Windows arrastra FFI de plataforma, y este runner no necesita nada mas.
fn buscar(dir: &Path, encontrados: &mut Vec<PathBuf>) {
    let Ok(entradas) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entradas.flatten() {
        let p = e.path();
        if p.is_dir() {
            buscar(&p, encontrados);
        } else if p.file_name().is_some_and(|n| n == "case.yaml") {
            encontrados.push(p);
        }
    }
}

fn descubrir(raiz: &Path) -> Vec<Case> {
    let mut ficheros = Vec::new();
    buscar(raiz, &mut ficheros);
    let mut casos: Vec<Case> = ficheros
        .into_iter()
        .map(|f| {
            let dir = f.parent().unwrap().to_path_buf();
            let texto = std::fs::read_to_string(&f).expect("case.yaml ilegible");
            let rel = dir.strip_prefix(raiz).unwrap();
            let mut comps = rel.components();
            let grupo = comps
                .next()
                .unwrap()
                .as_os_str()
                .to_string_lossy()
                .into_owned();
            let nombre = comps
                .next()
                .unwrap()
                .as_os_str()
                .to_string_lossy()
                .into_owned();
            let raw = campo(&texto, "expects")
                .unwrap_or_else(|| panic!("falta `expects:` en {}", dir.display()));
            let expects = Expects::parse(&raw, &nombre);
            Case {
                dir,
                grupo,
                nombre,
                expects,
            }
        })
        .collect();
    casos.sort_by(|a, b| (&a.grupo, &a.nombre).cmp(&(&b.grupo, &b.nombre)));
    casos
}

/// Ejecuta un caso invocando el binario. `Ok(())` si pasa.
fn ejecutar(caso: &Case) -> Result<(), String> {
    let ore = env!("CARGO_BIN_EXE_ore");

    let correr = |sub: &str, dir: &str| -> Result<(bool, String), String> {
        let salida = Command::new(ore)
            .arg(sub)
            .arg(caso.dir.join(dir))
            .output()
            .map_err(|e| format!("no se pudo invocar `ore`: {e}"))?;
        let texto = format!(
            "{}{}",
            String::from_utf8_lossy(&salida.stdout),
            String::from_utf8_lossy(&salida.stderr)
        );
        Ok((salida.status.success(), texto))
    };

    /// El PRIMER código emitido. La suite exige que no se falle antes con uno
    /// distinto del esperado (`conformance/README.md` §8), así que comparar el
    /// primero es exactamente la regla de precedencia.
    fn primer_codigo(texto: &str) -> Option<String> {
        let i = texto.find("error[")? + 6;
        let j = texto[i..].find(']')? + i;
        Some(texto[i..j].to_string())
    }

    // `validate` solo cubre `valid/` e `invalid/`. Los demás grupos afirman
    // operaciones —diff, forma canónica, digest, emisión— que aún no existen,
    // y enrutarlos aquí produciría ruido en vez de un marcador honesto.
    if !matches!(caso.grupo.as_str(), "valid" | "invalid") {
        return Err("no implementado".into());
    }

    match &caso.expects {
        Expects::Error(esperado) => {
            let (ok, texto) = correr("validate", "input")?;
            if texto.contains("no implementado") {
                return Err("no implementado".into());
            }
            if ok {
                return Err(if implementada(esperado) {
                    format!("aceptado, pero debía fallar con {esperado}")
                } else {
                    "no implementado".into()
                });
            }
            match primer_codigo(&texto) {
                Some(c) if &c == esperado => Ok(()),
                Some(c) => Err(format!("falló con {c}, se esperaba {esperado}")),
                None => Err("falló sin emitir código".into()),
            }
        }
        Expects::Accept => {
            let (ok, texto) = correr("validate", "input")?;
            if texto.contains("no implementado") {
                return Err("no implementado".into());
            }
            if ok {
                Ok(())
            } else {
                Err(format!(
                    "rechazado con {}",
                    primer_codigo(&texto).unwrap_or_default()
                ))
            }
        }
        _ => {
            let (_, texto) = correr("validate", "input")?;
            if texto.contains("no implementado") {
                Err("no implementado".into())
            } else {
                Err("operación no implementada en el runner".into())
            }
        }
    }
}

#[test]
fn suite_de_conformidad() {
    let raiz = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/oos/conformance")
        .canonicalize()
        .expect(
            "no se encuentra vendor/oos/conformance — \
             ejecuta `git submodule update --init`",
        );

    let casos = descubrir(&raiz);
    assert!(!casos.is_empty(), "la suite está vacía");

    let mut por_grupo: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    let mut fallos: Vec<(String, String, String)> = Vec::new();

    for c in &casos {
        let entrada = por_grupo.entry(c.grupo.as_str()).or_default();
        entrada.1 += 1;
        match ejecutar(c) {
            Ok(()) => entrada.0 += 1,
            Err(motivo) => fallos.push((c.grupo.clone(), c.nombre.clone(), motivo)),
        }
    }

    let verdes: usize = por_grupo.values().map(|(v, _)| v).sum();
    let total: usize = por_grupo.values().map(|(_, t)| t).sum();

    println!();
    println!("  ┌─────────────────────────────────────────────┐");
    println!("  │  SUITE DE CONFORMIDAD · OOS v1alpha1        │");
    println!("  └─────────────────────────────────────────────┘");
    println!();
    for (grupo, (v, t)) in &por_grupo {
        let barra = "█".repeat(v * 20 / t.max(&1)) + &"░".repeat(20 - v * 20 / t.max(&1));
        println!("    {grupo:<11} {barra}  {v:>2} / {t:<2}");
    }
    println!();
    println!("    {:<11} {:>25} / {total}", "TOTAL", verdes);
    println!();

    if verdes < total {
        let pendientes = fallos
            .iter()
            .filter(|(_, _, m)| m == "no implementado")
            .count();
        println!("    {pendientes} casos esperando implementación.");
        println!("    Fase 0 · `ore validate` + `ore compile` sobre los siete esquemas.");
        println!();
        let regresiones: Vec<_> = fallos
            .iter()
            .filter(|(_, _, m)| !m.starts_with("no implementado") && !m.starts_with("operación no"))
            .collect();
        if !regresiones.is_empty() {
            println!("    [31mRegresiones — fallan por algo que NO es «sin implementar»:[0m");
            for (g, n, m) in &regresiones {
                println!("      {g}/{n}: {m}");
            }
            println!();
        }
        assert!(regresiones.is_empty(), "{} regresiones", regresiones.len());
    }
}

/// La suite del submódulo debe contener exactamente lo que la especificación
/// declara. Si este test se rompe tras actualizar `vendor/oos`, la suite ha
/// cambiado y hay que mirar por qué.
#[test]
fn el_submodulo_trae_la_suite_completa() {
    let raiz = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/oos/conformance")
        .canonicalize()
        .expect("submódulo sin inicializar");
    let casos = descubrir(&raiz);

    let mut por_grupo: BTreeMap<&str, usize> = BTreeMap::new();
    for c in &casos {
        *por_grupo.entry(c.grupo.as_str()).or_default() += 1;
    }

    assert_eq!(casos.len(), 73, "número de casos inesperado");
    assert_eq!(por_grupo.get("invalid"), Some(&32));
    assert_eq!(por_grupo.get("diff"), Some(&20));
    assert_eq!(por_grupo.get("canonical"), Some(&9));
    assert_eq!(por_grupo.get("digest"), Some(&6));
    assert_eq!(por_grupo.get("emit"), Some(&5));
    assert_eq!(por_grupo.get("valid"), Some(&1));
}
