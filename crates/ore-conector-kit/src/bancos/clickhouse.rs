//! **El banco de ClickHouse**: las tablas `tipos`, `vacia` y `grande` y la vista `lenta` en una base propia de un ClickHouse de
//! pruebas (el contenedor `clickhouse/clickhouse-server`, desde 24.8), cargadas por el mismo transporte que usa el conector.
//!
//! `CLICKHOUSE_KIT_URL` es la de un usuario **que escribe** y **sin contraseña** (`clickhouse://kit@localhost:8123/kit`: el
//! conector no admite una contraseña por HTTP en claro); la base de la URL es donde viven las tablas. Necesita:
//!
//! ```sql
//! CREATE DATABASE kit;
//! CREATE USER kit IDENTIFIED WITH no_password;
//! GRANT ALL ON kit.* TO kit;
//! GRANT SELECT ON system.processes TO kit;   -- consultas_vivas (caso 10)
//! GRANT KILL QUERY ON *.* TO kit;            -- que el caso 10 pueda comprobar que la cancelación llega
//! GRANT SELECT ON system.parts TO kit;       -- la huella del verbo `cambio` (sin él, `respalda: false`)
//! ```
//!
//! **Lo que la semilla pierde en ClickHouse, y es lo que el kit mide:** nada de lo que importa a la semilla. La cadena vacía
//! **no** es nulo, `Integer` es un `Int64` entero, `Decimal<12, 2>` es un `Decimal(12, 2)` exacto, y las fechas y los instantes
//! se guardan en UTC (`Date32` y `DateTime64(6, 'UTC')`: el rango de ambos es 1900–2299).
//!
//! `grande` (10⁶ filas) se carga **dentro de ClickHouse** con un `INSERT … SELECT … FROM numbers()` (un segundo); se rehace solo si
//! no está entera. La vista `lenta` duerme 9 s por bloque antes de dar una fila (los casos 8 y 10).
//!
//! `sesiones` no aplica: HTTP no tiene sesión.
//!
//! Detrás de la feature `clickhouse`: arrastra `ore-clickhouse` (el conector).

use super::Banco;
use crate::semilla::{self, FILAS, GRANDE, TIPOS, Tabla};
use ore_base::tipos::{Fisico, Valor};
use ore_clickhouse::transporte::Cliente;

pub struct ClickHouse {
    url: String,
    c: Cliente,
    /// La base de la URL, donde viven las tablas `tipos`, `vacia` y `grande` y la vista `lenta`.
    base: String,
}

impl ClickHouse {
    pub fn new(url: &str) -> Result<ClickHouse, String> {
        let c = Cliente::abrir(url)
            .map_err(|f| format!("la URL de ClickHouse no sirve: {}", f.mensaje))?;
        let base = c.destino().base.clone().ok_or(
            "la URL de ClickHouse del kit lleva la base donde viven las tablas: `clickhouse://kit@localhost:8123/kit`",
        )?;
        // La base va entre comillas en el SQL: sólo caracteres que no las rompen.
        if !base.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(format!(
                "`{base}` no es un nombre de base que el kit admita (letras, números y `_`)"
            ));
        }
        // Se llega al servidor antes de seguir: un error aquí dice qué falta, y no «la semilla no se carga».
        c.texto("SELECT version()").map_err(|f| {
            format!(
                "no se llega al ClickHouse de pruebas: {} (¿existe el usuario sin contraseña y su base? ver la cabecera de bancos/clickhouse.rs)",
                f.tapado(url).mensaje
            )
        })?;
        Ok(ClickHouse {
            url: url.to_string(),
            c,
            base,
        })
    }

    fn sql(&self, sql: &str) -> Result<(), String> {
        self.c
            .ejecutar(sql)
            .map_err(|f| format!("la semilla no se carga ({}): {}", corto(sql), f.mensaje))
    }

    fn contar(&self, sql: &str) -> Result<i64, String> {
        let t = self
            .c
            .texto(sql)
            .map_err(|f| format!("{}: {}", corto(sql), f.mensaje))?;
        t.trim().parse::<i64>().map_err(|_| {
            format!(
                "{} no contestó un entero sino `{}`",
                corto(sql),
                corto(t.trim())
            )
        })
    }

    fn tabla(&self, nombre: &str) -> String {
        format!("{}.{}", self.base, nombre)
    }
}

fn corto(s: &str) -> String {
    s.chars().take(120).collect()
}

