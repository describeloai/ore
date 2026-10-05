//! **Los valores por defecto** (`docs/federation.md` §5), con su sitio en el
//! entorno para que una celda —o el kit— los cambie.

use std::time::Duration;

/// El presupuesto de una lectura: lo primero que llegue la corta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Presupuesto {
    pub filas: u64,
    pub bytes: u64,
    pub ms: u64,
}

#[derive(Debug, Clone)]
pub struct Cotas {
    /// El de una lectura que no trae el suyo: 100 000 filas o 64 MB, 30 s.
    pub presupuesto: Presupuesto,
    /// Lecturas a la vez por origen: 4.
    pub concurrencia: usize,
    /// Las que esperan detrás: 16.
    pub cola: usize,
    /// Lo que una espera dura como mucho: 10 s.
    pub espera: Duration,
    /// Un conector caliente sin uso se cierra a los 60 s.
    pub ociosa: Duration,
    /// Tras cancelar, lo que se espera a que el conector suelte antes de matarlo.
    pub soltar: Duration,
    /// 0053 F8 · el de una **copia** (`perfil: "copia"`): sin tope de filas ni
    /// de bytes que valga la pena, y 30 min.
    pub copia: Presupuesto,
    /// Lo que una copia espera en la cola: 5 min (la rehace un Job, no una persona).
    pub espera_copia: Duration,
    /// 0053 F8 · lo que dura como mucho catalogar, comprobar, explorar o el testigo: 10 min.
    pub verbo: Duration,
}

impl Default for Cotas {
    fn default() -> Cotas {
        Cotas {
            presupuesto: Presupuesto {
                filas: 100_000,
                bytes: 64 << 20,
                ms: 30_000,
            },
            concurrencia: 4,
            cola: 16,
            espera: Duration::from_secs(10),
            ociosa: Duration::from_secs(60),
            soltar: Duration::from_secs(5),
            copia: Presupuesto {
                filas: 1 << 40,
                bytes: 1 << 44,
                ms: 30 * 60_000,
            },
            espera_copia: Duration::from_secs(300),
            verbo: Duration::from_secs(600),
        }
    }
}

impl Cotas {
    /// Las de por defecto, con lo que diga el entorno (`ORE_FED_FILAS`,
    /// `ORE_FED_BYTES`, `ORE_FED_MS`, `ORE_FED_CONCURRENCIA`, `ORE_FED_COLA`,
    /// `ORE_FED_ESPERA_MS`, `ORE_FED_OCIOSA_MS`). Un valor que no es un número
    /// positivo es un error: arrancar con otro presupuesto del que se dijo, en
    /// silencio, protegería al origen de menos.
    pub fn del_entorno() -> Result<Cotas, String> {
        let mut c = Cotas::default();
        let n = |k: &str| -> Result<Option<u64>, String> {
            match std::env::var(k) {
                Err(_) => Ok(None),
                Ok(v) => v
                    .trim()
                    .parse::<u64>()
                    .ok()
                    .filter(|n| *n > 0)
                    .map(Some)
                    .ok_or_else(|| format!("`{k}={v}` no es un número positivo")),
            }
        };
        if let Some(v) = n("ORE_FED_FILAS")? {
            c.presupuesto.filas = v;
        }
        if let Some(v) = n("ORE_FED_BYTES")? {
            c.presupuesto.bytes = v;
        }
        if let Some(v) = n("ORE_FED_MS")? {
            c.presupuesto.ms = v;
        }
        if let Some(v) = n("ORE_FED_CONCURRENCIA")? {
            c.concurrencia = v as usize;
        }
        if let Some(v) = n("ORE_FED_COLA")? {
            c.cola = v as usize;
        }
        if let Some(v) = n("ORE_FED_ESPERA_MS")? {
            c.espera = Duration::from_millis(v);
        }
        if let Some(v) = n("ORE_FED_OCIOSA_MS")? {
            c.ociosa = Duration::from_millis(v);
        }
        if let Some(v) = n("ORE_FED_COPIA_MS")? {
            c.copia.ms = v;
        }
        if let Some(v) = n("ORE_FED_ESPERA_COPIA_MS")? {
            c.espera_copia = Duration::from_millis(v);
        }
        if let Some(v) = n("ORE_FED_VERBO_MS")? {
            c.verbo = Duration::from_millis(v);
        }
        Ok(c)
    }
}
