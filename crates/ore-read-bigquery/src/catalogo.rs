//! **El catálogo de un dataset de BigQuery**: el verbo `catalogo` (A3).
//!
//! Vivía dentro de `ore` (`lector.rs`) y ejecutaba `bq`, porque `ore` no puede
//! hablar con la red —el guardián de dependencias lo veta— y delegar en un CLI
//! era la forma de no hacerlo. Desde que este driver habla REST (A2) esa receta
//! era la única pieza que seguía pagando el CLI: 11-14 s por catálogo y
//! `--max_rows=100000`, que cortaba sin decirlo. Ahora es el verbo de este
//! programa, como el de `ore-read-postgres`, y `ore` la despacha igual que a
//! cualquier otra familia: por `externo`, la URL por stdin.
//!
//! # Una consulta, no N
//!
//! La forma ingenua —listar tablas y describir cada una— es la equivocada: con
//! `bq show` eran ~10 s por tabla. Por REST, `tables.get` son 0,3 s, y 369
//! tablas siguen siendo dos minutos. La receta hace **una** consulta a
//! `INFORMATION_SCHEMA` que trae columnas, tipos, nulabilidad, claves,
//! descripciones y filas de todo el dataset: 1,7-4,4 s medidos, 80 MB
//! facturados (el mínimo de 10 MB por vista). Y **pagina**: 369 tablas son
//! 6,3 MB en una sola respuesta, y el techo de una respuesta está cerca.
use crate::rest;
use ore_core::json::Json;
use ore_driver::catalogo::{Catalogo, Columna, Foranea, Tabla, escribir};
use std::collections::BTreeMap;

/// Una fila de la consulta: columna → valor, **sin los nulos** (una columna
/// ausente es un nulo, como en todo el árbol).
pub type Fila = BTreeMap<String, String>;

/// `bigquery://<proyecto>/<dataset>`.
pub fn destino(url: &str) -> Option<(String, String)> {
    let resto = url.trim().split_once("://")?.1;
    let mut p = resto.trim_end_matches('/').splitn(2, '/');
    let proyecto = p.next()?.trim();
    let dataset = p.next()?.trim();
    (!proyecto.is_empty() && !dataset.is_empty())
        .then(|| (proyecto.to_string(), dataset.to_string()))
}

/// El verbo: la URL llega por stdin (la misma forma que `ore-read-postgres`) y
/// el nombre de la fuente como argumento.
pub fn leer(t: &dyn rest::Transporte, fuente: &str, url: &str) -> Result<String, String> {
    let (proyecto, dataset) = destino(url).ok_or_else(|| {
        format!(
            "`{}` no nombra un dataset: `bigquery://<proyecto>/<dataset>`. Una URL sin dataset \
             SÍ se explora y no se descubre: el catálogo de BigQuery es por dataset. `ore source \
             explore <fuente>` lista los que hay, con la orden hecha",
            url.trim()
        )
    })?;
    let mut filas: Vec<Fila> = Vec::new();
    rest::consultar(
        t,
        &proyecto,
        &rest::Consulta {
            texto: &consulta(&dataset),
            parametros: &[],
            sin_job: false,
        },
        |campos, pagina| {
            for f in pagina {
                let celdas = f["f"].as_array().map(Vec::as_slice).unwrap_or(&[]);
                filas.push(
                    campos
                        .iter()
                        .zip(celdas)
                        .filter_map(|(c, v)| {
                            Some((
                                c["name"].as_str()?.to_string(),
                                v["v"].as_str()?.to_string(),
                            ))
                        })
                        .collect(),
                );
            }
            Ok(())
        },
    )?;
    Ok(armar(fuente, &dataset, &filas))
}

