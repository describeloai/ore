//! **Los tipos**: el nombre que Phoenix da a una columna (`UNSIGNED_INT`, `DECIMAL`, `INTEGER ARRAY`) y el
//! tipo de OOS que le toca (ficha 0071 §3.4).
//!
//! Lo que llega por el cable ya está decidido por Avatica y medido en X0:
//!
//! - los enteros (todos, también los `UNSIGNED_*`) llegan como `i64`: **sin signo es el de signo** (Phoenix los
//!   limita al máximo con signo);
//! - el `DECIMAL` llega **exacto como texto**: con `precision` y `escala` declaradas es `Decimal<p, s>`; sin
//!   ellas (`DECIMAL` a secas) la escala es de cada valor y viaja como `String` exacto, como el `decimal` de
//!   Cassandra;
//! - `TIMESTAMP` llega en **milisegundos** (los nanosegundos no llegan); `DATE`, en **días** (la hora no llega),
//!   así que el conector lo pide con `CAST(col AS TIMESTAMP)` y es un instante; `TIME`, en milisegundos del día.

use ore_base::tipos::Fisico;

/// El tipo de OOS de una columna de Phoenix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tipo {
    /// `VARCHAR`, `CHAR`. `String`.
    Texto,
    /// `TINYINT` … `BIGINT` y los `UNSIGNED_*`. `Integer`.
    Entero,
    /// `FLOAT`, `DOUBLE` (y sus `UNSIGNED_*`). `Float`.
    Real,
    Logico,
    /// `TIMESTAMP` (milisegundos) y, por defecto, `DATE` (que en Phoenix lleva fecha y hora): `DateTimeTz`.
    Instante,
    /// Solo por una declaración de la tabla (`DateTime`, sin zona): la hora de pared, en UTC.
    FechaHora,
    /// Solo por una declaración sobre un `DATE` (`Date`): el día, sin la hora.
    Fecha,
    /// `TIME`: milisegundos del día en Phoenix, microsegundos en OOS.
    Hora,
    /// `DECIMAL(p, s)`.
    Decimal {
        precision: u8,
        escala: u8,
    },
    /// `DECIMAL` sin precisión, con la escala de **cada** valor: `String` exacto.
    DecimalLibre,
    /// `BINARY`, `VARBINARY`: `Opaque`, en base64.
    Binario,
    /// `T ARRAY`: `String` con JSON.
    Lista,
    /// Un tipo que este conector no sabe leer (`BSON`, `JSON`…): se cataloga, no se lee.
    Otro(String),
}

impl Tipo {
    /// El tipo de una columna, por su `TYPE_NAME` de Phoenix y su precisión y escala (`COLUMN_SIZE` y
    /// `DECIMAL_DIGITS` de `getColumns`).
    pub fn de(nombre: &str, precision: u32, escala: u32) -> Tipo {
        let n = nombre.trim().to_ascii_uppercase();
        if n.ends_with(" ARRAY") {
            return Tipo::Lista;
        }
        match n.as_str() {
            "TINYINT" | "SMALLINT" | "INTEGER" | "BIGINT" | "UNSIGNED_TINYINT"
            | "UNSIGNED_SMALLINT" | "UNSIGNED_INT" | "UNSIGNED_LONG" => Tipo::Entero,
            "FLOAT" | "DOUBLE" | "UNSIGNED_FLOAT" | "UNSIGNED_DOUBLE" => Tipo::Real,
            "BOOLEAN" => Tipo::Logico,
            "VARCHAR" | "CHAR" => Tipo::Texto,
            "BINARY" | "VARBINARY" | "VARBINARY_ENCODED" => Tipo::Binario,
            "TIMESTAMP" | "UNSIGNED_TIMESTAMP" | "DATE" | "UNSIGNED_DATE" => Tipo::Instante,
            "TIME" | "UNSIGNED_TIME" => Tipo::Hora,
            // Phoenix admite hasta 38 dígitos; sin precisión es el de escala libre.
            "DECIMAL" if (1..=38).contains(&precision) && escala <= precision => Tipo::Decimal {
                precision: precision as u8,
                escala: escala as u8,
            },
            "DECIMAL" => Tipo::DecimalLibre,
            otro => Tipo::Otro(otro.to_string()),
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
            Tipo::Hora => Fisico::Hora,
            Tipo::Decimal { precision, escala } => Fisico::Decimal {
                precision: *precision,
                escala: *escala,
            },
            Tipo::Texto | Tipo::DecimalLibre | Tipo::Binario | Tipo::Lista | Tipo::Otro(_) => {
                Fisico::Texto
            }
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
            Tipo::Hora => "Time".into(),
            Tipo::Decimal { precision, escala } => format!("Decimal<{precision}, {escala}>"),
            Tipo::Binario => "Opaque".into(),
            _ => "String".into(),
        }
    }

