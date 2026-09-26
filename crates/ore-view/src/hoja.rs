//! **La hoja tipada**: de las filas en texto que da la copia al Z-set con
//! tipos que el circuito evalúa.
//!
//! `ore-store-<r2|gcs> leer` devuelve una copia como **una línea de cabecera**
//! —la del sobre, con `esquema`: campo → tipo de OOS— y **una fila por línea**,
//! cada una un objeto de cadenas donde un nulo es la propiedad ausente. Es
//! texto porque así lo dio el origen (`carga.rs`: solo `Integer` y `Boolean`
//! se estrechan en el Parquet; el resto viaja como cadena) y porque el
//! protocolo de la copia no distingue tipos: **los distingue la cabecera**, que
//! es donde el esquema es normativo.
//!
//! Aquí se cruza lo uno con lo otro. Un `Integer` es un [`Valor::Entero`]; un
//! `Decimal` o un `Float` es un [`Valor::Decimal`] **con sus dígitos tal y
//! como vinieron** —el circuito suma decimales de forma exacta, sin pasar por
//! un doble—; un `Boolean` es un [`Valor::Booleano`]; y todo lo demás —fechas,
//! horas, cadenas, opacos— es una [`Valor::Cadena`], que se compara por
//! igualdad y se agrupa, que es lo único que la gramática de `View` hace con
//! ellas.
//!
//! # Lo que se niega, y por qué aquí
//!
//! Un `Float` que llegue como `1e-05`, `NaN` o `Infinity` no tiene aritmética
//! exacta: sumarlo exigiría leerlo como doble, y entonces `0.1 + 0.2` dejaría
//! de ser lo que la copia dice. Se niega **en la hoja, nombrando la columna y
//! el valor**, y no más tarde como un «tipos distintos» sin fila: la persona
//! que ve el error tiene que poder ir al dato. Lo mismo un `Integer` que no
//! quepa en `i64` y un `Boolean` que no sea `true` o `false`: la cabecera
//! prometió un tipo y la fila no lo cumple, y eso es de quien escribió la copia.

use crate::delta_compiler::{Fila, Zset};
use crate::plan::Valor;
use std::collections::BTreeMap;

/// Un valor en texto, con el tipo que la cabecera le da.
pub fn valor(tipo: &str, texto: &str) -> Result<Valor, String> {
    Ok(match tipo {
        "Integer" => Valor::Entero(
            texto
                .parse()
                .map_err(|_| format!("`{texto}` no es un entero de 64 bits"))?,
        ),
        // `Decimal<p, s>` (0032 T4) se lee como un `Decimal`: su valor es el
        // mismo decimal plano, y la precisión es del contrato de la copia.
        t if t == "Decimal" || t == "Float" || t.starts_with("Decimal<") => {
            if !decimal_plano(texto) {
                return Err(format!(
                    "`{texto}` no es un decimal plano: un {tipo} con exponente, `NaN` o \
                     `Infinity` no tiene aritmética exacta y aquí no se lee como doble"
                ));
            }
            Valor::Decimal(texto.to_string()).normalizado()
        }
        "Boolean" => match texto {
            "true" => Valor::Booleano(true),
            "false" => Valor::Booleano(false),
            _ => return Err(format!("`{texto}` no es `true` ni `false`")),
        },
        _ => Valor::Cadena(texto.to_string()),
    })
}

/// `-?[0-9]+(\.[0-9]+)?` — lo que [`Valor::Decimal`] sabe sumar y comparar.
fn decimal_plano(s: &str) -> bool {
    let s = s.strip_prefix('-').unwrap_or(s);
    let (ent, frac) = s.split_once('.').unwrap_or((s, ""));
    let digitos = |p: &str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit());
    digitos(ent) && (s.split_once('.').is_none() || digitos(frac))
}

/// **Las filas de una copia, tipadas por su esquema.** Cada línea es un objeto
/// de cadenas; una propiedad ausente es un nulo y no entra en la fila. Una
/// columna que la cabecera no declare se lleva como cadena: la copia la trajo,
/// y callarla sería inventar un esquema.
///
/// Devuelve también **cuántas líneas se leyeron**: el Z-set funde filas
/// idénticas en un peso, así que su tamaño no es la cuenta de filas.
pub fn tipar<'a>(
    esquema: &BTreeMap<String, String>,
    lineas: impl IntoIterator<Item = &'a str>,
) -> Result<(Zset, u64), String> {
    let mut z = Zset::nuevo();
    let mut n = 0u64;
    for (i, linea) in lineas.into_iter().enumerate() {
        if linea.trim().is_empty() {
            continue;
        }
        let nodo = ore_core::parse::parse(linea)
            .map_err(|e| format!("la fila {} no analiza: {e:?}", i + 1))?;
        let mut fila = Fila::new();
        for (k, v) in nodo.entries() {
            let (Some(nombre), Some(texto)) = (k.as_str(), v.as_str()) else {
                return Err(format!(
                    "la fila {} no es un objeto plano de cadenas",
                    i + 1
                ));
            };
            let tipo = esquema.get(nombre).map_or("String", String::as_str);
            let valor = valor(tipo, texto)
                .map_err(|e| format!("fila {} · `{nombre}` ({tipo}): {e}", i + 1))?;
            fila.insert(nombre.to_string(), valor);
        }
        z.insertar(fila, 1);
        n += 1;
    }
    Ok((z, n))
}

