//! **Los tipos de CockroachDB**, hacia los de OOS y hacia Arrow (ADR 0080, paso 2).
//!
//! ⚠️ **Adaptado de `ore-read-postgres`** (`traducir`, `escalar`, `decimal`, `exacto`, `Forma`, `normalizar`, `elementos` y `poner`), sin
//! la geometría. Es código duplicado a propósito (ADR 0080 §1.1): el catálogo de CockroachDB (`pg_attribute`, `format_type`) contesta
//! con los mismos nombres que PostgreSQL —medido en 22.2, 24.3 y 25.4—, así que la traducción es la misma.
//!
//! Se lee **en texto**, como el origen Postgres: cada columna sale de la consulta como `col::STRING` y llega como los bytes UTF-8 de su
//! texto. No hay decodificador binario que pueda divergir ni tipo que no se sepa leer: lo que no es exacto sale como su texto.
//!
//! * `INT2/4/8`, `FLOAT4/8`, `BOOL`, `DATE`, `TIME`, `TIMESTAMP(TZ)`, `STRING`, `UUID`, `BYTES` y `DECIMAL(p, s)` (≤ 38 cifras) se tipan.
//! * `ENUM` es `String`; un array de uno de esos, `list<X>`.
//! * `TIMETZ`, `INTERVAL`, `JSONB`, `INET`, `BIT`, `DECIMAL` sin precisión, `GEOGRAPHY`, `GEOMETRY`… salen como su texto (`Opaque`).
//!   La geometría como EWKB nativo es de un paso posterior (ADR 0080 §3.1 D).

use ore_base::tipos::Fisico;

/// El tipo de OOS de un tipo de CockroachDB (con su precisión, tal como lo escribe `format_type`), o `None` si no se sabe.
pub fn traducir(tipo: &str, familia: &str, base: Option<&str>) -> Option<String> {
    let t = tipo.trim();
    if let Some(dentro) = t.strip_suffix("[]") {
        let d = dentro.trim();
        let x = decimal(d).or_else(|| escalar(d).map(String::from))?;
        // Un `bytea[]` sale como texto (`Opaque`): una lista de binarios aún no viaja en Arrow.
        return Some(if exacto(&x) && x != "Binary" {
            format!("list<{x}>")
        } else {
            "Opaque".into()
        });
    }
    match familia {
        "d" => {
            let b = base?;
            decimal(b).or_else(|| escalar(b).map(String::from))
        }
        "e" => Some("String".into()),
        _ => decimal(t).or_else(|| escalar(t).map(String::from)),
    }
}

/// `numeric(p, s)` y `numeric(p)` —escala 0— son `Decimal<p, s>` si caben en 38 cifras, el techo de Iceberg; si no, `String`.
/// `numeric` sin límites devuelve `None`: su precisión no se declaró.
fn decimal(pg: &str) -> Option<String> {
    let (base, resto) = pg.split_once('(')?;
    if !matches!(base.trim(), "numeric" | "decimal") {
        return None;
    }
    let n: Vec<u16> = resto
        .strip_suffix(')')?
        .split(',')
        .map(|x| x.trim().parse().ok())
        .collect::<Option<_>>()?;
    let (p, s) = match n[..] {
        [p] => (p, 0),
        [p, s] => (p, s),
        _ => return None,
    };
    Some(if (1..=38).contains(&p) && s <= p {
        format!("Decimal<{p}, {s}>")
    } else {
        "String".into()
    })
}

