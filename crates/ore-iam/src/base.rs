//! La base, y **el hecho y su huella en la misma transacción**.
//!
//! # La regla que este módulo existe para hacer cumplir
//!
//! Todo verbo que cambia algo escribe dos cosas: el hecho y su huella. No una
//! después de la otra — **dentro de la misma transacción**.
//!
//! Si se pudieran separar existiría un estado en el que el hecho ocurrió y
//! nadie lo anotó, y ese estado es **indistinguible de un borrado del
//! registro**. Es la lección de su `020`, que la descubrió contando filas
//! contra la base de verdad: *«el acto más importante del flujo —alguien ENTRA
//! en la organización— no dejaba rastro»*.
//!
//! Por eso [`Tx::anotar`] no es opcional ni se llama desde fuera: forma parte
//! de cada verbo, y el verbo entero se confirma o no ocurre.

use ore_core::json::Json;
use ore_entrada::identidad::Identidad;
use postgres::{Client, NoTls, Transaction};

/// A qué base y con qué credencial. **Nada de esto tiene defecto**: una cadena
/// de conexión por defecto es apuntar a una base que nadie eligió.
pub fn conectar(url: &str) -> Result<Client, String> {
    // Sin TLS, y se dice por qué: la conexión es dentro del clúster, de pod a
    // pod, y es la misma elección que ya hace Keycloak con esta misma base. Si
    // un día deja de bastar, deja de bastar para los dos a la vez.
    Client::connect(url, NoTls).map_err(|e| format!("no se pudo conectar: {e}"))
}

/// Una transacción con la huella dentro.
pub struct Tx<'a> {
    tx: Transaction<'a>,
    sujeto: Identidad,
    anotado: bool,
}

/// ⛔⛔ EL MENSAJE DE POSTGRES, ENTERO.
///
/// `Display` de `postgres::Error` dice **`db error`** y nada mas. Literalmente
/// eso: un 422 con tres palabras que no sirven para nada. Todo lo util —el
/// mensaje, la restriccion que salto, la columna, la pista del propio motor—
/// vive en `as_db_error()`, y hay que ir a buscarlo.
///
/// ⭐ Medido: `invitar` sin rol devolvia `{"error":"db error"}` y hubo que
/// deducir a mano que `invitacion.rol` seguia siendo `not null`. Con esto lo
/// habria dicho la primera vez.
///
/// ⚠️ Y esto sale hacia fuera, asi que no lleva el SQL: la consulta es nuestra
/// y su forma no es asunto de quien pregunta.
fn mal(e: postgres::Error) -> String {
    match e.as_db_error() {
        Some(d) => {
            let mut s = d.message().to_string();
            if let Some(c) = d.constraint() {
                s.push_str(&format!(" (restriccion `{c}`)"));
            }
            if let Some(p) = d.detail() {
                s.push_str(&format!(" · {p}"));
            }
            s
        }
        None => format!("{e}"),
    }
}

impl<'a> Tx<'a> {
    pub fn abrir(c: &'a mut Client, sujeto: &Identidad) -> Result<Tx<'a>, String> {
        Ok(Tx {
            tx: c
                .transaction()
                .map_err(|e| format!("no se pudo abrir la transacción: {e}"))?,
            sujeto: sujeto.clone(),
            anotado: false,
        })
    }

    pub fn ejecutar(
        &mut self,
        sql: &str,
        args: &[&(dyn postgres::types::ToSql + Sync)],
    ) -> Result<u64, String> {
        self.tx.execute(sql, args).map_err(mal)
    }

    pub fn uno(
        &mut self,
        sql: &str,
        args: &[&(dyn postgres::types::ToSql + Sync)],
    ) -> Result<Option<postgres::Row>, String> {
        self.tx.query_opt(sql, args).map_err(mal)
    }

    pub fn filas(
        &mut self,
        sql: &str,
        args: &[&(dyn postgres::types::ToSql + Sync)],
    ) -> Result<Vec<postgres::Row>, String> {
        self.tx.query(sql, args).map_err(mal)
    }

    /// La huella. `quien` y `agente` van **por separado** —`sub` y `act` de
    /// RFC 8693—, que es la misma forma que `ore-serve` escribe en el autor y
    /// el committer de cada commit de la forja. Fundirlos no sirve para
    /// contestar ninguna de las dos preguntas.
    /// ⚠️ `$5::text::jsonb` y no `$5::jsonb`. Con el segundo, Postgres infiere
    /// que el parámetro **es** `jsonb` y el driver intenta serializar una
    /// cadena como tal — lo que exige una crate de JSON que este árbol no
    /// tiene y no quiere. Con el doble casteo el parámetro viaja como texto y
    /// es Postgres quien lo convierte, que además es quien sabe si es válido.
    pub fn anotar(&mut self, operacion: &str, sobre: &str, detalle: Json) -> Result<(), String> {
        self.tx
            .execute(
                "insert into iam.huella (quien, agente, operacion, sobre, detalle)
                 values ($1, $2, $3, $4, $5::text::jsonb)",
                &[
                    &self.sujeto.persona,
                    &self.sujeto.agente,
                    &operacion,
                    &sobre,
                    &detalle.jcs(),
                ],
            )
            .map_err(|e| format!("no se pudo anotar la huella: {e}"))?;
        self.anotado = true;
        Ok(())
    }

    /// ⛔ Se niega a confirmar si nadie anotó. No es una comprobación de
    /// higiene: es la regla de arriba, hecha imposible de olvidar. Un verbo que
    /// cambia algo y no deja huella no llega a la base.
    pub fn confirmar(self) -> Result<(), String> {
        if !self.anotado {
            return Err("esta transacción cambia algo y no dejó huella. No se confirma.".into());
        }
        self.tx
            .commit()
            .map_err(|e| format!("no se pudo confirmar: {e}"))
    }

    /// ⭐ LA ÚNICA EXCEPCIÓN A LA REGLA DE ARRIBA, y con nombre: una OBSERVACIÓN.
    ///
    /// La huella es de los ACTOS —alguien invitó, concedió, pidió una celda—.
    /// Un snapshot que el informador de la celda empuja cada minuto (0026) no
    /// es un acto: es la misma medida repetida, y su valor es el último. Una
    /// huella por minuto y celda no sería un registro: sería el ruido que
    /// entierra el registro (1.440 al día por celda).
    ///
    /// ⚠️ Sólo `iam.celda_estado` se confirma por aquí, y el verbo que lo hace
    ///   SÍ anota —por `confirmar`, la de siempre— cuando la celda EMPIEZA a
    ///   informar o VUELVE tras un silencio: eso sí le interesa a quien lea la
    ///   actividad. Cualquier otro uso de esto es saltarse la regla, no
    ///   aplicarla.
    pub fn confirmar_observacion(self) -> Result<(), String> {
        self.tx
            .commit()
            .map_err(|e| format!("no se pudo confirmar: {e}"))
    }
}

pub use crate::id::nuevo as nuevo_id;
