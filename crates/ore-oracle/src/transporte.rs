//! **El transporte**: abrir una conexión, catalogar y volver cada celda texto canónico. Es lo único de este
//! crate que habla con un servidor.
//!
//! El driver (`oracledb`) es síncrono y habla el protocolo de Oracle directamente (thin): ni Instant Client ni
//! nada del sistema. Lo que no cubre está en la cabecera de `Cargo.toml` y en [`fallo_de`], que lo dice con
//! palabras en vez de dejar pasar su mensaje.
//!
//! # La sesión
//!
//! Al abrir se fijan **la zona y los formatos**, porque Oracle convierte el texto de un parámetro según la
//! sesión (`NLS_*`) y un cursor por fecha que dependiera del `NLS` del usuario daría otras filas sin fallar:
//!
//! - `TIME_ZONE = 'UTC'`: un `TIMESTAMP WITH LOCAL TIME ZONE` llega en la zona de la sesión, sin decir cuál.
//! - `NLS_DATE_FORMAT`, `NLS_TIMESTAMP_FORMAT`, `NLS_TIMESTAMP_TZ_FORMAT`: el texto canónico que sale de
//!   [`Fisico::analizar`](ore_base::tipos::Fisico::analizar), para que `cursor > :1` entienda el borde.
//! - `NLS_NUMERIC_CHARACTERS = '.,'`.

use crate::tipos::Tipo;
use crate::url::{Destino, analizar};
use oracledb::{
    Config, Connection, ErrorKind, JsonValue, OracleIntervalDS, OracleIntervalYM, OracleNumber,
    OracleTimestamp, Row,
};
use ore_driver::{Codigo, Fallo};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::time::Duration;

/// Oracle 12.1 es el protocolo más antiguo que habla el driver thin.
const NO_THIN: &str = "este servidor es anterior a Oracle 12.1: el driver thin no lo habla (haría falta el cliente OCI)";

/// Un fallo del driver, como el que el resto de ORE entiende. El mensaje de Oracle (`ORA-xxxxx`) va
/// **literal**: es lo único accionable.
pub fn fallo_de(e: oracledb::Error) -> Fallo {
    let texto = e.to_string();
    match e.kind() {
        ErrorKind::DbError(d) => {
            // El mensaje de Oracle suele traer ya su `ORA-xxxxx:`; no se repite.
            let m = d.message().trim();
            let ora = if m.starts_with("ORA-") {
                m.to_string()
            } else {
                format!("ORA-{:05}: {m}", d.code())
            };
            match d.code() {
                // 1017 usuario o contraseña; 28000 cuenta bloqueada; 28001 contraseña caducada; 28009 SYSDBA.
                1017 | 28000 | 28001 | 28009 => Fallo::new(Codigo::Credencial, ora),
                // 942 la tabla o la vista no existe (o no se ve); 4043 el objeto no existe.
                942 | 4043 => Fallo::new(Codigo::Objeto, ora),
                // 1013 el usuario canceló la operación; 1013 también es `call_timeout`.
                1013 | 3156 => Fallo::new(Codigo::Tiempo, ora).reintentable(true),
                // 12170 vence la conexión; 3113/3114 fin de fichero en el canal; 3135 conexión perdida.
                3113 | 3114 | 3135 | 12170 | 12537 | 12541 | 12543 | 12545 => {
                    Fallo::new(Codigo::Conexion, ora).reintentable(true)
                }
                _ => Fallo::origen(ora),
            }
        }
        ErrorKind::CallTimeoutExceeded => Fallo::new(Codigo::Tiempo, texto).reintentable(true),
        // Tras vencer `call_timeout` o cortarse la red el driver no sabe seguir: cierra la conexión.
        ErrorKind::UnableToRecover | ErrorKind::DeadConnection | ErrorKind::NotConnected => {
            Fallo::new(Codigo::Conexion, texto).reintentable(true)
        }
        ErrorKind::ServerVersionNotSupported => Fallo::new(Codigo::Conexion, NO_THIN),
        ErrorKind::NoCredentials => {
            Fallo::new(Codigo::Credencial, "la URL no trae `usuario:clave`")
        }
        ErrorKind::ListenerRefusedConnection(..)
        | ErrorKind::InvalidServiceName(..)
        | ErrorKind::InvalidSid(..)
        | ErrorKind::UnexpectedRefuse(..)
        | ErrorKind::InvalidConnectString(..)
        | ErrorKind::StreamOperation
        | ErrorKind::TlsOperation
        | ErrorKind::WalletUnreadable(..)
        | ErrorKind::WalletPasswordMissingOrInvalid(..)
        | ErrorKind::WalletPrivateKeyInvalid(..) => Fallo::new(Codigo::Conexion, texto),
        _ => Fallo::origen(texto),
    }
}

