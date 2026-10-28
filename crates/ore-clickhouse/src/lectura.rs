//! **El `Origen` de ClickHouse**: lo que declara y cómo se conecta, lee y cancela.
//!
//! Esqueleto andante: declara sus capacidades y rechaza leer con un error claro. Cada paso de la hoja de ruta (ADR 0077 §6)
//! lo engrosa, y el kit dice cuándo.

use crate::rowbinary::{ErrorLectura, analizar, cabecera, celda, columna, hay_otra};
use crate::traductor::{consulta, partes_del_objeto, tipos_de_la_tabla};
use crate::transporte::{Cliente, Param, fallo_de_io};
use crate::verbos::pide_final;
use ore_arrow::{Emisor, RecordBatchBuilder};
use ore_base::json::Json;
use ore_driver::capacidades::Capacidades;
use ore_driver::origen::Origen;
use ore_driver::{Codigo, Fallo, Peticion};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// El origen ClickHouse: sin estado, todo el estado vive en [`Sesion`] y [`Token`].
pub struct ClickHouse;

/// El suelo de versión (ADR 0077 §6, paso 2): la más antigua que X0 midió. Por debajo no se sabe qué hace.
pub const SUELO: (u32, u32) = (24, 8);

/// Los ajustes por consulta que el conector mandará (pasos 6 y 7). Se mandan sólo los que la versión declara
/// en `system.settings` (uno desconocido es el error 115) y el perfil permite (`readonly=1` no admite ninguno).
const AJUSTES: [&str; 2] = [
    "max_execution_time",
    "cancel_http_readonly_queries_on_client_close",
];

/// Una conexión abierta.
pub struct Sesion {
    pub cliente: Cliente,
    pub version: String,
    /// `readonly` del perfil: 0 escribe, 1 sólo lee y no admite ajustes, 2 sólo lee y admite ajustes.
    pub readonly: u8,
    /// Los de [`AJUSTES`] que se pueden mandar con esta versión y este perfil.
    pub ajustes: Vec<&'static str>,
    /// Si una tabla se lee con `FINAL`, por `(base, tabla)`: su motor no cambia sin recrearla, pero la sesión de `servir` dura, así
    /// que la respuesta caduca ([`CADUCA_MOTOR`]).
    motores: HashMap<(String, String), (Instant, bool)>,
}

/// Lo que dura lo que una sesión sabe del motor de una tabla.
const CADUCA_MOTOR: Duration = Duration::from_secs(60);

impl Sesion {
    /// ¿Se lee `base.tabla` con `FINAL`? Una tabla que no se ve, o una vista, no: el error (o la lectura) viene después, de la consulta.
    fn con_final(&mut self, base: &str, tabla: &str) -> Result<bool, Fallo> {
        let clave = (base.to_string(), tabla.to_string());
        if let Some((cuando, f)) = self.motores.get(&clave)
            && cuando.elapsed() < CADUCA_MOTOR
        {
            return Ok(*f);
        }
        let motor = self.cliente.texto_con(
            "SELECT engine FROM system.tables WHERE database = {d:String} AND name = {t:String}",
            &[("d", Param::Texto(base)), ("t", Param::Texto(tabla))],
        )?;
        let f = pide_final(motor.trim());
        self.motores.insert(clave, (Instant::now(), f));
        Ok(f)
    }
}

/// `24.8.14.39` → `(24, 8)`. Una versión que no se entiende no pasa: mejor parar que suponer.
pub fn version_mayor_menor(v: &str) -> Option<(u32, u32)> {
    let mut p = v.trim().split('.');
    Some((p.next()?.parse().ok()?, p.next()?.parse().ok()?))
}

/// Los `*_overflow_mode = break` del perfil: con ellos una consulta que pasa el límite **acaba bien y a medias**.
fn desbordes_en_break(filas: &str) -> Vec<&str> {
    filas
        .lines()
        .filter_map(|l| l.split_once('\t'))
        .filter(|(n, v)| n.ends_with("_overflow_mode") && v.trim() == "break")
        .map(|(n, _)| n)
        .collect()
}

