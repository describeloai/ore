//! **Los tipos**: el texto de CQL que `system_schema` guarda (`frozen<list<int>>`),
//! su árbol ([`Cql`]) y el tipo de OOS que le toca ([`Tipo`], ficha 0070 §4.3).
//!
//! El tipo exacto sale de `system_schema.columns.type`, **no** de los metadatos
//! de un resultado: el driver devuelve `frozen: false` para un
//! `frozen<list<int>>` (medido en X0).

use ore_core::tipos::Fisico;
use std::fmt;

/// Un tipo de CQL, tal como lo escribe `system_schema`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cql {
    Ascii,
    Text,
    TinyInt,
    SmallInt,
    Int,
    BigInt,
    Counter,
    Varint,
    Float,
    Double,
    Decimal,
    Boolean,
    Blob,
    Inet,
    Timestamp,
    Date,
    Time,
    Duration,
    Uuid,
    TimeUuid,
    List(Box<Cql>),
    Set(Box<Cql>),
    Map(Box<Cql>, Box<Cql>),
    Tuple(Vec<Cql>),
    /// Un tipo definido por el usuario: su nombre.
    Udt(String),
    /// `vector<T, n>` (Cassandra 5.0).
    Vector(Box<Cql>, u32),
    Frozen(Box<Cql>),
}

impl Cql {
    /// Lee el texto de `system_schema.columns.type`.
    pub fn parse(texto: &str) -> Result<Cql, String> {
        let mut p = Lector {
            b: texto.as_bytes(),
            i: 0,
        };
        let t = p.tipo()?;
        p.espacio();
        if p.i != p.b.len() {
            return Err(format!("`{texto}`: sobra texto tras el tipo"));
        }
        Ok(t)
    }

    /// Sin el `frozen<…>` de fuera: no cambia lo que se lee.
    pub fn sin_frozen(&self) -> &Cql {
        match self {
            Cql::Frozen(t) => t.sin_frozen(),
            otro => otro,
        }
    }
}

impl fmt::Display for Cql {
    /// El texto de CQL, como lo escribe `system_schema`: lo que el catálogo cita
    /// como `physicalType`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Cql::Ascii => f.write_str("ascii"),
            Cql::Text => f.write_str("text"),
            Cql::TinyInt => f.write_str("tinyint"),
            Cql::SmallInt => f.write_str("smallint"),
            Cql::Int => f.write_str("int"),
            Cql::BigInt => f.write_str("bigint"),
            Cql::Counter => f.write_str("counter"),
            Cql::Varint => f.write_str("varint"),
            Cql::Float => f.write_str("float"),
            Cql::Double => f.write_str("double"),
            Cql::Decimal => f.write_str("decimal"),
            Cql::Boolean => f.write_str("boolean"),
            Cql::Blob => f.write_str("blob"),
            Cql::Inet => f.write_str("inet"),
            Cql::Timestamp => f.write_str("timestamp"),
            Cql::Date => f.write_str("date"),
            Cql::Time => f.write_str("time"),
            Cql::Duration => f.write_str("duration"),
            Cql::Uuid => f.write_str("uuid"),
            Cql::TimeUuid => f.write_str("timeuuid"),
            Cql::List(t) => write!(f, "list<{t}>"),
            Cql::Set(t) => write!(f, "set<{t}>"),
            Cql::Map(k, v) => write!(f, "map<{k}, {v}>"),
            Cql::Tuple(v) => {
                f.write_str("tuple<")?;
                for (i, t) in v.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{t}")?;
                }
                f.write_str(">")
            }
            Cql::Udt(n) => {
                let simple = n
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
                    && !n.is_empty();
                if simple {
                    f.write_str(n)
                } else {
                    write!(f, "\"{n}\"")
                }
            }
            Cql::Vector(t, n) => write!(f, "vector<{t}, {n}>"),
            Cql::Frozen(t) => write!(f, "frozen<{t}>"),
        }
    }
}

enum Arg {
    T(Cql),
    N(u32),
}

struct Lector<'a> {
    b: &'a [u8],
    i: usize,
}