/// El tipo de ClickHouse con que se guarda cada tipo de la semilla.
fn clickhouse_de(tipo: &str) -> Result<&'static str, String> {
    Ok(match tipo {
        "Integer" => "Int64",
        "Float" => "Float64",
        "Boolean" => "Bool",
        "String" => "String",
        "Decimal<12, 2>" => "Decimal(12, 2)",
        "Date" => "Date32",
        // Los dos instantes se guardan en UTC: `local` es una hora de reloj sin zona, y se guarda tal cual.
        "DateTime" | "DateTimeTz" => "DateTime64(6, 'UTC')",
        otro => return Err(format!("la semilla no usa `{otro}`")),
    })
}

/// El literal de ClickHouse de un valor de la semilla (texto canónico de OOS).
fn literal(tipo: &str, v: &str) -> Result<String, String> {
    let malo = || format!("`{v}` no es `{tipo}`");
    let fisico = |t: &str| {
        let t = ore_base::types::parse_type(t).map_err(|_| malo())?;
        Ok::<Fisico, String>(Fisico::de(&t))
    };
    Ok(match tipo {
        "Integer" => v.parse::<i64>().map_err(|_| malo())?.to_string(),
        "Float" => {
            let f = v.parse::<f64>().map_err(|_| malo())?;
            if f.is_nan() {
                "nan".to_string()
            } else if f.is_infinite() {
                if f > 0.0 { "inf" } else { "-inf" }.to_string()
            } else {
                format!("{f:?}")
            }
        }
        "Boolean" => v.to_ascii_lowercase(),
        "String" => cadena(v),
        "Decimal<12, 2>" => v.to_string(),
        "Date" => match fisico("Date")?.analizar(v) {
            Some(d @ Valor::Fecha(_)) => format!("'{}'", d.texto(&Fisico::Fecha)),
            _ => return Err(malo()),
        },
        "DateTime" => match fisico("DateTime")?.analizar(v) {
            Some(d @ Valor::FechaHora(_)) => format!("'{}'", d.texto(&Fisico::FechaHora)),
            _ => return Err(malo()),
        },
        "DateTimeTz" => match fisico("DateTimeTz")?.analizar(v) {
            // `2026-10-04 10:00:00+00` → `'2026-10-04 10:00:00'` (UTC).
            Some(d @ Valor::Instante(_)) => {
                let t = d.texto(&Fisico::Instante);
                format!("'{}'", t.strip_suffix("+00").unwrap_or(&t))
            }
            _ => return Err(malo()),
        },
        otro => return Err(format!("la semilla no usa `{otro}`")),
    })
}

/// Un literal de cadena de ClickHouse: la barra, la comilla, el salto de línea, el tabulador y el NUL se escapan.
fn cadena(v: &str) -> String {
    let mut s = String::with_capacity(v.len() + 2);
    s.push('\'');
    for c in v.chars() {
        match c {
            '\\' => s.push_str("\\\\"),
            '\'' => s.push_str("\\'"),
            '\n' => s.push_str("\\n"),
            '\r' => s.push_str("\\r"),
            '\t' => s.push_str("\\t"),
            '\0' => s.push_str("\\0"),
            c => s.push(c),
        }
    }
    s.push('\'');
    s
}

