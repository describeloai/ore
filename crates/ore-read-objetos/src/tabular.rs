//! **El esquema de un fichero que es filas**, leyendo lo mínimo.
//!
//! - **Parquet**: el pie, por su sufijo (`bytes=-8` da su longitud; otra
//!   lectura, el pie entero). El esquema es exacto —`decimal128(12, 2)`,
//!   `timestamp[us, tz=UTC]`— y las filas vienen dichas. Medido en F1.
//! - **CSV**: los primeros 64 KB. El tipo se **deduce**, y lo deducido es una
//!   propuesta que alguien confirma (la decisión `tipo/*` de siempre), porque
//!   una muestra miente en los códigos: medido, `customer_zip_code_prefix`
//!   sale entero y un código postal brasileño empieza por cero (`01037`). Un
//!   valor con ceros a la izquierda, o una columna que se llama como un código,
//!   se queda en `String`. Y el BOM se quita (medido:
//!   `\u{feff}product_category_name`).
//! - **JSONL**: los primeros 64 KB; la unión de las claves (medido: `usuario`
//!   aparece el segundo día), y el tipo como lo dice el JSON.

use ore_driver::catalogo::Columna;
use ore_objetos::Origen;

/// Lo que se lee de un CSV o un JSONL para deducir: bastante para una muestra,
/// poco para un fichero de 60 MB (medido en F1: 100–200 ms por fichero).
const MUESTRA: usize = 64 * 1024;

pub struct Esquema {
    pub columnas: Vec<Columna>,
    pub filas: Option<u64>,
    /// Solo CSV: el separador, si no es la coma.
    pub separador: Option<char>,
    pub avisos: Vec<String>,
}

// ── Parquet ─────────────────────────────────────────────────────────────────

pub fn parquet(o: &dyn Origen, clave: &str) -> Result<Esquema, String> {
    let cola = o.rango(clave, "-8")?;
    if cola.len() < 8 || &cola[4..8] != b"PAR1" {
        return Err(format!("`{clave}` no termina como un Parquet (`PAR1`)"));
    }
    let n = u32::from_le_bytes([cola[0], cola[1], cola[2], cola[3]]) as usize;
    let pie = o.rango(clave, &format!("-{}", n + 8))?;
    if pie.len() < n + 8 {
        return Err(format!("el pie de `{clave}` llegó corto"));
    }
    let meta = parquet::file::metadata::ParquetMetaDataReader::decode_metadata(&pie[..n])
        .map_err(|e| format!("el pie de `{clave}` no se entiende: {e}"))?;
    let fm = meta.file_metadata();
    let esquema =
        parquet::arrow::parquet_to_arrow_schema(fm.schema_descr(), fm.key_value_metadata())
            .map_err(|e| format!("el esquema de `{clave}` no se traduce: {e}"))?;
    let columnas = esquema
        .fields()
        .iter()
        .map(|f| {
            let (tipo, fisico) = de_arrow(f.data_type());
            Columna {
                nombre: f.name().clone(),
                tipo,
                origen: Some(fisico),
                obligatoria: !f.is_nullable(),
                ..Default::default()
            }
        })
        .collect();
    Ok(Esquema {
        columnas,
        filas: u64::try_from(fm.num_rows()).ok(),
        separador: None,
        avisos: Vec::new(),
    })
}

/// Del tipo de Arrow al de OOS, con el físico escrito como lo escribe
/// `pyarrow` (el vocabulario de `physicalType` del contrato de tipos, 0032).
/// Lo que no tiene traducción se cita y no se traduce.
pub fn de_arrow(t: &arrow_schema::DataType) -> (Option<String>, String) {
    use arrow_schema::{DataType as D, TimeUnit};
    let unidad = |u: &TimeUnit| match u {
        TimeUnit::Second => "s",
        TimeUnit::Millisecond => "ms",
        TimeUnit::Microsecond => "us",
        TimeUnit::Nanosecond => "ns",
    };
    let (tipo, fisico): (Option<&str>, String) = match t {
        D::Utf8 => (Some("String"), "string".into()),
        D::LargeUtf8 => (Some("String"), "large_string".into()),
        D::Utf8View => (Some("String"), "string_view".into()),
        D::Boolean => (Some("Boolean"), "bool".into()),
        D::Int8 => (Some("Integer"), "int8".into()),
        D::Int16 => (Some("Integer"), "int16".into()),
        D::Int32 => (Some("Integer"), "int32".into()),
        D::Int64 => (Some("Integer"), "int64".into()),
        D::UInt8 => (Some("Integer"), "uint8".into()),
        D::UInt16 => (Some("Integer"), "uint16".into()),
        D::UInt32 => (Some("Integer"), "uint32".into()),
        D::UInt64 => (Some("Integer"), "uint64".into()),
        D::Float32 => (Some("Float"), "float".into()),
        D::Float64 => (Some("Float"), "double".into()),
        D::Date32 => (Some("Date"), "date32[day]".into()),
        D::Date64 => (Some("Date"), "date64[ms]".into()),
        D::Timestamp(u, Some(tz)) => (
            Some("DateTimeTz"),
            format!("timestamp[{}, tz={tz}]", unidad(u)),
        ),
        D::Timestamp(u, None) => (Some("DateTime"), format!("timestamp[{}]", unidad(u))),
        D::Decimal128(p, s) => {
            return (
                Some(format!("Decimal<{p}, {s}>")),
                format!("decimal128({p}, {s})"),
            );
        }
        otro => (None, format!("{otro}").to_ascii_lowercase()),
    };
    (tipo.map(String::from), fisico)
}