/// Como [`fallo_de`], pero sabiendo que la lectura tenía un `timeoutMs` y desde cuándo corre: si el driver
/// dice que no puede seguir (cierra la conexión al vencer `call_timeout`) y ya pasó el tiempo, el motivo es
/// **el tiempo**, no la red.
pub fn fallo_leyendo(
    e: oracledb::Error,
    timeout_ms: Option<u64>,
    desde: std::time::Instant,
) -> Fallo {
    let corte = matches!(
        e.kind(),
        ErrorKind::UnableToRecover | ErrorKind::CallTimeoutExceeded | ErrorKind::DeadConnection
    );
    match timeout_ms {
        // El driver espera hasta 2×: se da por vencido a partir de un 90 % del límite.
        Some(ms) if corte && desde.elapsed() >= Duration::from_millis(ms * 9 / 10) => Fallo::new(
            Codigo::Tiempo,
            format!("`timeoutMs` ({ms} ms) agotado: {e}"),
        )
        .reintentable(true),
        _ => fallo_de(e),
    }
}

/// Una conexión abierta. El driver no es `Sync`: cada hilo abre la suya.
pub struct Conexion {
    pub destino: Destino,
    pub c: Connection,
}

impl Conexion {
    pub fn abrir(url: &str) -> Result<Conexion, Fallo> {
        let destino = analizar(url).map_err(|m| Fallo::new(Codigo::Conexion, m))?;
        let (Some(usuario), Some(clave)) = (&destino.usuario, &destino.clave) else {
            return Err(Fallo::new(
                Codigo::Credencial,
                "la URL no trae `usuario:clave`",
            ));
        };
        let config = Config::default()
            .set_credentials(usuario, clave)
            .set_connect_string(&destino.cadena())
            .map_err(fallo_de)?;
        let c = oracledb::connect(config).map_err(fallo_de)?;
        for sql in [
            "ALTER SESSION SET TIME_ZONE = 'UTC'",
            "ALTER SESSION SET NLS_NUMERIC_CHARACTERS = '.,'",
            "ALTER SESSION SET NLS_DATE_FORMAT = 'YYYY-MM-DD HH24:MI:SS'",
            "ALTER SESSION SET NLS_TIMESTAMP_FORMAT = 'YYYY-MM-DD HH24:MI:SS.FF6'",
            "ALTER SESSION SET NLS_TIMESTAMP_TZ_FORMAT = 'YYYY-MM-DD HH24:MI:SS.FF6 TZH:TZM'",
        ] {
            c.execute(sql, &[]).map_err(fallo_de)?;
        }
        if let Some(ms) = destino.timeout_ms {
            c.set_call_timeout(Some(Duration::from_millis(ms)))
                .map_err(fallo_de)?;
        }
        Ok(Conexion { destino, c })
    }

    /// La primera línea de `v$version`, que no todo usuario puede leer: `PRODUCT_COMPONENT_VERSION` sí.
    pub fn version(&self) -> Result<String, Fallo> {
        let r = self
            .c
            .query_row(
                "SELECT version_full FROM product_component_version WHERE ROWNUM = 1",
                &[],
            )
            .or_else(|_| {
                self.c.query_row(
                    "SELECT version FROM product_component_version WHERE ROWNUM = 1",
                    &[],
                )
            })
            .map_err(fallo_de)?;
        r.get::<String>(0).map_err(fallo_de)
    }

