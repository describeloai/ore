//! Un fichero `.ts` → lo que dice de sí mismo (OOS v1alpha23 `01`).
//!
//! La misma regla que Python con la marca del lenguaje: TypeScript no decora
//! funciones sueltas, así que lo que promueve es la **exportación por
//! defecto** de un `.ts` de `functions/`, con su `export const config` al
//! lado. Y la misma disciplina: se lee, nunca se ejecuta, y sin el compilador
//! de TypeScript —no se infiere nada: la firma es lo escrito—.

mod derivar;
mod sintaxis;

pub use derivar::{es_de_funciones, nombre_del_fichero};

use crate::firma::{Derivacion, Fallo, Rango};

/// La versión de Node del puesto (`Dockerfile`, etapa `puesto-node`), que
/// ejecuta TypeScript borrando los tipos. Lo que no se borra se señala
/// ([`Derivacion::version`]).
pub const NODE_DEL_PUESTO: u8 = 24;

/// Lee `fuente` y deriva su exportación por defecto, si `ruta` es la de un
/// fichero de funciones ([`es_de_funciones`]). `ruta` es la del fichero desde
/// la carpeta del paquete, con `/`: es el `entrypoint`.
///
/// Como [`crate::python::derivar`], nunca falla ni se detiene en el primer
/// problema, y corre en una pila del tamaño del peor árbol para ese fuente:
/// analizar y visitar un anidamiento hostil (`[[[[…]]]]`) es recursivo. Soltar
/// el árbol no lo es: vive en un *arena*.
pub fn derivar(fuente: &str, ruta: &str) -> Derivacion {
    if fuente.len() > LIMITE {
        return Derivacion {
            sintaxis: vec![
                Fallo::new(
                    Rango::default(),
                    format!(
                        "el fichero tiene {} bytes; no se analiza más de {LIMITE}",
                        fuente.len()
                    ),
                )
                .ayuda("un fichero de funciones es código, no datos: los datos van en un dataset"),
            ],
            ..Derivacion::default()
        };
    }
    stacker::grow((1 << 20) + PILA_POR_BYTE * fuente.len(), || {
        derivar_en_esta_pila(fuente, ruta)
    })
}

/// [`derivar`] sin crecer la pila: para medir cuánta hace falta
/// (`examples/hostil_ts.rs`). Con un fichero hostil, desborda.
#[doc(hidden)]
pub fn derivar_en_esta_pila(fuente: &str, ruta: &str) -> Derivacion {
    derivar::derivar(&sintaxis::leer(fuente), ruta)
}

/// Lo más grande que se analiza: 256 KiB. Una función es un fichero, y
/// ninguna se acerca; el límite es el que deja la pila del peor caso en
/// 384 MiB de memoria virtual.
pub const LIMITE: usize = 256 << 10;

/// La pila por byte de fuente para el peor árbol posible, medida
/// (`examples/hostil_ts.rs`, n = 100 000, doblando la pila hasta que no
/// desborda): 1342 B/byte en debug con `[[[…]]]`, `((…))` en un tipo,
/// `string[][]…` y `{a:{a:…}}`, y 671 en release; `-1`, `1+1+…` y `()=>…`,
/// 335; una unión larga, nada. Es el analizador de oxc, no la derivación: el
/// caso de los corchetes no llega a ella. Cinco veces la de Ruff (168 B).
pub const PILA_POR_BYTE: usize = 1536;

/// Si merece la pena analizar el fichero para encontrar su función: tiene que
/// ser de `functions/` y decir `export default`. Es un filtro, no una
/// respuesta.
pub fn puede_tener_funciones(ruta: &str, fuente: &str) -> bool {
    es_de_funciones(ruta) && fuente.contains("export default")
}

#[cfg(test)]
mod tests;
