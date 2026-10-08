//! **El reconciliador** (0058 P4·2): lleva lo observado hacia lo deseado.
//!
//! Recorre las operaciones `en-curso` cuya hora (`siguiente`) ya llegó y da a
//! cada una **un intento entero**: todos sus pasos, cada uno idempotente
//! ([`crate::almacen`]). Si sale, la operación queda `hecha` y el recurso,
//! `listo`. Si no:
//!
//! - lo pasajero (el tenant aún no está activo, el controller no contesta) se
//!   reintenta, esperando más cuantos más intentos lleva (1, 2, 4… hasta 60 s);
//! - lo definitivo, o pasados [`INTENTOS`], deja la operación `fallida` con el
//!   motivo, y el recurso, `fallido`.
//!
//! ⭐ Un reinicio no pierde nada: lo que estaba en curso sigue en la base, con
//!   sus ids de tenant y timeline ya guardados, y se retoma donde iba.
//!
//! ⛔ Uno solo: `ore-postgres` es una réplica con `Recreate`. Dos
//!   reconciliadores sobre el mismo estado no son más disponibilidad.

use crate::almacen::{Almacen, Fallo};
use crate::base::{conectar, mal};
use postgres::Client;
use std::time::Duration;

/// Cuántos intentos antes de rendirse. Con la espera creciente, unos 30 min.
pub const INTENTOS: i32 = 40;

/// Cada cuánto mira si hay algo que hacer.
const CADA: Duration = Duration::from_secs(1);

/// En un hilo, para siempre. Con su propia conexión: la del API no se comparte.
pub fn arrancar(url: String, almacen: Box<dyn Almacen>) {
    std::thread::spawn(move || {
        let mut base: Option<Client> = None;
        loop {
            if base.as_ref().is_none_or(|c| c.is_closed()) {
                base = match conectar(&url) {
                    Ok(c) => Some(c),
                    Err(e) => {
                        eprintln!("reconciliador · {e}");
                        std::thread::sleep(Duration::from_secs(5));
                        continue;
                    }
                };
            }
            if let Some(c) = base.as_mut()
                && let Err(e) = vuelta(c, almacen.as_ref())
            {
                eprintln!("reconciliador · {e}");
            }
            std::thread::sleep(CADA);
        }
    });
}

/// Una vuelta: lo que toca ahora. Devuelve cuántas operaciones miró.
pub fn vuelta(c: &mut Client, a: &dyn Almacen) -> Result<usize, String> {
    let pendientes = c
        .query(
            "select id, tipo, organizacion, proyecto, rama, intentos from plano.operacion
              where estado = 'en-curso' and siguiente <= now()
              order by creada limit 20",
            &[],
        )
        .map_err(mal)?;
    for f in &pendientes {
        let op = Op {
            id: f.get(0),
            tipo: f.get(1),
            organizacion: f.get(2),
            proyecto: f.get(3),
            rama: f.get(4),
            intentos: f.get(5),
        };
        let resultado = intentar(c, a, &op);
        cerrar(c, &op, resultado)?;
    }
    Ok(pendientes.len())
}

struct Op {
    id: String,
    tipo: String,
    organizacion: String,
    proyecto: String,
    #[allow(dead_code)] // P4·2·3: las operaciones de rama
    rama: Option<String>,
    intentos: i32,
}

/// Un intento entero de una operación. `Ok` es que ya está.
fn intentar(c: &mut Client, a: &dyn Almacen, op: &Op) -> Result<(), Fallo> {
    let bd = |e: postgres::Error| Fallo::Reintentar(format!("la base: {}", mal(e)));
    match op.tipo.as_str() {
        "crear-proyecto" => {
            let f = c
                .query_one(
                    "select p.tenant, r.timeline from plano.proyecto p
                       join plano.rama r on r.organizacion = p.organizacion and r.proyecto = p.id
                      where p.organizacion = $1 and p.id = $2 and r.padre is null",
                    &[&op.organizacion, &op.proyecto],
                )
                .map_err(bd)?;
            let (tenant, main): (String, String) = (f.get(0), f.get(1));
            a.asegurar_tenant(&tenant)?;
            a.asegurar_timeline(&tenant, &main, None)?;
            c.execute(
                "update plano.rama set observado = 'lista'
                  where organizacion = $1 and proyecto = $2 and padre is null",
                &[&op.organizacion, &op.proyecto],
            )
            .map_err(bd)?;
            c.execute(
                "update plano.proyecto set observado = 'listo' where organizacion = $1 and id = $2",
                &[&op.organizacion, &op.proyecto],
            )
            .map_err(bd)?;
            Ok(())
        }
        "borrar-proyecto" => {
            let tenant: Option<String> = c
                .query_opt(
                    "select tenant from plano.proyecto where organizacion = $1 and id = $2",
                    &[&op.organizacion, &op.proyecto],
                )
                .map_err(bd)?
                .and_then(|f| f.get(0));
            if let Some(t) = tenant {
                a.borrar_tenant(&t)?;
            }
            // Las ramas se van con él (on delete cascade).
            c.execute(
                "delete from plano.proyecto where organizacion = $1 and id = $2",
                &[&op.organizacion, &op.proyecto],
            )
            .map_err(bd)?;
            Ok(())
        }
        otro => Err(Fallo::Definitivo(format!(
            "este reconciliador no sabe hacer `{otro}`"
        ))),
    }
}

/// Apunta lo que salió.
fn cerrar(c: &mut Client, op: &Op, r: Result<(), Fallo>) -> Result<(), String> {
    match r {
        Ok(()) => {
            c.execute(
                "update plano.operacion set estado = 'hecha', terminada = now(), error = null
                  where id = $1",
                &[&op.id],
            )
            .map_err(mal)?;
        }
        Err(Fallo::Reintentar(m)) if op.intentos + 1 < INTENTOS => {
            let espera = 2f64.powi(op.intentos.min(6)).min(60.0);
            c.execute(
                "update plano.operacion
                    set intentos = intentos + 1, error = $2,
                        siguiente = now() + make_interval(secs => $3)
                  where id = $1",
                &[&op.id, &m, &espera],
            )
            .map_err(mal)?;
        }
        Err(f) => {
            let m = format!("{} (tras {} intentos)", f.motivo(), op.intentos + 1);
            c.execute(
                "update plano.operacion
                    set estado = 'fallida', terminada = now(), error = $2, intentos = intentos + 1
                  where id = $1",
                &[&op.id, &m],
            )
            .map_err(mal)?;
            if op.tipo == "crear-proyecto" {
                c.execute(
                    "update plano.proyecto set observado = 'fallido' where organizacion = $1 and id = $2",
                    &[&op.organizacion, &op.proyecto],
                )
                .map_err(mal)?;
            }
            eprintln!("reconciliador · {} {} fallida: {m}", op.tipo, op.id);
        }
    }
    Ok(())
}
