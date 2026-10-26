//! **El banco de Oracle**: las tablas `KIT_*` en el esquema del propio usuario de un Oracle de pruebas (el
//! contenedor `gvenzl/oracle-free`, o cualquier Oracle 23ai), cargadas por el mismo driver thin con SQL.
//!
//! `ORACLE_KIT_URL` es la de un usuario **que escribe** (`oracle://usuario:clave@host:1521/FREEPDB1`): el kit
//! carga la semilla con ella y el conector lee con la misma. Necesita `CREATE TABLE`, `CREATE VIEW` y
//! `CREATE PROCEDURE`. Las tablas se llaman `KIT_TIPOS`, `KIT_VACIA`, `KIT_GRANDE` y la vista `KIT_LENTA`, para
//! no pisar las del usuario.
//!
//! **Lo que la semilla pierde en Oracle, y es lo que el kit mide:**
//!
//! - **la cadena vacía es nulo** ([`Banco::vacio_es_nulo`]): `''` se guarda como `NULL`;
//! - `Boolean` es el `BOOLEAN` de 23ai (en versiones anteriores sería un `NUMBER(1)`);
//! - `Integer` es un `NUMBER(19, 0)`: el borde de un `i64` tiene 19 dígitos, y la tabla lo declara `Integer`.
//!
//! `grande` (10⁶ filas) se carga **dentro de Oracle** con un `CREATE TABLE … AS SELECT` (unos segundos); se
//! rehace solo si no está entera. La vista `lenta` llama a una función que duerme 10 s por fila (los casos 8 y 10).
//!
//! `sesiones` y `consultas_vivas` cuentan en `V$SESSION`, que el usuario ve **solo si se le concedió**
//! (`GRANT SELECT ON v_$session`); sin eso salen «no aplica», que es la verdad.
//!
//! Detrás de la feature `oracle`: arrastra `ore-oracle` (el conector).

use super::Banco;
use crate::semilla::{self, FILAS, GRANDE, TIPOS, Tabla};
use ore_base::json::Json;
use ore_base::tipos::{Fisico, Valor};
use ore_oracle::transporte::Conexion;
use std::collections::BTreeMap;

pub struct Oracle {
    url: String,
    c: Conexion,
    /// El esquema del usuario (mayúsculas), donde viven las tablas `KIT_*`.
    esquema: String,
}

impl Oracle {
    pub fn new(url: &str) -> Result<Oracle, String> {
        let c = Conexion::abrir(url).map_err(|f| {
            format!(
                "no se llega al Oracle de pruebas: {}",
                f.tapado(url).mensaje
            )
        })?;
        let esquema = c
            .usuario()
            .map_err(|f| format!("no se pudo preguntar el usuario: {}", f.mensaje))?;
        Ok(Oracle {
            url: url.to_string(),
            c,
            esquema,
        })
    }

    fn sql(&self, sql: &str) -> Result<(), String> {
        self.c
            .c
            .execute(sql, &[])
            .map(|_| ())
            .map_err(ore_oracle::transporte::fallo_de)
            .map_err(|f| format!("la semilla no se carga ({}): {}", corto(sql), f.mensaje))
    }

    /// Una orden que puede fallar sin que importe (un `DROP` de lo que no estaba).
    fn intentar(&self, sql: &str) {
        let _ = self.c.c.execute(sql, &[]);
    }

    fn contar(&self, sql: &str) -> Result<i64, String> {
        let f = self
            .c
            .c
            .query_row(sql, &[])
            .map_err(ore_oracle::transporte::fallo_de)
            .map_err(|f| format!("{}: {}", corto(sql), f.mensaje))?;
        f.get::<oracledb::OracleNumber>(0)
            .map_err(ore_oracle::transporte::fallo_de)
            .map_err(|f| f.mensaje)?
            .to_string()
            .parse::<i64>()
            .map_err(|_| format!("{} no contestó un entero", corto(sql)))
    }
}

fn corto(s: &str) -> String {
    s.chars().take(120).collect()
}

