//! **La vara** (0081 §7.4, propiedad 9; S4): el corpus `sqllogictest` de DataFusion, en la versión exacta del
//! árbol, sobre la sesión de ORE. Si pasa, lo que el suelo cambia (los desvíos, la raíz de memoria, el derrame)
//! no rompe nada de lo que DataFusion promete.
//!
//! - **Qué se prueba** (D-S9): la configuración y el `RuntimeEnv` de un `Motor` de papel `Prueba`. Las tablas y
//!   funciones que cada fichero espera las pone la preparación del upstream (`TestContext`), y aquí se le
//!   cambian la configuración y el entorno por los de ORE. La política de la `Sesion` (sin DDL ni `SET`) no
//!   entra: el corpus es DDL de arriba abajo, y esa política tiene sus pruebas en `tests/suelo.rs`.
//! - **Qué corpus** (D-S11): `test_files/` de apache/datafusion en el tag, con arrow-testing y
//!   parquet-testing (`ci/corpus-slt.sh`). Fuera, como en el upstream por defecto, tpch, sqlite y el Parquet
//!   cifrado; y spark, que cambia el dialecto.
//! - **El bucle** es el de `run_test_file` y `run_file_in_runner` del binario `sqllogictests` del upstream
//!   (Apache-2.0, apache/datafusion 55.2.0, `datafusion/sqllogictest/bin/sqllogictests.rs`), registro a registro
//!   para poder exceptuar uno.
//! - **Las excepciones** (D-S12) están en `excepciones-de-la-vara.txt`, cada una con su categoría y su porqué. Una
//!   que ya no hace falta **es un fallo**: la lista no puede crecer en silencio ni quedarse vieja.
//!
//! ```text
//! ci/corpus-slt.sh <dir>
//! ORE_CORPUS_SLT=<dir> cargo test -p ore-runtime --test vara [-- <filtro>...]
//! ```
//!
//! Sin `ORE_CORPUS_SLT`, no corre: la suite normal no depende de la red.

use datafusion::execution::SessionStateBuilder;
use datafusion::execution::runtime_env::RuntimeEnv;
use datafusion::prelude::{SessionConfig, SessionContext};
use datafusion_sqllogictest::{
    DataFusion, Filter, TestContext, TestFile, df_value_validator, is_spark_path,
    read_dir_recursive, setup_scratch_dir, should_skip_file, should_skip_record, value_normalizer,
};
use futures::StreamExt;
use indicatif::ProgressBar;
use ore_runtime::{Motor, OreOpciones, Papel, Zocalos};
use sqllogictest::{Normalizer, Record, parse_file, strict_column_validator};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::str::FromStr;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

/// Dónde está el corpus: el directorio que se le dio a `ci/corpus-slt.sh`.
const CORPUS: &str = "ORE_CORPUS_SLT";
/// Si se da, el informe entero (cada fallo con su mensaje) se escribe ahí: para el triaje.
const INFORME: &str = "ORE_VARA_INFORME";
/// Los ficheros, relativos al directorio del crate `sqllogictest`, que es desde donde citan sus datos.
const FICHEROS: &str = "test_files/";
/// La raíz del corpus tal como sale en los planes (una ruta de `object_store`, sin la `/` del principio).
static RAIZ: OnceLock<String> = OnceLock::new();

/// El validador del upstream sobre lo que sale con la raíz del corpus cambiada por `WORKSPACE_ROOT`. Los
/// planes del corpus nombran así los ficheros, y el upstream la calcula desde donde se compiló su crate, que
/// aquí es el registro de cargo y no el corpus. Su normalizador sólo toca lo esperado; esto, lo que sale.
fn validar(normalizador: Normalizer, sale: &[Vec<String>], se_espera: &[String]) -> bool {
    let raiz = RAIZ.get().map(String::as_str).unwrap_or_default();
    let sale: Vec<Vec<String>> = sale
        .iter()
        .map(|fila| {
            fila.iter()
                .map(|v| v.replace(raiz, "WORKSPACE_ROOT"))
                .collect()
        })
        .collect();
    df_value_validator(normalizador, &sale, se_espera)
}

/// Las categorías de una excepción.
const CATEGORIAS: &[&str] = &["desvio", "entorno", "divergencia"];

/// Una excepción: un registro (o un fichero entero) que se sabe que falla, y por qué.
#[derive(Debug)]
struct Excepcion {
    fichero: String,
    linea: Option<u32>,
    categoria: String,
    porque: String,
}

/// `fichero.slt[:línea] | categoría | porqué`, una por línea; `#` comenta.
fn excepciones() -> Vec<Excepcion> {
    include_str!("excepciones-de-la-vara.txt")
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            let partes: Vec<&str> = l.splitn(3, '|').map(str::trim).collect();
            assert!(partes.len() == 3, "excepción mal escrita: `{l}`");
            let (fichero, linea) = match partes[0].rsplit_once(':') {
                Some((f, n)) => (f, Some(n.parse().expect("la línea es un número"))),
                None => (partes[0], None),
            };
            assert!(
                CATEGORIAS.contains(&partes[1]),
                "`{l}`: la categoría es una de {CATEGORIAS:?}"
            );
            assert!(!partes[2].is_empty(), "`{l}`: sin porqué");
            Excepcion {
                fichero: fichero.to_string(),
                linea,
                categoria: partes[1].to_string(),
                porque: partes[2].to_string(),
            }
        })
        .collect()
}

