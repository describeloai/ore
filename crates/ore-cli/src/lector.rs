//! El lector: de una fuente viva a un **catálogo**.
//!
//! # El compilador no habla con la nube, y eso no es una promesa
//!
//! `main.rs` rotula la sección del compilador *«CI · hermético: sin red, sin
//! credenciales, sin reloj»*. Esa línea puede significar dos cosas muy distintas,
//! y la diferencia es exactamente la que este proyecto lleva persiguiendo:
//!
//! - **Una política**: el binario sabe hablar por la red y se abstiene.
//! - **Una propiedad**: el binario **no sabe** hablar por la red.
//!
//! Lo segundo se comprueba mirando el árbol de dependencias; lo primero solo se
//! puede creer. Y la herméticidad no es una propiedad de un subcomando: es del
//! **artefacto**. Una pila TLS enlazada para `discover` está igual de presente
//! en `compile`.
//!
//! Medido, no supuesto. `ore` hoy enlaza **28 crates**, ninguna nativa. Un
//! cliente HTTPS mínimo —`reqwest` a secas, sin OAuth, sin el modelo REST de
//! BigQuery, sin un segundo driver— son **91**, cinco de ellas cripto o FFI.
//! Triplicar el árbol para que `discover` llame a una API le quitaría a `compile`
//! la única afirmación que podía demostrar.
//!
//! Y desde que existe `ore-read-postgres` esto no se sostiene sobre la buena
//! voluntad: `tests/dependencias.rs` lee el cierre de `ore-cli` en `Cargo.lock` y
//! falla si aparece una crate de red, de TLS o de FFI. La primera vez que corrió
//! corrigió la cifra que había escrita aquí, que era otra.
//!
//! # Cómo habla entonces: delegando
//!
//! ORE **no** abre un socket: ejecuta un programa que el usuario ya tiene y ya
//! autenticó, y lee su salida. Tres consecuencias, y las tres son buenas:
//!
//! 1. **La credencial nunca entra en el espacio de direcciones de ORE.** `bq`
//!    resuelve su propia autenticación. Es la misma doctrina que `source add` ya
//!    aplica —*«declara dónde buscarlo, no qué es»*— llevada un paso más: aquí ni
//!    siquiera hace falta buscarlo.
//! 2. **El sistema de tipos de la fuente vive de este lado de la costura.** El
//!    inductor recibe tipos de OOS; nunca ve un `NUMERIC`.
//! 3. **Añadir una fuente no añade una dependencia.** Postgres no trae un driver:
//!    trae una receta, o un `ore-read-postgres` en el PATH.
//!
//! # Una llamada, no N
//!
//! La forma ingenua —listar tablas y describir cada una— es la equivocada, y se
//! ve midiendo: `bq show` tarda ~10 s por tabla, así que **doce tablas agotaron
//! un límite de dos minutos**. Un almacén real tiene cientos. La receta de
//! BigQuery hace **una** consulta a `INFORMATION_SCHEMA` que devuelve columnas,
//! tipos, nulabilidad, claves, descripciones y número de filas de todo el
//! dataset: las mismas doce tablas, 13,8 s.
//!
//! # El programa delegado es del usuario, y puede fallar
//!
//! Puede faltar, no estar autenticado, o estar roto de formas que no son culpa de
//! nadie aquí: en la máquina donde se escribió esto, `bq` respondía `ERROR: (bq)
//! python3.14: command not found`. Ese texto es lo único accionable que existe,
//! así que **se muestra literal**. Un lector que dijera «no se pudo leer la
//! fuente» convertiría un problema de cinco minutos en una tarde.
//!
//! Y en Windows `bq` es `bq.cmd`: los programas se resuelven contra `PATH` y
//! `PATHEXT`, porque `CreateProcess` no lo hace por su cuenta.

use ore_core::json::Json;
use ore_core::parse::{self, Node};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const MANIFIESTO: &str = "ontology.config.yaml";
const SECRETOS: &str = ".env.local";

pub struct Fallo {
    pub codigo: u8,
    pub mensaje: String,
    pub ayuda: Vec<String>,
}

fn fallo(codigo: u8, mensaje: impl Into<String>, ayuda: &[&str]) -> Fallo {
    Fallo {
        codigo,
        mensaje: mensaje.into(),
        ayuda: ayuda.iter().map(|s| (*s).to_string()).collect(),
    }
}

