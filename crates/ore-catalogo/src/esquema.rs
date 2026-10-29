//! **El esquema, versionado.** Se aplica al conectar, en una transacción y bajo un cerrojo: dos
//! procesos que arrancan a la vez no lo aplican dos veces.

use postgres::Client;

/// La versión que este binario sabe servir.
pub const VERSION: i32 = 3;

const V1: &str = include_str!("esquema_v1.sql");
/// La puerta de los punteros (E1a·5c·1): escribir con CAS, la historia de una ref, la versión y
/// las ramas. Se añade sobre la v1 sin tocar sus tablas.
const V2: &str = include_str!("esquema_v2.sql");
/// Las escrituras de definición (E1a·5c·5c·1): los cerrojos de una fusión (`escribir` contesta
/// `OR429` en lo cerrado), el plan de fusión y su aplicación ligada a `git:<sha>`. Sobre la v2.
const V3: &str = include_str!("esquema_v3.sql");

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
    if v < 1 {
        tx.batch_execute(V1)?;
        tx.execute("INSERT INTO esquema_version (version) VALUES (1)", &[])?;
    }
    if v < 2 {
        tx.batch_execute(V2)?;
        tx.execute("INSERT INTO esquema_version (version) VALUES (2)", &[])?;
    }
    if v < 3 {
        tx.batch_execute(V3)?;
        tx.execute("INSERT INTO esquema_version (version) VALUES (3)", &[])?;
    }
    if v > VERSION {
        // Se dice y se sigue sin tocar nada: quien lee no rompe lo de otro.
        eprintln!("⚠ el esquema está en la versión {v} y este binario sabe la {VERSION}");
    }
    tx.commit()
}
