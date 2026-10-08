//! `ore-postgres` — **el plano de control de ORE Serverless Postgres** (ADR 0058, P4).
//!
//! Lo que tú decides arriba, el producto lo hace correr abajo. Una celda —el
//! workspace del cliente— pide proyectos, ramas, endpoints, roles y bases; esto
//! lo recuerda (lo deseado y lo observado) y, desde P4·2, lo hace existir en el
//! almacenamiento y en NeonVM.
//!
//! ```text
//!   ore-serve (la celda) ──WI de la celda──► ore-postgres ──► storage_controller, safekeepers (P4·2)
//!                                            org = la de la celda       NeonVM en ore-pg-computo (P4·3)
//!                                            (POST /access/v1/celda de ore-iam)
//! ```
//!
//! ⭐ Integrado en la identidad y la propiedad, aparte en el runtime (decidido 8):
//!   la identidad y la organización las dice el plano de control común
//!   (`ore-iam`); el cómputo y el almacenamiento son del producto. Si `ore-iam`
//!   cae se para la gestión, no los datos.
//!
//! Lo que hay: el contrato ([`api`]), la base y sus migraciones ([`base`]), la
//! verificación de la celda ([`celda`]) y, desde P4·2, el almacenamiento
//! ([`almacen`]) y el reconciliador que lo hace existir ([`reconciliador`]): un
//! proyecto es un tenant y su `main`, un timeline.

pub mod almacen;
pub mod api;
pub mod avisos;
pub mod base;
pub mod celda;
pub mod computos;
pub mod especificacion;
pub mod kube;
pub mod llaves;
pub mod olvidar;
pub mod proxy;
pub mod reconciliador;
pub mod scram;

use ore_entrada::http;
use std::net::TcpListener;
use std::process::ExitCode;
use std::sync::Mutex;

const USO: &str = "\
ore-postgres — el plano de control de ORE Serverless Postgres

  ore-postgres servir [--bind DIRECCION] [--iam DESTINO]
                      [--controlador DESTINO] [--safekeepers D1,D2,D3] [--llaves-almacen DIR]
                      [--llaves-proxy DIR] [--redis HOST:PUERTO]
  ore-postgres especificacion --computo NOMBRE --tenant T --timeline TL [--grupo G]
  ore-postgres token-computo NOMBRE
  ore-postgres kube-prueba [NAMESPACE-AJENO…]

  `especificacion` y `token-computo` son para las pruebas de fuego mientras el
  reconciliador no crea los cómputos (P4·3·1): escriben por la salida estándar el
  `config.json` de un cómputo y un token de una hora para su `compute_ctl`.
  Leen las llaves de `--llaves-almacen` (`admin`, `privada.pem`) y de
  `--llaves-computo` (`privada.pem`; por defecto /llaves/computo).

  La base sale de `ORE_POSTGRES_URL`; sin valor por defecto. Las migraciones se
  aplican al arrancar. `--iam` es dónde preguntar de qué organización es una celda
  (por defecto `ore-iam.identidad.svc.cluster.local.:8090`).

  El reconciliador (P4·2) habla con el storage_controller y los safekeepers de
  `ore-pg` con los tokens de `--llaves-almacen` (`admin`, `safekeeperdata`; por
  defecto /llaves/almacen). Sin ellos no arranca, se dice, y las operaciones
  esperan en curso.

  El proxy de Neon pregunta en /proxy/… con el token de `--llaves-proxy` (`token`;
  por defecto /llaves/proxy). Con `--redis HOST:PUERTO`, lo que el reconciliador da
  por hecho se le avisa por Redis para que olvide lo que guardaba (P5·1).
";

