//! **El banco de Cassandra**: el keyspace `kit` en un Cassandra de pruebas (un
//! `cassandra:5` en local o en el CI).
//!
//! `CASSANDRA_KIT_URL` es la de un rol que **escribe**: con ella se carga la semilla y
//! se pregunta por las sesiones. El conector lee con la misma URL. Un Cassandra sin
//! autenticación no tiene roles ni credenciales que comprobar, así que
//! `url_alternativa` y `url_mala` salen «no aplica» salvo que se diga:
//!
//! - `CASSANDRA_KIT_URL_ALT`: otra credencial al mismo servidor (caso 11);
//! - `CASSANDRA_KIT_AUTENTICADO=1`: el servidor comprueba la contraseña (caso 12).
//!
//! **Lo que la semilla pierde en Cassandra, y es lo que el kit mide:** un `timestamp`
//! es de milisegundos (la semilla trae microsegundos: [`Banco::milisegundos`]), y
//! `DateTime` (sin zona) y `DateTimeTz` son el mismo `timestamp`: lo que los distingue
//! es lo que la tabla declara en la petición (`tipos`), como en MongoDB.
//!
//! Detrás de la feature `cassandra`: el driver son ~110 crates y no es cosa de quien
//! solo prueba Postgres.

use super::Banco;
use crate::semilla::{self, FILAS, GRANDE, TIPOS, Tabla};
use ore_cassandra::transporte::{APLICACION, Conexion};
use ore_cassandra::{CqlDecimal, CqlValue};
use ore_core::json::Json;
use ore_core::tipos::{Fisico, Valor};
use std::collections::{BTreeMap, BTreeSet};

const KEYSPACE: &str = "kit";

pub struct Cassandra {
    url: String,
    c: Conexion,
}

impl Cassandra {
    pub fn new(url: &str) -> Result<Cassandra, String> {
        let c = Conexion::abrir_como(url, "ore-kit").map_err(|f| {
            format!(
                "no se llega al Cassandra de pruebas: {}",
                f.tapado(url).mensaje
            )
        })?;
        Ok(Cassandra {
            url: url.to_string(),
            c,
        })
    }

    fn cql(&self, cql: &str) -> Result<(), String> {
        self.c
            .ejecutar(cql)
            .map_err(|f| format!("la semilla no se carga ({cql}): {}", f.mensaje))
    }

    fn contar(&self, cql: &str) -> Result<i64, String> {
        let filas = self
            .c
            .consulta(cql)
            .map_err(|f| format!("{cql}: {}", f.mensaje))?;
        match filas.first().and_then(|f| f.first()).cloned().flatten() {
            Some(CqlValue::BigInt(n)) => Ok(n),
            otro => Err(format!("{cql} no contestó un número: {otro:?}")),
        }
    }
}

/// El tipo de CQL con que se guarda cada tipo de la semilla.
fn cql_de(tipo: &str) -> Result<&'static str, String> {
    Ok(match tipo {
        "Integer" => "bigint",
        "Float" => "double",
        "Boolean" => "boolean",
        "String" => "text",
        "Decimal<12, 2>" => "decimal",
        "Date" => "date",
        // Cassandra solo tiene el instante: la hora local es el mismo `timestamp` en UTC.
        "DateTime" | "DateTimeTz" => "timestamp",
        otro => return Err(format!("la semilla no usa `{otro}`")),
    })
}

/// Los bytes del entero sin escala de un decimal (complemento a dos, mínimos).
fn bytes_de(n: i64) -> Vec<u8> {
    let b = n.to_be_bytes();
    let mut i = 0;
    while i < 7
        && ((b[i] == 0x00 && b[i + 1] & 0x80 == 0) || (b[i] == 0xFF && b[i + 1] & 0x80 != 0))
    {
        i += 1;
    }
    b[i..].to_vec()
}

