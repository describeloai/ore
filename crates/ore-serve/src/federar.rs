//! **`ore-serve federar-probar`** (ADR 0053 F3·4): medir la pasarela de la
//! celda desde donde la llamará el coordinador, con la credencial de verdad.
//!
//! ```text
//! ore-serve federar-probar --agente-fichero F --idp H --emisor E --cofre C --organizacion O
//!     --fuente N --tipo postgres|bigquery|s3 --objeto S.T --proyeccion a,b[:col]
//!     [--extra '{json}'] [--pasarela ore-federation:8099] [--veces 6]
//!     [--a-la-vez 50] [--corte 1000] [--ms 30000]
//! ```
//!
//! Trae la credencial de `fuente-<N>` del custodio **como el agente de la
//! celda** —el mismo camino que la ruta de hoy y que el coordinador de F4—, y
//! mide: la primera lectura (fría), las siguientes (calientes), el primer
//! byte, el corte por filas y muchas a la vez. **Nunca imprime la credencial**:
//! lo que sale de la pasarela se tapa con ella antes de escribirse.
//!
//! No es una ruta: no la alcanza nadie de fuera del pod.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use ore_core::json::Json;

/// Una respuesta de la pasarela: estado, cuánto tardó la cabecera, cuánto el
/// cuerpo entero, sus bytes y sus *trailers*.
#[derive(Debug, Default)]
struct Medida {
    codigo: u16,
    cabecera_ms: u128,
    total_ms: u128,
    bytes: usize,
    finales: Vec<(String, String)>,
    cuerpo_json: String,
}

impl Medida {
    fn fin(&self, k: &str) -> String {
        self.finales
            .iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    }
}

fn leer(pasarela: &str, cuerpo: &str) -> Result<Medida, String> {
    let t = Instant::now();
    let mut s =
        TcpStream::connect(pasarela).map_err(|e| format!("no se llega a la pasarela: {e}"))?;
    s.set_read_timeout(Some(Duration::from_secs(120))).ok();
    let req = format!(
        "POST /v1/read HTTP/1.1\r\nhost: pasarela\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{cuerpo}",
        cuerpo.len()
    );
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let mut c = BufReader::new(s);
    let mut l = String::new();
    c.read_line(&mut l).map_err(|e| e.to_string())?;
    let mut m = Medida {
        codigo: l
            .split_whitespace()
            .nth(1)
            .and_then(|x| x.parse().ok())
            .unwrap_or(0),
        cabecera_ms: t.elapsed().as_millis(),
        ..Medida::default()
    };
    let mut troceado = false;
    let mut largo = 0usize;
    loop {
        let mut l = String::new();
        c.read_line(&mut l).map_err(|e| e.to_string())?;
        let l = l.trim_end();
        if l.is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            match k.trim().to_ascii_lowercase().as_str() {
                "transfer-encoding" => troceado = v.trim() == "chunked",
                "content-length" => largo = v.trim().parse().unwrap_or(0),
                _ => {}
            }
        }
    }
    if !troceado {
        let mut b = vec![0; largo];
        c.read_exact(&mut b).map_err(|e| e.to_string())?;
        m.bytes = b.len();
        m.cuerpo_json = String::from_utf8_lossy(&b).trim().to_string();
    } else {
        loop {
            let mut l = String::new();
            c.read_line(&mut l).map_err(|e| e.to_string())?;
            let n = usize::from_str_radix(l.trim(), 16).map_err(|_| "un trozo raro".to_string())?;
            if n == 0 {
                break;
            }
            let mut b = vec![0; n + 2];
            c.read_exact(&mut b).map_err(|e| e.to_string())?;
            m.bytes += n;
        }
        loop {
            let mut l = String::new();
            if c.read_line(&mut l).map_err(|e| e.to_string())? == 0 {
                break;
            }
            let l = l.trim_end();
            if l.is_empty() {
                break;
            }
            if let Some((k, v)) = l.split_once(':') {
                m.finales.push((k.trim().to_string(), v.trim().to_string()));
            }
        }
    }
    m.total_ms = t.elapsed().as_millis();
    Ok(m)
}

