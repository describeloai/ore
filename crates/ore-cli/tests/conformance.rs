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
    /// Un caso de `diff/`: la clasificación por eje debe coincidir con
    /// `expected.diff.json`. Puede afirmar varios códigos a la vez, porque un
    /// mismo cambio se evalúa contra los cuatro ejes por separado.
    Diff(Vec<String>),
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
            otro if otro.starts_with("OOS") && otro.contains(',') => {
                Self::Diff(otro.split(',').map(|c| c.trim().to_string()).collect())
            }
            otro if otro.starts_with("OOS") => Self::Error(otro.to_string()),
            otro => panic!("`expects: {otro}` desconocido en {dir}"),
        }
    }
}

/// Familias de código ya implementadas. Un caso que espera un código de una
/// familia ausente de esta lista está *pendiente*, no *roto*: distinguirlo es
/// lo que permite que el marcador solo suba y que una regresión de verdad
/// destaque.
const IMPLEMENTADAS: &[&str] = &[
    // OOS1xxx · sintaxis y esquema
    "OOS1001", "OOS1002", "OOS1003", "OOS1004", "OOS1005",
    // OOS2xxx · referencias e integridad. Falta OOS2013: exige regenerar el
    // esquema Cedar y compararlo, y eso es fase 2.
    "OOS2002", "OOS2003", "OOS2004", "OOS2005", "OOS2006", "OOS2007", "OOS2008", "OOS2009",
    "OOS2010", "OOS2011", "OOS2012", // OOS3xxx · sistema de tipos
    "OOS3001", "OOS3002", "OOS3003", "OOS3004", "OOS3005",
    // OOS4xxx · gobernanza y flujo
    "OOS4001", "OOS4002", "OOS4003", "OOS4006", "OOS4007", "OOS4008", "OOS4011", "OOS4012",
    "OOS4014", // OOS5xxx · compatibilidad
    "OOS5001", "OOS5002", "OOS5003", "OOS5006", "OOS5007", "OOS5008", "OOS5009", "OOS5010",
    "OOS5011", "OOS5012", "OOS5013", "OOS5014", "OOS5015", "OOS5016", "OOS5017", "OOS5018",
    "OOS5019", "OOS5020", "OOS5021", "OOS5022", // OOS6xxx · forma canónica
    "OOS6003",
];

