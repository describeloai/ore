//! **La celda que llama, y su organización** (0058 P4·1).
//!
//! En ORE la celda es el workspace: lo que el cliente aprovisiona. Su `ore-serve`
//! se presenta aquí con su token de Workload Identity —firmado por Google, con
//! audiencia `ore-postgres`— y **la organización sale de la celda, nunca del
//! cuerpo**.
//!
//! Quién es de qué organización lo sabe `ore-iam` (`iam.celda`, que escribe el
//! aprovisionador), y se le pregunta a él: `POST /access/v1/celda` con el mismo
//! token. `ore-iam` lo verifica entero —firma, emisor, audiencia, plazo— y
//! contesta la celda y su organización. Aquí no hay un segundo registro de
//! celdas que se pueda desincronizar.
//!
//! ⭐ Se guarda la respuesta lo que `ore-iam` diga (`vale`, techo [`VALE_MAXIMO`])
//!   y nunca más allá del `exp` del token. La llave es el resumen del token, no
//!   el token.
//!
//! ⛔ Si `ore-iam` no contesta, **503**: se para la gestión, no los datos. Las
//!   bases siguen sirviendo, porque conectarse a ellas no pasa por aquí.

use ore_core::parse;
use ore_entrada::http::{Plazos, pedir_con};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

/// Dónde vive `ore-iam`, con punto final (0047 M2: ahorra los dominios de búsqueda).
pub const DESTINO: &str = "ore-iam.identidad.svc.cluster.local.:8090";

/// El techo de lo que se guarda una respuesta, diga lo que diga `vale`. Con esto,
/// una celda retirada deja de poder gestionar, como mucho, en medio minuto.
pub const VALE_MAXIMO: i64 = 30;

/// Cuántas respuestas se guardan como mucho; pasado el techo se vacía entera.
const GUARDADAS_MAXIMAS: usize = 10_000;

/// La celda, ya resuelta por `ore-iam`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Celda {
    pub id: String,
    pub nombre: String,
    pub organizacion: String,
}

/// Por qué no hay celda.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SinCelda {
    /// No trae token: 401.
    Ausente,
    /// Trae uno y no es de ninguna celda viva: 401, con el motivo de `ore-iam`.
    NoVale(String),
    /// `ore-iam` no contestó, o contestó algo que no se entiende: 503.
    SinIam(String),
}

impl SinCelda {
    pub fn codigo(&self) -> u16 {
        match self {
            SinCelda::Ausente | SinCelda::NoVale(_) => 401,
            SinCelda::SinIam(_) => 503,
        }
    }

    pub fn motivo(&self) -> String {
        match self {
            SinCelda::Ausente => "falta el token de la celda (`Authorization: Bearer …`)".into(),
            SinCelda::NoVale(m) => format!("el token no es de una celda: {m}"),
            SinCelda::SinIam(m) => {
                format!("no se puede saber de qué organización es la celda (ore-iam: {m})")
            }
        }
    }
}

/// De un `Authorization` a la celda.
pub trait Celdas: Send + Sync {
    fn de(&self, autorizacion: Option<&str>) -> Result<Celda, SinCelda>;
}

/// Preguntándoselo a `ore-iam`.
pub struct PorOreIam {
    destino: String,
    plazo: Duration,
    guardadas: Mutex<HashMap<[u8; 32], (Celda, i64)>>,
}

impl PorOreIam {
    pub fn nuevo(destino: &str) -> PorOreIam {
        PorOreIam {
            destino: destino.to_string(),
            plazo: Duration::from_secs(2),
            guardadas: Mutex::new(HashMap::new()),
        }
    }
}

impl Celdas for PorOreIam {
    fn de(&self, autorizacion: Option<&str>) -> Result<Celda, SinCelda> {
        let autorizacion = autorizacion
            .map(str::trim)
            .filter(|a| a.len() > "Bearer ".len())
            .ok_or(SinCelda::Ausente)?;
        let llave: [u8; 32] = Sha256::digest(autorizacion.as_bytes()).into();
        let ahora = ahora();
        if let Ok(g) = self.guardadas.lock()
            && let Some((c, hasta)) = g.get(&llave)
            && ahora < *hasta
        {
            return Ok(c.clone());
        }
        let (codigo, cuerpo) = pedir_con(
            "POST",
            &self.destino,
            "/access/v1/celda",
            &[("Authorization", autorizacion)],
            None,
            Plazos {
                conectar: self.plazo,
                responder: self.plazo,
            },
        )
        .map_err(SinCelda::SinIam)?;
        let (celda, hasta) = leer_respuesta(codigo, &cuerpo, ahora)?;
        if let Ok(mut g) = self.guardadas.lock() {
            if g.len() >= GUARDADAS_MAXIMAS {
                g.clear();
            }
            g.insert(llave, (celda.clone(), hasta));
        }
        Ok(celda)
    }
}

