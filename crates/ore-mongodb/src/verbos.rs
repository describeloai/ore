//! **Los verbos que son de MongoDB** y no del esqueleto: `catalogo`, `check`,
//! `explorar` y `testigo`. Los cuatro verbos de lectura (`capacidades`,
//! `leer`, `estimar`, `servir`) están en [`crate::lectura`].
//!
//! Cada uno recibe lo que llega por stdin y devuelve la línea que sale por
//! stdout, o el motivo del fallo (que va a stderr, literal).

use crate::lectura::inferir;
use crate::transporte::{Coleccion, Conexion, TipoColeccion};
use crate::url::base_de;
use ore_base::json::Json;
use ore_driver::catalogo::{Catalogo, Columna, Tabla, escribir};
use std::process::ExitCode;
use std::time::Duration;

/// Las bases del sistema, que `catalogo` no recorre.
const DEL_SISTEMA: &[&str] = &["admin", "local", "config"];

/// Lo que se espera a la muestra de cada colección al catalogar.
const ESPERA_MUESTRA: Duration = Duration::from_secs(15);

/// El `main` de lo que no son los cuatro verbos v2: lee stdin y despacha.
pub fn otros(verbo: &str, args: &[String]) -> ExitCode {
    let mut entrada = String::new();
    if let Err(e) = std::io::Read::read_to_string(&mut std::io::stdin(), &mut entrada) {
        eprintln!("ore-read-mongodb: no se pudo leer stdin: {e}");
        return ExitCode::FAILURE;
    }
    let hecho = match verbo {
        "catalogo" => catalogo(
            entrada.trim(),
            args.first().map_or("mongodb", String::as_str),
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
            eprintln!("ore-read-mongodb: {m}");
            ExitCode::FAILURE
        }
    }
}

/// Un fallo del conector, como el texto que `ore` enseña: sin la credencial.
fn texto(f: ore_driver::Fallo, url: &str) -> String {
    f.tapado(url).mensaje
}

fn bases_a_recorrer(c: &Conexion, url: &str) -> Result<Vec<String>, String> {
    if let Some(b) = base_de(url) {
        return Ok(vec![b]);
    }
    Ok(c.bases()
        .map_err(|f| texto(f, url))?
        .into_iter()
        .filter(|b| !DEL_SISTEMA.contains(&b.as_str()))
        .collect())
}

/// **`catalogo`**: cada colección y vista de las bases, con el esquema que su
/// muestra deja inferir. Una colección vacía se cataloga sin columnas y se
/// avisa: no hay nada que inferir, y callarlo la haría pasar por una sin campos.
///
/// **Una colección que no se deja muestrear no tumba el catálogo**: se salta con
/// su aviso y su motivo (una vista cuyo pipeline tarda, un permiso que falta en
/// esa colección). Solo falla si no se pudo con ninguna: ahí el motivo es de
/// todas y es lo único accionable.
pub fn catalogo(url: &str, fuente: &str) -> Result<String, String> {
    if url.is_empty() {
        return Err("no llegó nada por stdin. `catalogo` espera la URL de MongoDB por ahí".into());
    }
    let c = Conexion::abrir(url).map_err(|f| texto(f, url))?;
    // La cara `D` es un hecho del servidor, no una preferencia: sin replica set no hay
    // change streams y decir otra cosa sería inventarlo (como `wal_level` en Postgres).
    let con_cambios = c.admite_cambios().unwrap_or(false);
    if !con_cambios {
        eprintln!(
            "ore-read-mongodb: aviso · el servidor no es un replica set: no tiene change streams, así que ninguna colección declara cambios"
        );
    }
    let mut tablas = Vec::new();
    let mut saltadas: Vec<String> = Vec::new();
    for base in bases_a_recorrer(&c, url)? {
        for col in c.colecciones(&base).map_err(|f| texto(f, url))? {
            match tabla_de(&c, &base, &col, con_cambios) {
                Ok(t) => tablas.push(t),
                Err(f) => {
                    let m = texto(f, url);
                    eprintln!(
                        "ore-read-mongodb: aviso · `{base}.{}` no se cataloga: {m}",
                        col.nombre
                    );
                    saltadas.push(m);
                }
            }
        }
    }
    if tablas.is_empty()
        && let Some(m) = saltadas.first()
    {
        return Err(format!(
            "ninguna de las {} colecciones se pudo catalogar; la primera: {m}",
            saltadas.len()
        ));
    }
    Ok(escribir(&Catalogo {
        fuente: fuente.to_string(),
        tablas,
        objetos: Vec::new(),
    }))
}

/// Una colección, como la tabla que su muestra deja ver.
fn tabla_de(
    c: &Conexion,
    base: &str,
    col: &Coleccion,
    con_cambios: bool,
) -> Result<Tabla, ore_driver::Fallo> {
    let muestra = c.muestra(
        base,
        &col.nombre,
        crate::lectura::MUESTRA,
        ESPERA_MUESTRA,
        None,
    )?;
    let esquema = inferir(&muestra.documentos)?;
    if esquema.documentos == 0 {
        eprintln!(
            "ore-read-mongodb: aviso · `{base}.{}` está vacía: sin documentos no hay esquema que inferir",
            col.nombre
        );
    }
    for a in &esquema.avisos {
        eprintln!(
            "ore-read-mongodb: aviso · `{base}.{}.{}`: {}",
            col.nombre, a.ruta, a.mensaje
        );
    }
    let columnas = esquema
        .columnas
        .iter()
        .map(|k| Columna {
            nombre: k.ruta.clone(),
            tipo: Some(k.tipo.oos()),
            origen: Some(k.tipo.fisico_origen().to_string()),
            // Obligatoria solo si todos los documentos vistos traían un valor.
            obligatoria: esquema.documentos > 0 && k.con_valor == esquema.documentos,
            valores: Vec::new(),
            descripcion: None,
        })
        .collect();
    Ok(Tabla {
        nombre: format!("{base}.{}", col.nombre),
        columnas,
        clave: vec!["_id".to_string()],
        filas_estimadas: Some(muestra.total),
        clase: match col.tipo {
            TipoColeccion::Vista => "view",
            _ => "table",
        }
        .to_string(),
        lee: Some(lee()),
        cambia: Some(cambios_de(col, con_cambios)),
        ..Tabla::default()
    })
}

