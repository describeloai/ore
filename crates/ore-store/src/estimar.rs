//! **`estimar`** (Query Q2, `docs/query.md` §3.4): lo que una consulta va a leer de
//! una tabla del lago, ANTES de correrla, sin abrir un solo fichero de datos.
//!
//! Lee los manifiestos del snapshot vigente y, por cada fichero de datos vivo,
//! decide con sus mínimos y máximos si los filtros lo dejan fuera; de los que
//! quedan, suma las filas y los bytes de **las columnas que la consulta usa**
//! (`column_sizes` de cada fichero, por el id de la columna y de sus hijos). Es
//! la cifra que en Q0 se cruzó con el tiempo real (`gb_estimados`), y la que la
//! admisión usa para elegir el motor.
//!
//! ```text
//! {"metadata_location": "...", "dataset": "...",
//!  "columnas": ["a", "b"] | ["*"] | [],
//!  "filtros": [{"columna": "a", "operador": "ge", "valor": "10"}, ...]}
//! → {"filas_total", "filas", "ficheros_total", "ficheros", "bytes_total",
//!    "bytes", "columnas", "sin_tamano", "filtros_usados", "filtros_ignorados", "snapshot"}
//! ```
//!
//! ⭐ Conservador siempre: lo que no se sabe comparar (un tipo sin conversión, un
//! `like`, un fichero sin mínimos) **no poda**. Una estimación por encima elige un
//! motor más grande; una por debajo, uno que no aguanta.

use crate::lago::{self, Lago};
use iceberg::spec::{DataContentType, Datum, NestedFieldRef, PrimitiveType, Schema, Type};
use ore_core::json::Json;
use ore_core::parse::Node;
use std::collections::BTreeSet;

/// Un filtro de la petición, ya con su valor del tipo de la columna.
struct Filtro {
    id: i32,
    operador: String,
    valores: Vec<Datum>,
}

