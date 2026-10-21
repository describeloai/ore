//! **El banco de Phoenix**: el esquema `KIT` en un Phoenix de pruebas (el laboratorio de
//! `pruebas-de-fuego/phoenix/`, o cualquier PQS), cargado por el propio Phoenix Query Server con SQL.
//!
//! `PHOENIX_KIT_URL` es la de un PQS **que escribe** (`phoenix://host:8765`): el kit carga la semilla con ella y
//! el conector lee con la misma. El laboratorio no tiene autenticación, ni ACL, ni consultas que se puedan
//! contar desde fuera: `url_alternativa`, `url_mala`, `sesiones` y `consultas_vivas` salen «no aplica».
//!
//! **Lo que la semilla pierde en Phoenix, y es lo que el kit mide** (ficha 0071 §3.12):
//!
//! - **la cadena vacía es nulo** ([`Banco::vacio_es_nulo`]): `''` se guarda como `NULL`;
//! - **un instante son milisegundos** por Avatica ([`Banco::milisegundos`]): se cargan truncados a ms, que es lo
//!   que el conector va a poder ver y comparar;
//! - `DateTime` y `DateTimeTz` son el mismo `TIMESTAMP`: los distingue lo que la tabla declara (`tipos`).
//!
//! `grande` (10⁶ filas) se carga **dentro de Phoenix**, duplicando con `UPSERT … SELECT` (≈ 70 s la primera vez);
//! una marca dice que está entera y las siguientes ejecuciones no la rehacen.
//!
//! Detrás de la feature `phoenix`: arrastra `ore-phoenix` (el conector), que no es cosa de quien solo prueba Postgres.

use super::Banco;
use crate::semilla::{self, FILAS, GRANDE, TIPOS, Tabla};
use ore_avatica::Valor as Celda;
use ore_core::json::Json;
use ore_core::tipos::{Fisico, Valor};
use ore_phoenix::traductor::literal_texto;
use ore_phoenix::transporte::Conexion;
use std::collections::BTreeMap;

const ESQUEMA: &str = "KIT";

pub struct Phoenix {
    url: String,
    c: Conexion,
}

impl Phoenix {
    pub fn new(url: &str) -> Result<Phoenix, String> {
        let c = Conexion::abrir(url).map_err(|f| {
            format!(
                "no se llega al Phoenix de pruebas: {}",
                f.tapado(url).mensaje
            )
        })?;
        // Sin esto, un `UPSERT` queda sin confirmar en la sesión.
        c.avatica
            .autocommit(true)
            .map_err(|e| format!("no se pudo activar el autocommit: {e}"))?;
        Ok(Phoenix {
            url: url.to_string(),
            c,
        })
    }

    fn sql(&self, sql: &str) -> Result<(), String> {
        self.c
            .avatica
            .ejecutar_orden(sql)
            .map(|_| ())
            .map_err(ore_phoenix::transporte::fallo_de)
            .map_err(|f| format!("la semilla no se carga ({}): {}", corto(sql), f.mensaje))
    }

    fn filas(&self, sql: &str) -> Result<Vec<Vec<Celda>>, String> {
        self.c
            .consulta_entera(sql)
            .map_err(|f| format!("{}: {}", corto(sql), f.mensaje))
    }

    fn contar(&self, sql: &str) -> Result<i64, String> {
        match self.filas(sql)?.first().and_then(|f| f.first()) {
            Some(Celda::Entero(n)) => Ok(*n),
            otro => Err(format!("{} no contestó un número: {otro:?}", corto(sql))),
        }
    }
}

fn corto(s: &str) -> String {
    s.chars().take(120).collect()
}

/// El tipo de Phoenix con que se guarda cada tipo de la semilla.
fn phoenix_de(tipo: &str) -> Result<&'static str, String> {
    Ok(match tipo {
        "Integer" => "BIGINT",
        "Float" => "DOUBLE",
        "Boolean" => "BOOLEAN",
        "String" => "VARCHAR",
        "Decimal<12, 2>" => "DECIMAL(12,2)",
        "Date" => "DATE",
        // Phoenix solo tiene el instante: la hora local es el mismo `TIMESTAMP` en UTC.
        "DateTime" | "DateTimeTz" => "TIMESTAMP",
        otro => return Err(format!("la semilla no usa `{otro}`")),
    })
}