    /// **Lo que la tabla declara**, si el dato lo puede llevar: un `DECIMAL` libre pasa a `Decimal<p, s>`, un
    /// `TIMESTAMP` a `DateTime`, un `DATE` a `Date`. Si no, es un error que nombra lo declarado, nunca una
    /// conversión a escondidas (0069 M3).
    ///
    /// `es_date`: la columna es un `DATE` de Phoenix. Solo a él se le puede declarar `Date`: un `TIMESTAMP`
    /// declarado `Date` tiraría la hora, que es justo lo que no se hace a escondidas.
    pub fn declarado(&self, declarado: &str, es_date: bool) -> Result<Tipo, String> {
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
            (Fisico::Texto, Type::Scalar(s), _) if s == "String" => Ok(self.clone()),
            (Fisico::Entero, _, Tipo::Entero) => Ok(Tipo::Entero),
            (Fisico::Real, _, Tipo::Real | Tipo::Entero) => Ok(Tipo::Real),
            (Fisico::Logico, _, Tipo::Logico) => Ok(Tipo::Logico),
            (
                Fisico::Decimal { precision, escala },
                _,
                Tipo::Decimal { .. } | Tipo::DecimalLibre | Tipo::Entero,
            ) => Ok(Tipo::Decimal { precision, escala }),
            (Fisico::Fecha, _, Tipo::Instante) if es_date => Ok(Tipo::Fecha),
            (Fisico::Instante, _, Tipo::Instante) => Ok(Tipo::Instante),
            (Fisico::FechaHora, _, Tipo::Instante) => Ok(Tipo::FechaHora),
            (Fisico::Hora, _, Tipo::Hora) => Ok(Tipo::Hora),
            (Fisico::Texto, Type::Scalar(s), _) => Err(format!(
                "la tabla declara `{declarado}` ({s}) y este origen no lo lee: usa `String`"
            )),
            _ => no(),
        }
    }

    /// Lo que es el dato, para un mensaje de error.
    pub fn descripcion(&self) -> &'static str {
        match self {
            Tipo::Texto => "varchar",
            Tipo::Entero => "un entero",
            Tipo::Real => "float/double",
            Tipo::Logico => "boolean",
            Tipo::Instante | Tipo::FechaHora => "timestamp/date",
            Tipo::Fecha => "date",
            Tipo::Hora => "time",
            Tipo::Decimal { .. } | Tipo::DecimalLibre => "decimal",
            Tipo::Binario => "binary/varbinary",
            Tipo::Lista => "un array",
            Tipo::Otro(_) => "un tipo que este origen no lee",
        }
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn los_tipos_de_x0() {
        for (n, t) in [
            ("VARCHAR", Tipo::Texto),
            ("CHAR", Tipo::Texto),
            ("TINYINT", Tipo::Entero),
            ("SMALLINT", Tipo::Entero),
            ("INTEGER", Tipo::Entero),
            ("BIGINT", Tipo::Entero),
            ("UNSIGNED_INT", Tipo::Entero),
            ("UNSIGNED_LONG", Tipo::Entero),
            ("FLOAT", Tipo::Real),
            ("DOUBLE", Tipo::Real),
            ("BOOLEAN", Tipo::Logico),
            ("BINARY", Tipo::Binario),
            ("VARBINARY", Tipo::Binario),
            ("DATE", Tipo::Instante),
            ("TIME", Tipo::Hora),
            ("TIMESTAMP", Tipo::Instante),
            ("UNSIGNED_TIMESTAMP", Tipo::Instante),
            ("INTEGER ARRAY", Tipo::Lista),
            ("VARCHAR ARRAY", Tipo::Lista),
            ("varchar", Tipo::Texto),
        ] {
            assert_eq!(Tipo::de(n, 0, 0), t, "{n}");
        }
        assert_eq!(Tipo::de("BSON", 0, 0), Tipo::Otro("BSON".into()));
    }

    #[test]
    fn un_decimal_con_precision_es_exacto_y_sin_ella_es_texto() {
        assert_eq!(
            Tipo::de("DECIMAL", 12, 3),
            Tipo::Decimal {
                precision: 12,
                escala: 3
            }
        );
        assert_eq!(
            Tipo::de("DECIMAL", 38, 0),
            Tipo::Decimal {
                precision: 38,
                escala: 0
            }
        );
        assert_eq!(Tipo::de("DECIMAL", 0, 0), Tipo::DecimalLibre);
        // Más de 38 dígitos no cabe en un decimal128 de Arrow.
        assert_eq!(Tipo::de("DECIMAL", 39, 2), Tipo::DecimalLibre);
        // Una escala mayor que la precisión no es un `Decimal<p, s>` válido.
        assert_eq!(Tipo::de("DECIMAL", 5, 7), Tipo::DecimalLibre);
    }

    #[test]
    fn el_tipo_de_oos_y_su_fisico() {
        assert_eq!(Tipo::Instante.oos(), "DateTimeTz");
        assert_eq!(Tipo::Binario.oos(), "Opaque");
        assert_eq!(Tipo::Lista.oos(), "String");
        assert_eq!(Tipo::DecimalLibre.oos(), "String");
        assert_eq!(
            Tipo::Decimal {
                precision: 12,
                escala: 2
            }
            .oos(),
            "Decimal<12, 2>"
        );
        assert_eq!(Tipo::Hora.fisico(), Fisico::Hora);
        assert_eq!(Tipo::Lista.fisico(), Fisico::Texto);
    }

    #[test]
    fn lo_declarado_manda_si_el_dato_lo_lleva() {
        assert_eq!(
            Tipo::DecimalLibre.declarado("Decimal<12, 2>", false),
            Ok(Tipo::Decimal {
                precision: 12,
                escala: 2
            })
        );
        assert_eq!(
            Tipo::Instante.declarado("DateTime", false),
            Ok(Tipo::FechaHora)
        );
        assert_eq!(
            Tipo::Instante.declarado("DateTimeTz", false),
            Ok(Tipo::Instante)
        );
        assert_eq!(Tipo::Texto.declarado("String", false), Ok(Tipo::Texto));
        assert_eq!(Tipo::Entero.declarado("Float", false), Ok(Tipo::Real));
        assert!(Tipo::Texto.declarado("Integer", false).is_err());
        assert!(Tipo::Entero.declarado("Boolean", false).is_err());
        assert!(Tipo::Binario.declarado("String", false).is_ok());
    }

    #[test]
    fn date_solo_se_declara_sobre_un_date_de_phoenix() {
        // Un `TIMESTAMP` declarado `Date` tiraría la hora: no se hace a escondidas.
        let e = Tipo::Instante.declarado("Date", false).unwrap_err();
        assert!(e.contains("`Date`"), "{e}");
        assert_eq!(Tipo::Instante.declarado("Date", true), Ok(Tipo::Fecha));
    }
}
