//! **El índice de una capa** (ADR 0072 I2·3): el Job de la capa, con `"tarea": "indice"`, construye y
//! publica el índice de una tabla del lago en su snapshot. Seis piezas, en este orden:
//!
//! 1. **leer la fuente**: sus ficheros vivos (de datos y de borrados por posición), cada uno con su
//!    ordinal, que es la mitad de la identidad;
//! 2. **contar la fuente** y su extensión, en DuckDB, en una pasada;
//! 3. y 4. **transformar, ordenar y escribir** el índice, en DuckDB (`ore_teselas::indice_sql`);
//! 5. **registrar** los Parquet en la tabla Iceberg del índice (`Lago::registrar`);
//! 6. **contar el índice, comprobar y publicar** (`ore_teselas::indice`): sin una `Comprobacion` que
//!    cuadre, nada se publica.
//!
//! Todo queda bajo `<tabla>/_ore_capa/i1/<snapshot>/`: el nombre sale del snapshot, así que repetir
//! el mismo snapshot no rehace nada que ya esté publicado.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Instant;

use ore_base::json::Json;
use ore_store::almacen::Almacen;
use ore_store::lago::{Cambio, Lago};
use ore_teselas::indice::{
    self, Comprobacion, Cuenta, Fuente, I1, Manifiesto, PiramideDeLaCapa, Publicado, Rejilla,
    TipoDeIdentidad,
};
use ore_teselas::indice_sql::{self, FicheroFuente, Lectura, Tramo};

use crate::{Lugar, partir, subir_como};

/// **La petición del índice**: `{"tarea": "indice", "fuente": {"metadata_location", "geometria",
/// "crs", "columnas": […], "credencial"?}, "capa", "suelo", "destino": {"ubicacion": <la de la
/// tabla>, "credencial"?}, "piramide"?: {"clave", "snapshot_fuente"}, "trabajo"}`.
#[derive(Debug, Clone)]
pub struct PeticionIndice {
    pub metadata_location: String,
    pub credencial: Option<String>,
    pub geometria: String,
    pub crs: String,
    pub columnas: Vec<String>,
    /// El nombre de la capa, el que lleva su fuente en el manifiesto.
    pub capa: String,
    pub suelo: u32,
    /// La ubicación de la tabla: el índice vive bajo ella ([`indice::prefijo`]).
    pub destino: Lugar,
    pub piramide: Option<PiramideDeLaCapa>,
    pub trabajo: PathBuf,
    /// El binario de DuckDB y dónde están sus extensiones (`spatial`, `httpfs`) sin red.
    pub duckdb: String,
    pub extensiones: Option<String>,
    /// Lo que se construye de una vez (`limites.filas_por_tramo`): lo que cabe en el disco del Job.
    pub filas_por_tramo: u64,
    /// Cuántos tramos ordenados de un nivel se funden en uno del siguiente
    /// (`limites.compactar_cada`, I4·3).
    pub compactar_cada: usize,
}

/// Por defecto, se funden de 4 en 4 (I4·3): la profundidad, acotada en ~4 por nivel en vez de +1,5
/// por carga (I4·0).
pub const COMPACTAR_CADA: usize = 4;

/// Por defecto, 100 M filas por tramo: ~5 GB de puntos en el índice, con el disco de la ordenación al
/// lado, holgado en los 40 GiB del Job (I2·4).
pub const FILAS_POR_TRAMO: u64 = 100_000_000;

/// ¿Es una petición de índice? (sin `tarea`, es de pirámide).
pub fn es_de_indice(texto: &str) -> bool {
    ore_base::parse::parse(texto)
        .ok()
        .and_then(|n| {
            n.get("tarea")
                .and_then(|(_, v)| v.as_str())
                .map(|t| t == "indice")
        })
        .unwrap_or(false)
}

