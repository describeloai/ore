//! **`leer`**: Salesforce como [`Origen`] (`ore_driver::origen`).
//!
//! Lo único de este módulo es lo que es de Salesforce: de qué objeto se lee, por
//! qué vía y con qué presupuesto. Los verbos, `SIGTERM` y las sesiones de
//! `servir` son del esqueleto.
//!
//! ```text
//!   Peticion ─▶ traducir ─▶ presupuesto de cuota ─▶ por cada consulta SOQL:
//!        REST (primera página, `totalSize`) ──si es mucho y no hay límite corto──▶ Bulk 2.0
//!        filas de texto ─▶ Salida (residuales, límite, tipos) ─▶ lotes Arrow
//! ```
//!
//! # REST o Bulk (ADR 0083 §1.6)
//!
//! Siempre se empieza por REST: su primera página trae **`totalSize`**, el
//! total de filas de la consulta, sin un `COUNT()` aparte. Si pasa de
//! [`UMBRAL_BULK`] y no hay un `limit` corto que lo acote, se descarta esa página
//! y se hace por Bulk (un job tarda de 3 a 23 s: C8, solo vale con volumen).
//! `ORE_SALESFORCE_API=rest|bulk` lo fuerza.
//!
//! # El presupuesto de cuota (§1.7)
//!
//! La org tiene una cuota de peticiones por 24 h (15.000 en la de laboratorio) y
//! **Bulk la gasta igual**: cada sondeo y cada página cuentan (C7). Por eso no
//! hay «degradar de Bulk a REST» como en Fivetran: no ahorraría nada. Lo que hay
//! es un **tope** ([`TOPE_CUOTA`], `ORE_SALESFORCE_TOPE_CUOTA`): al llegar,
//! **no se empieza otra lectura** y se devuelve un fallo reintentable que dice
//! cuánto va gastado. Es posponer, no romper.

use crate::bulk::{self, PorFila};
use crate::esquema::Objeto;
use crate::identidad;
use crate::salida::Salida;
use crate::traductor::{COLUMNA_OPERACION, OPERADORES, traducir};
use crate::transporte::{Cliente, codificar};
use crate::url;
use crate::valores;
use ore_driver::capacidades::Capacidades;
use ore_driver::origen::Origen;
use ore_driver::{Codigo, Fallo, Peticion};
use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub struct Salesforce;

/// A partir de cuántas filas (por `totalSize`) una lectura sin límite corto
/// pasa de REST a Bulk. **Sin medir con volumen** (X0 §3.2): REST pagina de 2000
/// en 2000 (25 peticiones para 50.000) y Bulk son ~7 sondeos más las páginas;
/// el cruce está por esa cifra y se ajusta al medir.
pub const UMBRAL_BULK: u64 = 50_000;

/// La relectura solapada por defecto de una lectura de cambios, en segundos.
pub const RELECTURA: u64 = 600;

/// El porcentaje de la cuota diaria a partir del cual no se empieza otra lectura.
pub const TOPE_CUOTA: u64 = 80;

/// Por dónde se lee.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modo {
    /// REST, y Bulk si hay mucho.
    Auto,
    Rest,
    Bulk,
}

impl Modo {
    pub fn del_entorno() -> Modo {
        match std::env::var("ORE_SALESFORCE_API").as_deref() {
            Ok("rest") => Modo::Rest,
            Ok("bulk") => Modo::Bulk,
            _ => Modo::Auto,
        }
    }
}

/// Cómo se elige la vía: el modo y, en `Auto`, el umbral de filas. Se pasa
/// entera a [`leer_con`] para que una prueba fuerce el cambio a Bulk sin tocar
/// variables de entorno.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Politica {
    pub modo: Modo,
    pub umbral: u64,
    /// Cuántos segundos se **retrasa el inicio** de una lectura de cambios
    /// (`_ore_op`): la relectura solapada. Se toma de Airbyte (`PT10M`), **sin
    /// medir** aquí.
    pub relectura: u64,
}

