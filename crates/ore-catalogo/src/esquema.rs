//! **El esquema, versionado.** Se aplica al conectar, en una transacción y bajo un cerrojo: dos
//! procesos que arrancan a la vez no lo aplican dos veces.

use postgres::Client;

/// La versión que este binario sabe servir.
pub const VERSION: i32 = 1;

const V1: &str = include_str!("esquema_v1.sql");

/// Lleva el esquema del inquilino (el del `search_path`) a [`VERSION`]. Uno más nuevo no se toca:
/// lo escribió un binario más nuevo, y este no sabe leerlo.
pub fn aplicar(c: &mut Client) -> Result<(), postgres::Error> {
    let mut tx = c.transaction()?;
    tx.execute(
        "SELECT pg_advisory_xact_lock(hashtext(current_schema()))",
        &[],
    )?;
    let hay: bool = tx
        .query_one("SELECT to_regclass('esquema_version') IS NOT NULL", &[])?
        .get(0);
    let v: i32 = if hay {
        tx.query_one("SELECT coalesce(max(version), 0) FROM esquema_version", &[])?
            .get(0)
    } else {
        0
    };
    if v == 0 {
        tx.batch_execute(V1)?;
        tx.execute("INSERT INTO esquema_version (version) VALUES (1)", &[])?;
    } else if v > VERSION {
        // Se dice y se sigue sin tocar nada: quien lee no rompe lo de otro.
        eprintln!("⚠ el esquema está en la versión {v} y este binario sabe la {VERSION}");
    }
    tx.commit()
}