fn valor(args: &[String], que: &str) -> Option<String> {
    args.iter()
        .position(|a| a == que)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

pub fn arrancar(args: Vec<String>) -> ExitCode {
    match args.first().map(String::as_str) {
        Some("servir") => servir(&args),
        Some("especificacion") => especificacion_mando(&args),
        Some("token-computo") => token_mando(&args),
        Some("kube-prueba") => kube_prueba(&args[1..]),
        Some("-h" | "--help") => {
            print!("{USO}");
            ExitCode::SUCCESS
        }
        Some(otro) => {
            eprintln!("✗ mando desconocido: `{otro}`\n\n{USO}");
            ExitCode::from(64)
        }
        None => {
            print!("{USO}");
            ExitCode::from(64)
        }
    }
}

/// Lo común a los dos mandos de prueba: el almacenamiento y las llaves.
fn para_probar(args: &[String]) -> Result<(almacen::Neon, llaves::Llave, llaves::Llave), String> {
    let dir = valor(args, "--llaves-almacen").unwrap_or_else(|| "/llaves/almacen".into());
    let dir_computo = valor(args, "--llaves-computo").unwrap_or_else(|| "/llaves/computo".into());
    let controlador = valor(args, "--controlador")
        .unwrap_or_else(|| "storage-controller.ore-pg.svc.cluster.local.:1234".into());
    let safekeepers = (0..3)
        .map(|i| format!("safekeeper-{i}.ore-pg.svc.cluster.local.:7676"))
        .collect();
    let neon = almacen::Neon::nuevo(&controlador, safekeepers, std::path::Path::new(&dir))?;
    let propia =
        llaves::Llave::del_fichero(&std::path::Path::new(&dir_computo).join("privada.pem"))?;
    let del_almacen = llaves::Llave::del_fichero(&std::path::Path::new(&dir).join("privada.pem"))?;
    Ok((neon, propia, del_almacen))
}

fn segundos() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn especificacion_mando(args: &[String]) -> ExitCode {
    use almacen::Almacen;
    let (Some(nombre), Some(tenant), Some(timeline)) = (
        valor(args, "--computo"),
        valor(args, "--tenant"),
        valor(args, "--timeline"),
    ) else {
        eprintln!("✗ `especificacion` necesita `--computo`, `--tenant` y `--timeline`");
        return ExitCode::from(64);
    };
    let grupo = valor(args, "--grupo").unwrap_or_else(|| "ore-pg".into());
    let (neon, propia, del_almacen) = match para_probar(args) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("✗ {e}");
            return ExitCode::from(66);
        }
    };
    let pageserver = match neon.pageserver_de(&tenant) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("✗ el pageserver del tenant: {}", e.motivo());
            return ExitCode::from(69);
        }
    };
    let sk = neon.safekeepers_pg();
    let ahora = especificacion::iso(segundos());
    let e = especificacion::especificacion(
        &especificacion::Computo {
            nombre: &nombre,
            tenant: &tenant,
            timeline: &timeline,
            pageserver: &pageserver,
            safekeepers: &sk,
            grupo: &grupo,
            ahora: &ahora,
            replica: false,
            datos: &especificacion::Datos::default(),
        },
        &propia,
        Some(&del_almacen),
    );
    println!("{}", e.pretty());
    ExitCode::SUCCESS
}

