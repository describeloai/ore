//! `ore-exec` — el ejecutor L2, invocado como programa.
//!
//! Dos verbos hoy:
//!
//! ```text
//! ore-exec validar <ruta>
//! ore-exec plan    <ruta> --entidad hr.Employee --props a,b --claves k1 \
//!                         --sujeto emp-42 --roles hr_analyst --claims k=v \
//!                         --purpose compensation_review \
//!                         --emisor https://… --audiencia ore
//! ```
//!
//! Sin `clap`: este binario vive fuera del compilador y no hay razón para que
//! su árbol crezca más de lo que ya crece por el evaluador. Las banderas se leen
//! a mano porque son ocho y no van a ser ochenta.

use ore_exec::{Consulta, Identidad, Motor, Rechazo};
use std::collections::BTreeMap;

fn valor(args: &[String], bandera: &str) -> Option<String> {
    let i = args.iter().position(|a| a == bandera)?;
    args.get(i + 1).cloned()
}

fn lista(args: &[String], bandera: &str) -> Vec<String> {
    valor(args, bandera)
        .map(|v| {
            v.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // `index` lleva subverbo, así que la ruta va un hueco más allá. Se lee aquí
    // y no dentro para que el motor se cargue una sola vez.
    let verbo = args.first().cloned().unwrap_or_default();
    let ruta = if verbo == "index" { args.get(2) } else { args.get(1) };
    let Some(ruta) = ruta else {
        eprintln!("uso: ore-exec <validar|plan> <ruta> [banderas]");
        eprintln!("     ore-exec index <build|traverse> <ruta> [banderas]");
        return std::process::ExitCode::from(64); // EX_USAGE
    };

    let mut motor = match Motor::cargar(std::path::Path::new(ruta)) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{e:?}");
            return std::process::ExitCode::from(65); // EX_DATAERR
        }
    };

    // El índice, si se pide. Se carga ANTES de nada porque su rechazo —«es de
    // otro bundle»— no es una condición de consulta: es que no se puede usar.
    if let Some(t) = valor(&args, "--indice")
        && let Err(e) = motor.cargar_topologia(std::path::Path::new(&t))
    {
        eprintln!("índice rechazado · {e}");
        return std::process::ExitCode::from(65);
    }

    match verbo.as_str() {
        "validar" => {
            let errores = motor.validar();
            let avisos = motor.avisos();
            for e in &errores {
                println!("error: {e}");
            }
            for a in &avisos {
                println!("aviso: {a}");
            }
            println!("\n{} errores, {} avisos", errores.len(), avisos.len());
            if errores.is_empty() {
                std::process::ExitCode::SUCCESS
            } else {
                std::process::ExitCode::from(65)
            }
        }
        "plan" => {
            let claims: BTreeMap<String, String> = lista(&args, "--claims")
                .iter()
                .filter_map(|kv| kv.split_once('=').map(|(k, v)| (k.into(), v.into())))
                .collect();
            let c = Consulta {
                quien: Identidad {
                    emisor: valor(&args, "--emisor").unwrap_or_default(),
                    audiencia: valor(&args, "--audiencia").unwrap_or_default(),
                    sujeto: valor(&args, "--sujeto").unwrap_or_default(),
                    roles: lista(&args, "--roles"),
                    claims,
                },
                accion: valor(&args, "--accion").unwrap_or_else(|| "read".into()),
                purpose: valor(&args, "--purpose").unwrap_or_default(),
                entidad: valor(&args, "--entidad").unwrap_or_default(),
                propiedades: lista(&args, "--props"),
                claves: lista(&args, "--claves").into_iter().map(|k| vec![k]).collect(),
                travesia: None,
            };

            match motor.planificar(&c) {
                Err(r) => {
                    // Una condición de tiempo de consulta NO es un código de
                    // documento: es un rechazo con nombre (`05-ejecutor` §9).
                    eprintln!("{}", rechazo(&r));
                    std::process::ExitCode::from(77) // EX_NOPERM
                }
                Ok(p) => {
                    if args.iter().any(|a| a == "--json") {
                        println!("{}", p.canonico());
                        return std::process::ExitCode::SUCCESS;
                    }
                    println!("① AUTORIZAR");
                    for (prop, aplicar) in &p.autorizadas {
                        println!("   ✓ {prop}{}", si_hay(aplicar));
                    }
                    for (prop, porque) in &p.podadas {
                        println!("   ✗ {prop} — {porque}");
                    }
                    println!("\n② TRAVESÍA");
                    if p.claves.is_empty() {
                        println!("   sin claves: el índice de topología es de M3");
                    } else {
                        println!("   {} clave(s)", p.claves.len());
                    }
                    println!("\n③ CARGA ÚTIL");
                    for l in &p.lecturas {
                        println!("   {} · {}", l.datasource, l.objeto);
                        if !l.clave_columnas.is_empty() {
                            println!("      clave: {}", l.clave_columnas.join(", "));
                        }
                        for (prop, col) in &l.proyeccion {
                            println!("      {prop} ← {col}");
                        }
                        for f in &l.filtros {
                            println!("      filtro: {} = {:?}  ({})", f.columna, f.valor, f.ambito);
                        }
                    }
                    println!("\n④ ENSAMBLAR");
                    if p.ensamblar_por.is_empty() {
                        println!("   una sola lectura: nada que ensamblar");
                    } else {
                        println!("   por {}", p.ensamblar_por.join(", "));
                    }
                    std::process::ExitCode::SUCCESS
                }
            }
        }
        "index" => indice(&motor, &args),
        otro => {
            eprintln!("`{otro}` no es un verbo de `ore-exec`");
            std::process::ExitCode::from(64)
        }
    }
}

