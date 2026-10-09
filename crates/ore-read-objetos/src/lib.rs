//! **El lector de un almacén de objetos, de cualquier proveedor** (ADR 0061
//! O0·2) — la familia de los objetos (ADR 0046 E4).
//!
//! Las otras familias leen filas de algo que ya las tiene: una base de datos,
//! un fichero NDJSON. Un almacén guarda **ficheros**, y de un fichero sale una
//! de dos cosas: filas —si es un Parquet, un CSV, un JSONL— o un objeto —un
//! PDF, una foto, un zip—. El catálogo dice las dos, y `ore source induce` las
//! escribe como `Table` con `format` y como `ObjectTable` (spec v1alpha16).
//!
//! Todo esto sólo le pide al almacén lo que dice [`ore_objetos::Origen`]:
//! listar, listar versiones, leer fijado. Cada driver pone lo suyo en un
//! [`driver::Proveedor`] —leer su URL, comprobar el acceso, su origen— y su
//! `main` es [`driver::main`].

pub mod bajar;
pub mod catalogo;
pub mod driver;
pub mod filas;
pub mod medio;
pub mod tabular;
pub mod versiones;