/// El literal de CQL de un valor de la semilla (texto canónico de OOS), como lo
/// guardaría quien tiene esos datos en Cassandra.
fn literal(tipo: &str, v: &str) -> Result<String, String> {
    let malo = || format!("`{v}` no es `{tipo}`");
    let fisico = |t: &str| {
        let t = ore_core::types::parse_type(t).map_err(|_| malo())?;
        Ok::<Fisico, String>(Fisico::de(&t))
    };
    Ok(match tipo {
        "Integer" => v.parse::<i64>().map_err(|_| malo())?.to_string(),
        "Float" => {
            // `NaN` e infinitos no están en la semilla: un literal decimal vale.
            v.parse::<f64>().map_err(|_| malo())?;
            v.to_string()
        }
        "Boolean" => v.to_string(),
        "String" => format!("'{}'", v.replace('\'', "''")),
        "Decimal<12, 2>" => v.to_string(),
        "Date" => match fisico("Date")?.analizar(v) {
            Some(d @ Valor::Fecha(_)) => format!("'{}'", d.texto(&Fisico::Fecha)),
            _ => return Err(malo()),
        },
        // Un `timestamp` es un entero de milisegundos: los microsegundos de la semilla
        // no llegan a existir en Cassandra.
        "DateTime" => match fisico("DateTime")?.analizar(v) {
            Some(Valor::FechaHora(us)) => us.div_euclid(1000).to_string(),
            _ => return Err(malo()),
        },
        "DateTimeTz" => match fisico("DateTimeTz")?.analizar(v) {
            Some(Valor::Instante(us)) => us.div_euclid(1000).to_string(),
            _ => return Err(malo()),
        },
        otro => return Err(format!("la semilla no usa `{otro}`")),
    })
}