fn si_hay(v: &[String]) -> String {
    if v.is_empty() {
        String::new()
    } else {
        format!("  [{}]", v.join(" · "))
    }
}

fn rechazo(r: &Rechazo) -> String {
    match r {
        Rechazo::PeticionInvalida(m) => {
            format!("petición inválida · {m}\n  no es una denegación: es una petición que no existe")
        }
        Rechazo::NoAutorizado { porque } => format!("no autorizado · {}", porque.join(" · ")),
        Rechazo::PlanRechazado {
            binding,
            campo,
            porque,
        } => format!("plan rechazado · `{binding}` · {campo}\n  {porque}"),
        Rechazo::SinBinding { propiedad } => {
            format!("sin binding · `{propiedad}` no la mapea ninguno: no hay de dónde leerla")
        }
        Rechazo::TravesiaNoDisponible { relacion } => format!(
            "travesía no disponible · `{relacion}`\n  no hay índice de topología cargado:              no es que no haya vecinos, es que no se pudo mirar"
        ),
    }
}

// ── El índice de topología ──────────────────────────────────────────────────

/// `index build` y `index traverse`.
///
/// **Dos actos, y se piden por separado porque fallan por separado**: leer las
/// aristas de una fuente necesita credenciales y un driver; construir el índice
/// con ellas es puro. Es la misma figura que `ore discover --source` frente a
/// `--from`, y lo que permite probar el determinismo sin una base de datos.
fn indice(motor: &Motor, args: &[String]) -> std::process::ExitCode {
    match args.get(1).map(String::as_str) {
        Some("build") => {
            let aristas = match valor(args, "--from") {
                Some(f) => match std::fs::read_to_string(&f) {
                    Ok(t) => leer_aristas(&t),
                    Err(e) => {
                        eprintln!("no se pudo leer `{f}`: {e}");
                        return std::process::ExitCode::from(66);
                    }
                },
                None => match aristas_de_la_fuente(motor, args) {
                    Ok(a) => a,
                    Err(e) => {
                        eprintln!("{e}");
                        return std::process::ExitCode::FAILURE;
                    }
                },
            };
            // La marca de agua la pone quien construye: el motor no lee el
            // reloj, y un índice que se fechara a sí mismo dejaría de ser
            // reproducible byte a byte.
            let marca = valor(args, "--marca").unwrap_or_default();
            let t = ore_exec::Topologia::construir(
                &ore_core::digest::bundle(&motor.paquete),
                &marca,
                &aristas,
            );
            let salida = valor(args, "-o").unwrap_or_else(|| "topologia.bin".into());
            match std::fs::write(&salida, t.bytes()) {
                Ok(()) => {
                    eprintln!(
                        "{} arista(s), {} relacion(es) -> {salida}",
                        aristas.len(),
                        t.relaciones().len()
                    );
                    std::process::ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("no se pudo escribir `{salida}`: {e}");
                    std::process::ExitCode::FAILURE
                }
            }
        }
        Some("traverse") => {
            let Some(t) = &motor.topologia else {
                eprintln!("travesia no disponible · falta `--indice <fichero>`");
                return std::process::ExitCode::from(77);
            };
            let rel = valor(args, "--relacion").unwrap_or_default();
            let desde = valor(args, "--desde").unwrap_or_default();
            let saltos: usize = valor(args, "--saltos")
                .and_then(|s| s.parse().ok())
                .unwrap_or(1);
            for k in t.travesia(&rel, &desde, saltos) {
                println!("{k}");
            }
            std::process::ExitCode::SUCCESS
        }
        _ => {
            eprintln!("uso: ore-exec index <build|traverse> <ruta> [banderas]");
            std::process::ExitCode::from(64)
        }
    }
}

