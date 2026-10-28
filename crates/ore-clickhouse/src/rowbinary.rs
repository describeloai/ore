//! **`RowBinaryWithNamesAndTypes`**: el texto de los tipos de ClickHouse, a qué físico de ORE va cada uno, y cómo se lee una celda.
//!
//! La cabecera de la respuesta trae los nombres y los tipos **de la propia consulta** (también si no hay filas), así que no
//! hace falta preguntar al catálogo antes de leer, y no hay carrera entre la pregunta y la lectura.
//!
//! Este módulo sabe los tipos que la semilla del kit usa y los enteros pequeños (paso 3 de la hoja de ruta, ADR 0077 §6).
//! Lo demás (`UInt64`, `Int128/256`, `Decimal` de más de 38 dígitos, `Time`, `Enum`, `Map`, `Tuple`…) es del paso 4 y **falla con
//! un error claro**: nunca se redondea ni se trunca.

use ore_arrow::{Columna, Escritor};
use ore_base::tipos::Fisico;
use std::io::{self, BufRead, Read};

/// Un tipo de ClickHouse que sabemos leer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tipo {
    /// `Int8…Int64` (con signo) y `UInt8…UInt32` (sin signo): caben en un `int64`.
    Entero {
        bytes: u8,
        signo: bool,
    },
    Real {
        bytes: u8,
    },
    Logico,
    Texto,
    Decimal {
        precision: u8,
        escala: u8,
    },
    /// `Date` (`UInt16`) y `Date32` (`Int32`): días desde 1970-01-01.
    Fecha {
        ancho: u8,
    },
    /// `DateTime` (segundos, `UInt32`) o `DateTime64(p)` (ticks de 10⁻ᵖ s, `Int64`): un instante.
    Instante {
        decimales: u8,
        ancho: u8,
        /// El tipo declarado de la tabla dice que es una hora de reloj sin zona (`DateTime`) y no un instante (`DateTimeTz`):
        /// en el servidor son el mismo `DateTime64(…, 'UTC')`, y los microsegundos son los mismos.
        pared: bool,
    },
    /// `UInt64`: no cabe en un `int64`, y sí exacto en un `decimal128(20, 0)`.
    Entero64SinSigno,
    /// `FixedString(N)`: N bytes, rellenos con ceros por la derecha.
    TextoFijo(usize),
    /// `Enum8/16('a' = 1, …)`: el texto es el nombre, que sólo viaja en el tipo.
    Enumerado {
        ancho: u8,
        nombres: Vec<(i16, String)>,
    },
    Uuid,
    Ipv4,
    Ipv6,
    Nulable(Box<Tipo>),
}

/// El texto de un tipo (`Nullable(Decimal(12, 2))`) a [`Tipo`]. Un tipo que no se sabe leer es un `Err` con su nombre.
pub fn analizar(texto: &str) -> Result<Tipo, String> {
    let t = texto.trim();
    if let Some(i) = envuelto(t, "Nullable") {
        return Ok(Tipo::Nulable(Box::new(analizar(i)?)));
    }
    // En RowBinary un `LowCardinality(T)` va como `T`: es una forma de almacenar, no de transmitir.
    if let Some(i) = envuelto(t, "LowCardinality") {
        return analizar(i);
    }
    let sin_signo = |n| Tipo::Entero {
        bytes: n,
        signo: false,
    };
    let con_signo = |n| Tipo::Entero {
        bytes: n,
        signo: true,
    };
    Ok(match t {
        "Int8" => con_signo(1),
        "Int16" => con_signo(2),
        "Int32" => con_signo(4),
        "Int64" => con_signo(8),
        "UInt8" => sin_signo(1),
        "UInt16" => sin_signo(2),
        "UInt32" => sin_signo(4),
        "UInt64" => Tipo::Entero64SinSigno,
        "UUID" => Tipo::Uuid,
        "IPv4" => Tipo::Ipv4,
        "IPv6" => Tipo::Ipv6,
        "Float32" => Tipo::Real { bytes: 4 },
        "Float64" => Tipo::Real { bytes: 8 },
        "Bool" => Tipo::Logico,
        "String" => Tipo::Texto,
        "Date" => Tipo::Fecha { ancho: 2 },
        "Date32" => Tipo::Fecha { ancho: 4 },
        "DateTime" => Tipo::Instante {
            decimales: 0,
            ancho: 4,
            pared: false,
        },
        _ => return analizar_con_argumentos(t),
    })
}

