//! **La conexión**: el cliente Avatica con la URL de Phoenix, la tabla del catálogo y los errores traducidos
//! a los siete códigos de ORE (ficha 0071 §3.8).
//!
//! Los errores de Phoenix llegan por el PQS **todos con código −1 y estado `00000`**: el de Phoenix va dentro
//! del texto (`ERROR 1012 (42M03): Table undefined…`). [`error_sql`] lo extrae y [`fallo_de`] lo clasifica.

use crate::esquema::{Objeto, Tabla, desde_metadatos, objetos_desde};
use crate::traductor::literal_texto;
use crate::url::{Destino, analizar};
use ore_avatica::Error;
use ore_driver::{Codigo, Fallo};
use std::collections::HashMap;

/// Una conexión abierta contra un PQS.
pub struct Conexion {
    pub destino: Destino,
    pub avatica: ore_avatica::Conexion,
}

impl Conexion {
    /// Abre con la URL de la petición. No ejecuta nada.
    pub fn abrir(url: &str) -> Result<Conexion, Fallo> {
        let destino = analizar(url).map_err(|m| Fallo::new(Codigo::Operador, m))?;
        Self::abrir_destino(destino, None)
    }

    /// Abre con un tiempo límite propio de **esta** conexión (`phoenix.query.timeoutMs`, que Phoenix lee al
    /// abrirla): lo que una petición con `timeoutMs` necesita sin tocar la sesión que `servir` reutiliza.
    pub fn abrir_destino(destino: Destino, timeout_ms: Option<u64>) -> Result<Conexion, Fallo> {
        let avatica =
            ore_avatica::Conexion::abrir(&destino.opciones(timeout_ms)).map_err(fallo_de)?;
        Ok(Conexion { destino, avatica })
    }

    /// Lo mismo, con la URL que ya se analizó.
    pub fn con_tiempo(&self, timeout_ms: u64) -> Result<Conexion, Fallo> {
        Self::abrir_destino(self.destino.clone(), Some(timeout_ms))
    }

    /// La tabla, del catálogo (`getColumns`). Un esquema vacío es «sin esquema». Si no tiene columnas, no
    /// existe: es `objeto`.
    pub fn tabla(&self, esquema: Option<&str>, nombre: &str) -> Result<Tabla, Fallo> {
        // El patrón de `getColumns` trata `%` y `_` como comodines: se filtra por igualdad exacta al leer el
        // resultado, y aquí se pide con el nombre tal cual (un comodín de más solo trae filas que sobran).
        let m = self
            .avatica
            .columnas(Some(esquema.unwrap_or("")), Some(nombre))
            .map_err(fallo_de)?;
        let t = desde_metadatos(esquema, nombre, &m).map_err(|e| Fallo::new(Codigo::Origen, e))?;
        t.ok_or_else(|| {
            let objeto = match esquema {
                Some(e) => format!("{e}.{nombre}"),
                None => nombre.to_string(),
            };
            Fallo::new(Codigo::Objeto, format!("la tabla `{objeto}` no existe en Phoenix (los nombres distinguen mayúsculas: `ESQUEMA.TABLA`)"))
        })
    }

    /// La tabla con su columna `ROW_TIMESTAMP`. `getColumns` no la trae (§3.16): sale de `SYSTEM.CATALOG`. Si
    /// preguntar falla, la tabla se queda **sin** incremental y se dice: no se inventa una columna, y tampoco se
    /// tumba una lectura que no la necesita.
    pub fn con_row_timestamp(&self, mut t: Tabla) -> Tabla {
        let esquema = match &t.esquema {
            Some(e) => format!("TABLE_SCHEM = {}", literal_texto(e)),
            None => "TABLE_SCHEM IS NULL".to_string(),
        };
        let sql = format!(
            "SELECT COLUMN_NAME FROM SYSTEM.CATALOG WHERE {esquema} AND TABLE_NAME = {}              AND IS_ROW_TIMESTAMP = TRUE AND COLUMN_NAME IS NOT NULL",
            literal_texto(&t.nombre)
        );
        match self.consulta_entera(&sql) {
            Ok(filas) => {
                t.row_timestamp = filas.first().and_then(|f| match f.first() {
                    Some(ore_avatica::Valor::Texto(c)) => Some(c.clone()),
                    _ => None,
                });
            }
            Err(f) => eprintln!(
                "ore-read-phoenix: aviso · `{}`: no se pudo preguntar por su ROW_TIMESTAMP, se lee sin incremental: {}",
                t.objeto(),
                f.mensaje
            ),
        }
        t
    }

