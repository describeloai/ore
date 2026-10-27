//! **Los tipos**: el nombre que Oracle da a una columna (`NUMBER`, `TIMESTAMP(6) WITH TIME ZONE`,
//! `INTERVAL DAY(2) TO SECOND(6)`) y el tipo de OOS que le toca (ficha 0075).
//!
//! El nombre y la precisión salen de `ALL_TAB_COLUMNS` (`DATA_TYPE`, `DATA_PRECISION`, `DATA_SCALE`), no de
//! los metadatos de un resultado: es el catálogo quien dice lo que la columna *es*. Medido el 2026-10-25 contra
//! Oracle 26ai Free con `oracledb`:
//!
//! - `NUMBER` llega **exacto**. Con precisión y escala declaradas es `Decimal<p, s>`; sin ellas (`NUMBER` a
//!   secas, hasta 38 dígitos con la escala de cada valor) viaja como texto exacto, como el `decimal` de
//!   Cassandra. `NUMBER(p, 0)` con `p ≤ 18` cabe en un `i64`: `Integer`.
//! - `DATE` **lleva hora** (segundos): es `DateTime`, no `Date`.
//! - `TIMESTAMP WITH TIME ZONE` y `WITH LOCAL TIME ZONE` son un **instante**: `DateTimeTz`.
//! - `INTERVAL`, `RAW`, `BLOB`, `JSON` son `Opaque` (como `interval`, `bytea` y `json` de PostgreSQL): el
//!   sistema de tipos no los modela todavía y se leen como texto (el binario, en base64).
//! - `XMLTYPE`, `VECTOR`, `BFILE` y los tipos de objeto se catalogan y **no se leen**: hacerlo exige envolver
//!   la columna en el `SELECT` (`XMLSERIALIZE`) y eso es de otra capa.

use ore_base::tipos::Fisico;

/// El tipo de OOS de una columna de Oracle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tipo {
    /// `CHAR`, `VARCHAR2`, `NCHAR`, `NVARCHAR2`, `CLOB`, `NCLOB`, `LONG`, `ROWID`, `UROWID`. `String`.
    Texto,
    /// `NUMBER(p, 0)` con `p ≤ 18`. `Integer`.
    Entero,
    /// `FLOAT`, `BINARY_FLOAT`, `BINARY_DOUBLE`. `Float`.
    Real,
    /// `BOOLEAN` (23ai).
    Logico,
    /// `TIMESTAMP WITH TIME ZONE` y `WITH LOCAL TIME ZONE`: un instante, `DateTimeTz`.
    Instante,
    /// `DATE` (lleva hora) y `TIMESTAMP`: la hora de pared, sin zona. `DateTime`.
    FechaHora,
    /// Solo por una declaración de la tabla (`Date`) sobre un `DATE` o `TIMESTAMP`: el día. Una fila con hora
    /// distinta de medianoche **no cabe** y es un error: la hora no se tira a escondidas.
    Fecha,
    /// `NUMBER(p, s)`.
    Decimal { precision: u8, escala: u8 },
    /// `NUMBER` sin precisión, o que no cabe en 38 dígitos: `String` exacto.
    DecimalLibre,
    /// `RAW`, `LONG RAW`, `BLOB`: `Binary` (v1alpha35), sus bytes.
    Binario,
    /// Solo por una declaración de la tabla (`String` u `Opaque`) sobre un `RAW`/`BLOB`: los mismos bytes **como
    /// texto base64**, que es lo que este origen daba antes de `Binary`. Una tabla ya declarada así sigue leyendo
    /// lo mismo (B0: no se migra nada por detrás); el catálogo nunca la propone.
    BinarioTexto,
    /// `INTERVAL YEAR TO MONTH` y `INTERVAL DAY TO SECOND`: `Opaque`, en ISO 8601 (`P12Y3M`).
    Intervalo,
    /// `JSON`: `Opaque`, su texto.
    Documento,
    /// Un tipo que este conector no sabe leer: se cataloga, no se lee.
    Otro(String),
}