    /// El usuario con el que se conectó, tal como lo guarda Oracle (mayúsculas).
    pub fn usuario(&self) -> Result<String, Fallo> {
        let r = self
            .c
            .query_row("SELECT USER FROM dual", &[])
            .map_err(fallo_de)?;
        r.get::<String>(0).map_err(fallo_de)
    }

    /// Las tablas y vistas que la URL deja ver: las del `esquema=` si lo nombra, y si no, todas las que el
    /// usuario puede leer menos las que mantiene Oracle.
    pub fn objetos(&self) -> Result<Vec<Objeto>, Fallo> {
        // `ORACLE_MAINTAINED` existe desde 12.1, que es el mínimo del thin.
        const DE_ORACLE: &str =
            "owner NOT IN (SELECT username FROM all_users WHERE oracle_maintained = 'Y')";
        // Sin tablas temporales (sin filas para otro), ni anidadas, ni las de la papelera (`BIN$…`), ni la
        // parte de desbordamiento de una IOT.
        let sql = |filtro: &str, marca2: &str| {
            format!(
                "SELECT owner, table_name, 'table', num_rows FROM all_tables \
                 WHERE {DE_ORACLE} {filtro} AND temporary = 'N' AND nested = 'NO' \
                 AND table_name NOT LIKE 'BIN$%' AND (iot_type IS NULL OR iot_type = 'IOT') \
                 UNION ALL \
                 SELECT owner, view_name, 'view', CAST(NULL AS NUMBER) FROM all_views \
                 WHERE {DE_ORACLE} {marca2} \
                 ORDER BY 1, 2"
            )
        };
        let cursor = match &self.destino.esquema {
            Some(e) => self
                .c
                .query(
                    &sql("AND owner = :1", "AND owner = :2"),
                    &[&e.as_str(), &e.as_str()],
                )
                .map_err(fallo_de)?,
            None => self.c.query(&sql("", ""), &[]).map_err(fallo_de)?,
        };
        let mut v = Vec::new();
        for f in cursor {
            let f = f.map_err(fallo_de)?;
            let filas = f
                .get::<Option<OracleNumber>>(3)
                .map_err(fallo_de)?
                .and_then(|n| n.to_string().parse::<u64>().ok());
            v.push(Objeto {
                esquema: f.get::<String>(0).map_err(fallo_de)?,
                nombre: f.get::<String>(1).map_err(fallo_de)?,
                vista: f.get::<String>(2).map_err(fallo_de)? == "view",
                filas_estimadas: filas,
            });
        }
        Ok(v)
    }

