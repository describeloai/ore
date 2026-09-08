//! `ore-entrada` — **por dónde llega una petición y quién la trae**.
//!
//! # Por qué existe
//!
//! Salió de un refactor con una razón concreta: `Identidad` y `SinIdentidad`
//! vivían **dentro del binario de la ontología**, que no tenía por qué ser su
//! dueño. En cuanto hubo un segundo servidor —`ore-iam`, el que administra
//! personas y concesiones— había dos opciones: copiar la definición de qué es
//! un sujeto, o sacarla. Copiarla es tener dos descripciones del mismo
//! contrato, y **la que se quede vieja no dará error: dará permiso.**
//!
//! # Qué es, y qué NO
//!
//! Es la **entrada**: el umbral por el que se cruza. No es la puerta que
//! decide.
//!
//! ```text
//!   ore-entrada   cómo llega la petición y quién la trae
//!   la puerta     qué puede hacer quien llegó        ← eso NO está aquí
//! ```
//!
//! La distinción no es sutileza: en la plataforma, `puerta` ya nombra al PDP
//! —AuthZEN, concesión, dueño—. Mezclar el transporte con la decisión en un
//! nombre habría acabado mezclándolos en el código.
//!
//! **No autoriza.** Dice quién pregunta y nada más.
//!
//! # Y por qué un crate y no dos
//!
//! Se planteó partirlo —`ore-http` y `ore-sujeto`— y se descartó: los dos
//! binarios necesitan las dos mitades, porque el diseño dice que **sin
//! proveedor de identidad no se montan las rutas de datos**. Separar lo que
//! siempre va junto añade ceremonia sin comprar aislamiento.
//!
//! El precio, dicho: el servidor HTTP no tenía ni una dependencia y aquí
//! hereda `rsa` y `sha2`. Da igual — quien enlace esto va a verificar tokens.
//!
//! # Las tres piezas
//!
//! | | qué hace |
//! |---|---|
//! | [`http`] | HTTP/1.1 con la biblioteca estándar. Sin `keep-alive`, con límites |
//! | [`identidad`] | el puerto **sin defecto**: sin proveedor, no hay superficie |
//! | [`oidc`] | verificar un token del realm, y la llave **no se va a buscar** |

pub mod http;
pub mod identidad;
pub mod oidc;

pub use http::{Peticion, Respuesta, servir};
pub use identidad::{Ajustes, Identidad, Proveedor, SinIdentidad};