/// El tipo de Oracle con que se guarda cada tipo de la semilla.
fn oracle_de(tipo: &str) -> Result<&'static str, String> {
    Ok(match tipo {
        // El borde de un `i64` (±9 223 372 036 854 775 807) tiene 19 dígitos.
        "Integer" => "NUMBER(19,0)",
        "Float" => "BINARY_DOUBLE",
        "Boolean" => "BOOLEAN",
        "String" => "VARCHAR2(4000)",
        "Decimal<12, 2>" => "NUMBER(12,2)",
        "Date" => "DATE",
        "DateTime" => "TIMESTAMP(6)",
        "DateTimeTz" => "TIMESTAMP(6) WITH TIME ZONE",
        otro => return Err(format!("la semilla no usa `{otro}`")),
    })
}

/// El literal de Oracle de un valor de la semilla (texto canónico de OOS), como lo guardaría quien tiene esos
/// datos en Oracle.
fn literal(tipo: &str, v: &str) -> Result<String, String> {
    let malo = || format!("`{v}` no es `{tipo}`");
    let fisico = |t: &str| {
        let t = ore_base::types::parse_type(t).map_err(|_| malo())?;
        Ok::<Fisico, String>(Fisico::de(&t))
    };
    Ok(match tipo {
        "Integer" => v.parse::<i64>().map_err(|_| malo())?.to_string(),
        "Float" => format!(
            "TO_BINARY_DOUBLE('{}')",
            v.parse::<f64>().map_err(|_| malo())?
        ),
        "Boolean" => v.to_ascii_uppercase(),
        // Las comillas se doblan; un salto de línea dentro de un literal es válido en Oracle.
        "String" => format!("'{}'", v.replace('\'', "''")),
        "Decimal<12, 2>" => v.to_string(),
        "Date" => match fisico("Date")?.analizar(v) {
            Some(d @ Valor::Fecha(_)) => format!("DATE '{}'", d.texto(&Fisico::Fecha)),
            _ => return Err(malo()),
        },
        "DateTime" => match fisico("DateTime")?.analizar(v) {
            Some(d @ Valor::FechaHora(_)) => {
                format!("TIMESTAMP '{}'", d.texto(&Fisico::FechaHora))
            }
            _ => return Err(malo()),
        },
        "DateTimeTz" => match fisico("DateTimeTz")?.analizar(v) {
            // `2026-10-04 10:00:00+00` → `2026-10-04 10:00:00 +00:00`.
            Some(d @ Valor::Instante(_)) => {
                let t = d.texto(&Fisico::Instante);
                let t = t.strip_suffix("+00").unwrap_or(&t);
                format!("TIMESTAMP '{t} +00:00'")
            }
            _ => return Err(malo()),
        },
        otro => return Err(format!("la semilla no usa `{otro}`")),
    })
}

