//! `ore-read-jsonl` — el lector de ficheros NDJSON, **la segunda familia**.
//!
//! # Por qué existe, y por qué no es un juguete
//!
//! El protocolo de [`ore-driver`](../../ore-driver/src/lib.rs) afirma que la
//! petición es **un fragmento del plan, no SQL**, y que traducir es del driver.
//! Una afirmación así no se demuestra con un segundo driver SQL: se demuestra con
//! uno que **no tenga SQL en absoluto**.
//!
//! Aquí no hay dialecto, ni conexión, ni credencial. Hay un fichero. Y el motor
//! no se entera: manda la misma petición y recibe las mismas filas.
//!
//! > **Si el mismo plan sirve a una base de datos y a un fichero, la petición
//! > estaba cortada por el sitio correcto.**
//!
//! # Y mide algo que ningún argumento mide
//!
//! El driver de PostgreSQL trae 114 crates y FFI de plataforma. Este trae **lo
//! que trae `ore-core`**, porque leer un fichero no exige hablar con nadie. Los
//! dos implementan el mismo contrato, y esa distancia es la que dice que el peso
//! del primero es de PostgreSQL y no del protocolo.
//!
//! # Qué es una «fuente» aquí
//!
//! La URL es una **ruta**, y el objeto es el nombre del fichero dentro de ella —
//! un directorio de NDJSON es un esquema, y cada fichero una tabla. Las columnas
//! son las claves de cada objeto JSON.

use std::io::Read as _;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let verbo = args.first().map(String::as_str).unwrap_or("catalogo");

    let mut entrada = String::new();
    if std::io::stdin().read_to_string(&mut entrada).is_err() {
        eprintln!("ore-read-jsonl: no se pudo leer stdin");
        return ExitCode::FAILURE;
    }

    let resultado = match verbo {
        "leer" => filas(&entrada),
        // El catálogo de un directorio de ficheros es otro trabajo y no lo
        // necesita M4. Decirlo es mejor que devolver un catálogo vacío, que
        // tendría el mismo aspecto que un directorio sin tablas.
        "catalogo" => Err("`ore-read-jsonl` no sabe leer un catálogo todavía. \
                           Lo que implementa es `leer`, que es el verbo de la fase ③"
            .to_string()),
        otro => Err(format!("`{otro}` no es un verbo de este lector")),
    };

    match resultado {
        Ok(salida) => {
            if !salida.is_empty() {
                println!("{salida}");
            }
            ExitCode::SUCCESS
        }
        Err(m) => {
            eprintln!("ore-read-jsonl: {m}");
            ExitCode::FAILURE
        }
    }
}

fn filas(peticion: &str) -> Result<String, String> {
    let p = ore_driver::leer_peticion(peticion)?;

    let ruta = std::path::Path::new(&p.url).join(&p.objeto);
    let texto = std::fs::read_to_string(&ruta)
        .map_err(|e| format!("no se pudo leer `{}`: {e}", ruta.display()))?;

    let mut out = String::new();
    for linea in texto.lines().filter(|l| !l.trim().is_empty()) {
        let n = ore_core::parse::parse(linea)
            .map_err(|e| format!("una línea del fichero no analiza: {e:?}"))?;
        let campo = |c: &str| n.get(c).and_then(|(_, v)| v.as_str()).unwrap_or("");

        // El recorte por clave, que es lo que el plan pide. La comparación es de
        // tuplas: una clave compuesta lo sigue siendo aquí.
        if !p.claves.is_empty() {
            let mia: Vec<&str> = p.clave_columnas.iter().map(|c| campo(c)).collect();
            if !p
                .claves
                .iter()
                .any(|t| t.iter().zip(&mia).all(|(a, b)| a == b))
            {
                continue;
            }
        }
        // Y el filtro del ámbito, que llega igual que a una base de datos.
        // El mismo vocabulario que en SQL, sobre texto: un fichero no tiene
        // tipos, así que `gt` compara cadenas — que es exactamente lo que hace
        // falta para una marca de agua ISO-8601, y nada más.
        if !p.filtros.iter().all(|(c, op, v)| match op.as_str() {
            "gt" => campo(c) > v.as_str(),
            _ => campo(c) == v,
        }) {
            continue;
        }

        let valores: Vec<Option<String>> = p
            .proyeccion
            .iter()
            .map(|(_, col)| n.get(col).and_then(|(_, v)| v.as_str()).map(String::from))
            .collect();
        out.push_str(&ore_driver::fila(&p, &valores));
        out.push('\n');
    }
    Ok(out.trim_end().to_string())
}
