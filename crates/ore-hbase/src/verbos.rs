//! **Los verbos que son de HBase** y no del esqueleto: `catalogo`, `check`, `explorar`, `testigo` y `cambio`. Los
//! cuatro de lectura (`capacidades`, `leer`, `estimar`, `servir`) estan en [`crate::lectura`].
//!
//! Cada uno recibe lo que llega por stdin y devuelve la linea que sale por stdout, o el motivo del fallo (que va a
//! stderr, literal, sin credencial: SIMPLE no tiene).

use crate::esquema::{CLAVE, MARCA, Tabla as TablaHbase, en_hbase, partir_objeto};
use crate::transporte::{Conexion, FILAS_DE_MUESTRA, fallo_de, muestrear};
use ore_base::json::Json;
use ore_driver::catalogo::{Catalogo, Columna, Tabla, escribir};
use ore_driver::{Codigo, Fallo};
use ore_hbase_rpc::proto::TableStateState;
use ore_hbase_rpc::{Cancelacion, Consulta, TablaEnMeta};
use std::collections::BTreeMap;
use std::process::ExitCode;

/// El espacio de nombres de las tablas del sistema (`hbase:meta`…).
const ESPACIO_DEL_SISTEMA: &str = "hbase";

/// El `main` de lo que no son los cuatro verbos v2: lee stdin y despacha.
pub fn otros(verbo: &str, args: &[String]) -> ExitCode {
    let mut entrada = String::new();
    if let Err(e) = std::io::Read::read_to_string(&mut std::io::stdin(), &mut entrada) {
        eprintln!("ore-read-hbase: no se pudo leer stdin: {e}");
        return ExitCode::FAILURE;
    }
    let hecho = match verbo {
        "catalogo" => catalogo(entrada.trim(), args.first().map_or("hbase", String::as_str)),
        "check" => check(&entrada),
        "explorar" => explorar(&entrada),
        "testigo" => testigo(&entrada),
        "cambio" => cambio(&entrada),
        otro => Err(format!(
            "verbo desconocido `{otro}` (capacidades, leer, servir, catalogo, check, explorar, testigo, cambio)"
        )),
    };
    match hecho {
        Ok(salida) => {
            println!("{salida}");
            ExitCode::SUCCESS
        }
        Err(m) => {
            eprintln!("ore-read-hbase: {m}");
            ExitCode::FAILURE
        }
    }
}

fn texto(f: Fallo) -> String {
    f.mensaje
}

/// Las tablas de usuario que la URL deja ver: las de su espacio, o las de todos menos el del sistema.
fn de_usuario(c: &Conexion) -> Result<Vec<TablaEnMeta>, Fallo> {
    let todas = c.cliente.tablas().map_err(fallo_de)?;
    Ok(todas
        .into_iter()
        .filter(|t| match &c.destino.espacio {
            Some(e) => &t.espacio == e,
            None => t.espacio != ESPACIO_DEL_SISTEMA,
        })
        .collect())
}

fn deshabilitada(t: &TablaEnMeta) -> bool {
    matches!(
        t.estado,
        Some(TableStateState::Disabled | TableStateState::Disabling)
    )
}

/// **`check`**: ¿responde esta fuente? Paso a paso, y el motivo del fallo va **literal**: ZooKeeper dice donde esta
/// `hbase:meta`, `hbase:meta` lista las tablas y, si la URL nombra un espacio, se lee **una fila de una** (el
/// permiso de lectura).
pub fn check(entrada: &str) -> Result<String, String> {
    let (url, _) = ore_driver::leer_coordenada(entrada)?;
    let hecho = (|| -> Result<(), Fallo> {
        let c = Conexion::abrir(&url)?;
        let tablas = de_usuario(&c)?;
        if let Some(e) = &c.destino.espacio {
            let Some(t) = tablas.iter().find(|t| !deshabilitada(t)) else {
                return Err(Fallo::new(
                    Codigo::Objeto,
                    format!(
                        "el espacio `{e}` no existe o no tiene tablas habilitadas (los nombres distinguen mayusculas)"
                    ),
                ));
            };
            let consulta = Consulta {
                filas_por_llamada: 1,
                filtro: Some((
                    "org.apache.hadoop.hbase.filter.FirstKeyOnlyFilter".to_string(),
                    Vec::new(),
                )),
                ..Consulta::default()
            };
            let mut l = c
                .cliente
                .leer(
                    &en_hbase(&t.espacio, &t.nombre),
                    &consulta,
                    &Cancelacion::nueva(),
                )
                .map_err(fallo_de)?;
            if let Some(f) = l.next() {
                f.map_err(fallo_de)?;
            }
        }
        Ok(())
    })();
    Ok(match hecho {
        Ok(()) => ore_driver::comprobacion(true, None),
        Err(f) => ore_driver::comprobacion(false, Some(&crate::url::publica(&texto(f)))),
    })
}