/// El literal de Phoenix de un valor de la semilla (texto canónico de OOS), como lo guardaría quien tiene esos
/// datos en Phoenix.
fn literal(tipo: &str, v: &str) -> Result<String, String> {
    let malo = || format!("`{v}` no es `{tipo}`");
    let fisico = |t: &str| {
        let t = ore_core::types::parse_type(t).map_err(|_| malo())?;
        Ok::<Fisico, String>(Fisico::de(&t))
    };
    Ok(match tipo {
        "Integer" => v.parse::<i64>().map_err(|_| malo())?.to_string(),
        "Float" => format!("{:?}", v.parse::<f64>().map_err(|_| malo())?),
        "Boolean" => v.to_ascii_uppercase(),
        "String" => literal_texto(v),
        "Decimal<12, 2>" => v.to_string(),
        "Date" => match fisico("Date")?.analizar(v) {
            Some(d @ Valor::Fecha(_)) => {
                format!(
                    "TO_DATE('{}', 'yyyy-MM-dd', 'GMT')",
                    d.texto(&Fisico::Fecha)
                )
            }
            _ => return Err(malo()),
        },
        // Milisegundos: lo que Avatica va a dejar ver. `TO_TIMESTAMP(…, 'SSSSSS')` los leería mal.
        "DateTime" => match fisico("DateTime")?.analizar(v) {
            Some(Valor::FechaHora(us)) => instante(us),
            _ => return Err(malo()),
        },
        "DateTimeTz" => match fisico("DateTimeTz")?.analizar(v) {
            Some(Valor::Instante(us)) => instante(us),
            _ => return Err(malo()),
        },
        otro => return Err(format!("la semilla no usa `{otro}`")),
    })
}

fn instante(us: i64) -> String {
    let ms = us.div_euclid(1000) * 1000;
    format!(
        "TIMESTAMP '{}'",
        Valor::FechaHora(ms).texto(&Fisico::FechaHora)
    )
}