impl Lector<'_> {
    fn espacio(&mut self) {
        while self.b.get(self.i).is_some_and(u8::is_ascii_whitespace) {
            self.i += 1;
        }
    }

    fn ver(&mut self) -> Option<u8> {
        self.espacio();
        self.b.get(self.i).copied()
    }

    /// Un identificador, y si iba entre comillas.
    fn palabra(&mut self) -> Result<(String, bool), String> {
        self.espacio();
        if self.b.get(self.i) == Some(&b'"') {
            let ini = self.i + 1;
            let mut j = ini;
            while j < self.b.len() && self.b[j] != b'"' {
                j += 1;
            }
            if j >= self.b.len() {
                return Err("un identificador entre comillas sin cerrar".into());
            }
            self.i = j + 1;
            return Ok((String::from_utf8_lossy(&self.b[ini..j]).into_owned(), true));
        }
        let ini = self.i;
        while self
            .b
            .get(self.i)
            .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_' || *c == b'.')
        {
            self.i += 1;
        }
        if ini == self.i {
            return Err(format!("se esperaba un tipo en la posición {ini}"));
        }
        Ok((
            String::from_utf8_lossy(&self.b[ini..self.i]).into_owned(),
            false,
        ))
    }

    fn args(&mut self) -> Result<Vec<Arg>, String> {
        let mut v = Vec::new();
        loop {
            self.espacio();
            if self.b.get(self.i).is_some_and(u8::is_ascii_digit) {
                let ini = self.i;
                while self.b.get(self.i).is_some_and(u8::is_ascii_digit) {
                    self.i += 1;
                }
                let n = std::str::from_utf8(&self.b[ini..self.i])
                    .unwrap_or("0")
                    .parse()
                    .map_err(|_| "una dimensión que no cabe".to_string())?;
                v.push(Arg::N(n));
            } else {
                v.push(Arg::T(self.tipo()?));
            }
            match self.ver() {
                Some(b',') => self.i += 1,
                Some(b'>') => {
                    self.i += 1;
                    return Ok(v);
                }
                _ => return Err("se esperaba `,` o `>`".into()),
            }
        }
    }

    fn tipo(&mut self) -> Result<Cql, String> {
        let (nombre, entre_comillas) = self.palabra()?;
        let hijos = if self.ver() == Some(b'<') {
            self.i += 1;
            Some(self.args()?)
        } else {
            None
        };
        let uno = |h: Option<Vec<Arg>>| -> Result<Cql, String> {
            match h.and_then(|mut v| (v.len() == 1).then(|| v.remove(0))) {
                Some(Arg::T(t)) => Ok(t),
                _ => Err(format!("`{nombre}` lleva un solo tipo")),
            }
        };
        let simple = |t: Cql, h: &Option<Vec<Arg>>| {
            if h.is_some() {
                Err(format!("`{nombre}` no lleva parámetros"))
            } else {
                Ok(t)
            }
        };
        // Un identificador entre comillas es un UDT, aunque se llame `text`.
        let clave = if entre_comillas {
            String::new()
        } else {
            nombre.to_ascii_lowercase()
        };
        match clave.as_str() {
            "ascii" => simple(Cql::Ascii, &hijos),
            "text" | "varchar" => simple(Cql::Text, &hijos),
            "tinyint" => simple(Cql::TinyInt, &hijos),
            "smallint" => simple(Cql::SmallInt, &hijos),
            "int" => simple(Cql::Int, &hijos),
            "bigint" => simple(Cql::BigInt, &hijos),
            "counter" => simple(Cql::Counter, &hijos),
            "varint" => simple(Cql::Varint, &hijos),
            "float" => simple(Cql::Float, &hijos),
            "double" => simple(Cql::Double, &hijos),
            "decimal" => simple(Cql::Decimal, &hijos),
            "boolean" => simple(Cql::Boolean, &hijos),
            "blob" => simple(Cql::Blob, &hijos),
            "inet" => simple(Cql::Inet, &hijos),
            "timestamp" => simple(Cql::Timestamp, &hijos),
            "date" => simple(Cql::Date, &hijos),
            "time" => simple(Cql::Time, &hijos),
            "duration" => simple(Cql::Duration, &hijos),
            "uuid" => simple(Cql::Uuid, &hijos),
            "timeuuid" => simple(Cql::TimeUuid, &hijos),
            "list" => Ok(Cql::List(Box::new(uno(hijos)?))),
            "set" => Ok(Cql::Set(Box::new(uno(hijos)?))),
            "frozen" => Ok(Cql::Frozen(Box::new(uno(hijos)?))),
            "map" => match hijos {
                Some(mut v) if v.len() == 2 => match (v.remove(0), v.remove(0)) {
                    (Arg::T(k), Arg::T(x)) => Ok(Cql::Map(Box::new(k), Box::new(x))),
                    _ => Err("`map` lleva dos tipos".into()),
                },
                _ => Err("`map` lleva dos tipos".into()),
            },
            "tuple" => match hijos {
                Some(v) => v
                    .into_iter()
                    .map(|a| match a {
                        Arg::T(t) => Ok(t),
                        Arg::N(_) => Err("`tuple` lleva tipos".to_string()),
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map(Cql::Tuple),
                None => Err("`tuple` lleva tipos".into()),
            },
            "vector" => match hijos {
                Some(mut v) if v.len() == 2 => match (v.remove(0), v.remove(0)) {
                    (Arg::T(t), Arg::N(n)) => Ok(Cql::Vector(Box::new(t), n)),
                    _ => Err("`vector` lleva un tipo y una dimensión".into()),
                },
                _ => Err("`vector` lleva un tipo y una dimensión".into()),
            },
            _ => simple(Cql::Udt(nombre.clone()), &hijos),
        }
    }
}

/// El tipo de OOS de una columna y cómo viaja (ficha 0070 §4.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tipo {
    /// `ascii`, `text`, `varchar`. `String`.
    Texto,
    /// `tinyint`, `smallint`, `int`, `bigint`, `counter`. `Integer`.
    Entero,
    /// `float`, `double`. `Float`.
    Real,
    Logico,
    /// `timestamp` (milisegundos): `DateTimeTz`.
    Instante,
    /// `date`.
    Fecha,
    /// Solo por una declaración de la tabla (`DateTime`, sin zona): la hora de
    /// pared, en UTC, de un `timestamp`.
    FechaHora,
    /// `time`: nanosegundos en CQL, microsegundos en OOS. Si el valor trae
    /// nanosegundos sueltos **no cabe**, y se dice (0032 §4).
    Hora,
    /// `Decimal<p, s>`: solo por una declaración de la tabla.
    Decimal {
        precision: u8,
        escala: u8,
    },
    /// `varint`, de cualquier tamaño: `String` con el entero exacto.
    Varint,
    /// `decimal`, con la escala de **cada** valor: `String` exacto (`1.2345`).
    DecimalLibre,
    /// `blob`: `Opaque`, en base64.
    Binario,
    /// `uuid`, `timeuuid`: `String`.
    Uuid,
    /// `duration`: `String` con la forma de CQL (`14mo3d14400000000000ns`).
    Duracion,
    /// `inet`: `String`.
    Inet,
    /// Colecciones, tuplas, UDT y `vector`: `String` con JSON.
    Json,
}