/// La cara `I`: HBase empuja el rango de la clave y evalua el resto en el conector, con la semantica de SQL. Leer
/// una tabla entera es lo que cuesta: `expensive`.
fn lee() -> Json {
    Json::obj([
        (
            "predicatePushdown",
            Json::Arr(
                crate::lectura::Hbase::CAPACIDADES_OPERADORES
                    .iter()
                    .map(|o| Json::s(*o))
                    .collect(),
            ),
        ),
        ("fullScan", Json::s("expensive")),
    ])
}

/// La cara `D`: HBase no tiene change streams, y el catalogo **no declara incremental**: lo declara quien lo quiere,
/// con `changes: { mode: upsert, key: [_clave], witness: field, field: _ts }` sobre la columna `_ts`. Ve las
/// actualizaciones y no los borrados (ficha 0072, N5), y eso lo decide el dueño de la tabla, no el descubrimiento.
fn cambios() -> Json {
    Json::obj([("mode", Json::s("none")), ("witness", Json::s("none"))])
}

/// Una tabla muestreada, como la tabla del catalogo.
fn tabla_de(t: &TablaHbase) -> Tabla {
    // El `origen` de una columna (su `physicalType` en la tabla) es **la codificacion de sus bytes**: el motor lo
    // manda en `codificaciones` y el conector lo lee. Una descripcion no puede ir ahi.
    let mut columnas = vec![Columna {
        nombre: CLAVE.to_string(),
        tipo: Some(t.clave.oos.clone()),
        origen: Some(t.clave.cod.nombre().to_string()),
        obligatoria: true,
        valores: Vec::new(),
        descripcion: Some("la clave de fila de HBase (bytes)".to_string()),
    }];
    columnas.push(Columna {
        nombre: MARCA.to_string(),
        tipo: Some("DateTime".to_string()),
        origen: Some("epoch-ms".to_string()),
        obligatoria: false,
        valores: Vec::new(),
        descripcion: Some(
            "la marca de tiempo de la fila (la mayor de sus celdas), no un dato guardado: es el cursor del incremental \
             (`witness: field`, `field: _ts`). Ve las actualizaciones y no los borrados; leerla pide las filas enteras"
                .to_string(),
        ),
    });
    columnas.extend(t.columnas.iter().map(|k| Columna {
        nombre: k.nombre.clone(),
        tipo: Some(k.tipo.oos.clone()),
        origen: Some(k.tipo.cod.nombre().to_string()),
        obligatoria: false,
        valores: Vec::new(),
        descripcion: Some(format!(
            "celda de HBase (bytes); con valor en {} de {} filas muestreadas. Si son numeros, fechas o decimales,              declara su tipo y pon su codificacion en `physicalType`",
            k.filas_con_valor, t.filas_muestreadas
        )),
    }));
    Tabla {
        nombre: t.objeto(),
        columnas,
        clave: vec![CLAVE.to_string()],
        clase: "table".to_string(),
        lee: Some(lee()),
        cambia: Some(cambios()),
        ..Tabla::default()
    }
}

/// **`catalogo`**: cada tabla habilitada que la URL deja ver, con las columnas que una **muestra** de filas
/// (repartida por regiones) enseña. HBase no tiene esquema: lo que no sale en la muestra no figura, y el catalogo lo
/// dice. Una tabla que no se deja catalogar no tumba el catalogo: se salta con su aviso y su motivo; solo falla si
/// no se pudo con ninguna.
pub fn catalogo(url: &str, fuente: &str) -> Result<String, String> {
    if url.is_empty() {
        return Err("no llego nada por stdin. `catalogo` espera la URL de HBase por ahi".into());
    }
    let c = Conexion::abrir(url).map_err(texto)?;
    let tablas_meta = de_usuario(&c).map_err(texto)?;
    let mut tablas = Vec::new();
    let mut saltadas: Vec<String> = Vec::new();
    let cancelacion = Cancelacion::nueva();
    for m in &tablas_meta {
        let objeto = format!("{}.{}", m.espacio, m.nombre);
        if deshabilitada(m) {
            eprintln!("ore-read-hbase: aviso · `{objeto}` esta deshabilitada: no se cataloga");
            continue;
        }
        match muestrear(
            &c.cliente,
            &m.espacio,
            &m.nombre,
            FILAS_DE_MUESTRA,
            &cancelacion,
        ) {
            Ok(t) => {
                eprintln!(
                    "ore-read-hbase: aviso · `{objeto}`: {} columna(s) de {} fila(s) muestreadas en {} region(es); \
                     lo que no sale en la muestra no figura",
                    t.columnas.len(),
                    t.filas_muestreadas,
                    t.regiones
                );
                for a in &t.avisos {
                    eprintln!("ore-read-hbase: aviso · `{objeto}`: {a}");
                }
                tablas.push(tabla_de(&t));
            }
            Err(e) => {
                let m = texto(fallo_de(e));
                eprintln!("ore-read-hbase: aviso · `{objeto}` no se cataloga: {m}");
                saltadas.push(m);
            }
        }
    }
    if tablas.is_empty()
        && let Some(m) = saltadas.first()
    {
        return Err(format!(
            "ninguna de las {} tablas se pudo catalogar; la primera: {m}",
            saltadas.len()
        ));
    }
    Ok(escribir(&Catalogo {
        fuente: fuente.to_string(),
        tablas,
        objetos: Vec::new(),
    }))
}