/// Lee el catálogo de una fuente declarada en el manifiesto y lo devuelve en el
/// mismo JSON que acepta `--from`. Que sean el mismo texto no es comodidad: es lo
/// que permite probar la costura por los dos lados.
pub fn catalogo(raiz: &Path, fuente: &str) -> Result<String, Fallo> {
    let (tipo, env) = declaracion(raiz, fuente)?;
    let url = url(raiz, &env, fuente)?;
    match tipo.as_str() {
        "bigquery" => bigquery(fuente, &url),
        _ => externo(&tipo, fuente, &url),
    }
}

// ── El manifiesto y el secreto ──────────────────────────────────────────────

fn declaracion(raiz: &Path, fuente: &str) -> Result<(String, String), Fallo> {
    let ruta = raiz.join(MANIFIESTO);
    let texto = std::fs::read_to_string(&ruta).map_err(|e| {
        fallo(
            66, // EX_NOINPUT
            format!("no se pudo leer `{}`: {e}", ruta.display()),
            &["  `ore init` crea uno."],
        )
    })?;
    let arbol = parse::parse(&texto)
        .map_err(|e| fallo(65, format!("`{MANIFIESTO}` no analiza: {e:?}"), &[]))?;

    let ds = arbol
        .get("datasources")
        .map(|(_, v)| v.items())
        .unwrap_or(&[]);
    let Some(d) = ds
        .iter()
        .find(|it| it.get("name").and_then(|(_, v)| v.as_str()) == Some(fuente))
    else {
        let nombres: Vec<&str> = ds
            .iter()
            .filter_map(|it| it.get("name").and_then(|(_, v)| v.as_str()))
            .collect();
        let ayuda = if nombres.is_empty() {
            "  No hay ninguna declarada. `ore source add --name <n> <url>`.".to_string()
        } else {
            format!("  Declaradas: {}", nombres.join(", "))
        };
        return Err(Fallo {
            codigo: 65,
            mensaje: format!("`{fuente}` no está declarada en `{MANIFIESTO}`"),
            ayuda: vec![ayuda],
        });
    };

    let campo = |k: &str| d.get(k).and_then(|(_, v)| v.as_str()).map(String::from);
    let tipo = campo("type").ok_or_else(|| {
        fallo(
            65,
            format!("la fuente `{fuente}` no declara `type`"),
            &["  Sin tipo no hay receta que aplicar, y adivinarla sería inventarla."],
        )
    })?;
    let env = campo("connectionEnv").ok_or_else(|| {
        fallo(
            65,
            format!("la fuente `{fuente}` no declara `connectionEnv`"),
            &["  Es el campo que dice DÓNDE está la conexión. Sin él no hay dónde mirar."],
        )
    })?;
    Ok((tipo, env))
}

/// El entorno del proceso manda; `.env.local` es el respaldo local.
///
/// Que ORE lea `.env.local` no es comodidad: `source add` lo **escribe**, y un
/// fichero que se escribe y nadie lee es la misma figura que este proyecto lleva
/// encontrando una y otra vez. En CI no existe, y ahí manda el entorno.
fn url(raiz: &Path, env: &str, fuente: &str) -> Result<String, Fallo> {
    if let Ok(v) = std::env::var(env)
        && !v.trim().is_empty()
    {
        return Ok(v.trim().to_string());
    }
    if let Ok(texto) = std::fs::read_to_string(raiz.join(SECRETOS)) {
        for linea in texto.lines() {
            let l = linea.trim();
            let l = l.strip_prefix("export ").unwrap_or(l);
            if l.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = l.split_once('=')
                && k.trim() == env
            {
                let v = v.trim().trim_matches('"').trim_matches('\'');
                if !v.is_empty() {
                    return Ok(v.to_string());
                }
            }
        }
    }
    Err(Fallo {
        codigo: 69, // EX_UNAVAILABLE
        mensaje: format!("`{env}` no está definida"),
        ayuda: vec![
            format!("  La declara la fuente `{fuente}`, y no se inventa."),
            "  Defínela en el entorno, o en `.env.local` del repositorio.".to_string(),
        ],
    })
}

// ── BigQuery ────────────────────────────────────────────────────────────────