    /// Una tabla o vista: sus columnas con el tipo del catálogo, su clave primaria, sus claves únicas y los
    /// comentarios de sus columnas.
    pub fn tabla(&self, esquema: &str, nombre: &str) -> Result<Tabla, Fallo> {
        let cursor = self
            .c
            .query(
                "SELECT c.column_name, c.data_type, c.data_precision, c.data_scale, c.nullable, \
                        m.comments \
                 FROM all_tab_columns c \
                 LEFT JOIN all_col_comments m \
                   ON m.owner = c.owner AND m.table_name = c.table_name AND m.column_name = c.column_name \
                 WHERE c.owner = :1 AND c.table_name = :2 \
                 ORDER BY c.column_id",
                &[&esquema, &nombre],
            )
            .map_err(fallo_de)?;
        let mut columnas = Vec::new();
        for f in cursor {
            let f = f.map_err(fallo_de)?;
            let tipo_oracle = f.get::<String>(1).map_err(fallo_de)?;
            let entero = |i: usize| -> Result<Option<i32>, Fallo> {
                Ok(f.get::<Option<OracleNumber>>(i)
                    .map_err(fallo_de)?
                    .and_then(|n| n.to_string().parse::<i32>().ok()))
            };
            let (p, s) = (entero(2)?, entero(3)?);
            columnas.push(Columna {
                nombre: f.get::<String>(0).map_err(fallo_de)?,
                tipo: Tipo::de(&tipo_oracle, p, s),
                tipo_oracle,
                precision: p,
                escala: s,
                anulable: f.get::<String>(4).map_err(fallo_de)? == "Y",
                descripcion: f
                    .get::<Option<String>>(5)
                    .map_err(fallo_de)?
                    .filter(|d| !d.trim().is_empty()),
            });
        }
        if columnas.is_empty() {
            return Err(Fallo::new(
                Codigo::Objeto,
                format!(
                    "`{esquema}.{nombre}` no existe o el usuario no lo ve (los nombres distinguen mayúsculas)"
                ),
            ));
        }
        // Las claves: `P` primaria, `U` única. Una fila por columna, en el orden de la restricción.
        let cursor = self
            .c
            .query(
                "SELECT k.constraint_name, k.constraint_type, cc.column_name \
                 FROM all_constraints k \
                 JOIN all_cons_columns cc \
                   ON cc.owner = k.owner AND cc.constraint_name = k.constraint_name \
                 WHERE k.owner = :1 AND k.table_name = :2 AND k.constraint_type IN ('P', 'U') \
                   AND k.status = 'ENABLED' \
                 ORDER BY k.constraint_name, cc.position",
                &[&esquema, &nombre],
            )
            .map_err(fallo_de)?;
        let mut claves: BTreeMap<String, (String, Vec<String>)> = BTreeMap::new();
        for f in cursor {
            let f = f.map_err(fallo_de)?;
            let e = claves
                .entry(f.get::<String>(0).map_err(fallo_de)?)
                .or_insert_with(|| (String::new(), Vec::new()));
            e.0 = f.get::<String>(1).map_err(fallo_de)?;
            e.1.push(f.get::<String>(2).map_err(fallo_de)?);
        }
        let mut clave = Vec::new();
        let mut unicas = Vec::new();
        for (_, (tipo, cols)) in claves {
            if tipo == "P" {
                clave = cols;
            } else {
                unicas.push(cols);
            }
        }
        Ok(Tabla {
            esquema: esquema.to_string(),
            nombre: nombre.to_string(),
            columnas,
            clave,
            unicas,
        })
    }
}

/// Un objeto del catálogo de Oracle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Objeto {
    pub esquema: String,
    pub nombre: String,
    pub vista: bool,
    /// `NUM_ROWS` de las estadísticas: una estimación, y puede no haberse calculado nunca.
    pub filas_estimadas: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Columna {
    pub nombre: String,
    pub tipo: Tipo,
    /// `DATA_TYPE` tal cual lo da Oracle (`TIMESTAMP(6) WITH TIME ZONE`, `NUMBER`…).
    pub tipo_oracle: String,
    pub precision: Option<i32>,
    pub escala: Option<i32>,
    pub anulable: bool,
    pub descripcion: Option<String>,
}

