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
//! Y dos banderas que valen para `plan` y para `responder`: `--indice` carga el
//! artefacto de topología y `--cache` el manifiesto de lo materializado. La
//! segunda **no rechaza un manifiesto de otro bundle**, a diferencia de la
//! primera: uno de otro bundle sí dice algo —que hay caché y que no sirve— y
//! callarlo haría que el plan dijera «no había caché».
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
    let ruta = if verbo == "index" {
        args.get(2)
    } else {
        args.get(1)
    };
    let Some(ruta) = ruta else {
        eprintln!("uso: ore-exec <validar|plan> <ruta> [banderas]");
        eprintln!("     ore-exec index <build|refresh|id|traverse> <ruta> [banderas]");
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

    // El manifiesto de cache. A diferencia del indice, NO se rechaza por ser de
    // otro bundle: un manifiesto de otro bundle si dice algo —que hay cache y
    // que no sirve— y callarlo haria que el plan dijera «no habia cache».
    if let Some(m) = valor(&args, "--cache")
        && let Err(e) = motor.cargar_cache(std::path::Path::new(&m))
    {
        eprintln!("manifiesto ilegible · {e}");
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
            let c = consulta_de(&args);
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
                        match &l.origen {
                            ore_exec::Origen::Cache { marca } => {
                                println!("      de la cache · marca {marca}");
                            }
                            ore_exec::Origen::Fuente { porque: Some(x) } => {
                                println!("      del origen · {x}");
                            }
                            ore_exec::Origen::Fuente { porque: None } => {}
                        }
                        if !l.clave_columnas.is_empty() {
                            println!("      clave: {}", l.clave_columnas.join(", "));
                        }
                        for (prop, col) in &l.proyeccion {
                            println!("      {prop} ← {col}");
                        }
                        for f in &l.filtros {
                            println!(
                                "      filtro: {} = {:?}  ({})",
                                f.columna, f.valor, f.ambito
                            );
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
        "responder" => {
            let c = consulta_de(&args);
            match motor.planificar(&c) {
                Err(r) => {
                    eprintln!("{}", rechazo(&r));
                    std::process::ExitCode::from(77)
                }
                Ok(p) => match motor.responder(
                    &p,
                    valor(&args, "--instante").as_deref(),
                    valor(&args, "--sla").as_deref(),
                ) {
                    Err(e) => {
                        eprintln!("no se pudo responder · {e}");
                        std::process::ExitCode::FAILURE
                    }
                    Ok(r) => {
                        for f in &r.filas {
                            let campos: Vec<String> =
                                f.iter().map(|(k, v)| format!("{k}={v:?}")).collect();
                            println!("{}", campos.join("  "));
                        }
                        eprintln!();
                        for (donde, de) in &r.origenes {
                            eprintln!("origen   {donde} <- {de}");
                        }
                        eprintln!("digest   {}", r.digest);
                        eprintln!("marca    {}", r.marca.as_deref().unwrap_or("—"));
                        eprintln!("instante {}", r.instante.as_deref().unwrap_or("—"));
                        if let Some(d) = &r.degradado {
                            eprintln!("DEGRADADO · {d}");
                        }
                        std::process::ExitCode::SUCCESS
                    }
                },
            }
        }
        otro => {
            eprintln!("`{otro}` no es un verbo de `ore-exec`");
            std::process::ExitCode::from(64)
        }
    }
}

/// La consulta, leida de las banderas. Una sola vez: `plan` y `responder` piden
/// lo mismo, y dos lecturas de las mismas banderas acabarian divergiendo.
fn consulta_de(args: &[String]) -> Consulta {
    let claims: BTreeMap<String, String> = lista(args, "--claims")
        .iter()
        .filter_map(|kv| kv.split_once('=').map(|(k, v)| (k.into(), v.into())))
        .collect();
    Consulta {
        quien: Identidad {
            emisor: valor(args, "--emisor").unwrap_or_default(),
            audiencia: valor(args, "--audiencia").unwrap_or_default(),
            sujeto: valor(args, "--sujeto").unwrap_or_default(),
            roles: lista(args, "--roles"),
            claims,
        },
        accion: valor(args, "--accion").unwrap_or_else(|| "read".into()),
        purpose: valor(args, "--purpose").unwrap_or_default(),
        entidad: valor(args, "--entidad").unwrap_or_default(),
        propiedades: lista(args, "--props"),
        claves: lista(args, "--claves")
            .into_iter()
            .map(|k| vec![k])
            .collect(),
        travesia: None,
        // El instante entra en la CONSULTA porque decidir si lo materializado
        // esta rancio decide si se abre una conexion, y eso es planificar. No
        // rompe la pureza: se recibe, no se lee.
        instante: valor(args, "--instante"),
        sla: valor(args, "--sla"),
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
            format!(
                "petición inválida · {m}\n  no es una denegación: es una petición que no existe"
            )
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
        Rechazo::SinClaveParaEnsamblar { propiedad } => format!(
            "sin clave para ensamblar · `{propiedad}` no está autorizada, y hay dos lecturas              que juntar: sin clave, la fila mezclaría dos personas"
        ),
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
                    // La version se imprime porque es lo que una propuesta cita
                    // como «con que correspondencia se resolvieron las claves»,
                    // y quien construye el indice es el unico que la sabe antes
                    // de que nadie lo lea.
                    eprintln!("version  {}", t.version());
                    std::process::ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("no se pudo escribir `{salida}`: {e}");
                    std::process::ExitCode::FAILURE
                }
            }
        }
        Some("refresh") => match refrescar(motor, args) {
            Ok((previas, nuevas)) => {
                eprintln!("{previas} arista(s) previas + {nuevas} nueva(s)");
                std::process::ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e}");
                std::process::ExitCode::FAILURE
            }
        },
        Some("id") => {
            let Some(t) = &motor.topologia else {
                eprintln!("falta `--indice <fichero>`");
                return std::process::ExitCode::from(64);
            };
            // Las tres afirmaciones del artefacto, por separado, porque caducan
            // por separado: el bundle cuando se recompila el modelo, la version
            // cuando aparece o desaparece una arista, y la marca en cada
            // refresco.
            println!("bundle   {}", t.digest);
            println!("version  {}", t.version());
            let marca = if t.marca.is_empty() {
                "—"
            } else {
                t.marca.as_str()
            };
            println!("marca    {marca}");
            std::process::ExitCode::SUCCESS
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
            eprintln!("uso: ore-exec index <build|refresh|id|traverse> <ruta> [banderas]");
            std::process::ExitCode::from(64)
        }
    }
}

