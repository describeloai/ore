//! `ore-read-bigquery` — las **filas** de BigQuery, la fase ③.
//!
//! # Qué estaba abierto
//!
//! `ore` trae dentro la receta del **catálogo** de BigQuery: ejecuta `bq`, que
//! el usuario ya tiene y ya autenticó, y traduce lo que dice. Para las filas no
//! traía nada — `ore materialize` llama a `ore-read-<tipo> leer` y **no tiene
//! receta interna de ninguna clase**—, así que BigQuery entraba y no salía: se
//! podían espejar tablas y proponer vistas, y ninguna copia se podía poblar.
//!
//! # Por qué un binario si la otra mitad es una receta
//!
//! Porque el reparto de la fase ③ es *un programa por familia*, y no hay ningún
//! sitio en `materializar.rs` donde meter una excepción sin abrirla para todas.
//! Respetar el reparto sale más barato que hacerle un hueco a la primera fuente
//! que lo pida.
//!
//! Y delega en `bq` por lo que la receta ya midió: **la credencial nunca entra
//! en el espacio de direcciones de ORE**. Este programa no abre un socket, no
//! lee un fichero de servicio y no sabe qué es un token. Ejecuta un programa que
//! el usuario ya autenticó y lee su stdout.
//!
//! # Los tres verbos, y por qué uno se niega
//!
//! | verbo | qué hace |
//! |---|---|
//! | `leer` | las filas del fragmento de plan que llega por stdin |
//! | `testigo` | hasta dónde está el origen, **si sabe fecharse** |
//! | `catalogo` | **se niega**: esa mitad vive dentro de `ore` y es la que corre |
//!
//! `lector::catalogo` despacha `bigquery` a su receta y no llega nunca aquí, así
//! que implementarlo sería una segunda versión de lo mismo que nadie puede
//! alcanzar — y dos derivaciones de la misma cosa divergen en la que ninguna
//! prueba ejerce.
//!
//! # Lo que NO está medido contra un dataset real, y se dice
//!
//! La traducción está probada entera y **sin servidor**, que es lo que hace que
//! *«el SQL emitido contiene solo las columnas proyectadas»* sea un aserto: un
//! aserto que exigiera un servidor no se ejecutaría nunca en la suite.
//!
//! Lo que **no** se ha ejercido es la ejecución contra un dataset real, porque
//! no se ha nombrado ninguno. `bq` sí arranca en esta máquina —`This is BigQuery
//! CLI 2.1.36`— y conviene dejar escrito por qué el primer sondeo dijo lo
//! contrario: lanzado **desde Git Bash** contesta `ERROR: (bq) python3.14:
//! command not found`, y lanzado como proceso —que es lo que hacen `ore` y
//! esto— contesta bien. Un lector que no arranca se parece demasiado a uno que
//! falta, y desde el intérprete equivocado se parece a los dos.

mod consultas;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::Read as _;
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let verbo = args.first().map(String::as_str).unwrap_or("leer");

    let mut entrada = String::new();
    if std::io::stdin().read_to_string(&mut entrada).is_err() {
        eprintln!("ore-read-bigquery: no se pudo leer stdin");
        return ExitCode::FAILURE;
    }
    if entrada.trim().is_empty() {
        eprintln!(
            "ore-read-bigquery: no llegó nada por stdin. `leer` espera una petición JSON y \
             `testigo` una coordenada, y las dos van por ahí y no por la línea de órdenes"
        );
        return ExitCode::FAILURE;
    }

    let resultado = match verbo {
        "leer" => filas(&entrada),
        "testigo" => testigo(&entrada),
        // **¿Responde esta fuente?** Se le pide a `bq` lo mas barato que hay:
        // `SELECT 1`. Contesta a la vez por la autenticacion, por el proyecto y
        // por que el propio `bq` arranca — que en esta maquina fue justo el
        // fallo que se disfrazo de otra cosa.
        "check" => match ore_driver::leer_coordenada(&entrada) {
            Err(e) => Err(e),
            Ok((url, _)) => Ok(match proyecto(&url).and_then(|p| {
                bq(
                    &p,
                    &consultas::Invocacion {
                        consulta: "SELECT 1 AS ok".to_string(),
                        parametros: Vec::new(),
                    },
                )
            }) {
                Ok(_) => ore_driver::comprobacion(true, None),
                Err(e) => ore_driver::comprobacion(false, Some(&e)),
            }),
        },
        // **El quinto verbo: que contiene esta fuente.** Y existe por una
        // asimetria real, no por simetria: una URL de BigQuery nombra UN
        // dataset, asi que hay que saberselo antes de declararlo. Las otras dos
        // familias abarcan su fuente entera y no tienen esa pregunta.
        "explorar" => match ore_driver::leer_coordenada(&entrada) {
            Err(e) => Err(e),
            Ok((url, _)) => explorar(&url),
        },
        "catalogo" => Err("`ore` trae la receta del catálogo de BigQuery dentro y es la que \
                           corre: `lector::catalogo` despacha `bigquery` a la suya y no llega \
                           aquí. Lo que implementa este programa es `leer`, que es el verbo de \
                           la fase ③"
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
            eprintln!("ore-read-bigquery: {m}");
            ExitCode::FAILURE
        }
    }
}

