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
    let (Some(verbo), Some(ruta)) = (args.first(), args.get(1)) else {
        eprintln!("uso: ore-exec <validar|plan> <ruta> [banderas]");
        return std::process::ExitCode::from(64); // EX_USAGE
    };

    let motor = match Motor::cargar(std::path::Path::new(ruta)) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{e:?}");
            return std::process::ExitCode::from(65); // EX_DATAERR
        }
    };

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
                claves: lista(&args, "--claves"),
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
                        println!("   {} clave(s): {}", p.claves.len(), p.claves.join(", "));
                    }
                    println!("\n③ CARGA ÚTIL");
                    for l in &p.lecturas {
                        println!("   {} · {}", l.datasource, l.objeto);
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
    }
}
