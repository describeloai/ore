//! `ore-read-bigquery` — las **filas** de BigQuery, la fase ③.
//!
//! # Qué estaba abierto
//!
//! `ore` trae dentro la receta del **catálogo** de BigQuery. Para las filas no
//! traía nada —`ore materialize` llama a `ore-read-<tipo> leer` y **no tiene
//! receta interna de ninguna clase**—, así que BigQuery entraba y no salía: se
//! podían espejar tablas y proponer vistas, y ninguna copia se podía poblar.
//!
//! # Por qué un binario si la otra mitad es una receta
//!
//! Porque el reparto de la fase ③ es *un programa por familia*, y no hay ningún
//! sitio en `materializar.rs` donde meter una excepción sin abrirla para todas.
//!
//! # Por REST, y no por `bq` (A2, 2026-09-26)
//!
//! Hasta A2 delegaba en el CLI `bq` para que la credencial no entrara en este
//! proceso. Contra un dataset real (`pruebas-de-fuego/bigquery-real.sh`) se
//! midió lo que costaba: el texto `'null'` se volvía NULL, un TIMESTAMP perdía
//! los microsegundos y llegaba sin zona —la copia lo dejaba como texto—, y cada
//! llamada tardaba 8-11 s en arrancar el intérprete de `bq`. Por REST la misma
//! consulta tarda 0,5-0,9 s y los valores llegan tipados ([`valores`]).
//!
//! La credencial es el token de la cuenta que corre (`ore-gcp`): el del
//! metadata server, renovado; en local `ORE_GCP_TOKEN`. Antes vivía en el
//! proceso de `bq`, en el mismo pod; ahora en este. **Acotarla no sirve**:
//! BigQuery ignora la Credential Access Boundary (medido). La frontera es el
//! IAM de la cuenta de servicio del driver.
//!
//! # Los verbos
//!
//! | verbo | qué hace |
//! |---|---|
//! | `leer` | las filas del fragmento de plan que llega por stdin, **página a página** |
//! | `testigo` | hasta dónde está el origen, **si sabe fecharse** |
//! | `check` | `SELECT 1`, sin crear un job |
//! | `explorar` | los datasets del proyecto, todas las páginas |
//! | `catalogo` | **se niega** todavía: esa mitad vive dentro de `ore` (A3 la muda) |
//!
//! # La truncación que no avisa
//!
//! El CLI cortaba en `--max_rows` sin decirlo y este driver pedía una fila de
//! más para notarlo, con un tope de un millón. Por REST no hay tope: se pagina
//! hasta el final, cada página se escribe según llega, y al terminar las filas
//! contadas tienen que ser el `totalRows` que dijo el servidor ([`rest`]).

mod consultas;
mod rest;
mod valores;

use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};
use std::process::ExitCode;

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
    let resultado = rest::Http::del_entorno().and_then(|http| verbo_(&http, verbo, &entrada));
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

