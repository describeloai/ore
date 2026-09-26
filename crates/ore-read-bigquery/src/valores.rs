//! **De la celda de la API al texto de la fila.**
//!
//! La API entrega cada valor como cadena dentro de `{"f": [{"v": …}]}`, y el
//! esquema del resultado dice de qué tipo es. Lo que sale es el texto que
//! `ore-store` sabe estrechar al físico de 0032 —el mismo que emite
//! `ore-read-postgres`—, y **un nulo es la columna ausente** (`ore_driver::fila`).
//!
//! # Las tres pérdidas que había, y dónde se cierran
//!
//! | con `bq --format=prettyjson` | aquí |
//! |---|---|
//! | el texto `'null'` era un NULL | un nulo es `null` de JSON y el texto es `"null"`: no se tocan |
//! | un TIMESTAMP perdía los microsegundos | llega en µs enteros (`useInt64Timestamp`) |
//! | un TIMESTAMP llegaba sin zona y se quedaba como texto | se escribe con [`Valor::Instante`], el texto que `ore-store` estrecha |
//!
//! # Lo anidado
//!
//! Un `STRUCT` o un `ARRAY` salen como **JSON**, con los nombres de sus campos:
//! la copia los guarda como texto —la fila «sin tipo» de 0032— y no se pierde
//! nada ni se inventa un modelo (el catálogo sigue diciendo que no los
//! traduce). Dentro, cada escalar sigue la columna «JSON de la consola» de 0032:
//! un entero es número si cabe en 2⁵³ y cadena si no, un decimal es cadena
//! siempre, `NaN` e infinitos son cadena.
use ore_core::tipos::{Fisico, Valor};
use serde_json::{Map, Value};

/// El texto de una celda, o `None` si es nula.
pub fn texto(campo: &Value, celda: &Value) -> Result<Option<String>, String> {
    if celda.is_null() {
        return Ok(None);
    }
    if repetido(campo) || registro(campo) {
        return Ok(Some(json(campo, celda)?.to_string()));
    }
    let s = celda
        .as_str()
        .ok_or_else(|| format!("la celda de `{}` no es texto: {celda}", nombre(campo)))?;
    escalar(tipo(campo), s).map(Some)
}

fn nombre(campo: &Value) -> &str {
    campo["name"].as_str().unwrap_or("?")
}

fn tipo(campo: &Value) -> &str {
    campo["type"].as_str().unwrap_or("")
}

fn repetido(campo: &Value) -> bool {
    campo["mode"] == "REPEATED"
}

fn registro(campo: &Value) -> bool {
    matches!(tipo(campo), "RECORD" | "STRUCT")
}

/// Un escalar, en su texto. Solo TIMESTAMP cambia: los demás ya vienen en la
/// forma que su físico analiza (`2026-09-26T10:11:12.123456`, `NaN`,
/// `123…789.123…`), medido tipo a tipo en `tests/rest/tipos-int64.json`.
fn escalar(tipo: &str, s: &str) -> Result<String, String> {
    Ok(match tipo {
        "TIMESTAMP" => instante(s)?,
        _ => s.to_string(),
    })
}

/// µs desde la época → `YYYY-MM-DD HH:MM:SS[.ffffff]+00`, con el mismo código
/// que usa `ore-store` para volver a texto: lo que sale aquí es, por
/// construcción, lo que allí se estrecha.
fn instante(s: &str) -> Result<String, String> {
    let us: i64 = s.parse().map_err(|_| {
        format!(
            "un TIMESTAMP llegó como `{s}` y no como µs enteros: ¿falta \
             `formatOptions.useInt64Timestamp`?"
        )
    })?;
    Ok(Valor::Instante(us).texto(&Fisico::Instante))
}

/// Un valor anidado en JSON. `celda` es el `v` de la API.
fn json(campo: &Value, celda: &Value) -> Result<Value, String> {
    if celda.is_null() {
        return Ok(Value::Null);
    }
    if repetido(campo) {
        let mut uno = campo.clone();
        uno["mode"] = Value::from("NULLABLE");
        let elementos = celda
            .as_array()
            .ok_or_else(|| format!("`{}` es REPEATED y no llegó una lista", nombre(campo)))?;
        return elementos
            .iter()
            .map(|e| json(&uno, &e["v"]))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array);
    }
    if registro(campo) {
        let subcampos = campo["fields"].as_array().map(Vec::as_slice).unwrap_or(&[]);
        let valores = celda["f"].as_array().map(Vec::as_slice).unwrap_or(&[]);
        let mut o = Map::new();
        for (c, v) in subcampos.iter().zip(valores) {
            o.insert(nombre(c).to_string(), json(c, &v["v"])?);
        }
        return Ok(Value::Object(o));
    }
    let s = celda
        .as_str()
        .ok_or_else(|| format!("la celda de `{}` no es texto: {celda}", nombre(campo)))?;
    Ok(match tipo(campo) {
        "INTEGER" | "INT64" => match s.parse::<i64>() {
            Ok(n) if n.unsigned_abs() <= 1 << 53 => Value::from(n),
            _ => Value::from(s),
        },
        "FLOAT" | "FLOAT64" => match s.parse::<f64>() {
            Ok(f) if f.is_finite() => Value::from(f),
            _ => Value::from(s),
        },
        "BOOLEAN" | "BOOL" => Value::from(s == "true"),
        t => Value::from(escalar(t, s)?),
    })
}