fn token_mando(args: &[String]) -> ExitCode {
    let Some(nombre) = args.get(1) else {
        eprintln!("✗ `token-computo` necesita el nombre del cómputo");
        return ExitCode::from(64);
    };
    match para_probar(args) {
        Ok((_, propia, _)) => {
            println!(
                "{}",
                llaves::token_de_computo(&propia, nombre, segundos() + 3600)
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("✗ {e}");
            ExitCode::from(66)
        }
    }
}

/// P4·3·2: lo que la cuenta de `ore-postgres` puede en Kubernetes, medido desde
/// dentro. En `ore-pg-computo` crea, lee y borra un ConfigMap; en cualquier otro
/// namespace, 403.
fn kube_prueba(ajenos: &[String]) -> ExitCode {
    let k = match kube::Kube::del_pod() {
        Ok(k) => k,
        Err(e) => {
            eprintln!("✗ {e}");
            return ExitCode::from(69);
        }
    };
    let mut fallos = 0;
    let mut ver = |bien: bool, que: String| {
        println!("  {} {que}", if bien { "✓" } else { "✗" });
        fallos += u8::from(!bien);
    };
    let (ns, nombre) = ("ore-pg-computo", "ore-postgres-prueba");
    let c = kube::configmap(ns, nombre);
    let objeto = kube::configmap_con(ns, nombre, "hola", "P4·3·2");
    ver(
        k.aplicar(&c, &objeto).is_ok(),
        format!("{ns}: crear un ConfigMap"),
    );
    ver(
        k.aplicar(&c, &objeto).is_ok(),
        format!("{ns}: aplicarlo otra vez (idempotente)"),
    );
    ver(
        k.leer(&c)
            .ok()
            .flatten()
            .is_some_and(|r| r.contains("P4·3·2")),
        format!("{ns}: leerlo"),
    );
    ver(k.borrar(&c).is_ok(), format!("{ns}: borrarlo"));
    ver(
        k.leer(&c).ok().flatten().is_none(),
        format!("{ns}: ya no está"),
    );
    for otro in ajenos {
        let c = kube::configmap(otro, nombre);
        match k.aplicar(&c, &kube::configmap_con(otro, nombre, "hola", "no")) {
            Ok(()) => {
                let _ = k.borrar(&c);
                ver(false, format!("{otro}: ¡PUDO crear un ConfigMap!"));
            }
            Err(e) => ver(
                e.contains(": 403 "),
                format!(
                    "{otro}: no puede ({})",
                    e.chars().take(60).collect::<String>()
                ),
            ),
        }
        match k.pedir(
            "GET",
            &format!("/api/v1/namespaces/{otro}/secrets"),
            None,
            None,
        ) {
            Ok((403, _)) => ver(true, format!("{otro}: no lee sus Secrets (403)")),
            Ok((c, _)) => ver(false, format!("{otro}: leer sus Secrets dio {c}")),
            Err(e) => ver(false, format!("{otro}: {e}")),
        }
    }
    if fallos == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

/// Lo que el reconciliador necesita para crear VMs: el API de Kubernetes, las
/// dos llaves y la imagen de cómputo.
fn computos_de(
    imagen: &str,
    ns: &str,
    llaves_almacen: &str,
    llaves_computo: &str,
    safekeepers: Vec<String>,
) -> Result<computos::Neonvm, String> {
    if imagen.is_empty() {
        return Err(
            "falta `--imagen-computo` (la vm-compute-node-v17 de ci/neon/computo.yaml)".into(),
        );
    }
    let kube = kube::Kube::del_pod()?;
    let propia =
        llaves::Llave::del_fichero(&std::path::Path::new(llaves_computo).join("privada.pem"))?;
    let almacen =
        llaves::Llave::del_fichero(&std::path::Path::new(llaves_almacen).join("privada.pem"))?;
    Ok(computos::Neonvm::nuevo(
        ns,
        imagen,
        "neon",
        safekeepers,
        kube,
        propia,
        almacen,
    ))
}

fn servir(args: &[String]) -> ExitCode {
    let url = match std::env::var("ORE_POSTGRES_URL") {
        Ok(u) if !u.is_empty() => u,
        _ => {
            eprintln!("✗ falta `ORE_POSTGRES_URL`: sin ella no hay dónde recordar nada,");
            eprintln!("  y un valor por defecto sería apuntar a una base que nadie eligió.");
            return ExitCode::from(64);
        }
    };
    let bind = valor(args, "--bind").unwrap_or_else(|| "127.0.0.1:8100".into());
    let iam = valor(args, "--iam").unwrap_or_else(|| celda::DESTINO.into());
    let controlador = valor(args, "--controlador")
        .unwrap_or_else(|| "storage-controller.ore-pg.svc.cluster.local.:1234".into());
    let safekeepers: Vec<String> = valor(args, "--safekeepers")
        .map(|v| {
            v.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_else(|| {
            (0..3)
                .map(|i| format!("safekeeper-{i}.ore-pg.svc.cluster.local.:7676"))
                .collect()
        });
    let llaves = valor(args, "--llaves-almacen").unwrap_or_else(|| "/llaves/almacen".into());
    let llaves_computo =
        valor(args, "--llaves-computo").unwrap_or_else(|| "/llaves/computo".into());
    let imagen = valor(args, "--imagen-computo").unwrap_or_default();
    let ns_computo = valor(args, "--ns-computo").unwrap_or_else(|| "ore-pg-computo".into());
    let llaves_proxy = valor(args, "--llaves-proxy").unwrap_or_else(|| "/llaves/proxy".into());
    let redis = valor(args, "--redis");

    let mut base = match base::conectar(&url) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("✗ {e}");
            return ExitCode::from(69);
        }
    };
    match base::migrar(&mut base) {
        Ok(a) if a.is_empty() => eprintln!("  base         al día"),
        Ok(a) => eprintln!("  base         migrada: {}", a.join(", ")),
        Err(e) => {
            eprintln!("✗ {e}");
            return ExitCode::from(70);
        }
    }
    let escucha = match TcpListener::bind(&bind) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("✗ no se pudo escuchar en `{bind}`: {e}");
            return ExitCode::from(73);
        }
    };
    eprintln!("ore-postgres · {bind}");
    eprintln!("  celdas       la organización de cada una, de ore-iam en {iam}");
    let mut avisos = None;
    match almacen::Neon::nuevo(
        &controlador,
        safekeepers.clone(),
        std::path::Path::new(&llaves),
    ) {
        Ok(neon) => {
            eprintln!(
                "  almacén      controller {controlador} · safekeepers {}",
                safekeepers.join(", ")
            );
            let sk_pg = neon.safekeepers_pg();
            let neon: std::sync::Arc<dyn almacen::Almacen> = std::sync::Arc::new(neon);
            let computos: std::sync::Arc<dyn computos::Computos> =
                match computos_de(&imagen, &ns_computo, &llaves, &llaves_computo, sk_pg) {
                    Ok(c) => {
                        eprintln!("  cómputo      VMs en {ns_computo} · imagen {imagen}");
                        std::sync::Arc::new(c)
                    }
                    Err(e) => {
                        eprintln!("  ⚠ SIN CÓMPUTOS: {e}");
                        eprintln!("    los endpoints quedan en curso hasta que esté");
                        std::sync::Arc::new(computos::SinKube(e))
                    }
                };
            reconciliador::arrancar(url.clone(), neon.clone(), computos.clone());
            // P4·5: los avisos del controller, con la pública del almacenamiento.
            match llaves::Publica::del_fichero(&std::path::Path::new(&llaves).join("publica.pem")) {
                Ok(infra) => {
                    eprintln!(
                        "  avisos       /avisos/notify-attach (token infra del almacenamiento)"
                    );
                    avisos = Some(avisos::Avisos {
                        almacen: neon,
                        computos,
                        infra,
                    });
                }
                Err(e) => eprintln!("  ⚠ SIN AVISOS: {e}"),
            }
        }
        Err(e) => {
            eprintln!("  ⚠ SIN RECONCILIADOR: {e}");
            eprintln!("    las operaciones quedan en curso hasta que se monten los tokens");
        }
    }
    // P5·1: las preguntas del proxy, con SU token (un Secret que sólo montan él y esto).
    let proxy = match std::fs::read_to_string(std::path::Path::new(&llaves_proxy).join("token")) {
        Ok(t) if !t.trim().is_empty() => {
            eprintln!("  proxy        /proxy/get_endpoint_access_control · /proxy/wake_compute");
            Some(proxy::Proxy {
                token: t.trim().to_string(),
            })
        }
        _ => {
            eprintln!("  ⚠ SIN PROXY: no hay token en {llaves_proxy}/token");
            None
        }
    };
    // P5·1: lo que el reconciliador da por hecho, el proxy lo olvida (por Redis).
    match redis {
        Some(r) => {
            eprintln!(
                "  olvidar      lo hecho → PUBLISH {} en {r}",
                olvidar::CANAL
            );
            olvidar::arrancar(url.clone(), r);
        }
        None => eprintln!("  ⚠ SIN REDIS: el proxy tarda hasta 4 min en ver una contraseña nueva"),
    }
    let servidor = api::Servidor {
        base: Mutex::new(base),
        celdas: Box::new(celda::PorOreIam::nuevo(&iam)),
        url: Some(url),
        avisos,
        proxy,
    };
    match http::servir(escucha, move |p| servidor.atender(p)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("✗ el servidor terminó: {e}");
            ExitCode::from(70)
        }
    }
}
