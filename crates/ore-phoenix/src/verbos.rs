//! **Los verbos que son de Phoenix** y no del esqueleto: `check`, `testigo` y `cambio`. Los cuatro de lectura
//! (`capacidades`, `leer`, `estimar`, `servir`) están en [`crate::lectura`]. `catalogo` y `explorar` son de M3
//! (ficha 0071 §6): se contestan con un error que lo dice, no con un catálogo a medias.
//!
//! Cada uno recibe lo que llega por stdin y devuelve la línea que sale por stdout, o el motivo del fallo
//! (que va a stderr, literal, sin la credencial).

use crate::lectura::partir_objeto;
use crate::transporte::{Conexion, fallo_de};
use ore_avatica::Valor;
use ore_driver::{Codigo, Fallo};
use std::process::ExitCode;

/// El `main` de lo que no son los cuatro verbos v2: lee stdin y despacha.
pub fn otros(verbo: &str, _args: &[String]) -> ExitCode {
    let mut entrada = String::new();
    if let Err(e) = std::io::Read::read_to_string(&mut std::io::stdin(), &mut entrada) {
        eprintln!("ore-read-phoenix: no se pudo leer stdin: {e}");
        return ExitCode::FAILURE;
    }
    let hecho = match verbo {
        "check" => check(&entrada),
        "testigo" => testigo(&entrada),
        "cambio" => cambio(&entrada),
        "catalogo" | "explorar" => Err(format!(
            "`{verbo}` todavía no está en este origen: es del hito M3 de la ficha 0071 (se lee con `leer` si ya conoces la tabla)"
        )),
        otro => Err(format!(
            "verbo desconocido `{otro}` (capacidades, leer, servir, check, testigo, cambio)"
        )),
    };
    match hecho {
        Ok(salida) => {
            println!("{salida}");
            ExitCode::SUCCESS
        }
        Err(m) => {
            eprintln!("ore-read-phoenix: {m}");
            ExitCode::FAILURE
        }
    }
}

/// Un fallo del conector, como el texto que `ore` enseña: sin la credencial.
fn texto(f: Fallo, url: &str) -> String {
    f.tapado(url).mensaje
}

fn texto_de(v: &Valor) -> Option<&str> {
    match v {
        Valor::Texto(s) => Some(s),
        _ => None,
    }
}

/// **`check`**: ¿responde esta fuente? Paso a paso, y el motivo del fallo va **literal** (es lo único
/// accionable): abre (red, TLS, credencial), pregunta los esquemas y, si la URL nombra uno, lista sus tablas y
/// lee **una fila de una** (el permiso de lectura).
pub fn check(entrada: &str) -> Result<String, String> {
    let (url, _) = ore_driver::leer_coordenada(entrada)?;
    let hecho = (|| -> Result<(), Fallo> {
        let c = Conexion::abrir(&url)?;
        c.avatica.esquemas(None).map_err(fallo_de)?;
        if let Some(e) = &c.destino.esquema {
            let t = c
                .avatica
                .tablas(Some(e), None, &["TABLE", "VIEW"])
                .map_err(fallo_de)?;
            let (ie, it) = (t.indice("TABLE_SCHEM"), t.indice("TABLE_NAME"));
            let (Some(ie), Some(it)) = (ie, it) else {
                return Err(Fallo::origen(
                    "`getTables` no trae TABLE_SCHEM y TABLE_NAME: no es un Phoenix que se entienda",
                ));
            };
            // El patrón trata `_` como comodín: solo vale el esquema exacto.
            let primera = t
                .filas
                .iter()
                .find(|f| texto_de(&f[ie]) == Some(e.as_str()))
                .and_then(|f| texto_de(&f[it]).map(str::to_string));
            let Some(nombre) = primera else {
                return Err(Fallo::new(
                    Codigo::Objeto,
                    format!(
                        "el esquema `{e}` no existe o no tiene tablas (los nombres distinguen mayúsculas)"
                    ),
                ));
            };
            let tabla = c.tabla(Some(e), &nombre)?;
            c.consulta_entera(&format!("SELECT 1 FROM {} LIMIT 1", tabla.objeto_citado()))?;
        }
        Ok(())
    })();
    Ok(match hecho {
        Ok(()) => ore_driver::comprobacion(true, None),
        Err(f) => ore_driver::comprobacion(false, Some(&texto(f, &url))),
    })
}

/// **`testigo`**: Phoenix no sabe fecharse por tabla. Una columna `ROW_TIMESTAMP` daría un cursor (M7), pero no
/// una posición del origen que respalde una copia; devolver «ahora» inventaría una marca que el origen no
/// respalda: contesta `none`, que es una respuesta cierta (ADR 0016).
pub fn testigo(entrada: &str) -> Result<String, String> {
    let (url, _) = ore_driver::leer_coordenada(entrada)?;
    // Se abre para decir que el origen responde antes de decir que no sabe fecharse.
    Conexion::abrir(&url).map_err(|f| texto(f, &url))?;
    Ok(ore_driver::testigo("none", None))
}

/// **`cambio`** (ADR 0063 S3): ¿cambió ESTA tabla desde la última copia? Phoenix no tiene contadores por tabla
/// que cuenten escrituras, así que **no hay huella que respalde**: `respalda: false`, y la copia no se fía. Si
/// la tabla no está es un error, no «no respalda».
pub fn cambio(entrada: &str) -> Result<String, String> {
    let (url, objeto) = ore_driver::leer_coordenada(entrada)?;
    let (esquema, nombre) = partir_objeto(&objeto).map_err(|f| texto(f, &url))?;
    let c = Conexion::abrir(&url).map_err(|f| texto(f, &url))?;
    c.tabla(esquema, nombre).map_err(|f| texto(f, &url))?;
    Ok(ore_driver::huella("sin-contadores", None, false))
}