/// `bigquery://<proyecto>/<dataset>`.
fn destino(url: &str) -> Option<(String, String)> {
    let resto = url.split_once("://")?.1;
    let mut p = resto.trim_end_matches('/').splitn(2, '/');
    let proyecto = p.next()?.trim();
    let dataset = p.next()?.trim();
    (!proyecto.is_empty() && !dataset.is_empty())
        .then(|| (proyecto.to_string(), dataset.to_string()))
}

/// Una consulta, todo el dataset.
///
/// `ANY_VALUE` sobre la tabla referenciada no es una elección: en una clave
/// compuesta todas las filas nombran la **misma** tabla, y sin él el producto
/// cartesiano multiplicaría las columnas. Salió midiendo: una clave de dos
/// columnas devolvía cuatro filas.
fn consulta(d: &str) -> String {
    format!(
        "WITH kc AS (\n\
         \x20 SELECT k.table_name, k.column_name\n\
         \x20 FROM `{d}`.INFORMATION_SCHEMA.KEY_COLUMN_USAGE k\n\
         \x20 JOIN `{d}`.INFORMATION_SCHEMA.TABLE_CONSTRAINTS tc ON tc.constraint_name = k.constraint_name\n\
         \x20 WHERE tc.constraint_type = 'PRIMARY KEY'\n\
         ), fk AS (\n\
         \x20 SELECT k.table_name, k.column_name, ANY_VALUE(u.table_name) AS ref_table\n\
         \x20 FROM `{d}`.INFORMATION_SCHEMA.KEY_COLUMN_USAGE k\n\
         \x20 JOIN `{d}`.INFORMATION_SCHEMA.TABLE_CONSTRAINTS tc ON tc.constraint_name = k.constraint_name\n\
         \x20  AND tc.constraint_type = 'FOREIGN KEY'\n\
         \x20 JOIN `{d}`.INFORMATION_SCHEMA.CONSTRAINT_COLUMN_USAGE u ON u.constraint_name = k.constraint_name\n\
         \x20 GROUP BY k.table_name, k.column_name\n\
         ), n AS (SELECT table_id, row_count FROM `{d}.__TABLES__`)\n\
         SELECT c.table_name, t.table_type, n.row_count, c.column_name, c.ordinal_position,\n\
         \x20      c.is_nullable, c.data_type, fp.description AS column_description,\n\
         \x20      kc.column_name IS NOT NULL AS is_key, fk.ref_table\n\
         FROM `{d}`.INFORMATION_SCHEMA.COLUMNS c\n\
         JOIN `{d}`.INFORMATION_SCHEMA.TABLES t ON t.table_name = c.table_name\n\
         LEFT JOIN n ON n.table_id = c.table_name\n\
         LEFT JOIN `{d}`.INFORMATION_SCHEMA.COLUMN_FIELD_PATHS fp\n\
         \x20 ON fp.table_name = c.table_name AND fp.field_path = c.column_name\n\
         LEFT JOIN kc ON kc.table_name = c.table_name AND kc.column_name = c.column_name\n\
         LEFT JOIN fk ON fk.table_name = c.table_name AND fk.column_name = c.column_name\n\
         ORDER BY c.table_name, c.ordinal_position"
    )
}

fn bigquery(fuente: &str, url: &str) -> Result<String, Fallo> {
    let (proyecto, dataset) = destino(url).ok_or_else(|| {
        fallo(
            65,
            "la URL de una fuente `bigquery` no tiene la forma esperada",
            &["  `bigquery://<proyecto>/<dataset>`"],
        )
    })?;
    let salida = ejecutar(
        "bq",
        &[
            "query".to_string(),
            "--format=prettyjson".to_string(),
            "--use_legacy_sql=false".to_string(),
            "--max_rows=100000".to_string(),
            "--quiet".to_string(),
            format!("--project_id={proyecto}"),
        ],
        // La consulta va por **stdin**, no en la línea de órdenes, y las tres
        // razones apuntan al mismo sitio. `argv` tiene un límite de ~32 kB en
        // Windows y una consulta crece. Rust se niega a pasarle a un `.cmd`
        // argumentos con saltos de línea desde CVE-2024-24576, y `bq` en Windows
        // ES un `.cmd`. Y es lo mismo que ya hace `externo`: lo que es un dato no
        // viaja por la línea de órdenes, que la lee cualquiera.
        Some(&consulta(&dataset)),
    )?;
    let filas = parse::parse(&salida).map_err(|e| {
        fallo(
            65,
            format!("lo que devolvió `bq` no analiza: {e:?}"),
            &["  Se esperaba el JSON de `bq query --format=prettyjson`."],
        )
    })?;
    Ok(armar(fuente, &dataset, filas.items()).pretty())
}