/// **`explorar`**: los espacios de nombres con tablas que la URL deja ver.
pub fn explorar(entrada: &str) -> Result<String, String> {
    let (url, _) = ore_driver::leer_coordenada(entrada)?;
    let c = Conexion::abrir(&url).map_err(texto)?;
    let tablas = de_usuario(&c).map_err(texto)?;
    let mut por_espacio: BTreeMap<&str, usize> = BTreeMap::new();
    for t in &tablas {
        *por_espacio.entry(t.espacio.as_str()).or_default() += 1;
    }
    let contiene: Vec<Json> = por_espacio
        .iter()
        .map(|(e, n)| Json::obj([("nombre", Json::s(*e)), ("tablas", Json::s(n.to_string()))]))
        .collect();
    let nota = match &c.destino.espacio {
        Some(e) => format!("la URL nombra el espacio `{e}`: el catalogo recorre solo ese"),
        None => {
            "la URL no nombra un espacio: el catalogo recorre todos, menos `hbase` (el del sistema)"
                .to_string()
        }
    };
    Ok(Json::obj([("contiene", Json::Arr(contiene)), ("nota", Json::s(nota))]).pretty())
}

/// **`testigo`**: hasta donde esta el origen. HBase no sabe fecharse por posicion, pero toda fila tiene marca de
/// tiempo: con `cursor: _ts` el testigo es un **limite**, `reloj − rezago` en milisegundos (`field`), porque una lectura
/// con `end = T` entrega todo lo que tenia marca ≤ T y HBase no tiene `ORDER BY` por marca para buscar el maximo.
/// Sin `cursor`, `none`, que es una respuesta cierta (ADR 0016): devolver «ahora» inventaria una marca que el origen
/// no respalda.
pub fn testigo(entrada: &str) -> Result<String, String> {
    let (url, objeto) = ore_driver::leer_coordenada(entrada)?;
    let cursor = ore_base::parse::parse(entrada).ok().and_then(|n| {
        n.get("cursor")
            .and_then(|(_, v)| v.as_str())
            .map(String::from)
    });
    // Se abre para decir que el origen responde antes de decir que no sabe fecharse.
    let c = Conexion::abrir(&url).map_err(texto)?;
    c.cliente
        .servidor_de_meta()
        .map_err(|e| texto(fallo_de(e)))?;
    let ahora = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
    testigo_de(cursor.as_deref(), &objeto, ahora, c.destino.rezago_ms)
}

/// El testigo, con el reloj y el rezago ya dichos (puro).
pub fn testigo_de(
    cursor: Option<&str>,
    objeto: &str,
    ahora_ms: u64,
    rezago_ms: u64,
) -> Result<String, String> {
    match cursor {
        None => Ok(ore_driver::testigo("none", None)),
        Some(MARCA) => Ok(ore_driver::testigo(
            "field",
            Some(&ahora_ms.saturating_sub(rezago_ms).to_string()),
        )),
        Some(otro) => Err(format!(
            "un rango sobre `{otro}` de `{objeto}`: HBase solo sabe avanzar por la marca de tiempo de la fila \
             (`{MARCA}`); sobre otra columna seria un barrido entero que se llama incremental"
        )),
    }
}

/// **`cambio`**: ¿cambio ESTA tabla desde la ultima copia? HBase no tiene contadores por tabla que cuenten
/// escrituras: no hay huella que respalde, y la copia no se fia. Si la tabla no esta es un error, no «no respalda».
pub fn cambio(entrada: &str) -> Result<String, String> {
    let (url, objeto) = ore_driver::leer_coordenada(entrada)?;
    let (espacio, tabla) = partir_objeto(&objeto).map_err(texto)?;
    let c = Conexion::abrir(&url).map_err(texto)?;
    c.cliente
        .regiones(&en_hbase(&espacio, &tabla))
        .map_err(|e| texto(fallo_de(e)))?;
    Ok(ore_driver::huella("sin-contadores", None, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_testigo_de_la_marca_es_el_reloj_menos_el_rezago() {
        let t = testigo_de(Some("_ts"), "default.t", 1_700_000_010_000, 5_000).unwrap();
        assert!(
            t.contains("\"modo\":\"field\"") && t.contains("\"valor\":\"1700000005000\""),
            "{t}"
        );
        // el rezago nunca da un valor negativo
        let t = testigo_de(Some("_ts"), "default.t", 100, 5_000).unwrap();
        assert!(t.contains("\"valor\":\"0\""), "{t}");
    }

    #[test]
    fn sin_cursor_no_sabe_fecharse_y_con_otro_cursor_se_niega() {
        let t = testigo_de(None, "default.t", 1, 0).unwrap();
        assert!(
            t.contains("\"modo\":\"none\"") && !t.contains("valor"),
            "{t}"
        );
        let e = testigo_de(Some("f:fecha"), "default.t", 1, 0).unwrap_err();
        assert!(e.contains("_ts") && e.contains("f:fecha"), "{e}");
    }
}
