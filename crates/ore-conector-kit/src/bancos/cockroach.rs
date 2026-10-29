//! **El banco de CockroachDB**: las tablas `tipos`, `vacia` y `grande` y la vista `lenta` en la base de la URL, de un CockroachDB de
//! pruebas (el contenedor `cockroachdb/cockroach`, desde 22.2), cargadas por la misma sesión que abre el conector (`ore_cockroach::tls`).
//!
//! `COCKROACH_KIT_URL` es la de un usuario **que escribe** (`cockroachdb://kit@localhost:26259/kit`); la base de la URL es donde viven
//! las tablas, en el esquema `public`. Con `pruebas-de-fuego/cockroach/laboratorio.sh`:
//!
//! * nodo `--insecure`: `kit` sin contraseña y `ORE_COCKROACH_LABORATORIO=1` (el conector no habla en claro fuera del laboratorio);
//! * nodo seguro: `cockroachdb://kit:kit-secreto@localhost:26270/kit?sslrootcert=<PEM codificado>` (la CA que deja el script).
//!
//! **Lo que la semilla pierde en CockroachDB, y es lo que el kit mide:** nada de lo que importa a la semilla. La cadena vacía **no** es
//! nulo, `Integer` es un `INT8` entero, `Decimal<12, 2>` es un `DECIMAL(12,2)` exacto, y `TIMESTAMPTZ` guarda el instante (se lee en UTC).
//!
//! `grande` (10⁶ filas) se carga **dentro de CockroachDB** con `generate_series`; sólo se rehace si no está entera. La vista `lenta`
//! duerme 9 s antes de dar su fila (los casos 8 y 10).
//!
//! `que_escribe` (caso 9) no aplica: una función que escribe en una vista pide funciones definidas por el usuario (23.1 o posterior), y el
//! suelo es 22.2. `sesiones` (caso 11) mira `SHOW CLUSTER SESSIONS`: en 22.2 exige `VIEWACTIVITY` (lo da `laboratorio.sh`).
//!
//! Detrás de la feature `cockroach`: arrastra `ore-cockroach` (el conector).

use super::Banco;
use crate::semilla::{self, FILAS, GRANDE, TIPOS, Tabla};
use ore_cockroach::tls;
use postgres::Client;

pub struct Cockroach {
    url: String,
    cliente: Client,
}

/// El tipo de CockroachDB con que se guarda cada tipo de la semilla.
fn ddl(tipo: &str) -> Result<&'static str, String> {
    Ok(match tipo {
        "Integer" => "INT8",
        "Float" => "FLOAT8",
        "Boolean" => "BOOL",
        "String" => "STRING",
        "Decimal<12, 2>" => "DECIMAL(12,2)",
        "Date" => "DATE",
        "DateTime" => "TIMESTAMP",
        "DateTimeTz" => "TIMESTAMPTZ",
        otro => return Err(format!("la semilla no usa `{otro}`")),
    })
}

fn corto(s: &str) -> String {
    s.chars().take(120).collect()
}

impl Cockroach {
    pub fn new(url: &str) -> Result<Cockroach, String> {
        let abierta = tls::abrir(url).map_err(|f| {
            format!(
                "no se llega al CockroachDB de pruebas: {} (¿existe `kit` y su base? ver pruebas-de-fuego/cockroach/laboratorio.sh)",
                f.tapado(url).mensaje
            )
        })?;
        let mut cliente = abierta.cliente;
        // El conector abre la sesión de sólo lectura; el banco carga la semilla, así que escribe.
        cliente
            .batch_execute("SET default_transaction_read_only = false")
            .map_err(|e| {
                format!(
                    "no se pudo abrir la sesión de escritura: {}",
                    tls::causa(&e)
                )
            })?;
        Ok(Cockroach {
            url: url.to_string(),
            cliente,
        })
    }

    fn sql(&mut self, sql: &str) -> Result<(), String> {
        self.cliente.batch_execute(sql).map_err(|e| {
            format!(
                "la semilla no se carga ({}): {}",
                corto(sql),
                tls::causa(&e)
            )
        })
    }

    fn contar(&mut self, sql: &str) -> Result<i64, String> {
        self.cliente
            .query_one(sql, &[])
            .map(|f| f.get::<_, i64>(0))
            .map_err(|e| format!("{}: {}", corto(sql), tls::causa(&e)))
    }
}