/// Una consulta, todo el dataset.
///
/// `ANY_VALUE` sobre la tabla referenciada no es una elección: en una clave
/// compuesta todas las filas nombran la **misma** tabla, y sin él el producto
/// cartesiano multiplicaría las columnas. Salió midiendo: una clave de dos
/// columnas devolvía cuatro filas.
pub fn consulta(d: &str) -> String {
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
         ), n AS (SELECT table_id, row_count FROM `{d}.__TABLES__`\n\
         ), o AS (\n\
         \x20 SELECT table_name,\n\
         \x20        MAX(IF(option_name = 'require_partition_filter', option_value, NULL)) AS exige_filtro,\n\
         \x20        MAX(IF(option_name = 'enable_change_history', option_value, NULL)) AS historial\n\
         \x20 FROM `{d}`.INFORMATION_SCHEMA.TABLE_OPTIONS GROUP BY table_name\n\
         )\n\
         SELECT c.table_name, t.table_type, n.row_count, c.column_name, c.ordinal_position,\n\
         \x20      c.is_nullable, c.data_type, fp.description AS column_description,\n\
         \x20      kc.column_name IS NOT NULL AS is_key, fk.ref_table,\n\
         \x20      c.is_partitioning_column, o.exige_filtro, o.historial\n\
         FROM `{d}`.INFORMATION_SCHEMA.COLUMNS c\n\
         JOIN `{d}`.INFORMATION_SCHEMA.TABLES t ON t.table_name = c.table_name\n\
         LEFT JOIN n ON n.table_id = c.table_name\n\
         LEFT JOIN `{d}`.INFORMATION_SCHEMA.COLUMN_FIELD_PATHS fp\n\
         \x20 ON fp.table_name = c.table_name AND fp.field_path = c.column_name\n\
         LEFT JOIN kc ON kc.table_name = c.table_name AND kc.column_name = c.column_name\n\
         LEFT JOIN fk ON fk.table_name = c.table_name AND fk.column_name = c.column_name\n\
         LEFT JOIN o ON o.table_name = c.table_name\n\
         ORDER BY c.table_name, c.ordinal_position"
    )
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
/// Un decimal lleva su precisión ([`decimal`]); dentro de una lista no, porque
/// `list<T>` es de escalares (02-entity §3.3).
fn traducir(bq: &str) -> Option<String> {
    let t = bq.trim();
    if let Some(dentro) = t.strip_prefix("ARRAY<").and_then(|r| r.strip_suffix('>')) {
        return tipo_oos(dentro).map(|e| format!("list<{e}>"));
    }
    decimal(t).or_else(|| tipo_oos(t).map(String::from))
}

/// **Un decimal de BigQuery con su precisión** (02-entity §3.2, 0032 T4).
///
/// | origen | OOS | por qué |
/// |---|---|---|
/// | `NUMERIC` | `Decimal<38, 9>` | la precisión implícita de BigQuery, dicha |
/// | `NUMERIC(P)`, `NUMERIC(P, S)` | `Decimal<P, S>` (S = 0 si falta) | siempre cabe: `P ≤ S + 29 ≤ 38` |
/// | `BIGNUMERIC(P, S)` con `P ≤ 38` | `Decimal<P, S>` | cabe en `decimal128` |
/// | `BIGNUMERIC`, o `P > 38` | `String` | 76 cifras no caben en Iceberg (techo 38): el valor exacto viaja como texto, y la cita dice qué era |
///
/// Antes todos eran `Decimal` a secas, y la copia usaba `(38, 18)`: un
/// NUMERIC de más de 20 cifras enteras no cabía y la columna entera se quedaba
/// como texto (medido el 2026-09-26).
fn decimal(bq: &str) -> Option<String> {
    let (base, args) = match bq.split_once('(') {
        Some((b, r)) => (b.trim(), Some(r.strip_suffix(')')?)),
        None => (bq.trim(), None),
    };
    let grande = match base {
        "NUMERIC" | "DECIMAL" => false,
        "BIGNUMERIC" | "BIGDECIMAL" => true,
        _ => return None,
    };
    let (p, s) = match args {
        None if grande => return Some("String".into()),
        None => (38, 9),
        Some(a) => {
            let n: Vec<u16> = a
                .split(',')
                .map(|x| x.trim().parse().ok())
                .collect::<Option<_>>()?;
            match n[..] {
                [p] => (p, 0),
                [p, s] => (p, s),
                _ => return None,
            }
        }
    };
    Some(if p <= 38 && s <= p {
        format!("Decimal<{p}, {s}>")
    } else {
        "String".into()
    })
}

fn clase(table_type: &str) -> &'static str {
    match table_type {
        "VIEW" => "view",
        "MATERIALIZED VIEW" => "materializedView",
        _ => "table",
    }
}

// ── Las dos caras ───────────────────────────────────────────────────────────