fn escalar(pg: &str) -> Option<&'static str> {
    // `numeric(12,2)` y `character varying(255)` son el mismo tipo que sin paréntesis: la precisión es una restricción.
    let base = pg.split_once('(').map_or(pg, |(b, _)| b).trim();
    Some(match base {
        "smallint" | "integer" | "bigint" | "int2" | "int4" | "int8" | "oid" => "Integer",
        "numeric" | "decimal" => "Decimal",
        "real" | "double precision" | "float4" | "float8" => "Float",
        "boolean" | "bool" => "Boolean",
        "text" | "character varying" | "character" | "varchar" | "char" | "bpchar" | "name"
        | "string" | "\"char\"" => "String",
        // Un UUID tiene una forma canónica en texto y se compara como texto.
        "uuid" => "String",
        "date" => "Date",
        "time without time zone" | "time" => "Time",
        // Lleva un desfase que `Time` no tiene, y el texto cambia entre versiones (`+02:00:00` en 22.2, `+02` desde 24.3).
        "time with time zone" | "timetz" => "Opaque",
        "timestamp without time zone" | "timestamp" => "DateTime",
        "timestamp with time zone" | "timestamptz" => "DateTimeTz",
        "bytea" | "bytes" => "Binary",
        "jsonb" | "json" | "inet" | "interval" | "bit" | "bit varying" | "varbit" | "geography"
        | "geometry" | "box2d" | "tsvector" | "tsquery" | "pg_lsn" | "regclass" => "Opaque",
        _ => return None,
    })
}

/// ¿Se tipa con exactitud lo que `traducir` dice? Es la misma pregunta para el catálogo y para la lectura.
pub fn exacto(oos: &str) -> bool {
    matches!(
        oos,
        "Integer"
            | "Float"
            | "Boolean"
            | "Date"
            | "Time"
            | "DateTime"
            | "DateTimeTz"
            | "String"
            | "Binary"
    ) || oos.starts_with("Decimal<")
}

/// El físico de una columna por su tipo, o `None` si va como texto.
fn fisico(formato: &str, familia: &str, base: Option<&str>) -> Option<Fisico> {
    let oos = traducir(formato, familia, base)?;
    if !exacto(&oos) {
        return None;
    }
    let t = ore_base::types::parse_type(&oos).ok()?;
    Some(Fisico::de(&t))
}

/// Lo que sale de una columna: un escalar —con su físico, o como texto— o la lista de un escalar exacto (`bigint[]` → `List<Int64>`).
#[derive(Debug, Clone, PartialEq)]
pub enum Forma {
    Escalar(Option<Fisico>),
    Lista(Fisico),
}

impl Forma {
    pub fn de(formato: &str, familia: &str, base: Option<&str>) -> Forma {
        match formato.trim().strip_suffix("[]") {
            Some(dentro) => match fisico(dentro.trim(), "b", None) {
                // Una lista de binarios sale como texto, como dice el catálogo (`Opaque`).
                Some(f) if f != Fisico::Binario => Forma::Lista(f),
                _ => Forma::Escalar(None),
            },
            None => Forma::Escalar(fisico(formato, familia, base)),
        }
    }

    pub fn escalar(&self) -> Option<&Fisico> {
        match self {
            Forma::Escalar(f) => f.as_ref(),
            Forma::Lista(f) => Some(f),
        }
    }

    /// Cómo la pide la consulta: por su texto, que es lo que `rust-postgres` entrega como los bytes de un `text`.
    pub fn en_la_consulta(&self, col: &str) -> String {
        format!("ore_q.\"{}\"::STRING", col.replace('"', "\"\""))
    }

    /// La forma de `ore-base`: sin tipo exacto, texto.
    pub fn ore(&self) -> ore_base::tipos::Forma {
        use ore_base::tipos::Forma as O;
        match self {
            Forma::Escalar(f) => O::Escalar(f.unwrap_or(Fisico::Texto)),
            Forma::Lista(f) => O::Lista(Box::new(O::Escalar(*f))),
        }
    }
}

/// Lo que un valor necesitó para caber, contado por columna y dicho al final.
pub type Aviso = &'static str;

const AVISO_INFINITO_FECHA: Aviso =
    "valores `infinity`/`-infinity` llevados a 9999-12-31 / 0001-01-01";
const AVISO_DECIMAL: Aviso = "valores `NaN`/infinito en un decimal, como nulos";