fn percentil(v: &[u128], p: f64) -> u128 {
    if v.is_empty() {
        return 0;
    }
    let mut v = v.to_vec();
    v.sort_unstable();
    v[((v.len() as f64 - 1.0) * p).round() as usize]
}

/// `a,b:col` → `{"a":"a","b":"col"}`.
fn proyeccion(s: &str) -> Json {
    Json::Obj(
        s.split(',')
            .filter(|x| !x.is_empty())
            .map(|x| match x.split_once(':') {
                Some((p, c)) => (p.to_string(), Json::s(c)),
                None => (x.to_string(), Json::s(x)),
            })
            .collect(),
    )
}

pub fn probar(args: &[String]) -> ExitCode {
    match intentar(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("✗ federar-probar: {e}");
            ExitCode::FAILURE
        }
    }
}

fn intentar(args: &[String]) -> Result<(), String> {
    let valor = |k: &str| {
        args.iter()
            .position(|a| a == k)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let falta = |k: &str| valor(k).ok_or_else(|| format!("falta `{k}`"));
    let fuente = falta("--fuente")?;
    let tipo = falta("--tipo")?;
    let objeto = falta("--objeto")?;
    let proy = proyeccion(&falta("--proyeccion")?);
    let pasarela = valor("--pasarela").unwrap_or_else(|| "ore-federation:8099".into());
    let veces: usize = valor("--veces").and_then(|v| v.parse().ok()).unwrap_or(6);
    let a_la_vez: usize = valor("--a-la-vez")
        .and_then(|v| v.parse().ok())
        .unwrap_or(50);
    let corte: Option<u64> = valor("--corte").and_then(|v| v.parse().ok());
    let ms: u64 = valor("--ms").and_then(|v| v.parse().ok()).unwrap_or(30_000);
    let extra = match valor("--extra") {
        Some(e) => match ore_core::parse::parse(&e).map(|n| Json::de_node(&n)) {
            Ok(Json::Obj(m)) => m,
            _ => return Err("`--extra` no es un objeto JSON".into()),
        },
        None => Default::default(),
    };

    // La credencial, como el agente de la celda (el camino de F4).
    let agente = crate::agente::Agente::de(
        &falta("--idp")?,
        &falta("--emisor")?,
        &falta("--agente-fichero")?,
    )?;
    let binario = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("ore-serve"));
    let (url, _) = crate::datasets::credencial_del_cofre(
        &agente,
        &falta("--cofre")?,
        &falta("--organizacion")?,
        &fuente,
        &binario,
    )
    .map_err(|r| format!("{} {}", r.codigo, r.cuerpo.jcs()))?;
    let tapa = |t: &str| ore_driver_tapar(t, &url);

    let cuerpo = |id: &str, filas: Option<u64>| {
        let mut pet = extra.clone();
        pet.insert("objeto".into(), Json::s(objeto.as_str()));
        pet.insert("proyeccion".into(), proy.clone());
        let mut o = std::collections::BTreeMap::new();
        o.insert("id".to_string(), Json::s(id));
        o.insert("origen".to_string(), Json::s(fuente.as_str()));
        o.insert("tipo".to_string(), Json::s(tipo.as_str()));
        o.insert("url".to_string(), Json::s(url.as_str()));
        o.insert("peticion".to_string(), Json::Obj(pet));
        let mut p = vec![("ms", Json::Int(ms as i64))];
        if let Some(f) = filas {
            p.push(("filas", Json::Int(f as i64)));
        }
        o.insert("presupuesto".to_string(), Json::obj(p));
        Json::Obj(o).jcs()
    };
    let base = format!(
        "probar-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    );
    let decir = |nombre: &str, m: &Medida| {
        println!(
            "{nombre:<12} {} · cabecera {} ms · total {} ms · {} B · {} {} · {} filas{}",
            m.codigo,
            m.cabecera_ms,
            m.total_ms,
            m.bytes,
            m.fin("ore-estado"),
            m.fin("ore-motivo"),
            m.fin("ore-filas"),
            if m.cuerpo_json.is_empty() {
                String::new()
            } else {
                format!(" · {}", tapa(&m.cuerpo_json))
            }
        );
    };

    println!("federar-probar · fuente `{fuente}` ({tipo}) · `{objeto}` · pasarela {pasarela}");
    // ① Fría y ② calientes.
    let mut ms_calientes = Vec::new();
    for i in 0..veces {
        let m = leer(&pasarela, &cuerpo(&format!("{base}-v{i}"), None)).map_err(|e| tapa(&e))?;
        decir(if i == 0 { "fría" } else { "caliente" }, &m);
        if i > 0 {
            ms_calientes.push(m.total_ms);
        }
    }
    if !ms_calientes.is_empty() {
        println!(
            "calientes    p50 {} ms · p95 {} ms ({} lecturas)",
            percentil(&ms_calientes, 0.5),
            percentil(&ms_calientes, 0.95),
            ms_calientes.len()
        );
    }
    // ③ El corte por filas.
    if let Some(f) = corte {
        let m =
            leer(&pasarela, &cuerpo(&format!("{base}-corte"), Some(f))).map_err(|e| tapa(&e))?;
        decir("corte", &m);
    }
    // ④ Muchas a la vez.
    if a_la_vez > 0 {
        let t = Instant::now();
        let hilos: Vec<_> = (0..a_la_vez)
            .map(|i| {
                let c = cuerpo(&format!("{base}-a{i}"), None);
                let p = pasarela.clone();
                std::thread::spawn(move || leer(&p, &c))
            })
            .collect();
        let (mut ok, mut saturadas, mut otras) = (0, 0, Vec::new());
        let mut tiempos = Vec::new();
        for h in hilos {
            match h.join() {
                Ok(Ok(m)) if m.codigo == 200 && m.fin("ore-estado") == "completo" => {
                    ok += 1;
                    tiempos.push(m.total_ms);
                }
                Ok(Ok(m)) if m.codigo == 503 => saturadas += 1,
                Ok(Ok(m)) => otras.push(format!("{} {}", m.codigo, tapa(&m.cuerpo_json))),
                Ok(Err(e)) => otras.push(tapa(&e)),
                Err(_) => otras.push("un hilo murió".into()),
            }
        }
        println!(
            "a la vez     {a_la_vez}: {ok} completas, {saturadas} saturadas (503), {} otras · p50 {} ms · p95 {} ms · todo en {} ms{}",
            otras.len(),
            percentil(&tiempos, 0.5),
            percentil(&tiempos, 0.95),
            t.elapsed().as_millis(),
            otras
                .first()
                .map(|o| format!(" · p.ej. {o}"))
                .unwrap_or_default()
        );
    }
    // Y lo que la pasarela dice de este origen.
    if let Ok(mut s) = TcpStream::connect(&pasarela) {
        let _ =
            s.write_all(b"GET /v1/origins HTTP/1.1\r\nhost: pasarela\r\nconnection: close\r\n\r\n");
        let mut t = String::new();
        let _ = s.read_to_string(&mut t);
        if let Some((_, cuerpo)) = t.split_once("\r\n\r\n") {
            println!("origenes     {}", tapa(cuerpo.trim()));
        }
    }
    Ok(())
}

/// `ore_driver::tapar`, sin enlazar el crate entero: lo que pueda llevar la
/// credencial (la `url` y su clave) sale tapado.
fn ore_driver_tapar(texto: &str, url: &str) -> String {
    let mut t = texto.replace(url, "<url>");
    if let Some(resto) = url.split_once("://").map(|x| x.1)
        && let Some((usuario, _)) = resto.split_once('@')
        && let Some((_, clave)) = usuario.split_once(':')
        && clave.len() >= 4
    {
        t = t.replace(clave, "<clave>");
    }
    t
}