/// De qué variable sale la credencial del refresco, y si hay que avisar.
///
/// `--url-env` manda porque quien opera puede saber más que el manifiesto. Si no
/// lo da, se busca `refreshEnv` en la fuente; y si tampoco está, se usa la de las
/// consultas **avisando**: colapsar las dos identidades es una decisión, y una
/// decisión en silencio no es una decisión (`05-ejecutor` §6.2).
fn identidad_de_refresco(motor: &Motor, args: &[String]) -> Result<(String, bool), String> {
    if let Some(v) = valor(args, "--url-env") {
        return Ok((v, false));
    }
    let ds = motor
        .lecturas_de_aristas()
        .first()
        .map(|(_, l)| l.datasource.clone())
        .ok_or("no hay ninguna relación con `via` que leer")?;
    match motor.variables_de(&ds)? {
        (_, Some(refresco)) => Ok((refresco, false)),
        (consultas, None) => Ok((consultas, true)),
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

/// `index refresh` — **reconstruir la ventana, no mutar**.
///
/// CSR no admite actualizaciones dinámicas sin reconstruir el array de aristas
/// entero, y eso aquí **no es un defecto**: refrescar es reconstruir por
/// ventana, y era el modo de operación desde el ADR 0006. Lo que el refresco
/// añade es que la ventana no empiece de cero — se leen solo las filas que la
/// marca de agua deja fuera, y se fusionan sobre las que ya había.
///
/// # Las tres estrategias, y cuál está implementada
///
/// | | Qué es la marca de agua | Estado |
/// |---|---|---|
/// | `poll` | la propiedad que declara `watermark` | **implementada** |
/// | `cdc` | igual, pero el origen emite los cambios | pendiente: exige una fuente que los emita |
/// | `table_version` | la versión nativa del formato | pendiente: **es de la caché de carga útil**, no de aquí |
///
/// # Lo que un refresco incremental NO puede ver
///
/// > **Una arista borrada no tiene marca de agua.**
///
/// `poll` lee lo que cambió *después* de un instante; una fila que ya no está no
/// cambió después de nada — desapareció. Así que un refresco incremental
/// **conserva aristas muertas**, y la única forma de quitarlas es reconstruir
/// entero. Eso no es un fallo de esta implementación: es la propiedad del
/// mecanismo, y `freshnessSLA` existe justamente para acotar cuánto tiempo se
/// vive con ella.
fn refrescar(motor: &Motor, args: &[String]) -> Result<(usize, usize), String> {
    // `--anterior`, no `--desde`: en `traverse`, `--desde` es una CLAVE. La
    // misma bandera con dos significados en el mismo verbo es una trampa
    // esperando a la primera prisa.
    let anterior = valor(args, "--anterior")
        .map(|f| -> Result<ore_exec::Topologia, String> {
            let b = std::fs::read(&f).map_err(|e| format!("no se pudo leer `{f}`: {e}"))?;
            ore_exec::Topologia::leer(&b)
        })
        .transpose()?;

    let marca_anterior = anterior
        .as_ref()
        .map(|t| t.marca.clone())
        .unwrap_or_default();
    let mut aristas = anterior.as_ref().map(|t| t.aristas()).unwrap_or_default();
    let previas = aristas.len();

    // La marca nueva la pone quien refresca: el motor no lee el reloj, y un
    // artefacto que se fechara a sí mismo dejaría de ser reproducible.
    let marca = valor(args, "--marca").ok_or("falta `--marca <instante>`")?;
    if !marca_anterior.is_empty() && marca <= marca_anterior {
        return Err(format!(
            "la marca nueva `{marca}` no es posterior a `{marca_anterior}`: un refresco que \
             no avanza no es un refresco"
        ));
    }

    let nuevas = leer_aristas_incremental(motor, args, &marca_anterior)?;

    // **La fusión no es una suma.** Una fila ES el conjunto de aristas de su
    // clave para esa relación, así que si la fila vuelve en el delta lo que
    // traiga **sustituye** a lo que hubiera.
    //
    // Lo encontró ejecutarlo. La primera versión sumaba, y con eso un cambio de
    // jefe dejaba las dos aristas: `emp-42` habría quedado reportando a la vez a
    // `jefa` y al `ceo`, y la cadena de mando habría tenido dos ramas. Sumar
    // hacía que un cambio se pareciera a una ampliación.
    let tocadas: std::collections::BTreeSet<(String, String)> = nuevas
        .iter()
        .map(|(r, d, _)| (r.clone(), d.clone()))
        .collect();
    aristas.retain(|(r, d, _)| !tocadas.contains(&(r.clone(), d.clone())));
    let reemplazadas = previas - aristas.len();
    aristas.extend(nuevas.iter().cloned());

    let t =
        ore_exec::Topologia::construir(&ore_core::digest::bundle(&motor.paquete), &marca, &aristas);
    let salida = valor(args, "-o").unwrap_or_else(|| "topologia.oretopo".into());
    std::fs::write(&salida, t.bytes())
        .map_err(|e| format!("no se pudo escribir `{salida}`: {e}"))?;

    if !marca_anterior.is_empty() {
        eprintln!(
            "aviso: un refresco incremental no ve una FILA BORRADA — una fila que ya no está \
             no cambió después de nada, así que sus aristas sobreviven. Un CAMBIO sí se ve: \
             la fila vuelve en el delta y sustituye lo que hubiera. Reconstruye entero para \
             quitar lo borrado."
        );
        eprintln!("{reemplazadas} arista(s) sustituidas por el delta");
    }
    Ok((previas, nuevas.len()))
}

/// Como `aristas_de_la_fuente`, pero acotado por la marca de agua.
///
/// El predicado es `gt` sobre la propiedad que el binding declara en
/// `materialization.topology.watermark`. Es el **único** sitio donde el motor
/// emite un operador de orden, y se puede: una marca de agua **no tiene
/// principal**, así que no filtra por nadie.
fn leer_aristas_incremental(
    motor: &Motor,
    args: &[String],
    desde: &str,
) -> Result<Vec<ore_exec::Arista>, String> {
    let (var, avisar) = identidad_de_refresco(motor, args)?;
    let url = std::env::var(&var).map_err(|_| format!("la variable `{var}` no está puesta"))?;
    if avisar {
        eprintln!(
            "aviso: el refresco usa `{var}`, la misma credencial que responde consultas. \
             `05-ejecutor` §6.2 pide que puedan ser distintas: declara `refreshEnv` en la \
             fuente. El que refresca necesita lectura amplia; el que responde, por clave."
        );
    }
    let tipo = valor(args, "--tipo").unwrap_or_else(|| "postgres".into());

    let mut out = Vec::new();
    for (relacion, mut lectura) in motor.lecturas_de_aristas() {
        if !desde.is_empty() {
            let Some(columna) = motor.columna_de_marca(&relacion) else {
                return Err(format!(
                    "`{relacion}` no declara `materialization.topology.watermark`, así que no \
                     hay desde dónde continuar: sin ella el refresco solo puede recargar entero"
                ));
            };
            lectura.filtros.push(ore_exec::Filtro {
                // La marca de agua no sale de una propiedad del principal: sale
                // del binding. Se nombra igual que la columna porque no hay una
                // propiedad corta a la que atribuirsela.
                propiedad: columna.clone(),
                columna,
                operador: "gt".into(),
                valor: desde.to_string(),
                ambito: "marca-de-agua".into(),
            });
        }
        let salida = std::process::Command::new(format!("ore-read-{tipo}"))
            .args(["leer", &lectura.datasource])
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
            return Err(String::from_utf8_lossy(&salida.stderr).trim().to_string());
        }
        for linea in String::from_utf8_lossy(&salida.stdout)
            .lines()
            .filter(|l| !l.trim().is_empty())
        {
            let Ok(n) = ore_core::parse::parse(linea) else {
                continue;
            };
            let d = n.get("desde").and_then(|(_, v)| v.as_str()).unwrap_or("");
            let h = n.get("hasta").and_then(|(_, v)| v.as_str()).unwrap_or("");
            if !d.is_empty() && !h.is_empty() {
                out.push((relacion.clone(), d.to_string(), h.to_string()));
            }
        }
    }
    Ok(out)
}
