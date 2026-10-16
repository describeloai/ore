//! **Cada valor, en su texto de Postgres.**
//!
//! Hasta ADR 0065 PG2 esto decodificaba a mano el binario del protocolo —enteros
//! en big-endian, `numeric` en base 10 000, días desde 2000-01-01— y escribía
//! cada valor como Postgres lo escribiría (W1: `filas` dejaba nulas las columnas
//! que no eran texto). Lo que no sabía leer se negaba: en PG0 eran 27 de 38
//! tipos (arrays, `interval`, `inet`, `timetz`, pgvector, PostGIS…).
//!
//! Ahora las lecturas van por el protocolo simple (`lectura.rs`: `DECLARE` y
//! `FETCH`), que trae **el texto que escribe el propio Postgres**, con la
//! sesión fijada (`lectura::ajustes`): no hay decodificador que pueda divergir
//! ni tipo que no se sepa leer. Lo que queda aquí es decir bien un error.

/// El error del crate envuelve el nuestro —«error deserializing column 1»,
/// «db error»— y lo que dice de verdad está en la causa. Se sigue la cadena
/// entera.
pub fn causa(e: &dyn std::error::Error) -> String {
    let mut partes = vec![e.to_string()];
    let mut actual = e.source();
    while let Some(c) = actual {
        partes.push(c.to_string());
        actual = c.source();
    }
    partes.join(": ")
}