/// **El texto de CockroachDB, hacia el analizador.** Lo que escribe distinto de lo que `ore_base::tipos` lee: `t`/`f`, y los valores que el
/// tipo de destino no tiene.
pub fn normalizar<'a>(
    f: &Fisico,
    t: &'a str,
) -> (Option<std::borrow::Cow<'a, str>>, Option<Aviso>) {
    use std::borrow::Cow::{Borrowed, Owned};
    let fecha = |fin: &str, ini: &str| match t {
        "infinity" => (Some(Owned(fin.to_string())), Some(AVISO_INFINITO_FECHA)),
        "-infinity" => (Some(Owned(ini.to_string())), Some(AVISO_INFINITO_FECHA)),
        _ => (Some(Borrowed(t)), None),
    };
    match f {
        Fisico::Logico => match t {
            "t" => (Some(Borrowed("true")), None),
            "f" => (Some(Borrowed("false")), None),
            _ => (Some(Borrowed(t)), None),
        },
        Fisico::Decimal { .. } if matches!(t, "NaN" | "Infinity" | "-Infinity") => {
            (None, Some(AVISO_DECIMAL))
        }
        Fisico::Fecha => fecha("9999-12-31", "0001-01-01"),
        Fisico::FechaHora => fecha("9999-12-31 23:59:59.999999", "0001-01-01 00:00:00"),
        Fisico::Instante => fecha("9999-12-31 23:59:59.999999+00", "0001-01-01 00:00:00+00"),
        // `bytes` sale en hexadecimal (`\xDEAD`, el `bytea_output` por defecto); el texto canónico de `Binary` es base64.
        Fisico::Binario => match t.strip_prefix("\\x").and_then(|h| {
            if h.is_empty() {
                Some(Vec::new())
            } else {
                ore_base::firma::hex(h)
            }
        }) {
            Some(b) => (Some(Owned(ore_base::tipos::base64_codificar(&b))), None),
            None => (Some(Borrowed(t)), None),
        },
        _ => (Some(Borrowed(t)), None),
    }
}

/// **Los elementos de un array de una dimensión**, de su texto (`{1,2,NULL,"a,b"}`). Uno de más dimensiones no cabe en una lista.
pub fn elementos(t: &str) -> Result<Vec<Option<String>>, String> {
    let dentro = t
        .strip_prefix('{')
        .and_then(|r| r.strip_suffix('}'))
        .ok_or_else(|| format!("`{t}` no es un array"))?;
    if dentro.starts_with('{') {
        return Err("un array de más de una dimensión no cabe en una lista".into());
    }
    let mut out = Vec::new();
    if dentro.is_empty() {
        return Ok(out);
    }
    let mut cs = dentro.chars().peekable();
    loop {
        if cs.peek() == Some(&'"') {
            cs.next();
            let mut v = String::new();
            loop {
                match cs.next() {
                    Some('\\') => v.extend(cs.next()),
                    Some('"') => break,
                    Some(c) => v.push(c),
                    None => return Err(format!("`{t}`: una comilla sin cerrar")),
                }
            }
            out.push(Some(v));
        } else {
            let mut v = String::new();
            while let Some(&c) = cs.peek() {
                if c == ',' {
                    break;
                }
                v.push(c);
                cs.next();
            }
            out.push(if v.eq_ignore_ascii_case("NULL") {
                None
            } else {
                Some(v)
            });
        }
        match cs.next() {
            Some(',') => continue,
            None => break,
            Some(c) => return Err(format!("`{t}`: `{c}` donde iba una coma")),
        }
    }
    Ok(out)
}

/// Escribe un valor en su texto en la columna: lo normaliza, lo analiza y devuelve el aviso, si lo hubo. `Err` con el tipo si no cabe;
/// entonces no escribe nada de esa celda.
pub fn poner(
    w: &mut ore_arrow::Escritor<'_>,
    forma: &Forma,
    v: Option<&str>,
) -> Result<Option<Aviso>, String> {
    if let Forma::Lista(f) = forma {
        let Some(t) = v else {
            w.nulo();
            return Ok(None);
        };
        let mut aviso = None;
        for e in elementos(t)? {
            let mut h = w.elemento().map_err(|e| e.0)?;
            aviso = poner_escalar(&mut h, Some(f), e.as_deref())?.or(aviso);
        }
        w.cerrar_lista().map_err(|e| e.0)?;
        return Ok(aviso);
    }
    poner_escalar(w, forma.escalar(), v)
}

