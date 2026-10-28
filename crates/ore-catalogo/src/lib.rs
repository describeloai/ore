//! **El catálogo transaccional de los punteros** (ADR 0079): la librería que lo mantiene y el
//! andamio que lo llena.
//!
//! | Módulo | Qué | Tras el corte |
//! |---|---|---|
//! | [`esquema`] | el esquema versionado | se queda |
//! | [`almacen`] | las transacciones del catálogo | se queda (ore-serve escribe con él) |
//! | [`cargar`] | **el andamio**: sigue a git y replica | se apaga |
//! | [`auditar`] | el catálogo frente a una referencia | se queda, como guardián |
//! | [`puerta`] | [`Catalogo`]: la puerta de los punteros sobre el catálogo (§19.2) | se queda |
//!
//! El binario tiene, de momento, el modo `seguir` (cargar + auditar), `auditar` suelto y
//! `punteros` (el protocolo de la puerta por stdin y stdout, para los Jobs, ·5d). El modo
//! `guardar` (auditar + reconciliar + relé) llega con E1a·5.

pub mod almacen;
pub mod auditar;
pub mod cargar;
pub mod esquema;
pub mod git;
pub mod puerta;

pub use puerta::Catalogo;

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use ore_base::json::Json;

const USO: &str = "\
ore-catalogo — el catálogo transaccional de los punteros (ADR 0079)

  ore-catalogo seguir  --esquema <t_x> --forja <url|ruta> --espejo <dir> [opciones]
  ore-catalogo auditar --esquema <t_x> --forja <url|ruta> --espejo <dir> [--testigo-fichero <f>]
  ore-catalogo punteros --esquema <t_x> [--sembrar]

  La base, por CATALOGO_URL (postgres://…). Sin ella no arranca.

  seguir:
    --testigo-fichero <f>  el token de la forja
    --cada <s>             cada cuánto mira la forja (1)
    --auditar-cada <s>     auditoría completa aunque nada cambie (300)
    --una-vez              una pasada y su auditoría, y sale (0 si todo cuadra)
    --verificar-pasos      tras cada commit replicado, main frente al árbol de ese commit
    --hasta-n <n>          parar tras n commits de main (el banco: una caída a propósito)

  punteros: sirve la puerta de los punteros (ore_core::punteros::protocolo) por stdin y
  stdout, una conexión por proceso, hasta que se cierre stdin. `abrir` elige la ref
  (sin ella, main); la raíz del árbol cuenta sólo por sus `resultados/`.
    --sembrar              la primera apertura de cada árbol lo siembra en main (las pruebas)
";

struct Args {
    esquema: String,
    forja: String,
    espejo: PathBuf,
    testigo: Option<String>,
    cada: Duration,
    auditar_cada: Duration,
    una_vez: bool,
    op: cargar::Opciones,
}

fn leer(args: &[String]) -> Result<Args, String> {
    let mut a = Args {
        esquema: String::new(),
        forja: String::new(),
        espejo: PathBuf::new(),
        testigo: None,
        cada: Duration::from_secs(1),
        auditar_cada: Duration::from_secs(300),
        una_vez: false,
        op: cargar::Opciones::default(),
    };
    let mut it = args.iter().skip(1);
    while let Some(k) = it.next() {
        let mut valor = || it.next().cloned().ok_or(format!("falta el valor de `{k}`"));
        let segundos = |v: String| {
            v.parse::<u64>()
                .map(Duration::from_secs)
                .map_err(|_| format!("`{k}`: `{v}` no son segundos"))
        };
        match k.as_str() {
            "--esquema" => a.esquema = valor()?,
            "--forja" => a.forja = valor()?,
            "--espejo" => a.espejo = PathBuf::from(valor()?),
            "--testigo-fichero" => {
                let f = valor()?;
                a.testigo = Some(std::fs::read_to_string(&f).map_err(|e| format!("{f}: {e}"))?);
            }
            "--cada" => a.cada = segundos(valor()?)?,
            "--auditar-cada" => a.auditar_cada = segundos(valor()?)?,
            "--una-vez" => a.una_vez = true,
            "--verificar-pasos" => a.op.verificar_pasos = true,
            "--hasta-n" => {
                let v = valor()?;
                a.op.hasta_n = Some(v.parse().map_err(|_| format!("`--hasta-n`: `{v}`"))?);
            }
            otro => return Err(format!("opción desconocida: `{otro}`")),
        }
    }
    if a.esquema.is_empty() || a.forja.is_empty() || a.espejo.as_os_str().is_empty() {
        return Err("faltan `--esquema`, `--forja` o `--espejo`".into());
    }
    Ok(a)
}

pub fn arrancar(args: Vec<String>) -> ExitCode {
    let Some(mando) = args.first().map(String::as_str) else {
        print!("{USO}");
        return ExitCode::from(64);
    };
    if matches!(mando, "-h" | "--help") {
        print!("{USO}");
        return ExitCode::SUCCESS;
    }
    let url = match std::env::var("CATALOGO_URL") {
        Ok(u) if !u.is_empty() => u,
        _ => {
            eprintln!("✗ falta `CATALOGO_URL`: sin ella no se sabe a qué catálogo se escribe.");
            return ExitCode::from(64);
        }
    };
    if mando == "punteros" {
        return punteros(&args, url);
    }
    let a = match leer(&args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("✗ {e}\n\n{USO}");
            return ExitCode::from(64);
        }
    };
    let preparar = || -> Result<(git::Espejo, almacen::Almacen), String> {
        Ok((
            git::Espejo::abrir(&a.espejo, &a.forja, a.testigo.as_deref())?,
            almacen::Almacen::conectar(&url, &a.esquema)?,
        ))
    };
    let (e, mut alm) = match preparar() {
        Ok(x) => x,
        Err(m) => {
            eprintln!("✗ {m}");
            return ExitCode::from(2);
        }
    };
    match mando {
        "seguir" => seguir(&a, &e, &mut alm),
        "auditar" => match auditoria(&e, &mut alm) {
            Ok((n, difs)) => {
                println!("{}", informe_de_auditoria(n, &difs).jcs());
                if difs.is_empty() {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::from(1)
                }
            }
            Err(m) => {
                eprintln!("✗ {m}");
                ExitCode::from(2)
            }
        },
        otro => {
            eprintln!("✗ mando desconocido: `{otro}`\n\n{USO}");
            ExitCode::from(64)
        }
    }
}