/// `Nombre(resto)` → `resto`.
fn envuelto<'a>(t: &'a str, nombre: &str) -> Option<&'a str> {
    t.strip_prefix(nombre)?
        .strip_prefix('(')?
        .strip_suffix(')')
        .map(str::trim)
}

fn analizar_con_argumentos(t: &str) -> Result<Tipo, String> {
    let no = || format!("el tipo `{t}` todavía no se sabe leer (ADR 0077, paso 4)");
    if let Some(a) = envuelto(t, "Decimal") {
        let (p, s) = a.split_once(',').ok_or_else(no)?;
        let (p, s): (u8, u8) = (
            p.trim().parse().map_err(|_| no())?,
            s.trim().parse().map_err(|_| no())?,
        );
        return decimal(t, p, s);
    }
    for (nombre, precision) in [("Decimal32", 9u8), ("Decimal64", 18), ("Decimal128", 38)] {
        if let Some(a) = envuelto(t, nombre) {
            return decimal(t, precision, a.parse().map_err(|_| no())?);
        }
    }
    if let Some(a) = envuelto(t, "DateTime64") {
        // `DateTime64(6)` o `DateTime64(6, 'UTC')`: un instante en el servidor siempre es UTC; la zona sólo cambia cómo se escribe.
        let p = a.split(',').next().unwrap_or("").trim();
        let decimales: u8 = p.parse().map_err(|_| no())?;
        if decimales > 9 {
            return Err(no());
        }
        return Ok(Tipo::Instante {
            decimales,
            ancho: 8,
            pared: false,
        });
    }
    // `DateTime('Europe/Madrid')`: lo almacenado son segundos UTC; la zona sólo cambia cómo se escribe.
    if envuelto(t, "DateTime").is_some() {
        return Ok(Tipo::Instante {
            decimales: 0,
            ancho: 4,
            pared: false,
        });
    }
    if let Some(a) = envuelto(t, "FixedString") {
        let n: usize = a.parse().map_err(|_| no())?;
        if n == 0 || n > MAX_FIJO {
            return Err(no());
        }
        return Ok(Tipo::TextoFijo(n));
    }
    for (nombre, ancho) in [("Enum8", 1u8), ("Enum16", 2)] {
        if let Some(a) = envuelto(t, nombre) {
            return Ok(Tipo::Enumerado {
                ancho,
                nombres: nombres_de_enum(a).ok_or_else(no)?,
            });
        }
    }
    Err(no())
}

/// El tope de un `FixedString`: más de eso no es un texto fijo sino un blob.
const MAX_FIJO: usize = 1 << 20;

/// `'a' = 1, 'b\'c' = -2` → `[(1, "a"), (-2, "b'c")]`. Los nombres pueden llevar comas, paréntesis y comillas escapadas.
fn nombres_de_enum(a: &str) -> Option<Vec<(i16, String)>> {
    let mut c = a.chars().peekable();
    let mut v = Vec::new();
    loop {
        while c.next_if(|x| x.is_whitespace()).is_some() {}
        if c.next()? != '\'' {
            return None;
        }
        let mut nombre = String::new();
        loop {
            match c.next()? {
                '\'' => break,
                '\\' => nombre.push(match c.next()? {
                    'n' => '\n',
                    't' => '\t',
                    '0' => '\0',
                    otro => otro,
                }),
                otro => nombre.push(otro),
            }
        }
        while c.next_if(|x| x.is_whitespace()).is_some() {}
        if c.next()? != '=' {
            return None;
        }
        let mut num = String::new();
        while let Some(x) = c.next_if(|x| *x != ',') {
            num.push(x);
        }
        v.push((num.trim().parse().ok()?, nombre));
        if c.next().is_none() {
            return Some(v);
        }
    }
}