/// **Lo que `ore-store leer` escribe, entero**: la cabecera en la primera
/// línea y las filas debajo. Devuelve el esquema, el Z-set y las filas leídas.
pub fn de_leer(salida: &str) -> Result<(BTreeMap<String, String>, Zset, u64), String> {
    let (cabecera, filas) = salida.split_once('\n').unwrap_or((salida, ""));
    let n = ore_core::parse::parse(cabecera)
        .map_err(|e| format!("la cabecera de la copia no analiza: {e:?}"))?;
    let esquema: BTreeMap<String, String> = n
        .get("esquema")
        .map(|(_, e)| {
            e.entries()
                .iter()
                .filter_map(|(k, v)| Some((k.as_str()?.to_string(), v.as_str()?.to_string())))
                .collect()
        })
        .ok_or("a la cabecera de la copia le falta `esquema`")?;
    let (z, leidas) = tipar(&esquema, filas.lines())?;
    Ok((esquema, z, leidas))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn esquema(pares: &[(&str, &str)]) -> BTreeMap<String, String> {
        pares
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// Cada tipo de la cabecera da su clase de valor; el nulo no entra.
    #[test]
    fn la_cabecera_tipa_y_el_nulo_es_ausencia() {
        let e = esquema(&[
            ("n", "Integer"),
            ("peso", "Float"),
            ("ok", "Boolean"),
            ("dia", "Date"),
        ]);
        let (z, leidas) = tipar(
            &e,
            [
                r#"{"dia":"2020-02-29","n":"7","ok":"true","peso":"12.50"}"#,
                r#"{"n":"-1","ok":"false"}"#,
                "",
            ],
        )
        .unwrap();
        assert_eq!(leidas, 2);
        let filas: Vec<&Fila> = z.presentes().map(|(f, _)| f).collect();
        let primera = filas
            .iter()
            .find(|f| f.get("n") == Some(&Valor::Entero(7)))
            .unwrap();
        assert_eq!(primera.get("peso"), Some(&Valor::Decimal("12.5".into())));
        assert_eq!(primera.get("ok"), Some(&Valor::Booleano(true)));
        assert_eq!(
            primera.get("dia"),
            Some(&Valor::Cadena("2020-02-29".into()))
        );
        let segunda = filas
            .iter()
            .find(|f| f.get("n") == Some(&Valor::Entero(-1)))
            .unwrap();
        assert!(
            !segunda.contains_key("peso"),
            "el nulo es la propiedad ausente"
        );
    }

    /// Lo que la cabecera promete y la fila no cumple se niega con fila, columna y valor.
    #[test]
    fn lo_que_no_cumple_el_tipo_se_niega_nombrando_el_dato() {
        let e = esquema(&[("peso", "Float"), ("n", "Integer"), ("ok", "Boolean")]);
        let err = tipar(&e, [r#"{"peso":"1e-05"}"#]).unwrap_err();
        assert!(
            err.contains("fila 1") && err.contains("`peso`") && err.contains("1e-05"),
            "{err}"
        );
        let err = tipar(&e, [r#"{"peso":"NaN"}"#]).unwrap_err();
        assert!(err.contains("NaN"), "{err}");
        let err = tipar(&e, [r#"{"n":"x"}"#]).unwrap_err();
        assert!(err.contains("`n`") && err.contains("entero"), "{err}");
        let err = tipar(&e, [r#"{"ok":"t"}"#]).unwrap_err();
        assert!(err.contains("`ok`"), "{err}");
    }

    /// Dos filas iguales son una con peso 2: el Z-set es un multiconjunto.
    #[test]
    fn las_filas_iguales_pesan() {
        let e = esquema(&[("pais", "String")]);
        let (z, leidas) = tipar(&e, [r#"{"pais":"ES"}"#, r#"{"pais":"ES"}"#]).unwrap();
        assert_eq!(leidas, 2);
        let (f, w) = z.presentes().next().unwrap();
        assert_eq!((f.get("pais"), w), (Some(&Valor::Cadena("ES".into())), 2));
    }

    /// La salida de `ore-store leer`, entera: cabecera y filas.
    #[test]
    fn de_leer_cruza_la_cabecera_con_las_filas() {
        let salida = "{\"bundle\":\"x\",\"esquema\":{\"n\":\"Integer\",\"pais\":\"String\"},\"plan\":\"p\"}\n{\"n\":\"1\",\"pais\":\"ES\"}\n{\"n\":\"2\",\"pais\":\"PT\"}\n";
        let (esquema, z, leidas) = de_leer(salida).unwrap();
        assert_eq!(esquema.get("n").map(String::as_str), Some("Integer"));
        assert_eq!(leidas, 2);
        assert_eq!(z.presentes().count(), 2);
    }
}
