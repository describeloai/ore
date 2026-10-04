//! **El banco de Postgres**: el esquema `kit` en un Postgres de pruebas (el
//! servicio del CI, o un contenedor en local), y dos roles de lectura.
//!
//! `PG_URL` es la de un administrador: con ella se carga la semilla y se
//! pregunta por las sesiones. El conector nunca la ve: lee con `kit_lector`, y
//! el caso 11 abre otra con `kit_otro`.
//!
//! `kit_lector` **puede** insertar en `kit.marcas`: así, lo único que impide
//! que leer `kit.escribe` escriba es la sesión de sólo lectura del conector, y
//! eso es lo que el caso 9 prueba. Sin el permiso, el caso pasaría por el
//! permiso y no por el conector.

use super::Banco;
use crate::semilla::{self, FILAS, GRANDE, TIPOS, Tabla};
use postgres::{Client, NoTls};

const LECTOR: (&str, &str) = ("kit_lector", "kit-lector-clave");
const OTRO: (&str, &str) = ("kit_otro", "kit-otro-clave");

pub struct Postgres {
    admin_url: String,
    cliente: Client,
}

/// El tipo de Postgres de cada tipo de OOS de la semilla.
fn ddl(tipo: &str) -> &'static str {
    match tipo {
        "Integer" => "bigint",
        "Decimal<12, 2>" => "numeric(12,2)",
        "Float" => "double precision",
        "String" => "text",
        "Date" => "date",
        "DateTime" => "timestamp",
        "DateTimeTz" => "timestamptz",
        "Boolean" => "boolean",
        otro => panic!("la semilla no usa `{otro}`"),
    }
}

/// `PG_URL` con otro usuario y otra clave.
fn con_usuario(url: &str, usuario: &str, clave: &str) -> String {
    let (esquema, resto) = url.split_once("://").unwrap_or(("postgres", url));
    let resto = resto.rsplit_once('@').map_or(resto, |(_, h)| h);
    format!("{esquema}://{usuario}:{clave}@{resto}")
}

impl Postgres {
    pub fn new(admin_url: &str) -> Result<Postgres, String> {
        let cliente = Client::connect(admin_url, NoTls)
            .map_err(|e| format!("no se llega al Postgres de pruebas: {e}"))?;
        Ok(Postgres {
            admin_url: admin_url.to_string(),
            cliente,
        })
    }

    fn uno(&mut self, sql: &str) -> Result<i64, String> {
        self.cliente
            .query_one(sql, &[])
            .map(|f| f.get::<_, i64>(0))
            .map_err(|e| format!("{e}"))
    }

    const ROLES: &'static str = "'kit_lector', 'kit_otro'";
}