impl Banco for ClickHouse {
    fn familia(&self) -> &'static str {
        "clickhouse"
    }

    fn cargar(&mut self) -> Result<(), String> {
        let columnas = TIPOS
            .iter()
            .map(|c| {
                let t = clickhouse_de(c.tipo)?;
                // `id` identifica las filas que vuelven: no es nulo nunca. Las demás columnas sí pueden serlo.
                Ok(if c.nombre == "id" {
                    format!("`{}` {t}", c.nombre)
                } else {
                    format!("`{}` Nullable({t})", c.nombre)
                })
            })
            .collect::<Result<Vec<_>, String>>()?
            .join(", ");
        for nombre in ["tipos", "vacia"] {
            let t = self.tabla(nombre);
            self.sql(&format!("DROP TABLE IF EXISTS {t}"))?;
            self.sql(&format!(
                "CREATE TABLE {t} ({columnas}) ENGINE = MergeTree ORDER BY `id`"
            ))?;
        }
        let mut filas = Vec::new();
        for fila in FILAS {
            let mut vals = Vec::new();
            for (c, v) in TIPOS.iter().zip(fila.iter()) {
                vals.push(match v {
                    // La cadena vacía **no** es nulo en ClickHouse: se carga como lo que es.
                    Some(v) => literal(c.tipo, v)?,
                    None => "NULL".to_string(),
                });
            }
            filas.push(format!("({})", vals.join(", ")));
        }
        self.sql(&format!(
            "INSERT INTO {} VALUES {}",
            self.tabla("tipos"),
            filas.join(", ")
        ))?;

        // `grande` son 10⁶ filas: sólo se rehace si no está entera.
        let g = self.tabla("grande");
        let enteras = self.contar(&format!("SELECT count() FROM {g}")).ok() == Some(GRANDE as i64);
        if !enteras {
            self.sql(&format!("DROP VIEW IF EXISTS {}", self.tabla("lenta")))?;
            self.sql(&format!("DROP TABLE IF EXISTS {g}"))?;
            self.sql(&format!(
                "CREATE TABLE {g} (`id` Int64, `grupo` Nullable(Int64), `importe` Nullable(Decimal(12, 2)), `nota` Nullable(String)) \
                 ENGINE = MergeTree ORDER BY `id`"
            ))?;
            // importe = id / 100 con dos decimales, **exacto**: por texto, no por una división de coma flotante.
            self.sql(&format!(
                "INSERT INTO {g} SELECT toInt64(n), toInt64(n % 100), \
                 toDecimal64(concat(toString(intDiv(n, 100)), '.', leftPad(toString(n % 100), 2, '0')), 2), \
                 concat('fila-', toString(n)) \
                 FROM (SELECT number + 1 AS n FROM numbers({GRANDE}))"
            ))?;
        }

        // Lo que tarda ~9 s sin dar una fila (casos 8 y 10): `sleep` admite 3 s como máximo por llamada, y se evalúa una vez por bloque.
        self.sql(&format!(
            "CREATE OR REPLACE VIEW {} AS SELECT `id`, `grupo`, `importe`, `nota` FROM {g} \
             WHERE sleep(3) + sleep(3) + sleep(3) = 0",
            self.tabla("lenta")
        ))?;

        let n = self.contar(&format!("SELECT count() FROM {}", self.tabla("tipos")))?;
        if n != semilla::FILAS.len() as i64 {
            return Err(format!(
                "`tipos` tiene {n} filas y debería tener {}",
                FILAS.len()
            ));
        }
        let n = self.contar(&format!("SELECT count() FROM {g}"))?;
        if n != GRANDE as i64 {
            return Err(format!(
                "`grande` tiene {n} filas y debería tener {GRANDE}: bórrala y vuelve a cargar"
            ));
        }
        Ok(())
    }

    fn url(&self) -> String {
        self.url.clone()
    }

    /// El servidor de pruebas tiene un solo usuario que escribe: no hay otra credencial que probar.
    fn url_alternativa(&self) -> Option<String> {
        None
    }

    /// Un usuario que no existe: ClickHouse lo rechaza (`AUTHENTICATION_FAILED`, 516). No hay una contraseña que equivocar:
    /// el conector no admite una contraseña por HTTP en claro.
    fn url_mala(&self) -> Option<String> {
        let (esquema, resto) = self.url.split_once("://")?;
        let (_, destino) = resto.rsplit_once('@')?;
        Some(format!("{esquema}://nadie-existe@{destino}"))
    }

    /// Lo que el motor manda y el servidor no distingue: `local` y `instante` son el mismo `DateTime64(6, 'UTC')`, y los tipos que
    /// la tabla declara (`tipos`, ADR 0069 M3) dicen cuál es la hora de reloj. Con catálogo manda el catálogo; esto sólo desempata.
    fn completar(
        &self,
        tabla: Tabla,
        peticion: &mut std::collections::BTreeMap<String, ore_base::json::Json>,
    ) {
        use ore_base::json::Json;
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
        self.tabla(tabla.nombre())
    }

    /// Un objeto que tarda en dar su primera fila (casos 8 y 10).
    fn lenta(&self) -> Option<String> {
        Some(self.tabla("lenta"))
    }

    /// Las consultas del usuario que siguen en marcha sobre `objeto`. Necesita ver `system.processes`.
    fn consultas_vivas(&mut self, objeto: &str) -> Option<u64> {
        if !objeto
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
        {
            return None;
        }
        self.contar(&format!(
            "SELECT count() FROM system.processes WHERE replaceAll(query, '`', '') LIKE '%{objeto}%' AND query NOT LIKE '%system.processes%'"
        ))
        .ok()
        .map(|n| n as u64)
    }

    // `sesiones` no aplica (HTTP no tiene sesión); `que_escribe` y `escrituras`, de momento, tampoco.
}