// ── La coordenada de la fuente ──────────────────────────────────────────────

/// El proyecto, de la URL. `bigquery://<proyecto>/<dataset>`.
///
/// El dataset no se toma de aquí: viene dentro del objeto —`dataset.tabla`, que
/// es lo que emite el catálogo— y tomarlo de los dos sitios sería el segundo
/// sitio que puede discrepar del primero.
fn proyecto(url: &str) -> Result<String, String> {
    let resto = url.strip_prefix("bigquery://").ok_or_else(|| {
        format!("`{url}` no es una URL de BigQuery: se esperaba `bigquery://<proyecto>/<dataset>`")
    })?;
    let p = resto.split('/').next().unwrap_or("").trim();
    if p.is_empty() {
        return Err(format!("`{url}` no nombra ningún proyecto"));
    }
    Ok(p.to_string())
}

// ── ⑤ · Las filas ───────────────────────────────────────────────────────────

fn filas(peticion: &str) -> Result<String, String> {
    let p = ore_driver::leer_peticion(peticion)?;

    // **El rango, o la negativa.** BigQuery sabe recortar por una columna —es
    // un `WHERE` más— y este driver no sabe leer su historial de cambios: para
    // eso haría falta la función de tabla `CHANGES`, y el rango sería sobre un
    // instante de confirmación y no sobre una columna. Se declara lo que se
    // sabe y la comprobación es del protocolo, no de aquí.
    if let Some(porque) = ore_driver::rango_servible(&p, true, false) {
        return Err(porque);
    }

    let proyecto = proyecto(&p.url)?;
    // La forma es de `ore-sql` y el dialecto una constante suya. Lo que este
    // fichero pone es el objeto YA CUALIFICADO —BigQuery antepone el proyecto y
    // PostgreSQL no tiene nada que anteponer— y el transporte.
    //
    // Y la consulta de tipos va DENTRO de `preparar`: quien decide si hace
    // falta es el dialecto, no este fichero. Antes lo sabia de memoria, que es
    // lo que el tercer driver tendria que recordar.
    let c = ore_sql::preparar(
        &p,
        &ore_sql::dialectos::BIGQUERY,
        &consultas::cualificado(&proyecto, &p.objeto),
        || tipos_de(&proyecto, &p.objeto),
    )?;
    let salida = bq(&proyecto, &consultas::Invocacion {
        consulta: c.texto,
        parametros: c.parametros,
    })?;

    let arbol = ore_core::parse::parse(&salida)
        .map_err(|e| format!("lo que devolvió `bq` no analiza: {e:?}"))?;

    let mut out = String::new();
    for fila in arbol.items() {
        let valores: Vec<Option<String>> = p
            .proyeccion
            .iter()
            .map(|(_, col)| {
                fila.get(col)
                    .and_then(|(_, v)| v.as_str())
                    // `bq --format=prettyjson` escribe `null` para una celda sin
                    // valor, y el analizador de OOS —que lee JSON porque JSON es
                    // un subconjunto de YAML— lo entrega como el texto `null`.
                    // Así que una cadena cuyo contenido sea literalmente `null`
                    // se confunde con la ausencia. Se dice porque es una pérdida
                    // real, y es la de aguas abajo también: `ore_driver::fila`
                    // ya convierte la ausencia en cadena vacía.
                    .filter(|s| *s != "null")
                    .map(String::from)
            })
            .collect();
        out.push_str(&ore_driver::fila(&p, &valores));
        out.push('\n');
    }
    Ok(out.trim_end().to_string())
}

