//! **El lote dorado de BigQuery por consulta** (ORE 0075, fase 6): lo que `Col` produce hoy, fijado antes de tocar
//! cómo se arma.
//!
//! Cada columna recibe los textos como los deja `valores::texto` (enteros y decimales tal cual, `NaN`, bytes en base64,
//! un TIMESTAMP ya como `YYYY-MM-DD HH:MM:SS+00`) por el mismo camino que la página (`Col::poner` y `Col::fin`). Se
//! vuelcan el tipo de Arrow de cada físico, lo que dijo cada celda y lo que quedó en la columna. Se compara con
//! `tests/dorado/lote.txt`; si un cambio mueve una línea, falla.
//!
//! Regenerar (solo con una razón dicha en el commit): `ORE_ACTUALIZAR=1 cargo test -p ore-read-bigquery dorado`.

use super::*;
use std::fmt::Write as _;

/// Una columna de BigQuery en construcción: lo que la página hace con una celda, en un lote de una columna.
struct Una {
    fisico: Option<Fisico>,
    lote: RecordBatchBuilder,
}

impl Una {
    fn de(fisico: Option<Fisico>) -> Una {
        let c = Columna::new("c", fisico.unwrap_or(Fisico::Texto), "");
        Una {
            fisico,
            lote: RecordBatchBuilder::nuevo(&[c]).unwrap(),
        }
    }

    fn poner(&mut self, v: Option<&str>) -> Result<(), String> {
        let r = poner(&mut self.lote.col(0), self.fisico.as_ref(), v);
        if r.is_ok() {
            self.lote.fila_escrita();
        }
        r
    }

    fn fin(&mut self) -> Result<arrow_array::ArrayRef, String> {
        let lote = self.lote.finish().map_err(|e| e.to_string())?;
        Ok(lote.column(0).clone())
    }
}

/// Una celda: su texto (`None` es nulo).
type Celda = Option<&'static str>;

/// Vuelca una columna: su físico, su tipo de Arrow, y por celda el resultado de `poner`.
fn columna(nombre: &str, fisico: Option<Fisico>, celdas: &[Celda]) -> String {
    let mut s = String::new();
    writeln!(
        s,
        "== {nombre}: {fisico:?} -> {:?}",
        tipo_arrow(fisico.as_ref())
    )
    .unwrap();
    let mut col = Una::de(fisico);
    for (i, c) in celdas.iter().enumerate() {
        let r = col.poner(*c);
        writeln!(s, "  {i} {:<34} -> {r:?}", format!("{c:?}")).unwrap();
    }
    match col.fin() {
        Ok(a) => {
            writeln!(s, "  tipo {:?}", a.data_type()).unwrap();
            writeln!(s, "  nulos {} de {}", a.null_count(), a.len()).unwrap();
            for l in format!("{a:?}").lines() {
                writeln!(s, "  | {l}").unwrap();
            }
        }
        Err(e) => writeln!(s, "  fin -> Err({e:?})").unwrap(),
    }
    s
}

fn fijado(caso: &str, actual: &str) {
    let ruta = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/dorado")
        .join(format!("{caso}.txt"));
    if std::env::var_os("ORE_ACTUALIZAR").is_some() {
        std::fs::create_dir_all(ruta.parent().unwrap()).unwrap();
        std::fs::write(&ruta, actual).unwrap();
        return;
    }
    let esperado = std::fs::read_to_string(&ruta)
        .unwrap_or_else(|e| panic!("falta {}: {e}", ruta.display()))
        .replace("\r\n", "\n");
    assert_eq!(esperado, actual, "el caso `{caso}` ya no es el de antes");
}