fn decimal(t: &str, precision: u8, escala: u8) -> Result<Tipo, String> {
    if precision == 0 || precision > 38 || escala > precision {
        return Err(format!(
            "el tipo `{t}` no cabe en un decimal128 (38 dígitos) y no se redondea (ADR 0077 §1)"
        ));
    }
    Ok(Tipo::Decimal { precision, escala })
}

impl Tipo {
    /// **El tipo declarado de la tabla** (`tipos` de la petición, ADR 0069 M3). Con catálogo manda el catálogo: el declarado sólo
    /// resuelve lo que el servidor no distingue. Un instante declarado `DateTime` es una hora de reloj sin zona.
    pub fn declarado(self, oos: &str) -> Tipo {
        let declara_pared = ore_base::types::parse_type(oos)
            .map(|t| Fisico::de(&t) == Fisico::FechaHora)
            .unwrap_or(false);
        match self {
            Tipo::Nulable(i) => Tipo::Nulable(Box::new(i.declarado(oos))),
            Tipo::Instante {
                decimales, ancho, ..
            } if declara_pared => Tipo::Instante {
                decimales,
                ancho,
                pared: true,
            },
            otro => otro,
        }
    }

    /// El físico de ORE y el tipo de OOS que va en los metadatos del campo.
    pub fn fisico(&self) -> (Fisico, String) {
        match self {
            Tipo::Entero { .. } => (Fisico::Entero, "Integer".into()),
            Tipo::Real { .. } => (Fisico::Real, "Float".into()),
            Tipo::Logico => (Fisico::Logico, "Boolean".into()),
            Tipo::Texto
            | Tipo::TextoFijo(_)
            | Tipo::Enumerado { .. }
            | Tipo::Uuid
            | Tipo::Ipv4
            | Tipo::Ipv6 => (Fisico::Texto, "String".into()),
            Tipo::Entero64SinSigno => (
                Fisico::Decimal {
                    precision: 20,
                    escala: 0,
                },
                "Decimal<20, 0>".into(),
            ),
            Tipo::Decimal { precision, escala } => (
                Fisico::Decimal {
                    precision: *precision,
                    escala: *escala,
                },
                format!("Decimal<{precision}, {escala}>"),
            ),
            Tipo::Fecha { .. } => (Fisico::Fecha, "Date".into()),
            Tipo::Instante { pared: true, .. } => (Fisico::FechaHora, "DateTime".into()),
            Tipo::Instante { .. } => (Fisico::Instante, "DateTimeTz".into()),
            Tipo::Nulable(i) => i.fisico(),
        }
    }
}

/// La columna de Arrow de una propiedad, desde el tipo que el servidor dijo.
pub fn columna(propiedad: &str, tipo: &Tipo) -> Columna {
    let (fisico, oos) = tipo.fisico();
    Columna::new(propiedad, fisico, oos)
}

// ───────────────────────────── la cabecera ─────────────────────────────

/// Un fallo al leer: del flujo (un corte, un error del servidor), de un dato que no cabe, o de armar el lote.
#[derive(Debug)]
pub enum ErrorLectura {
    Io(io::Error),
    Dato(String),
}

impl From<io::Error> for ErrorLectura {
    fn from(e: io::Error) -> Self {
        ErrorLectura::Io(e)
    }
}

impl From<ore_arrow::ErrorArrow> for ErrorLectura {
    fn from(e: ore_arrow::ErrorArrow) -> Self {
        ErrorLectura::Dato(e.to_string())
    }
}

/// Un entero de longitud variable (LEB128), como los de las cadenas y el número de columnas.
fn varint<R: Read>(r: &mut R) -> Result<u64, ErrorLectura> {
    let (mut n, mut desp) = (0u64, 0u32);
    loop {
        let mut b = [0u8; 1];
        r.read_exact(&mut b)?;
        if desp >= 64 {
            return Err(ErrorLectura::Dato(
                "un entero de longitud variable desmedido".into(),
            ));
        }
        n |= u64::from(b[0] & 0x7f) << desp;
        if b[0] & 0x80 == 0 {
            return Ok(n);
        }
        desp += 7;
    }
}