/// Las aristas en NDJSON, una por linea.
fn leer_aristas(texto: &str) -> Vec<ore_exec::Arista> {
    texto
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| {
            let n = ore_core::parse::parse(l).ok()?;
            Some((
                n.get("relacion")?.1.as_str()?.to_string(),
                n.get("desde")?.1.as_str()?.to_string(),
                n.get("hasta")?.1.as_str()?.to_string(),
            ))
        })
        .collect()
}

/// Lee las aristas **de las fuentes declaradas**, por el driver.
///
/// Una relacion con `via` es una arista: la clave de la entidad apunta a la
/// clave del destino a traves de una columna. Asi que leerlas es una lectura por
/// el mismo protocolo de la fase ③ — **el driver no se entera de que esto es un
/// indice**, y esa es la prueba de que el protocolo era el correcto.
fn aristas_de_la_fuente(motor: &Motor, args: &[String]) -> Result<Vec<ore_exec::Arista>, String> {
    let Some(env) = valor(args, "--url-env") else {
        return Err("falta `--url-env <VARIABLE>`: la credencial no se adivina".into());
    };
    let url = std::env::var(&env).map_err(|_| format!("la variable `{env}` no esta puesta"))?;
    let tipo = valor(args, "--tipo").unwrap_or_else(|| "postgres".into());

    let mut out = Vec::new();
    for (relacion, lectura) in motor.lecturas_de_aristas() {
        let salida = std::process::Command::new(format!("ore-read-{tipo}"))
            .args(["leer", "aristas"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut h| {
                use std::io::Write as _;
                h.stdin
                    .as_mut()
                    .expect("stdin")
                    .write_all(lectura.peticion(&url).as_bytes())?;
                h.wait_with_output()
            })
            .map_err(|e| format!("no se pudo invocar `ore-read-{tipo}`: {e}"))?;
        if !salida.status.success() {
            return Err(String::from_utf8_lossy(&salida.stderr).to_string());
        }
        for linea in String::from_utf8_lossy(&salida.stdout)
            .lines()
            .filter(|l| !l.trim().is_empty())
        {
            let Ok(n) = ore_core::parse::parse(linea) else {
                continue;
            };
            let desde = n.get("desde").and_then(|(_, v)| v.as_str()).unwrap_or("");
            let hasta = n.get("hasta").and_then(|(_, v)| v.as_str()).unwrap_or("");
            if !desde.is_empty() && !hasta.is_empty() {
                out.push((relacion.clone(), desde.to_string(), hasta.to_string()));
            }
        }
    }
    Ok(out)
}
