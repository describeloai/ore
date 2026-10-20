//! **Los verbos que son de Phoenix** y no del esqueleto: `catalogo`, `check`, `explorar`, `testigo` y
//! `cambio`. Los cuatro de lectura (`capacidades`, `leer`, `estimar`, `servir`) están en [`crate::lectura`].
//!
//! Cada uno recibe lo que llega por stdin y devuelve la línea que sale por stdout, o el motivo del fallo
//! (que va a stderr, literal, sin la credencial).

use crate::esquema::{Objeto, Tabla as TablaPhoenix};
use crate::lectura::partir_objeto;
use crate::transporte::{Conexion, fallo_de};
use ore_avatica::Valor;
use ore_core::json::Json;
use ore_driver::catalogo::{Catalogo, Columna, Tabla, escribir};
use ore_driver::{Codigo, Fallo};
use std::collections::BTreeSet;
use std::process::ExitCode;

/// El `main` de lo que no son los cuatro verbos v2: lee stdin y despacha.
pub fn otros(verbo: &str, args: &[String]) -> ExitCode {
    let mut entrada = String::new();
    if let Err(e) = std::io::Read::read_to_string(&mut std::io::stdin(), &mut entrada) {
        eprintln!("ore-read-phoenix: no se pudo leer stdin: {e}");
        return ExitCode::FAILURE;
    }
    let hecho = match verbo {
        "catalogo" => catalogo(
            entrada.trim(),
            args.first().map_or("phoenix", String::as_str),
        ),
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

/// **`catalogo`**: cada tabla y vista que la URL deja ver, con los tipos de `getColumns` (no se infiere
/// nada) y su clave. Una tabla que no se deja catalogar no tumba el catálogo: se salta con su aviso y su
/// motivo; solo falla si no se pudo con ninguna. No lleva `filas_estimadas`: el `EXPLAIN` de Phoenix se
/// equivoca (X0 §3.9) y eso es de `estimar`.
pub fn catalogo(url: &str, fuente: &str) -> Result<String, String> {
    if url.is_empty() {
        return Err("no llegó nada por stdin. `catalogo` espera la URL de Phoenix por ahí".into());
    }
    let c = Conexion::abrir(url).map_err(|f| texto(f, url))?;
    let mut tablas = Vec::new();
    let mut saltadas: Vec<String> = Vec::new();
    for o in c.objetos().map_err(|f| texto(f, url))? {
        let objeto = nombre_de(&o);
        match c.tabla(o.esquema.as_deref(), &o.nombre) {
            Ok(t) => {
                for a in avisos(&c, &o, &t) {
                    eprintln!("ore-read-phoenix: aviso · `{objeto}`: {a}");
                }
                tablas.push(tabla_de(&t, &o));
            }
            Err(f) => {
                let no_esta = f.codigo == Codigo::Objeto;
                let mut m = texto(f, url);
                // `getTables` la lista y `getColumns` no la ve: una vista de inquilino (X0 §3.5).
                if no_esta && c.destino.tenant.is_none() {
                    m.push_str("; si es una vista de inquilino, abre con `tenant=`");
                }
                eprintln!("ore-read-phoenix: aviso · `{objeto}` no se cataloga: {m}");
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

fn nombre_de(o: &Objeto) -> String {
    match &o.esquema {
        Some(e) => format!("{e}.{}", o.nombre),
        None => o.nombre.clone(),
    }
}

/// Lo que quien cataloga debe saber de esta tabla y el catálogo no puede llevar.
fn avisos(c: &Conexion, o: &Objeto, t: &TablaPhoenix) -> Vec<String> {
    let mut a = Vec::new();
    if let Some(n) = o.sal {
        a.push(format!(
            "lleva sal ({n} cubos): la clave física no es el orden de la tabla"
        ));
    }
    if o.multi_tenant && c.destino.tenant.is_none() {
        a.push(
            "es multi-tenant y la URL no lleva `tenant=`: lee las filas de todos los inquilinos"
                .into(),
        );
    }
    if t.columnas.iter().any(|k| k.es_date) {
        a.push("tiene columnas `DATE`: se leen como `DateTimeTz` (Phoenix guarda la hora)".into());
    }
    a
}

/// Una tabla o vista de Phoenix, como la tabla del catálogo.
fn tabla_de(t: &TablaPhoenix, o: &Objeto) -> Tabla {
    let clave = t.clave_primaria();
    let columnas = t
        .columnas
        .iter()
        .map(|k| Columna {
            nombre: k.nombre.clone(),
            tipo: Some(k.tipo.oos()),
            // El tipo de Phoenix tal cual (`UNSIGNED_INT`, `INTEGER ARRAY`): con él, el motor sabe lo que el
            // escalar no lleva.
            origen: Some(k.tipo_phoenix.clone()),
            obligatoria: !k.anulable || clave.contains(&k.nombre),
            valores: Vec::new(),
            descripcion: None,
        })
        .collect();
    Tabla {
        nombre: t.objeto(),
        columnas,
        clave,
        clase: if o.vista { "view" } else { "table" }.to_string(),
        lee: Some(lee()),
        cambia: Some(cambios()),
        ..Tabla::default()
    }
}

/// La cara `I`: este origen no empuja nada todavía (M4), y leer una tabla de Phoenix entera es lo que
/// cuesta: `expensive`.
fn lee() -> Json {
    Json::obj([
        ("predicatePushdown", Json::Arr(Vec::new())),
        ("fullScan", Json::s("expensive")),
    ])
}

/// La cara `D`: Phoenix no tiene change streams; el cursor sobre `ROW_TIMESTAMP` es de M7.
fn cambios() -> Json {
    Json::obj([("mode", Json::s("none")), ("witness", Json::s("none"))])
}

/// **`explorar`**: los esquemas con tablas o vistas que la URL deja ver (los de Phoenix no son más que un
/// prefijo del nombre). Lo que `catalogo` recorre si la URL no nombra uno.
pub fn explorar(entrada: &str) -> Result<String, String> {
    let (url, _) = ore_driver::leer_coordenada(entrada)?;
    let c = Conexion::abrir(&url).map_err(|f| texto(f, &url))?;
    let objetos = c.objetos().map_err(|f| texto(f, &url))?;
    let esquemas: BTreeSet<&str> = objetos
        .iter()
        .filter_map(|o| o.esquema.as_deref())
        .collect();
    let sueltas = objetos.iter().filter(|o| o.esquema.is_none()).count();
    let contiene: Vec<Json> = esquemas
        .iter()
        .map(|e| Json::obj([("nombre", Json::s(*e))]))
        .collect();
    let mut nota = match &c.destino.esquema {
        Some(e) => format!("la URL nombra el esquema `{e}`: el catálogo recorre solo ese"),
        None => {
            "la URL no nombra un esquema: el catálogo recorre todos, menos `SYSTEM`".to_string()
        }
    };
    if sueltas > 0 {
        nota.push_str(&format!(
            "; además hay {sueltas} tabla(s) sin esquema (se catalogan con el resto)"
        ));
    }
    Ok(Json::obj([("contiene", Json::Arr(contiene)), ("nota", Json::s(nota))]).pretty())
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
