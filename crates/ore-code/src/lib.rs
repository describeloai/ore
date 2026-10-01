//! **El código es la fuente** (ORE 0050, OOS v1alpha18 01 §4).
//!
//! Nadie escribe una función en YAML. El cliente escribe Python —un `def` con
//! `@function` y sus anotaciones de tipo— y el `Function` del catálogo **se
//! deriva** de él: es un artefacto generado, como el esquema Cedar o
//! `ontology.lock`, y `OOS2013` cobra que se quede atrás.
//!
//! # Leer, nunca ejecutar
//!
//! La firma se saca del **árbol sintáctico**, como hacen Ruff, ty o Pyright, y
//! no importando el módulo, como haría FastAPI o Pydantic. Importar es ejecutar
//! código del cliente dentro del compilador o de `ore-serve`, con sus efectos y
//! sus dependencias; y además daría otra respuesta en cada máquina. Leer es
//! una función pura del texto.
//!
//! Por eso la derivación **resuelve nombres como Python**: `date` es una fecha
//! solo si viene de `datetime`, `@function` es el de `ore` solo si el fichero lo
//! importó de `ore` —con alias o sin él— y nada lo tapó después. Un decorador
//! se resuelve donde está el `def`, porque se evalúa al definirlo; una
//! anotación también, salvo entre comillas o con `from __future__ import
//! annotations`, que se resuelven al final del módulo.
//!
//! # Las piezas
//!
//! - [`firma`]: lo que se deriva, **sin lenguaje**: parámetros con su tipo OOS,
//!   lo que devuelve, lo que lee, los modelos. TypeScript llenará la misma.
//! - [`python`]: un `.py` → su [`Derivacion`]. Dentro, `sintaxis` es lo único
//!   que toca el parser de Ruff (versión exacta, API interna), y `derivar` es
//!   §4 sobre esa sintaxis propia.
//! - [`emitir`]: una firma → el documento YAML, determinista e idempotente.
//! - [`lineas`]: de un desplazamiento en bytes a línea y columna, para que un
//!   diagnóstico apunte al `.py` como lo haría un compilador.

pub mod emitir;
pub mod firma;
pub mod lineas;
pub mod python;

pub use firma::{Campo, Def, Derivacion, Fallo, Firma, Funcion, Rango, Salida, Tipo};