fn implementada(codigo: &str) -> bool {
    IMPLEMENTADAS.contains(&codigo)
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

    let correr_args = |sub: &str, args: &[&str]| -> Result<(bool, String), String> {
        let salida = Command::new(ore)
            .arg(sub)
            .args(args.iter().map(|d| caso.dir.join(d)))
            .output()
            .map_err(|e| format!("no se pudo invocar `ore`: {e}"))?;
        let texto = format!(
            "{}{}",
            String::from_utf8_lossy(&salida.stdout),
            String::from_utf8_lossy(&salida.stderr)
        );
        Ok((salida.status.success(), texto))
    };

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

    if caso.grupo == "diff" {
        return ejecutar_diff(caso, &correr_args);
    }
    if matches!(caso.grupo.as_str(), "canonical" | "digest") {
        return ejecutar_compile(caso, &correr_args);
    }

    // `validate` cubre `valid/` e `invalid/`; `diff` cubre `diff/`; `compile`
    // cubre `canonical/` y `digest/`. Queda `emit/`, que afirma una operación
    // que aún no existe: enrutarla aquí produciría ruido en vez de un marcador
    // honesto.
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

/// Los pares `(código, eje)` de un `changes: [...]`, y el salto exigido.
///
/// Lo normativo de un caso de `diff/` es la **clasificación**: qué código, en
/// qué eje. `subject`, `from` y `to` son informativos —el README de la suite lo
/// dice— así que compararlos convertiría una diferencia de redacción en un
/// fallo de conformidad.
fn clasificacion(texto: &str) -> (Vec<String>, String) {
    let mut pares = Vec::new();
    // Cada objeto de `changes` es un `{ … }` sin anidar dentro del array.
    for trozo in texto.split('{').skip(1) {
        let trozo = &trozo[..trozo.find('}').unwrap_or(trozo.len())];
        if let (Some(c), Some(e)) = (campo_json(trozo, "code"), campo_json(trozo, "axis")) {
            pares.push(format!("{c}/{e}"));
        }
    }
    pares.sort();
    (pares, campo_json(texto, "requiredBump").unwrap_or_default())
}

fn campo_json(texto: &str, clave: &str) -> Option<String> {
    let marca = format!("\"{clave}\"");
    let i = texto.find(&marca)? + marca.len();
    let resto = texto[i..].trim_start().strip_prefix(':')?.trim_start();
    let resto = resto.strip_prefix('"')?;
    Some(resto[..resto.find('"')?].to_string())
}

/// Invocar el binario y quedarse con si salió bien y qué escribió.
type Invocar<'a> = &'a dyn Fn(&str, &[&str]) -> Result<(bool, String), String>;

fn ejecutar_diff(caso: &Case, correr: Invocar<'_>) -> Result<(), String> {
    let esperado = std::fs::read_to_string(caso.dir.join("expected.diff.json"))
        .map_err(|_| "falta expected.diff.json".to_string())?;
    let (codigos_esperados, bump_esperado) = clasificacion(&esperado);

    if codigos_esperados
        .iter()
        .any(|p| !implementada(p.split('/').next().unwrap_or("")))
    {
        return Err("no implementado".into());
    }

    let (_, salida) = correr("diff", &["before", "after"])?;
    if salida.contains("no implementado") {
        return Err("no implementado".into());
    }
    let (codigos, bump) = clasificacion(&salida);

    if codigos != codigos_esperados {
        return Err(format!(
            "clasificación {codigos:?}, se esperaba {codigos_esperados:?}"
        ));
    }
    if bump != bump_esperado {
        return Err(format!("salto `{bump}`, se esperaba `{bump_esperado}`"));
    }
    Ok(())
}

/// Un campo de la salida de `ore compile`.
///
/// La comparación es entre **dos salidas del mismo binario**, nunca contra un
/// valor escrito a mano. Es lo que `conformance/README.md` §4.1 exige de estos
/// grupos: nadie audita 400 bytes de JSON canónico ni calcula un SHA-256 a
/// mano, así que lo que se afirma son **relaciones** —convergen, no convergen,
/// mismo digest, distinto digest— y todas son verificables leyendo las entradas.
fn campo_compile(salida: &str, clave: &str) -> Option<String> {
    campo_json(salida, clave)
}

fn ejecutar_compile(caso: &Case, correr: Invocar<'_>) -> Result<(), String> {
    let compilar = |dir: &str| -> Result<String, String> {
        let (ok, texto) = correr("compile", &[dir])?;
        if texto.contains("no implementado") {
            return Err("no implementado".into());
        }
        if !ok {
            return Err(format!(
                "`{dir}` no compila: {}",
                texto.lines().next().unwrap_or("")
            ));
        }
        Ok(texto)
    };

    match &caso.expects {
        // `canonical/` afirma sobre la forma canónica; `digest/` sobre el
        // digest del bundle, que es el que incorpora el lock (§5.3).
        Expects::Converge | Expects::Diverge => {
            let (a, b) = (compilar("a")?, compilar("b")?);
            let (ca, cb) = (seccion_canonica(&a), seccion_canonica(&b));
            let convergen = ca == cb;
            match (&caso.expects, convergen) {
                (Expects::Converge, false) => Err("no convergen y debían".into()),
                (Expects::Diverge, true) => Err("convergen y NO debían".into()),
                _ => Ok(()),
            }
        }
        Expects::SameDigest | Expects::DifferentDigest => {
            let (a, b) = (compilar("a")?, compilar("b")?);
            let (da, db) = (
                campo_compile(&a, "bundle").unwrap_or_default(),
                campo_compile(&b, "bundle").unwrap_or_default(),
            );
            if da.is_empty() {
                return Err("sin digest de bundle en la salida".into());
            }
            match (&caso.expects, da == db) {
                (Expects::SameDigest, false) => Err(format!("digests distintos: {da} / {db}")),
                (Expects::DifferentDigest, true) => Err("mismo digest y NO debía".into()),
                _ => Ok(()),
            }
        }
        // Pureza: sin reloj, sin aleatoriedad, sin red. Tres ejecuciones.
        Expects::Stable => {
            let uno = compilar("input")?;
            for _ in 0..2 {
                if compilar("input")? != uno {
                    return Err("dos compilaciones del mismo paquete difieren".into());
                }
            }
            Ok(())
        }
        // N8: lo que el compilador calcula no se serializa en el paquete fuente.
        Expects::Canonical => {
            let salida = compilar("input")?;
            let canonica = seccion_canonica(&salida);
            let esperado = std::fs::read_to_string(caso.dir.join("expected.absent.json"))
                .map_err(|_| "falta expected.absent.json".to_string())?;
            for (puntero, prohibida) in claves_prohibidas(&esperado) {
                let ambito = objeto_en(&canonica, &puntero);
                if ambito.contains(&format!("\"{prohibida}\"")) {
                    return Err(format!("`{puntero}` contiene `{prohibida}`"));
                }
            }
            Ok(())
        }
        otro => Err(format!("`{otro:?}` no encaja en compile")),
    }
}

/// El bloque `"canonical"` de la salida, sin los digests: comparar los digests
/// sería comparar dos veces lo mismo, y ante una divergencia no diría **qué**
/// divergió.
fn seccion_canonica(salida: &str) -> String {
    let Some(i) = salida.find("\"canonical\"") else {
        return String::new();
    };
    let resto = &salida[i..];
    let fin = resto
        .find(
            "
  \"digest\"",
        )
        .unwrap_or(resto.len());
    resto[..fin].to_string()
}

/// Los pares `(puntero, clave)` de `expected.absent.json`.
///
/// El puntero importa: `labels` en `baseSalary` es una etiqueta **declarada** y
/// tiene que estar. En `totalCompensation`, que es derivada, la etiqueta la
/// computa el compilador y no debe aparecer. La misma clave, prohibida en un
/// sitio y obligatoria en otro — comprobarla en todo el documento confundiría
/// las dos cosas.
fn claves_prohibidas(esperado: &str) -> Vec<(String, String)> {
    esperado
        .split("{ \"at\"")
        .skip(1)
        .filter_map(|t| {
            let at = entrecomillado(t)?;
            let resto = &t[t.find("\"key\"")?..];
            Some((at, entrecomillado(&resto["\"key\"".len()..])?))
        })
        .collect()
}

fn entrecomillado(t: &str) -> Option<String> {
    let t = t
        .trim_start()
        .strip_prefix(':')?
        .trim_start()
        .strip_prefix('"')?;
    Some(t[..t.find('"')?].to_string())
}

/// El texto del objeto que hay en un puntero JSON, delimitado por llaves
/// equilibradas. Suficiente para una afirmación de ausencia, y evita meter un
/// analizador de JSON en el arnés.
fn objeto_en(texto: &str, puntero: &str) -> String {
    let mut ambito = texto;
    for seg in puntero.split('/').filter(|s| !s.is_empty()) {
        let Some(i) = ambito.find(&format!("\"{seg}\"")) else {
            return String::new();
        };
        let resto = &ambito[i..];
        let Some(abre) = resto.find('{') else {
            return String::new();
        };
        let mut nivel = 0usize;
        let mut fin = resto.len();
        for (j, c) in resto[abre..].char_indices() {
            match c {
                '{' => nivel += 1,
                '}' => {
                    nivel -= 1;
                    if nivel == 0 {
                        fin = abre + j + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        ambito = &resto[abre..fin];
    }
    ambito.to_string()
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