/// `--esquema` y `--sembrar` del modo `punteros`.
fn leer_punteros(args: &[String]) -> Result<(String, bool), String> {
    let (mut esquema, mut sembrar) = (String::new(), false);
    let mut it = args.iter().skip(1);
    while let Some(k) = it.next() {
        match k.as_str() {
            "--esquema" => esquema = it.next().cloned().ok_or("falta el valor de `--esquema`")?,
            "--sembrar" => sembrar = true,
            otro => return Err(format!("opción desconocida: `{otro}`")),
        }
    }
    if !almacen::esquema_valido(&esquema) {
        return Err(format!("falta `--esquema` o no es válido: `{esquema}`"));
    }
    Ok((esquema, sembrar))
}

/// **`ore-catalogo punteros`**: el protocolo, por stdin y stdout. No conecta hasta el `abrir`: un
/// fallo al conectar vuelve como un 503 por el protocolo, no como un proceso muerto.
fn punteros(args: &[String], url: String) -> ExitCode {
    let (esquema, sembrar) = match leer_punteros(args) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("✗ {e}\n\n{USO}");
            return ExitCode::from(64);
        }
    };
    let mut s = puerta::servidor(url, esquema, sembrar);
    let entrada = std::io::stdin().lock();
    let salida = std::io::stdout().lock();
    match ore_core::punteros::protocolo::servir(entrada, salida, &mut s) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("✗ punteros: {e}");
            ExitCode::from(2)
        }
    }
}

fn auditoria(
    e: &git::Espejo,
    a: &mut almacen::Almacen,
) -> Result<(usize, Vec<almacen::Diferencia>), String> {
    e.traer()?;
    auditar::auditar(e, a)
}

fn informe_de_auditoria(n: usize, difs: &[almacen::Diferencia]) -> Json {
    Json::obj([
        ("comparados", Json::Int(n as i64)),
        ("diferencias", Json::Int(difs.len() as i64)),
        (
            "primeras",
            Json::Arr(
                difs.iter()
                    .take(20)
                    .map(|d| {
                        Json::s(format!(
                            "{} · {} · {} {}",
                            d.rama, d.activo, d.motivo, d.campos
                        ))
                    })
                    .collect(),
            ),
        ),
    ])
}

