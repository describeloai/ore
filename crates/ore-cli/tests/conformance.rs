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
//! Los casos del árbol `v1alpha1` del submódulo `vendor/oos`, agrupados por la
//! operación que afirman. Arrancaron todos en rojo y hoy están todos en verde;
//! lo que este runner protege a partir de aquí es que ninguno vuelva. Cuántos
//! son lo cuenta un testigo, no este comentario.
//!
//! `IMPLEMENTADAS` deja de ser una lista que crece y pasa a ser un cierre: un
//! caso que espera un código ausente de ella cuenta como *pendiente* en vez de
//! *roto*. Con la suite entera en verde ya no hay pendientes, y esa distinción
//! solo sirve ya para el día que `vendor/oos` traiga un caso nuevo.

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
    // OOS2xxx · referencias e integridad. `OOS2001` lo reservó v1alpha1 y lo
    // activa v1alpha3: su caso vive en el otro árbol.
    "OOS2001", "OOS2002", "OOS2003", "OOS2004", "OOS2005", "OOS2006", "OOS2007", "OOS2008",
    "OOS2009", "OOS2010", "OOS2011", "OOS2012", "OOS2013",
    // Lo introduce v1alpha6 con la firma: la familia habla de referencias —lo
    // importado no es de quien dice ser— y la versión es otra cosa.
    "OOS2016", "OOS2017", // OOS3xxx · sistema de tipos
    "OOS3001", "OOS3002", "OOS3003", "OOS3004", "OOS3005",
    // OOS4xxx · gobernanza y flujo
    "OOS4001", "OOS4002", "OOS4003", "OOS4006", "OOS4007", "OOS4008", "OOS4011", "OOS4012",
    "OOS4014", // Lo introduce v1alpha2 al promover `expression` de prosa a CEL.
    "OOS4015", // OOS5xxx · compatibilidad
    "OOS5001", "OOS5002", "OOS5003", "OOS5006", "OOS5007", "OOS5008", "OOS5009", "OOS5010",
    "OOS5011", "OOS5012", "OOS5013", "OOS5014", "OOS5015", "OOS5016", "OOS5017", "OOS5018",
    "OOS5019", "OOS5020", "OOS5021", "OOS5022",
    // Los introduce v1alpha3: el eje POLICY sobre el plano de gobierno.
    "OOS5023", "OOS5024",
    // Lo introduce v1alpha4: una forma que exige más conceptos que antes.
    "OOS5025",
    // Lo introduce v1alpha5 al darle consumidor a `contextSurface`, y es de
    // v1alpha1: el espejo de OOS5012 que la tabla de compatibilidad no tenía.
    "OOS5026", // OOS6xxx · forma canónica
    "OOS6003",
    // OOS7xxx · efectos e integridad. Borrador de v1alpha2, contado aparte.
    "OOS7001", "OOS7002", "OOS7003", "OOS7004", "OOS7005", "OOS7006", "OOS7007", "OOS7008",
    "OOS7009", "OOS7011",
    // OOS8xxx · gobierno. Borrador de v1alpha3, contado aparte.
    "OOS8001", "OOS8002", "OOS8003", "OOS8005", "OOS8006",
    // OOS9xxx · significado. Borrador de v1alpha4, contado aparte.
    "OOS9001", "OOS9003", "OOS9004",
];

fn implementada(codigo: &str) -> bool {
    IMPLEMENTADAS.contains(&codigo) || emitidos().contains(codigo)
}

/// El caso trae un `apiVersion` que esta implementación todavía no entiende.
///
/// Sustituye a una lista de versiones escrita a mano —`v1alpha2`, `v1alpha3`—
/// que había que ampliar con cada borrador nuevo y **recortar** con cada uno
/// implementado. Nadie recorta una lista así: la de v1alpha3 seguía puesta con
/// v1alpha3 ya soportado, y un caso que hubiera empezado a fallar de verdad se
/// habría contado como pendiente sin que nadie lo viera.
///
/// Aquí se **deriva de lo que el compilador dice** (P2): pide la versión y no
/// la tiene. Un `apiVersion` **ausente** comparte código y no comparte mensaje,
/// y por eso el mensaje forma parte de la condición: ese sí es un defecto del
/// documento, y debe contarse.
fn version_no_soportada(texto: &str) -> bool {
    texto.contains("[OOS1002]") && texto.contains("no está soportada")
}