pub fn peticion_indice(texto: &str) -> Result<PeticionIndice, String> {
    let n = ore_base::parse::parse(texto).map_err(|e| format!("la petición no es JSON: {e:?}"))?;
    let en = |o: &str, k: &str| {
        n.get(o)
            .and_then(|(_, v)| v.get(k))
            .and_then(|(_, v)| v.as_str())
            .map(str::to_string)
    };
    let token = |o: &str| {
        n.get(o)
            .and_then(|(_, f)| f.get("credencial"))
            .and_then(|(_, c)| c.get("gcs.oauth2.token"))
            .and_then(|(_, v)| v.as_str())
            .map(str::to_string)
    };
    let s = |k: &str| n.get(k).and_then(|(_, v)| v.as_str()).map(str::to_string);
    let falta = |k: &str| format!("falta `{k}`");
    let suelo: u32 = s("suelo")
        .ok_or_else(|| falta("suelo"))?
        .parse()
        .map_err(|_| "`suelo` no es un entero".to_string())?;
    if !(1..=20).contains(&suelo) {
        return Err(format!("`suelo` {suelo}: entre 1 y 20"));
    }
    let columnas = n
        .get("fuente")
        .and_then(|(_, f)| f.get("columnas"))
        .map(|(_, v)| {
            v.items()
                .iter()
                .filter_map(|c| c.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let piramide = match n.get("piramide").map(|(_, v)| v) {
        None => None,
        Some(p) => {
            let c = |k: &str| {
                p.get(k)
                    .and_then(|(_, v)| v.as_str())
                    .map(str::to_string)
                    .ok_or_else(|| falta(&format!("piramide.{k}")))
            };
            Some(PiramideDeLaCapa {
                clave: c("clave")?,
                snapshot_fuente: c("snapshot_fuente")?,
            })
        }
    };
    Ok(PeticionIndice {
        metadata_location: en("fuente", "metadata_location")
            .ok_or_else(|| falta("fuente.metadata_location"))?,
        credencial: token("fuente"),
        geometria: en("fuente", "geometria").unwrap_or_else(|| "geom".into()),
        crs: en("fuente", "crs").unwrap_or_else(|| "EPSG:4326".into()),
        columnas,
        capa: s("capa").ok_or_else(|| falta("capa"))?,
        suelo,
        destino: Lugar {
            ubicacion: en("destino", "ubicacion").ok_or_else(|| falta("destino.ubicacion"))?,
            token: token("destino"),
        },
        piramide,
        trabajo: PathBuf::from(s("trabajo").unwrap_or_else(|| "/tmp".into())),
        filas_por_tramo: n
            .get("limites")
            .and_then(|(_, l)| l.get("filas_por_tramo"))
            .and_then(|(_, v)| v.as_str())
            .map_or(Ok(FILAS_POR_TRAMO), |v| {
                v.parse()
                    .map_err(|_| "`limites.filas_por_tramo` no es un entero".to_string())
            })?,
        compactar_cada: n
            .get("limites")
            .and_then(|(_, l)| l.get("compactar_cada"))
            .and_then(|(_, v)| v.as_str())
            .map_or(Ok(COMPACTAR_CADA), |v| {
                v.parse()
                    .ok()
                    .filter(|k: &usize| *k >= 2)
                    .ok_or_else(|| "`limites.compactar_cada` es un entero de 2 o más".to_string())
            })?,
        duckdb: std::env::var("ORE_DUCKDB").unwrap_or_else(|_| "duckdb".into()),
        extensiones: std::env::var("ORE_DUCKDB_EXTENSIONES").ok(),
    })
}

fn cuenta_gcs(cubo: &str, token: Option<&str>) -> Result<ore_store::gcs::Cuenta, String> {
    match token {
        Some(t) => ore_store::gcs::Cuenta::con_token(cubo, t),
        None => ore_store::gcs::Cuenta::de_la_identidad(cubo),
    }
}

/// `gs://cubo/clave` → `https://storage.googleapis.com/cubo/clave`: por donde lee DuckDB.
fn por_https(uri: &str) -> Result<String, String> {
    let (cubo, clave) = partir(uri)?;
    Ok(format!("https://storage.googleapis.com/{cubo}/{clave}"))
}

/// **Una sesión de DuckDB**: lo de antes (extensiones, credencial, disco temporal) y `sql`, por la
/// entrada estándar (la credencial no toca el disco). `-bail`: el primer error para la sesión.
fn duckdb(p: &PeticionIndice, secretos: &[(String, String)], sql: &str) -> Result<f64, String> {
    let temp = p.trabajo.join("temp");
    std::fs::create_dir_all(&temp).map_err(|e| e.to_string())?;
    let mut guion = String::new();
    if let Some(e) = &p.extensiones {
        guion += &format!("SET extension_directory = '{}';\n", e.replace('\'', "''"));
    }
    guion += &format!(
        "LOAD spatial;\nLOAD httpfs;\nSET temp_directory = '{}';\n",
        temp.display().to_string().replace('\'', "''"),
    );
    // un secreto por cubo: la fuente y el destino (donde se cuenta lo subido) pueden ser dos
    for (i, (cubo, token)) in secretos.iter().enumerate() {
        guion += &format!(
            "CREATE OR REPLACE SECRET ore_{i} (TYPE http, BEARER_TOKEN '{}', SCOPE 'https://storage.googleapis.com/{cubo}/');\n",
            token.replace('\'', "''"),
        );
    }
    guion += sql;
    let t = Instant::now();
    let mut hijo = Command::new(&p.duckdb)
        .args(["-bail", ":memory:"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("DuckDB (`{}`) no arranca: {e}", p.duckdb))?;
    hijo.stdin
        .take()
        .expect("entrada")
        .write_all(guion.as_bytes())
        .map_err(|e| e.to_string())?;
    let salida = hijo.wait_with_output().map_err(|e| e.to_string())?;
    if !salida.status.success() {
        let error = String::from_utf8_lossy(&salida.stderr);
        return Err(format!(
            "DuckDB falló: {}",
            error.lines().take(8).collect::<Vec<_>>().join(" · ")
        ));
    }
    Ok(t.elapsed().as_secs_f64())
}

/// La primera fila de un JSON por líneas (lo que escribe `COPY … (FORMAT json)`).
fn fila_json(ruta: &Path) -> Result<ore_base::parse::Node, String> {
    let t = std::fs::read_to_string(ruta).map_err(|e| format!("`{}`: {e}", ruta.display()))?;
    let linea = t
        .lines()
        .next()
        .ok_or_else(|| format!("`{}` vacío", ruta.display()))?;
    ore_base::parse::parse(linea).map_err(|e| format!("`{}`: {e:?}", ruta.display()))
}

fn numero<T: std::str::FromStr>(n: &ore_base::parse::Node, k: &str) -> Result<T, String> {
    n.get(k)
        .and_then(|(_, v)| v.as_str())
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| format!("DuckDB no dio `{k}`"))
}

/// Envuelve una consulta (sin su `;`) para que DuckDB deje su fila en `ruta`.
fn a_json(consulta: &str, ruta: &Path) -> String {
    format!(
        "COPY ({}) TO '{}' (FORMAT json);\n",
        consulta.trim().trim_end_matches(';'),
        ruta.display().to_string().replace('\'', "''")
    )
}

/// Lo que el Job abre una vez y usan construir y seguir.
struct Contexto<'a> {
    p: &'a PeticionIndice,
    lago: Lago,
    tabla: ore_store::lago::Tabla,
    secuencia_fuente: u64,
    secretos: Vec<(String, String)>,
    destino: Arc<dyn Almacen>,
    cubo_d: String,
    prefijo: String,
    /// El nombre de la instantánea (su `metadata.json` sin la extensión).
    instantanea: String,
    t0: Instant,
}

/// **Cómo llega la geometría a DuckDB** (WKB, o ya `GEOMETRY`): DuckDB lo decide al enlazar, así
/// que se pregunta antes y la lectura se genera para lo que diga.
fn probar_geometria(c: &Contexto, lectura: &mut Lectura) -> Result<String, String> {
    std::fs::create_dir_all(&c.p.trabajo).map_err(|e| e.to_string())?;
    let tipo_json = c.p.trabajo.join("tipo.json");
    let _ = std::fs::remove_file(&tipo_json);
    duckdb(
        c.p,
        &c.secretos,
        &a_json(&indice_sql::sql_tipo_de_la_geometria(lectura)?, &tipo_json),
    )?;
    let tipo = fila_json(&tipo_json)?
        .get("tipo")
        .and_then(|(_, v)| v.as_str())
        .map(str::to_string)
        .unwrap_or_default();
    lectura.en_wkb = match tipo.as_str() {
        "BLOB" => true,
        "GEOMETRY" => false,
        o => {
            return Err(format!(
                "`{}` llega a DuckDB como `{o}`: ni WKB ni geometría",
                c.p.geometria
            ));
        }
    };
    Ok(tipo)
}

/// La cuenta (filas y suma de ids) y la extensión de lo que lee `sql_lectura`.
fn contar(c: &Contexto, sql_lectura: &str) -> Result<(Cuenta, [f64; 4], f64), String> {
    let ext_json = c.p.trabajo.join("extension.json");
    let _ = std::fs::remove_file(&ext_json);
    let s = duckdb(
        c.p,
        &c.secretos,
        &(sql_lectura.to_string() + &a_json(indice_sql::SQL_EXTENSION_Y_CUENTA, &ext_json)),
    )?;
    let e = fila_json(&ext_json)?;
    let cuenta = Cuenta {
        filas: numero(&e, "filas")?,
        suma_ids: numero(&e, "suma_ids")?,
    };
    let ext = [
        numero::<f64>(&e, "xmin").unwrap_or(f64::NAN),
        numero(&e, "ymin").unwrap_or(f64::NAN),
        numero(&e, "xmax").unwrap_or(f64::NAN),
        numero(&e, "ymax").unwrap_or(f64::NAN),
    ];
    Ok((cuenta, ext, s))
}

/// Lo que se escribió: `(URIs, por donde se leen, bytes, segundos de DuckDB, tramos)`.
type Escritos = (Vec<String>, Vec<String>, u64, f64, usize);

/// **3 y 4 · El índice, por tramos de la clave**, a `<dir>data/<nombre>-NNNN.parquet`: tras
/// `sql_previo` (la lectura de la fuente, o lo que se funde), `cortes(n)` dice dónde cortar y
/// `tramo(t, fichero)` escribe un tramo. Lo que cabe en el disco del Job es un tramo, no la tabla (I2·4:
/// 600 M filas desbordaron los 40 GiB de una vez): cada tramo es un `COPY` con su filtro, en su
/// fichero, que se sube y se borra antes del siguiente. Si algo falla a medias, lo subido se borra:
/// no lo nombra ningún manifiesto (y el directorio puede ser el del índice que se sigue).
#[allow(clippy::too_many_arguments)]
fn escribir_tramos(
    c: &Contexto,
    sql_previo: &str,
    cortes: &dyn Fn(usize) -> String,
    tramo_sql: &dyn Fn(&Tramo, &str) -> Result<String, String>,
    filas: u64,
    dir: &str,
    nombre: &str,
    di: &mut dyn FnMut(&str, Json),
) -> Result<Escritos, String> {
    let p = c.p;
    let n_tramos = filas.div_ceil(p.filas_por_tramo.max(1)) as usize;
    let tramos = if n_tramos <= 1 {
        vec![Tramo::default()]
    } else {
        let cortes_json = p.trabajo.join("cortes.json");
        let _ = std::fs::remove_file(&cortes_json);
        duckdb(
            p,
            &c.secretos,
            &(sql_previo.to_string() + &a_json(&cortes(n_tramos), &cortes_json)),
        )?;
        let cortes: Vec<u64> = fila_json(&cortes_json)?
            .get("cortes")
            .map(|(_, v)| {
                v.items()
                    .iter()
                    .filter_map(|c| c.as_str()?.parse().ok())
                    .collect()
            })
            .unwrap_or_default();
        Tramo::de_cortes(&cortes)
    };
    let salida = p.trabajo.join("indice");
    let (mut uris, mut https, mut bytes, mut segundos) = (Vec::new(), Vec::new(), 0u64, 0.0);
    let mut subidas: Vec<String> = Vec::new();
    for (i, tramo) in tramos.iter().enumerate() {
        let hecho = (|| -> Result<(), String> {
            let _ = std::fs::remove_dir_all(&salida);
            std::fs::create_dir_all(&salida).map_err(|e| e.to_string())?;
            let local = salida.join(format!("{nombre}-{i:04}.parquet"));
            let s = duckdb(
                p,
                &c.secretos,
                &(sql_previo.to_string() + &tramo_sql(tramo, &local.display().to_string())?),
            )?;
            segundos += s;
            let b = std::fs::metadata(&local).map_err(|e| e.to_string())?.len();
            bytes += b;
            let clave = format!("{dir}data/{nombre}-{i:04}.parquet");
            subir_como(&c.destino, &clave, &local, "application/vnd.apache.parquet")?;
            subidas.push(clave.clone());
            let _ = std::fs::remove_file(&local);
            uris.push(format!("gs://{}/{clave}", c.cubo_d));
            https.push(format!(
                "https://storage.googleapis.com/{}/{clave}",
                c.cubo_d
            ));
            di(
                "tramo",
                Json::obj([
                    ("tramo", Json::Int(i as i64)),
                    ("de", Json::Int(tramos.len() as i64)),
                    ("bytes", Json::s(b.to_string())),
                    ("segundos", Json::s(format!("{s:.1}"))),
                ]),
            );
            Ok(())
        })();
        if let Err(e) = hecho {
            borrar(&c.destino, &subidas);
            return Err(e);
        }
    }
    let _ = std::fs::remove_dir_all(&salida);
    Ok((uris, https, bytes, segundos, tramos.len()))
}

/// Borra lo subido que no llegó a publicarse.
fn borrar(destino: &Arc<dyn Almacen>, claves: &[String]) {
    for k in claves {
        let _ = destino.borrar(k);
    }
}

/// La cuenta y la profundidad de unos ficheros del índice (por donde los lee DuckDB).
fn contar_el_indice(
    c: &Contexto,
    https: &[String],
    profundidad_de: &[String],
) -> Result<(Cuenta, f64, u64), String> {
    let cuenta_json = c.p.trabajo.join("cuenta.json");
    let _ = std::fs::remove_file(&cuenta_json);
    let profundidad_json = c.p.trabajo.join("profundidad.json");
    let _ = std::fs::remove_file(&profundidad_json);
    let mut sql = String::new();
    if !https.is_empty() {
        sql += &a_json(&indice_sql::sql_cuenta_indice(https), &cuenta_json);
    }
    sql += &a_json(
        &indice_sql::sql_profundidad(profundidad_de),
        &profundidad_json,
    );
    duckdb(c.p, &c.secretos, &sql)?;
    let cuenta = if https.is_empty() {
        Cuenta::default()
    } else {
        let n = fila_json(&cuenta_json)?;
        Cuenta {
            filas: numero(&n, "filas")?,
            suma_ids: numero(&n, "suma_ids")?,
        }
    };
    let pr = fila_json(&profundidad_json)?;
    Ok((cuenta, numero(&pr, "profundidad")?, numero(&pr, "grupos")?))
}

/// La tabla de ficheros de un índice, `[[ordinal, uri], …]`.
fn tabla_de_ficheros(ficheros: &[(u32, String)]) -> String {
    Json::obj([(
        "ficheros",
        Json::Arr(
            ficheros
                .iter()
                .map(|(o, u)| Json::Arr(vec![Json::Int(*o as i64), Json::s(u.clone())]))
                .collect(),
        ),
    )])
    .jcs()
}

fn leer_tabla_de_ficheros(
    destino: &dyn Almacen,
    clave: &str,
) -> Result<Vec<(u32, String)>, String> {
    let t = destino
        .leer(clave)?
        .ok_or_else(|| format!("no está la tabla de ficheros `{clave}`"))?;
    let n = ore_base::parse::parse(&t).map_err(|e| format!("`{clave}`: {e:?}"))?;
    n.get("ficheros")
        .map(|(_, v)| v.items())
        .unwrap_or_default()
        .iter()
        .map(|par| {
            let p = par.items();
            match (
                p.first().and_then(|o| o.as_str()?.parse().ok()),
                p.get(1).and_then(|u| u.as_str()),
            ) {
                (Some(o), Some(u)) => Ok((o, u.to_string())),
                _ => Err(format!("`{clave}`: una entrada no es `[ordinal, uri]`")),
            }
        })
        .collect()
}

/// La suma con desbordamiento de los ids de las filas `0..n` del fichero de ordinal `o`
/// (`o << 40 | fila`): `n·(o << 40) + n(n−1)/2`, para cotejar lo que sale sin leerlo.
fn suma_de_un_fichero(o: u32, n: u64) -> u64 {
    let base = (o as u64) << 40;
    let triangular = ((n as u128) * (n.saturating_sub(1) as u128) / 2) as u64;
    n.wrapping_mul(base).wrapping_add(triangular)
}

/// **Construir y publicar el índice** de la fuente en su snapshot vigente: si ya hay uno de una
/// instantánea anterior, **siguiéndolo** (I4·2: sólo lo que cambió); si no se puede, entero.
pub fn construir_indice(
    p: &PeticionIndice,
    di: &mut dyn FnMut(&str, Json),
) -> Result<Publicado, String> {
    let t0 = Instant::now();
    // ── 1 · la fuente ───────────────────────────────────────────────────────────────────────────
    let (cubo, _) = partir(&p.metadata_location)?;
    let fuente = cuenta_gcs(cubo, p.credencial.as_deref())?;
    let token = fuente.token()?;
    let lago = Lago::nuevo(Arc::new(fuente));
    let tabla = lago.abrir(&p.metadata_location, &p.capa)?;
    let secuencia_fuente = tabla
        .metadata()
        .current_snapshot()
        .map_or(0, |s| s.sequence_number().max(0) as u64);
    let (cubo_d, _) = partir(&p.destino.ubicacion)?;
    let cuenta_d = cuenta_gcs(cubo_d, p.destino.token.as_deref())?;
    let secretos = vec![
        (cubo.to_string(), token.clone()),
        (cubo_d.to_string(), cuenta_d.token()?),
    ];
    let destino: Arc<dyn Almacen> = Arc::new(cuenta_d);
    let prefijo = indice::prefijo_en_el_almacen(&p.destino.ubicacion, &I1)?;
    let instantanea = indice::instantanea_de(&p.metadata_location)?.to_string();
    let c = Contexto {
        p,
        lago,
        tabla,
        secuencia_fuente,
        secretos,
        destino,
        cubo_d: cubo_d.to_string(),
        prefijo,
        instantanea,
        t0,
    };
    if let Some(v) = indice::vigente(c.destino.as_ref(), &c.prefijo)? {
        if v.secuencia_fuente >= c.secuencia_fuente {
            // el de esta instantánea, con su copia por nombre (la deja si falta: un índice de antes
            // de I3·4 la gana al relanzarse)
            if v.snapshot_fuente == p.metadata_location {
                indice::dejar_el_de_la_instantanea(c.destino.as_ref(), &c.prefijo, &v)?;
            }
            di(
                "ya",
                Json::obj([("generacion", Json::Int(v.generacion as i64))]),
            );
            return Ok(Publicado::Superado(v));
        }
        if let Some(r) = seguir(&c, &v, di)? {
            return Ok(r);
        }
    }
    construir_entero(&c, di)
}

/// **Seguir a su tabla** (I4·2): el índice `v` es de una instantánea anterior S; pasa a la de
/// ahora T con lo que cambió entre las dos ([`Lago::diferencia`]): lo nuevo, indexado aparte con
/// la misma rejilla y ordinales nuevos (nunca se reusan); lo que sale, a borrados por posición del
/// índice (`indice_sql::sql_quitar`), cotejado con la fuente (las filas de cada fichero retirado y
/// lo que nombran sus borrados); un commit sobre la misma tabla del índice; el manifiesto de T.
/// `Ok(None)`: no se puede (y dice por qué); se reconstruye.
fn seguir(
    c: &Contexto,
    v: &Manifiesto,
    di: &mut dyn FnMut(&str, Json),
) -> Result<Option<Publicado>, String> {
    let p = c.p;
    let mut no = |motivo: String| {
        di("reconstruir", Json::obj([("motivo", Json::s(motivo))]));
        Ok(None)
    };
    if v.identidad != TipoDeIdentidad::Posicion {
        return no("el índice no identifica por posición".into());
    }
    let Some(clave_ficheros) = &v.ficheros else {
        return no("el índice no tiene tabla de ficheros".into());
    };
    if v.suelo != p.suelo {
        return no(format!("el suelo era {} y ahora es {}", v.suelo, p.suelo));
    }
    let s = match c.lago.abrir(&v.snapshot_fuente, &p.capa) {
        Ok(s) => s,
        Err(e) => return no(format!("la instantánea del índice ya no se abre: {e}")),
    };
    let d = match c.lago.diferencia(&s, &c.tabla)? {
        Cambio::Diferencia(d) => d,
        Cambio::Reconstruir(m) => return no(m),
    };
    if !d.mismo_esquema {
        return no("el esquema de la fuente cambió".into());
    }
    let (entran, salen) = d.filas();
    if salen.saturating_mul(2) > v.filas {
        return no(format!(
            "salen {salen} de {} filas: es más barato reconstruir",
            v.filas
        ));
    }
    // la tabla de ficheros: lo retirado sale, lo nuevo entra con ordinales que no se reusan
    let viejos = leer_tabla_de_ficheros(c.destino.as_ref(), clave_ficheros)?;
    let por_uri: std::collections::HashMap<&str, u32> =
        viejos.iter().map(|(o, u)| (u.as_str(), *o)).collect();
    let mut retirados = Vec::with_capacity(d.retirados.len());
    for f in &d.retirados {
        match por_uri.get(f.ruta.as_str()) {
            Some(o) => retirados.push((*o, f.filas)),
            None => {
                return no(format!(
                    "`{}` sale y no está en la tabla de ficheros",
                    f.ruta
                ));
            }
        }
    }
    let mut siguiente = viejos.iter().map(|(o, _)| o + 1).max().unwrap_or(0);
    let nuevos: Vec<FicheroFuente> = d
        .anadidos
        .iter()
        .map(|f| {
            let o = siguiente;
            siguiente += 1;
            Ok(FicheroFuente {
                ordinal: o,
                ruta: por_https(&f.ruta)?,
                uri: f.ruta.clone(),
            })
        })
        .collect::<Result<_, String>>()?;
    // la tabla del índice, y sus columnas: las de esta petición, con los mismos ids
    let lago_d = Lago::nuevo(c.destino.clone());
    let idx = lago_d.abrir(&v.indice, &format!("{}-{}", p.capa, I1.nombre))?;
    let arrow = indice::esquema(&I1, &Lago::campos_arrow(&c.tabla, &p.columnas)?)?;
    let ids = Lago::ids_de_tabla_nueva(&arrow)?;
    let esquema_idx = idx.metadata().current_schema();
    if ids
        .iter()
        .any(|(n, id)| esquema_idx.field_by_name(n).map(|f| f.id) != Some(*id))
    {
        return no("las columnas del índice no son las de la petición".into());
    }
    let dir_idx = format!(
        "{}/",
        indice::clave_de(
            v.indice
                .rsplit_once("/metadata/")
                .map(|(t, _)| t)
                .ok_or_else(|| format!("`{}` no es un metadata.json", v.indice))?
        )?
    );
    let (vivos_idx, borrados_idx) = lago_d.vivos(&idx)?;
    let borrados_fuente: Vec<String> = d
        .borrados_anadidos
        .iter()
        .map(|b| por_https(b))
        .collect::<Result<_, _>>()?;
    di(
        "sigue",
        Json::obj([
            ("desde", Json::s(v.snapshot_fuente.clone())),
            ("instantaneas", Json::Int(d.instantaneas as i64)),
            ("ficheros_nuevos", Json::Int(nuevos.len() as i64)),
            ("ficheros_retirados", Json::Int(retirados.len() as i64)),
            ("borrados_nuevos", Json::Int(borrados_fuente.len() as i64)),
            ("filas_que_entran", Json::s(entran.to_string())),
            ("filas_que_salen", Json::s(salen.to_string())),
        ]),
    );
    let mut subidas: Vec<String> = Vec::new();
    let r = (|| -> Result<Publicado, String> {
        // ── lo nuevo ────────────────────────────────────────────────────────────────────────────
        let (mut nuevo_fuente, mut nuevo_idx, mut uris) =
            (Cuenta::default(), Cuenta::default(), Vec::new());
        if !nuevos.is_empty() {
            let mut lectura = Lectura {
                ficheros: nuevos.clone(),
                // los borrados nuevos de la fuente: los de sus ficheros nuevos se aplican al leer
                borrados: borrados_fuente.clone(),
                geometria: p.geometria.clone(),
                en_wkb: true,
                crs: p.crs.clone(),
                columnas: p.columnas.clone(),
                suelo: v.suelo,
            };
            probar_geometria(c, &mut lectura)?;
            let sql_lectura = indice_sql::sql_lectura(&lectura)?;
            nuevo_fuente = contar(c, &sql_lectura)?.0;
            let (u, https, _, segundos, tramos) = escribir_tramos(
                c,
                &sql_lectura,
                &|n| indice_sql::sql_cortes(&v.rejilla, n),
                &|t, f| indice_sql::sql_indice(&lectura, &I1, &v.rejilla, &ids, f, t),
                nuevo_fuente.filas,
                &dir_idx,
                &format!("indice-{}", c.instantanea),
                di,
            )?;
            subidas.extend(u.iter().filter_map(|x| indice::clave_de(x).ok()));
            nuevo_idx = contar_el_indice(c, &https, &https)?.0;
            uris = u;
            di(
                "nuevo",
                Json::obj([
                    ("filas", Json::s(nuevo_fuente.filas.to_string())),
                    ("tramos", Json::Int(tramos as i64)),
                    ("segundos", Json::s(format!("{segundos:.1}"))),
                ]),
            );
        }
        // ── lo que sale ─────────────────────────────────────────────────────────────────────────
        let (mut quitado, mut esperado, mut borr_uris) =
            (Cuenta::default(), Cuenta::default(), Vec::new());
        if !retirados.is_empty() || !borrados_fuente.is_empty() {
            let local = c.p.trabajo.join("borrados.parquet");
            let _ = std::fs::remove_file(&local);
            let cuenta_json = c.p.trabajo.join("quitar.json");
            let _ = std::fs::remove_file(&cuenta_json);
            let indice_vivo: Vec<(String, String)> = vivos_idx
                .iter()
                .map(|u| Ok((u.clone(), por_https(u)?)))
                .collect::<Result<_, String>>()?;
            let ordinales: Vec<u32> = retirados.iter().map(|(o, _)| *o).collect();
            let t = Instant::now();
            duckdb(
                p,
                &c.secretos,
                &(indice_sql::sql_quitar(
                    &indice_vivo,
                    &borrados_idx
                        .iter()
                        .map(|b| por_https(b))
                        .collect::<Result<Vec<_>, _>>()?,
                    &ordinales,
                    &borrados_fuente,
                    &viejos,
                    &local.display().to_string(),
                ) + &a_json(&indice_sql::sql_cuenta_quitar(&ordinales), &cuenta_json)),
            )?;
            let q = fila_json(&cuenta_json)?;
            quitado = Cuenta {
                filas: numero(&q, "filas")?,
                suma_ids: numero(&q, "suma_ids")?,
            };
            // lo que dice la fuente: las filas de cada fichero retirado (sin las que ya estaban
            // fuera) y las que nombran sus borrados nuevos sobre ficheros que se quedan
            let (ya_filas, ya_suma): (u64, u64) =
                (numero(&q, "ya_filas")?, numero(&q, "ya_suma_ids")?);
            let (f_filas, f_suma): (u64, u64) =
                (numero(&q, "fuente_filas")?, numero(&q, "fuente_suma_ids")?);
            let (mut n, mut suma) = (0u64, 0u64);
            for (o, filas) in &retirados {
                n += filas;
                suma = suma.wrapping_add(suma_de_un_fichero(*o, *filas));
            }
            esperado = Cuenta {
                filas: (n + f_filas).saturating_sub(ya_filas),
                suma_ids: suma.wrapping_add(f_suma).wrapping_sub(ya_suma),
            };
            if quitado != esperado {
                return Err(format!(
                    "lo que sale del índice no cuadra con la fuente: {} filas (suma {}) y la fuente \
                     dice {} (suma {})",
                    quitado.filas, quitado.suma_ids, esperado.filas, esperado.suma_ids
                ));
            }
            let posiciones: u64 = numero(&q, "posiciones")?;
            if posiciones > 0 {
                let clave = format!("{dir_idx}data/borrados-{}.parquet", c.instantanea);
                subir_como(&c.destino, &clave, &local, "application/vnd.apache.parquet")?;
                subidas.push(clave.clone());
                borr_uris.push(format!("gs://{}/{clave}", c.cubo_d));
            }
            let _ = std::fs::remove_file(&local);
            di(
                "quitado",
                Json::obj([
                    ("filas", Json::s(quitado.filas.to_string())),
                    ("posiciones", Json::s(posiciones.to_string())),
                    (
                        "segundos",
                        Json::s(format!("{:.1}", t.elapsed().as_secs_f64())),
                    ),
                ]),
            );
        }
        // ── la puerta: lo de T, por pasos (lo de S, lo nuevo, lo que sale) ──────────────────────
        let de_fuente = Cuenta {
            filas: (v.filas + nuevo_fuente.filas).saturating_sub(esperado.filas),
            suma_ids: v
                .suma_ids
                .wrapping_add(nuevo_fuente.suma_ids)
                .wrapping_sub(esperado.suma_ids),
        };
        let del_indice = Cuenta {
            filas: (v.filas + nuevo_idx.filas).saturating_sub(quitado.filas),
            suma_ids: v
                .suma_ids
                .wrapping_add(nuevo_idx.suma_ids)
                .wrapping_sub(quitado.suma_ids),
        };
        let comprobacion = indice::comprobar(
            &p.metadata_location,
            de_fuente,
            del_indice,
            p.piramide.as_ref(),
        )
        .map_err(|d| {
            d.iter()
                .map(|x| x.to_string())
                .collect::<Vec<_>>()
                .join("; ")
        })?;
        // ── un commit sobre la misma tabla del índice, y lo de T ────────────────────────────────
        let escrito = lago_d.anadir(
            &idx,
            &uris,
            &borr_uris,
            &[],
            [("ore.indice.fuente".to_string(), p.metadata_location.clone())].into(),
        )?;
        // compactar por niveles (I4·3): si falla, se publica lo seguido igual; compactar no
        // bloquea que el mapa vaya al día
        let tabla_t = match compactar(c, &lago_d, &escrito.tabla, &v.rejilla, &ids, &dir_idx, di) {
            Ok(t) => t,
            Err(e) => {
                di(
                    "compactar",
                    Json::obj([("error", Json::s(e)), ("sigue", Json::s("sin compactar"))]),
                );
                escrito.tabla.clone()
            }
        };
        let ficheros_t: Vec<(u32, String)> = viejos
            .iter()
            .filter(|(o, _)| !retirados.iter().any(|(r, _)| r == o))
            .cloned()
            .chain(nuevos.iter().map(|f| (f.ordinal, f.uri.clone())))
            .collect();
        let clave_ficheros_t = format!("{}{}/ficheros.json", c.prefijo, c.instantanea);
        c.destino
            .subir(&clave_ficheros_t, tabla_de_ficheros(&ficheros_t).as_bytes())?;
        subidas.push(clave_ficheros_t.clone());
        let vivos_t: Vec<String> = lago_d
            .vivos(&tabla_t)?
            .0
            .iter()
            .map(|u| por_https(u))
            .collect::<Result<_, _>>()?;
        let (_, prof, grupos) = contar_el_indice(c, &[], &vivos_t)?;
        let m = Manifiesto {
            receta: I1.nombre.into(),
            generacion: 0,
            fuente: v.fuente.clone(),
            snapshot_fuente: p.metadata_location.clone(),
            secuencia_fuente: c.secuencia_fuente,
            identidad: TipoDeIdentidad::Posicion,
            indice: tabla_t
                .metadata_location()
                .ok_or("la tabla del índice no tiene metadata.json")?
                .to_string(),
            ficheros: Some(clave_ficheros_t),
            piramide: p.piramide.clone(),
            suelo: v.suelo,
            rejilla: v.rejilla,
            profundidad: (prof * 100.0).round() / 100.0,
            filas: de_fuente.filas,
            suma_ids: de_fuente.suma_ids,
            creado_ms: ore_store::lago::ahora_ms(),
        };
        let r = indice::publicar(c.destino.as_ref(), &c.prefijo, m, &comprobacion)?;
        if let Publicado::Hecho(m) = &r {
            indice::dejar_el_de_la_instantanea(c.destino.as_ref(), &c.prefijo, m)?;
        }
        di(
            "seguido",
            Json::obj([
                ("filas", Json::s(de_fuente.filas.to_string())),
                ("profundidad", Json::s(format!("{prof:.2}"))),
                ("grupos", Json::Int(grupos as i64)),
            ]),
        );
        Ok(r)
    })();
    let r = match r {
        Ok(r) => r,
        Err(e) => {
            borrar(&c.destino, &subidas);
            return Err(e);
        }
    };
    fin(c, &r, di)?;
    Ok(Some(r))
}

/// **El nivel de un tramo ordenado del índice**, por el nombre de sus ficheros (I4·3): la base es lo
/// que construyó el índice entero (`indice-NNNN`) o una fusión con ella (`base-<inst>-NNNN`); una
/// carga que se siguió es el nivel 0 (`indice-<inst>-NNNN`); lo fundido, el suyo
/// (`nivel<L>-<inst>-NNNN`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Nivel {
    N(u32),
    Base,
}

/// `(el tramo al que pertenece un fichero, su nivel)`: el nombre sin `-NNNN.parquet`.
fn tramo_de(ruta: &str) -> (String, Nivel) {
    let nombre = ruta.rsplit('/').next().unwrap_or(ruta);
    let nombre = nombre.strip_suffix(".parquet").unwrap_or(nombre);
    let tramo = match nombre.rsplit_once('-') {
        Some((t, n)) if n.len() == 4 && n.bytes().all(|b| b.is_ascii_digit()) => t,
        _ => nombre,
    };
    let nivel = if tramo == "indice" || tramo.starts_with("base-") {
        Nivel::Base
    } else if let Some(l) = tramo
        .strip_prefix("nivel")
        .and_then(|r| r.split_once('-'))
        .and_then(|(l, _)| l.parse().ok())
    {
        Nivel::N(l)
    } else {
        Nivel::N(0)
    };
    (tramo.to_string(), nivel)
}

/// **Qué fundir** (I4·3): el nivel más bajo con `k` tramos o más, en uno del siguiente; y si lo que
/// sale tendría tantas filas como la base, todo en una base nueva (el índice, reordenado entero
/// desde sí mismo). `tramos`: `(tramo, nivel, filas)`. `None`: nada que fundir.
fn que_fundir(tramos: &[(String, Nivel, u64)], k: usize) -> Option<(Vec<String>, Nivel)> {
    let mut por_nivel: std::collections::BTreeMap<u32, Vec<&(String, Nivel, u64)>> =
        Default::default();
    for t in tramos {
        if let Nivel::N(l) = t.1 {
            por_nivel.entry(l).or_default().push(t);
        }
    }
    let (l, juntos) = por_nivel.into_iter().find(|(_, v)| v.len() >= k)?;
    let filas: u64 = juntos.iter().map(|t| t.2).sum();
    let base: u64 = tramos
        .iter()
        .filter(|t| t.1 == Nivel::Base)
        .map(|t| t.2)
        .sum();
    if filas >= base {
        return Some((tramos.iter().map(|t| t.0.clone()).collect(), Nivel::Base));
    }
    Some((
        juntos.iter().map(|t| t.0.clone()).collect(),
        Nivel::N(l + 1),
    ))
}

/// **Compactar por niveles** (I4·3): si un nivel junta [`PeticionIndice::compactar_cada`] tramos,
/// se funden ([`que_fundir`]) leyendo el índice, no la fuente: lo vivo (sin lo que quitan sus
/// borrados), reordenado por la clave de su caja, por tramos de la clave; lo que entra tiene que ser
/// lo que sale (filas y suma de ids), y un commit retira lo fundido y añade lo nuevo. Devuelve la
/// tabla del índice, compactada o como estaba.
fn compactar(
    c: &Contexto,
    lago_d: &Lago,
    tabla: &ore_store::lago::Tabla,
    rejilla: &Rejilla,
    ids: &[(String, i32)],
    dir_idx: &str,
    di: &mut dyn FnMut(&str, Json),
) -> Result<ore_store::lago::Tabla, String> {
    let ficheros = lago_d.ficheros_de_datos(tabla)?;
    let mut tramos: Vec<(String, Nivel, u64)> = Vec::new();
    for f in &ficheros {
        let (t, n) = tramo_de(&f.ruta);
        match tramos.iter_mut().find(|x| x.0 == t) {
            Some(x) => x.2 += f.filas,
            None => tramos.push((t, n, f.filas)),
        }
    }
    let Some((juntar, destino)) = que_fundir(&tramos, c.p.compactar_cada) else {
        return Ok(tabla.clone());
    };
    let fuera: Vec<String> = ficheros
        .iter()
        .filter(|f| juntar.contains(&tramo_de(&f.ruta).0))
        .map(|f| f.ruta.clone())
        .collect();
    let leer: Vec<(String, String)> = fuera
        .iter()
        .map(|u| Ok((u.clone(), por_https(u)?)))
        .collect::<Result<_, String>>()?;
    let borrados: Vec<String> = lago_d
        .vivos(tabla)?
        .1
        .iter()
        .map(|b| por_https(b))
        .collect::<Result<_, _>>()?;
    let t = Instant::now();
    let vista = indice_sql::sql_vista_fundir(&leer, &borrados, rejilla);
    let antes_json = c.p.trabajo.join("fundir.json");
    let _ = std::fs::remove_file(&antes_json);
    duckdb(
        c.p,
        &c.secretos,
        &(vista.clone() + &a_json(indice_sql::SQL_CUENTA_FUNDIR, &antes_json)),
    )?;
    let a = fila_json(&antes_json)?;
    let antes = Cuenta {
        filas: numero(&a, "filas")?,
        suma_ids: numero(&a, "suma_ids")?,
    };
    let nombre = match destino {
        Nivel::Base => format!("base-{}", c.instantanea),
        Nivel::N(l) => format!("nivel{l}-{}", c.instantanea),
    };
    let (uris, https, _, segundos, partes) = escribir_tramos(
        c,
        &vista,
        &indice_sql::sql_cortes_fundir,
        &|t, f| indice_sql::sql_fundir(ids, f, t),
        antes.filas,
        dir_idx,
        &nombre,
        di,
    )?;
    let subidas: Vec<String> = uris
        .iter()
        .filter_map(|u| indice::clave_de(u).ok())
        .collect();
    let r = (|| -> Result<ore_store::lago::Tabla, String> {
        let despues = contar_el_indice(c, &https, &https)?.0;
        if despues != antes {
            return Err(format!(
                "lo fundido no es lo que entró: {} filas (suma {}) de {} (suma {})",
                despues.filas, despues.suma_ids, antes.filas, antes.suma_ids
            ));
        }
        let e = lago_d.anadir(tabla, &uris, &[], &fuera, Default::default())?;
        Ok(e.tabla)
    })();
    match r {
        Ok(nueva) => {
            di(
                "compactado",
                Json::obj([
                    ("tramos_fundidos", Json::Int(juntar.len() as i64)),
                    ("en", Json::s(nombre)),
                    ("ficheros_fuera", Json::Int(fuera.len() as i64)),
                    ("filas", Json::s(antes.filas.to_string())),
                    ("partes", Json::Int(partes as i64)),
                    (
                        "segundos",
                        Json::s(format!("{:.1}", t.elapsed().as_secs_f64())),
                    ),
                    ("duckdb_s", Json::s(format!("{segundos:.1}"))),
                ]),
            );
            Ok(nueva)
        }
        Err(e) => {
            borrar(&c.destino, &subidas);
            Err(e)
        }
    }
}

/// **Construir el índice entero** (I2): la fuente entera, su extensión y una rejilla congelada
/// nueva, por tramos, en `<prefijo><instantánea>/`, registrado como una tabla nueva.
fn construir_entero(c: &Contexto, di: &mut dyn FnMut(&str, Json)) -> Result<Publicado, String> {
    let p = c.p;
    let (datos, borrados) = c.lago.vivos(&c.tabla)?;
    let dir = format!("{}{}/", c.prefijo, c.instantanea);
    let ficheros: Vec<FicheroFuente> = datos
        .iter()
        .enumerate()
        .map(|(i, uri)| {
            Ok(FicheroFuente {
                ordinal: i as u32,
                ruta: por_https(uri)?,
                uri: uri.clone(),
            })
        })
        .collect::<Result<_, String>>()?;
    let mut lectura = Lectura {
        ficheros: ficheros.clone(),
        borrados: borrados
            .iter()
            .map(|b| por_https(b))
            .collect::<Result<_, _>>()?,
        geometria: p.geometria.clone(),
        en_wkb: true,
        crs: p.crs.clone(),
        columnas: p.columnas.clone(),
        suelo: p.suelo,
    };
    let tipo = probar_geometria(c, &mut lectura)?;
    let sql_lectura = indice_sql::sql_lectura(&lectura)?;
    di(
        "inicio",
        Json::obj([
            ("ore_piramide", Json::s(crate::VERSION)),
            ("instantanea", Json::s(p.metadata_location.clone())),
            ("secuencia", Json::Int(c.secuencia_fuente as i64)),
            ("ficheros", Json::Int(datos.len() as i64)),
            ("borrados", Json::Int(borrados.len() as i64)),
            ("destino", Json::s(format!("gs://{}/{dir}", c.cubo_d))),
            ("geometria", Json::s(tipo)),
        ]),
    );
    // ── 2 · la extensión y la cuenta de la fuente ───────────────────────────────────────────────
    let (de_fuente, ext, s1) = contar(c, &sql_lectura)?;
    if de_fuente.filas == 0 {
        return Err("la fuente no tiene filas: no hay índice que construir".into());
    }
    let rejilla = Rejilla::congelar(ext, &I1)?;
    di(
        "fuente",
        Json::obj([
            ("filas", Json::s(de_fuente.filas.to_string())),
            ("segundos", Json::s(format!("{s1:.1}"))),
        ]),
    );
    // ── 3 y 4 · el índice, por tramos de la clave ───────────────────────────────────────────────
    let arrow = indice::esquema(&I1, &Lago::campos_arrow(&c.tabla, &p.columnas)?)?;
    let ids = Lago::ids_de_tabla_nueva(&arrow)?;
    let t = Instant::now();
    let (uris, https, bytes, s2, tramos) = escribir_tramos(
        c,
        &sql_lectura,
        &|n| indice_sql::sql_cortes(&rejilla, n),
        &|t, f| indice_sql::sql_indice(&lectura, &I1, &rejilla, &ids, f, t),
        de_fuente.filas,
        &dir,
        "indice",
        di,
    )?;
    let subidas: Vec<String> = uris
        .iter()
        .filter_map(|u| indice::clave_de(u).ok())
        .collect();
    let r = (|| -> Result<Publicado, String> {
        // la cuenta y la profundidad, sobre lo subido; la puerta: un índice que no cuadra no se
        // publica, y lo subido se va
        let (del_indice, prof, grupos) = contar_el_indice(c, &https, &https)?;
        let comprobacion: Comprobacion = indice::comprobar(
            &p.metadata_location,
            de_fuente,
            del_indice,
            p.piramide.as_ref(),
        )
        .map_err(|d| {
            d.iter()
                .map(|x| x.to_string())
                .collect::<Vec<_>>()
                .join("; ")
        })?;
        di(
            "indice",
            Json::obj([
                ("tramos", Json::Int(tramos as i64)),
                ("bytes", Json::s(bytes.to_string())),
                ("profundidad", Json::s(format!("{prof:.2}"))),
                ("grupos", Json::Int(grupos as i64)),
                ("segundos", Json::s(format!("{s2:.1}"))),
            ]),
        );
        // ── 5 · registrar ───────────────────────────────────────────────────────────────────────
        let clave_ficheros = format!("{dir}ficheros.json");
        c.destino.subir(
            &clave_ficheros,
            tabla_de_ficheros(
                &ficheros
                    .iter()
                    .map(|f| (f.ordinal, f.uri.clone()))
                    .collect::<Vec<_>>(),
            )
            .as_bytes(),
        )?;
        let escrito = Lago::nuevo(c.destino.clone()).registrar(
            &format!("gs://{}/{}", c.cubo_d, dir.trim_end_matches('/')),
            &format!("{}-{}", p.capa, I1.nombre),
            &arrow,
            &uris,
            [
                ("ore.indice.receta".to_string(), I1.nombre.to_string()),
                ("ore.indice.fuente".to_string(), p.metadata_location.clone()),
            ]
            .into(),
        )?;
        if escrito.filas < del_indice.filas {
            return Err(format!(
                "la tabla del índice tiene {} filas y se escribieron {} de la fuente",
                escrito.filas, del_indice.filas
            ));
        }
        di(
            "registrado",
            Json::obj([(
                "indice_y_subir_s",
                Json::s(format!("{:.1}", t.elapsed().as_secs_f64())),
            )]),
        );
        // ── 6 · publicar ────────────────────────────────────────────────────────────────────────
        let m = Manifiesto {
            receta: I1.nombre.into(),
            generacion: 0,
            fuente: Fuente {
                tipo: "lago".into(),
                nombre: p.capa.clone(),
                ubicacion: p.destino.ubicacion.clone(),
            },
            snapshot_fuente: p.metadata_location.clone(),
            secuencia_fuente: c.secuencia_fuente,
            identidad: TipoDeIdentidad::Posicion,
            indice: escrito
                .tabla
                .metadata_location()
                .ok_or("la tabla del índice no tiene metadata.json")?
                .to_string(),
            ficheros: Some(clave_ficheros),
            piramide: p.piramide.clone(),
            suelo: p.suelo,
            rejilla,
            profundidad: (prof * 100.0).round() / 100.0,
            filas: de_fuente.filas,
            suma_ids: de_fuente.suma_ids,
            creado_ms: ore_store::lago::ahora_ms(),
        };
        let r = indice::publicar(c.destino.as_ref(), &c.prefijo, m, &comprobacion)?;
        // la copia por nombre, la que encuentra quien sirve con un GET (I3·4)
        if let Publicado::Hecho(m) = &r {
            indice::dejar_el_de_la_instantanea(c.destino.as_ref(), &c.prefijo, m)?;
        }
        Ok(r)
    })();
    let r = match r {
        Ok(r) => r,
        Err(e) => {
            // lo de este directorio no lo nombra ningún manifiesto: se va entero
            if let Ok(objetos) = c.destino.listar(&dir) {
                borrar(&c.destino, &objetos);
            }
            borrar(&c.destino, &subidas);
            return Err(e);
        }
    };
    fin(c, &r, di)?;
    Ok(r)
}

/// Retirar lo viejo, limpiar el disco del Job y decir cómo acabó.
fn fin(c: &Contexto, r: &Publicado, di: &mut dyn FnMut(&str, Json)) -> Result<(), String> {
    let retiradas = indice::retirar(c.destino.as_ref(), &c.prefijo, 3)?;
    let _ = std::fs::remove_dir_all(c.p.trabajo.join("indice"));
    let _ = std::fs::remove_dir_all(c.p.trabajo.join("temp"));
    let (que, m) = match r {
        Publicado::Hecho(m) => ("hecho", m),
        Publicado::Superado(m) => ("superado", m),
    };
    di(
        "fin",
        Json::obj([
            ("resultado", Json::s(que)),
            ("generacion", Json::Int(m.generacion as i64)),
            ("indice", Json::s(m.indice.clone())),
            ("filas", Json::s(m.filas.to_string())),
            ("profundidad", Json::s(format!("{}", m.profundidad))),
            (
                "total_s",
                Json::s(format!("{:.1}", c.t0.elapsed().as_secs_f64())),
            ),
            (
                "retiradas",
                Json::Arr(retiradas.into_iter().map(|g| Json::Int(g as i64)).collect()),
            ),
        ]),
    );
    Ok(())
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// I4·3: el nivel de un tramo sale del nombre de sus ficheros.
    #[test]
    fn el_nivel_de_un_tramo_sale_de_su_nombre() {
        let d = "gs://c/t/_ore_capa/i1/00001-a/data/";
        assert_eq!(
            tramo_de(&format!("{d}indice-0000.parquet")),
            ("indice".into(), Nivel::Base)
        );
        assert_eq!(
            tramo_de(&format!("{d}base-00009-x-0002.parquet")),
            ("base-00009-x".into(), Nivel::Base)
        );
        assert_eq!(
            tramo_de(&format!("{d}indice-00003-b-0000.parquet")),
            ("indice-00003-b".into(), Nivel::N(0))
        );
        assert_eq!(
            tramo_de(&format!("{d}nivel2-00007-c-0001.parquet")),
            ("nivel2-00007-c".into(), Nivel::N(2))
        );
    }

    /// I4·3: se funde el nivel más bajo que junta `k`; si lo fundido pesaría como la base, todo.
    #[test]
    fn se_funde_el_nivel_que_junta_k_y_todo_si_pesa_como_la_base() {
        let t = |n: &str, l: Nivel, f: u64| (n.to_string(), l, f);
        let base = t("indice", Nivel::Base, 1_000);
        // tres cargas: nada
        let tres: Vec<_> = (0..3)
            .map(|i| t(&format!("indice-{i}"), Nivel::N(0), 10))
            .collect();
        let mut v = vec![base.clone()];
        v.extend(tres.clone());
        assert_eq!(que_fundir(&v, 4), None);
        // la cuarta: las cuatro, a nivel 1
        v.push(t("indice-3", Nivel::N(0), 10));
        let (juntar, a) = que_fundir(&v, 4).unwrap();
        assert_eq!((juntar.len(), a), (4, Nivel::N(1)));
        assert!(!juntar.contains(&"indice".to_string()));
        // con k = 2, ya con dos
        assert_eq!(que_fundir(&v[..3], 2).unwrap().1, Nivel::N(1));
        // el nivel más bajo primero
        let w = vec![
            base.clone(),
            t("nivel1-a", Nivel::N(1), 40),
            t("nivel1-b", Nivel::N(1), 40),
            t("indice-x", Nivel::N(0), 10),
            t("indice-y", Nivel::N(0), 10),
        ];
        assert_eq!(que_fundir(&w, 2).unwrap().1, Nivel::N(1));
        // si lo fundido pesa como la base: todo, en una base nueva
        let pesado = vec![
            t("indice", Nivel::Base, 100),
            t("indice-x", Nivel::N(0), 60),
            t("indice-y", Nivel::N(0), 60),
        ];
        let (juntar, a) = que_fundir(&pesado, 2).unwrap();
        assert_eq!((juntar.len(), a), (3, Nivel::Base));
    }

    /// La suma cerrada de los ids de un fichero es la de sumarlos uno a uno, con desbordamiento.
    #[test]
    fn la_suma_de_un_fichero_es_la_de_sus_filas() {
        for (o, n) in [
            (0u32, 0u64),
            (0, 1),
            (3, 7),
            (5, 100_000),
            (u32::MAX >> 8, 1_000),
        ] {
            let uno_a_uno = (0..n).fold(0u64, |s, f| s.wrapping_add(((o as u64) << 40) | f));
            assert_eq!(suma_de_un_fichero(o, n), uno_a_uno, "{o} {n}");
        }
    }

    #[test]
    fn la_peticion_del_indice() {
        let t = r#"{"tarea": "indice",
          "fuente": {"metadata_location": "gs://c/t/metadata/00003-a.metadata.json", "geometria": "g",
                     "crs": "EPSG:4326", "columnas": ["tarifa", "cuando"],
                     "credencial": {"gcs.oauth2.token": "x"}},
          "capa": "query.default.viajes", "suelo": 13,
          "destino": {"ubicacion": "gs://c/t"},
          "piramide": {"clave": "p", "snapshot_fuente": "gs://c/t/metadata/00003-a.metadata.json"},
          "trabajo": "/trabajo"}"#;
        assert!(es_de_indice(t));
        assert!(!es_de_indice(r#"{"fuente": {}}"#));
        let p = peticion_indice(t).unwrap();
        assert_eq!(p.columnas, ["tarifa", "cuando"]);
        assert_eq!(p.credencial.as_deref(), Some("x"));
        assert_eq!((p.suelo, p.destino.token.clone()), (13, None));
        assert_eq!(p.piramide.unwrap().clave, "p");
        assert_eq!(p.filas_por_tramo, FILAS_POR_TRAMO);
        assert_eq!(p.compactar_cada, COMPACTAR_CADA);
        let con = t.replace(
            "\"trabajo\"",
            "\"limites\": {\"filas_por_tramo\": 10}, \"trabajo\"",
        );
        assert_eq!(peticion_indice(&con).unwrap().filas_por_tramo, 10);
        assert!(peticion_indice(&t.replace("\"suelo\": 13", "\"suelo\": 30")).is_err());
        assert!(peticion_indice(&t.replace("\"capa\"", "\"otra\"")).is_err());
        assert_eq!(
            por_https("gs://c/t/data/a.parquet").unwrap(),
            "https://storage.googleapis.com/c/t/data/a.parquet"
        );
    }

    #[test]
    fn la_consulta_va_a_un_json() {
        let s = a_json("SELECT 1 AS a;\n", Path::new("/t/x.json"));
        assert_eq!(s, "COPY (SELECT 1 AS a) TO '/t/x.json' (FORMAT json);\n");
    }
}
