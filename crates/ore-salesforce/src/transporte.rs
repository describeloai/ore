//! **La red**: canjear el JWT, llamar a la API REST, reintentar, y contar la
//! cuota (ADR 0083 §1.7). Lo único del crate que abre un socket.
//!
//! - El `access_token` se guarda [`identidad::VIDA_DEL_TOKEN`] y se renueva al
//!   primer `401` (la org puede acortar la sesión).
//! - `429` y `5xx` se reintentan con la espera de `Retry-After` o creciente.
//! - **`REQUEST_LIMIT_EXCEEDED` no se reintenta**: es la cuota de 24 h (F3), y
//!   esperar segundos no la devuelve. Es un fallo del origen con su motivo.
//! - Cada respuesta trae `Sforce-Limit-Info: api-usage=n/max` (F4): se guarda, y
//!   es lo que lee el presupuesto de cuota ([`crate::lectura::excedido`]).

use crate::identidad;
use crate::url::Fuente;
use ore_driver::{Codigo, Fallo};
use rsa::RsaPrivateKey;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const AGENTE: &str = "ore-read-salesforce";
const REINTENTOS: u32 = 3;
const ESPERA_MAXIMA: u64 = 30;
const TIEMPO: Duration = Duration::from_secs(60);

/// Las peticiones que ha hecho este proceso (cada una cuenta en la cuota).
static PETICIONES: AtomicU64 = AtomicU64::new(0);

pub fn peticiones() -> u64 {
    PETICIONES.load(Ordering::Relaxed)
}

pub struct Cliente {
    pub fuente: Fuente,
    clave: RsaPrivateKey,
    agente: ureq::Agent,
    /// `(access_token, instance_url, cuándo se canjeó)`.
    token: Mutex<Option<(String, String, Instant)>>,
    uso: Mutex<Option<(u64, u64)>>,
}

/// Lo que contesta una llamada: el cuerpo y las cabeceras que interesan
/// (`Sforce-Locator` y `Sforce-NumberOfRecords` de Bulk, y `Date`: la hora de
/// la org).
#[derive(Debug, Clone, Default)]
pub struct Respuesta {
    pub cuerpo: String,
    cabeceras: Vec<(String, String)>,
}

impl Respuesta {
    /// Una cabecera, sin distinguir mayúsculas.
    pub fn cabecera(&self, nombre: &str) -> Option<&str> {
        self.cabeceras
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(nombre))
            .map(|(_, v)| v.as_str())
    }
}

/// `api-usage=25/15000` → `(25, 15000)`.
pub fn uso_de(cabecera: &str) -> Option<(u64, u64)> {
    let v = cabecera
        .split(',')
        .find_map(|p| p.trim().strip_prefix("api-usage="))?;
    let (n, max) = v.split_once('/')?;
    Some((n.trim().parse().ok()?, max.trim().parse().ok()?))
}

/// Una consulta SOQL, lista para ir en `?q=`: todo lo que no es de los
/// caracteres sin reservar se escapa.
pub fn codificar(s: &str) -> String {
    let mut o = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{b:02X}"));
        }
    }
    o
}

/// `[{"message":"…","errorCode":"MALFORMED_QUERY"}]` → `(MALFORMED_QUERY, …)`.
pub fn motivo_de(cuerpo: &str) -> (String, String) {
    let v: serde_json::Value = serde_json::from_str(cuerpo).unwrap_or_default();
    let e = v.get(0).unwrap_or(&v);
    let codigo = e["errorCode"].as_str().unwrap_or("").to_string();
    let m = e["message"].as_str().unwrap_or(cuerpo.trim());
    (codigo, m.lines().next().unwrap_or("").to_string())
}

impl Cliente {
    pub fn nuevo(fuente: Fuente, pem: &str) -> Result<Cliente, Fallo> {
        let clave = identidad::clave(pem).map_err(|m| Fallo::new(Codigo::Credencial, m))?;
        let tls = native_tls::TlsConnector::new().map_err(|e| {
            Fallo::new(
                Codigo::Conexion,
                format!("no se pudo abrir el TLS de la plataforma: {e}"),
            )
        })?;
        Ok(Cliente {
            fuente,
            clave,
            agente: ureq::AgentBuilder::new()
                .tls_connector(std::sync::Arc::new(tls))
                .timeout(TIEMPO)
                .user_agent(AGENTE)
                .build(),
            token: Mutex::new(None),
            uso: Mutex::new(None),
        })
    }

    /// `/services/data/v68.0<resto>`.
    pub fn api(&self, resto: &str) -> String {
        format!("/services/data/v{}{resto}", self.fuente.version)
    }