/// Lo que dio un fichero.
#[derive(Debug, Default)]
struct Informe {
    registros: usize,
    /// Los registros que fallaron: su línea y el error.
    fallos: Vec<(u32, String)>,
    /// Lo que falló del fichero y no de un registro: prepararlo, leerlo, o que dejara la configuración cambiada.
    del_fichero: Option<String>,
}

fn linea<T: sqllogictest::ColumnType>(r: &Record<T>) -> u32 {
    match r {
        Record::Statement { loc, .. } | Record::Query { loc, .. } | Record::System { loc, .. } => {
            loc.line()
        }
        _ => 0,
    }
}

/// Corre un fichero sobre la configuración y el entorno de ORE.
async fn correr(
    fichero: TestFile,
    config: SessionConfig,
    entorno: Arc<RuntimeEnv>,
) -> Option<Informe> {
    let TestFile {
        path,
        relative_path,
    } = fichero;
    // La preparación del upstream para este fichero: sus tablas, sus funciones, su planificador de tipos.
    // `None`: necesita una feature que no traemos, y se salta como en el upstream.
    let preparado = TestContext::try_new_for_test_file(&relative_path).await?;
    // Sin crear el catálogo por defecto al construir: `new_from_existing` lo apaga porque ya existe, y volver a
    // crearlo pisaría el que tiene las tablas del fichero (medido en S4·3). Construido, la opción vuelve a su
    // valor: el fichero la ve, en `SHOW ALL` y al hacer `RESET`.
    let mut estado = SessionStateBuilder::new_from_existing(preparado.session_ctx().state())
        .with_config(config.with_create_default_catalog_and_schema(false))
        .with_runtime_env(entorno)
        .build();
    estado
        .config_mut()
        .options_mut()
        .catalog
        .create_default_catalog_and_schema = true;
    let ctx = SessionContext::new_with_state(estado);
    let mut informe = Informe::default();
    if let Err(e) = setup_scratch_dir(&relative_path) {
        informe.del_fichero = Some(format!("el directorio de trabajo: {e}"));
        return Some(informe);
    }
    let registros = match path
        .canonicalize()
        .map_err(|e| e.to_string())
        .and_then(|p| parse_file(&p).map_err(|e| e.to_string()))
    {
        Ok(r) => r,
        Err(e) => {
            informe.del_fichero = Some(format!("leerlo: {e}"));
            return Some(informe);
        }
    };
    let cambios = Arc::new(Mutex::new(Vec::new()));
    let mut runner = sqllogictest::Runner::new(|| async {
        Ok(
            DataFusion::new(ctx.clone(), relative_path.clone(), ProgressBar::hidden())
                .with_config_change_errors(Arc::clone(&cambios)),
        )
    });
    runner.add_label("Datafusion");
    runner.with_column_validator(strict_column_validator);
    runner.with_normalizer(value_normalizer);
    runner.with_validator(validar);
    for registro in registros {
        if let Record::Halt { .. } = registro {
            break;
        }
        if should_skip_record::<DataFusion>(&registro, &[]) {
            continue;
        }
        let n = linea(&registro);
        informe.registros += 1;
        if let Err(e) = runner.run_async(registro).await {
            informe.fallos.push((n, e.to_string()));
        }
    }
    runner.shutdown_async().await;
    let cambios = cambios.lock().unwrap();
    if !cambios.is_empty() {
        informe.del_fichero = Some(cambios.join("\n"));
    }
    Some(informe)
}

/// Los ficheros del corpus que corren: los de `test_files/` menos lo que deja fuera D-S11, y los filtros dados.
fn ficheros(filtros: &[Filter]) -> Vec<TestFile> {
    let mut f: Vec<TestFile> = read_dir_recursive(FICHEROS)
        .expect("el corpus")
        .into_iter()
        .map(|p| TestFile::new(p, &[FICHEROS]))
        .filter(|f| f.is_slt_file())
        .filter(|f| !should_skip_file(&f.path, filtros))
        .filter(|f| !f.relative_path_starts_with("tpch"))
        .filter(|f| !f.relative_path_starts_with("sqlite"))
        .filter(|f| !f.relative_path_starts_with("encrypted_parquet.slt"))
        .filter(|f| !is_spark_path(&f.relative_path))
        .collect();
    f.sort_unstable();
    f
}