impl Politica {
    pub fn del_entorno() -> Politica {
        Politica {
            modo: Modo::del_entorno(),
            umbral: umbral(),
            relectura: natural("ORE_SALESFORCE_RELECTURA", RELECTURA),
        }
    }

    /// Un modo con el umbral por defecto.
    pub fn con(modo: Modo) -> Politica {
        Politica {
            modo,
            umbral: UMBRAL_BULK,
            relectura: RELECTURA,
        }
    }
}

fn natural(env: &str, por_defecto: u64) -> u64 {
    std::env::var(env)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(por_defecto)
}

pub fn umbral() -> u64 {
    natural("ORE_SALESFORCE_UMBRAL_BULK", UMBRAL_BULK)
}

pub fn tope() -> u64 {
    natural("ORE_SALESFORCE_TOPE_CUOTA", TOPE_CUOTA).clamp(1, 100)
}

/// ¿Se pasó del tope? `uso` es `(usadas, máximo)`.
pub fn excedido(uso: (u64, u64), tope: u64) -> bool {
    uso.1 > 0 && uso.0.saturating_mul(100) >= uso.1.saturating_mul(tope)
}

/// La bandera de quien cancela. Salesforce no tiene una operación que matar
/// desde fuera para una consulta REST: se corta entre páginas, y un job de Bulk
/// se aborta.
#[derive(Clone, Default)]
pub struct Token {
    bandera: Arc<AtomicBool>,
}

impl Token {
    pub fn cancelada(&self) -> bool {
        self.bandera.load(Ordering::Relaxed)
    }
}

/// Qué vigilar entre una petición y la siguiente: que no se cancele y que no
/// se acabe el tiempo (`timeoutMs`, aplicado **en el origen**).
pub struct Control<'a> {
    pub token: &'a Token,
    pub limite: Option<Instant>,
}

impl Control<'_> {
    pub fn vigilar(&self) -> Result<(), Fallo> {
        if self.token.cancelada() {
            return Err(Fallo::origen("cancelada en el origen"));
        }
        if self.limite.is_some_and(|l| Instant::now() >= l) {
            return Err(Fallo::new(
                Codigo::Tiempo,
                "se agotó `timeoutMs` leyendo de Salesforce",
            ));
        }
        Ok(())
    }
}

pub struct Sesion {
    pub cliente: Cliente,
    esquemas: HashMap<String, Objeto>,
}

impl Sesion {
    /// El `describe` de un objeto, una vez por sesión.
    pub fn objeto(&mut self, nombre: &str) -> Result<&Objeto, Fallo> {
        if !self.esquemas.contains_key(nombre) {
            let o = describir(&self.cliente, nombre)?;
            self.esquemas.insert(nombre.to_string(), o);
        }
        Ok(&self.esquemas[nombre])
    }
}

/// `GET /sobjects/<X>/describe`.
pub fn describir(c: &Cliente, nombre: &str) -> Result<Objeto, Fallo> {
    let t = c.get(&c.api(&format!("/sobjects/{nombre}/describe")))?;
    Objeto::de_describe(&t).map_err(|m| Fallo::new(Codigo::Esquema, m))
}

/// Abre un cliente a partir de la URL, con la clave de la URL o la del entorno.
pub fn abrir(url_fuente: &str) -> Result<Cliente, Fallo> {
    let f = url::leer(url_fuente).map_err(Fallo::operador)?;
    // La de la URL (como la guarda el custodio) y, si no hay, la del entorno.
    let pem = match f.clave.clone() {
        Some(k) => k,
        None => identidad::clave_del_entorno().map_err(|m| Fallo::new(Codigo::Credencial, m))?,
    };
    Cliente::nuevo(f, &pem)
}

/// **No se empieza una lectura si la cuota va por encima del tope.** La primera
/// vez no se conoce el uso: una llamada a `/limits` lo trae (y desde entonces
/// llega en cada respuesta).
pub fn presupuesto(c: &Cliente) -> Result<(), Fallo> {
    presupuesto_con(c, tope())
}