impl Tipo {
    pub fn de(t: &Cql) -> Tipo {
        match t.sin_frozen() {
            Cql::Ascii | Cql::Text => Tipo::Texto,
            Cql::TinyInt | Cql::SmallInt | Cql::Int | Cql::BigInt | Cql::Counter => Tipo::Entero,
            Cql::Float | Cql::Double => Tipo::Real,
            Cql::Boolean => Tipo::Logico,
            Cql::Timestamp => Tipo::Instante,
            Cql::Date => Tipo::Fecha,
            Cql::Time => Tipo::Hora,
            Cql::Varint => Tipo::Varint,
            Cql::Decimal => Tipo::DecimalLibre,
            Cql::Blob => Tipo::Binario,
            Cql::Uuid | Cql::TimeUuid => Tipo::Uuid,
            Cql::Duration => Tipo::Duracion,
            Cql::Inet => Tipo::Inet,
            Cql::List(_)
            | Cql::Set(_)
            | Cql::Map(..)
            | Cql::Tuple(_)
            | Cql::Udt(_)
            | Cql::Vector(..)
            | Cql::Frozen(_) => Tipo::Json,
        }
    }

    /// El físico de Arrow que le toca (0032 §1).
    pub fn fisico(&self) -> Fisico {
        match self {
            Tipo::Entero => Fisico::Entero,
            Tipo::Real => Fisico::Real,
            Tipo::Logico => Fisico::Logico,
            Tipo::Instante => Fisico::Instante,
            Tipo::Fecha => Fisico::Fecha,
            Tipo::FechaHora => Fisico::FechaHora,
            Tipo::Hora => Fisico::Hora,
            Tipo::Decimal { precision, escala } => Fisico::Decimal {
                precision: *precision,
                escala: *escala,
            },
            Tipo::Texto
            | Tipo::Varint
            | Tipo::DecimalLibre
            | Tipo::Binario
            | Tipo::Uuid
            | Tipo::Duracion
            | Tipo::Inet
            | Tipo::Json => Fisico::Texto,
        }
    }

