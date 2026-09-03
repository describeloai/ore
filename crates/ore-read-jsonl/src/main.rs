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
        "testigo" => testigo(&entrada),
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

/// **El tercer verbo: hasta donde esta este fichero.**
///
/// La primera version de esto se negaba —«un directorio no tiene versiones»— y
/// era **demasiado modesta**. Un fichero si sabe fecharse: el digest de su
/// contenido **nombra exactamente esta version de el**, y eso es justo lo que
/// `snapshot` significa. El de Iceberg tampoco es un numero que crezca: es una
/// identidad.
///
/// Y no hay reloj de por medio, que es lo que lo hace bueno. La `mtime` habria
/// sido la respuesta comoda y es peor: dos escrituras en el mismo segundo
/// empatan, y entonces el testigo miente sobre haber cambiado.
///
/// # Lo que este testigo NO permite, y esta bien
///
/// Un rango. Dos digests no se ordenan, asi que no se puede pedir «lo que hay
/// entre A y B» — y por eso el modo es `snapshot` y no `log`. Un origen con
/// `snapshot` se lee ENTERO en su version, y lo que compra a cambio es que la
/// copia sea atomica: el testigo y las filas son la misma cosa.
fn testigo(peticion: &str) -> Result<String, String> {
    let (url, objeto) = ore_driver::leer_coordenada(peticion)?;
    let cursor = ore_core::parse::parse(peticion).ok().and_then(|n| {
        n.get("cursor")
            .and_then(|(_, v)| v.as_str())
            .map(String::from)
    });
    let ruta = std::path::Path::new(&url).join(&objeto);
    match std::fs::read(&ruta) {
        // **Si le nombran una columna, se fecha por ella.** Un fichero sabe
        // hacer las dos cosas, y cual de las dos se quiere lo dice la tabla al
        // declarar su `witness`. El maximo de la columna ES el testigo, que es
        // el modelo de *cursor field* que medio sector usa.
        Ok(b) if cursor.is_some() => {
            let c = cursor.unwrap_or_default();
            let texto = String::from_utf8_lossy(&b);
            let maximo = texto
                .lines()
                .filter(|l| !l.trim().is_empty())
                .filter_map(|l| ore_core::parse::parse(l).ok())
                .filter_map(|n| n.get(&c).and_then(|(_, v)| v.as_str()).map(String::from))
                .max();
            match maximo {
                Some(m) => Ok(ore_driver::testigo("field", Some(&m))),
                // Un fichero vacio no tiene maximo, y eso no es un fallo: es que
                // no hay por donde avanzar todavia.
                None => Ok(ore_driver::testigo("field", None)),
            }
        }
        Ok(b) => Ok(ore_driver::testigo(
            "snapshot",
            Some(&ore_core::digest::de_bytes(&b)),
        )),
        // No poder leerlo es un fallo, no un «no se sabe»: la peticion nombra un
        // fichero que deberia estar.
        Err(e) => Err(format!("no se pudo leer `{}`: {e}", ruta.display())),
    }
}

fn filas(peticion: &str) -> Result<String, String> {
    let p = ore_driver::leer_peticion(peticion)?;

    // **El rango, o la negativa.** Un fichero sabe recortar por una columna
    // —es un filtro mas— y no sabe leer un changelog, porque no tiene: guarda
    // el estado presente y nada mas. Las dos cosas se declaran aqui y la
    // comprobacion es del protocolo, no de este driver.
    if let Some(porque) = ore_driver::rango_servible(&p, true, false) {
        return Err(porque);
    }

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
        // **El rango.** `start` es exclusivo: lo que ya estaba en la copia
        // anterior no se vuelve a pedir, y `end` acota por arriba para que el
        // testigo y las filas sean el mismo instante.
        if let Some(cursor) = p.cursor.as_deref() {
            let v = campo(cursor);
            if p.start.as_deref().is_some_and(|s| v <= s) {
                continue;
            }
            if p.end.as_deref().is_some_and(|e| v > e) {
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