/// `estimar`, la petición en JSON.
pub fn estimar(lago: &Lago, n: &Node) -> Result<String, String> {
    let campo = |k: &str| n.get(k).and_then(|(_, v)| v.as_str()).map(String::from);
    let ml = campo("metadata_location")
        .ok_or("a `estimar` le falta `metadata_location`: el puntero del dataset")?;
    let dataset = campo("dataset").unwrap_or_else(|| "dataset".into());
    let columnas: Vec<String> = n
        .get("columnas")
        .map(|(_, v)| {
            v.items()
                .iter()
                .filter_map(|c| c.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_else(|| vec!["*".into()]);
    let tabla = lago.abrir(&ml, &dataset)?;
    let meta = tabla.metadata();
    let esquema = meta.current_schema().clone();

    // Las columnas: sus ids y los de sus hijos (un struct, una lista, un mapa
    // guardan sus bytes en las hojas). `*`, todas; ninguna, sólo metadatos.
    let todas = columnas.iter().any(|c| c == "*");
    let mut suyas: Vec<(String, BTreeSet<i32>)> = Vec::new();
    let mut desconocidas = Vec::new();
    if !todas {
        for c in &columnas {
            match esquema.field_by_name(c) {
                Some(f) => {
                    let mut ids = BTreeSet::new();
                    ids_de(f, &mut ids);
                    suyas.push((c.clone(), ids));
                }
                None => desconocidas.push(c.clone()),
            }
        }
    }
    let ids: BTreeSet<i32> = suyas.iter().flat_map(|(_, i)| i.iter().copied()).collect();

    // Los filtros que se saben comparar; los demás, ignorados (no podan).
    let (mut filtros, mut ignorados) = (Vec::new(), Vec::new());
    for f in n
        .get("filtros")
        .map(|(_, v)| v.items().to_vec())
        .unwrap_or_default()
    {
        let c = |k: &str| f.get(k).and_then(|(_, v)| v.as_str()).map(String::from);
        let (Some(col), Some(op)) = (c("columna"), c("operador")) else {
            continue;
        };
        let valores: Vec<String> = match f.get("valor").map(|(_, v)| v) {
            Some(v) if !v.items().is_empty() => v
                .items()
                .iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect(),
            Some(v) => v.as_str().map(|s| vec![s.to_string()]).unwrap_or_default(),
            None => Vec::new(),
        };
        match filtro_de(&esquema, &col, &op, &valores) {
            Some(f) => filtros.push(f),
            None => ignorados.push(format!("{col} {op}")),
        }
    }

    let Some(s) = meta.current_snapshot() else {
        return Ok(respuesta(
            &columnas,
            [0; 6],
            &desconocidas,
            &filtros,
            &ignorados,
            None,
        ));
    };
    let cuentas = lago::runtime().block_on(async {
        let io = tabla.file_io();
        let lista = tabla
            .manifest_list_reader(s)
            .load()
            .await
            .map_err(|e| format!("la lista de manifiestos de `{ml}`: {e}"))?;
        // [filas_total, filas, ficheros_total, ficheros, bytes_total, bytes]
        let mut c = [0u64; 6];
        let mut con_tamano: BTreeSet<i32> = BTreeSet::new();
        for mf in lista.entries() {
            let m = mf
                .load_manifest(io)
                .await
                .map_err(|e| format!("un manifiesto de `{ml}`: {e}"))?;
            for e in m.entries().iter().filter(|e| e.is_alive()) {
                let d = e.data_file();
                if d.content_type() != DataContentType::Data {
                    continue;
                }
                c[0] += d.record_count();
                c[2] += 1;
                c[4] += d.file_size_in_bytes();
                if !filtros.iter().all(|f| puede_tener(f, d)) {
                    continue;
                }
                c[1] += d.record_count();
                c[3] += 1;
                c[5] += if todas {
                    d.file_size_in_bytes()
                } else {
                    ids.iter()
                        .filter_map(|i| d.column_sizes().get(i))
                        .sum::<u64>()
                };
                con_tamano.extend(ids.iter().filter(|i| d.column_sizes().contains_key(i)));
            }
        }
        Ok::<_, String>((c, con_tamano))
    })?;
    let (cuentas, con_tamano) = cuentas;
    // Las que ningún fichero leído dice cuánto pesan (`column_sizes` sin sus
    // ids): la cifra de bytes se queda corta para ellas, y se dice.
    let sin_tamano: Vec<String> = suyas
        .iter()
        .filter(|(_, i)| cuentas[3] > 0 && i.is_disjoint(&con_tamano))
        .map(|(c, _)| c.clone())
        .collect();
    let mut no_sabe = desconocidas.clone();
    no_sabe.extend(sin_tamano);
    Ok(respuesta(
        &columnas,
        cuentas,
        &no_sabe,
        &filtros,
        &ignorados,
        Some(s.snapshot_id()),
    ))
}

fn respuesta(
    columnas: &[String],
    c: [u64; 6],
    sin_tamano: &[String],
    filtros: &[Filtro],
    ignorados: &[String],
    snapshot: Option<i64>,
) -> String {
    let n = |x: u64| Json::Int(x as i64);
    Json::obj([
        ("filas_total", n(c[0])),
        ("filas", n(c[1])),
        ("ficheros_total", n(c[2])),
        ("ficheros", n(c[3])),
        ("bytes_total", n(c[4])),
        ("bytes", n(c[5])),
        (
            "columnas",
            Json::Arr(columnas.iter().map(Json::s).collect()),
        ),
        (
            "sin_tamano",
            Json::Arr(sin_tamano.iter().map(Json::s).collect()),
        ),
        ("filtros_usados", Json::Int(filtros.len() as i64)),
        (
            "filtros_ignorados",
            Json::Arr(ignorados.iter().map(Json::s).collect()),
        ),
        (
            "snapshot",
            Json::s(snapshot.map(|s| s.to_string()).unwrap_or_default()),
        ),
    ])
    .jcs()
}

/// El id de una columna y los de todo lo que lleva dentro.
fn ids_de(f: &NestedFieldRef, ids: &mut BTreeSet<i32>) {
    ids.insert(f.id);
    match f.field_type.as_ref() {
        Type::Struct(s) => s.fields().iter().for_each(|h| ids_de(h, ids)),
        Type::List(l) => ids_de(&l.element_field, ids),
        Type::Map(m) => {
            ids_de(&m.key_field, ids);
            ids_de(&m.value_field, ids);
        }
        Type::Primitive(_) => {}
    }
}

/// El filtro con su valor convertido al tipo de la columna, o `None` si no se
/// sabe comparar (columna anidada o desconocida, tipo sin conversión, operador
/// que no poda).
fn filtro_de(esquema: &Schema, col: &str, op: &str, valores: &[String]) -> Option<Filtro> {
    let f = esquema.field_by_name(col)?;
    let Type::Primitive(t) = f.field_type.as_ref() else {
        return None;
    };
    match op {
        "isNull" | "isNotNull" => {
            return Some(Filtro {
                id: f.id,
                operador: op.into(),
                valores: Vec::new(),
            });
        }
        "eq" | "neq" | "lt" | "le" | "gt" | "ge" | "in" => {}
        _ => return None,
    }
    let valores: Option<Vec<Datum>> = valores.iter().map(|v| datum(t, v)).collect();
    let valores = valores?;
    if valores.is_empty() {
        return None;
    }
    Some(Filtro {
        id: f.id,
        operador: op.into(),
        valores,
    })
}

fn datum(t: &PrimitiveType, v: &str) -> Option<Datum> {
    let v = v.trim();
    Some(match t {
        PrimitiveType::Boolean => Datum::bool(v.parse::<bool>().ok()?),
        PrimitiveType::Int => Datum::int(v.parse::<i32>().ok()?),
        PrimitiveType::Long => Datum::long(v.parse::<i64>().ok()?),
        PrimitiveType::Float => Datum::float(v.parse::<f32>().ok()?),
        PrimitiveType::Double => Datum::double(v.parse::<f64>().ok()?),
        PrimitiveType::String => Datum::string(v),
        PrimitiveType::Date => Datum::date_from_str(v).ok()?,
        PrimitiveType::Timestamp => Datum::timestamp_from_str(v).ok()?,
        PrimitiveType::Timestamptz => Datum::timestamptz_from_str(v).ok()?,
        _ => return None,
    })
}

/// ¿Puede este fichero tener filas que pasen el filtro? Sin mínimos o máximos,
/// sí: no se poda lo que no se sabe.
fn puede_tener(f: &Filtro, d: &iceberg::spec::DataFile) -> bool {
    let nulos = d.null_value_counts().get(&f.id).copied();
    match f.operador.as_str() {
        "isNull" => return nulos != Some(0),
        "isNotNull" => return nulos != Some(d.record_count()),
        _ => {}
    }
    let (lo, hi) = (d.lower_bounds().get(&f.id), d.upper_bounds().get(&f.id));
    // Todo nulo: ninguna comparación es verdadera.
    if nulos == Some(d.record_count()) && d.record_count() > 0 {
        return false;
    }
    let menor_o_igual =
        |a: Option<&Datum>, b: &Datum| a.is_none_or(|a| a.partial_cmp(b).is_none_or(|o| o.is_le()));
    let mayor_o_igual =
        |a: Option<&Datum>, b: &Datum| a.is_none_or(|a| a.partial_cmp(b).is_none_or(|o| o.is_ge()));
    let v = &f.valores[0];
    match f.operador.as_str() {
        "eq" => menor_o_igual(lo, v) && mayor_o_igual(hi, v),
        "in" => f
            .valores
            .iter()
            .any(|v| menor_o_igual(lo, v) && mayor_o_igual(hi, v)),
        "neq" => !(lo == Some(v) && hi == Some(v)),
        "lt" => lo.is_none_or(|a| a.partial_cmp(v).is_none_or(|o| o.is_lt())),
        "le" => menor_o_igual(lo, v),
        "gt" => hi.is_none_or(|a| a.partial_cmp(v).is_none_or(|o| o.is_gt())),
        "ge" => mayor_o_igual(hi, v),
        _ => true,
    }
}