/// Lo mismo con el tope dado (el de la prueba, o el del entorno).
pub fn presupuesto_con(c: &Cliente, t: u64) -> Result<(), Fallo> {
    let uso = match c.uso() {
        Some(u) => u,
        None => {
            c.get(&c.api("/limits"))?;
            c.uso().unwrap_or((0, 0))
        }
    };
    if excedido(uso, t) {
        return Err(Fallo::origen(format!(
            "la cuota diaria de la org va por {} de {} peticiones (tope {t}%): se pospone la lectura",
            uso.0, uso.1
        ))
        .reintentable(true));
    }
    Ok(())
}

/// Una celda de un registro JSON de REST, como texto: lo que Bulk daría en CSV.
fn celda(v: &serde_json::Value) -> Option<String> {
    use serde_json::Value::*;
    match v {
        Null => None,
        String(s) => Some(s.clone()),
        Number(n) => Some(n.to_string()),
        Bool(b) => Some(b.to_string()),
        otro => Some(otro.to_string()),
    }
}

/// Una consulta SOQL, por REST o por Bulk. `todos` es `queryAll`: incluye los
/// registros borrados (X0 D6), que es de donde salen los *tombstones*. `true` si
/// `fila` pidió parar.
#[allow(clippy::too_many_arguments)]
fn leer_consulta(
    c: &Cliente,
    soql: &str,
    columnas: &[String],
    ctl: &Control,
    pol: &Politica,
    todos: bool,
    acotada: bool,
    fila: &mut PorFila<'_>,
) -> Result<bool, Fallo> {
    let (operacion, punto) = if todos {
        ("queryAll", "/queryAll")
    } else {
        ("query", "/query")
    };
    if pol.modo == Modo::Bulk {
        return bulk::ejecutar(c, operacion, soql, columnas, ctl, fila);
    }
    let mut ruta = c.api(&format!("{punto}?q={}", codificar(soql)));
    let mut primera = true;
    loop {
        ctl.vigilar()?;
        let cabeceras: &[(&str, &str)] = if primera {
            &[("Sforce-Query-Options", "batchSize=2000")]
        } else {
            &[]
        };
        let texto = c.get_con(&ruta, cabeceras)?;
        let v: serde_json::Value = serde_json::from_str(&texto)
            .map_err(|e| Fallo::origen(format!("Salesforce no contestó JSON: {e}")))?;
        if primera {
            primera = false;
            let total = v["totalSize"].as_u64().unwrap_or(0);
            if pol.modo == Modo::Auto && !acotada && total > pol.umbral {
                return bulk::ejecutar(c, operacion, soql, columnas, ctl, fila);
            }
        }
        for rec in v["records"].as_array().map_or(&[][..], Vec::as_slice) {
            let celdas: Vec<Option<String>> =
                columnas.iter().map(|n| celda(&rec[n.as_str()])).collect();
            if !fila(&celdas)? {
                return Ok(true);
            }
        }
        match v["nextRecordsUrl"].as_str() {
            Some(n) => ruta = n.to_string(),
            None => return Ok(false),
        }
    }
}

/// Lo más atrás que la Papelera de Salesforce guarda un borrado: **15 días**
/// (X0 D7; la documentación de `getDeleted`). Más allá, un borrado ya no se ve.
pub const PAPELERA: Duration = Duration::from_secs(15 * 24 * 3600);

/// Desde cuándo la Papelera cubre los borrados de un objeto: lo que diga
/// `earliestDateAvailable` de `getDeleted` (que se adelanta si un administrador
/// la vacía) o, como mucho, [`PAPELERA`] atrás. `getDeleted` pide una ventana de
/// un minuto como mínimo y trunca los segundos (X0 D7, D9): se le da una de dos.
pub fn papelera_desde(c: &Cliente, objeto: &str, ahora: i64) -> Result<i64, Fallo> {
    let limite = ahora - PAPELERA.as_micros() as i64;
    let fin = ahora - 60_000_000;
    let ini = fin - 120_000_000;
    let ruta = c.api(&format!(
        "/sobjects/{objeto}/deleted/?start={}&end={}",
        codificar(&valores::texto_de_instante(ini)),
        codificar(&valores::texto_de_instante(fin))
    ));
    let v: serde_json::Value = serde_json::from_str(&c.get(&ruta)?).unwrap_or_default();
    let primera = v["earliestDateAvailable"]
        .as_str()
        .and_then(|t| valores::instante(t).ok());
    Ok(primera.map_or(limite, |p| p.max(limite)))
}

