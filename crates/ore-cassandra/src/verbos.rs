//! **Los verbos que son de Cassandra** y no del esqueleto: `catalogo`, `check`,
//! `explorar`, `testigo` y `cambio`. Los cuatro de lectura (`capacidades`, `leer`,
//! `estimar`, `servir`) están en [`crate::lectura`].
//!
//! Cada uno recibe lo que llega por stdin y devuelve la línea que sale por stdout, o
//! el motivo del fallo (que va a stderr, literal, sin la credencial).

use crate::esquema::Tabla as TablaCql;
use crate::transporte::{Conexion, Permisos};
use crate::url::analizar;
use ore_base::json::Json;
use ore_driver::catalogo::{Catalogo, Columna, Tabla, escribir};
use ore_driver::{Codigo, Fallo};
use scylla::value::CqlValue;
use std::process::ExitCode;

/// Lo que `catalogo` no recorre: el sistema de Cassandra (`system`, `system_schema`,
/// `system_auth`, `system_distributed`, `system_traces`, `system_views`,
/// `system_virtual_schema`) y lo que ponga cualquiera con ese prefijo.
fn del_sistema(keyspace: &str) -> bool {
    keyspace.starts_with("system")
}

/// El `main` de lo que no son los cuatro verbos v2: lee stdin y despacha.
pub fn otros(verbo: &str, args: &[String]) -> ExitCode {
    let mut entrada = String::new();
    if let Err(e) = std::io::Read::read_to_string(&mut std::io::stdin(), &mut entrada) {
        eprintln!("ore-read-cassandra: no se pudo leer stdin: {e}");
        return ExitCode::FAILURE;
    }
    let hecho = match verbo {
        "catalogo" => catalogo(
            entrada.trim(),
            args.first().map_or("cassandra", String::as_str),
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
            eprintln!("ore-read-cassandra: {m}");
            ExitCode::FAILURE
        }
    }
}

/// Un fallo del conector, como el texto que `ore` enseña: sin la credencial.
fn texto(f: Fallo, url: &str) -> String {
    f.tapado(url).mensaje
}

/// Un literal de texto de CQL: entre comillas simples, con las simples duplicadas.
fn literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn palabras(filas: Vec<Vec<Option<CqlValue>>>) -> Vec<String> {
    let mut v: Vec<String> = filas
        .into_iter()
        .filter_map(|f| match f.into_iter().next().flatten() {
            Some(CqlValue::Text(s) | CqlValue::Ascii(s)) => Some(s),
            _ => None,
        })
        .collect();
    v.sort();
    v
}

/// Los keyspaces que el rol ve y que no son del sistema; o el que nombra la URL.
fn keyspaces_a_recorrer(
    c: &Conexion,
    url: &str,
    permisos: &Permisos,
) -> Result<Vec<String>, String> {
    if let Some(k) = &c.destino.keyspace {
        return Ok(vec![k.clone()]);
    }
    let todos = palabras(
        c.consulta("SELECT keyspace_name FROM system_schema.keyspaces")
            .map_err(|f| texto(f, url))?,
    );
    Ok(todos
        .into_iter()
        .filter(|k| !del_sistema(k) && permisos.ve_keyspace(k))
        .collect())
}

/// Los permisos del rol. Si no se pudieron preguntar no se esconde nada (el servidor
/// sigue negando lo que no se puede leer), pero se dice.
fn permisos_o_todo(c: &Conexion, url: &str) -> Permisos {
    match c.permisos() {
        Ok(p) => p,
        Err(f) => {
            eprintln!(
                "ore-read-cassandra: aviso · no se pudieron leer los permisos del rol ({}): el catálogo lista todo lo que `system_schema` enseña",
                texto(f, url)
            );
            Permisos::todo()
        }
    }
}