/// Lo que contestó `ore-iam`, y hasta cuándo vale guardarlo.
pub fn leer_respuesta(codigo: u16, cuerpo: &str, ahora: i64) -> Result<(Celda, i64), SinCelda> {
    let n = parse::parse(cuerpo).ok();
    let texto = |camino: &[&str]| -> Option<String> {
        let mut nodo = n.as_ref()?;
        for k in camino {
            nodo = nodo.get(k)?.1;
        }
        nodo.as_str().map(str::to_string)
    };
    match codigo {
        200 => {}
        401 => {
            return Err(SinCelda::NoVale(
                texto(&["error"]).unwrap_or_else(|| "ore-iam no la reconoce".into()),
            ));
        }
        c => {
            return Err(SinCelda::SinIam(format!(
                "{c}: {}",
                texto(&["error"]).unwrap_or_default()
            )));
        }
    }
    let (Some(id), Some(organizacion)) = (texto(&["celda", "id"]), texto(&["organizacion"])) else {
        return Err(SinCelda::SinIam(
            "la respuesta no trae `celda.id` y `organizacion`".into(),
        ));
    };
    let numero = |k: &str| texto(&[k]).and_then(|v| v.parse::<i64>().ok());
    let vale = numero("vale").unwrap_or(0).clamp(0, VALE_MAXIMO);
    let hasta = (ahora + vale).min(numero("vence").unwrap_or(ahora));
    Ok((
        Celda {
            id,
            nombre: texto(&["celda", "nombre"]).unwrap_or_default(),
            organizacion,
        },
        hasta,
    ))
}

/// Celdas fijas, por token: para las pruebas.
pub struct Fijas(pub HashMap<String, Celda>);

impl Celdas for Fijas {
    fn de(&self, autorizacion: Option<&str>) -> Result<Celda, SinCelda> {
        let a = autorizacion.ok_or(SinCelda::Ausente)?;
        let t = a.strip_prefix("Bearer ").unwrap_or(a);
        self.0
            .get(t)
            .cloned()
            .ok_or_else(|| SinCelda::NoVale("no es una de las fijas".into()))
    }
}

fn ahora() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn la_respuesta_de_ore_iam_da_la_celda_y_cuanto_se_guarda() {
        let r = r#"{"celda":{"id":"cel_1","nombre":"demo"},"organizacion":"org_a","producto":"ore-postgres","vence":1100,"vale":30}"#;
        let (c, hasta) = leer_respuesta(200, r, 1000).unwrap();
        assert_eq!(c.organizacion, "org_a");
        assert_eq!(c.nombre, "demo");
        assert_eq!(hasta, 1030);
        // Nunca más allá del token.
        let (_, hasta) = leer_respuesta(200, r, 1090).unwrap();
        assert_eq!(hasta, 1100);
    }

    #[test]
    fn vale_no_pasa_del_techo() {
        let r = r#"{"celda":{"id":"c"},"organizacion":"o","vence":99999,"vale":3600}"#;
        assert_eq!(leer_respuesta(200, r, 0).unwrap().1, VALE_MAXIMO);
    }

    #[test]
    fn un_401_es_que_no_vale_y_lo_demas_es_que_no_hay_ore_iam() {
        assert_eq!(
            leer_respuesta(401, r#"{"error":"retirada"}"#, 0).unwrap_err(),
            SinCelda::NoVale("retirada".into())
        );
        assert_eq!(leer_respuesta(502, "", 0).unwrap_err().codigo(), 503);
        assert_eq!(leer_respuesta(200, "{}", 0).unwrap_err().codigo(), 503);
    }

    #[test]
    fn sin_token_no_se_pregunta() {
        let p = PorOreIam::nuevo("127.0.0.1:9");
        assert_eq!(p.de(None).unwrap_err(), SinCelda::Ausente);
        assert_eq!(p.de(Some("Bearer ")).unwrap_err(), SinCelda::Ausente);
    }
}