#[test]
fn el_lote_de_bigquery_es_el_de_siempre() {
    let dec = |precision, escala| Fisico::Decimal { precision, escala };
    let mut s = String::new();
    let mut c = |n: &str, f: Option<Fisico>, celdas: &[Celda]| s.push_str(&columna(n, f, celdas));
    c(
        "sin tipo",
        None,
        &[
            Some("hola"),
            Some(""),
            Some("null"),
            None,
            Some("{\"a\":1}"),
        ],
    );
    c(
        "texto",
        Some(Fisico::Texto),
        &[Some("ñandú"), Some(" "), Some("42"), None],
    );
    c(
        "entero",
        Some(Fisico::Entero),
        &[
            Some("42"),
            Some("-7"),
            Some("9223372036854775807"),
            Some("-9223372036854775808"),
            Some("9223372036854775808"),
            Some("1.0"),
            Some("x"),
            Some(""),
            None,
        ],
    );
    c(
        "real",
        Some(Fisico::Real),
        &[
            Some("1.5"),
            Some("-0"),
            Some("NaN"),
            Some("Infinity"),
            Some("-Infinity"),
            Some("1e300"),
            Some("nada"),
            None,
        ],
    );
    c(
        "logico",
        Some(Fisico::Logico),
        &[
            Some("true"),
            Some("false"),
            Some("t"),
            Some("TRUE"),
            Some("1"),
            Some("x"),
            None,
        ],
    );
    c(
        "decimal(12,2)",
        Some(dec(12, 2)),
        &[
            Some("1234.56"),
            Some("-0.05"),
            Some("7"),
            Some("1.239"),
            Some("9999999999.99"),
            Some("99999999999.99"),
            Some("NaN"),
            Some("doce"),
            None,
        ],
    );
    c(
        "decimal(38,9)",
        Some(dec(38, 9)),
        &[
            Some("12345678901234567890123456789.123456789"),
            Some("-0.000000001"),
            Some("0"),
            None,
        ],
    );
    c(
        "fecha",
        Some(Fisico::Fecha),
        &[
            Some("2026-09-26"),
            Some("1970-01-01"),
            Some("0001-01-01"),
            Some("2026-02-30"),
            Some("26/09/2026"),
            None,
        ],
    );
    c(
        "hora",
        Some(Fisico::Hora),
        &[
            Some("10:11:12"),
            Some("10:11:12.123456"),
            Some("23:59:59.999999"),
            Some("24:00:00"),
            Some("las diez"),
            None,
        ],
    );
    c(
        "fecha-hora",
        Some(Fisico::FechaHora),
        &[
            Some("2026-09-26T10:11:12"),
            Some("2026-09-26T10:11:12.123456"),
            Some("2026-09-26 10:11:12"),
            Some("2026-09-26"),
            Some("ayer"),
            None,
        ],
    );
    c(
        "instante",
        Some(Fisico::Instante),
        &[
            Some("2026-09-26 10:11:12+00"),
            Some("2026-09-26 10:11:12.123456+00"),
            Some("1969-12-31 23:59:59.999999+00"),
            Some("2026-09-26T10:11:12Z"),
            Some("2026-09-26T12:11:12+02:00"),
            Some("2026-09-26 10:11:12"),
            Some("pronto"),
            None,
        ],
    );
    c(
        "binario",
        Some(Fisico::Binario),
        &[
            Some("AAEC/w=="),
            Some(""),
            Some("aGVsbG8="),
            Some("sin relleno"),
            Some("====="),
            None,
        ],
    );
    fijado("lote", &s);
}

#[test]
fn el_esquema_por_fisico_es_el_de_siempre() {
    let mut s = String::new();
    for f in [
        None,
        Some(Fisico::Texto),
        Some(Fisico::Entero),
        Some(Fisico::Real),
        Some(Fisico::Logico),
        Some(Fisico::Decimal {
            precision: 38,
            escala: 9,
        }),
        Some(Fisico::Fecha),
        Some(Fisico::Hora),
        Some(Fisico::FechaHora),
        Some(Fisico::Instante),
        Some(Fisico::Binario),
    ] {
        writeln!(s, "{f:?} -> {:?}", tipo_arrow(f.as_ref())).unwrap();
    }
    fijado("esquema", &s);
}