/// Los códigos que el compilador **sabe emitir de verdad**, leídos de su propio
/// código fuente.
///
/// `IMPLEMENTADAS` se escribe a mano, y se midió lo que eso cuesta: llevaba
/// **tres códigos de retraso** —`OOS2014`, `OOS2015` y `OOS3006`, los tres
/// implementados y con casos— y el marcador anunciaba *«5 casos esperando
/// implementación»* sobre cinco casos que pasaban.
///
/// > **Un marcador que se queda corto tiene exactamente el mismo aspecto que
/// > uno que dice la verdad**, y este además se equivocaba en la dirección
/// > cómoda: hacia abajo, que no molesta a nadie.
///
/// La lista sigue —un código puede emitirse desde el CLI y no desde el núcleo—
/// pero deja de ser la única fuente. **Lo derivable no se declara** (P2), y
/// aquí lo derivable es una búsqueda de texto: `Code::Oos1234` en el fuente,
/// excluyendo el fichero que **declara** el catálogo, donde están todos.
///
/// Se lee como texto, no enlazando `ore-core`: la disciplina de consumidor
/// externo se mantiene, igual que `dependencias.rs` lee `Cargo.lock`.
fn emitidos() -> &'static std::collections::BTreeSet<String> {
    static CACHE: std::sync::OnceLock<std::collections::BTreeSet<String>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| {
        let raiz = Path::new(env!("CARGO_MANIFEST_DIR")).join("../ore-core/src");
        let mut out = std::collections::BTreeSet::new();
        let mut pila = vec![raiz];
        while let Some(d) = pila.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    pila.push(p);
                } else if p.extension().and_then(|x| x.to_str()) == Some("rs")
                    && p.file_name().and_then(|x| x.to_str()) != Some("code.rs")
                    && let Ok(texto) = std::fs::read_to_string(&p)
                {
                    for trozo in texto.split("Code::Oos").skip(1) {
                        let n: String = trozo.chars().take_while(char::is_ascii_digit).collect();
                        if n.len() == 4 {
                            out.insert(format!("OOS{n}"));
                        }
                    }
                }
            }
        }
        out
    })
}

