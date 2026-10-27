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
use crate::url::{Acceso, Destino, analizar};
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

/// Un fallo **al abrir la conexión**, con la causa dicha en palabras. El driver solo cuenta que «el listener rechazó la
/// conexión (ORA-12506)»; aquí se añade **qué es eso, contra qué dirección ocurrió y qué se hace**. El texto del driver
/// (con su `CONNECTION_ID`, lo que Soporte de Oracle pide) va al final, literal.
pub fn fallo_al_conectar(e: oracledb::Error, destino: &Destino) -> Fallo {
    match explicar_conexion(e.kind(), destino, &e.to_string()) {
        Some(f) => f,
        None => fallo_de(e),
    }
}

fn explicar_conexion(kind: &ErrorKind, d: &Destino, texto: &str) -> Option<Fallo> {
    let donde = format!(
        "{}:{} ({} `{}`, {})",
        d.host,
        d.puerto,
        match d.acceso {
            Acceso::Servicio(_) => "servicio",
            Acceso::Sid(_) => "SID",
        },
        d.acceso.nombre(),
        if d.tls { "TLS" } else { "sin TLS" }
    );
    let nube = d.host.ends_with(".oraclecloud.com");
    let (causa, accion, reintentable): (String, String, bool) = match kind {
        ErrorKind::ListenerRefusedConnection(_, _, _, 12506) => (
            "el listener rechazó la conexión por el filtro de acceso del servicio (ORA-12506):              llegó a la base, pero la dirección de origen no está permitida"
                .into(),
            if nube {
                "en la consola de OCI, base de datos → Network → Access control list: debe contener                  la IP desde la que sale ORE (o `0.0.0.0/0` para probar), guardada y con la base                  de nuevo en Available. Si «Require mutual TLS» sigue activo, la base también                  rechaza clientes sin wallet: desactívalo"
                    .into()
            } else {
                "revisa las reglas de acceso del listener (`valid_node_checking`, ACL de red) y que                  la IP de ORE esté permitida"
                    .into()
            },
            false,
        ),
        ErrorKind::ListenerRefusedConnection(_, _, _, n @ (12516 | 12519 | 12520 | 12521)) => (
            format!(
                "el listener no tiene ahora un manejador libre para el servicio (ORA-{n}): la                  base está llena de sesiones o arrancando"
            ),
            "espera y reintenta; si persiste, la base llegó a su límite de sesiones".into(),
            true,
        ),
        ErrorKind::ListenerRefusedConnection(_, _, _, n @ (12526..=12529)) => (
            format!("la instancia no admite conexiones nuevas todavía (ORA-{n}): arrancando, en mantenimiento o restringida"),
            "espera a que esté disponible; en OCI Always Free, una base parada se arranca con More actions → Start".into(),
            true,
        ),
        ErrorKind::ListenerRefusedConnection(_, _, _, n) => (
            format!("el listener rechazó la conexión (ORA-{n}) sin que el driver sepa más"),
            "busca ese código en la documentación de Oracle y comprueba el servicio y el puerto".into(),
            false,
        ),
        ErrorKind::InvalidServiceName(..) => (
            format!(
                "el listener responde, pero no conoce el servicio `{}` (ORA-12514)",
                d.acceso.nombre()
            ),
            if nube {
                "copia el nombre completo desde Database connection (perfil `_low`, `_medium`…); en                  Autonomous lleva un prefijo `xxxx_` y el sufijo `.adb.oraclecloud.com`"
                    .into()
            } else {
                "comprueba el nombre con `lsnrctl services` (distingue el servicio de la PDB del de la CDB)".into()
            },
            false,
        ),
        ErrorKind::InvalidSid(..) => (
            format!("el listener responde, pero no conoce el SID `{}` (ORA-12505)", d.acceso.nombre()),
            "usa el nombre de servicio en vez del SID, o corrige el SID".into(),
            false,
        ),
        ErrorKind::UnexpectedRefuse(..) => (
            "el listener rechazó la conexión sin decir el motivo".into(),
            "mira el log del listener del servidor con el `CONNECTION_ID` de abajo".into(),
            false,
        ),
        ErrorKind::TlsOperation => (
            "falló la negociación TLS con el servidor".into(),
            if d.tls {
                format!(
                    "comprueba que el puerto {} es de TLS (en Autonomous: 1522 con wallet/mTLS, 1521                      sin wallet si «Require mutual TLS» está desactivado) y que el certificado del                      servidor lo firma una CA pública",
                    d.puerto
                )
            } else {
                "el servidor habla TLS: activa `tls=true` en la URL".into()
            },
            false,
        ),
        ErrorKind::WalletUnreadable(..)
        | ErrorKind::WalletPasswordMissingOrInvalid(..)
        | ErrorKind::WalletPrivateKeyInvalid(..) => (
            "el driver intentó leer una wallet y no pudo".into(),
            "ORE no usa wallets por fichero: desactiva «Require mutual TLS» en la base y usa TLS".into(),
            false,
        ),
        ErrorKind::StreamOperation => (
            "no se pudo abrir o mantener el canal con el servidor (red)".into(),
            if nube {
                "comprueba que el host y el puerto son los de Database connection, que la base no está                  parada (Always Free se detiene sola) y que la ACL deja pasar la IP de ORE"
                    .into()
            } else {
                "comprueba host, puerto y cortafuegos: nada contestó, o cortó la conexión".into()
            },
            true,
        ),
        _ => return None,
    };
    Some(
        Fallo::new(
            Codigo::Conexion,
            format!("no se pudo conectar a Oracle en {donde}: {causa}. Qué hacer: {accion}. [driver: {texto}]"),
        )
        .reintentable(reintentable),
    )
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
        let c = oracledb::connect(config).map_err(|e| fallo_al_conectar(e, &destino))?;
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

    fn destino(url: &str) -> Destino {
        analizar(url).unwrap()
    }

    fn rechazo(n: usize) -> ErrorKind {
        ErrorKind::ListenerRefusedConnection("ID".into(), "h".into(), 1521, n)
    }

    #[test]
    fn el_rechazo_por_acl_dice_la_causa_y_donde_arreglarlo() {
        let d = destino(
            "oracle://ADMIN:x@adb.ca-toronto-1.oraclecloud.com:1522/svc_low.adb.oraclecloud.com?tls=true",
        );
        let f = explicar_conexion(&rechazo(12506), &d, "texto del driver").unwrap();
        assert_eq!(f.codigo, Codigo::Conexion);
        assert!(!f.reintentable);
        for q in [
            "adb.ca-toronto-1.oraclecloud.com:1522",
            "servicio `svc_low.adb.oraclecloud.com`",
            "TLS",
            "ORA-12506",
            "Access control list",
            "mutual TLS",
            "[driver: texto del driver]",
        ] {
            assert!(f.mensaje.contains(q), "falta `{q}` en: {}", f.mensaje);
        }
        assert!(
            !f.mensaje.contains("ADMIN") && !f.mensaje.contains(":x@"),
            "{}",
            f.mensaje
        );
    }

    #[test]
    fn fuera_de_la_nube_el_consejo_no_habla_de_oci() {
        let d = destino("oracle://u:p@db.interna:1521/ORCLPDB1");
        let f = explicar_conexion(&rechazo(12506), &d, "t").unwrap();
        assert!(
            f.mensaje.contains("sin TLS") && !f.mensaje.contains("OCI"),
            "{}",
            f.mensaje
        );
    }

    #[test]
    fn un_listener_ocupado_o_una_base_arrancando_se_reintenta() {
        let d = destino("oracle://u:p@h/S");
        for n in [12516, 12519, 12520, 12521, 12526, 12527, 12528, 12529] {
            let f = explicar_conexion(&rechazo(n), &d, "t").unwrap();
            assert!(f.reintentable, "ORA-{n}");
            assert!(f.mensaje.contains(&format!("ORA-{n}")), "{}", f.mensaje);
        }
    }

    #[test]
    fn un_codigo_desconocido_conserva_su_numero() {
        let d = destino("oracle://u:p@h/S");
        let f = explicar_conexion(&rechazo(12999), &d, "t").unwrap();
        assert!(
            f.mensaje.contains("ORA-12999") && !f.reintentable,
            "{}",
            f.mensaje
        );
    }

    #[test]
    fn servicio_inexistente_nombra_el_servicio() {
        let d = destino("oracle://u:p@h/NOEXISTE");
        let k = ErrorKind::InvalidServiceName("ID".into(), "NOEXISTE".into(), "h".into(), 1521);
        let f = explicar_conexion(&k, &d, "t").unwrap();
        assert!(
            f.mensaje.contains("`NOEXISTE`") && f.mensaje.contains("ORA-12514"),
            "{}",
            f.mensaje
        );
    }

    #[test]
    fn tls_y_red_se_distinguen() {
        let con = destino("oracle://u:p@h:1522/S?tls=true");
        let sin = destino("oracle://u:p@h/S");
        let f = explicar_conexion(&ErrorKind::TlsOperation, &con, "t").unwrap();
        assert!(
            f.mensaje.contains("1522") && f.mensaje.contains("mTLS"),
            "{}",
            f.mensaje
        );
        let f = explicar_conexion(&ErrorKind::TlsOperation, &sin, "t").unwrap();
        assert!(f.mensaje.contains("tls=true"), "{}", f.mensaje);
        let f = explicar_conexion(&ErrorKind::StreamOperation, &sin, "t").unwrap();
        assert!(
            f.reintentable && f.mensaje.contains("cortafuegos"),
            "{}",
            f.mensaje
        );
    }

    #[test]
    fn lo_que_no_es_de_conexion_sigue_su_camino() {
        let d = destino("oracle://u:p@h/S");
        assert!(explicar_conexion(&ErrorKind::OutOfData, &d, "t").is_none());
    }

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