// ── La deducción, compartida por CSV y JSONL ────────────────────────────────

/// Una clase de valor. `Cero` es un entero escrito con ceros a la izquierda:
/// como entero pierde los ceros, así que es texto.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Valor {
    Booleano,
    Entero,
    Cero,
    Decimal,
    Fecha,
    FechaHora,
    FechaHoraTz,
    Texto,
    Json,
}

fn digitos(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

fn clase_de_texto(v: &str) -> Option<Valor> {
    let v = v.trim();
    if v.is_empty() {
        return None;
    }
    let sin_signo = v.strip_prefix('-').unwrap_or(v);
    if digitos(sin_signo) {
        return Some(if sin_signo.len() > 1 && sin_signo.starts_with('0') {
            Valor::Cero
        } else {
            Valor::Entero
        });
    }
    if let Some((e, d)) = sin_signo.split_once('.')
        && digitos(e)
        && digitos(d)
    {
        return Some(Valor::Decimal);
    }
    if v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("false") {
        return Some(Valor::Booleano);
    }
    let b = v.as_bytes();
    let es_fecha = |x: &[u8]| {
        x.len() >= 10
            && x[..4].iter().all(u8::is_ascii_digit)
            && x[4] == b'-'
            && x[5..7].iter().all(u8::is_ascii_digit)
            && x[7] == b'-'
            && x[8..10].iter().all(u8::is_ascii_digit)
    };
    if es_fecha(b) {
        if b.len() == 10 {
            return Some(Valor::Fecha);
        }
        if b.len() >= 19 && (b[10] == b' ' || b[10] == b'T') && b[13] == b':' && b[16] == b':' {
            let cola = &v[19..];
            let tz = cola.ends_with('Z')
                || cola
                    .rfind(['+', '-'])
                    .is_some_and(|i| cola[i..].contains(':'));
            return Some(if tz {
                Valor::FechaHoraTz
            } else {
                Valor::FechaHora
            });
        }
    }
    Some(Valor::Texto)
}

/// Lo que la columna es, visto todo lo que se vio.
fn tipo_de(nombre: &str, vistas: &[Valor]) -> (Option<String>, Option<String>) {
    let mut v: Vec<Valor> = vistas.to_vec();
    v.sort();
    v.dedup();
    if v.is_empty() {
        // Solo nulos en la muestra: no se sabe, y se dice.
        return (None, Some("sin valores en la muestra".into()));
    }
    let n = nombre.to_ascii_lowercase();
    // Medido en F1: un código postal con ceros se lee entero en una muestra
    // que no los trae. Un nombre de código se queda en texto.
    let es_codigo = ["zip", "postal", "cep", "_code", "codigo", "cod_"]
        .iter()
        .any(|k| n.contains(k));
    use Valor::*;
    let t = match v.as_slice() {
        [Json] => return (None, Some("json".into())),
        _ if v.contains(&Json) => return (None, Some("json".into())),
        [Entero] if es_codigo => "String",
        [Entero] => "Integer",
        [Decimal] | [Entero, Decimal] => "Float",
        [Booleano] => "Boolean",
        [Fecha] => "Date",
        [FechaHora] | [Fecha, FechaHora] => "DateTime",
        [FechaHoraTz] => "DateTimeTz",
        _ => "String",
    };
    (Some(t.to_string()), None)
}

// ── CSV ─────────────────────────────────────────────────────────────────────

/// Las filas de un trozo de CSV, con comillas (`""` es una comilla, y un salto
/// de línea entre comillas es del campo). Lo que hay tras el último salto de
/// un trozo cortado se descarta: sería media fila.
fn filas_csv(texto: &str, sep: char, cortado: bool) -> Vec<Vec<String>> {
    let texto = if cortado {
        match texto.rfind('\n') {
            Some(i) => &texto[..i],
            None => texto,
        }
    } else {
        texto
    };
    let mut filas = Vec::new();
    let mut fila = Vec::new();
    let mut campo = String::new();
    let mut entre = false;
    let mut cs = texto.chars().peekable();
    while let Some(c) = cs.next() {
        match c {
            '"' if entre && cs.peek() == Some(&'"') => {
                campo.push('"');
                cs.next();
            }
            '"' => entre = !entre,
            c if c == sep && !entre => fila.push(std::mem::take(&mut campo)),
            '\r' if !entre => {}
            '\n' if !entre => {
                fila.push(std::mem::take(&mut campo));
                filas.push(std::mem::take(&mut fila));
            }
            c => campo.push(c),
        }
    }
    if !campo.is_empty() || !fila.is_empty() {
        fila.push(campo);
        filas.push(fila);
    }
    filas
}

pub fn csv(o: &dyn Origen, clave: &str, tamano: u64, tsv: bool) -> Result<Esquema, String> {
    let bytes = o.rango(clave, &format!("0-{}", MUESTRA - 1))?;
    let cortado = (bytes.len() as u64) < tamano;
    let mut texto = String::from_utf8_lossy(&bytes).into_owned();
    let mut avisos = Vec::new();
    if let Some(sin) = texto.strip_prefix('\u{feff}') {
        texto = sin.to_string();
        avisos.push(format!("`{clave}` empieza por un BOM: se quita"));
    }
    let primera = texto.lines().next().unwrap_or("");
    let sep = if tsv {
        '\t'
    } else {
        [',', ';', '\t', '|']
            .into_iter()
            .max_by_key(|s| primera.matches(*s).count())
            .filter(|s| primera.contains(*s))
            .unwrap_or(',')
    };
    let filas = filas_csv(&texto, sep, cortado);
    let Some((cabecera, datos)) = filas.split_first() else {
        return Err(format!("`{clave}` está vacío"));
    };
    let columnas = cabecera
        .iter()
        .enumerate()
        .map(|(i, nombre)| {
            let nombre = nombre.trim();
            let nombre = if nombre.is_empty() {
                format!("columna_{}", i + 1)
            } else {
                nombre.to_string()
            };
            let vistas: Vec<Valor> = datos
                .iter()
                .filter_map(|f| f.get(i).and_then(|v| clase_de_texto(v)))
                .collect();
            let (tipo, origen) = tipo_de(&nombre, &vistas);
            Columna {
                nombre,
                tipo,
                origen,
                ..Default::default()
            }
        })
        .collect();
    Ok(Esquema {
        columnas,
        filas: None,
        separador: (sep != ',').then_some(sep),
        avisos,
    })
}

// ── JSONL ───────────────────────────────────────────────────────────────────

pub fn jsonl(o: &dyn Origen, clave: &str, tamano: u64) -> Result<Esquema, String> {
    use ore_core::parse::{Node, Style};
    let bytes = o.rango(clave, &format!("0-{}", MUESTRA - 1))?;
    let cortado = (bytes.len() as u64) < tamano;
    let texto = String::from_utf8_lossy(&bytes);
    let mut lineas: Vec<&str> = texto.lines().filter(|l| !l.trim().is_empty()).collect();
    if cortado {
        lineas.pop();
    }
    let mut orden: Vec<String> = Vec::new();
    let mut vistas: std::collections::BTreeMap<String, Vec<Valor>> = Default::default();
    let mut malas = 0usize;
    for l in &lineas {
        let Ok(n) = ore_core::parse::parse(l) else {
            malas += 1;
            continue;
        };
        for (k, v) in n.entries() {
            let Some(k) = k.as_str() else { continue };
            if !vistas.contains_key(k) {
                orden.push(k.to_string());
            }
            let clase = match v {
                Node::Mapping { .. } | Node::Sequence { .. } => Some(Valor::Json),
                Node::Scalar {
                    raw,
                    style: Style::Plain,
                    ..
                } => match raw.as_str() {
                    "null" | "~" | "" => None,
                    "true" | "false" => Some(Valor::Booleano),
                    otro => clase_de_texto(otro),
                },
                // Una cadena del JSON es texto aunque parezca un número:
                // `"7"` no es un entero (lo mismo que `ore-read-jsonl`).
                Node::Scalar { raw, .. } => match clase_de_texto(raw) {
                    Some(Valor::Fecha | Valor::FechaHora | Valor::FechaHoraTz) => {
                        clase_de_texto(raw)
                    }
                    Some(_) => Some(Valor::Texto),
                    None => None,
                },
            };
            let e = vistas.entry(k.to_string()).or_default();
            if let Some(c) = clase {
                e.push(c);
            }
        }
    }
    if orden.is_empty() {
        return Err(format!("`{clave}` no tiene ninguna línea JSON con claves"));
    }
    let mut avisos = Vec::new();
    if malas > 0 {
        avisos.push(format!(
            "`{clave}`: {malas} línea(s) de la muestra no analizan"
        ));
    }
    let columnas = orden
        .into_iter()
        .map(|nombre| {
            let (tipo, origen) = tipo_de(
                &nombre,
                vistas.get(&nombre).map(Vec::as_slice).unwrap_or(&[]),
            );
            Columna {
                nombre,
                tipo,
                origen,
                ..Default::default()
            }
        })
        .collect();
    Ok(Esquema {
        columnas,
        filas: None,
        separador: None,
        avisos,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ore_objetos::memoria::EnMemoria;

    fn tipos(e: &Esquema) -> Vec<(String, Option<String>)> {
        e.columnas
            .iter()
            .map(|c| (c.nombre.clone(), c.tipo.clone()))
            .collect()
    }

    /// Lo medido en F1, en pequeño: el BOM, el código postal con ceros que la
    /// muestra no trae, un decimal, una fecha y un texto entre comillas con el
    /// separador dentro.
    #[test]
    fn un_csv_con_lo_que_midio_f1() {
        let t = "\u{feff}product_category_name,customer_zip_code_prefix,cep,precio,fecha,nota\n\
                 beleza,14409,01037,58.90,2017-10-02,\"hola, qué tal\"\n\
                 esporte,68030,08775,12,2017-10-03,\n";
        let o = EnMemoria::con(&[("a.csv", t.as_bytes().to_vec())]);
        let e = csv(&o, "a.csv", t.len() as u64, false).unwrap();
        assert_eq!(
            tipos(&e),
            [
                ("product_category_name".to_string(), Some("String".into())),
                (
                    "customer_zip_code_prefix".to_string(),
                    Some("String".into())
                ),
                ("cep".to_string(), Some("String".into())),
                ("precio".to_string(), Some("Float".into())),
                ("fecha".to_string(), Some("Date".into())),
                ("nota".to_string(), Some("String".into())),
            ]
        );
        assert!(e.avisos.iter().any(|a| a.contains("BOM")));
        assert!(e.separador.is_none());
    }

    #[test]
    fn un_csv_con_punto_y_coma() {
        let t = "id;n\n1;2\n";
        let o = EnMemoria::con(&[("b.csv", t.as_bytes().to_vec())]);
        let e = csv(&o, "b.csv", t.len() as u64, false).unwrap();
        assert_eq!(e.separador, Some(';'));
        assert_eq!(e.columnas[1].tipo.as_deref(), Some("Integer"));
    }

    /// Medido en F1: `usuario` aparece el segundo día. La unión, en el orden en
    /// que aparecen; y `"7"` es texto.
    #[test]
    fn un_jsonl_es_la_union_de_sus_claves() {
        let t = "{\"ts\":\"2026-09-27T10:00:00Z\",\"nivel\":\"info\",\"n\":3}\n\
                 {\"ts\":\"2026-09-28T10:00:00Z\",\"nivel\":\"warn\",\"n\":4.5,\"usuario\":\"7\",\"ctx\":{\"a\":1}}\n";
        let o = EnMemoria::con(&[("l.jsonl", t.as_bytes().to_vec())]);
        let e = jsonl(&o, "l.jsonl", t.len() as u64).unwrap();
        assert_eq!(
            tipos(&e),
            [
                ("ts".to_string(), Some("DateTimeTz".into())),
                ("nivel".to_string(), Some("String".into())),
                ("n".to_string(), Some("Float".into())),
                ("usuario".to_string(), Some("String".into())),
                ("ctx".to_string(), None),
            ]
        );
        assert_eq!(e.columnas[4].origen.as_deref(), Some("json"));
    }

    #[test]
    fn el_fisico_de_arrow_se_escribe_como_pyarrow() {
        use arrow_schema::{DataType as D, TimeUnit};
        assert_eq!(
            de_arrow(&D::Decimal128(12, 2)),
            (Some("Decimal<12, 2>".into()), "decimal128(12, 2)".into())
        );
        assert_eq!(
            de_arrow(&D::Timestamp(TimeUnit::Microsecond, Some("UTC".into()))),
            (Some("DateTimeTz".into()), "timestamp[us, tz=UTC]".into())
        );
        assert_eq!(
            de_arrow(&D::Int64),
            (Some("Integer".into()), "int64".into())
        );
        assert_eq!(de_arrow(&D::Binary).0, None);
    }
}