    /// **La hora de la org**, en microsegundos UTC, de la cabecera `Date` de una
    /// llamada barata (`/limits`). El testigo de una lectura por cursor se toma
    /// de aquí y no del reloj de quien lee: `SystemModstamp` lo pone Salesforce.
    pub fn ahora(&self) -> Result<i64, Fallo> {
        let r = self.enviar("GET", &self.api("/limits"), None, &[])?;
        let d = r
            .cabecera("date")
            .ok_or_else(|| Fallo::origen("Salesforce no dijo la hora (sin cabecera `Date`)"))?;
        crate::valores::instante_de_http(d).map_err(Fallo::origen)
    }

    /// `(usadas, máximo)` de la cuota diaria, de la última respuesta.
    pub fn uso(&self) -> Option<(u64, u64)> {
        *self.uso.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn canjear(&self) -> Result<(String, String), Fallo> {
        let a = identidad::aserto(
            &self.fuente.cliente,
            &self.fuente.usuario,
            &self.fuente.audiencia(),
            identidad::ahora(),
            &self.clave,
        );
        PETICIONES.fetch_add(1, Ordering::Relaxed);
        let r = self
            .agente
            .post(&format!("{}/services/oauth2/token", self.fuente.endpoint()))
            .send_form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", &a),
            ]);
        match r {
            Ok(x) => {
                let c = x.into_string().map_err(|e| {
                    Fallo::new(Codigo::Conexion, format!("no se pudo leer el canje: {e}"))
                })?;
                let v: serde_json::Value = serde_json::from_str(&c).map_err(|e| {
                    Fallo::new(
                        Codigo::Credencial,
                        format!("el canje no contestó JSON: {e}"),
                    )
                })?;
                let t = v["access_token"].as_str().ok_or_else(|| {
                    Fallo::new(Codigo::Credencial, "el canje no trajo `access_token`")
                })?;
                let inst = v["instance_url"]
                    .as_str()
                    .unwrap_or(&self.fuente.endpoint())
                    .to_string();
                Ok((t.to_string(), inst))
            }
            Err(ureq::Error::Status(c, x)) => {
                let cuerpo = x.into_string().unwrap_or_default();
                let f = Fallo::new(Codigo::Credencial, identidad::explicar_canje(c, &cuerpo));
                Err(if c >= 500 {
                    Fallo::new(Codigo::Conexion, f.mensaje).reintentable(true)
                } else {
                    f
                })
            }
            Err(e) => Err(Fallo::new(
                Codigo::Conexion,
                format!("Salesforce no contesta en `{}`: {e}", self.fuente.host),
            )),
        }
    }

    /// El token vigente y el sitio de la API. `renovar` lo tira y canjea otro.
    fn sesion(&self, renovar: bool) -> Result<(String, String), Fallo> {
        let mut g = self.token.lock().unwrap_or_else(|e| e.into_inner());
        if !renovar
            && let Some((t, i, cuando)) = g.as_ref()
            && cuando.elapsed() < identidad::VIDA_DEL_TOKEN
        {
            return Ok((t.clone(), i.clone()));
        }
        let (t, i) = self.canjear()?;
        *g = Some((t.clone(), i.clone(), Instant::now()));
        Ok((t, i))
    }

    /// Un `GET` a `ruta` (`/services/data/…`), con lo que la API pide.
    pub fn get(&self, ruta: &str) -> Result<String, Fallo> {
        self.get_con(ruta, &[])
    }

    pub fn get_con(&self, ruta: &str, cabeceras: &[(&str, &str)]) -> Result<String, Fallo> {
        Ok(self.enviar("GET", ruta, None, cabeceras)?.cuerpo)
    }

    /// Una llamada con cuerpo JSON opcional (`POST`, `PATCH`), con los
    /// reintentos y el renovado del token de siempre.
    pub fn enviar(
        &self,
        metodo: &str,
        ruta: &str,
        json: Option<&str>,
        cabeceras: &[(&str, &str)],
    ) -> Result<Respuesta, Fallo> {
        let (mut intento, mut renovado) = (0u32, false);
        loop {
            let (t, inst) = self.sesion(false)?;
            let url = if ruta.starts_with("http") {
                ruta.to_string()
            } else {
                format!("{inst}{ruta}")
            };
            let mut r = self
                .agente
                .request(metodo, &url)
                .set("authorization", &format!("Bearer {t}"));
            for (k, v) in cabeceras {
                r = r.set(k, v);
            }
            PETICIONES.fetch_add(1, Ordering::Relaxed);
            let hecho = match json {
                Some(c) => r.set("content-type", "application/json").send_string(c),
                None => r.call(),
            };
            let (f, espera) = match hecho {
                Ok(x) => {
                    if let Some(u) = x.header("sforce-limit-info").and_then(uso_de) {
                        *self.uso.lock().unwrap_or_else(|e| e.into_inner()) = Some(u);
                    }
                    let cabeceras = ["sforce-locator", "sforce-numberofrecords", "date"]
                        .iter()
                        .filter_map(|n| x.header(n).map(|v| (n.to_string(), v.to_string())))
                        .collect();
                    let cuerpo = x.into_string().map_err(|e| {
                        Fallo::new(
                            Codigo::Conexion,
                            format!("no se pudo leer la respuesta de Salesforce: {e}"),
                        )
                    })?;
                    return Ok(Respuesta { cuerpo, cabeceras });
                }
                Err(ureq::Error::Status(c, x)) => {
                    let espera = x.header("retry-after").and_then(|v| v.parse::<u64>().ok());
                    let cuerpo = x.into_string().unwrap_or_default();
                    let (codigo, m) = motivo_de(&cuerpo);
                    if c == 401 && !renovado {
                        renovado = true;
                        self.sesion(true)?;
                        continue;
                    }
                    let f = match (c, codigo.as_str()) {
                        (_, "REQUEST_LIMIT_EXCEEDED") => Fallo::new(
                            Codigo::Origen,
                            format!(
                                "se agotó la cuota diaria de API de la org (REQUEST_LIMIT_EXCEEDED): {m}"
                            ),
                        ),
                        (401, _) | (403, _) => {
                            Fallo::new(Codigo::Credencial, format!("{c} {codigo}: {m}"))
                        }
                        (404, _) => Fallo::new(Codigo::Objeto, format!("404 {codigo}: {m}")),
                        (400, _) => Fallo::new(Codigo::Operador, format!("{codigo}: {m}")),
                        (429, _) | (500..=599, _) => {
                            Fallo::new(Codigo::Origen, format!("{c} {codigo}: {m}"))
                                .reintentable(true)
                        }
                        _ => Fallo::new(Codigo::Origen, format!("{c} {codigo}: {m}")),
                    };
                    (f, espera)
                }
                Err(e) => (
                    Fallo::new(
                        Codigo::Conexion,
                        format!("Salesforce no contesta en `{}`: {e}", self.fuente.host),
                    )
                    .reintentable(true),
                    None,
                ),
            };
            if !f.reintentable || intento >= REINTENTOS {
                return Err(f);
            }
            std::thread::sleep(Duration::from_secs(
                espera.unwrap_or(1 << intento).min(ESPERA_MAXIMA),
            ));
            intento += 1;
        }
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// F4: lo que Salesforce dijo en X0.
    #[test]
    fn la_cuota_sale_de_la_cabecera() {
        assert_eq!(uso_de("api-usage=25/15000"), Some((25, 15000)));
        assert_eq!(
            uso_de("per-app-api-usage=1/9, api-usage=63/15000"),
            Some((63, 15000))
        );
        assert_eq!(uso_de("nada"), None);
    }

    #[test]
    fn una_consulta_se_codifica_para_el_q() {
        assert_eq!(
            codificar("SELECT Id FROM Account"),
            "SELECT%20Id%20FROM%20Account"
        );
        assert_eq!(codificar("a != 'ñ'"), "a%20%21%3D%20%27%C3%B1%27");
    }

    #[test]
    fn las_cabeceras_se_buscan_sin_mayusculas() {
        let r = Respuesta {
            cuerpo: String::new(),
            cabeceras: vec![("sforce-locator".into(), "abc".into())],
        };
        assert_eq!(r.cabecera("Sforce-Locator"), Some("abc"));
        assert_eq!(r.cabecera("otra"), None);
    }

    #[test]
    fn el_motivo_de_un_error_de_la_api() {
        let (c, m) = motivo_de(
            r#"[{"message":"\nSELECT x\n ^\nERROR at Row:1:Column:8\nNo such column","errorCode":"INVALID_FIELD"}]"#,
        );
        assert_eq!(c, "INVALID_FIELD");
        assert_eq!(
            m, "",
            "la primera línea de ese mensaje está vacía; se cita lo que hay"
        );
        let (c, m) = motivo_de(
            r#"[{"message":"Maximum SOQL offset allowed for apiName Account is 2000","errorCode":"NUMBER_OUTSIDE_VALID_RANGE"}]"#,
        );
        assert_eq!(c, "NUMBER_OUTSIDE_VALID_RANGE");
        assert!(m.contains("offset"));
    }
}
