//! **El kit de conformidad de los conectores** (ADR 0053 F2·1,
//! `docs/federation.md` §2).
//!
//! Un conector entra en la federación cuando pasa esto, no cuando alguien lo
//! revisa: lanza el binario como lo lanzaría la pasarela, contra unos datos de
//! partida que el propio kit carga en el origen ([`semilla`]), y comprueba los
//! catorce casos del contrato ([`casos`]). Lo que el conector devuelve se
//! compara con lo que la semilla dice que tiene que devolver, calculado aquí
//! con la semántica de SQL y los tipos de 0032 —nunca preguntándole a otro
//! conector—.
//!
//! Cada origen aporta un [`bancos::Banco`]: cómo cargar la semilla y lo que
//! sólo él sabe contestar (si una consulta sigue viva, cuántas sesiones hay).
//! Postgres y S3 (el S3 de mentira de `pruebas-de-fuego/de-mentira.py`, o un
//! MinIO) corren en el CI.
//!
//! ```text
//! PG_URL=postgres://postgres:x@localhost:5432/postgres \
//!   ore-kit --conector target/debug/ore-read-postgres --banco postgres
//! ```

pub mod bancos;
pub mod casos;
pub mod conector;
pub mod respuesta;
pub mod semilla;

/// Los operadores que el kit prueba: todos los que una petición sabe llevar.
pub const OPERADORES_PROBADOS: &[&str] = ore_driver::OPERADORES;