impl Banco for Oracle {
    fn familia(&self) -> &'static str {
        "oracle"
    }

    fn cargar(&mut self) -> Result<(), String> {
        let columnas = TIPOS
            .iter()
            .map(|c| {
                Ok(format!(
                    "\"{}\" {}{}",
                    c.nombre,
                    oracle_de(c.tipo)?,
                    if c.nombre == "id" { " NOT NULL" } else { "" }
                ))
            })
            .collect::<Result<Vec<_>, String>>()?
            .join(", ");
        for nombre in ["KIT_TIPOS", "KIT_VACIA"] {
            self.intentar(&format!("DROP TABLE {nombre} PURGE"));
            self.sql(&format!(
                "CREATE TABLE {nombre} ({columnas}, CONSTRAINT {nombre}_PK PRIMARY KEY (\"id\"))"
            ))?;
        }
        for fila in FILAS {
            let mut vals = Vec::new();
            for (c, v) in TIPOS.iter().zip(fila.iter()) {
                vals.push(match v {
                    // El vacío es nulo en Oracle: se carga como lo guardaría él.
                    Some(v) if !v.is_empty() || c.tipo != "String" => literal(c.tipo, v)?,
                    _ => "NULL".to_string(),
                });
            }
            self.sql(&format!(
                "INSERT INTO KIT_TIPOS VALUES ({})",
                vals.join(", ")
            ))?;
        }
        self.c.c.commit().map_err(|e| e.to_string())?;

        // `grande` son 10⁶ filas: solo se rehace si no está entera.
        let hecha = self.contar("SELECT COUNT(*) FROM user_tables WHERE table_name = 'KIT_GRANDE'");
        let enteras = matches!(hecha, Ok(1))
            && self.contar("SELECT COUNT(*) FROM KIT_GRANDE").ok() == Some(GRANDE as i64);
        if !enteras {
            self.intentar("DROP VIEW KIT_LENTA");
            self.intentar("DROP TABLE KIT_GRANDE PURGE");
            self.sql(
                "CREATE TABLE KIT_GRANDE (\"id\" NUMBER(19,0) NOT NULL PRIMARY KEY, \"grupo\" NUMBER(19,0), \
                 \"importe\" NUMBER(12,2), \"nota\" VARCHAR2(30)) NOLOGGING",
            )?;
            self.sql(&format!(
                "INSERT /*+ APPEND */ INTO KIT_GRANDE \
                 SELECT n, MOD(n, 100), n / 100, 'fila-' || TO_CHAR(n) \
                 FROM (SELECT (a.n - 1) * 1000 + b.n AS n \
                       FROM (SELECT LEVEL n FROM dual CONNECT BY LEVEL <= {}) a, \
                            (SELECT LEVEL n FROM dual CONNECT BY LEVEL <= 1000) b)",
                GRANDE / 1000
            ))?;
            self.c.c.commit().map_err(|e| e.to_string())?;
        }

        // Lo que tarda ~10 s sin dar una fila (casos 8 y 10): una función que duerme, por fila.
        self.sql(
            "CREATE OR REPLACE FUNCTION KIT_DORMIR RETURN NUMBER IS \
             BEGIN DBMS_SESSION.SLEEP(10); RETURN 1; END;",
        )?;
        self.sql(
            "CREATE OR REPLACE VIEW KIT_LENTA AS SELECT g.* FROM KIT_GRANDE g WHERE KIT_DORMIR() = 1",
        )?;

        let n = self.contar("SELECT COUNT(*) FROM KIT_TIPOS")?;
        if n != semilla::FILAS.len() as i64 {
            return Err(format!("`KIT_TIPOS` tiene {n} filas"));
        }
        let g = self.contar("SELECT COUNT(*) FROM KIT_GRANDE")?;
        if g != GRANDE as i64 {
            return Err(format!(
                "`KIT_GRANDE` tiene {g} filas y debería tener {GRANDE}: bórrala y vuelve a cargar"
            ));
        }
        Ok(())
    }

    fn url(&self) -> String {
        self.url.clone()
    }

    /// El contenedor de pruebas tiene un solo usuario: no hay otra credencial que probar.
    fn url_alternativa(&self) -> Option<String> {
        None
    }

    /// La misma URL con otra contraseña: Oracle la rechaza (`ORA-01017`).
    fn url_mala(&self) -> Option<String> {
        let (esquema, resto) = self.url.split_once("://")?;
        let (credencial, destino) = resto.rsplit_once('@')?;
        let usuario = credencial.split(':').next()?;
        Some(format!("{esquema}://{usuario}:contrasena-mala@{destino}"))
    }

    /// Lo que el motor manda y el dato no lleva: los tipos que la tabla declara (`tipos`). Sin ellos un
    /// `NUMBER(19, 0)` es un decimal y un `DATE` lleva hora.
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
        format!(
            "{}.KIT_{}",
            self.esquema,
            tabla.nombre().to_ascii_uppercase()
        )
    }

    /// Un objeto que tarda en dar su primera fila (casos 8 y 10).
    fn lenta(&self) -> Option<String> {
        Some(format!("{}.KIT_LENTA", self.esquema))
    }

    /// Las consultas activas de otra sesión del mismo usuario: lo único que el conector abre. Necesita ver
    /// `V$SESSION`.
    fn consultas_vivas(&mut self, _objeto: &str) -> Option<u64> {
        self.contar(
            "SELECT COUNT(*) FROM v$session WHERE username = USER AND type = 'USER' \
             AND status = 'ACTIVE' AND audsid <> SYS_CONTEXT('USERENV', 'SESSIONID')",
        )
        .ok()
        .map(|n| n as u64)
    }

    fn sesiones(&mut self) -> Option<u64> {
        self.contar(
            "SELECT COUNT(*) FROM v$session WHERE username = USER AND type = 'USER' \
             AND audsid <> SYS_CONTEXT('USERENV', 'SESSIONID')",
        )
        .ok()
        .map(|n| n as u64)
    }

    fn vacio_es_nulo(&self) -> bool {
        true
    }
}