    /// El tipo de OOS, como lo escribe el catálogo (`type`).
    pub fn oos(&self) -> String {
        match self {
            Tipo::Entero => "Integer".into(),
            Tipo::Real => "Float".into(),
            Tipo::Logico => "Boolean".into(),
            Tipo::Instante => "DateTimeTz".into(),
            Tipo::Fecha => "Date".into(),
            Tipo::FechaHora => "DateTime".into(),
            Tipo::Hora => "Time".into(),
            Tipo::Decimal { precision, escala } => format!("Decimal<{precision}, {escala}>"),
            Tipo::Binario => "Opaque".into(),
            _ => "String".into(),
        }
    }

    /// El tipo tal como lo dice el origen (`physicalType`).
    pub fn fisico_origen(&self) -> &'static str {
        match self {
            Tipo::Texto => "text",
            Tipo::Entero => "int/bigint",
            Tipo::Real => "double",
            Tipo::Logico => "boolean",
            Tipo::Instante | Tipo::FechaHora => "timestamp",
            Tipo::Fecha => "date",
            Tipo::Hora => "time",
            Tipo::Decimal { .. } | Tipo::DecimalLibre => "decimal",
            Tipo::Varint => "varint",
            Tipo::Binario => "blob",
            Tipo::Uuid => "uuid",
            Tipo::Duracion => "duration",
            Tipo::Inet => "inet",
            Tipo::Json => "collection/udt",
        }
    }

    /// Si un filtro de SQL significa lo mismo, escrito en CQL, sobre esta columna.
    /// Lo que no se compara como SQL (bytes, colecciones, duraciones, enteros
    /// sin tope, decimales de escala libre) **no se empuja**: lo evalúa el conector
    /// o se rechaza.
    pub fn empujable(&self) -> bool {
        !matches!(
            self,
            Tipo::Binario | Tipo::Json | Tipo::Duracion | Tipo::Varint | Tipo::DecimalLibre
        )
    }

    /// **Lo que la tabla declara**, si el dato lo puede llevar: un `varint` o un
    /// `decimal` pasan a `Integer` o `Decimal<p, s>`. Si no, es un error que
    /// nombra lo declarado, nunca una conversión a escondidas (como 0069 M3).
    pub fn declarado(&self, declarado: &str) -> Result<Tipo, String> {
        use ore_core::types::{Type, parse_type};
        let t = parse_type(declarado)
            .map_err(|e| format!("`{declarado}` no es un tipo de OOS: {e:?}"))?;
        let no = || {
            Err(format!(
                "la tabla declara `{declarado}` y el dato es {}",
                self.fisico_origen()
            ))
        };
        match (Fisico::de(&t), &t, self) {
            (Fisico::Texto, Type::Scalar(s), _) if s == "String" => Ok(self.clone()),
            (Fisico::Entero, _, Tipo::Entero | Tipo::Varint) => Ok(Tipo::Entero),
            (Fisico::Real, _, Tipo::Real | Tipo::Entero) => Ok(Tipo::Real),
            (Fisico::Logico, _, Tipo::Logico) => Ok(Tipo::Logico),
            (
                Fisico::Decimal { precision, escala },
                _,
                Tipo::Decimal { .. } | Tipo::DecimalLibre | Tipo::Varint | Tipo::Entero,
            ) => Ok(Tipo::Decimal { precision, escala }),
            (Fisico::Fecha, _, Tipo::Fecha) => Ok(Tipo::Fecha),
            (Fisico::Instante, _, Tipo::Instante) => Ok(Tipo::Instante),
            (Fisico::FechaHora, _, Tipo::Instante) => Ok(Tipo::FechaHora),
            (Fisico::Hora, _, Tipo::Hora) => Ok(Tipo::Hora),
            (Fisico::Texto, Type::Scalar(s), _) => Err(format!(
                "la tabla declara `{declarado}` ({s}) y este origen no lo lee: usa `String`"
            )),
            _ => no(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cql(s: &str) -> Cql {
        Cql::parse(s).unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    #[test]
    fn los_escalares_de_system_schema() {
        for (s, t) in [
            ("text", Tipo::Texto),
            ("varchar", Tipo::Texto),
            ("ascii", Tipo::Texto),
            ("tinyint", Tipo::Entero),
            ("smallint", Tipo::Entero),
            ("int", Tipo::Entero),
            ("bigint", Tipo::Entero),
            ("counter", Tipo::Entero),
            ("varint", Tipo::Varint),
            ("decimal", Tipo::DecimalLibre),
            ("float", Tipo::Real),
            ("double", Tipo::Real),
            ("boolean", Tipo::Logico),
            ("blob", Tipo::Binario),
            ("inet", Tipo::Inet),
            ("timestamp", Tipo::Instante),
            ("date", Tipo::Fecha),
            ("time", Tipo::Hora),
            ("duration", Tipo::Duracion),
            ("uuid", Tipo::Uuid),
            ("timeuuid", Tipo::Uuid),
        ] {
            assert_eq!(Tipo::de(&cql(s)), t, "{s}");
        }
    }

    #[test]
    fn lo_compuesto_es_json_y_el_frozen_no_cambia_el_escalar() {
        assert_eq!(cql("frozen<list<int>>").sin_frozen(), &cql("list<int>"));
        assert_eq!(Tipo::de(&cql("frozen<list<int>>")), Tipo::Json);
        assert_eq!(Tipo::de(&cql("frozen<int>")), Tipo::Entero);
        assert_eq!(
            cql("map<text, frozen<tuple<int, text>>>"),
            Cql::Map(
                Box::new(Cql::Text),
                Box::new(Cql::Frozen(Box::new(Cql::Tuple(vec![Cql::Int, Cql::Text]))))
            )
        );
        assert_eq!(
            cql("vector<float, 3>"),
            Cql::Vector(Box::new(Cql::Float), 3)
        );
        assert_eq!(Tipo::de(&cql("vector<float, 3>")), Tipo::Json);
    }

    #[test]
    fn un_nombre_que_no_es_nativo_es_un_udt_aunque_lleve_comillas() {
        assert_eq!(cql("direccion"), Cql::Udt("direccion".into()));
        assert_eq!(
            cql("frozen<\"Mi Tipo\">").sin_frozen(),
            &Cql::Udt("Mi Tipo".into())
        );
        assert_eq!(cql("\"text\""), Cql::Udt("text".into()));
        assert_eq!(Tipo::de(&cql("frozen<direccion>")), Tipo::Json);
    }

    #[test]
    fn el_texto_de_vuelta_es_el_de_system_schema() {
        for s in [
            "int",
            "frozen<list<int>>",
            "map<text, frozen<tuple<int, text>>>",
            "vector<float, 3>",
            "frozen<direccion>",
            "frozen<\"Mi Tipo\">",
            "set<uuid>",
        ] {
            assert_eq!(cql(s).to_string(), s);
        }
    }

    #[test]
    fn lo_mal_formado_se_dice() {
        for s in [
            "",
            "list<",
            "list<int",
            "map<text>",
            "int<text>",
            "vector<float>",
            "list<int> x",
            "tuple",
        ] {
            assert!(Cql::parse(s).is_err(), "`{s}` debería fallar");
        }
    }

    #[test]
    fn el_tipo_de_oos_y_su_fisico() {
        assert_eq!(Tipo::Hora.oos(), "Time");
        assert_eq!(Tipo::Binario.oos(), "Opaque");
        assert_eq!(Tipo::Varint.oos(), "String");
        assert_eq!(Tipo::Varint.fisico(), Fisico::Texto);
        assert_eq!(Tipo::Instante.fisico(), Fisico::Instante);
        assert!(!Tipo::Varint.empujable() && !Tipo::Json.empujable() && Tipo::Entero.empujable());
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
        assert_eq!(Tipo::Varint.declarado("Integer"), Ok(Tipo::Entero));
        assert_eq!(Tipo::Texto.declarado("String"), Ok(Tipo::Texto));
        assert!(Tipo::Texto.declarado("Integer").is_err());
        assert!(Tipo::Binario.declarado("String").is_ok());
        assert!(Tipo::Entero.declarado("Boolean").is_err());
        assert_eq!(Tipo::Instante.declarado("DateTime"), Ok(Tipo::FechaHora));
        assert_eq!(Tipo::FechaHora.oos(), "DateTime");
    }
}
