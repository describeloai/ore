//! **La pasarela del Federation Engine** (ADR 0053 F3, `docs/federation.md` §3).
//!
//! Un servicio por celda, sin estado y con el rol `driver`, que recibe una
//! lectura de `ore-serve` (`POST /v1/read`), la hace con un conector ya caliente
//! y devuelve el flujo Arrow con su presupuesto cumplido.
//!
//! Lo que F0 midió y esto resuelve:
//!
//! | medido | aquí |
//! |---|---|
//! | abrir proceso y conexión es casi todo el coste de una lectura pequeña | [`fondo`]: un `servir` por familia y credencial, reutilizado |
//! | 50 lecturas a la vez y 7 fallan con «too many clients» | [`fondo`]: `concurrencia` por origen, con cola y espera máxima |
//! | 2·10⁶ filas de BigQuery, ~117 s y nada la paró | [`lectura`]: filas, bytes y tiempo; corta y cancela en el origen |
//!
//! ⛔ La credencial llega en cada petición y vive en memoria: al conector le
//!   llega por su entrada, nunca por `argv`, el entorno o el disco; lo que el
//!   conector escribe por su salida de error se tapa antes de salir.

pub mod capacidades;
pub mod cotas;
pub mod fondo;
pub mod lectura;
pub mod pasarela;