impl Banco for Phoenix {
    fn familia(&self) -> &'static str {
        "phoenix"
    }

    fn cargar(&mut self) -> Result<(), String> {
        let columnas = TIPOS
            .iter()
            .map(|c| {
                Ok(format!(
                    "\"{}\" {}{}",
                    c.nombre,
                    phoenix_de(c.tipo)?,
                    if c.nombre == "id" { " NOT NULL" } else { "" }
                ))
            })
            .collect::<Result<Vec<_>, String>>()?
            .join(", ");
        for nombre in ["TIPOS", "VACIA"] {
            self.sql(&format!("DROP TABLE IF EXISTS {ESQUEMA}.{nombre}"))?;
            self.sql(&format!(
                "CREATE TABLE {ESQUEMA}.{nombre} ({columnas}, CONSTRAINT pk PRIMARY KEY (\"id\"))"
            ))?;
        }
        for fila in FILAS {
            let mut vals = Vec::new();
            for (c, v) in TIPOS.iter().zip(fila.iter()) {
                vals.push(match v {
                    // El vacío es nulo en Phoenix: se carga como lo guardaría él.
                    Some(v) if !v.is_empty() || c.tipo != "String" => literal(c.tipo, v)?,
                    _ => "NULL".to_string(),
                });
            }
            self.sql(&format!(
                "UPSERT INTO {ESQUEMA}.TIPOS VALUES ({})",
                vals.join(", ")
            ))?;
        }

        // `grande` son 10⁶ filas: solo se rehace si no está entera, y lo dice una marca.
        self.sql(&format!(
            "CREATE TABLE IF NOT EXISTS {ESQUEMA}.MARCAS (\"k\" VARCHAR NOT NULL PRIMARY KEY, \"v\" BIGINT)"
        ))?;
        let hecha = self
            .filas(&format!(
                "SELECT \"v\" FROM {ESQUEMA}.MARCAS WHERE \"k\" = 'grande'"
            ))?
            .first()
            .and_then(|f| f.first().cloned())
            == Some(Celda::Entero(GRANDE as i64));
        if !hecha {
            self.sql(&format!("DROP VIEW IF EXISTS {ESQUEMA}.LENTA"))?;
            self.sql(&format!("DROP TABLE IF EXISTS {ESQUEMA}.GRANDE"))?;
            self.sql(&format!(
                "CREATE TABLE {ESQUEMA}.GRANDE (\"id\" BIGINT NOT NULL PRIMARY KEY, \"grupo\" BIGINT, \"importe\" DECIMAL(12,2), \"nota\" VARCHAR)"
            ))?;
            self.sql(&format!(
                "UPSERT INTO {ESQUEMA}.GRANDE VALUES (1, 1, 0.01, 'fila-1')"
            ))?;
            // Duplicar dentro de Phoenix: no sale ni un dato del servidor. `TO_CHAR(…, '#')`: sin la máscara mete
            // separadores de miles (`fila-1,001`) y `CAST(… AS VARCHAR)` no existe para un entero.
            let mut n = 1i64;
            while n < GRANDE as i64 {
                let hasta = n.min(GRANDE as i64 - n);
                self.sql(&format!(
                    "UPSERT INTO {ESQUEMA}.GRANDE (\"id\", \"grupo\", \"importe\", \"nota\") \
                     SELECT \"id\" + {n}, (\"id\" + {n}) % 100, CAST(\"id\" + {n} AS DECIMAL(12,2)) / 100, \
                     'fila-' || TO_CHAR(\"id\" + {n}, '#') FROM {ESQUEMA}.GRANDE WHERE \"id\" <= {hasta}"
                ))?;
                n += hasta;
            }
            self.sql(&format!(
                "UPSERT INTO {ESQUEMA}.MARCAS VALUES ('grande', {GRANDE})"
            ))?;
        }
        // Lo que tarda ~10 s sin dar una fila (casos 8 y 10): Phoenix no tiene `sleep` ni vistas con uniones, pero
        // un `REGEXP_SUBSTR` que no casa nunca, sobre 10⁶ filas, sí. **Que no case nunca**: el patrón de M4 acababa
        // en `9999` y sí casaba (280 filas, doce en los primeros 0,8 s). Con la lectura partida en paralelo (M5)
        // esas filas llegaban antes de que la pasarela cancelase y agotaban su presupuesto de filas (`404` en el
        // `DELETE`). Se recrea siempre: `IF NOT EXISTS` dejaría la definición vieja en un laboratorio ya sembrado.
        self.sql(&format!("DROP VIEW IF EXISTS {ESQUEMA}.LENTA"))?;
        self.sql(&format!(
            "CREATE VIEW {ESQUEMA}.LENTA AS SELECT * FROM {ESQUEMA}.GRANDE \
             WHERE REGEXP_SUBSTR(\"nota\", '(f|fi|fil|fila)+-[0-9]*x') IS NOT NULL"
        ))?;

        let n = self.contar(&format!("SELECT COUNT(*) FROM {ESQUEMA}.TIPOS"))?;
        if n != semilla::FILAS.len() as i64 {
            return Err(format!("`{ESQUEMA}.TIPOS` tiene {n} filas"));
        }
        let g = self.contar(&format!("SELECT COUNT(*) FROM {ESQUEMA}.GRANDE"))?;
        if g != GRANDE as i64 {
            return Err(format!(
                "`{ESQUEMA}.GRANDE` tiene {g} filas y debería tener {GRANDE}: bórrala y vuelve a cargar"
            ));
        }
        Ok(())
    }

    fn url(&self) -> String {
        self.url.clone()
    }

    /// El laboratorio no tiene autenticación: no hay otra credencial que probar.
    fn url_alternativa(&self) -> Option<String> {
        None
    }

    /// Ni una que se rechace: en un PQS sin autenticación la «mala» entraría.
    fn url_mala(&self) -> Option<String> {
        None
    }

    /// Lo que el motor manda y el dato no lleva: los tipos que la tabla declara (`tipos`). Sin ellos un
    /// `TIMESTAMP` es siempre un instante, un `DATE` lleva hora y un `DECIMAL` no tiene precisión.
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
        format!("{ESQUEMA}.{}", tabla.nombre().to_ascii_uppercase())
    }

    /// Un objeto que tarda en dar su primera fila (casos 8 y 10).
    fn lenta(&self) -> Option<String> {
        Some(format!("{ESQUEMA}.LENTA"))
    }

    fn milisegundos(&self) -> bool {
        true
    }

    fn vacio_es_nulo(&self) -> bool {
        true
    }
}