// ── La costura de tipos ─────────────────────────────────────────────────────

/// De un tipo de BigQuery al vocabulario de escalares de OOS.
///
/// `None` significa **no lo sé traducir**, y ahí termina el trabajo de este
/// módulo: no se sustituye por `Opaque`. `Opaque` afirma *«hay un valor y su
/// interior no se modela»*, que es cierto de un `BYTES` y **falso** de un
/// `STRUCT<nom STRING, direccion STRUCT<…>>`, cuya estructura el origen acaba de
/// enumerar. Traducirlo a `Opaque` tiraría un hecho; traducirlo a entidades
/// anidadas inventaría un modelo. Se reporta.
fn tipo_oos(bq: &str) -> Option<&'static str> {
    let base = bq.split_once('(').map_or(bq, |(b, _)| b).trim();
    Some(match base {
        "INT64" | "INTEGER" | "INT" | "SMALLINT" | "BIGINT" | "TINYINT" | "BYTEINT" => "Integer",
        "NUMERIC" | "DECIMAL" | "BIGNUMERIC" | "BIGDECIMAL" => "Decimal",
        "FLOAT64" | "FLOAT" => "Float",
        "BOOL" | "BOOLEAN" => "Boolean",
        "STRING" => "String",
        "DATE" => "Date",
        "TIME" => "Time",
        "DATETIME" => "DateTime",
        // `TIMESTAMP` en BigQuery es un instante absoluto; `DATETIME` es civil.
        // La distinción existe en los dos sistemas de tipos y se conserva.
        "TIMESTAMP" => "DateTimeTz",
        "BYTES" | "JSON" | "GEOGRAPHY" | "INTERVAL" => "Opaque",
        _ => return None,
    })
}

/// `ARRAY<X>` es `list<X>` si `X` se sabe traducir. `ARRAY<STRUCT<…>>` no.
fn traducir(bq: &str) -> Option<String> {
    let t = bq.trim();
    if let Some(dentro) = t.strip_prefix("ARRAY<").and_then(|r| r.strip_suffix('>')) {
        return tipo_oos(dentro).map(|e| format!("list<{e}>"));
    }
    tipo_oos(t).map(String::from)
}

fn clase(table_type: &str) -> &'static str {
    match table_type {
        "VIEW" => "view",
        "MATERIALIZED VIEW" => "materializedView",
        _ => "table",
    }
}

