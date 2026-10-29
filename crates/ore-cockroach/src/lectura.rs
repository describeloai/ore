//! **El `Origen` de CockroachDB**: lo que declara y cómo se conecta, lee y cancela.
//!
//! Declara sus capacidades, `conectar` abre la sesión y comprueba la versión, `leer` lee la proyección de una tabla (pasos 2 a 5) y `estimar`
//! la cuenta de las estadísticas (paso 6). Cada paso de la hoja de ruta (ADR 0080 §6) lo engrosa, y el kit dice cuándo.

use crate::tls::{self, Abierta};
use ore_driver::capacidades::Capacidades;
use ore_driver::origen::Origen;
use ore_driver::{Codigo, Fallo, Peticion};
use postgres_native_tls::MakeTlsConnector;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// El origen CockroachDB: sin estado, todo el estado vive en [`Sesion`] y [`Token`].
pub struct Cockroach;

/// El suelo de versión (ADR 0080 §1.2): la más antigua que X0 midió. Por debajo no se sabe qué hace.
pub const SUELO: (u32, u32) = (22, 2);

/// Una conexión abierta.
pub struct Sesion {
    pub cliente: postgres::Client,
    /// `v25.4.18`, tal cual la da `crdb_internal.node_build_info`.
    pub version: String,
    /// El conector TLS con que se abrió: cancelar habla TLS con la misma CA.
    pub tls: MakeTlsConnector,
    pub timeout_ms: Option<u64>,
    /// Las conexiones con que se lee una tabla grande; `1` la lee con una sola.
    pub tramos: usize,
}

/// Lo que une al que lee con quien cancela: la bandera y las consultas que corren ahora (el `CancelToken` de cada conexión y el TLS
/// para hablar con el servidor). Son varias cuando una tabla grande se lee en tramos.
#[derive(Clone, Default)]
pub struct Token {
    /// La lectura debe parar: la pidió quien cancela, o un tramo falló y los demás sobran.
    cancelada: Arc<AtomicBool>,
    /// La pidió quien cancela (`SIGTERM`, `{"cancelar": id}`): lo que distingue una cancelación de un tiempo agotado, que el servidor
    /// contesta igual (`57014`).
    pedida: Arc<AtomicBool>,
    en_vuelo: Arc<Mutex<Vec<(postgres::CancelToken, MakeTlsConnector)>>>,
}

impl Token {
    pub fn cancelada(&self) -> bool {
        self.cancelada.load(Ordering::SeqCst)
    }

    /// ¿La pidió quien cancela? (y no un tramo que falló).
    pub fn pedida(&self) -> bool {
        self.pedida.load(Ordering::SeqCst)
    }

    /// Empieza una lectura: el token de la anterior puede venir cancelado.
    pub(crate) fn rearmar(&self) {
        self.cancelada.store(false, Ordering::SeqCst);
        self.pedida.store(false, Ordering::SeqCst);
        if let Ok(mut v) = self.en_vuelo.lock() {
            v.clear();
        }
    }

    /// Una conexión más está leyendo: `cancelar` la alcanza.
    pub(crate) fn registrar(&self, t: postgres::CancelToken, tls: MakeTlsConnector) {
        if let Ok(mut v) = self.en_vuelo.lock() {
            v.push((t, tls));
        }
    }

    pub(crate) fn terminar(&self) {
        if let Ok(mut v) = self.en_vuelo.lock() {
            v.clear();
        }
    }

    /// Quien cancela: marca la lectura como pedida y corta en el servidor todo lo que corre. Se llama desde otro hilo.
    pub(crate) fn parar(&self) {
        self.pedida.store(true, Ordering::SeqCst);
        self.abortar();
    }

    /// Un tramo falló: los demás paran, pero **no** es una cancelación pedida (el error que cuenta es el del tramo).
    pub(crate) fn abortar(&self) {
        self.cancelada.store(true, Ordering::SeqCst);
        if let Ok(v) = self.en_vuelo.lock() {
            for (t, tls) in v.iter() {
                let _ = t.cancel_query(tls.clone());
            }
        }
    }
}

/// `v25.4.18` → `(25, 4)`. Una versión que no se entiende no pasa: mejor parar que suponer.
pub fn version_mayor_menor(v: &str) -> Option<(u32, u32)> {
    let mut p = v.trim().trim_start_matches('v').split('.');
    Some((p.next()?.parse().ok()?, p.next()?.parse().ok()?))
}

/// **Un error de CockroachDB, con su código del contrato.** Por el SQLSTATE cuando lo hay; si no, es la conexión.
pub fn fallo_de(e: &postgres::Error) -> Fallo {
    if let Some(db) = e.as_db_error() {
        let estado = db.code().code();
        let codigo = match estado {
            "28P01" | "28000" => Codigo::Credencial,
            "42P01" | "42703" | "3F000" | "3D000" | "42704" | "42883" => Codigo::Objeto,
            "57014" => Codigo::Tiempo,
            "53300" | "57P01" | "57P03" => Codigo::Conexion,
            c if c.starts_with("08") => Codigo::Conexion,
            _ => Codigo::Origen,
        };
        // `40001` (reinicio de la transacción) sale bien a la segunda. `XXUUU` (un instante por debajo del GC) no: pide una carga
        // entera (ADR 0080 §3.1, C7).
        let reintentable = codigo.reintentable() || estado == "40001";
        return Fallo::new(codigo, format!("{estado}: {}", db.message()))
            .reintentable(reintentable);
    }
    let causa = tls::causa(e);
    if e.is_closed() || causa.contains("timed out") || causa.contains("connect") {
        return Fallo::new(Codigo::Conexion, causa);
    }
    Fallo::origen(causa)
}