/// Los tipos de las columnas de la tabla, del `INFORMATION_SCHEMA` de su
/// dataset. Es la consulta de más que GoogleSQL obliga a hacer, y el porqué
/// está en [`ore_sql`].
fn tipos_de(proyecto: &str, objeto: &str) -> Result<BTreeMap<String, String>, String> {
    let salida = bq(proyecto, &consultas::tipos(proyecto, objeto)?)?;
    let arbol = ore_core::parse::parse(&salida)
        .map_err(|e| format!("lo que devolvió `bq` para los tipos no analiza: {e:?}"))?;
    let mut out = BTreeMap::new();
    for f in arbol.items() {
        let campo = |k: &str| f.get(k).and_then(|(_, v)| v.as_str()).map(String::from);
        if let (Some(c), Some(t)) = (campo("column_name"), campo("data_type")) {
            out.insert(c, t);
        }
    }
    if out.is_empty() {
        return Err(format!(
            "`{objeto}` no devolvió ninguna columna. O no existe, o la credencial de `bq` no \
             alcanza a su `INFORMATION_SCHEMA`"
        ));
    }
    Ok(out)
}

// ── ③ · El testigo ──────────────────────────────────────────────────────────

/// **Hasta dónde está el origen ahora.**
///
/// Con un *cursor field* declarado, el máximo de esa columna: el mismo modelo
/// que `ore-read-jsonl`, y el que medio sector usa.
///
/// # Y sin él, `none` — que es una respuesta cierta y no una rendición
///
/// El protocolo dice que *«`valor: None` con `modo: "none"` es la respuesta de
/// un origen que no sabe fecharse»*. Este driver no sabe fechar una tabla de
/// BigQuery, y hay dos formas de no saberlo que conviene no confundir:
///
/// - **`log`** es lo que la receta del catálogo emite cuando la tabla tiene el
///   historial de cambios encendido, y su ordinal es un instante de
///   confirmación. Servirlo exigiría leer por `CHANGES`, que es otro camino de
///   lectura entero — y prometer el testigo sin poder servir su rango dejaría
///   una copia que se fecha y no se refresca.
/// - **`snapshot`** se podría sacar de la metadata de almacenamiento, y sería
///   mentir un poco: `leer` no fija `FOR SYSTEM_TIME AS OF`, así que el testigo
///   y las filas no serían el mismo instante. Es justo lo que `snapshot`
///   promete.
///
/// Contestar `none` deja la copia sin fecha y **lo dice**; `materialize` avisa
/// cuando el origen contesta menos de lo que la tabla declara, y ese aviso es
/// preferible a una marca que no respalda nada.
fn testigo(peticion: &str) -> Result<String, String> {
    let (url, objeto) = ore_driver::leer_coordenada(peticion)?;
    let cursor = ore_core::parse::parse(peticion).ok().and_then(|n| {
        n.get("cursor")
            .and_then(|(_, v)| v.as_str())
            .map(String::from)
    });
    let Some(c) = cursor else {
        return Ok(ore_driver::testigo("none", None));
    };
    let proyecto = proyecto(&url)?;
    let salida = bq(&proyecto, &consultas::maximo(&proyecto, &objeto, &c)?)?;
    let arbol = ore_core::parse::parse(&salida)
        .map_err(|e| format!("lo que devolvió `bq` no analiza: {e:?}"))?;
    let maximo = arbol
        .items()
        .first()
        .and_then(|f| f.get("m"))
        .and_then(|(_, v)| v.as_str())
        .filter(|s| *s != "null")
        .map(String::from);
    // Una tabla vacía no tiene máximo, y eso no es un fallo: es que no hay por
    // dónde avanzar todavía.
    Ok(ore_driver::testigo("field", maximo.as_deref()))
}

// ── ⓪ · Que contiene esta fuente ────────────────────────────────────────────

