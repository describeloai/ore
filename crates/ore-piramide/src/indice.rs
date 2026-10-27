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
use ore_store::lago::Lago;
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
}

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

/// **Construir y publicar el índice** de la fuente en su snapshot vigente.
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
    if let Some(v) = indice::vigente(destino.as_ref(), &prefijo)?
        && v.secuencia_fuente >= secuencia_fuente
    {
        di(
            "ya",
            Json::obj([("generacion", Json::Int(v.generacion as i64))]),
        );
        return Ok(Publicado::Superado(v));
    }
    let (datos, borrados) = lago.vivos(&tabla)?;
    let instantanea = p
        .metadata_location
        .rsplit('/')
        .next()
        .and_then(|f| f.strip_suffix(".metadata.json"))
        .ok_or_else(|| format!("`{}` no es un metadata.json", p.metadata_location))?;
    let dir = format!("{prefijo}{instantanea}/");
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
    // ── 0 · cómo llega la geometría a DuckDB (WKB, o ya `GEOMETRY`) ─────────────────────────────
    std::fs::create_dir_all(&p.trabajo).map_err(|e| e.to_string())?;
    let tipo_json = p.trabajo.join("tipo.json");
    let _ = std::fs::remove_file(&tipo_json);
    duckdb(
        p,
        &secretos,
        &a_json(&indice_sql::sql_tipo_de_la_geometria(&lectura)?, &tipo_json),
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
                p.geometria
            ));
        }
    };
    let sql_lectura = indice_sql::sql_lectura(&lectura)?;
    di(
        "inicio",
        Json::obj([
            ("ore_piramide", Json::s(crate::VERSION)),
            ("instantanea", Json::s(p.metadata_location.clone())),
            ("secuencia", Json::Int(secuencia_fuente as i64)),
            ("ficheros", Json::Int(datos.len() as i64)),
            ("borrados", Json::Int(borrados.len() as i64)),
            ("destino", Json::s(format!("gs://{cubo_d}/{dir}"))),
            ("geometria", Json::s(tipo.clone())),
        ]),
    );
    // ── 2 · la extensión y la cuenta de la fuente ───────────────────────────────────────────────
    let ext_json = p.trabajo.join("extension.json");
    let _ = std::fs::remove_file(&ext_json);
    let s1 = duckdb(
        p,
        &secretos,
        &(sql_lectura.clone() + &a_json(indice_sql::SQL_EXTENSION_Y_CUENTA, &ext_json)),
    )?;
    let e = fila_json(&ext_json)?;
    let de_fuente = Cuenta {
        filas: numero(&e, "filas")?,
        suma_ids: numero(&e, "suma_ids")?,
    };
    if de_fuente.filas == 0 {
        return Err("la fuente no tiene filas: no hay índice que construir".into());
    }
    let ext = [
        numero::<f64>(&e, "xmin")?,
        numero(&e, "ymin")?,
        numero(&e, "xmax")?,
        numero(&e, "ymax")?,
    ];
    let rejilla = Rejilla::congelar(ext, &I1)?;
    di(
        "fuente",
        Json::obj([
            ("filas", Json::s(de_fuente.filas.to_string())),
            ("segundos", Json::s(format!("{s1:.1}"))),
        ]),
    );
    // ── 3 y 4 · el índice, por tramos de la clave ───────────────────────────────────────────────
    // Lo que cabe en el disco del Job es un tramo, no la tabla (I2·4: 600 M filas desbordaron los
    // 40 GiB de una vez). Cada tramo es un `COPY` con su filtro, en su fichero, que se sube y se
    // borra antes del siguiente; la cuenta y la profundidad se leen después sobre lo subido.
    let arrow = indice::esquema(&I1, &Lago::campos_arrow(&tabla, &p.columnas)?)?;
    let ids = Lago::ids_de_tabla_nueva(&arrow)?;
    let n_tramos = de_fuente.filas.div_ceil(p.filas_por_tramo.max(1)) as usize;
    let tramos = if n_tramos <= 1 {
        vec![Tramo::default()]
    } else {
        let cortes_json = p.trabajo.join("cortes.json");
        let _ = std::fs::remove_file(&cortes_json);
        duckdb(
            p,
            &secretos,
            &(sql_lectura.clone()
                + &a_json(&indice_sql::sql_cortes(&rejilla, n_tramos), &cortes_json)),
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
    let t = Instant::now();
    let (mut uris, mut https, mut bytes, mut s2) = (Vec::new(), Vec::new(), 0u64, 0.0);
    // si algo falla a medias, lo subido no se queda: no lo nombra ningún manifiesto
    let limpiar = |destino: &Arc<dyn Almacen>| {
        if let Ok(objetos) = destino.listar(&dir) {
            for o in objetos {
                let _ = destino.borrar(&o);
            }
        }
    };
    for (i, tramo) in tramos.iter().enumerate() {
        let hecho = (|| -> Result<(), String> {
            let _ = std::fs::remove_dir_all(&salida);
            std::fs::create_dir_all(&salida).map_err(|e| e.to_string())?;
            let local = salida.join(format!("indice-{i:04}.parquet"));
            let s = duckdb(
                p,
                &secretos,
                &(sql_lectura.clone()
                    + &indice_sql::sql_indice(
                        &lectura,
                        &I1,
                        &rejilla,
                        &ids,
                        &local.display().to_string(),
                        tramo,
                    )?),
            )?;
            s2 += s;
            let b = std::fs::metadata(&local).map_err(|e| e.to_string())?.len();
            bytes += b;
            let clave = format!("{dir}data/indice-{i:04}.parquet");
            subir_como(&destino, &clave, &local, "application/vnd.apache.parquet")?;
            let _ = std::fs::remove_file(&local);
            uris.push(format!("gs://{cubo_d}/{clave}"));
            https.push(format!("https://storage.googleapis.com/{cubo_d}/{clave}"));
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
            limpiar(&destino);
            return Err(e);
        }
    }
    // la cuenta y la profundidad, sobre lo subido
    let cuenta_json = p.trabajo.join("cuenta.json");
    let _ = std::fs::remove_file(&cuenta_json);
    let profundidad_json = p.trabajo.join("profundidad.json");
    let _ = std::fs::remove_file(&profundidad_json);
    let comprobado = (|| -> Result<(Cuenta, f64, u64), String> {
        duckdb(
            p,
            &secretos,
            &(a_json(&indice_sql::sql_cuenta_indice(&https), &cuenta_json)
                + &a_json(&indice_sql::sql_profundidad(&https), &profundidad_json)),
        )?;
        let c = fila_json(&cuenta_json)?;
        let pr = fila_json(&profundidad_json)?;
        Ok((
            Cuenta {
                filas: numero(&c, "filas")?,
                suma_ids: numero(&c, "suma_ids")?,
            },
            numero(&pr, "profundidad")?,
            numero(&pr, "grupos")?,
        ))
    })();
    let (del_indice, prof, grupos) = match comprobado {
        Ok(v) => v,
        Err(e) => {
            limpiar(&destino);
            return Err(e);
        }
    };
    // la puerta: un índice que no cuadra no se publica, y lo subido se va
    let comprobacion: Comprobacion = match indice::comprobar(
        &p.metadata_location,
        de_fuente,
        del_indice,
        p.piramide.as_ref(),
    ) {
        Ok(c) => c,
        Err(d) => {
            limpiar(&destino);
            return Err(d
                .iter()
                .map(|x| x.to_string())
                .collect::<Vec<_>>()
                .join("; "));
        }
    };
    di(
        "indice",
        Json::obj([
            ("tramos", Json::Int(tramos.len() as i64)),
            ("bytes", Json::s(bytes.to_string())),
            ("profundidad", Json::s(format!("{prof:.2}"))),
            ("grupos", Json::Int(grupos as i64)),
            ("segundos", Json::s(format!("{s2:.1}"))),
        ]),
    );
    // ── 5 · registrar ───────────────────────────────────────────────────────────────────────────
    let tabla_de_ficheros = Json::obj([(
        "ficheros",
        Json::Arr(
            ficheros
                .iter()
                .map(|f| Json::Arr(vec![Json::Int(f.ordinal as i64), Json::s(f.uri.clone())]))
                .collect(),
        ),
    )]);
    let clave_ficheros = format!("{dir}ficheros.json");
    destino.subir(&clave_ficheros, tabla_de_ficheros.jcs().as_bytes())?;
    let escrito = Lago::nuevo(destino.clone()).registrar(
        &format!("gs://{cubo_d}/{}", dir.trim_end_matches('/')),
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
    let subir_s = t.elapsed().as_secs_f64();
    // ── 6 · publicar ────────────────────────────────────────────────────────────────────────────
    let m = Manifiesto {
        receta: I1.nombre.into(),
        generacion: 0,
        fuente: Fuente {
            tipo: "lago".into(),
            nombre: p.capa.clone(),
            ubicacion: p.destino.ubicacion.clone(),
        },
        snapshot_fuente: p.metadata_location.clone(),
        secuencia_fuente,
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
    let r = indice::publicar(destino.as_ref(), &prefijo, m, &comprobacion)?;
    let retiradas = indice::retirar(destino.as_ref(), &prefijo, 3)?;
    let _ = std::fs::remove_dir_all(&salida);
    let _ = std::fs::remove_dir_all(p.trabajo.join("temp"));
    let (que, m) = match &r {
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
            ("indice_y_subir_s", Json::s(format!("{subir_s:.1}"))),
            (
                "total_s",
                Json::s(format!("{:.1}", t0.elapsed().as_secs_f64())),
            ),
            (
                "retiradas",
                Json::Arr(retiradas.into_iter().map(|g| Json::Int(g as i64)).collect()),
            ),
        ]),
    );
    Ok(r)
}

#[cfg(test)]
mod pruebas {
    use super::*;

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