/// La versión del servidor, de `crdb_internal.node_build_info`: `SHOW server_version` dice siempre `13.0.0` y `version()` se escribe
/// distinto en cada versión. Un servidor que no tiene esa tabla es PostgreSQL de verdad (o cualquier otro), no CockroachDB.
fn version_de(c: &mut postgres::Client) -> Result<String, Fallo> {
    let fila = c
        .query_opt(
            "SELECT value FROM crdb_internal.node_build_info WHERE field = 'Version'",
            &[],
        )
        .map_err(|e| match e.as_db_error().map(|d| d.code().code()) {
            Some("42P01" | "3F000" | "42704") => Fallo::origen(
                "el servidor no es CockroachDB (no tiene `crdb_internal.node_build_info`): ORE lo lee como `postgresql://` con el \
                 origen de PostgreSQL",
            ),
            _ => fallo_de(&e),
        })?;
    let Some(fila) = fila else {
        return Err(Fallo::origen(
            "CockroachDB no dio su versión en `crdb_internal.node_build_info`",
        ));
    };
    Ok(fila.get::<_, String>(0))
}

impl Origen for Cockroach {
    type Sesion = Sesion;
    type Token = Token;

    const CAPACIDADES: Capacidades = Capacidades {
        conector: "ore-read-cockroach",
        version: env!("CARGO_PKG_VERSION"),
        // Se declara lo que se sabe llevar y el kit comprueba: caso 2 prueba cada operador declarado, caso 3 que el resto se rechaza.
        operadores: ore_driver::OPERADORES,
        limit: true,
        order_by: true,
        estimar: true,
        servir: true,
    };

    fn conectar(url: &str) -> Result<Sesion, Fallo> {
        // La primera consulta es también la prueba de la credencial: un usuario inexistente sale como `credencial`.
        let Abierta {
            mut cliente,
            tls,
            timeout_ms,
            tramos,
        } = tls::abrir(url)?;
        let version = version_de(&mut cliente)?;
        match version_mayor_menor(&version) {
            Some(v) if v >= SUELO => {}
            Some(_) => {
                return Err(Fallo::origen(format!(
                    "CockroachDB {version}: ORE exige {}.{} o posterior",
                    SUELO.0, SUELO.1
                )));
            }
            None => {
                return Err(Fallo::origen(format!(
                    "no se entiende la versión de CockroachDB: {version:?}"
                )));
            }
        }
        Ok(Sesion {
            cliente,
            version,
            tls,
            timeout_ms,
            tramos,
        })
    }

    fn leer(
        s: &mut Sesion,
        token: &Token,
        p: &Peticion,
        salida: &mut dyn Write,
    ) -> Result<u64, Fallo> {
        crate::leer::leer(s, token, p, salida)
    }

    fn estimar(s: &mut Sesion, p: &Peticion) -> Result<String, Fallo> {
        crate::leer::estimar(s, p)
    }

    fn cancelar(token: &Token) {
        token.parar();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelar_es_pedido_y_abortar_un_tramo_no() {
        let t = Token::default();
        assert!(!t.cancelada() && !t.pedida());
        t.abortar();
        assert!(
            t.cancelada() && !t.pedida(),
            "un tramo que falla no es una cancelación pedida"
        );
        t.rearmar();
        assert!(!t.cancelada());
        Cockroach::cancelar(&t);
        assert!(t.cancelada() && t.pedida());
        t.rearmar();
        assert!(
            !t.cancelada() && !t.pedida(),
            "una lectura nueva empieza sin cancelar"
        );
    }

    #[test]
    fn la_version_se_lee_con_o_sin_v() {
        assert_eq!(version_mayor_menor("v25.4.18"), Some((25, 4)));
        assert_eq!(version_mayor_menor("22.2.19"), Some((22, 2)));
        assert_eq!(version_mayor_menor(" v24.3.37 "), Some((24, 3)));
    }

    #[test]
    fn una_version_que_no_se_entiende_no_pasa() {
        for v in ["", "v", "nightly", "v25", "x.y.z"] {
            assert_eq!(version_mayor_menor(v), None, "{v:?}");
        }
    }

    #[test]
    fn el_suelo_deja_pasar_lo_medido_y_nada_por_debajo() {
        assert!(version_mayor_menor("v22.2.19").unwrap() >= SUELO);
        assert!(version_mayor_menor("v26.2.9").unwrap() >= SUELO);
        assert!(version_mayor_menor("v22.1.22").unwrap() < SUELO);
        assert!(version_mayor_menor("v21.2.17").unwrap() < SUELO);
    }

    #[test]
    fn declara_los_diez_operadores_limit_orden_estimar_y_servir() {
        let c = Cockroach::CAPACIDADES;
        assert_eq!(c.conector, "ore-read-cockroach");
        assert_eq!(c.operadores.len(), 10);
        assert!(c.limit && c.order_by);
        assert!(c.estimar && c.servir);
    }
}