/// **Los datasets del proyecto.**
///
/// `bq ls` y no una consulta: listar datasets no es preguntarle nada a ninguno,
/// y `INFORMATION_SCHEMA` es **por dataset** — para recorrerlos con SQL habria
/// que saberselos ya, que es justo lo que esto viene a contestar.
///
/// Devuelve, por cada uno, la URL que habria que declarar. Que salga hecha no
/// es comodidad: es lo que evita que alguien la componga a mano y se equivoque
/// en el separador.
fn explorar(url: &str) -> Result<String, String> {
    let proyecto = proyecto(url)?;
    let ruta = resolver("bq").ok_or_else(|| {
        "no se encontro `bq` en el PATH. Es el cliente de BigQuery, y viene con el SDK de          Google: este programa no habla con BigQuery, habla con el"
            .to_string()
    })?;
    let salida = Command::new(&ruta)
        .args([
            "ls".to_string(),
            "--format=prettyjson".to_string(),
            "--datasets=true".to_string(),
            "--max_results=1000".to_string(),
            format!("--project_id={proyecto}"),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("no se pudo ejecutar `{}`: {e}", ruta.display()))?;
    if !salida.status.success() {
        return Err(format!(
            "`bq ls` fallo:
{}",
            String::from_utf8_lossy(&salida.stderr).trim()
        ));
    }
    let texto = String::from_utf8_lossy(&salida.stdout).into_owned();
    let arbol = ore_core::parse::parse(&texto)
        .map_err(|e| format!("lo que devolvio `bq ls` no analiza: {e:?}"))?;
    let mut fuera: Vec<ore_core::json::Json> = Vec::new();
    for d in arbol.items() {
        // `datasetReference.datasetId` es lo documentado; `id` —`proyecto:ds`—
        // es el respaldo. Se prueban los dos y no se inventa ninguno.
        let nombre = d
            .get("datasetReference")
            .and_then(|(_, r)| r.get("datasetId"))
            .and_then(|(_, v)| v.as_str())
            .map(String::from)
            .or_else(|| {
                d.get("id")
                    .and_then(|(_, v)| v.as_str())
                    .and_then(|s| s.split_once(':'))
                    .map(|(_, ds)| ds.to_string())
            });
        let Some(n) = nombre else { continue };
        fuera.push(ore_core::json::Json::obj([
            ("nombre", ore_core::json::Json::s(&n)),
            (
                "url",
                ore_core::json::Json::s(format!("bigquery://{proyecto}/{n}")),
            ),
        ]));
    }
    if fuera.is_empty() {
        return Err(format!(
            "`{proyecto}` no tiene ningun dataset visible con esta credencial. Una lista vacia              tendria el mismo aspecto que un proyecto al que no se llega, asi que se dice"
        ));
    }
    Ok(ore_core::json::Json::obj([(
        "contiene",
        ore_core::json::Json::Arr(fuera),
    )])
    .pretty())
}

// ── Ejecutar `bq` ───────────────────────────────────────────────────────────

/// `CreateProcess` no consulta `PATHEXT`, y en Windows `bq` **es un `.cmd`**.
/// Resolver a mano es lo que hace que este programa lo encuentre igual que lo
/// encuentra `ore`.
fn resolver(programa: &str) -> Option<PathBuf> {
    let exts: Vec<OsString> = std::env::var_os("PATHEXT")
        .map(|p| {
            p.to_string_lossy()
                .split(';')
                .filter(|e| !e.is_empty())
                .map(OsString::from)
                .collect()
        })
        .unwrap_or_default();
    for dir in std::env::split_paths(&std::env::var_os("PATH")?) {
        let base = dir.join(programa);
        for e in &exts {
            let mut con = base.clone().into_os_string();
            con.push(e);
            let p = PathBuf::from(con);
            if p.is_file() {
                return Some(p);
            }
        }
        if base.is_file() {
            return Some(base);
        }
    }
    None
}

fn bq(proyecto: &str, i: &consultas::Invocacion) -> Result<String, String> {
    let ruta = resolver("bq").ok_or_else(|| {
        "no se encontró `bq` en el PATH. Es el cliente de BigQuery, y viene con el SDK de \
         Google: este programa no habla con BigQuery, habla con él"
            .to_string()
    })?;
    let mut args: Vec<String> = vec![
        "query".into(),
        "--format=prettyjson".into(),
        "--use_legacy_sql=false".into(),
        "--max_rows=1000000".into(),
        "--quiet".into(),
        format!("--project_id={proyecto}"),
    ];
    args.extend(i.parametros.iter().map(|p| format!("--parameter={p}")));

    let mut hijo = Command::new(&ruta)
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("no se pudo ejecutar `{}`: {e}", ruta.display()))?;
    if let Some(mut s) = hijo.stdin.take() {
        use std::io::Write as _;
        let _ = s.write_all(i.consulta.as_bytes());
    }
    let salida = hijo
        .wait_with_output()
        .map_err(|e| format!("`bq` no terminó: {e}"))?;
    if !salida.status.success() {
        // Su stderr, literal. `bq` avisa de que le falta un intérprete o de que
        // no hay sesión, y las dos cosas se arreglan solas en cuanto se leen:
        // resumirlas convierte un problema de cinco minutos en una tarde.
        return Err(format!(
            "`bq` falló:\n{}",
            String::from_utf8_lossy(&salida.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&salida.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_proyecto_sale_de_la_url_y_el_dataset_no() {
        assert_eq!(proyecto("bigquery://acme/hr").as_deref(), Ok("acme"));
        assert_eq!(proyecto("bigquery://acme").as_deref(), Ok("acme"));
        assert!(proyecto("postgres://x").is_err());
        assert!(proyecto("bigquery:///hr").is_err());
    }
}