/// Lo que une al que lee con quien cancela.
#[derive(Clone, Default)]
pub struct Token {
    cancelada: Arc<AtomicBool>,
    /// La consulta que está en el servidor ahora mismo: `(url, query_id)`. Quien cancela la mata por ahí; si no puede (no tiene el
    /// permiso `KILL QUERY`), la mata el cierre de la conexión.
    en_vuelo: Arc<Mutex<Option<(String, String)>>>,
}

impl Token {
    pub fn cancelada(&self) -> bool {
        self.cancelada.load(Ordering::SeqCst)
    }
}

/// Lo que el cliente espera de más sobre `timeoutMs` antes de cortar él: que al servidor le dé tiempo a contestar con su propio error.
const GRACIA_MS: u64 = 2_000;

/// Lo que tarda como mucho el `KILL QUERY` de quien cancela: el proceso va a morir y no se espera más.
const TOPE_KILL: Duration = Duration::from_secs(3);

fn nuevo_query_id() -> String {
    use std::sync::atomic::AtomicU64;
    static N: AtomicU64 = AtomicU64::new(0);
    let ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!(
        "ore-{}-{ns}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    )
}

/// Los ajustes de esta lectura: sólo los que la sesión puede mandar (versión y perfil).
fn ajustes_de(permitidos: &[&'static str], tiempo_ms: Option<u64>) -> Vec<(&'static str, String)> {
    let mut v = Vec::new();
    for a in permitidos {
        match (*a, tiempo_ms) {
            // Segundos con decimales: `1000` ms es `1`, `1500` ms es `1.5`. Cero es «sin límite» en ClickHouse, y cero ms no es eso.
            ("max_execution_time", Some(ms)) => {
                v.push((*a, format!("{}", (ms.max(1) as f64) / 1000.0)))
            }
            ("max_execution_time", None) => {}
            // Cerrar la conexión cancela la consulta (si no, una que no emite nada sigue viva hasta que intente escribir).
            ("cancel_http_readonly_queries_on_client_close", _) => v.push((*a, "1".to_string())),
            _ => {}
        }
    }
    v
}

/// `KILL QUERY` de una consulta propia, sin esperar a que acabe. Mejor esfuerzo: sin el permiso, falla y ya.
fn matar(url: &str, query_id: &str) -> Result<(), Fallo> {
    Cliente::abrir(url)?.con_tope(TOPE_KILL)?.texto_con(
        "KILL QUERY WHERE query_id = {q:String} ASYNC",
        &[("q", crate::transporte::Param::Texto(query_id))],
    )?;
    Ok(())
}

/// La estimación desde la fila de `system.tables`: la **tabla entera** (los filtros no se tienen en cuenta), sin leer un dato. `\N` es que el
/// motor no lo sabe (una vista, una tabla distribuida): si no sabe ninguno, el objeto no estima.
fn estimacion(hoja: &str, base: &str, tabla: &str) -> Result<String, Fallo> {
    let Some((filas, bytes)) = hoja.trim_end().split_once('\t') else {
        return Err(Fallo::new(
            Codigo::Objeto,
            format!("`{base}.{tabla}` no existe, o el usuario no puede verla"),
        ));
    };
    let numero = |t: &str| t.trim().parse::<i64>().ok();
    let (filas, bytes) = (numero(filas), numero(bytes));
    if filas.is_none() && bytes.is_none() {
        return Err(Fallo::operador(format!(
            "ClickHouse no sabe cuánto tiene `{base}.{tabla}` (una vista o un motor que no guarda su tamaño)"
        )));
    }
    let mut campos = vec![("fuente", Json::s("system.tables"))];
    if let Some(f) = filas {
        campos.push(("filas", Json::Int(f)));
    }
    if let Some(b) = bytes {
        campos.push(("bytes", Json::Int(b)));
    }
    Ok(Json::obj(campos).jcs())
}

/// Filas por lote Arrow: la memoria de una lectura es un lote, y el primer byte sale al llenarse el primero.
pub const FILAS_POR_LOTE: usize = 65_536;

fn de_lectura(e: ErrorLectura) -> Fallo {
    match e {
        ErrorLectura::Io(e) => fallo_de_io(&e),
        ErrorLectura::Dato(m) => Fallo::origen(m),
    }
}

/// La respuesta, de la cabecera al último lote. Una fila a medias (el servidor falló después de volcar su búfer) **no** es una
/// fila: el error llega de la lectura y el lote en curso se descarta.
fn leer_flujo<R: BufRead>(
    r: &mut R,
    p: &Peticion,
    token: &Token,
    salida: &mut dyn Write,
) -> Result<u64, Fallo> {
    let cab = cabecera(r).map_err(de_lectura)?;
    if cab.len() != p.proyeccion.len() {
        return Err(Fallo::origen(format!(
            "ClickHouse devolvió {} columnas y se pidieron {}",
            cab.len(),
            p.proyeccion.len()
        )));
    }
    let mut tipos = Vec::with_capacity(cab.len());
    let mut columnas = Vec::with_capacity(cab.len());
    for ((propiedad, col), (nombre, texto)) in p.proyeccion.iter().zip(&cab) {
        let t = analizar(texto).map_err(|m| {
            Fallo::new(
                Codigo::Esquema,
                format!("`{}`, columna `{nombre}`: {m}", p.objeto),
            )
        })?;
        // El catálogo manda; lo declarado sólo resuelve lo que el servidor no distingue (`DateTime` frente a `DateTimeTz`).
        let t = match p.tipos.iter().find(|(c, _)| c == col) {
            Some((_, declarado)) => t.declarado(declarado),
            None => t,
        };
        columnas.push(columna(propiedad, &t));
        tipos.push(t);
    }
    let mut lote = RecordBatchBuilder::nuevo(&columnas)
        .map_err(|e| Fallo::origen(format!("no se pudo armar el lote: {e}")))?;
    // La cabecera Arrow sale **después** de que el servidor aceptó la consulta y dijo sus tipos: lo que falla antes es un
    // error limpio y no un flujo a medias.
    let mut emisor = Emisor::nuevo(salida, &lote.schema())?;
    let (mut filas, mut buf) = (0u64, Vec::new());
    let vaciar = |lote: &mut RecordBatchBuilder, emisor: &mut Emisor<&mut dyn Write>| {
        let l = lote
            .finish()
            .map_err(|e| Fallo::origen(format!("no se pudo cerrar el lote: {e}")))?;
        emisor.lote(&l)
    };
    while hay_otra(r).map_err(de_lectura)? {
        for (i, t) in tipos.iter().enumerate() {
            celda(t, r, &mut buf, &mut lote.col(i)).map_err(de_lectura)?;
        }
        lote.fila_escrita();
        filas += 1;
        if lote.filas() >= FILAS_POR_LOTE {
            vaciar(&mut lote, &mut emisor)?;
            if token.cancelada() {
                return Err(Fallo::origen(format!("cancelada con {filas} filas leídas")));
            }
        }
    }
    if lote.filas() > 0 {
        vaciar(&mut lote, &mut emisor)?;
    }
    emisor.fin()?;
    Ok(filas)
}

impl Origen for ClickHouse {
    type Sesion = Sesion;
    type Token = Token;

    const CAPACIDADES: Capacidades = Capacidades {
        conector: "ore-read-clickhouse",
        version: env!("CARGO_PKG_VERSION"),
        // Se declara lo que se sabe llevar y el kit comprueba: caso 2 prueba cada operador declarado, caso 3 que el
        // resto se rechaza. Empieza vacío y crece con cada caso en verde.
        operadores: ore_driver::OPERADORES,
        limit: true,
        order_by: true,
        estimar: true,
        servir: true,
    };

    fn conectar(url: &str) -> Result<Sesion, Fallo> {
        let cliente = Cliente::abrir(url)?;
        // Una sola ida: la versión y el perfil. Es también la prueba de la credencial (usuario inexistente → `credencial`).
        let hoja = cliente.texto("SELECT version(), toString(getSetting('readonly'))")?;
        let (version, readonly) = hoja.trim_end().split_once('\t').ok_or_else(|| {
            Fallo::origen("ClickHouse contestó a `version()` con algo que no se entiende")
        })?;
        let version = version.to_string();
        let readonly: u8 = readonly.trim().parse().unwrap_or(0);
        match version_mayor_menor(&version) {
            Some(v) if v >= SUELO => {}
            Some(_) => {
                return Err(Fallo::origen(format!(
                    "ClickHouse {version}: ORE exige {}.{} o posterior",
                    SUELO.0, SUELO.1
                )));
            }
            None => {
                return Err(Fallo::origen(format!(
                    "no se entiende la versión de ClickHouse: {version:?}"
                )));
            }
        }
        // `break` trunca en silencio: el perfil que lo lleva no sirve para leer.
        let cambiados = cliente.texto(
            "SELECT name, value FROM system.settings WHERE changed AND endsWith(name, '_overflow_mode')",
        )?;
        let rotos = desbordes_en_break(&cambiados);
        if !rotos.is_empty() {
            return Err(Fallo::operador(format!(
                "el perfil del usuario tiene {} = 'break': una consulta que pasa el límite acabaría bien con filas de menos; \
                 ORE no lo admite (usa 'throw')",
                rotos.join(", ")
            )));
        }
        let ajustes = if readonly == 1 {
            Vec::new()
        } else {
            let lista = AJUSTES.map(|a| format!("'{a}'")).join(",");
            let hay = cliente.texto(&format!(
                "SELECT name FROM system.settings WHERE name IN ({lista})"
            ))?;
            AJUSTES
                .into_iter()
                .filter(|a| hay.lines().any(|l| l == *a))
                .collect()
        };
        Ok(Sesion {
            cliente,
            version,
            readonly,
            ajustes,
            motores: HashMap::new(),
        })
    }

    fn leer(
        s: &mut Sesion,
        token: &Token,
        p: &Peticion,
        salida: &mut dyn Write,
    ) -> Result<u64, Fallo> {
        // El token se rearma al empezar: el de la lectura anterior puede venir cancelado.
        token.cancelada.store(false, Ordering::SeqCst);
        Self::CAPACIDADES.admite(p)?;
        if !p.claves.is_empty() {
            return Err(Fallo::operador(
                "el recorte por clave todavía no se empuja a ClickHouse",
            ));
        }
        if p.cursor.is_some() || p.start.is_some() || p.end.is_some() {
            return Err(Fallo::operador(
                "el rango incremental todavía no se empuja a ClickHouse (ADR 0077 §5)",
            ));
        }
        let base = s.cliente.destino().base.as_deref();
        // Los tipos de la tabla sólo hacen falta para atar los valores de los filtros.
        let tipos = if p.filtros.is_empty() {
            HashMap::new()
        } else {
            let (b, t) = partes_del_objeto(&p.objeto, base)?;
            tipos_de_la_tabla(&s.cliente, &b, &t)?
        };
        let (b, t) = partes_del_objeto(&p.objeto, base)?;
        let con_final = s.con_final(&b, &t)?;
        let base = s.cliente.destino().base.as_deref();
        let (sql, atados) = consulta(p, base, &tipos, con_final)?;
        // El tiempo de la petición, o el de la URL. Lo corta el servidor (`max_execution_time`) si el perfil deja mandar ajustes, y lo
        // corta el cliente un poco después si no (o si el servidor no contesta): en ambos casos la conexión se cierra.
        let tiempo = p.timeout_ms.or(s.cliente.destino().timeout_ms);
        let con_tope;
        let cliente = match tiempo {
            Some(ms) => {
                con_tope = s
                    .cliente
                    .con_tope(Duration::from_millis(ms.saturating_add(GRACIA_MS)))?;
                &con_tope
            }
            None => &s.cliente,
        };
        let ajustes = ajustes_de(&s.ajustes, tiempo);
        let ajustes: Vec<(&str, &str)> = ajustes.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let query_id = nuevo_query_id();
        *token.en_vuelo.lock().unwrap() = Some((p.url.clone(), query_id.clone()));
        let hecho = cliente
            .flujo(
                &sql,
                "RowBinaryWithNamesAndTypes",
                &atados.como_params(),
                &ajustes,
                Some(&query_id),
            )
            .and_then(|flujo| {
                let mut r = BufReader::with_capacity(64 * 1024, flujo);
                leer_flujo(&mut r, p, token, salida)
            });
        *token.en_vuelo.lock().unwrap() = None;
        match hecho {
            // Cancelar mata la consulta y el servidor contesta con un error suyo: lo que importa es que se canceló.
            Err(_) if token.cancelada() => Err(Fallo::origen("lectura cancelada")),
            otro => otro,
        }
    }

    fn estimar(s: &mut Sesion, p: &Peticion) -> Result<String, Fallo> {
        let (b, t) = partes_del_objeto(&p.objeto, s.cliente.destino().base.as_deref())?;
        let hoja = s.cliente.texto_con(
            "SELECT total_rows, total_bytes FROM system.tables WHERE database = {d:String} AND name = {t:String}",
            &[("d", Param::Texto(&b)), ("t", Param::Texto(&t))],
        )?;
        estimacion(&hoja, &b, &t)
    }

    fn cancelar(token: &Token) {
        token.cancelada.store(true, Ordering::SeqCst);
        // Matar la consulta en el servidor, sin esperar: si el proceso muere, la conexión se cierra y con el ajuste
        // `cancel_http_readonly_queries_on_client_close` el servidor la corta también; esto cubre al perfil que no puede mandarlo.
        let vuelo = token.en_vuelo.lock().unwrap().clone();
        if let Some((url, query_id)) = vuelo {
            let _ = matar(&url, &query_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_conector_declara_lo_que_sabe() {
        let c = ClickHouse::CAPACIDADES;
        assert_eq!(c.conector, "ore-read-clickhouse");
        assert_eq!(c.operadores, ore_driver::OPERADORES);
        assert!(c.limit && c.order_by && c.estimar && c.servir);
    }

    #[test]
    fn cancelar_activa_la_bandera_y_leer_la_rearma() {
        let t = Token::default();
        assert!(!t.cancelada());
        ClickHouse::cancelar(&t);
        assert!(t.cancelada());
        let mut s = Sesion {
            cliente: Cliente::abrir("clickhouse://x").expect("cliente"),
            version: "24.8.1.1".into(),
            readonly: 0,
            ajustes: Vec::new(),
            motores: HashMap::new(),
        };
        let p = ore_driver::leer_peticion(
            r#"{"url":"clickhouse://x","objeto":"t","proyeccion":{"id":"id"}}"#,
        )
        .expect("petición");
        let _ = ClickHouse::leer(&mut s, &t, &p, &mut Vec::new());
        assert!(!t.cancelada());
    }

    #[test]
    fn la_version_se_lee_y_el_suelo_se_compara() {
        assert_eq!(version_mayor_menor("24.8.14.39"), Some((24, 8)));
        assert_eq!(
            version_mayor_menor(
                "26.4.1.1
"
            ),
            Some((26, 4))
        );
        assert!(version_mayor_menor("24.3.9.1").unwrap() < SUELO);
        assert!(version_mayor_menor("25.3.1.1").unwrap() >= SUELO);
        assert_eq!(version_mayor_menor("raro"), None);
        assert_eq!(version_mayor_menor("24"), None);
    }

    #[test]
    fn solo_el_break_en_un_overflow_mode_se_rechaza() {
        let hoja = "max_rows_to_read	100
read_overflow_mode	break
group_by_overflow_mode	throw
sort_overflow_mode	break
join_algorithm	break
";
        assert_eq!(
            desbordes_en_break(hoja),
            vec!["read_overflow_mode", "sort_overflow_mode"]
        );
        assert!(desbordes_en_break("").is_empty());
    }
}

#[cfg(test)]
mod pruebas_de_lectura {
    use super::*;
    use std::io::{self, Read};

    /// Lo que entrega, y luego un error de lectura: un servidor que falla a mitad de flujo.
    struct ConError {
        datos: io::Cursor<Vec<u8>>,
    }

    impl Read for ConError {
        fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
            match self.datos.read(b)? {
                0 => Err(io::Error::other("el servidor falló")),
                n => Ok(n),
            }
        }
    }

    fn peticion(proy: &str) -> Peticion {
        ore_driver::leer_peticion(&format!(
            r#"{{"url":"clickhouse://x","objeto":"kit.t","proyeccion":{proy}}}"#
        ))
        .expect("petición")
    }

    fn cabecera_de(columnas: &[(&str, &str)]) -> Vec<u8> {
        let mut b = vec![columnas.len() as u8];
        for s in columnas
            .iter()
            .map(|c| c.0)
            .chain(columnas.iter().map(|c| c.1))
        {
            b.push(s.len() as u8);
            b.extend_from_slice(s.as_bytes());
        }
        b
    }

    #[test]
    fn una_tabla_vacia_dice_su_esquema_y_cierra_el_flujo() {
        let mut flujo = cabecera_de(&[("id", "Int64")]);
        flujo.shrink_to_fit();
        let mut salida = Vec::new();
        let n = leer_flujo(
            &mut BufReader::new(&flujo[..]),
            &peticion(r#"{"a":"id"}"#),
            &Token::default(),
            &mut salida,
        )
        .unwrap();
        assert_eq!(n, 0);
        assert!(!salida.is_empty(), "el esquema sale aunque no haya filas");
    }

    #[test]
    fn un_error_a_mitad_de_fila_es_el_error_y_no_una_fila_a_medias() {
        let mut datos = cabecera_de(&[("id", "Int64")]);
        datos.extend_from_slice(&1i64.to_le_bytes());
        datos.extend_from_slice(&[7, 7, 7]); // tres bytes de una segunda fila
        let mut salida = Vec::new();
        let f = leer_flujo(
            &mut BufReader::new(ConError {
                datos: io::Cursor::new(datos),
            }),
            &peticion(r#"{"a":"id"}"#),
            &Token::default(),
            &mut salida,
        )
        .expect_err("el servidor falló");
        assert_eq!(f.codigo, Codigo::Conexion, "{f:?}");
    }

    #[test]
    fn un_tipo_que_no_se_sabe_leer_es_un_cambio_de_esquema_con_su_nombre() {
        let datos = cabecera_de(&[("n", "Int128")]);
        let f = leer_flujo(
            &mut BufReader::new(&datos[..]),
            &peticion(r#"{"a":"n"}"#),
            &Token::default(),
            &mut Vec::new(),
        )
        .expect_err("Int128");
        assert_eq!(f.codigo, Codigo::Esquema);
        assert!(f.mensaje.contains("Int128"), "{f:?}");
    }

    #[test]
    fn la_estimacion_dice_lo_que_el_servidor_sabe_y_nada_mas() {
        let e = estimacion("1000000\t47000000\n", "kit", "grande").unwrap();
        assert_eq!(
            e,
            r#"{"bytes":47000000,"filas":1000000,"fuente":"system.tables"}"#
        );
        // Una vista no sabe sus filas pero puede saber sus bytes; si no sabe ninguno, no estima.
        let e = estimacion("\\N\t10\n", "kit", "v").unwrap();
        assert_eq!(e, r#"{"bytes":10,"fuente":"system.tables"}"#);
        assert_eq!(
            estimacion("\\N\t\\N\n", "kit", "v").unwrap_err().codigo,
            Codigo::Operador
        );
        // Ninguna fila: la tabla no existe (o no se ve).
        assert_eq!(
            estimacion("", "kit", "t").unwrap_err().codigo,
            Codigo::Objeto
        );
    }
}