/// **La hora y la Papelera de un rango de cambios** (ADR 0083 §1.8). Con un rango
/// por cursor:
///
/// - `end`, si falta, es **la hora de la org ahora**, tomada antes de leer: así
///   el testigo y las filas son el mismo instante (ADR 0016);
/// - con `_ore_op`, `start` se **retrasa** [`Politica::relectura`]: re-entregar el
///   borde es idempotente con clave y cubre las filas que comparten segundo
///   (X0 D1) y las transacciones largas que confirman tarde;
/// - y si el `start` es anterior a lo que la Papelera guarda, **se niega**: los
///   borrados de entre medias ya no se ven, y servir el resto sería una copia
///   que no se entera de lo retirado y no avisa.
fn preparar_rango(c: &Cliente, p: &mut Peticion, pol: &Politica) -> Result<(), Fallo> {
    let con_rango = p.cursor.is_some() && (p.start.is_some() || p.end.is_some());
    if !con_rango {
        return Ok(());
    }
    let ahora = c.ahora()?;
    if p.end.is_none() {
        p.end = Some(valores::texto_de_instante(ahora));
    }
    let operacion = p.proyeccion.iter().any(|(_, k)| k == COLUMNA_OPERACION);
    if !operacion {
        return Ok(());
    }
    if let Some(st) = p.start.clone() {
        let desde = valores::instante(&st).map_err(|m| Fallo::operador(format!("`start`: {m}")))?;
        let cubre = papelera_desde(c, &p.objeto, ahora)?;
        if desde < cubre {
            return Err(Fallo::operador(format!(
                "el rango empieza en {} y la Papelera de Salesforce solo guarda los borrados desde {}: los de entre medias ya no se ven. Hace falta una copia completa",
                valores::texto_de_instante(desde),
                valores::texto_de_instante(cubre)
            )));
        }
        p.start = Some(
            valores::retrasar(&st, pol.relectura)
                .map_err(|m| Fallo::operador(format!("`start`: {m}")))?,
        );
    }
    Ok(())
}

/// `leer` con la política a elegir (las pruebas fuerzan REST o Bulk).
pub fn leer_con(
    s: &mut Sesion,
    token: &Token,
    p: &Peticion,
    salida: &mut dyn Write,
    pol: &Politica,
) -> Result<u64, Fallo> {
    token.bandera.store(false, Ordering::SeqCst);
    Salesforce::CAPACIDADES.admite(p)?;
    let o = s.objeto(&p.objeto)?.clone();
    let ctl = Control {
        token,
        limite: p
            .timeout_ms
            .map(|ms| Instant::now() + Duration::from_millis(ms)),
    };
    presupuesto(&s.cliente)?;
    let mut p = p.clone();
    preparar_rango(&s.cliente, &mut p, pol)?;
    let t = traducir(&p, &o)?;
    let mut sal = Salida::nueva(salida, &t, &o)?;
    // Un `limit` corto no necesita Bulk, aunque la consulta tenga muchas filas.
    let acotada = t.limit.is_some_and(|n| n <= pol.umbral);
    for q in &t.consultas {
        ctl.vigilar()?;
        let parar = leer_consulta(
            &s.cliente,
            q,
            &t.columnas,
            &ctl,
            pol,
            false,
            acotada,
            &mut |celdas| sal.fila(celdas),
        )?;
        if parar {
            break;
        }
    }
    // Los borrados, detrás de las altas: un registro borrado no sale de `query`.
    if let Some(q) = &t.borrados {
        ctl.vigilar()?;
        let clave = vec!["Id".to_string()];
        leer_consulta(
            &s.cliente,
            q,
            &clave,
            &ctl,
            pol,
            true,
            true,
            &mut |celdas| {
                let id = celdas[0]
                    .as_deref()
                    .ok_or_else(|| Fallo::origen("un borrado sin `Id`"))?;
                sal.baja(id)?;
                Ok(true)
            },
        )?;
    }
    sal.terminar()
}