/// Agrupa las filas planas de la consulta en tablas. Llegan ordenadas por
/// `(table_name, ordinal_position)`, y ese orden se conserva: el orden de las
/// columnas es del origen y no nos toca reordenarlo.
fn armar(fuente: &str, dataset: &str, filas: &[Node]) -> Json {
    struct Acc {
        clase: &'static str,
        filas: Option<i64>,
        columnas: Vec<Json>,
        clave: Vec<Json>,
        foraneas: BTreeMap<String, Vec<String>>,
    }
    fn campo(n: &Node, k: &str) -> Option<String> {
        n.get(k)
            .and_then(|(_, v)| v.as_str())
            .filter(|s| *s != "null")
            .map(String::from)
    }

    let mut orden: Vec<String> = Vec::new();
    let mut tablas: BTreeMap<String, Acc> = BTreeMap::new();

    for f in filas {
        let (Some(tabla), Some(columna), Some(tipo_bruto)) = (
            campo(f, "table_name"),
            campo(f, "column_name"),
            campo(f, "data_type"),
        ) else {
            continue;
        };
        let acc = tablas.entry(tabla.clone()).or_insert_with(|| {
            orden.push(tabla.clone());
            Acc {
                clase: clase(campo(f, "table_type").as_deref().unwrap_or("")),
                filas: campo(f, "row_count").and_then(|r| r.parse().ok()),
                columnas: Vec::new(),
                clave: Vec::new(),
                foraneas: BTreeMap::new(),
            }
        });

        let mut c: BTreeMap<String, Json> = BTreeMap::new();
        c.insert("name".to_string(), Json::s(&columna));
        match traducir(&tipo_bruto) {
            Some(t) => {
                c.insert("type".to_string(), Json::s(t));
            }
            // Sin `type`. `sourceType` no se interpreta aguas abajo: se cita.
            None => {
                c.insert("sourceType".to_string(), Json::s(&tipo_bruto));
            }
        }
        if campo(f, "is_nullable").as_deref() == Some("NO") {
            c.insert("required".to_string(), Json::Bool(true));
        }
        if let Some(d) = campo(f, "column_description").filter(|d| !d.trim().is_empty()) {
            c.insert("description".to_string(), Json::s(d));
        }
        acc.columnas.push(Json::Obj(c));

        if campo(f, "is_key").as_deref() == Some("true") {
            acc.clave.push(Json::s(&columna));
        }
        if let Some(r) = campo(f, "ref_table") {
            acc.foraneas
                .entry(format!("{dataset}.{r}"))
                .or_default()
                .push(columna);
        }
    }

    let tablas = orden
        .into_iter()
        .filter_map(|t| {
            let a = tablas.remove(&t)?;
            let mut o: BTreeMap<String, Json> = BTreeMap::new();
            o.insert("name".to_string(), Json::s(format!("{dataset}.{t}")));
            o.insert("kind".to_string(), Json::s(a.clase));
            o.insert("columns".to_string(), Json::Arr(a.columnas));
            if !a.clave.is_empty() {
                o.insert("primaryKey".to_string(), Json::Arr(a.clave));
            }
            if !a.foraneas.is_empty() {
                o.insert(
                    "foreignKeys".to_string(),
                    Json::Arr(
                        a.foraneas
                            .into_iter()
                            .map(|(destino, cols)| {
                                Json::obj([
                                    ("columns", Json::Arr(cols.iter().map(Json::s).collect())),
                                    ("references", Json::s(destino)),
                                ])
                            })
                            .collect(),
                    ),
                );
            }
            if let Some(n) = a.filas {
                o.insert("rows".to_string(), Json::Int(n));
            }
            Some(Json::Obj(o))
        })
        .collect();

    Json::obj([("source", Json::s(fuente)), ("tables", Json::Arr(tablas))])
}

// ── La costura de extensión ─────────────────────────────────────────────────

/// Un tipo sin receta propia se busca como `ore-read-<tipo>` en el `PATH`, al
/// modo de los subcomandos de `git` o `cargo`.
///
/// La URL viaja por **stdin**, nunca por la línea de órdenes: `argv` es legible
/// por cualquier proceso de la máquina, y para casi todo lo que no sea BigQuery
/// la URL lleva la credencial dentro.
fn externo(tipo: &str, fuente: &str, url: &str) -> Result<String, Fallo> {
    let programa = format!("ore-read-{tipo}");
    if resolver(&programa).is_none() {
        return Err(fallo(
            69,
            format!("no hay lector para una fuente de tipo `{tipo}`"),
            &[
                "  ORE trae una receta para `bigquery`. Para el resto delega:",
                "  pon un `ore-read-<tipo>` en el PATH que lea la URL por stdin y",
                "  escriba un catálogo por stdout, o pásale uno hecho con `--from`.",
                "  No se inventa un lector, igual que no se inventa un tipo.",
            ],
        ));
    }
    let salida = ejecutar(&programa, &[fuente.to_string()], Some(url))?;
    // Se comprueba que analiza aquí para que el error diga QUIÉN lo produjo.
    parse::parse(&salida).map_err(|e| {
        fallo(
            65,
            format!("lo que devolvió `{programa}` no analiza: {e:?}"),
            &["  Un lector externo escribe un catálogo JSON por stdout."],
        )
    })?;
    Ok(salida)
}

// ── Ejecutar un programa ajeno ──────────────────────────────────────────────