fn main() -> ExitCode {
    let Some(corpus) = std::env::var_os(CORPUS) else {
        eprintln!("vara: sin `{CORPUS}`, no corre (ci/corpus-slt.sh <dir>; {CORPUS}=<dir>)");
        return ExitCode::SUCCESS;
    };
    let excepciones = excepciones();
    let filtros: Vec<Filter> = std::env::args()
        .skip(1)
        .filter(|a| !a.starts_with('-'))
        .map(|a| Filter::from_str(&a).expect("un filtro"))
        .collect();
    let raiz = Path::new(&corpus)
        .join("datafusion")
        .canonicalize()
        .expect("el corpus: ci/corpus-slt.sh <dir>");
    let _ = RAIZ.set(
        raiz.display()
            .to_string()
            .trim_start_matches('/')
            .to_string(),
    );
    let dir = raiz.join("datafusion/sqllogictest");
    std::env::set_current_dir(&dir)
        .unwrap_or_else(|e| panic!("vara: el corpus en `{}`: {e}", dir.display()));

    let motor = Motor::arrancar(Papel::Prueba, OreOpciones::default(), &Zocalos::default())
        .expect("el motor de prueba");
    let (config, entorno) = (motor.config().clone(), motor.entorno().clone());
    let ficheros = ficheros(&filtros);
    let hilos = std::thread::available_parallelism().map_or(4, |n| n.get());
    let inicio = Instant::now();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("el runtime de la vara");
    let informes: BTreeMap<String, Option<Informe>> = runtime.block_on(
        futures::stream::iter(ficheros)
            .map(|f| {
                let nombre = f.relative_path.display().to_string();
                let tarea = tokio::spawn(correr(f, config.clone(), entorno.clone()));
                async move {
                    let informe = match tarea.await {
                        Ok(i) => i,
                        Err(e) => Some(Informe {
                            del_fichero: Some(format!("la tarea cayó: {e}")),
                            ..Informe::default()
                        }),
                    };
                    (nombre, informe)
                }
            })
            .buffer_unordered(hilos)
            .collect(),
    );
    let tiempo = inicio.elapsed();
    drop(runtime);
    drop(motor);

    // ── Contrastar con las excepciones ──────────────────────────────────────────────────────────────────
    let exceptuada = |fichero: &str, linea: Option<u32>| {
        excepciones
            .iter()
            .find(|e| e.fichero == fichero && (e.linea.is_none() || e.linea == linea))
    };
    let mut nuevos: Vec<String> = Vec::new();
    let mut por_categoria: BTreeMap<&str, usize> = BTreeMap::new();
    let mut usadas: BTreeSet<(String, Option<u32>)> = BTreeSet::new();
    let mut detalle = String::new();
    let (mut corridos, mut saltados, mut registros, mut verdes) = (0, 0, 0, 0);
    for (fichero, informe) in &informes {
        let Some(i) = informe else {
            saltados += 1;
            continue;
        };
        corridos += 1;
        registros += i.registros;
        if i.fallos.is_empty() && i.del_fichero.is_none() {
            verdes += 1;
        }
        let fallos = i
            .fallos
            .iter()
            .map(|(n, m)| (Some(*n), m.as_str()))
            .chain(i.del_fichero.iter().map(|m| (None, m.as_str())));
        for (n, mensaje) in fallos {
            let donde = match n {
                Some(n) => format!("{fichero}:{n}"),
                None => fichero.clone(),
            };
            detalle.push_str(&format!("── {donde}\n{mensaje}\n\n"));
            match exceptuada(fichero, n) {
                Some(e) => {
                    *por_categoria.entry(e.categoria.as_str()).or_default() += 1;
                    usadas.insert((e.fichero.clone(), e.linea));
                }
                None => nuevos.push(format!(
                    "{donde}: {}",
                    mensaje.lines().take(6).collect::<Vec<_>>().join("\n    ")
                )),
            }
        }
    }
    // Una excepción de un fichero que corrió y no se usó ya no hace falta.
    let caducadas: Vec<&Excepcion> = excepciones
        .iter()
        .filter(|e| matches!(informes.get(&e.fichero), Some(Some(_))))
        .filter(|e| !usadas.contains(&(e.fichero.clone(), e.linea)))
        .collect();

    if let Some(ruta) = std::env::var_os(INFORME) {
        std::fs::write(PathBuf::from(&ruta), &detalle).expect("el informe");
    }
    println!(
        "vara: {corridos} ficheros corridos ({verdes} en verde), {saltados} saltados por el upstream, \
         {registros} registros, en {:.1} s",
        tiempo.as_secs_f64()
    );
    for (c, n) in &por_categoria {
        println!("  exceptuados por {c}: {n}");
    }
    for n in &nuevos {
        println!("FALLO {n}");
    }
    for e in &caducadas {
        println!(
            "CADUCADA {}{} ({}: {}): ya pasa; quítala de la lista",
            e.fichero,
            e.linea.map(|n| format!(":{n}")).unwrap_or_default(),
            e.categoria,
            e.porque
        );
    }
    if nuevos.is_empty() && caducadas.is_empty() {
        println!("vara: en verde");
        ExitCode::SUCCESS
    } else {
        println!(
            "vara: {} fallos sin excepción y {} excepciones caducadas",
            nuevos.len(),
            caducadas.len()
        );
        ExitCode::FAILURE
    }
}