/// **La cara `I`**: qué se le puede pedir a este objeto.
///
/// # Lo que se EMPUJA, no lo que el motor sabe
///
/// BigQuery contesta los seis operadores de GoogleSQL, pero entre el
/// planificador y el dataset está este driver, y una petición solo sabe
/// expresar los de `ore_driver::OPERADORES`. Un filtro que el driver no sabe
/// poner **se cae de la petición**, y la consulta devuelve más filas de las que
/// pidió sin que nadie vea un error.
///
/// Y `OPERADORES` **no** es esta lista aunque se parezca: es lo que una
/// petición lleva (incluye `gt` por la marca de agua), y `predicatePushdown` es
/// lo que un plan empuja, con el vocabulario del esquema publicado —`gt` no
/// está: el orden se declara con `range`—. La intersección honesta es **`eq`**.
///
/// Lo que sí se sondea es lo que cambia de tabla a tabla:
///
/// | lo que dice el servidor | lo que emite | por qué |
/// |---|---|---|
/// | `require_partition_filter = true` | `fullScan: forbidden` y `requiredFilters` | BigQuery **rechaza** la consulta sin filtro de partición |
/// | cualquier otro objeto legible | `fullScan: expensive` | se factura por bytes leídos |
fn reads(particion: Option<&str>, exige_filtro: bool) -> Json {
    let operadores = ["eq"];
    let mut o: BTreeMap<String, Json> = BTreeMap::new();
    o.insert(
        "predicatePushdown".to_string(),
        Json::Arr(operadores.iter().map(|o| Json::s(*o)).collect()),
    );
    match (exige_filtro, particion) {
        (true, Some(c)) => {
            o.insert("fullScan".to_string(), Json::s("forbidden"));
            o.insert("requiredFilters".to_string(), Json::Arr(vec![Json::s(c)]));
        }
        _ => {
            o.insert("fullScan".to_string(), Json::s("expensive"));
        }
    }
    Json::Obj(o)
}

/// **La cara `D`**, sondeada: qué cambios emite este objeto de verdad.
///
/// | lo que dice el servidor | lo que emite | por qué |
/// |---|---|---|
/// | no es `BASE TABLE` | `{none, none}` | una vista no tiene flujo propio, y una materializada **se refresca**, que no es un changelog |
/// | `enable_change_history` sin poner o a `false` | `{none, none}` | sin historial no sale ningún cambio |
/// | `enable_change_history = true` | `{retract, log}` | el historial trae los borrados |
///
/// La tercera fila **no se ha medido contra un dataset real**: encender el
/// historial es modificar el dataset. Se dice, en vez de suponerse verificado.
fn changes(table_type: &str, historial: bool) -> Json {
    if table_type != "BASE TABLE" || !historial {
        return Json::obj([("mode", Json::s("none")), ("witness", Json::s("none"))]);
    }
    Json::obj([("mode", Json::s("retract")), ("witness", Json::s("log"))])
}