impl Banco for Postgres {
    fn familia(&self) -> &'static str {
        "postgres"
    }

    fn cargar(&mut self) -> Result<(), String> {
        let columnas = TIPOS
            .iter()
            .map(|c| format!("{} {}", c.nombre, ddl(c.tipo)))
            .collect::<Vec<_>>()
            .join(", ");
        let roles = [LECTOR, OTRO]
            .iter()
            .map(|(u, c)| {
                format!(
                    "DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '{u}') \
                     THEN CREATE ROLE {u} LOGIN; END IF; END $$; \
                     ALTER ROLE {u} LOGIN PASSWORD '{c}';"
                )
            })
            .collect::<String>();
        // `grande` sólo se rehace si no está entera: son 10⁶ filas.
        let grande_bien = self
            .uno(
                "SELECT CASE WHEN to_regclass('kit.grande') IS NULL THEN 0 \
                 ELSE (SELECT count(*) FROM kit.grande) END::bigint",
            )
            .unwrap_or(0)
            == GRANDE as i64;
        let mut sql = String::from("CREATE SCHEMA IF NOT EXISTS kit;");
        sql.push_str(&roles);
        sql.push_str(
            "DROP VIEW IF EXISTS kit.lenta, kit.escribe; \
             DROP FUNCTION IF EXISTS kit.marca(); \
             DROP TABLE IF EXISTS kit.tipos, kit.vacia, kit.marcas;",
        );
        sql.push_str(&format!(
            "CREATE TABLE kit.tipos ({columnas}); CREATE TABLE kit.vacia ({columnas});"
        ));
        if !grande_bien {
            sql.push_str(&format!(
                "DROP TABLE IF EXISTS kit.grande; \
                 CREATE TABLE kit.grande AS SELECT g::bigint AS id, (g % 100)::bigint AS grupo, \
                 (g::numeric / 100)::numeric(12,2) AS importe, 'fila-' || g AS nota \
                 FROM generate_series(1, {GRANDE}) g;"
            ));
        }
        sql.push_str(
            "CREATE VIEW kit.lenta AS SELECT 1::bigint AS id FROM pg_sleep(20); \
             CREATE TABLE kit.marcas (cuando timestamptz); \
             CREATE FUNCTION kit.marca() RETURNS bigint LANGUAGE sql VOLATILE \
               AS $$ INSERT INTO kit.marcas VALUES (now()) RETURNING 1::bigint $$; \
             CREATE VIEW kit.escribe AS SELECT kit.marca() AS id;",
        );
        for (u, _) in [LECTOR, OTRO] {
            sql.push_str(&format!(
                "GRANT USAGE ON SCHEMA kit TO {u}; \
                 GRANT SELECT ON ALL TABLES IN SCHEMA kit TO {u}; \
                 GRANT INSERT ON kit.marcas TO {u}; \
                 GRANT EXECUTE ON FUNCTION kit.marca() TO {u};"
            ));
        }
        self.cliente
            .batch_execute(&sql)
            .map_err(|e| format!("la semilla no se carga: {e}"))?;

        let marcas: Vec<String> = TIPOS
            .iter()
            .enumerate()
            .map(|(i, c)| format!("${}::text::{}", i + 1, ddl(c.tipo)))
            .collect();
        let insertar = format!("INSERT INTO kit.tipos VALUES ({})", marcas.join(", "));
        for fila in FILAS {
            let ps: Vec<&(dyn postgres::types::ToSql + Sync)> = fila
                .iter()
                .map(|v| v as &(dyn postgres::types::ToSql + Sync))
                .collect();
            self.cliente
                .execute(insertar.as_str(), &ps)
                .map_err(|e| format!("una fila de la semilla no entra: {e}"))?;
        }
        // Que lo que hay es lo que la semilla dice.
        let n = self.uno("SELECT count(*) FROM kit.tipos")?;
        if n != semilla::FILAS.len() as i64 {
            return Err(format!("`kit.tipos` tiene {n} filas"));
        }
        Ok(())
    }

    fn url(&self) -> String {
        con_usuario(&self.admin_url, LECTOR.0, LECTOR.1)
    }

    fn url_alternativa(&self) -> Option<String> {
        Some(con_usuario(&self.admin_url, OTRO.0, OTRO.1))
    }

    fn url_mala(&self) -> Option<String> {
        Some(con_usuario(
            &self.admin_url,
            LECTOR.0,
            "una-clave-que-no-es",
        ))
    }

    fn objeto(&self, tabla: Tabla) -> String {
        format!("kit.{}", tabla.nombre())
    }

    fn lenta(&self) -> Option<String> {
        Some("kit.lenta".into())
    }

    fn que_escribe(&self) -> Option<String> {
        Some("kit.escribe".into())
    }

    fn escrituras(&mut self) -> Option<Result<u64, String>> {
        Some(
            self.uno("SELECT count(*) FROM kit.marcas")
                .map(|n| n as u64)
                .map_err(|e| format!("`kit.marcas` ya no se puede leer: {e}")),
        )
    }

    fn consultas_vivas(&mut self, objeto: &str) -> Option<u64> {
        let tabla = objeto
            .rsplit('.')
            .next()
            .unwrap_or(objeto)
            .replace('\'', "");
        self.uno(&format!(
            "SELECT count(*) FROM pg_stat_activity WHERE usename IN ({}) \
             AND state = 'active' AND query ILIKE '%{tabla}%'",
            Self::ROLES
        ))
        .ok()
        .map(|n| n as u64)
    }

    fn sesiones(&mut self) -> Option<u64> {
        self.uno(&format!(
            "SELECT count(*) FROM pg_stat_activity WHERE usename IN ({})",
            Self::ROLES
        ))
        .ok()
        .map(|n| n as u64)
    }

    fn limpiar(&mut self) {
        let _ = self.cliente.batch_execute(&format!(
            "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE usename IN ({})",
            Self::ROLES
        ));
        // Lo que un caso dejó escrito no cuenta para el siguiente.
        let _ = self.cliente.batch_execute("TRUNCATE kit.marcas");
    }
}
