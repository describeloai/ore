//! # ore-view — el motor de vistas
//!
//! **No ejecuta nada**, y eso no es una limitación nuestra. Es la descripción
//! literal de los dos artefactos más usados de esta categoría: **Apache
//! Calcite**, que no tiene ni almacenamiento ni ejecución y es el planificador
//! de media industria, y **Substrait**, que es una especificación sin motor.
//!
//! > **Un motor de vistas es un compilador de álgebra relacional con un catálogo
//! > versionado, un modelo de capacidades y un reescritor. La ejecución es de
//! > otro.**
//!
//! # Los siete órganos, y por qué se empieza por este
//!
//! Catálogo · **IR** · expansor y reescritor · capacidades y empuje · ejecución
//! del residuo · mantenimiento incremental · linaje a nivel de columna.
//!
//! Cuatro son metadatos y tres son cómputo — salvo que el IR es metadato
//! **sobre** cómputo, y es el que desbloquea a los otros tres. Sin un plan que
//! se pueda mirar no hay reescritura, no hay linaje derivado, no hay reparto por
//! capacidades y no hay incremental: solo hay cadenas de SQL.
//!
//! Los peldaños y lo que cada uno cuesta están en `docs/handoff-view-engine.md`.
//! Esto es **M0** —el IR, en [`plan`] y [`esquema`]— y **M1** —el expansor, en
//! [`catalogo`]—.
//!
//! # Lo que este crate no toca todavía
//!
//! No lee documentos OOS, no sabe qué es una entidad y no conoce el retículo. Es
//! álgebra sobre nombres y tipos. Lo que lo conectará con el resto —M1 el
//! expansor, M3 el flujo de etiquetas— viene después y **a propósito**: la pieza
//! se está construyendo por su cuenta antes de decidir cómo entra.

pub mod catalogo;
pub mod esquema;
pub mod plan;

pub use catalogo::{Catalogo, Expansion, Vista};
pub use esquema::{Desajuste, Esquema, esquema};
pub use plan::{Agregacion, Agregado, Comparador, Expr, Junta, Lectura, Nodo, Opaca, Valor};