/// La cara `D`, **sondeada**: qué cambios puede emitir esta colección de verdad.
///
/// | lo que dice el servidor | lo que emite | por qué |
/// |---|---|---|
/// | una vista, una serie temporal, o sin change streams | `{none, none}` | no hay flujo propio que leer |
/// | una colección en un replica set | `{upsert, key: [_id], log}` | un alta o una actualización trae el documento entero; un borrado, su `_id`, que es exactamente un tombstone por clave. La posición es el `operationTime`, ordinal y replayable |
fn cambios_de(col: &Coleccion, con_cambios: bool) -> Json {
    if con_cambios && col.tipo == TipoColeccion::Coleccion {
        Json::obj([
            ("mode", Json::s("upsert")),
            ("key", Json::Arr(vec![Json::s("_id")])),
            ("witness", Json::s("log")),
        ])
    } else {
        Json::obj([("mode", Json::s("none")), ("witness", Json::s("none"))])
    }
}

/// La cara `I`: lo que la inducción declara que se puede empujar.
fn lee() -> Json {
    Json::obj([
        (
            "predicatePushdown",
            Json::Arr(
                ore_driver::EMPUJE_INDUCIDO
                    .iter()
                    .map(|o| Json::s(*o))
                    .collect(),
            ),
        ),
        ("fullScan", Json::s("cheap")),
    ])
}

/// **`check`**: ¿responde esta fuente? Abre, hace `ping` y, si la URL nombra una
/// base, la lista: es lo que prueba, a la vez, la red, el DNS/SRV, el TLS, la
/// identidad y el permiso de lectura. El motivo del fallo va **literal**.
pub fn check(entrada: &str) -> Result<String, String> {
    let (url, _) = ore_driver::leer_coordenada(entrada)?;
    let hecho = Conexion::abrir(&url).and_then(|c| {
        c.ping()?;
        match base_de(&url) {
            Some(b) => c.colecciones(&b).map(|_| ()),
            None => c.bases().map(|_| ()),
        }
    });
    Ok(match hecho {
        Ok(()) => ore_driver::comprobacion(true, None),
        Err(f) => ore_driver::comprobacion(false, Some(&texto(f, &url))),
    })
}

/// **`explorar`**: las bases que tiene la fuente. Lo que `catalogo` recorre si
/// la URL no nombra una.
pub fn explorar(entrada: &str) -> Result<String, String> {
    let (url, _) = ore_driver::leer_coordenada(entrada)?;
    let c = Conexion::abrir(&url).map_err(|f| texto(f, &url))?;
    let contiene: Vec<Json> = bases_a_recorrer(&c, &url)?
        .iter()
        .map(|b| Json::obj([("nombre", Json::s(b))]))
        .collect();
    let nota = match base_de(&url) {
        Some(b) => format!("la URL nombra la base `{b}`: el catálogo recorre solo esa"),
        None => format!(
            "la URL no nombra una base: el catálogo recorre todas menos {}",
            DEL_SISTEMA.join(", ")
        ),
    };
    Ok(Json::obj([("contiene", Json::Arr(contiene)), ("nota", Json::s(nota))]).pretty())
}

/// **`testigo`**: hasta dónde está el servidor ahora. El `operationTime` de un
/// replica set es ordinal y de commit; un servidor suelto no lo tiene y
/// contesta `none`, que es una respuesta cierta.
pub fn testigo(entrada: &str) -> Result<String, String> {
    let (url, _) = ore_driver::leer_coordenada(entrada)?;
    let c = Conexion::abrir(&url).map_err(|f| texto(f, &url))?;
    Ok(match c.testigo().map_err(|f| texto(f, &url))? {
        Some(v) => ore_driver::testigo("log", Some(&v)),
        None => ore_driver::testigo("none", None),
    })
}

/// **`cambio`** (ADR 0063 S3): ¿cambió ESTA colección desde la última copia?
///
/// MongoDB no tiene contadores por colección que cuenten escrituras (`count` y
/// tamaño no se mueven con una actualización en el sitio, y `serverStatus` es
/// del servidor entero), así que **no hay huella que respalde**: se contesta
/// `respalda: false` y la copia no se fía, que es lo seguro. Si la colección no
/// está es un error, no «no respalda»: la copia que antes tenía señal no puede
/// tomar un origen que ya no tiene la colección por uno que no sabe contestar.
pub fn cambio(entrada: &str) -> Result<String, String> {
    let (url, objeto) = ore_driver::leer_coordenada(entrada)?;
    let (base, coleccion) = crate::lectura::partir_objeto(&objeto).map_err(|f| texto(f, &url))?;
    let c = Conexion::abrir(&url).map_err(|f| texto(f, &url))?;
    let existe = c
        .colecciones(base)
        .map_err(|f| texto(f, &url))?
        .iter()
        .any(|k| k.nombre == coleccion);
    if !existe {
        return Err(format!("`{objeto}` no está en el origen"));
    }
    Ok(ore_driver::huella("sin-contadores", None, false))
}