/// **`catalogo`**: cada tabla de los keyspaces que el rol puede leer, con sus tipos
/// de `system_schema` (no se infiere nada) y su clave. Una tabla que no se deja
/// catalogar no tumba el catálogo: se salta con su aviso y su motivo; solo falla si
/// no se pudo con ninguna.
pub fn catalogo(url: &str, fuente: &str) -> Result<String, String> {
    if url.is_empty() {
        return Err(
            "no llegó nada por stdin. `catalogo` espera la URL de Cassandra por ahí".into(),
        );
    }
    let c = Conexion::abrir(url).map_err(|f| texto(f, url))?;
    let permisos = permisos_o_todo(&c, url);
    let mut tablas = Vec::new();
    let mut saltadas: Vec<String> = Vec::new();
    for ks in keyspaces_a_recorrer(&c, url, &permisos)? {
        for nombre in c.tablas(&ks).map_err(|f| texto(f, url))? {
            // `system_schema` enseña tablas que el rol no lee (X0 §3.5): no se
            // catalogan, ni se dice cuáles son (un nombre ajeno no es asunto suyo).
            if !permisos.puede_leer(&ks, &nombre) {
                continue;
            }
            match c.tabla(&ks, &nombre) {
                Ok(t) => {
                    for a in &t.avisos {
                        eprintln!("ore-read-cassandra: aviso · `{ks}.{nombre}`: {a}");
                    }
                    let filas = filas_estimadas(&c, &t);
                    tablas.push(tabla_de(&t, filas));
                }
                Err(f) => {
                    let m = texto(f, url);
                    eprintln!("ore-read-cassandra: aviso · `{ks}.{nombre}` no se cataloga: {m}");
                    saltadas.push(m);
                }
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

/// Las **particiones** estimadas de una tabla (`system.size_estimates`, que el
/// servidor mantiene y no cuesta leer). Son filas solo si la tabla no tiene
/// clustering: con él una partición son muchas filas, y no se inventa un número.
fn filas_estimadas(c: &Conexion, t: &TablaCql) -> Option<u64> {
    if !t.clustering().is_empty() {
        return None;
    }
    let cql = format!(
        "SELECT partitions_count FROM system.size_estimates WHERE keyspace_name = {} AND table_name = {}",
        literal(&t.keyspace),
        literal(&t.nombre)
    );
    let filas = c.consulta(&cql).ok()?;
    let mut total = 0i64;
    for f in &filas {
        if let Some(CqlValue::BigInt(n)) = f.first().cloned().flatten() {
            total += n.max(0);
        }
    }
    (!filas.is_empty() && total > 0).then_some(total as u64)
}

/// Una tabla de Cassandra, como la tabla del catálogo.
fn tabla_de(t: &TablaCql, filas_estimadas: Option<u64>) -> Tabla {
    let clave = t.clave_primaria();
    let columnas = t
        .columnas
        .iter()
        .map(|k| Columna {
            nombre: k.nombre.clone(),
            tipo: Some(k.tipo.oos()),
            // Se cita el tipo de CQL tal cual (`frozen<list<int>>`, `decimal`): con
            // él, el motor sabe lo que el escalar no lleva.
            origen: Some(k.cql.to_string()),
            // Solo la clave no es nula nunca.
            obligatoria: clave.contains(&k.nombre),
            valores: Vec::new(),
            descripcion: None,
        })
        .collect();
    Tabla {
        nombre: format!("{}.{}", t.keyspace, t.nombre),
        columnas,
        clave,
        filas_estimadas,
        clase: "table".to_string(),
        lee: Some(lee()),
        cambia: Some(cambios()),
        ..Tabla::default()
    }
}

/// La cara `I`: lo que la inducción declara que se puede empujar. Leer una tabla de
/// Cassandra entera es lo que cuesta, no un atajo: `expensive` hasta que alguien diga
/// otra cosa.
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
        ("fullScan", Json::s("expensive")),
    ])
}

/// La cara `D`. Cassandra no tiene change streams (CDC es por nodo y sobre el
/// commitlog), así que ninguna tabla declara cambios; el cursor sobre una columna
/// designada y un SAI es de M4.
fn cambios() -> Json {
    Json::obj([("mode", Json::s("none")), ("witness", Json::s("none"))])
}

/// **`check`**: ¿responde esta fuente? Paso a paso, y el motivo del fallo va
/// **literal** (es lo único accionable): abre (red, DNS, TLS, credencial), pregunta la
/// versión y el partidor y, si la URL nombra un keyspace, lista sus tablas y lee **una
/// fila de una** (el permiso de lectura).
pub fn check(entrada: &str) -> Result<String, String> {
    let (url, _) = ore_driver::leer_coordenada(entrada)?;
    let hecho = (|| -> Result<(), Fallo> {
        let c = Conexion::abrir(&url)?;
        let (_, partidor) = c.servidor()?;
        if !partidor.ends_with("Murmur3Partitioner") {
            return Err(Fallo::origen(format!(
                "el clúster usa `{partidor}` y este conector recorre por rangos de `Murmur3Partitioner`"
            )));
        }
        c.rangos()?;
        if let Some(k) = &c.destino.keyspace {
            let tablas = c.tablas(k)?;
            if tablas.is_empty() {
                return Err(Fallo::new(
                    Codigo::Objeto,
                    format!("el keyspace `{k}` no existe o no tiene tablas que el rol vea"),
                ));
            }
            let permisos = c.permisos().unwrap_or_else(|_| Permisos::todo());
            if let Some(t) = tablas.iter().find(|t| permisos.puede_leer(k, t)) {
                let tabla = c.tabla(k, t)?;
                let col = tabla.particion()[0].nombre.clone();
                c.consulta(&format!(
                    "SELECT {} FROM {} LIMIT 1",
                    crate::esquema::citar(&col),
                    tabla.objeto_citado()
                ))?;
            } else {
                return Err(Fallo::new(
                    Codigo::Credencial,
                    format!("el rol no tiene `SELECT` sobre ninguna tabla de `{k}`"),
                ));
            }
        }
        Ok(())
    })();
    Ok(match hecho {
        Ok(()) => ore_driver::comprobacion(true, None),
        Err(f) => ore_driver::comprobacion(false, Some(&texto(f, &url))),
    })
}

/// **`explorar`**: los keyspaces que el rol ve. Lo que `catalogo` recorre si la URL no
/// nombra uno.
pub fn explorar(entrada: &str) -> Result<String, String> {
    let (url, _) = ore_driver::leer_coordenada(entrada)?;
    let c = Conexion::abrir(&url).map_err(|f| texto(f, &url))?;
    let permisos = permisos_o_todo(&c, &url);
    let contiene: Vec<Json> = keyspaces_a_recorrer(&c, &url, &permisos)?
        .iter()
        .map(|k| Json::obj([("nombre", Json::s(k))]))
        .collect();
    let nota = match &c.destino.keyspace {
        Some(k) => format!("la URL nombra el keyspace `{k}`: el catálogo recorre solo ese"),
        None => "la URL no nombra un keyspace: el catálogo recorre los que el rol puede leer, menos los del sistema".to_string(),
    };
    Ok(Json::obj([("contiene", Json::Arr(contiene)), ("nota", Json::s(nota))]).pretty())
}

/// **`testigo`**: Cassandra no sabe fecharse. No tiene una posición ordinal de
/// commit que se pueda comparar (el CDC es del commitlog de cada nodo), y devolver
/// «ahora» inventaría una marca que el origen no respalda: contesta `none`, que es
/// una respuesta cierta (ADR 0016).
pub fn testigo(entrada: &str) -> Result<String, String> {
    let (url, _) = ore_driver::leer_coordenada(entrada)?;
    // Se abre para decir que el origen responde antes de decir que no sabe fecharse.
    Conexion::abrir(&url).map_err(|f| texto(f, &url))?;
    Ok(ore_driver::testigo("none", None))
}

/// **`cambio`** (ADR 0063 S3): ¿cambió ESTA tabla desde la última copia?
///
/// Cassandra no tiene contadores por tabla que cuenten escrituras, así que **no hay
/// huella que respalde**: `respalda: false`, y la copia no se fía. Si la tabla no está
/// es un error, no «no respalda».
pub fn cambio(entrada: &str) -> Result<String, String> {
    let (url, objeto) = ore_driver::leer_coordenada(entrada)?;
    let (ks, nombre) = crate::lectura::partir_objeto(&objeto).map_err(|f| texto(f, &url))?;
    let c = Conexion::abrir(&url).map_err(|f| texto(f, &url))?;
    c.tabla(ks, nombre).map_err(|f| texto(f, &url))?;
    Ok(ore_driver::huella("sin-contadores", None, false))
}

/// La URL, para quien necesite saber si nombra un keyspace sin abrir nada.
pub fn keyspace_de(url: &str) -> Option<String> {
    analizar(url).ok().and_then(|d| d.keyspace)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::esquema::pruebas::{pedidos, sai};

    #[test]
    fn los_keyspaces_del_sistema_no_se_recorren() {
        for k in [
            "system",
            "system_schema",
            "system_auth",
            "system_distributed",
            "system_traces",
            "system_views",
            "system_virtual_schema",
        ] {
            assert!(del_sistema(k), "{k}");
        }
        assert!(!del_sistema("ore_prueba") && !del_sistema("mi_system"));
    }

    #[test]
    fn un_literal_no_se_sale_de_sus_comillas() {
        assert_eq!(literal("a'b"), "'a''b'");
        assert_eq!(literal("x'; DROP TABLE t; --"), "'x''; DROP TABLE t; --'");
    }

    #[test]
    fn la_tabla_del_catalogo() {
        let t = pedidos(&[sai("i", "estado")]);
        let c = tabla_de(&t, None);
        assert_eq!(c.nombre, "ore_prueba.pedidos");
        assert_eq!(c.clave, ["cliente", "dia", "hora", "id"]);
        let col = |n: &str| c.columnas.iter().find(|k| k.nombre == n).unwrap();
        assert_eq!(col("cliente").tipo.as_deref(), Some("Integer"));
        assert!(col("cliente").obligatoria && !col("total").obligatoria);
        assert_eq!(col("total").tipo.as_deref(), Some("String"));
        assert_eq!(col("total").origen.as_deref(), Some("decimal"));
        assert_eq!(col("hora").tipo.as_deref(), Some("DateTimeTz"));
        assert_eq!(c.clase, "table");
        let lee = c.lee.unwrap().jcs();
        assert!(lee.contains("\"fullScan\":\"expensive\""), "{lee}");
        assert!(lee.contains("\"predicatePushdown\""), "{lee}");
        assert_eq!(
            c.cambia.unwrap().jcs(),
            r#"{"mode":"none","witness":"none"}"#
        );
    }

    #[test]
    fn el_catalogo_se_escribe_y_se_vuelve_a_leer() {
        let t = pedidos(&[]);
        let json = escribir(&Catalogo {
            fuente: "cassandra".into(),
            tablas: vec![tabla_de(&t, Some(42))],
            objetos: Vec::new(),
        });
        let leido = Catalogo::leer(&json).expect("el lector lo entiende");
        assert_eq!(leido.fuente(), "cassandra");
        assert_eq!(leido.tablas.len(), 1);
        assert_eq!(leido.tablas[0].filas_estimadas, Some(42));
        assert_eq!(leido.tablas[0].clave.len(), 4);
    }

    #[test]
    fn el_keyspace_sale_de_la_url() {
        assert_eq!(
            keyspace_de("cassandra://u:p@h/ventas").as_deref(),
            Some("ventas")
        );
        assert_eq!(keyspace_de("cassandra://h"), None);
        assert_eq!(keyspace_de("mongodb://h/x"), None);
    }
}
