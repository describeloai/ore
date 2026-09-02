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
//! # Las piezas construidas
//!
//! Se nombran con la terminología del sector y no con nombres propios: un
//! ingeniero de datos tiene que poder leer esto sin traducir nada. El plano
//! entero —con las seis que faltan— está en `docs/handoff-view-engine.md`.
//!
//! | Pieza | Módulo | Fase | Qué contesta |
//! |---|---|---|---|
//! | **Plan IR** | [`plan`] | M0 | qué se va a hacer, con identidad determinista |
//! | **Schema Resolver** | [`schema`] | M0 | qué columnas salen y de qué tipo |
//! | **View Expander** | [`catalog`] | M1 | una cadena de vistas es un plan |
//! | **Lineage Analyzer** | [`lineage`] | M2 | de qué columna raíz sale cada salida, y por qué arista |
//! | **Flow Checker** | [`flow`] | M3 | por qué esto no compila |
//! | **Pushdown Planner** | [`capabilities`] | M4 | qué hace el origen y qué queda de residuo |
//! | **Filter Tree** | [`filter_tree`] | M5 | de todas las materializaciones, cuáles podrían servir |
//! | **View Matcher** | [`view_matcher`] | M5 | si esta la contesta, con qué compensation, y qué hereda |
//!
//! # Lo que este crate no toca todavía
//!
//! No lee documentos OOS, no sabe qué es una entidad y no conoce el retículo. Es
//! álgebra sobre nombres y tipos. Lo que lo conectará con el resto viene
//! después y **a propósito**: la pieza se está construyendo por su cuenta antes
//! de decidir cómo entra.

pub mod capabilities;
pub mod catalog;
pub mod filter_tree;
pub mod flow;
pub mod lineage;
pub mod plan;
pub mod schema;
pub mod view_matcher;

pub use capabilities::{Capacidades, Peticion, Recorrido, Reparto, repartir};
pub use catalog::{Catalogo, Expansion, Vista};
pub use filter_tree::{FilterTree, Hoja, Materializacion, Registro, firma};
pub use flow::{Clasificacion, Fuga, Veredicto, comprobar};
pub use lineage::{Arista, Clase, Directa, Indirecta, Linaje, Raiz, linaje};
pub use plan::{Agregacion, Agregado, Comparador, Expr, Junta, Lectura, Nodo, Opaca, Valor};
pub use schema::{Desajuste, Esquema, esquema};
pub use view_matcher::{NoContesta, Restriccion, Rewrite, cotejar, sello};