struct Case {
    dir: PathBuf,
    grupo: String,
    nombre: String,
    expects: Expects,
    /// A qué formato exporta un caso `structure`. Estaba escrito en los
    /// `case.yaml` desde el principio y el runner lo ignoraba, exportando
    /// siempre a Cedar: un campo que nadie lee es peor que uno que no existe,
    /// porque promete algo.
    formato: String,
    /// Si el caso DECLARÓ `format:`. Sin él, `formato` es un valor por defecto
    /// y no una afirmación: un caso `same` sin formato compara digests de
    /// paquete; uno que declara el formato compara **el artefacto emitido**,
    /// que es lo que dice estar comparando.
    formato_explicito: bool,
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
            // Cada borrador tiene su propio arbol y su propio marcador.
            // Mezclarlos no daria un numero falso: daria un numero que ya no
            // se sabe que mide.
            // Se DERIVA del nombre en vez de enumerarse: una lista escrita a
            // mano aqui envejece en silencio la primera vez que llega un
            // borrador nuevo, y el marcador de v1alpha1 se lleva sus casos sin
            // que nada avise. Paso con v1alpha5.
            if p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("v1alpha") && n != "v1alpha1")
            {
                continue;
            }
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
            // El formato se LEE. Antes cualquier valor desconocido caia en
            // `cedar` en silencio, asi que los casos de v1alpha5 exportaban un
            // esquema Cedar y se comparaban contra aserciones de GraphQL — y
            // pasaban, porque el comprobador ignoraba lo que no entendia. Verde
            // por la razon equivocada es peor que rojo.
            let declarado = campo(&texto, "format");
            let formato = match declarado.as_deref() {
                Some("odcs") => "odcs",
                Some("ossie") => "ossie",
                Some("graphql") => "graphql",
                // El paquete publicable. No sale de `export` y no debe: empaquetar
                // valida antes, porque publicar lo que no compila reparte un
                // problema. Que comando lo produce es asunto del corredor.
                Some("oob") => "oob",
                Some("cedar-schema") | None => "cedar",
                Some(otro) => panic!("`format: {otro}` desconocido en {nombre}"),
            }
            .to_string();
            Case {
                dir,
                grupo,
                nombre,
                expects,
                formato,
                formato_explicito: declarado.is_some(),
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
    // `pack` va con `emit` porque la MECANICA es la misma —producir un
    // artefacto y afirmar su forma— y es una familia aparte porque lo que
    // produce no lo es: `emit` habla formatos ajenos y un `.oob` es NUESTRO
    // artefacto (`v1alpha6/00-scope` §3). Mezclarlos habria dado un numero que
    // ya no se sabe que mide.
    if matches!(caso.grupo.as_str(), "emit" | "pack") {
        return ejecutar_emit(caso);
    }

    // Cada grupo por su operación: `validate` para `valid/` e `invalid/`,
    // `diff` para `diff/`, `compile` para `canonical/` y `digest/`, `export`
    // para `emit/` y `pack` para `pack/`.
    if !matches!(caso.grupo.as_str(), "valid" | "invalid") {
        return Err("no implementado".into());
    }

    match &caso.expects {
        Expects::Error(esperado) => {
            // Antes de mirar nada: un caso que espera un codigo de una familia
            // sin implementar esta pendiente, y da igual con que fallara. Sin
            // esto, un caso de v1alpha2 contaria como regresion por fallar con
            // OOS1002 — que es lo correcto hoy y no dice nada del caso.
            if !implementada(esperado) {
                return Err("no implementado".into());
            }
            let (ok, texto) = correr("validate", "input")?;
            // Un caso cuyo `apiVersion` esta implementacion no entiende no se
            // mide: falla con OOS1002 por la version, no por lo que afirma. La
            // excepcion es el caso que afirma OOS1002 — ese SI mide justo eso.
            if esperado != "OOS1002" && version_no_soportada(&texto) {
                return Err("no implementado".into());
            }
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
            // Una apiVersion que esta implementacion no entiende todavia. Se
            // limpia sola: el dia que ORE la soporte, el caso se mide de
            // verdad sin tocar el arnes.
            if texto.contains("no implementado") || version_no_soportada(&texto) {
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
        // Con `format:` declarado se comparan los ARTEFACTOS, no los digests.
        // El digest de dos escrituras del mismo paquete ya coincide por la forma
        // canónica, así que compararlo no afirmaría nada sobre la emisión —
        // sería verde por la razón equivocada.
        Expects::SameDigest | Expects::DifferentDigest if caso.formato_explicito => {
            let emitir = |lado: &str| -> Result<String, String> {
                if !EMISORES.contains(&caso.formato.as_str()) {
                    return Err("no implementado".into());
                }
                let (ok, texto) = exportar(&caso.dir.join(lado), &caso.formato)?;
                if !ok {
                    return Err(format!("no emite: {}", texto.lines().next().unwrap_or("")));
                }
                Ok(texto)
            };
            let (a, b) = (emitir("a")?, emitir("b")?);
            match (&caso.expects, a == b) {
                (Expects::SameDigest, false) => {
                    Err("dos escrituras del mismo paquete emiten artefactos distintos".into())
                }
                (Expects::DifferentDigest, true) => Err("mismo artefacto y NO debía".into()),
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

// ── emit/ ───────────────────────────────────────────────────────────────────

/// Invoca `ore export` sobre una ruta absoluta.
fn exportar(destino: &Path, formato: &str) -> Result<(bool, String), String> {
    // `oob` no sale de `export` y no debe: empaquetar valida antes, y publicar
    // lo que no compila reparte un problema. Qué comando produce cada artefacto
    // es asunto del corredor, que **no es normativo** (`conformance/README` §8);
    // lo que el caso declara es qué artefacto se está afirmando.
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ore"));
    if formato == "oob" {
        cmd.arg("pack").arg(destino);
    } else {
        cmd.arg("export").arg(destino).arg("--format").arg(formato);
    }
    let salida = cmd
        .output()
        .map_err(|e| format!("no se pudo invocar `ore`: {e}"))?;
    // Si emitio, lo que se afirma es EL ARTEFACTO — stdout— y no lo que el
    // comando contara por stderr mientras tanto. Concatenar los dos metia el
    // resumen de `pack` dentro de lo comparado, y una asercion sobre el
    // artefacto pasaba a hablar tambien de la charla. Si NO emitio, lo unico que
    // hay es el error, y es lo que un caso `emit-fails` mira.
    let texto = if salida.status.success() {
        String::from_utf8_lossy(&salida.stdout).to_string()
    } else {
        format!(
            "{}{}",
            String::from_utf8_lossy(&salida.stdout),
            String::from_utf8_lossy(&salida.stderr)
        )
    };
    Ok((salida.status.success(), texto))
}

/// Un contrato de otro formato dentro de `input/`, si lo hay. Distingue las dos
/// direcciones de la ida y vuelta sin necesidad de declararlas en `case.yaml`:
/// un fichero suelto es una entrada ajena; un directorio, un paquete OOS.
fn contrato_ajeno(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir.join("input"))
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.to_string_lossy().contains(".odcs."))
}

/// Formatos con emisor. Uno ausente de esta lista deja sus casos en
/// *pendiente*, igual que `IMPLEMENTADAS` hace con los codigos: el marcador solo
/// sube, y una regresion de verdad destaca.
/// Lo que este motor sabe producir. `oob` esta aqui aunque no salga de
/// `export`: la lista dice que ARTEFACTOS existen, no de que comando salen.
const EMISORES: &[&str] = &["odcs", "ossie", "cedar", "graphql", "oos", "json", "oob"];

fn ejecutar_emit(caso: &Case) -> Result<(), String> {
    if !EMISORES.contains(&caso.formato.as_str()) {
        return Err("no implementado".into());
    }
    let entrada = caso.dir.join("input");

    match &caso.expects {
        // La emisión DEBE fallar. Que falle no basta: tiene que fallar por lo
        // que el caso dice, y por eso se comprueba el motivo y no solo el
        // código de salida.
        Expects::EmitFails => {
            // El formato es el del caso, no `ossie` fijo: habia un solo caso
            // `emit-fails` cuando se escribio esto, y la constante se quedo.
            let (ok, texto) = exportar(&entrada, &caso.formato)?;
            if ok {
                return Err("emitió, y debía fallar".into());
            }
            // Que falle, y no por no estar implementado. NO se afirma el texto
            // del diagnostico: es la misma razon por la que `structure` afirma
            // propiedades y no cadenas — dos implementaciones pueden redactar
            // distinto y ser ambas correctas.
            if texto.contains("no implementad") {
                return Err("no implementado".into());
            }
            Ok(())
        }

        Expects::Roundtrip => match contrato_ajeno(&caso.dir) {
            // ODCS → OOS → ODCS. Se compara contra la forma canónica del
            // fichero original SIN interpretar: si el perfil perdiera o
            // renombrara algo, aparecería aquí.
            Some(contrato) => {
                let (ok, ida) = exportar(&contrato, "odcs")?;
                if !ok {
                    return Err(format!("no emite: {}", ida.lines().next().unwrap_or("")));
                }
                let (_, crudo) = exportar(&contrato, "json")?;
                if ida.trim() == crudo.trim() {
                    Ok(())
                } else {
                    Err("la ida y vuelta por ODCS pierde o añade algo".into())
                }
            }
            // OOS → ODCS → OOS. El intermedio va a un fichero temporal: la
            // composición es del arnés, no del binario. ORE no se examina.
            None => {
                let (ok, odcs) = exportar(&entrada, "odcs")?;
                if !ok {
                    return Err(format!("no emite: {}", odcs.lines().next().unwrap_or("")));
                }
                let tmp = std::env::temp_dir().join(format!("ore-rt-{}.odcs.json", caso.nombre));
                std::fs::write(&tmp, odcs.as_bytes()).map_err(|e| e.to_string())?;
                let (_, vuelta) = exportar(&tmp, "oos")?;
                let (_, original) = exportar(&entrada, "oos")?;
                let _ = std::fs::remove_file(&tmp);
                if vuelta.trim() == original.trim() {
                    Ok(())
                } else {
                    Err("la ida y vuelta por ODCS no es la identidad".into())
                }
            }
        },

        // Propiedades estructurales, no texto: dos implementaciones pueden
        // formatear el esquema distinto y ser ambas correctas.
        Expects::Structure => {
            let (ok, esquema) = exportar(&entrada, &caso.formato)?;
            if !ok {
                return Err(format!(
                    "no emite: {}",
                    esquema.lines().next().unwrap_or("")
                ));
            }
            let esperado = std::fs::read_to_string(caso.dir.join("expected.structure.json"))
                .map_err(|_| "falta expected.structure.json".to_string())?;
            comprobar_estructura(&esquema, &esperado)
        }

        otro => Err(format!("`{otro:?}` no encaja en emit")),
    }
}

/// Las afirmaciones de `expected.structure.json`, comprobadas contra el esquema.
/// Un bloque con una clave que este comprobador no entiende **no pasa**.
///
/// Ignorarla en silencio es lo que dejo cuatro casos de v1alpha5 en verde
/// afirmando cosas que nadie miraba: sus aserciones usaban `type`, `field` y
/// `scalar`, y aqui solo se leian `entity`, `members`, `actions` y `contains`.
/// Un comprobador que exceptua de mas tiene el mismo aspecto que uno que
/// funciona.
fn claves_conocidas(bloque: &str) -> Result<(), String> {
    // `description`, `mustContain` y `mustNotContain` son la envoltura del
    // fichero: caen en el primer bloque al partir por `{`, y no son aserciones.
    const CONOCIDAS: &[&str] = &[
        "entity",
        "in",
        "members",
        "actions",
        "contains",
        "principalOf",
        "attributes",
        "context",
        "attributesOn",
        "reason",
        "description",
        "mustContain",
        "mustNotContain",
    ];
    let mut resto = bloque;
    while let Some(i) = resto.find('"') {
        resto = &resto[i + 1..];
        let Some(j) = resto.find('"') else { break };
        let clave = &resto[..j];
        let tras = resto[j + 1..].trim_start();
        resto = &resto[j + 1..];
        // Solo lo que va seguido de `:` es una clave; lo demas son valores.
        if tras.starts_with(':') && !CONOCIDAS.contains(&clave) {
            return Err(format!(
                "el caso afirma `{clave}`, que este comprobador no sabe leer"
            ));
        }
    }
    Ok(())
}

fn comprobar_estructura(esquema: &str, esperado: &str) -> Result<(), String> {
    let (debe, no_debe) = esperado
        .split_once("\"mustNotContain\"")
        .unwrap_or((esperado, ""));

    for item in debe.split('{').skip(1) {
        let bloque = &item[..item.find('}').unwrap_or(item.len())];
        claves_conocidas(bloque)?;
        // `{ "entity": "X", "in": [...] }` — el tipo existe y, si se declaran
        // padres, los tiene.
        if let Some(tipo) = campo_json(bloque, "entity") {
            let ambito = objeto_en(esquema, &format!("/entityTypes/{tipo}"));
            if ambito.is_empty() {
                return Err(format!("falta el tipo `{tipo}`"));
            }
            for padre in lista_json(bloque, "in") {
                // `EntityType` no es un tipo: dice «algún tipo de entidad».
                let presente = if padre == "EntityType" {
                    ambito.matches('"').count() > 4
                } else {
                    ambito.contains(&format!("\"{padre}\""))
                };
                if !presente {
                    return Err(format!("`{tipo}` no es miembro de `{padre}`"));
                }
            }
        }
        for m in lista_json(bloque, "members") {
            if !esquema.contains(&format!("\"{m}\"")) {
                return Err(format!("falta la etiqueta `{m}`"));
            }
        }
        for a in lista_json(bloque, "actions") {
            if objeto_en(esquema, &format!("/actions/{a}")).is_empty() {
                return Err(format!("falta la acción `{a}`"));
            }
        }
        // `principalOf` — el tipo es principal de esas acciones. Sin esto la
        // jerarquía emitida no la puede usar nadie: `resource in principal`
        // sería un error de tipos.
        if let Some(tipo) = campo_json(bloque, "entity") {
            for a in lista_json(bloque, "principalOf") {
                let acc = objeto_en(esquema, &format!("/actions/{a}"));
                if !acc.contains(&format!("\"{tipo}\"")) {
                    return Err(format!("`{tipo}` no es principal de `{a}`"));
                }
            }
            let forma = objeto_en(esquema, &format!("/entityTypes/{tipo}/shape"));
            for at in lista_json(bloque, "attributes") {
                if !forma.contains(&format!("\"{at}\"")) {
                    return Err(format!("`{tipo}` no declara el atributo `{at}`"));
                }
            }
        }
        // `context` — cada acción lo declara. Se leía para OOS5015 y no se
        // declaraba en ninguna parte.
        for c in lista_json(bloque, "context") {
            for a in ["read", "aggregate", "export", "invoke"] {
                let ctx = objeto_en(esquema, &format!("/actions/{a}/appliesTo/context"));
                if !ctx.contains(&format!("\"{c}\"")) {
                    return Err(format!("`{a}` no declara `context.{c}`"));
                }
            }
        }
        // La forma genérica: una cadena que tiene que aparecer. Las tres de
        // arriba son de Cedar; esta sirve para cualquier formato, y es lo que
        // permite afirmar sobre un contrato ODCS sin escribir un comparador
        // por emisor.
        for c in lista_json(bloque, "contains") {
            if !esquema.contains(&c) {
                return Err(format!("falta `{c}` en la salida"));
            }
        }
    }

    for item in no_debe.split('{').skip(1) {
        let bloque = &item[..item.find('}').unwrap_or(item.len())];
        claves_conocidas(bloque)?;
        if let Some(tipo) = campo_json(bloque, "entity")
            && esquema.contains(&format!("\"{tipo}\""))
        {
            return Err(format!(
                "`{tipo}` no está declarado y aparece en el esquema"
            ));
        }
        for c in lista_json(bloque, "contains") {
            if esquema.contains(&c) {
                return Err(format!("`{c}` aparece y no debería"));
            }
        }
        // Un recurso descrito con atributos obligaría al motor a LEERLO para
        // autorizarlo, que es la lectura gobernada que decide su propio acceso.
        for t in lista_json(bloque, "attributesOn") {
            if !objeto_en(esquema, &format!("/entityTypes/{t}/shape")).is_empty() {
                return Err(format!("`{t}` declara atributos y no debería"));
            }
        }
    }
    Ok(())
}

/// Los elementos de un array de cadenas, por su clave.
fn lista_json(texto: &str, clave: &str) -> Vec<String> {
    let marca = format!("\"{clave}\"");
    let Some(i) = texto.find(&marca) else {
        return Vec::new();
    };
    let resto = &texto[i + marca.len()..];
    let Some(abre) = resto.find('[') else {
        return Vec::new();
    };
    let cierra = resto.find(']').unwrap_or(resto.len());
    if cierra < abre {
        return Vec::new();
    }
    resto[abre + 1..cierra]
        .split(',')
        .filter_map(|t| leer_cadena(t.trim()))
        .collect()
}

/// Una cadena JSON, respetando las comillas escapadas.
///
/// Hacía falta al añadir `contains`: afirmar sobre un contrato ODCS pide
/// escribir una comilla escapada dentro del JSON esperado, y la versión ingenua
/// —cortar en la siguiente comilla— devolvía una barra suelta. El caso fallaba
/// diciendo «falta `\`», que es un mensaje sobre el runner disfrazado de
/// diagnóstico sobre la salida.
///
/// No parte por comas dentro de la cadena: ninguna de las que se afirman las
/// lleva, y un analizador completo aquí sería el analizador de JSON que
/// `ore-core` decidió no tener.
fn leer_cadena(t: &str) -> Option<String> {
    let cuerpo = t.strip_prefix('"')?;
    let mut out = String::new();
    let mut cs = cuerpo.chars();
    while let Some(c) = cs.next() {
        match c {
            '\\' => out.push(cs.next()?),
            '"' => return Some(out),
            c => out.push(c),
        }
    }
    None
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

/// El marcador del borrador de v1alpha2, contado aparte.
///
/// Arranca en 0 y ese es el punto de partida, igual que `73` arrancó en 0. Lo
/// que este test protege mientras tanto es que **no haya regresiones**: un caso
/// del borrador que falle por algo que no sea «sin implementar» es un fallo real
/// aunque la especificación no sea normativa todavía.
/// Marcador de un borrador. Vive aparte del de v1alpha1 a proposito: un
/// numero que mezcla una especificacion cerrada con una en curso no informa
/// de nada, porque ya no se sabe que mide.
fn marcador(version: &str, familia: &str, titulo: &str) {
    let raiz = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../vendor/oos/conformance/{version}"))
        .canonicalize()
        .expect("submódulo sin inicializar");

    let casos = descubrir(&raiz);
    assert!(!casos.is_empty(), "el borrador de {version} está vacío");

    let mut verdes = 0usize;
    let mut regresiones: Vec<(String, String, String)> = Vec::new();
    for c in &casos {
        match ejecutar(c) {
            Ok(()) => verdes += 1,
            Err(m) if m == "no implementado" => {}
            Err(m) => regresiones.push((c.grupo.clone(), c.nombre.clone(), m)),
        }
    }

    let total = casos.len();
    let barra = "█".repeat(verdes * 20 / total) + &"░".repeat(20 - verdes * 20 / total);
    println!();
    // La caja se dimensiona al título en vez de fijarse a mano: un rótulo más
    // largo que el ancho no se sale del marco, que es lo que pasó al renombrar
    // este borrador.
    let ancho = titulo.chars().count().max(41) + 4;
    let borde: String = "─".repeat(ancho);
    println!("  ┌{borde}┐");
    println!("  │{:<ancho$}│", format!("  {titulo}"));
    println!("  └{borde}┘");
    println!();
    println!("    {familia:<11} {barra}  {verdes:>2} / {total:<2}");
    println!();
    println!("    No normativo. `spec/v1alpha1/` sigue mandando.");
    println!();

    if !regresiones.is_empty() {
        println!("    [31mRegresiones:[0m");
        for (g, n, m) in &regresiones {
            println!("      {g}/{n}: {m}");
        }
        println!();
    }
    assert!(regresiones.is_empty(), "{} regresiones", regresiones.len());
}

#[test]
fn borrador_de_v1alpha2() {
    marcador(
        "v1alpha2",
        "efectos",
        "BORRADOR · OOS v1alpha2 · efectos y derivación",
    );
}

/// El borrador de v1alpha3 arranca entero en pendiente: ningun `OOS8xxx` esta
/// implementado y los casos `accept` traen `apiVersion: oos.dev/v1alpha3`, que
/// esta implementacion todavia rechaza. Se limpia solo, caso a caso, segun se
/// implemente — que es exactamente lo que hizo el de v1alpha2.
#[test]
fn borrador_de_v1alpha3() {
    marcador("v1alpha3", "gobierno", "BORRADOR · OOS v1alpha3 · gobierno");
}

/// Los esquemas publicados tienen que ser JSON bien formado.
///
/// Parece obvio y no lo estaba: **nada lo comprobaba**. `ore-core` no lleva
/// analizador de JSON —`json.rs` solo emite, y esa es una decisión— así que un
/// esquema roto viajaba hasta el repositorio público sin que un solo test se
/// pusiera en rojo. Ocurrió: una descripción con un salto de línea sin escapar
/// se subió a `main` con la suite entera en verde.
///
/// Esto no es un analizador: es el escáner mínimo que atrapa esa clase de
/// fallo —un carácter de control dentro de una cadena, o una comilla sin
/// cerrar—, que es la que produce un fichero que no parsea en ninguna parte.
#[test]
fn borrador_de_v1alpha4() {
    marcador(
        "v1alpha4",
        "significado",
        "BORRADOR · OOS v1alpha4 · significado",
    );
}

/// El borrador de v1alpha5 arranco entero en pendiente —`--format graphql` no
/// existia— y hoy esta entero en verde: sus casos certifican los cuatro peldanos
/// de `01-emision-graphql` §6. Cuantos son lo dice el marcador al correr.
#[test]
fn borrador_de_v1alpha5() {
    marcador(
        "v1alpha5",
        "emisión",
        "BORRADOR · OOS v1alpha5 · emisión a GraphQL",
    );
}

/// El borrador de v1alpha6 no anade gramatica: anade un ARTEFACTO —el paquete
/// publicable— y un contrato con el programa que lo trae. Su suite empieza por
/// el formato, que es la mitad que no necesita un registro para existir.
#[test]
fn borrador_de_v1alpha6() {
    marcador(
        "v1alpha6",
        "distribución, firma y log",
        "BORRADOR · OOS v1alpha6 · distribución, firma y log",
    );
}

/// El borrador de v1alpha7 anade el primer `kind` desde v1alpha4 —la vista— y
/// es el primero que SUSTITUYE a uno: absorbe al `Binding`. Sus casos
/// certifican el primer peldano de `01-view` §9: la cadena compila y se niega.
/// Los otros dos peldanos son la migracion, y no son casos.
#[test]
fn borrador_de_v1alpha7() {
    marcador("v1alpha7", "la vista", "BORRADOR · OOS v1alpha7 · la vista");
}

/// El borrador de v1alpha8 anade `Table`, adelgaza `View` y retira `Binding`.
/// Arranca **entero en pendiente**, como arrancó el de v1alpha3: esta
/// implementacion no entiende todavia `oos.dev/v1alpha8`, asi que los dieciseis
/// casos fallan con `OOS1002` por la version y ninguno mide lo que afirma. Se
/// limpia solo, caso a caso, segun se implemente.
///
/// Los dos que valen dinero son `OOS2020` —lo que no se puede leer se debe
/// materializar— y `OOS2021` —sin retractacion no se mantiene lo mutable—:
/// ningun competidor los comprueba al compilar.
#[test]
fn borrador_de_v1alpha8() {
    marcador("v1alpha8", "la tabla", "BORRADOR · OOS v1alpha8 · la tabla");
}

#[test]
fn los_esquemas_publicados_son_json_bien_formado() {
    let raiz = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/oos/schemas")
        .canonicalize()
        .expect("submódulo sin inicializar");

    let mut ficheros = Vec::new();
    fn recorrer(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(es) = std::fs::read_dir(dir) else {
            return;
        };
        for e in es.flatten() {
            let p = e.path();
            if p.is_dir() {
                recorrer(&p, out);
            } else if p.extension().is_some_and(|x| x == "json") {
                out.push(p);
            }
        }
    }
    recorrer(&raiz, &mut ficheros);
    assert!(!ficheros.is_empty(), "sin esquemas que comprobar");

    let mut malos = Vec::new();
    for f in &ficheros {
        let t = std::fs::read_to_string(f).expect("esquema ilegible");
        let (mut en_cadena, mut escapado, mut linea) = (false, false, 1usize);
        for c in t.chars() {
            if c == '\n' {
                linea += 1;
            }
            match (en_cadena, escapado, c) {
                (true, true, _) => escapado = false,
                (true, false, '\\') => escapado = true,
                (true, false, '"') => en_cadena = false,
                (true, false, c) if (c as u32) < 0x20 => {
                    malos.push(format!(
                        "{}:{linea}: carácter de control en una cadena",
                        f.display()
                    ));
                    break;
                }
                (false, _, '"') => en_cadena = true,
                _ => {}
            }
        }
        if en_cadena {
            malos.push(format!("{}: cadena sin cerrar", f.display()));
        }
    }
    assert!(
        malos.is_empty(),
        "esquemas mal formados:\n  {}",
        malos.join("\n  ")
    );
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

    assert_eq!(casos.len(), 90, "número de casos inesperado");
    assert_eq!(por_grupo.get("invalid"), Some(&42));
    assert_eq!(por_grupo.get("diff"), Some(&22));
    assert_eq!(por_grupo.get("canonical"), Some(&9));
    assert_eq!(por_grupo.get("digest"), Some(&8));
    assert_eq!(por_grupo.get("emit"), Some(&5));
    assert_eq!(por_grupo.get("valid"), Some(&4));
}