/// Quita los paréntesis de un nombre de tipo: `TIMESTAMP(6) WITH TIME ZONE` es `TIMESTAMP WITH TIME ZONE`,
/// `INTERVAL DAY(2) TO SECOND(6)` es `INTERVAL DAY TO SECOND`.
fn sin_parentesis(nombre: &str) -> String {
    let mut fuera = String::with_capacity(nombre.len());
    let mut hondo = 0u32;
    for c in nombre.chars() {
        match c {
            '(' => hondo += 1,
            ')' => hondo = hondo.saturating_sub(1),
            c if hondo == 0 => fuera.push(c),
            _ => {}
        }
    }
    fuera.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl Tipo {
    /// El tipo de una columna, por su `DATA_TYPE` de Oracle y su `DATA_PRECISION` y `DATA_SCALE` (`None`: el
    /// catálogo los da nulos, como en un `NUMBER` a secas).
    pub fn de(nombre: &str, precision: Option<i32>, escala: Option<i32>) -> Tipo {
        let n = sin_parentesis(&nombre.trim().to_ascii_uppercase());
        match n.as_str() {
            "NUMBER" => Tipo::numero(precision, escala),
            // `FLOAT(b)` es un `NUMBER` con precisión binaria; Oracle guarda hasta 38 dígitos exactos, y aquí
            // es un `Float`: lo que el nombre promete.
            "FLOAT" | "BINARY_FLOAT" | "BINARY_DOUBLE" => Tipo::Real,
            "BOOLEAN" => Tipo::Logico,
            "CHAR" | "VARCHAR2" | "VARCHAR" | "NCHAR" | "NVARCHAR2" | "CLOB" | "NCLOB" | "LONG"
            | "ROWID" | "UROWID" => Tipo::Texto,
            "DATE" | "TIMESTAMP" => Tipo::FechaHora,
            "TIMESTAMP WITH TIME ZONE" | "TIMESTAMP WITH LOCAL TIME ZONE" => Tipo::Instante,
            "INTERVAL YEAR TO MONTH" | "INTERVAL DAY TO SECOND" => Tipo::Intervalo,
            "RAW" | "LONG RAW" | "BLOB" => Tipo::Binario,
            "JSON" => Tipo::Documento,
            otro => Tipo::Otro(otro.to_string()),
        }
    }

    /// `NUMBER(p, s)` según lo que el catálogo dice de él.
    ///
    /// - sin precisión ni escala: `NUMBER` libre, texto exacto;
    /// - escala 0: un entero de hasta 18 dígitos cabe en `i64`; de 19 a 38, `Decimal<p, 0>`; sin precisión
    ///   (`NUMBER(*, 0)`), `Decimal<38, 0>`;
    /// - escala positiva: `Decimal<p, s>`; si la escala pasa de la precisión (`NUMBER(3, 5)`: `0,00ddd`), la
    ///   precisión de Arrow tiene que ser al menos la escala;
    /// - escala negativa (`NUMBER(10, -2)`: redondea a centenas): un entero de `p - s` dígitos;
    /// - lo que pasa de 38 dígitos no cabe en un decimal128 de Arrow.
    fn numero(precision: Option<i32>, escala: Option<i32>) -> Tipo {
        let decimal = |p: i32, s: i32| {
            if (1..=38).contains(&p) && (0..=p).contains(&s) {
                Tipo::Decimal {
                    precision: p as u8,
                    escala: s as u8,
                }
            } else {
                Tipo::DecimalLibre
            }
        };
        match (precision, escala) {
            (None, None) => Tipo::DecimalLibre,
            (None, Some(0)) => decimal(38, 0),
            (None, Some(_)) => Tipo::DecimalLibre,
            (Some(p), Some(0)) if p <= 18 => Tipo::Entero,
            (Some(p), Some(s)) if s < 0 => decimal(p.saturating_sub(s), 0),
            (Some(p), Some(s)) => decimal(p.max(s), s),
            // Una precisión sin escala (lo que el catálogo da en un `FLOAT`) no es un `NUMBER` de punto fijo.
            (Some(_), None) => Tipo::DecimalLibre,
        }
    }

    /// El físico de Arrow que le toca (0032 §1).
    pub fn fisico(&self) -> Fisico {
        match self {
            Tipo::Entero => Fisico::Entero,
            Tipo::Real => Fisico::Real,
            Tipo::Logico => Fisico::Logico,
            Tipo::Instante => Fisico::Instante,
            Tipo::FechaHora => Fisico::FechaHora,
            Tipo::Fecha => Fisico::Fecha,
            Tipo::Decimal { precision, escala } => Fisico::Decimal {
                precision: *precision,
                escala: *escala,
            },
            Tipo::Texto
            | Tipo::DecimalLibre
            | Tipo::BinarioTexto
            | Tipo::Intervalo
            | Tipo::Documento
            | Tipo::Otro(_) => Fisico::Texto,
            Tipo::Binario => Fisico::Binario,
        }
    }

    /// El tipo de OOS, como lo escribe el catálogo (`type`).
    pub fn oos(&self) -> String {
        match self {
            Tipo::Entero => "Integer".into(),
            Tipo::Real => "Float".into(),
            Tipo::Logico => "Boolean".into(),
            Tipo::Instante => "DateTimeTz".into(),
            Tipo::FechaHora => "DateTime".into(),
            Tipo::Fecha => "Date".into(),
            Tipo::Decimal { precision, escala } => format!("Decimal<{precision}, {escala}>"),
            Tipo::Binario => "Binary".into(),
            Tipo::BinarioTexto | Tipo::Intervalo | Tipo::Documento => "Opaque".into(),
            _ => "String".into(),
        }
    }

    /// Si este conector sabe leer la columna (los `Otro` solo se catalogan).
    pub fn se_lee(&self) -> bool {
        !matches!(self, Tipo::Otro(_))
    }

    /// **Lo que la tabla declara**, si el dato lo puede llevar: un `NUMBER` libre pasa a `Decimal<p, s>`, un
    /// entero a `Float`. Si no, es un error que nombra lo declarado, nunca una conversión a escondidas.
    pub fn declarado(&self, declarado: &str) -> Result<Tipo, String> {
        use ore_base::types::{Type, parse_type};
        let t = parse_type(declarado)
            .map_err(|e| format!("`{declarado}` no es un tipo de OOS: {e:?}"))?;
        let no = || {
            Err(format!(
                "la tabla declara `{declarado}` y el dato es {}",
                self.descripcion()
            ))
        };
        match (Fisico::de(&t), &t, self) {
            // Un `RAW`/`BLOB` declarado `String` u `Opaque` (lo de antes de `Binary`): los bytes como texto base64.
            (Fisico::Texto, Type::Scalar(s), Tipo::Binario | Tipo::BinarioTexto)
                if s == "String" || s == "Opaque" =>
            {
                Ok(Tipo::BinarioTexto)
            }
            (Fisico::Binario, _, Tipo::Binario | Tipo::BinarioTexto) => Ok(Tipo::Binario),
            (Fisico::Texto, Type::Scalar(s), _) if s == "String" => Ok(self.clone()),
            (Fisico::Entero, _, Tipo::Entero) => Ok(Tipo::Entero),
            // Un `NUMBER(19, 0)`, `NUMBER(38, 0)` o `NUMBER` libre que la tabla dice entero: cada valor se
            // comprueba al leer (uno con decimales, o que desborda un `i64`, es un error con su columna y fila).
            (Fisico::Entero, _, Tipo::Decimal { escala: 0, .. } | Tipo::DecimalLibre) => {
                Ok(Tipo::Entero)
            }
            (Fisico::Real, _, Tipo::Real | Tipo::Entero) => Ok(Tipo::Real),
            (Fisico::Logico, _, Tipo::Logico) => Ok(Tipo::Logico),
            (
                Fisico::Decimal { precision, escala },
                _,
                Tipo::Decimal { .. } | Tipo::DecimalLibre | Tipo::Entero,
            ) => Ok(Tipo::Decimal { precision, escala }),
            (Fisico::Instante, _, Tipo::Instante) => Ok(Tipo::Instante),
            (Fisico::FechaHora, _, Tipo::FechaHora) => Ok(Tipo::FechaHora),
            // El día de un `DATE`/`TIMESTAMP`: cada valor se comprueba al leer.
            (Fisico::Fecha, _, Tipo::FechaHora | Tipo::Fecha) => Ok(Tipo::Fecha),
            (Fisico::Texto, Type::Scalar(s), _) => Err(format!(
                "la tabla declara `{declarado}` ({s}) y este origen no lo lee: usa `String`"
            )),
            _ => no(),
        }
    }

    /// Lo que es el dato, para un mensaje de error.
    pub fn descripcion(&self) -> &'static str {
        match self {
            Tipo::Texto => "varchar2/clob",
            Tipo::Entero => "un entero",
            Tipo::Real => "float/binary_double",
            Tipo::Logico => "boolean",
            Tipo::Instante => "timestamp with time zone",
            Tipo::FechaHora | Tipo::Fecha => "date/timestamp",
            Tipo::Decimal { .. } | Tipo::DecimalLibre => "number",
            Tipo::Binario | Tipo::BinarioTexto => "raw/blob",
            Tipo::Intervalo => "interval",
            Tipo::Documento => "json",
            Tipo::Otro(_) => "un tipo que este origen no lee",
        }
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// Cada tipo de la sonda del 2026-10-25, como lo da `ALL_TAB_COLUMNS`.
    #[test]
    fn los_tipos_de_la_sonda() {
        let d = |precision, escala| Tipo::Decimal { precision, escala };
        for (n, p, s, t) in [
            ("NUMBER", None, None, Tipo::DecimalLibre),
            ("NUMBER", Some(18), Some(0), Tipo::Entero),
            ("NUMBER", Some(38), Some(0), d(38, 0)),
            ("NUMBER", Some(12), Some(2), d(12, 2)),
            ("NUMBER", Some(10), Some(-2), d(12, 0)),
            ("FLOAT", Some(126), None, Tipo::Real),
            ("BINARY_FLOAT", None, None, Tipo::Real),
            ("BINARY_DOUBLE", None, None, Tipo::Real),
            ("CHAR", None, None, Tipo::Texto),
            ("VARCHAR2", None, None, Tipo::Texto),
            ("NVARCHAR2", None, None, Tipo::Texto),
            ("CLOB", None, None, Tipo::Texto),
            ("DATE", None, None, Tipo::FechaHora),
            ("TIMESTAMP(6)", None, None, Tipo::FechaHora),
            ("TIMESTAMP(9)", None, None, Tipo::FechaHora),
            ("TIMESTAMP(6) WITH TIME ZONE", None, None, Tipo::Instante),
            (
                "TIMESTAMP(6) WITH LOCAL TIME ZONE",
                None,
                None,
                Tipo::Instante,
            ),
            ("INTERVAL YEAR(4) TO MONTH", None, None, Tipo::Intervalo),
            ("INTERVAL DAY(5) TO SECOND(6)", None, None, Tipo::Intervalo),
            ("RAW", None, None, Tipo::Binario),
            ("BLOB", None, None, Tipo::Binario),
            ("JSON", None, None, Tipo::Documento),
            ("BOOLEAN", None, None, Tipo::Logico),
            ("varchar2", None, None, Tipo::Texto),
        ] {
            assert_eq!(Tipo::de(n, p, s), t, "{n} {p:?} {s:?}");
        }
        for otro in ["XMLTYPE", "VECTOR", "BFILE", "SDO_GEOMETRY", "ANYDATA"] {
            assert_eq!(
                Tipo::de(otro, None, None),
                Tipo::Otro(otro.into()),
                "{otro}"
            );
        }
    }

    #[test]
    fn un_number_se_ajusta_a_lo_que_cabe_en_arrow() {
        // NUMBER(*, 0): sin precisión, escala 0.
        assert_eq!(
            Tipo::de("NUMBER", None, Some(0)),
            Tipo::Decimal {
                precision: 38,
                escala: 0
            }
        );
        // La escala pasa de la precisión: 0,00ddd. Arrow quiere precisión ≥ escala.
        assert_eq!(
            Tipo::de("NUMBER", Some(3), Some(5)),
            Tipo::Decimal {
                precision: 5,
                escala: 5
            }
        );
        // Una escala negativa que no cabe en 38 dígitos es texto.
        assert_eq!(Tipo::de("NUMBER", Some(38), Some(-5)), Tipo::DecimalLibre);
        // Sin precisión y con escala: la escala es de cada valor.
        assert_eq!(Tipo::de("NUMBER", None, Some(2)), Tipo::DecimalLibre);
    }

    #[test]
    fn el_tipo_de_oos_y_su_fisico() {
        assert_eq!(Tipo::Instante.oos(), "DateTimeTz");
        assert_eq!(Tipo::FechaHora.oos(), "DateTime");
        assert_eq!(Tipo::Binario.oos(), "Binary");
        assert_eq!(Tipo::Binario.fisico(), Fisico::Binario);
        assert_eq!(Tipo::BinarioTexto.oos(), "Opaque");
        assert_eq!(Tipo::BinarioTexto.fisico(), Fisico::Texto);
        assert_eq!(Tipo::Intervalo.oos(), "Opaque");
        assert_eq!(Tipo::Documento.oos(), "Opaque");
        assert_eq!(Tipo::DecimalLibre.oos(), "String");
        assert_eq!(
            Tipo::Decimal {
                precision: 12,
                escala: 2
            }
            .oos(),
            "Decimal<12, 2>"
        );
        assert_eq!(Tipo::Intervalo.fisico(), Fisico::Texto);
        assert_eq!(Tipo::FechaHora.fisico(), Fisico::FechaHora);
        assert_eq!(Tipo::Instante.fisico(), Fisico::Instante);
        assert!(Tipo::Binario.se_lee() && !Tipo::Otro("XMLTYPE".into()).se_lee());
    }

    #[test]
    fn lo_declarado_manda_si_el_dato_lo_lleva() {
        assert_eq!(
            Tipo::DecimalLibre.declarado("Decimal<12, 2>"),
            Ok(Tipo::Decimal {
                precision: 12,
                escala: 2
            })
        );
        assert_eq!(Tipo::Entero.declarado("Float"), Ok(Tipo::Real));
        // Un `NUMBER(19, 0)` declarado entero: vale; con decimales, no.
        let d19 = Tipo::de("NUMBER", Some(19), Some(0));
        assert_eq!(d19.declarado("Integer"), Ok(Tipo::Entero));
        assert_eq!(Tipo::DecimalLibre.declarado("Integer"), Ok(Tipo::Entero));
        assert!(
            Tipo::de("NUMBER", Some(12), Some(2))
                .declarado("Integer")
                .is_err()
        );
        assert_eq!(Tipo::Texto.declarado("String"), Ok(Tipo::Texto));
        // `Binary` es lo que propone el catálogo; `String` y `Opaque` (lo de antes) siguen valiendo, como texto base64.
        assert_eq!(Tipo::Binario.declarado("Binary"), Ok(Tipo::Binario));
        assert_eq!(Tipo::Binario.declarado("String"), Ok(Tipo::BinarioTexto));
        assert_eq!(Tipo::Binario.declarado("Opaque"), Ok(Tipo::BinarioTexto));
        assert!(Tipo::Binario.declarado("Integer").is_err());
        assert!(Tipo::Texto.declarado("Binary").is_err());
        assert!(Tipo::Texto.declarado("Integer").is_err());
        assert!(Tipo::Entero.declarado("Boolean").is_err());
        // Un `DATE` declarado `Date`: vale, y cada valor con hora se rechaza al leer (no a escondidas).
        assert_eq!(Tipo::FechaHora.declarado("Date"), Ok(Tipo::Fecha));
        assert_eq!(Tipo::Fecha.fisico(), Fisico::Fecha);
        assert_eq!(Tipo::Fecha.oos(), "Date");
        // Pero un texto no es una fecha.
        assert!(Tipo::Texto.declarado("Date").is_err());
    }
}