/// `CreateProcess` no consulta `PATHEXT`, así que hay que resolver a mano. Y no
/// solo por Windows: saber qué fichero exacto se va a ejecutar es lo que permite
/// nombrarlo en el error.
///
/// **Las extensiones van primero**, y eso costó un error. En el `bin` del SDK de
/// Google conviven `bq` —un guion de shell— y `bq.cmd`. Los dos son ficheros, así
/// que probar el nombre desnudo primero encontraba el guion y `CreateProcess`
/// respondía *«%1 no es una aplicación Win32 válida»*: en Windows `is_file()` no
/// es «es ejecutable», y los dos candidatos tienen exactamente el mismo aspecto.
/// Donde `PATHEXT` no existe —todo lo que no sea Windows— la lista está vacía y el
/// nombre desnudo es el único candidato, que es lo correcto allí.
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

fn ejecutar(programa: &str, args: &[String], entrada: Option<&str>) -> Result<String, Fallo> {
    let ruta = resolver(programa).ok_or_else(|| {
        fallo(
            69,
            format!("no se encontró `{programa}` en el PATH"),
            &["  Es el programa que habla con la fuente. ORE no lo lleva dentro."],
        )
    })?;

    let mut cmd = Command::new(&ruta);
    cmd.args(args)
        .stdin(if entrada.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut hijo = cmd.spawn().map_err(|e| {
        fallo(
            69,
            format!("no se pudo ejecutar `{}`: {e}", ruta.display()),
            &[],
        )
    })?;
    if let Some(t) = entrada
        && let Some(mut s) = hijo.stdin.take()
    {
        use std::io::Write as _;
        let _ = s.write_all(t.as_bytes());
    }
    let salida = hijo
        .wait_with_output()
        .map_err(|e| fallo(69, format!("`{programa}` no terminó: {e}"), &[]))?;

    if !salida.status.success() {
        // Su stderr literal es lo único accionable que existe. Resumirlo aquí
        // convertiría un problema de cinco minutos en una tarde: `bq` avisa de
        // que le falta un intérprete, o de que no hay sesión, y las dos cosas se
        // arreglan solas en cuanto se leen.
        let err = String::from_utf8_lossy(&salida.stderr);
        let mut ayuda = vec![format!("  {}", ruta.display())];
        ayuda.extend(
            err.lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| format!("  │ {l}")),
        );
        return Err(Fallo {
            codigo: 69,
            mensaje: format!(
                "`{programa}` falló ({})",
                salida
                    .status
                    .code()
                    .map_or_else(|| "sin código".to_string(), |c| c.to_string())
            ),
            ayuda,
        });
    }
    String::from_utf8(salida.stdout)
        .map_err(|_| fallo(65, format!("`{programa}` no devolvió UTF-8"), &[]))
}