fn verbo_(t: &dyn rest::Transporte, verbo: &str, entrada: &str) -> Result<String, String> {
    match verbo {
        "leer" => {
            let salida = std::io::stdout();
            let mut salida = std::io::BufWriter::new(salida.lock());
            filas(t, entrada, &mut salida)?;
            salida
                .flush()
                .map_err(|e| format!("no se pudo escribir la salida: {e}"))?;
            Ok(String::new())
        }
        "testigo" => testigo(t, entrada),
        // **¿Responde esta fuente?** Lo más barato que hay: `SELECT 1`, sin
        // job. Contesta a la vez por el token, por el proyecto y por el permiso
        // de lanzar consultas en él.
        "check" => {
            let (url, _) = ore_driver::leer_coordenada(entrada)?;
            let r = proyecto(&url).and_then(|p| {
                rest::consultar(
                    t,
                    &p,
                    &rest::Consulta {
                        texto: "SELECT 1 AS ok",
                        parametros: &[],
                        sin_job: true,
                    },
                    |_, _| Ok(()),
                )
            });
            Ok(match r {
                Ok(_) => ore_driver::comprobacion(true, None),
                Err(e) => ore_driver::comprobacion(false, Some(&e)),
            })
        }
        // **Qué contiene esta fuente.** Existe por una asimetría real: una URL
        // de BigQuery nombra UN dataset, así que hay que sabérselo antes de
        // declararlo. Las otras dos familias abarcan su fuente entera.
        "explorar" => {
            let (url, _) = ore_driver::leer_coordenada(entrada)?;
            explorar(t, &url)
        }
        "catalogo" => Err(
            "`ore` trae la receta del catálogo de BigQuery dentro y es la que \
                           corre: `lector::catalogo` despacha `bigquery` a la suya y no llega \
                           aquí. Lo que implementa este programa es `leer`, que es el verbo de \
                           la fase ③"
                .to_string(),
        ),
        otro => Err(format!("`{otro}` no es un verbo de este lector")),
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

fn filas(
    t: &dyn rest::Transporte,
    peticion: &str,
    salida: &mut dyn std::io::Write,
) -> Result<u64, String> {
    let p = ore_driver::leer_peticion(peticion)?;

    // **El rango, o la negativa.** BigQuery sabe recortar por una columna —es
    // un `WHERE` más— y este driver no sabe leer su historial de cambios: para
    // eso haría falta la función de tabla `CHANGES`, y el rango sería sobre un
    // instante de confirmación y no sobre una columna.
    if let Some(porque) = ore_driver::rango_servible(&p, true, false) {
        return Err(porque);
    }

    let proyecto = proyecto(&p.url)?;
    // La forma es de `ore-sql`, y quien decide si hacen falta los tipos es el
    // dialecto, no este fichero.
    let c = ore_sql::preparar(
        &p,
        &ore_sql::dialectos::BIGQUERY,
        &consultas::cualificado(&proyecto, &p.objeto),
        || tipos_de(t, &proyecto, &p.objeto),
    )?;

    rest::consultar(
        t,
        &proyecto,
        &rest::Consulta {
            texto: &c.texto,
            parametros: &c.parametros,
            sin_job: false,
        },
        |campos, pagina| {
            // Por nombre y no por posición: la proyección se de-duplica, así
            // que la columna *i* del resultado no es la propiedad *i*.
            let indice: BTreeMap<&str, usize> = campos
                .iter()
                .enumerate()
                .filter_map(|(i, c)| c["name"].as_str().map(|n| (n, i)))
                .collect();
            for fila in pagina {
                let celdas = fila["f"].as_array().map(Vec::as_slice).unwrap_or(&[]);
                let mut valores = Vec::with_capacity(p.proyeccion.len());
                for (_, col) in &p.proyeccion {
                    let i = *indice
                        .get(col.as_str())
                        .ok_or_else(|| format!("BigQuery no devolvió la columna `{col}`"))?;
                    valores.push(valores::texto(&campos[i], &celdas[i]["v"])?);
                }
                writeln!(salida, "{}", ore_driver::fila(&p, &valores))
                    .map_err(|e| format!("no se pudo escribir la fila: {e}"))?;
            }
            Ok(())
        },
    )
}

/// Los tipos de las columnas de la tabla, de `tables.get`: sin job y sin coste.
/// En los nombres de GoogleSQL, que es lo que un parámetro espera.
fn tipos_de(
    t: &dyn rest::Transporte,
    proyecto: &str,
    objeto: &str,
) -> Result<BTreeMap<String, String>, String> {
    let (dataset, tabla) = consultas::partes(objeto)?;
    let campos = rest::esquema(t, proyecto, dataset, tabla)?;
    let out: BTreeMap<String, String> = campos
        .iter()
        .filter_map(|c| {
            Some((
                c["name"].as_str()?.to_string(),
                valores::estandar(c["type"].as_str()?).to_string(),
            ))
        })
        .collect();
    if out.is_empty() {
        return Err(format!("`{objeto}` no tiene ninguna columna"));
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
/// - **`log`** es lo que la receta del catálogo emite cuando la tabla tiene el
///   historial de cambios encendido. Servirlo exigiría leer por `CHANGES`, que
///   es otro camino de lectura entero (Fase B).
/// - **`snapshot`** exigiría que `leer` fijara `FOR SYSTEM_TIME AS OF`, y no lo
///   hace: el testigo y las filas no serían el mismo instante.
fn testigo(t: &dyn rest::Transporte, peticion: &str) -> Result<String, String> {
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
    let mut maximo: Option<String> = None;
    rest::consultar(
        t,
        &proyecto,
        &rest::Consulta {
            texto: &consultas::maximo(&proyecto, &objeto, &c)?,
            parametros: &[],
            sin_job: true,
        },
        |_, filas| {
            // `m` es la única columna; una tabla vacía da NULL, y eso no es un
            // fallo: es que no hay por dónde avanzar todavía.
            if let Some(f) = filas.first() {
                maximo = f["f"][0]["v"].as_str().map(String::from);
            }
            Ok(())
        },
    )?;
    Ok(ore_driver::testigo("field", maximo.as_deref()))
}

// ── ⓪ · Qué contiene esta fuente ────────────────────────────────────────────

/// **Los datasets del proyecto**, con la URL que habría que declarar. Que salga
/// hecha evita que alguien la componga a mano y se equivoque en el separador.
fn explorar(t: &dyn rest::Transporte, url: &str) -> Result<String, String> {
    use ore_core::json::Json;
    let proyecto = proyecto(url)?;
    let nombres = rest::datasets(t, &proyecto)?;
    if nombres.is_empty() {
        return Err(format!(
            "`{proyecto}` no tiene ningún dataset visible con esta credencial. Una lista vacía \
             tendría el mismo aspecto que un proyecto al que no se llega, así que se dice"
        ));
    }
    let fuera = nombres
        .iter()
        .map(|n| {
            Json::obj([
                ("nombre", Json::s(n)),
                ("url", Json::s(format!("bigquery://{proyecto}/{n}"))),
            ])
        })
        .collect();
    Ok(Json::obj([("contiene", Json::Arr(fuera))]).pretty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rest::pruebas::{Guion, grabada};

    #[test]
    fn el_proyecto_sale_de_la_url_y_el_dataset_no() {
        assert_eq!(proyecto("bigquery://acme/hr").as_deref(), Ok("acme"));
        assert_eq!(proyecto("bigquery://acme").as_deref(), Ok("acme"));
        assert!(proyecto("postgres://x").is_err());
        assert!(proyecto("bigquery:///hr").is_err());
    }

    /// `leer` de punta a punta contra lo grabado: `tables.get` para los tipos
    /// (el filtro los pide) y la consulta; la salida es la fila de
    /// `ore_driver::fila`, con el nulo ausente y el texto `null` intacto.
    #[test]
    fn leer_emite_las_filas_de_la_semilla() {
        let g = Guion::new(vec![
            Ok(grabada("pedidos-tables-get")),
            Ok(grabada("pedidos-query")),
        ]);
        let peticion = r#"{"url":"bigquery://p/ventas","objeto":"ventas.pedidos",
            "proyeccion":{"id":"id","total":"total","ts":"ts"},
            "filtros":[{"columna":"id","operador":"gt","valor":"ore-e2e-"}]}"#;
        let mut salida = Vec::new();
        let n = filas(&g, peticion, &mut salida).unwrap();
        let texto = String::from_utf8(salida).unwrap();
        let lineas: Vec<&str> = texto.lines().collect();
        assert_eq!(n, 8);
        assert_eq!(
            lineas[1],
            r#"{"id":"ore-e2e-p2","total":"0","ts":"2026-09-02 23:59:59.123456+00"}"#
        );
        assert_eq!(lineas[4], r#"{"id":"ore-e2e-p5"}"#);
        let pedidas = g.pedidas.borrow();
        assert!(pedidas[0].starts_with("GET projects/p/datasets/ventas/tables/pedidos"));
        assert!(pedidas[1].contains(r#""type":"STRING""#), "{}", pedidas[1]);
    }

    #[test]
    fn el_texto_null_de_clientes_sigue_siendo_texto() {
        let g = Guion::new(vec![
            Ok(grabada("clientes-tables-get")),
            Ok(grabada("clientes-query")),
        ]);
        let peticion = r#"{"url":"bigquery://p/ventas","objeto":"ventas.clientes",
            "proyeccion":{"id":"id","pais":"pais"}}"#;
        let mut salida = Vec::new();
        filas(&g, peticion, &mut salida).unwrap();
        let texto = String::from_utf8(salida).unwrap();
        assert!(
            texto.contains(r#"{"id":"ore-e2e-c4","pais":"null"}"#),
            "{texto}"
        );
        assert!(texto.contains(r#"{"id":"ore-e2e-c3"}"#), "{texto}");
    }

    #[test]
    fn explorar_da_la_url_hecha() {
        let g = Guion::new(vec![Ok(grabada("datasets-list"))]);
        let s = explorar(&g, "bigquery://p").unwrap();
        assert!(s.contains("bigquery://p/ventas"), "{s}");
    }
}