/// El tipo de una columna como lo quiere un parámetro de consulta: la API
/// describe el esquema con los nombres de antes (`INTEGER`, `FLOAT`,
/// `BOOLEAN`, `RECORD`) y el parámetro espera los de GoogleSQL.
pub fn estandar(t: &str) -> &str {
    match t {
        "INTEGER" => "INT64",
        "FLOAT" => "FLOAT64",
        "BOOLEAN" => "BOOL",
        "RECORD" => "STRUCT",
        otro => otro,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rest::pruebas::grabada;

    /// La fila de `tests/rest/tipos-int64.json`, columna → texto.
    fn tipos() -> std::collections::BTreeMap<String, Option<String>> {
        let r = grabada("tipos-int64");
        let campos = r["schema"]["fields"].as_array().unwrap();
        let fila = r["rows"][0]["f"].as_array().unwrap();
        campos
            .iter()
            .zip(fila)
            .map(|(c, v)| (nombre(c).to_string(), texto(c, &v["v"]).unwrap()))
            .collect()
    }

    fn es(t: &std::collections::BTreeMap<String, Option<String>>, col: &str, esperado: &str) {
        assert_eq!(t[col].as_deref(), Some(esperado), "columna `{col}`");
    }

    #[test]
    fn las_tres_perdidas_de_bq_no_se_repiten() {
        let t = tipos();
        es(&t, "textonull", "null");
        assert_eq!(t["nulo"], None, "un NULL es la columna ausente");
        es(&t, "vacio", "");
        es(&t, "tsmax", "9999-12-31 23:59:59.999999+00");
        es(&t, "tsmin", "0001-01-01 00:00:00+00");
    }

    /// Cada tipo medido, en el texto que su físico de 0032 analiza.
    #[test]
    fn cada_tipo_en_el_texto_de_su_fisico() {
        let t = tipos();
        es(&t, "dt", "2026-09-26T10:11:12.123456");
        es(&t, "t", "23:59:59.999999");
        es(&t, "dmin", "0001-01-01");
        es(&t, "nmin", "-99999999999999999999999999999.999999999");
        es(
            &t,
            "bn",
            "123456789012345678901234567890123456789.12345678901234567890123456789012345678",
        );
        es(&t, "i64max", "9223372036854775807");
        es(&t, "nan", "NaN");
        es(&t, "inf", "Infinity");
        es(&t, "ninf", "-Infinity");
        es(&t, "tiny", "1.0E-300");
        es(&t, "b", "true");
        es(&t, "byt", "AP9ob2xh");
        es(&t, "g", "POINT(-8.4 43.37)");
        es(&t, "iv", "0-0 1 0:0:0");
        es(&t, "rg", "[2024-01-01, 2024-02-01)");
        es(&t, "j", r#"{"a":[1,2,{"b":null}],"c":"null"}"#);
        // Y lo que de verdad importa de cada texto: que su físico lo analiza.
        for (col, f) in [
            ("dt", Fisico::FechaHora),
            ("t", Fisico::Hora),
            ("dmin", Fisico::Fecha),
            ("tsmax", Fisico::Instante),
            ("tsmin", Fisico::Instante),
            ("nan", Fisico::Real),
            ("inf", Fisico::Real),
            ("tiny", Fisico::Real),
            ("i64max", Fisico::Entero),
            ("b", Fisico::Logico),
        ] {
            let s = t[col].as_deref().unwrap();
            assert!(
                f.analizar(s).is_some(),
                "`{col}` = `{s}` no lo analiza {f:?}"
            );
        }
    }

    #[test]
    fn lo_anidado_sale_como_json_con_nombres() {
        let t = tipos();
        es(&t, "st", r#"{"a":1,"b":{"c":"x","d":[1,2]}}"#);
        es(&t, "arr", r#"[{"k":1,"v":"v"},{"k":2,"v":"w"}]"#);
        es(&t, "ints", "[1,2]");
        es(&t, "emptyarr", "[]");
    }

    /// Sin `useInt64Timestamp` el instante es un float: se niega en vez de
    /// escribir un instante corrido.
    #[test]
    fn un_timestamp_en_float_se_niega() {
        let r = grabada("tipos-por-defecto");
        let campos = r["schema"]["fields"].as_array().unwrap();
        let i = campos.iter().position(|c| c["name"] == "tsmax").unwrap();
        let e = texto(&campos[i], &r["rows"][0]["f"][i]["v"]).unwrap_err();
        assert!(e.contains("useInt64Timestamp"), "{e}");
    }

    #[test]
    fn la_semilla_de_pedidos_llega_exacta() {
        let r = grabada("pedidos-query");
        let campos = r["schema"]["fields"].as_array().unwrap();
        let filas: Vec<Vec<Option<String>>> = r["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                campos
                    .iter()
                    .zip(f["f"].as_array().unwrap())
                    .map(|(c, v)| texto(c, &v["v"]).unwrap())
                    .collect()
            })
            .collect();
        let p2 = &filas[1];
        assert_eq!(p2[3].as_deref(), Some("2026-09-02 23:59:59.123456+00"));
        let p4 = &filas[3];
        assert_eq!(p4[2].as_deref(), Some("12345678901234567890.123456789"));
        assert_eq!(
            p4[3].as_deref(),
            Some("2026-09-03 10:00:00+00"),
            "+02 → UTC"
        );
        let p5 = &filas[4];
        assert_eq!((p5[2].as_deref(), p5[3].as_deref()), (None, None));
    }

    #[test]
    fn los_nombres_de_antes_pasan_a_googlesql() {
        assert_eq!(estandar("INTEGER"), "INT64");
        assert_eq!(estandar("FLOAT"), "FLOAT64");
        assert_eq!(estandar("BOOLEAN"), "BOOL");
        assert_eq!(estandar("TIMESTAMP"), "TIMESTAMP");
    }
}