/// Agrupa las filas planas de la consulta en tablas. Llegan ordenadas por
/// `(table_name, ordinal_position)`, y ese orden se conserva: el orden de las
/// columnas es del origen y no nos toca reordenarlo.
pub fn armar(fuente: &str, dataset: &str, filas: &[Fila]) -> String {
    struct Acc {
        clase: &'static str,
        filas: Option<u64>,
        columnas: Vec<Columna>,
        clave: Vec<String>,
        foraneas: BTreeMap<String, Vec<String>>,
        tipo: String,
        particion: Option<String>,
        exige_filtro: bool,
        historial: bool,
    }
    let campo = |f: &Fila, k: &str| f.get(k).cloned();

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
                tipo: campo(f, "table_type").unwrap_or_default(),
                particion: None,
                // `true` y no `TRUE`: `INFORMATION_SCHEMA.TABLE_OPTIONS`
                // devuelve el literal de GoogleSQL en minúsculas.
                exige_filtro: campo(f, "exige_filtro").as_deref() == Some("true"),
                historial: campo(f, "historial").as_deref() == Some("true"),
            }
        });

        // El tipo **y** su cita, como `ore-read-postgres` desde 0032 T2: la
        // traducción es lo que el árbol entiende; la cita es el hecho, y dice
        // qué era una columna que viaja como texto (un BIGNUMERIC) o que no se
        // supo traducir (un STRUCT). Antes era una cosa o la otra.
        let tipo = traducir(&tipo_bruto);
        let origen = Some(tipo_bruto.clone());
        acc.columnas.push(Columna {
            nombre: columna.clone(),
            tipo,
            origen,
            obligatoria: campo(f, "is_nullable").as_deref() == Some("NO"),
            // GoogleSQL no tiene tipos enumerados: la ausencia es la respuesta.
            valores: Vec::new(),
            descripcion: campo(f, "column_description").filter(|d| !d.trim().is_empty()),
        });

        if campo(f, "is_partitioning_column").as_deref() == Some("YES") {
            acc.particion = Some(columna.clone());
        }
        if campo(f, "is_key").as_deref() == Some("true") {
            acc.clave.push(columna.clone());
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
            Some(Tabla {
                nombre: format!("{dataset}.{t}"),
                columnas: a.columnas,
                clave: a.clave,
                // BigQuery no publica claves alternativas: la ausencia es una
                // respuesta, y el emisor no escribe lo que no se dijo.
                unicas: Vec::new(),
                foraneas: a
                    .foraneas
                    .into_iter()
                    .map(|(destino, columnas)| Foranea {
                        columnas,
                        destino,
                        destino_columnas: Vec::new(),
                    })
                    .collect(),
                filas: a.filas,
                clase: a.clase.to_string(),
                // Las dos caras, del objeto y no de quien lo consulta.
                lee: Some(reads(a.particion.as_deref(), a.exige_filtro)),
                cambia: Some(changes(&a.tipo, a.historial)),
            })
        })
        .collect();

    // **El emisor es el del protocolo**: un catálogo de este driver y uno de
    // `ore-read-postgres` son el mismo texto para la misma forma.
    escribir(&Catalogo {
        fuente: fuente.to_string(),
        tablas,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rest::pruebas::{Guion, grabada};

    /// Las filas exactas que devolvió `bq` sobre `rubix_demo_ventas`, recortadas.
    /// Es una captura, no un invento: la clave compuesta de `clientes` y el
    /// `STRUCT` de `pedidos_anidados` son los dos casos que costaron sangre.
    /// (Mudada de `ore-cli/src/lector.rs` en A3, con sus pruebas.)
    const FILAS: &str = r#"[
  {"table_name":"clientes","table_type":"BASE TABLE","row_count":"5000","column_name":"id","ordinal_position":"1","is_nullable":"NO","data_type":"INT64","column_description":null,"is_key":"true","ref_table":null,"is_partitioning_column":"NO","exige_filtro":null,"historial":null},
  {"table_name":"clientes","table_type":"BASE TABLE","row_count":"5000","column_name":"cod_pais","ordinal_position":"2","is_nullable":"NO","data_type":"STRING","column_description":null,"is_key":"true","ref_table":null},
  {"table_name":"pedidos","table_type":"BASE TABLE","row_count":"50000","column_name":"id_pedido","ordinal_position":"1","is_nullable":"NO","data_type":"INT64","column_description":null,"is_key":"true","ref_table":null},
  {"table_name":"pedidos","table_type":"BASE TABLE","row_count":"50000","column_name":"fecha","ordinal_position":"5","is_nullable":"YES","data_type":"STRING","column_description":"Formato DDMMAAAA. NO tocar, viene del AS/400.","is_key":"false","ref_table":null},
  {"table_name":"pedidos","table_type":"BASE TABLE","row_count":"50000","column_name":"creado_en","ordinal_position":"6","is_nullable":"NO","data_type":"TIMESTAMP","column_description":null,"is_key":"false","ref_table":null,"is_partitioning_column":"YES","exige_filtro":"false","historial":null},
  {"table_name":"pedidos_anidados","table_type":"BASE TABLE","row_count":"0","column_name":"cliente","ordinal_position":"2","is_nullable":"YES","data_type":"STRUCT<nom STRING, direccion STRUCT<calle STRING>>","column_description":null,"is_key":"false","ref_table":null},
  {"table_name":"pedidos_anidados","table_type":"BASE TABLE","row_count":"0","column_name":"etiquetas","ordinal_position":"4","is_nullable":"NO","data_type":"ARRAY<STRING>","column_description":null,"is_key":"false","ref_table":null},
  {"table_name":"v_pedidos_2019","table_type":"VIEW","row_count":"0","column_name":"id_pedido","ordinal_position":"1","is_nullable":"YES","data_type":"INT64","column_description":null,"is_key":"false","ref_table":null,"is_partitioning_column":"NO","exige_filtro":null,"historial":null}
]"#;

    fn catalogo() -> String {
        let v: Vec<BTreeMap<String, serde_json::Value>> = serde_json::from_str(FILAS).unwrap();
        let filas: Vec<Fila> = v
            .into_iter()
            .map(|f| {
                f.into_iter()
                    .filter_map(|(k, v)| Some((k, v.as_str()?.to_string())))
                    .collect()
            })
            .collect();
        armar("bq_ventas", "rubix_demo_ventas", &filas)
    }

    /// Las dos caras de cada objeto, como hechos que el servidor afirma. Antes
    /// se comprobaba a través del inductor de `ore-cli`; su lector es este
    /// mismo (`ore_driver::catalogo::Catalogo`), y lo que se afirma es el JSON.
    #[test]
    fn la_receta_emite_las_dos_caras_de_cada_objeto() {
        let c = catalogo();
        let cat = Catalogo::leer(&c).expect("el protocolo lo lee");
        for t in &cat.tablas {
            let lee = t.lee.as_ref().expect("sin reads").pretty();
            assert!(lee.contains("\"predicatePushdown\""), "{lee}");
            assert!(lee.contains("\"eq\""), "{lee}");
            assert!(t.cambia.is_some(), "`{}` sin changes", t.nombre);
        }
        let pedidos = cat
            .tablas
            .iter()
            .find(|t| t.nombre == "rubix_demo_ventas.pedidos")
            .unwrap();
        assert!(
            pedidos
                .lee
                .as_ref()
                .unwrap()
                .pretty()
                .contains("\"expensive\"")
        );
        let clientes = &cat.tablas[0];
        assert!(
            clientes
                .cambia
                .as_ref()
                .unwrap()
                .pretty()
                .contains("\"none\"")
        );
    }

    /// `require_partition_filter` no es un consejo: BigQuery rechaza la
    /// consulta. Rama **no medida contra un dataset real**; se prueba aquí.
    #[test]
    fn una_tabla_que_exige_filtro_de_particion_prohibe_el_recorrido() {
        let r = reads(Some("creado_en"), true).pretty();
        assert!(r.contains("\"forbidden\""), "{r}");
        assert!(r.contains("requiredFilters"), "{r}");
        assert!(r.contains("creado_en"), "{r}");
        let s = reads(Some("creado_en"), false).pretty();
        assert!(
            s.contains("\"expensive\"") && !s.contains("requiredFilters"),
            "{s}"
        );
    }

    #[test]
    fn los_cambios_salen_de_lo_que_el_servidor_afirma() {
        let modo = |t: &str, h: bool| {
            if changes(t, h).pretty().contains("retract") {
                "retract"
            } else {
                "none"
            }
        };
        assert_eq!(modo("VIEW", true), "none");
        assert_eq!(modo("MATERIALIZED VIEW", true), "none");
        assert_eq!(modo("BASE TABLE", false), "none");
        assert_eq!(modo("BASE TABLE", true), "retract");
        assert!(changes("BASE TABLE", true).pretty().contains("log"));
    }

    /// Lo que sale tiene que entrar: el lector del protocolo, que es el que usa
    /// el inductor, lo lee entero.
    #[test]
    fn lo_que_produce_el_lector_lo_lee_el_protocolo() {
        let c = catalogo();
        let cat = Catalogo::leer(&c).unwrap_or_else(|e| panic!("no se relee: {e}\n{c}"));
        assert_eq!(cat.tablas.len(), 4);
        assert_eq!(cat.tablas[0].clave, vec!["id", "cod_pais"]);
    }

    /// El fan-out que salió midiendo: `clientes` tiene clave de dos columnas y el
    /// join con `CONSTRAINT_COLUMN_USAGE` la devolvía cuatro veces.
    #[test]
    fn una_clave_compuesta_no_multiplica_columnas() {
        let c = catalogo();
        assert_eq!(c.matches("\"name\": \"cod_pais\"").count(), 1, "{c}");
        assert_eq!(c.matches("\"cod_pais\"").count(), 2, "{c}");
    }

    #[test]
    fn timestamp_no_es_datetime() {
        assert_eq!(traducir("TIMESTAMP").as_deref(), Some("DateTimeTz"));
        assert_eq!(traducir("DATETIME").as_deref(), Some("DateTime"));
        assert_eq!(
            traducir("NUMERIC(10, 2)").as_deref(),
            Some("Decimal<10, 2>")
        );
        assert_eq!(traducir("ARRAY<STRING>").as_deref(), Some("list<String>"));
    }

    /// Los decimales con su precisión (0032 T4), fila a fila de [`decimal`].
    #[test]
    fn cada_decimal_con_su_precision() {
        for (bq, oos) in [
            ("NUMERIC", "Decimal<38, 9>"),
            ("NUMERIC(10, 2)", "Decimal<10, 2>"),
            ("NUMERIC(10)", "Decimal<10, 0>"),
            ("BIGNUMERIC(38, 10)", "Decimal<38, 10>"),
            ("BIGNUMERIC(40, 2)", "String"),
            ("BIGNUMERIC", "String"),
            ("DECIMAL", "Decimal<38, 9>"),
        ] {
            assert_eq!(traducir(bq).as_deref(), Some(oos), "{bq}");
        }
        assert_eq!(traducir("ARRAY<NUMERIC>").as_deref(), Some("list<Decimal>"));
        // Y la cita va siempre, con la traducción al lado.
        let c = catalogo();
        assert!(c.contains("\"sourceType\": \"INT64\""), "{c}");
        assert!(c.contains("\"type\": \"Integer\""), "{c}");
    }

    /// El caso que decide la doctrina: un `STRUCT` **no** es `Opaque`.
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

    #[test]
    fn la_descripcion_del_origen_sobrevive() {
        assert!(catalogo().contains("Formato DDMMAAAA. NO tocar, viene del AS/400."));
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
            destino("bigquery://trino-k8s/rubix_demo_ventas\n"),
            Some(("trino-k8s".to_string(), "rubix_demo_ventas".to_string()))
        );
        assert_eq!(destino("bigquery://trino-k8s"), None);
    }

    /// La respuesta REST grabada en A0 (la receta sobre `ventas`) da el
    /// catálogo de las dos tablas, con `id` obligatorio.
    #[test]
    fn la_respuesta_grabada_de_ventas_da_su_catalogo() {
        let g = Guion::new(vec![Ok(grabada("catalogo"))]);
        let c = leer(&g, "bq", "bigquery://p/ventas").unwrap();
        let cat = Catalogo::leer(&c).unwrap();
        let nombres: Vec<&str> = cat.tablas.iter().map(|t| t.nombre.as_str()).collect();
        assert_eq!(nombres, ["ventas.clientes", "ventas.pedidos"]);
        assert!(
            cat.tablas[1].columnas[0].obligatoria,
            "pedidos.id es REQUIRED"
        );
    }

    /// Y la copia de la receta que usa el grabador no puede envejecer en
    /// silencio: es literalmente esta consulta.
    #[test]
    fn la_receta_del_grabador_es_esta() {
        let ruta = format!(
            "{}/../../pruebas-de-fuego/grabar-bigquery-catalogo.sql",
            env!("CARGO_MANIFEST_DIR")
        );
        let grabador = std::fs::read_to_string(&ruta).unwrap();
        let sql: String = grabador
            .lines()
            .filter(|l| !l.starts_with("--"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(sql.trim(), consulta("{d}").trim());
    }

    /// Un catálogo de más de una página se lee entero: la guarda de `totalRows`
    /// es la de las filas, y aquí se ve que cruza páginas.
    #[test]
    fn un_catalogo_de_varias_paginas_se_lee_entero() {
        let mut p1 = grabada("catalogo");
        let mut p2 = p1.clone();
        let filas = p1["rows"].as_array().unwrap().clone();
        p1["rows"] = serde_json::Value::Array(filas[..3].to_vec());
        p1["pageToken"] = "t".into();
        p2["rows"] = serde_json::Value::Array(filas[3..].to_vec());
        let g = Guion::new(vec![Ok(p1), Ok(p2)]);
        let c = leer(&g, "bq", "bigquery://p/ventas").unwrap();
        assert_eq!(Catalogo::leer(&c).unwrap().tablas.len(), 2);
    }
}
