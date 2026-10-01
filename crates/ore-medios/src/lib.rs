//! **`ore-medios`: la media de una celda, en un proceso vivo** (ADR 0049, B2).
//!
//! Hoy cada petición sobre una colección encadena tres procesos —`ore-serve`
//! lanza `ore`, que lanza `ore-store-gcs`— tras un `git fetch`, lee el listado
//! entero para dar una página y firma cada URL con una conexión TLS nueva:
//! 100 URLs, 52 s (medido en B2·0). Este proceso vive, y por eso puede:
//!
//! - **guardar el índice** de cada colección por su `metadata_location` —la
//!   transacción que el puntero nombra—: una página por cursor y una búsqueda
//!   por huella cuestan menos de 1 ms, y cargarlo, 0,63 s por millón de ítems;
//! - **firmar con un cliente vivo**: 17 ms la URL, 100 en 0,69 s.
//!
//! No decide quién puede leer, ni sabe de ramas: eso es de `ore-serve`, que
//! autentica, comprueba la concesión, lee el puntero de la rama y le pasa aquí
//! el `metadata_location`. Así este proceso no toca git y lo que sirve es
//! siempre lo de la transacción que se le nombra.
//!
//! Las operaciones son las del contrato (`docs/media.md`): `list`, `stat` y
//! `url`; y `open` —los bytes, fijados— en [`contenido`] (B3); y `put` —una
//! colección escrita, con transacciones— en [`escritura`] (B4b·1).

pub mod contenido;
pub mod escritura;
pub mod firma;
pub mod indice;
pub mod permisos;
pub mod servicio;
