//! **La base de `ore-postgres`**: `ore_postgres`, en el mismo Postgres de `ore-pg`
//! que el `storage_controller` (`storcon-db`), con su copia diaria.
//!
//! Las migraciones van dentro del binario y se aplican al arrancar, en orden y
//! una vez cada una: el proceso que las necesita es el único que las conoce, y
//! no hay un Job aparte que pueda ir por delante o por detrás de él.

use postgres::{Client, NoTls};

/// Las migraciones, en orden. Una aplicada no se cambia nunca: se añade otra.
const MIGRACIONES: &[(&str, &str)] = &[
    (
        "001-el-esqueleto",
        include_str!("../migraciones/001-el-esqueleto.sql"),
    ),
    (
        "002-el-tenant-y-las-ramas",
        include_str!("../migraciones/002-el-tenant-y-las-ramas.sql"),
    ),
    (
        "003-los-endpoints",
        include_str!("../migraciones/003-los-endpoints.sql"),
    ),
    (
        "004-roles-y-bases",
        include_str!("../migraciones/004-roles-y-bases.sql"),
    ),
    (
        "005-quien-entra",
        include_str!("../migraciones/005-quien-entra.sql"),
    ),
];

/// Un número cualquiera, pero siempre el mismo: dos procesos que arrancan a la
/// vez no migran a la vez.
const CERROJO: i64 = 0x0058_0004;

pub fn conectar(url: &str) -> Result<Client, String> {
    // Sin TLS: de pod a pod dentro del clúster, como `ore-iam` y el
    // `storage_controller` contra esta misma instancia.
    Client::connect(url, NoTls).map_err(|e| format!("no se pudo conectar: {}", mal(e)))
}

/// Aplica lo que falte. Devuelve los nombres de las que aplicó.
pub fn migrar(c: &mut Client) -> Result<Vec<&'static str>, String> {
    let mut tx = c.transaction().map_err(mal)?;
    tx.execute("select pg_advisory_xact_lock($1)", &[&CERROJO])
        .map_err(mal)?;
    tx.batch_execute(
        "create table if not exists public.migracion (
           nombre text primary key,
           cuando timestamptz not null default now()
         )",
    )
    .map_err(mal)?;
    let mut aplicadas = Vec::new();
    for (nombre, sql) in MIGRACIONES {
        let ya = tx
            .query_opt(
                "select 1 from public.migracion where nombre = $1",
                &[nombre],
            )
            .map_err(mal)?
            .is_some();
        if ya {
            continue;
        }
        tx.batch_execute(sql)
            .map_err(|e| format!("la migración `{nombre}`: {}", mal(e)))?;
        tx.execute(
            "insert into public.migracion (nombre) values ($1)",
            &[nombre],
        )
        .map_err(mal)?;
        aplicadas.push(*nombre);
    }
    tx.commit().map_err(mal)?;
    Ok(aplicadas)
}

/// El mensaje de Postgres, entero (`Display` sólo dice `db error`; ver
/// `ore-iam::base`). Sin el SQL: eso es nuestro.
pub fn mal(e: postgres::Error) -> String {
    match e.as_db_error() {
        Some(d) => {
            let mut s = d.message().to_string();
            if let Some(c) = d.constraint() {
                s.push_str(&format!(" (restricción `{c}`)"));
            }
            if let Some(p) = d.detail() {
                s.push_str(&format!(" · {p}"));
            }
            s
        }
        None => format!("{e}"),
    }
}

/// ¿Saltó esta restricción única?
pub fn choca(e: &postgres::Error, restriccion: &str) -> bool {
    e.as_db_error().is_some_and(|d| {
        *d.code() == postgres::error::SqlState::UNIQUE_VIOLATION
            && (d.constraint() == Some(restriccion) || restriccion.is_empty())
    })
}