fn poner_escalar(
    w: &mut ore_arrow::Escritor<'_>,
    f: Option<&Fisico>,
    v: Option<&str>,
) -> Result<Option<Aviso>, String> {
    let Some(t) = v else {
        w.nulo();
        return Ok(None);
    };
    let f = match f {
        None | Some(Fisico::Texto) => return w.texto(t).map(|_| None).map_err(|e| e.0),
        Some(f) => f,
    };
    let (t, aviso) = normalizar(f, t);
    match t {
        None => w.nulo(),
        Some(t) => {
            let valor = f
                .analizar(&t)
                .ok_or_else(|| format!("`{t}` no es un {}", f.arrow()))?;
            w.valor(&valor)
                .map_err(|_| format!("`{t}` no es un {}", f.arrow()))?;
        }
    }
    Ok(aviso)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_tipo_sale_del_catalogo_y_solo_lo_exacto_se_tipa() {
        assert_eq!(fisico("bigint", "b", None), Some(Fisico::Entero));
        assert_eq!(
            fisico("numeric(12,2)", "b", None),
            Some(Fisico::Decimal {
                precision: 12,
                escala: 2
            })
        );
        assert_eq!(fisico("numeric", "b", None), None, "sin precisión, texto");
        assert_eq!(
            fisico("time with time zone", "b", None),
            None,
            "con desfase, texto"
        );
        assert_eq!(fisico("jsonb", "b", None), None);
        assert_eq!(fisico("geography", "b", None), None);
        assert_eq!(fisico("boolean", "b", None), Some(Fisico::Logico));
        assert_eq!(
            fisico("timestamp with time zone", "b", None),
            Some(Fisico::Instante)
        );
    }

    #[test]
    fn un_enum_es_string_y_un_array_de_un_escalar_exacto_es_una_lista() {
        assert_eq!(traducir("estado", "e", None).as_deref(), Some("String"));
        assert_eq!(
            traducir("bigint[]", "b", None).as_deref(),
            Some("list<Integer>")
        );
        assert_eq!(
            traducir("text[]", "b", None).as_deref(),
            Some("list<String>")
        );
        assert_eq!(traducir("bytea[]", "b", None).as_deref(), Some("Opaque"));
        assert_eq!(
            Forma::de("bigint[]", "b", None),
            Forma::Lista(Fisico::Entero)
        );
        assert_eq!(Forma::de("jsonb[]", "b", None), Forma::Escalar(None));
    }

    #[test]
    fn un_decimal_grande_viaja_como_texto() {
        assert_eq!(
            traducir("numeric(38,10)", "b", None).as_deref(),
            Some("Decimal<38, 10>")
        );
        assert_eq!(
            traducir("numeric(40,2)", "b", None).as_deref(),
            Some("String")
        );
    }

    #[test]
    fn la_consulta_pide_cada_columna_por_su_texto_y_cita_el_nombre() {
        assert_eq!(
            Forma::Escalar(None).en_la_consulta("a\"b"),
            "ore_q.\"a\"\"b\"::STRING"
        );
    }

    #[test]
    fn los_elementos_de_un_array() {
        assert_eq!(
            elementos("{1,2,NULL,4}").unwrap(),
            vec![Some("1".into()), Some("2".into()), None, Some("4".into())]
        );
        assert_eq!(
            elementos("{a,\"b,c\",\"d\\\"e\"}").unwrap(),
            vec![Some("a".into()), Some("b,c".into()), Some("d\"e".into())]
        );
        assert_eq!(elementos("{}").unwrap(), Vec::<Option<String>>::new());
        assert!(
            elementos("{{1,2},{3,4}}").is_err(),
            "dos dimensiones no caben"
        );
        assert!(elementos("1,2").is_err());
    }

    #[test]
    fn el_texto_de_cockroach_hacia_el_analizador() {
        assert_eq!(normalizar(&Fisico::Logico, "t").0.as_deref(), Some("true"));
        assert_eq!(
            normalizar(&Fisico::Logico, "false").0.as_deref(),
            Some("false")
        );
        assert_eq!(
            normalizar(&Fisico::Fecha, "infinity").1,
            Some(AVISO_INFINITO_FECHA)
        );
        assert_eq!(
            normalizar(&Fisico::Binario, "\\xdeadbeef").0.as_deref(),
            Some("3q2+7w==")
        );
        assert_eq!(normalizar(&Fisico::Binario, "\\x").0.as_deref(), Some(""));
    }
}