// ── Comprobaciones ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Las filas exactas que devolvió `bq` sobre `rubix_demo_ventas`, recortadas.
    /// Es una captura, no un invento: la clave compuesta de `clientes` y el
    /// `STRUCT` de `pedidos_anidados` son los dos casos que costaron sangre.
    const FILAS: &str = r#"[
  {"table_name":"clientes","table_type":"BASE TABLE","row_count":"5000","column_name":"id","ordinal_position":"1","is_nullable":"NO","data_type":"INT64","column_description":null,"is_key":"true","ref_table":null},
  {"table_name":"clientes","table_type":"BASE TABLE","row_count":"5000","column_name":"cod_pais","ordinal_position":"2","is_nullable":"NO","data_type":"STRING","column_description":null,"is_key":"true","ref_table":null},
  {"table_name":"pedidos","table_type":"BASE TABLE","row_count":"50000","column_name":"id_pedido","ordinal_position":"1","is_nullable":"NO","data_type":"INT64","column_description":null,"is_key":"true","ref_table":null},
  {"table_name":"pedidos","table_type":"BASE TABLE","row_count":"50000","column_name":"fecha","ordinal_position":"5","is_nullable":"YES","data_type":"STRING","column_description":"Formato DDMMAAAA. NO tocar, viene del AS/400.","is_key":"false","ref_table":null},
  {"table_name":"pedidos","table_type":"BASE TABLE","row_count":"50000","column_name":"creado_en","ordinal_position":"6","is_nullable":"NO","data_type":"TIMESTAMP","column_description":null,"is_key":"false","ref_table":null},
  {"table_name":"pedidos_anidados","table_type":"BASE TABLE","row_count":"0","column_name":"cliente","ordinal_position":"2","is_nullable":"YES","data_type":"STRUCT<nom STRING, direccion STRUCT<calle STRING>>","column_description":null,"is_key":"false","ref_table":null},
  {"table_name":"pedidos_anidados","table_type":"BASE TABLE","row_count":"0","column_name":"etiquetas","ordinal_position":"4","is_nullable":"NO","data_type":"ARRAY<STRING>","column_description":null,"is_key":"false","ref_table":null},
  {"table_name":"v_pedidos_2019","table_type":"VIEW","row_count":"0","column_name":"id_pedido","ordinal_position":"1","is_nullable":"YES","data_type":"INT64","column_description":null,"is_key":"false","ref_table":null}
]"#;

    fn catalogo() -> String {
        let n = parse::parse(FILAS).unwrap();
        armar("bq_ventas", "rubix_demo_ventas", n.items()).pretty()
    }

    /// Lo que sale tiene que entrar: es la misma costura, por el otro lado.
    #[test]
    fn lo_que_produce_el_lector_lo_lee_el_inductor() {
        let c = catalogo();
        let cat = crate::inductor::Catalogo::leer(&c)
            .unwrap_or_else(|e| panic!("el inductor no lee lo que el lector escribe: {e}\n{c}"));
        let i = crate::inductor::inducir(&cat, "ventas");
        assert!(
            i.ficheros.contains_key("entities/Clientes.yaml"),
            "{:?}",
            i.ficheros.keys().collect::<Vec<_>>()
        );
    }

    /// El fan-out que salió midiendo: `clientes` tiene clave de dos columnas y el
    /// join con `CONSTRAINT_COLUMN_USAGE` la devolvía cuatro veces.
    #[test]
    fn una_clave_compuesta_no_multiplica_columnas() {
        let c = catalogo();
        assert_eq!(c.matches("\"name\": \"cod_pais\"").count(), 1, "{c}");
        assert_eq!(c.matches("\"cod_pais\"").count(), 2, "{c}");
    }

    /// `TIMESTAMP` es un instante; `DATETIME` es civil. Los dos sistemas de tipos
    /// hacen la distinción, así que perderla sería una traducción peor.
    #[test]
    fn timestamp_no_es_datetime() {
        assert_eq!(traducir("TIMESTAMP").as_deref(), Some("DateTimeTz"));
        assert_eq!(traducir("DATETIME").as_deref(), Some("DateTime"));
        assert_eq!(traducir("NUMERIC(10, 2)").as_deref(), Some("Decimal"));
        assert_eq!(traducir("ARRAY<STRING>").as_deref(), Some("list<String>"));
    }

    /// El caso que decide la doctrina: un `STRUCT` **no** es `Opaque`. `Opaque`
    /// dice «no hay estructura que modelar» y el origen acaba de enumerarla.
    #[test]
    fn un_struct_no_se_disfraza_de_opaque() {
        assert_eq!(traducir("STRUCT<nom STRING>"), None);
        assert_eq!(traducir("ARRAY<STRUCT<sku STRING>>"), None);
        assert_eq!(traducir("BYTES").as_deref(), Some("Opaque"));
        let c = catalogo();
        assert!(c.contains("\"sourceType\": \"STRUCT<"), "{c}");
        assert!(
            !c.contains("\"Opaque\""),
            "tradujo un STRUCT a Opaque:\n{c}"
        );
    }

    /// La descripción es un hecho del origen, escrito por quien conoce el dato.
    /// Perderla es perder lo mejor que trae un catálogo.
    #[test]
    fn la_descripcion_del_origen_sobrevive() {
        assert!(
            catalogo().contains("Formato DDMMAAAA. NO tocar, viene del AS/400."),
            "se perdió la descripción"
        );
    }

    #[test]
    fn una_vista_se_declara_vista() {
        let c = catalogo();
        assert!(c.contains("\"kind\": \"view\""), "{c}");
        assert!(c.contains("\"kind\": \"table\""), "{c}");
    }

    #[test]
    fn la_url_se_parte_en_proyecto_y_dataset() {
        assert_eq!(
            destino("bigquery://trino-k8s/rubix_demo_ventas"),
            Some(("trino-k8s".to_string(), "rubix_demo_ventas".to_string()))
        );
        assert_eq!(destino("bigquery://trino-k8s"), None);
    }
}