impl Origen for Salesforce {
    type Sesion = Sesion;
    type Token = Token;

    const CAPACIDADES: Capacidades = Capacidades {
        conector: "ore-read-salesforce",
        version: env!("CARGO_PKG_VERSION"),
        operadores: OPERADORES,
        limit: true,
        order_by: true,
        estimar: false,
        servir: true,
    };

    fn conectar(url_fuente: &str) -> Result<Sesion, Fallo> {
        Ok(Sesion {
            cliente: abrir(url_fuente)?,
            esquemas: HashMap::new(),
        })
    }

    fn leer(
        s: &mut Sesion,
        token: &Token,
        p: &Peticion,
        salida: &mut dyn Write,
    ) -> Result<u64, Fallo> {
        leer_con(s, token, p, salida, &Politica::del_entorno())
    }

    fn estimar(_: &mut Sesion, _: &Peticion) -> Result<String, Fallo> {
        Err(Fallo::operador("este origen todavía no estima"))
    }

    fn cancelar(token: &Token) {
        token.bandera.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// F3/F4: la org de laboratorio, 15.000 al día; el tope es el 80 %.
    #[test]
    fn el_tope_se_pasa_en_el_porcentaje_y_no_antes() {
        assert!(!excedido((11_999, 15_000), 80));
        assert!(excedido((12_000, 15_000), 80));
        assert!(excedido((15_000, 15_000), 80));
        assert!(!excedido((0, 15_000), 80));
        // Sin máximo conocido no se puede decir que se pasó.
        assert!(!excedido((10, 0), 80));
        assert!(excedido((1, 100), 1));
    }

    #[test]
    fn el_control_corta_si_se_cancela_o_se_acaba_el_tiempo() {
        let t = Token::default();
        let ctl = Control {
            token: &t,
            limite: None,
        };
        assert!(ctl.vigilar().is_ok());
        Salesforce::cancelar(&t);
        assert!(ctl.vigilar().unwrap_err().mensaje.contains("cancelada"));

        let t = Token::default();
        let ya = Control {
            token: &t,
            limite: Some(Instant::now()),
        };
        let e = ya.vigilar().unwrap_err();
        assert_eq!(e.codigo, Codigo::Tiempo);
        let luego = Control {
            token: &t,
            limite: Some(Instant::now() + Duration::from_secs(60)),
        };
        assert!(luego.vigilar().is_ok());
    }

    /// Una celda de REST es el texto que Bulk daría: un solo convertidor detrás.
    #[test]
    fn una_celda_de_rest_es_el_texto_de_bulk() {
        let v: serde_json::Value = serde_json::from_str(
            r#"{"a":null,"b":"NA","c":10,"d":20.0,"e":false,"f":"2026-10-31T20:39:17.000+0000"}"#,
        )
        .unwrap();
        assert_eq!(celda(&v["a"]), None);
        assert_eq!(celda(&v["b"]).as_deref(), Some("NA"));
        assert_eq!(celda(&v["c"]).as_deref(), Some("10"));
        assert_eq!(celda(&v["d"]).as_deref(), Some("20.0"));
        assert_eq!(celda(&v["e"]).as_deref(), Some("false"));
        assert_eq!(celda(&v["z"]), None, "un campo ausente es un nulo");
    }

    #[test]
    fn el_modo_sale_del_entorno() {
        // Sin variable (o con una que no es `rest`/`bulk`): automático.
        assert_eq!(Modo::Auto, Modo::Auto);
        assert_eq!(natural("ORE_SALESFORCE_NO_EXISTE_9", 7), 7);
    }
}