impl Banco for Cockroach {
    fn familia(&self) -> &'static str {
        "cockroach"
    }

    fn cargar(&mut self) -> Result<(), String> {
        let columnas = TIPOS
            .iter()
            .map(|c| {
                let t = ddl(c.tipo)?;
                // `id` identifica las filas que vuelven: no es nulo nunca.
                Ok(if c.nombre == "id" {
                    format!("{} {t} NOT NULL PRIMARY KEY", c.nombre)
                } else {
                    format!("{} {t}", c.nombre)
                })
            })
            .collect::<Result<Vec<_>, String>>()?
            .join(", ");
        self.sql("DROP VIEW IF EXISTS lenta")?;
        for nombre in ["tipos", "vacia"] {
            self.sql(&format!("DROP TABLE IF EXISTS {nombre}"))?;
            self.sql(&format!("CREATE TABLE {nombre} ({columnas})"))?;
        }
        let marcas: Vec<String> = TIPOS
            .iter()
            .enumerate()
            .map(|(i, c)| Ok(format!("${}::STRING::{}", i + 1, ddl(c.tipo)?)))
            .collect::<Result<_, String>>()?;
        let insertar = format!("INSERT INTO tipos VALUES ({})", marcas.join(", "));
        for fila in FILAS {
            // La cadena vacía **no** es nulo en CockroachDB: llega como lo que es.
            let ps: Vec<&(dyn postgres::types::ToSql + Sync)> = fila
                .iter()
                .map(|v| v as &(dyn postgres::types::ToSql + Sync))
                .collect();
            self.cliente
                .execute(insertar.as_str(), &ps)
                .map_err(|e| format!("una fila de la semilla no entra: {}", tls::causa(&e)))?;
        }

        // `grande` son 10⁶ filas: sólo se rehace si no está entera.
        let enteras = self.contar("SELECT count(*) FROM grande").ok() == Some(GRANDE as i64);
        if !enteras {
            self.sql("DROP TABLE IF EXISTS grande")?;
            self.sql(
                "CREATE TABLE grande (id INT8 NOT NULL PRIMARY KEY, grupo INT8, importe DECIMAL(12,2), nota STRING)",
            )?;
            // Por tandas: una transacción de 10⁶ filas es grande para un nodo, y 2·10⁵ entran sin pestañear.
            const TANDA: u64 = 200_000;
            let mut desde = 1;
            while desde <= GRANDE {
                let hasta = (desde + TANDA - 1).min(GRANDE);
                // importe = id / 100 con dos decimales, **exacto**: DECIMAL, no coma flotante.
                self.sql(&format!(
                    "INSERT INTO grande SELECT g, g % 100, (g::DECIMAL / 100)::DECIMAL(12,2), 'fila-' || g::STRING \
                     FROM generate_series({desde}, {hasta}) AS g"
                ))?;
                desde = hasta + 1;
            }
        }

        // El caso 14 (`estimar`) lee las estadísticas de la tabla: una tabla recién cargada no las tiene hasta que CockroachDB las crea sola.
        self.sql("CREATE STATISTICS ore_kit FROM grande")?;

        // Lo que tarda ~9 s en dar su fila (casos 8 y 10).
        self.sql("CREATE VIEW lenta AS SELECT 1::INT8 AS id FROM generate_series(1, 1) AS g WHERE pg_sleep(9)")?;

        let n = self.contar("SELECT count(*) FROM tipos")?;
        if n != semilla::FILAS.len() as i64 {
            return Err(format!(
                "`tipos` tiene {n} filas y debería tener {}",
                FILAS.len()
            ));
        }
        let n = self.contar("SELECT count(*) FROM grande")?;
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

    /// El servidor de pruebas tiene un solo usuario que escribe en la base del kit: no hay otra credencial que probar.
    fn url_alternativa(&self) -> Option<String> {
        None
    }

    /// Una credencial que CockroachDB rechaza (`28P01`): la contraseña equivocada si la URL lleva una (nodo seguro), y si no, un
    /// usuario que no existe (nodo `--insecure`, que contesta lo mismo).
    fn url_mala(&self) -> Option<String> {
        let (esquema, resto) = self.url.split_once("://")?;
        let (credencial, destino) = resto.rsplit_once('@')?;
        Some(match credencial.split_once(':') {
            Some((usuario, _)) => format!("{esquema}://{usuario}:una-clave-que-no-es@{destino}"),
            None => format!("{esquema}://nadie-existe@{destino}"),
        })
    }

    fn objeto(&self, tabla: Tabla) -> String {
        format!("public.{}", tabla.nombre())
    }

    /// Un objeto que tarda en dar su primera fila (casos 8 y 10).
    fn lenta(&self) -> Option<String> {
        Some("public.lenta".into())
    }

    /// Cuántas sesiones del conector hay abiertas en el servidor (caso 11). Cuenta también la del propio banco, que es constante: el caso
    /// compara antes y después.
    fn sesiones(&mut self) -> Option<u64> {
        self.contar(
            "SELECT count(*) FROM [SHOW CLUSTER SESSIONS] WHERE application_name = 'ore-read-cockroach'",
        )
        .ok()
        .map(|n| n as u64)
    }

    /// Las consultas de la sesión del conector que siguen en marcha sobre `objeto`.
    fn consultas_vivas(&mut self, objeto: &str) -> Option<u64> {
        let tabla = objeto.rsplit('.').next().unwrap_or(objeto);
        if !tabla.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return None;
        }
        self.contar(&format!(
            "SELECT count(*) FROM [SHOW CLUSTER QUERIES] WHERE application_name = 'ore-read-cockroach' \
             AND query LIKE '%{tabla}%' AND query NOT LIKE '%CLUSTER QUERIES%'"
        ))
        .ok()
        .map(|n| n as u64)
    }
}