/// La línea de salud de una pasada (y de su auditoría, si la hubo): lo que leerá `GET /salud`.
fn salud(
    p: &cargar::Pasada,
    ms: u128,
    aud: Option<&(usize, Vec<almacen::Diferencia>)>,
    cuentas: [i64; 4],
) -> Json {
    let mut campos = vec![
        ("main", Json::s(p.main.get(..12).unwrap_or(&p.main))),
        ("traida", Json::Bool(p.traida)),
        ("commits", Json::Int(p.commits as i64)),
        ("con_cambios", Json::Int(p.con_cambios as i64)),
        ("pasos_verificados", Json::Int(p.pasos_verificados as i64)),
        (
            "pasos_mal",
            Json::Arr(p.pasos_mal.iter().take(10).map(Json::s).collect()),
        ),
        ("ramas_replicadas", Json::Int(p.ramas_replicadas as i64)),
        ("ramas_borradas", Json::Int(p.ramas_borradas as i64)),
        ("cortada", Json::Bool(p.cortada)),
        ("pasada_ms", Json::Int(ms as i64)),
        (
            "catalogo",
            Json::obj([
                ("commits", Json::Int(cuentas[0])),
                ("punteros_main", Json::Int(cuentas[1])),
                ("historia", Json::Int(cuentas[2])),
                ("eventos", Json::Int(cuentas[3])),
            ]),
        ),
    ];
    if let Some((n, difs)) = aud {
        campos.push(("auditoria", informe_de_auditoria(*n, difs)));
    }
    Json::obj(campos)
}

fn seguir(a: &Args, e: &git::Espejo, alm: &mut almacen::Almacen) -> ExitCode {
    let mut ultima: Option<Instant> = None;
    loop {
        let t = Instant::now();
        match cargar::pasada(e, alm, &a.op) {
            Ok(p) => {
                let ms = t.elapsed().as_millis();
                let movio = p.commits > 0 || p.ramas_replicadas > 0 || p.ramas_borradas > 0;
                let toca = !p.cortada
                    && (a.una_vez || movio || ultima.is_none_or(|u| u.elapsed() >= a.auditar_cada));
                let aud = if toca {
                    ultima = Some(Instant::now());
                    match auditar::auditar(e, alm) {
                        Ok(x) => Some(x),
                        Err(m) => {
                            eprintln!("✗ auditoría: {m}");
                            None
                        }
                    }
                } else {
                    None
                };
                if a.una_vez || movio || aud.is_some() {
                    let cuentas = alm.cuentas().unwrap_or_default();
                    println!("{}", salud(&p, ms, aud.as_ref(), cuentas).jcs());
                }
                if a.una_vez {
                    let bien = p.pasos_mal.is_empty()
                        && aud.is_none_or(|(_, d)| d.is_empty())
                        && (p.cortada || toca);
                    return if bien {
                        ExitCode::SUCCESS
                    } else {
                        ExitCode::from(1)
                    };
                }
            }
            Err(m) => {
                eprintln!("✗ pasada: {m}");
                if a.una_vez {
                    return ExitCode::from(2);
                }
            }
        }
        std::thread::sleep(a.cada);
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn v(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn los_argumentos() {
        let a = leer(&v(
            "seguir --esquema t_victor --forja /f.git --espejo /e --una-vez --hasta-n 40 --cada 2",
        ))
        .unwrap();
        assert_eq!(a.esquema, "t_victor");
        assert!(a.una_vez);
        assert_eq!(a.op.hasta_n, Some(40));
        assert_eq!(a.cada, Duration::from_secs(2));
        assert!(leer(&v("seguir --esquema t_victor")).is_err());
        assert!(leer(&v("seguir --esquema t --forja f --espejo e --nada")).is_err());
        assert!(leer(&v("seguir --esquema t --forja f --espejo e --cada x")).is_err());
    }

    #[test]
    fn los_argumentos_de_punteros() {
        assert_eq!(
            leer_punteros(&v("punteros --esquema t_victor")).unwrap(),
            ("t_victor".to_string(), false)
        );
        assert!(
            leer_punteros(&v("punteros --esquema t_x --sembrar"))
                .unwrap()
                .1
        );
        assert!(leer_punteros(&v("punteros")).is_err());
        assert!(leer_punteros(&v("punteros --esquema T;x")).is_err());
        assert!(leer_punteros(&v("punteros --esquema t_x --nada")).is_err());
    }
}