    /// Todas las columnas `ROW_TIMESTAMP` del servidor en **una** consulta, para un catálogo: preguntar tabla a
    /// tabla costaría una ida y vuelta por tabla. La clave es `(esquema, tabla)`.
    pub fn row_timestamps(&self) -> Result<HashMap<(Option<String>, String), String>, Fallo> {
        let filas = self.consulta_entera(
            "SELECT TABLE_SCHEM, TABLE_NAME, COLUMN_NAME FROM SYSTEM.CATALOG              WHERE IS_ROW_TIMESTAMP = TRUE AND COLUMN_NAME IS NOT NULL",
        )?;
        let mut m = HashMap::new();
        for f in filas {
            if let [
                esquema,
                ore_avatica::Valor::Texto(tabla),
                ore_avatica::Valor::Texto(col),
            ] = f.as_slice()
            {
                let esquema = match esquema {
                    ore_avatica::Valor::Texto(e) if !e.is_empty() => Some(e.clone()),
                    _ => None,
                };
                m.insert((esquema, tabla.clone()), col.clone());
            }
        }
        Ok(m)
    }

    /// La hora del servidor en milisegundos desde la época. Es **su** reloj, no el de quien pregunta: el tope de un
    /// testigo se compara con las marcas que el servidor pone.
    pub fn ahora_ms(&self) -> Result<i64, Fallo> {
        let filas = self
            .consulta_entera("SELECT CAST(CURRENT_TIME() AS BIGINT) FROM SYSTEM.CATALOG LIMIT 1")?;
        match filas.first().and_then(|f| f.first()) {
            Some(ore_avatica::Valor::Entero(n)) => Ok(*n),
            otro => Err(Fallo::origen(format!(
                "Phoenix no dio su hora: se esperaba un entero y llegó {otro:?}"
            ))),
        }
    }

    /// Las tablas y vistas (`getTables`) del esquema que nombra la URL, o de todos. Sin columnas: esas se
    /// piden de una en una (`tabla`), porque un `getColumns` de un esquema falla entero si alguna vista de
    /// inquilino lo ensucia (X0 §3.5).
    pub fn objetos(&self) -> Result<Vec<Objeto>, Fallo> {
        let m = self
            .avatica
            .tablas(self.destino.esquema.as_deref(), None, &["TABLE", "VIEW"])
            .map_err(fallo_de)?;
        let mut o = objetos_desde(&m).map_err(|e| Fallo::new(Codigo::Origen, e))?;
        // El patrón trata `_` como comodín: solo vale el esquema exacto.
        if let Some(e) = &self.destino.esquema {
            o.retain(|x| x.esquema.as_deref() == Some(e.as_str()));
        }
        Ok(o)
    }

    /// Ejecuta una consulta y devuelve su lectura frame a frame.
    pub fn consulta(&self, sql: &str) -> Result<ore_avatica::Lectura, Fallo> {
        self.avatica.ejecutar(sql).map_err(fallo_de)
    }

    /// Ejecuta una consulta y trae **todas** sus filas. Solo para lo pequeño (un `SELECT 1`, un catálogo).
    pub fn consulta_entera(&self, sql: &str) -> Result<Vec<ore_avatica::Fila>, Fallo> {
        let mut l = self.consulta(sql)?;
        let mut filas = Vec::new();
        while let Some(f) = l.siguiente_frame().map_err(fallo_de)? {
            filas.extend(f);
        }
        Ok(filas)
    }
}