/// Una cadena (longitud + bytes) en `buf`. Crece según llega: una longitud corrupta no reserva memoria de golpe.
fn cadena<R: Read>(r: &mut R, buf: &mut Vec<u8>) -> Result<(), ErrorLectura> {
    let n = varint(r)?;
    buf.clear();
    let leidos = r.take(n).read_to_end(buf)?;
    if leidos as u64 != n {
        return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
    }
    Ok(())
}

/// La cabecera: `(nombre, tipo)` de cada columna, tal como la consulta las devuelve.
pub fn cabecera<R: Read>(r: &mut R) -> Result<Vec<(String, String)>, ErrorLectura> {
    let n = varint(r)?;
    if n == 0 || n > 4096 {
        return Err(ErrorLectura::Dato(format!(
            "la cabecera de ClickHouse dice {n} columnas"
        )));
    }
    let mut buf = Vec::new();
    let mut texto = |r: &mut R| -> Result<String, ErrorLectura> {
        cadena(r, &mut buf)?;
        String::from_utf8(buf.clone())
            .map_err(|_| ErrorLectura::Dato("la cabecera de ClickHouse no es UTF-8".into()))
    };
    let nombres = (0..n).map(|_| texto(r)).collect::<Result<Vec<_>, _>>()?;
    let tipos = (0..n).map(|_| texto(r)).collect::<Result<Vec<_>, _>>()?;
    Ok(nombres.into_iter().zip(tipos).collect())
}

// ───────────────────────────── las celdas ─────────────────────────────

/// ¿Hay otra fila? `false` es el fin **limpio**: ni un byte más. Un corte a mitad de fila no llega aquí, sino como error de `celda`.
pub fn hay_otra<R: BufRead>(r: &mut R) -> Result<bool, ErrorLectura> {
    Ok(!r.fill_buf()?.is_empty())
}

fn bytes<const N: usize, R: Read>(r: &mut R) -> Result<[u8; N], ErrorLectura> {
    let mut b = [0u8; N];
    r.read_exact(&mut b)?;
    Ok(b)
}

fn potencia10(n: u32) -> i64 {
    10i64.pow(n)
}