impl Columna {
    /// El tipo tal cual lo dijo Oracle, con la precisión y la escala que el escalar de OOS no lleva.
    pub fn origen(&self) -> String {
        match (self.precision, self.escala) {
            (Some(p), Some(s)) if self.tipo_oracle == "NUMBER" => format!("NUMBER({p},{s})"),
            (Some(p), None) if self.tipo_oracle == "NUMBER" => format!("NUMBER({p})"),
            _ => self.tipo_oracle.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tabla {
    pub esquema: String,
    pub nombre: String,
    pub columnas: Vec<Columna>,
    pub clave: Vec<String>,
    pub unicas: Vec<Vec<String>>,
}

impl Tabla {
    /// El nombre que ORE le da: `ESQUEMA.TABLA`.
    pub fn objeto(&self) -> String {
        format!("{}.{}", self.esquema, self.nombre)
    }

    pub fn columna(&self, nombre: &str) -> Option<&Columna> {
        self.columnas.iter().find(|c| c.nombre == nombre)
    }
}

// ── Las celdas ───────────────────────────────────────────────────────────────────────────────────────────

/// La celda `i` de la fila, como el texto canónico que [`Fisico::analizar`](ore_base::tipos::Fisico::analizar)
/// entiende. `None` es `NULL` (y en Oracle `''` también lo es).
pub fn celda(f: &Row, i: usize, c: &Columna) -> Result<Option<String>, Fallo> {
    let dt = c.tipo_oracle.to_ascii_uppercase();
    macro_rules! tomar {
        ($t:ty, $a_texto:expr) => {
            f.get::<Option<$t>>(i)
                .map_err(fallo_de)?
                .map(|v| ($a_texto)(v))
        };
    }
    Ok(match &c.tipo {
        Tipo::Texto => tomar!(String, |s| s),
        Tipo::Entero | Tipo::Decimal { .. } | Tipo::DecimalLibre => {
            tomar!(OracleNumber, |n: OracleNumber| n.to_string())
        }
        Tipo::Real if dt == "BINARY_FLOAT" => tomar!(f32, |x: f32| x.to_string()),
        Tipo::Real if dt == "BINARY_DOUBLE" => tomar!(f64, |x: f64| x.to_string()),
        Tipo::Real => tomar!(OracleNumber, |n: OracleNumber| n.to_string()),
        Tipo::Logico => tomar!(bool, |b: bool| b.to_string()),
        Tipo::FechaHora => tomar!(OracleTimestamp, |t: OracleTimestamp| hora_de_pared(&t)),
        Tipo::Fecha => match f.get::<Option<OracleTimestamp>>(i).map_err(fallo_de)? {
            None => None,
            Some(t) if (t.hour(), t.minute(), t.second(), t.nanoseconds()) == (0, 0, 0, 0) => {
                Some(format!("{:04}-{:02}-{:02}", t.year(), t.month(), t.day()))
            }
            // La tabla declara `Date` y este valor lleva hora: no se descarta a escondidas.
            Some(t) => {
                return Err(Fallo::new(
                    Codigo::Esquema,
                    format!(
                        "columna `{}`: la tabla declara `Date` y el valor {} lleva hora; declara `DateTime`",
                        c.nombre,
                        hora_de_pared(&t)
                    ),
                ));
            }
        },
        Tipo::Instante => tomar!(OracleTimestamp, |t: OracleTimestamp| instante(&t)),
        Tipo::Intervalo if dt.contains("YEAR") => {
            tomar!(OracleIntervalYM, |x: OracleIntervalYM| x.to_string())
        }
        Tipo::Intervalo => tomar!(OracleIntervalDS, |x: OracleIntervalDS| x.to_string()),
        // Los bytes viajan como base64, el texto canónico de `Binary`; `Fisico::analizar` los decodifica al armar
        // el lote (una conversión por celda, como las demás). `BinarioTexto` los deja en texto.
        Tipo::Binario | Tipo::BinarioTexto => {
            tomar!(Vec<u8>, |b: Vec<u8>| ore_driver::base64::codificar(&b))
        }
        Tipo::Documento => tomar!(JsonValue, |j: JsonValue| json(&j)),
        Tipo::Otro(t) => {
            return Err(Fallo::new(
                Codigo::Esquema,
                format!("la columna `{}` es {t}, que este origen no lee", c.nombre),
            ));
        }
    })
}

/// `2026-10-25 13:14:15.123456`: los campos tal cual, a microsegundos (Arrow guarda µs; los ns se pierden y se
/// dice en el catálogo).
fn hora_de_pared(t: &OracleTimestamp) -> String {
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:06}",
        t.year(),
        t.month(),
        t.day(),
        t.hour(),
        t.minute(),
        t.second(),
        t.nanoseconds() / 1000
    )
}

/// Un instante: los campos que el driver da ya están **en UTC** (medido: `13:14:15 +05:30` llega como
/// `07:44:15` con el desfase aparte), así que el desfase no se suma otra vez.
fn instante(t: &OracleTimestamp) -> String {
    format!("{}+00", hora_de_pared(t))
}

/// El texto JSON de un valor de la columna `JSON`, con las claves ordenadas: el mismo documento da los mismos
/// bytes. Un `JsonValue` guarda los objetos en un `HashMap`, así que el orden original no se conserva.
fn json(v: &JsonValue) -> String {
    let mut s = String::new();
    escribir_json(v, &mut s);
    s
}

fn escribir_json(v: &JsonValue, s: &mut String) {
    match v {
        JsonValue::Null => s.push_str("null"),
        JsonValue::Boolean(b) => s.push_str(if *b { "true" } else { "false" }),
        JsonValue::Number(n) => s.push_str(&n.to_string()),
        JsonValue::BinaryDouble(x) if x.is_finite() => {
            let _ = write!(s, "{x}");
        }
        JsonValue::BinaryFloat(x) if x.is_finite() => {
            let _ = write!(s, "{x}");
        }
        // JSON no tiene `NaN` ni infinito: un texto es lo más fiel.
        JsonValue::BinaryDouble(x) => cadena_json(&x.to_string(), s),
        JsonValue::BinaryFloat(x) => cadena_json(&x.to_string(), s),
        JsonValue::String(t) => cadena_json(t, s),
        JsonValue::Raw(b) | JsonValue::JsonId(b) => {
            cadena_json(&ore_driver::base64::codificar(b), s)
        }
        JsonValue::Timestamp(t) => cadena_json(&t.to_string(), s),
        JsonValue::IntervalDS(x) => cadena_json(&x.to_string(), s),
        JsonValue::IntervalYM(x) => cadena_json(&x.to_string(), s),
        JsonValue::Vector(_) => cadena_json("<vector>", s),
        JsonValue::JsonArray(v) => {
            s.push('[');
            for (i, x) in v.iter().enumerate() {
                if i > 0 {
                    s.push(',');
                }
                escribir_json(x, s);
            }
            s.push(']');
        }
        JsonValue::JsonObject(m) => {
            let mut claves: Vec<&String> = m.keys().collect();
            claves.sort();
            s.push('{');
            for (i, k) in claves.into_iter().enumerate() {
                if i > 0 {
                    s.push(',');
                }
                cadena_json(k, s);
                s.push(':');
                escribir_json(&m[k], s);
            }
            s.push('}');
        }
    }
}

fn cadena_json(t: &str, s: &mut String) {
    s.push('"');
    for c in t.chars() {
        match c {
            '"' => s.push_str("\\\""),
            '\\' => s.push_str("\\\\"),
            '\n' => s.push_str("\\n"),
            '\r' => s.push_str("\\r"),
            '\t' => s.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(s, "\\u{:04x}", c as u32);
            }
            c => s.push(c),
        }
    }
    s.push('"');
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn un_documento_json_sale_con_las_claves_ordenadas() {
        let mut o = HashMap::new();
        o.insert("z".to_string(), JsonValue::Null);
        o.insert("a".to_string(), JsonValue::String("x\"y\n".into()));
        o.insert(
            "k".to_string(),
            JsonValue::JsonArray(vec![JsonValue::Boolean(true), JsonValue::BinaryDouble(1.5)]),
        );
        assert_eq!(
            json(&JsonValue::JsonObject(o)),
            r#"{"a":"x\"y\n","k":[true,1.5],"z":null}"#
        );
        assert_eq!(json(&JsonValue::BinaryDouble(f64::INFINITY)), "\"inf\"");
    }

    #[test]
    fn el_tipo_de_origen_lleva_precision_y_escala() {
        let c = |t: &str, p, s| Columna {
            nombre: "X".into(),
            tipo: Tipo::de(t, p, s),
            tipo_oracle: t.into(),
            precision: p,
            escala: s,
            anulable: true,
            descripcion: None,
        };
        assert_eq!(c("NUMBER", Some(12), Some(2)).origen(), "NUMBER(12,2)");
        assert_eq!(c("NUMBER", None, None).origen(), "NUMBER");
        assert_eq!(
            c("TIMESTAMP(6) WITH TIME ZONE", None, None).origen(),
            "TIMESTAMP(6) WITH TIME ZONE"
        );
    }
}