/// El código y el estado de Phoenix de un mensaje del PQS: `ERROR 1012 (42M03): …`. El **último** de la
/// cadena (`A -> B -> C`), que es la causa. `None` si el mensaje no trae ninguno.
pub fn error_sql(mensaje: &str) -> Option<(u32, String, String)> {
    let i = mensaje.rfind("ERROR ")?;
    let resto = &mensaje[i + "ERROR ".len()..];
    let (codigo, resto) = resto.split_once(' ')?;
    let codigo: u32 = codigo.parse().ok()?;
    let resto = resto.strip_prefix('(')?;
    let (estado, resto) = resto.split_once(')')?;
    if estado.len() != 5 || !estado.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return None;
    }
    let texto = resto.trim_start_matches(':').trim();
    // Hasta el siguiente eslabón de la cadena, si lo hay.
    let texto = texto.split(" -> ").next().unwrap_or(texto);
    Some((codigo, estado.to_string(), texto.to_string()))
}

/// El texto de un error del servidor para quien lee: la causa de Phoenix si la hay, y si no, el mensaje
/// recortado (la pila de Java queda fuera).
fn texto_servidor(mensaje: &str) -> String {
    match error_sql(mensaje) {
        Some((c, e, t)) => format!("Phoenix {c} ({e}): {t}"),
        None => {
            let t: String = mensaje.chars().take(400).collect();
            t
        }
    }
}

/// **Un error de Avatica, en uno de los siete códigos de ORE.**
///
/// - red → `conexion` (reintentable); tiempo agotado y el límite de Phoenix → `tiempo`;
/// - HTTP 401/403 → `credencial`; 502/503/504 de un balanceador → `conexion` (reintentable); otro → `origen`;
/// - Phoenix: tabla o esquema que no existe → `objeto`; columna que no existe → `esquema`; tipos que no se
///   comparan → `operador` (lo que se pidió está mal); permisos → `credencial`; el clúster de HBase caído →
///   `conexion` (reintentable); el resto, `origen`;
/// - un statement perdido → `conexion` (se reinicia la lectura entera); un truncado o una respuesta fuera del
///   protocolo → `origen`, sin reintento.
pub fn fallo_de(e: Error) -> Fallo {
    let reintentable = e.reintentable();
    match &e {
        Error::Red(m) => Fallo::new(
            Codigo::Conexion,
            format!("sin conexión con Phoenix Query Server: {m}"),
        ),
        Error::TiempoRed(m) => Fallo::new(
            Codigo::Tiempo,
            format!("Phoenix Query Server no contestó a tiempo: {m}"),
        ),
        // Un certificado malo o un servidor que no habla TLS 1.2 no mejora al repetirlo, y la credencial no salió.
        Error::Tls(_) => Fallo::new(Codigo::Conexion, e.to_string()).reintentable(false),
        Error::Http {
            estado: 401 | 403, ..
        } => Fallo::new(
            Codigo::Credencial,
            "Phoenix Query Server rechaza la credencial (HTTP 401/403)",
        ),
        Error::Http { estado, cuerpo } => {
            let c = if matches!(estado, 502..=504) {
                Codigo::Conexion
            } else {
                Codigo::Origen
            };
            Fallo::new(
                c,
                format!("respuesta HTTP {estado} que no es de Phoenix Query Server: {cuerpo}"),
            )
            .reintentable(reintentable)
        }
        Error::Servidor { mensaje, .. } => {
            if e.es_tiempo_limite() {
                return Fallo::new(
                    Codigo::Tiempo,
                    "Phoenix agotó el tiempo límite de la consulta (`timeoutMs`)",
                );
            }
            // El PQS ya no conoce la conexión y reabrirla no bastó: se reinició y no vuelve, o está a medias. Se
            // empieza la lectura de nuevo.
            if e.conexion_perdida() {
                return Fallo::new(
                    Codigo::Conexion,
                    "Phoenix Query Server perdió la conexión (¿se reinició?) y no se pudo reabrir",
                );
            }
            let t = texto_servidor(mensaje);
            let codigo = match error_sql(mensaje) {
                // Tabla indefinida, esquema indefinido.
                Some((1012, ..)) | Some((722, ..)) => Codigo::Objeto,
                // Columna indefinida o ambigua.
                Some((504, ..)) | Some((502, ..)) => Codigo::Esquema,
                // Tipos que no se comparan: lo que se pidió está mal.
                Some((203, ..)) => Codigo::Operador,
                _ => clasificar_por_texto(mensaje),
            };
            Fallo::new(codigo, t)
        }
        Error::Protocolo(m) => Fallo::new(
            Codigo::Origen,
            format!("Phoenix Query Server contestó fuera del protocolo: {m}"),
        ),
        Error::StatementPerdido => Fallo::new(Codigo::Conexion, e.to_string()),
        Error::Truncado { .. } => Fallo::new(Codigo::Origen, e.to_string()),
    }
}