/// Una celda de `tipo` leída de `r` y escrita en `e`.
pub fn celda<R: Read>(
    tipo: &Tipo,
    r: &mut R,
    buf: &mut Vec<u8>,
    e: &mut Escritor<'_>,
) -> Result<(), ErrorLectura> {
    match tipo {
        Tipo::Nulable(dentro) => {
            if bytes::<1, _>(r)?[0] == 1 {
                e.nulo();
                Ok(())
            } else {
                celda(dentro, r, buf, e)
            }
        }
        Tipo::Entero { bytes: 1, signo } => {
            let b = bytes::<1, _>(r)?;
            Ok(e.entero(if *signo {
                i64::from(b[0] as i8)
            } else {
                i64::from(b[0])
            })?)
        }
        Tipo::Entero { bytes: 2, signo } => {
            let b = bytes::<2, _>(r)?;
            Ok(e.entero(if *signo {
                i64::from(i16::from_le_bytes(b))
            } else {
                i64::from(u16::from_le_bytes(b))
            })?)
        }
        Tipo::Entero { bytes: 4, signo } => {
            let b = bytes::<4, _>(r)?;
            Ok(e.entero(if *signo {
                i64::from(i32::from_le_bytes(b))
            } else {
                i64::from(u32::from_le_bytes(b))
            })?)
        }
        Tipo::Entero { signo: true, .. } => Ok(e.entero(i64::from_le_bytes(bytes::<8, _>(r)?))?),
        Tipo::Entero { .. } => Err(ErrorLectura::Dato("entero sin signo de 64 bits".into())),
        Tipo::Real { bytes: 4 } => Ok(e.real(f64::from(f32::from_le_bytes(bytes::<4, _>(r)?)))?),
        Tipo::Real { .. } => Ok(e.real(f64::from_le_bytes(bytes::<8, _>(r)?))?),
        Tipo::Logico => Ok(e.logico(bytes::<1, _>(r)?[0] != 0)?),
        Tipo::Texto => {
            cadena(r, buf)?;
            let s = std::str::from_utf8(buf).map_err(|_| {
                ErrorLectura::Dato(
                    "un String que no es UTF-8 (ADR 0077 §1: binario, paso 4)".into(),
                )
            })?;
            Ok(e.texto(s)?)
        }
        Tipo::Decimal { precision, .. } => {
            let x = match *precision {
                1..=9 => i128::from(i32::from_le_bytes(bytes::<4, _>(r)?)),
                10..=18 => i128::from(i64::from_le_bytes(bytes::<8, _>(r)?)),
                _ => i128::from_le_bytes(bytes::<16, _>(r)?),
            };
            Ok(e.decimal(x)?)
        }
        Tipo::Fecha { ancho: 2 } => Ok(e.fecha(i32::from(u16::from_le_bytes(bytes::<2, _>(r)?)))?),
        Tipo::Fecha { .. } => Ok(e.fecha(i32::from_le_bytes(bytes::<4, _>(r)?))?),
        Tipo::Entero64SinSigno => Ok(e.decimal(i128::from(u64::from_le_bytes(bytes::<8, _>(r)?)))?),
        Tipo::TextoFijo(n) => {
            buf.clear();
            buf.resize(*n, 0);
            r.read_exact(buf)?;
            // El relleno son ceros por la derecha; un cero dentro del texto es del texto.
            let fin = buf.iter().rposition(|b| *b != 0).map_or(0, |i| i + 1);
            let s = std::str::from_utf8(&buf[..fin])
                .map_err(|_| ErrorLectura::Dato("un FixedString que no es UTF-8".into()))?;
            Ok(e.texto(s)?)
        }
        Tipo::Enumerado { ancho, nombres } => {
            let n = if *ancho == 1 {
                i16::from(bytes::<1, _>(r)?[0] as i8)
            } else {
                i16::from_le_bytes(bytes::<2, _>(r)?)
            };
            match nombres.iter().find(|(v, _)| *v == n) {
                Some((_, nombre)) => Ok(e.texto(nombre)?),
                None => Err(ErrorLectura::Dato(format!(
                    "el Enum trae {n}, que su tipo no nombra"
                ))),
            }
        }
        Tipo::Uuid => {
            // Dos UInt64 little-endian: cada mitad viene con los bytes del revés respecto al texto canónico.
            let b = bytes::<16, _>(r)?;
            let mut c = [0u8; 16];
            for i in 0..8 {
                c[i] = b[7 - i];
                c[8 + i] = b[15 - i];
            }
            let h: String = c.iter().map(|x| format!("{x:02x}")).collect();
            Ok(e.texto(&format!(
                "{}-{}-{}-{}-{}",
                &h[0..8],
                &h[8..12],
                &h[12..16],
                &h[16..20],
                &h[20..32]
            ))?)
        }
        Tipo::Ipv4 => {
            let [d, c, b, a] = bytes::<4, _>(r)?;
            Ok(e.texto(&format!("{a}.{b}.{c}.{d}"))?)
        }
        Tipo::Ipv6 => Ok(e.texto(&std::net::Ipv6Addr::from(bytes::<16, _>(r)?).to_string())?),
        Tipo::Instante {
            decimales, ancho, ..
        } => {
            let ticks = if *ancho == 4 {
                i64::from(u32::from_le_bytes(bytes::<4, _>(r)?))
            } else {
                i64::from_le_bytes(bytes::<8, _>(r)?)
            };
            let micros = match (*decimales as i32 - 6).signum() {
                // 10⁻ᵖ s con p < 6: a microsegundos, exacto.
                -1 => ticks
                    .checked_mul(potencia10(6 - u32::from(*decimales)))
                    .ok_or_else(|| {
                        ErrorLectura::Dato(format!(
                            "el instante {ticks} se sale de los microsegundos"
                        ))
                    })?,
                0 => ticks,
                // p > 6 (nanosegundos): lo que sobra de los microsegundos se pierde, y eso es redondear. Se rechaza.
                _ => {
                    let f = potencia10(u32::from(*decimales) - 6);
                    if ticks.rem_euclid(f) != 0 {
                        return Err(ErrorLectura::Dato(format!(
                            "el instante {ticks} (DateTime64({decimales})) tiene más precisión que un microsegundo y no se redondea"
                        )));
                    }
                    ticks / f
                }
            };
            Ok(e.micros(micros)?)
        }
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use ore_arrow::RecordBatchBuilder;

    #[test]
    fn los_tipos_de_la_semilla_se_analizan() {
        let c = |t| analizar(t).unwrap_or_else(|e| panic!("{t}: {e}"));
        assert_eq!(
            c("Int64"),
            Tipo::Entero {
                bytes: 8,
                signo: true
            }
        );
        assert_eq!(
            c("Nullable(Decimal(12, 2))"),
            Tipo::Nulable(Box::new(Tipo::Decimal {
                precision: 12,
                escala: 2
            }))
        );
        assert_eq!(
            c("Decimal64(4)"),
            Tipo::Decimal {
                precision: 18,
                escala: 4
            }
        );
        assert_eq!(
            c("Nullable(DateTime64(6, 'UTC'))"),
            Tipo::Nulable(Box::new(Tipo::Instante {
                decimales: 6,
                ancho: 8,
                pared: false
            }))
        );
        assert_eq!(
            c("LowCardinality(Nullable(String))"),
            Tipo::Nulable(Box::new(Tipo::Texto))
        );
        assert_eq!(c("Date32"), Tipo::Fecha { ancho: 4 });
    }

    #[test]
    fn lo_que_no_cabe_falla_con_su_nombre() {
        for t in [
            "UInt128",
            "Int128",
            "Int256",
            "Decimal(40, 2)",
            "Decimal256(2)",
            "Map(String, Int64)",
            "Array(Int64)",
            "Time",
        ] {
            let e = analizar(t).expect_err(t);
            assert!(e.contains(t.split('(').next().unwrap()), "{t}: {e}");
        }
    }

    #[test]
    fn la_cabecera_se_lee_y_una_cortada_es_un_error() {
        let mut b = vec![2u8];
        for s in ["id", "texto", "Int64", "Nullable(String)"] {
            b.push(s.len() as u8);
            b.extend_from_slice(s.as_bytes());
        }
        let h = cabecera(&mut &b[..]).unwrap();
        assert_eq!(h[0], ("id".into(), "Int64".into()));
        assert_eq!(h[1], ("texto".into(), "Nullable(String)".into()));
        assert!(matches!(
            cabecera(&mut &b[..b.len() - 3]),
            Err(ErrorLectura::Io(_))
        ));
    }

    fn una(tipo: &str, datos: &[u8]) -> Result<ore_arrow::RecordBatchBuilder, ErrorLectura> {
        let t = analizar(tipo).unwrap();
        let mut cons = RecordBatchBuilder::nuevo(&[columna("x", &t)]).unwrap();
        let mut r = datos;
        celda(&t, &mut r, &mut Vec::new(), &mut cons.col(0))?;
        cons.fila_escrita();
        Ok(cons)
    }

    #[test]
    fn las_celdas_de_cada_tipo() {
        assert_eq!(una("Int64", &(-5i64).to_le_bytes()).unwrap().filas(), 1);
        assert_eq!(una("Nullable(Int64)", &[1]).unwrap().filas(), 1);
        assert_eq!(
            una(
                "Nullable(Int64)",
                &[&[0u8][..], &7i64.to_le_bytes()].concat()
            )
            .unwrap()
            .filas(),
            1
        );
        assert_eq!(una("String", &[3, b'a', b'b', b'c']).unwrap().filas(), 1);
        assert_eq!(
            una("Decimal(12, 2)", &12345i64.to_le_bytes())
                .unwrap()
                .filas(),
            1
        );
        assert_eq!(
            una("Decimal(5, 2)", &12345i32.to_le_bytes())
                .unwrap()
                .filas(),
            1
        );
        assert_eq!(
            una("Decimal(30, 2)", &12345i128.to_le_bytes())
                .unwrap()
                .filas(),
            1
        );
        assert_eq!(una("Bool", &[1]).unwrap().filas(), 1);
        assert_eq!(una("Float64", &1.5f64.to_le_bytes()).unwrap().filas(), 1);
        assert_eq!(una("Date32", &(-1i32).to_le_bytes()).unwrap().filas(), 1);
        assert_eq!(
            una("DateTime64(3)", &1500i64.to_le_bytes())
                .unwrap()
                .filas(),
            1
        );
    }

    #[test]
    fn una_celda_cortada_es_un_error_de_io_y_no_una_fila() {
        assert!(matches!(una("Int64", &[1, 2, 3]), Err(ErrorLectura::Io(_))));
        // La longitud dice 5 y sólo hay 2 bytes.
        assert!(matches!(
            una("String", &[5, b'a', b'b']),
            Err(ErrorLectura::Io(_))
        ));
    }

    #[test]
    fn la_precision_de_nanosegundos_solo_pasa_si_es_exacta() {
        assert_eq!(
            una("DateTime64(9)", &2_000i64.to_le_bytes())
                .unwrap()
                .filas(),
            1
        );
        assert!(matches!(
            una("DateTime64(9)", &2_001i64.to_le_bytes()),
            Err(ErrorLectura::Dato(_))
        ));
        assert!(matches!(
            una("DateTime64(3)", &i64::MAX.to_le_bytes()),
            Err(ErrorLectura::Dato(_))
        ));
    }

    #[test]
    fn los_tipos_de_texto_se_analizan() {
        assert_eq!(analizar("UInt64").unwrap(), Tipo::Entero64SinSigno);
        assert_eq!(analizar("FixedString(4)").unwrap(), Tipo::TextoFijo(4));
        assert_eq!(analizar("UUID").unwrap(), Tipo::Uuid);
        assert_eq!(
            analizar("Nullable(Enum8('a' = 1, 'b, (c)' = -2, 'it\\'s' = 3))").unwrap(),
            Tipo::Nulable(Box::new(Tipo::Enumerado {
                ancho: 1,
                nombres: vec![(1, "a".into()), (-2, "b, (c)".into()), (3, "it's".into())]
            }))
        );
        assert!(analizar("Enum8('a' 1)").is_err());
        assert!(analizar("FixedString(0)").is_err());
    }

    #[test]
    fn el_tipo_declarado_desempata_la_hora_de_reloj_y_nada_mas() {
        let i = analizar("Nullable(DateTime64(6, 'UTC'))").unwrap();
        assert_eq!(i.fisico().0, Fisico::Instante);
        let pared = i.clone().declarado("DateTime");
        assert_eq!(pared.fisico(), (Fisico::FechaHora, "DateTime".into()));
        // Lo declarado que no desempata no cambia lo que dice el catálogo.
        assert_eq!(i.clone().declarado("DateTimeTz"), i);
        assert_eq!(i.clone().declarado("Integer"), i);
        let e = analizar("Int64").unwrap();
        assert_eq!(e.clone().declarado("DateTime"), e);
    }

    #[test]
    fn las_celdas_de_texto_y_el_entero_grande() {
        assert_eq!(una("UInt64", &u64::MAX.to_le_bytes()).unwrap().filas(), 1);
        assert_eq!(una("FixedString(4)", b"ab  ").unwrap().filas(), 1);
        assert_eq!(una("Enum8('a' = 1, 'b' = 2)", &[2]).unwrap().filas(), 1);
        assert!(matches!(
            una("Enum8('a' = 1)", &[9]),
            Err(ErrorLectura::Dato(_))
        ));
        assert_eq!(una("IPv4", &[4, 3, 2, 1]).unwrap().filas(), 1);
        assert_eq!(una("IPv6", &[0u8; 16]).unwrap().filas(), 1);
        assert_eq!(una("UUID", &[0u8; 16]).unwrap().filas(), 1);
    }
}