impl Banco for Cassandra {
    fn familia(&self) -> &'static str {
        "cassandra"
    }

    fn cargar(&mut self) -> Result<(), String> {
        self.cql(&format!(
            "CREATE KEYSPACE IF NOT EXISTS {KEYSPACE} WITH replication = {{'class': 'SimpleStrategy', 'replication_factor': 1}}"
        ))?;
        let columnas = TIPOS
            .iter()
            .map(|c| Ok(format!("\"{}\" {}", c.nombre, cql_de(c.tipo)?)))
            .collect::<Result<Vec<_>, String>>()?
            .join(", ");
        for nombre in ["tipos", "vacia"] {
            self.cql(&format!("DROP TABLE IF EXISTS {KEYSPACE}.{nombre}"))?;
            self.cql(&format!(
                "CREATE TABLE {KEYSPACE}.{nombre} ({columnas}, PRIMARY KEY (\"id\"))"
            ))?;
        }
        for fila in FILAS {
            let mut cols = Vec::new();
            let mut vals = Vec::new();
            for (c, v) in TIPOS.iter().zip(fila.iter()) {
                // Un nulo se omite: insertarlo escribiría una lápida.
                if let Some(v) = v {
                    cols.push(format!("\"{}\"", c.nombre));
                    vals.push(literal(c.tipo, v)?);
                }
            }
            self.cql(&format!(
                "INSERT INTO {KEYSPACE}.tipos ({}) VALUES ({})",
                cols.join(", "),
                vals.join(", ")
            ))?;
        }

        // `grande` son 10⁶ filas: solo se rehace si no está entera, y lo dice una
        // marca (un `count(*)` de 10⁶ filas se pasa del tiempo de rango del servidor).
        self.cql(&format!(
            "CREATE TABLE IF NOT EXISTS {KEYSPACE}.marcas (k text PRIMARY KEY, v bigint)"
        ))?;
        let hecha = self
            .c
            .consulta(&format!(
                "SELECT v FROM {KEYSPACE}.marcas WHERE k = 'grande'"
            ))
            .map_err(|f| f.mensaje)?
            .first()
            .and_then(|f| f.first().cloned().flatten())
            == Some(CqlValue::BigInt(GRANDE as i64));
        if !hecha {
            self.cql(&format!("DROP TABLE IF EXISTS {KEYSPACE}.grande"))?;
            self.cql(&format!(
                "CREATE TABLE {KEYSPACE}.grande (id bigint PRIMARY KEY, grupo bigint, importe decimal, nota text)"
            ))?;
            let mut g = 1i64;
            while g <= GRANDE as i64 {
                let hasta = (g + 20_000 - 1).min(GRANDE as i64);
                let lote: Vec<Vec<CqlValue>> = (g..=hasta)
                    .map(|i| {
                        vec![
                            CqlValue::BigInt(i),
                            CqlValue::BigInt(i % 100),
                            // importe = i / 100, con dos decimales: el entero sin escala es `i`.
                            CqlValue::Decimal(CqlDecimal::from_signed_be_bytes_and_exponent(
                                bytes_de(i),
                                2,
                            )),
                            CqlValue::Text(format!("fila-{i}")),
                        ]
                    })
                    .collect();
                self.c
                    .cargar(
                        &format!(
                            "INSERT INTO {KEYSPACE}.grande (id, grupo, importe, nota) VALUES (?, ?, ?, ?)"
                        ),
                        lote,
                        256,
                    )
                    .map_err(|f| format!("la semilla `grande` no se carga: {}", f.mensaje))?;
                g = hasta + 1;
            }
            self.cql(&format!(
                "INSERT INTO {KEYSPACE}.marcas (k, v) VALUES ('grande', {GRANDE})"
            ))?;
        }
        let n = self.contar(&format!("SELECT count(*) FROM {KEYSPACE}.tipos"))?;
        if n != semilla::FILAS.len() as i64 {
            return Err(format!("`{KEYSPACE}.tipos` tiene {n} filas"));
        }
        Ok(())
    }

    fn url(&self) -> String {
        self.url.clone()
    }

    fn url_alternativa(&self) -> Option<String> {
        std::env::var("CASSANDRA_KIT_URL_ALT")
            .ok()
            .filter(|u| !u.is_empty())
    }

    /// La misma URL con una contraseña que el servidor rechaza. Solo si el servidor
    /// comprueba contraseñas: en uno sin autenticación la «mala» entraría.
    fn url_mala(&self) -> Option<String> {
        if std::env::var("CASSANDRA_KIT_AUTENTICADO").ok().as_deref() != Some("1") {
            return None;
        }
        let (esquema, resto) = self.url.split_once("://")?;
        let (cred, hosts) = resto.split_once('@')?;
        let (usuario, _) = cred.split_once(':')?;
        Some(format!("{esquema}://{usuario}:MALA@{hosts}"))
    }

    /// Lo que el motor manda y el dato no lleva: los tipos que la tabla declara
    /// (`tipos`). Sin ellos un `timestamp` es siempre un instante y un `decimal` no
    /// tiene precisión.
    fn completar(&self, tabla: Tabla, peticion: &mut BTreeMap<String, Json>) {
        peticion.insert(
            "tipos".into(),
            Json::Arr(
                tabla
                    .columnas()
                    .iter()
                    .map(|c| Json::Arr(vec![Json::s(c.nombre), Json::s(c.tipo)]))
                    .collect(),
            ),
        );
    }

    fn objeto(&self, tabla: Tabla) -> String {
        format!("{KEYSPACE}.{}", tabla.nombre())
    }

    fn milisegundos(&self) -> bool {
        true
    }

    /// Los **clientes** del conector con conexiones abiertas, contados por su
    /// `CLIENT_ID` en `system_views.clients` (el conector se presenta con
    /// [`APLICACION`] y un id distinto por sesión): un cliente de Cassandra abre varias
    /// conexiones, y una sesión no son sus conexiones.
    fn sesiones(&mut self) -> Option<u64> {
        let filas = self
            .c
            .consulta("SELECT client_options FROM system_views.clients")
            .ok()?;
        let mut ids = BTreeSet::new();
        for fila in filas {
            let Some(CqlValue::Map(pares)) = fila.into_iter().next().flatten() else {
                continue;
            };
            let texto = |k: &str| {
                pares.iter().find_map(|(a, b)| match (a, b) {
                    (CqlValue::Text(a), CqlValue::Text(b)) if a == k => Some(b.clone()),
                    _ => None,
                })
            };
            if texto("APPLICATION_NAME").as_deref() == Some(APLICACION)
                && let Some(id) = texto("CLIENT_ID")
            {
                ids.insert(id);
            }
        }
        Some(ids.len() as u64)
    }
}