/// Lo que Phoenix no dice con un código que se entienda, por el texto de la excepción de Java.
fn clasificar_por_texto(m: &str) -> Codigo {
    let t = m.to_ascii_lowercase();
    if t.contains("accessdeniedexception")
        || t.contains("insufficient permissions")
        || t.contains("access denied")
    {
        Codigo::Credencial
    } else if t.contains("retriesexhausted")
        || t.contains("connectexception")
        || t.contains("serverNotRunning".to_ascii_lowercase().as_str())
        || t.contains("sockettimeout")
        || t.contains("no server address listed")
        || t.contains("zookeeper")
    {
        Codigo::Conexion
    } else {
        Codigo::Origen
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn servidor(m: &str) -> Error {
        Error::Servidor {
            codigo: u32::MAX,
            sql_state: "00000".into(),
            mensaje: m.into(),
            excepciones: vec![],
        }
    }

    #[test]
    fn el_codigo_de_phoenix_sale_del_texto() {
        // Tal como llegó en las trazas de X0 (`error_tabla`).
        let m = "RuntimeException: org.apache.phoenix.schema.TableNotFoundException: ERROR 1012 (42M03): Table undefined. tableName=NO_EXISTE -> TableNotFoundException: ERROR 1012 (42M03): Table undefined. tableName=NO_EXISTE";
        assert_eq!(
            error_sql(m),
            Some((
                1012,
                "42M03".into(),
                "Table undefined. tableName=NO_EXISTE".into()
            ))
        );
        assert_eq!(
            error_sql("SQLException: ERROR 504 (42703): Undefined column. columnName=X"),
            Some((504, "42703".into(), "Undefined column. columnName=X".into()))
        );
        for sin in [
            "",
            "nada",
            "ERROR x (42M03): a",
            "ERROR 1012 (4): a",
            "ERROR 1012 42M03: a",
            "TimeoutException",
        ] {
            assert_eq!(error_sql(sin), None, "`{sin}`");
        }
    }

    #[test]
    fn los_errores_de_phoenix_en_los_siete_codigos() {
        let caso = |m: &str| fallo_de(servidor(m));
        let f = caso(
            "RuntimeException: TableNotFoundException: ERROR 1012 (42M03): Table undefined. tableName=T",
        );
        assert_eq!(f.codigo, Codigo::Objeto);
        assert!(
            f.mensaje.starts_with("Phoenix 1012 (42M03)")
                && !f.mensaje.contains("RuntimeException"),
            "{}",
            f.mensaje
        );
        assert!(!f.reintentable);
        assert_eq!(
            caso("ColumnNotFoundException: ERROR 504 (42703): Undefined column. columnName=X")
                .codigo,
            Codigo::Esquema
        );
        assert_eq!(caso("TypeMismatchException: ERROR 203 (22005): Type mismatch. BIGINT and VARCHAR for ID = 'x'").codigo, Codigo::Operador);
        assert_eq!(
            caso("ParseException: ERROR 601 (42P00): Syntax error. Encountered \"SELEC\"").codigo,
            Codigo::Origen
        );
        assert_eq!(
            caso("ArithmeticException: ERROR 202 (22012): Divide by zero.").codigo,
            Codigo::Origen
        );
        assert_eq!(caso("AccessDeniedException: Insufficient permissions for user 'ana' (table=T, action=READ)").codigo, Codigo::Credencial);
        let f = caso("RetriesExhaustedException: Failed after attempts=36");
        assert_eq!(f.codigo, Codigo::Conexion);
        assert_eq!(caso("algo raro que nadie previó").codigo, Codigo::Origen);
    }

    #[test]
    fn una_conexion_que_el_servidor_perdio_es_conexion_y_reintentable() {
        let f = fallo_de(servidor(
            "org.apache.calcite.avatica.NoSuchConnectionException
	at JdbcMeta.getConnection",
        ));
        assert_eq!((f.codigo, f.reintentable), (Codigo::Conexion, true));
        assert!(f.mensaje.contains("perdió la conexión"), "{}", f.mensaje);
    }

    #[test]
    fn el_tiempo_limite_es_tiempo() {
        for m in [
            "RuntimeException: java.sql.SQLTimeoutException: . Query couldn't be completed in the allotted time: 1500 ms",
            "RuntimeException: java.sql.SQLException: Encountered exception in sub plan [0] execution. -> TimeoutException: (null exception message)",
        ] {
            let f = fallo_de(servidor(m));
            assert_eq!(f.codigo, Codigo::Tiempo, "{m}");
        }
    }

    #[test]
    fn un_fallo_de_tls_es_conexion_no_reintentable_y_dice_que_la_credencial_no_salio() {
        let f = fallo_de(Error::Tls("certificate verify failed".into()));
        assert_eq!((f.codigo, f.reintentable), (Codigo::Conexion, false));
        assert!(
            f.mensaje.contains("certificate verify failed"),
            "{}",
            f.mensaje
        );
        assert!(
            f.mensaje.contains("no se envió la credencial"),
            "{}",
            f.mensaje
        );
    }

    #[test]
    fn la_red_http_y_el_protocolo() {
        assert_eq!(fallo_de(Error::Red("x".into())).codigo, Codigo::Conexion);
        assert!(fallo_de(Error::Red("x".into())).reintentable);
        assert_eq!(
            fallo_de(Error::TiempoRed("x".into())).codigo,
            Codigo::Tiempo
        );
        let f = fallo_de(Error::Http {
            estado: 401,
            cuerpo: "<html>".into(),
        });
        assert_eq!((f.codigo, f.reintentable), (Codigo::Credencial, false));
        assert!(!f.mensaje.contains("<html>"));
        assert_eq!(
            fallo_de(Error::Http {
                estado: 403,
                cuerpo: String::new()
            })
            .codigo,
            Codigo::Credencial
        );
        let f = fallo_de(Error::Http {
            estado: 503,
            cuerpo: "x".into(),
        });
        assert_eq!((f.codigo, f.reintentable), (Codigo::Conexion, true));
        let f = fallo_de(Error::Http {
            estado: 404,
            cuerpo: "x".into(),
        });
        assert_eq!((f.codigo, f.reintentable), (Codigo::Origen, false));
        // Un statement perdido: se reinicia la lectura entera (reintentable). Un truncado, no.
        let f = fallo_de(Error::StatementPerdido);
        assert_eq!((f.codigo, f.reintentable), (Codigo::Conexion, true));
        let f = fallo_de(Error::Truncado { leidas: 7 });
        assert_eq!((f.codigo, f.reintentable), (Codigo::Origen, false));
        assert_eq!(
            fallo_de(Error::Protocolo("x".into())).codigo,
            Codigo::Origen
        );
    }

    #[test]
    fn un_mensaje_largo_se_recorta_y_la_pila_de_java_no_sale() {
        let m = format!("RuntimeException: {}", "x".repeat(5000));
        let f = fallo_de(servidor(&m));
        assert!(f.mensaje.chars().count() <= 400, "{}", f.mensaje.len());
    }
}
