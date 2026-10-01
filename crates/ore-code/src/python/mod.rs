//! Un fichero `.py` → lo que dice de sí mismo (OOS v1alpha18 01 §4).

mod derivar;
mod sintaxis;

use crate::firma::{Derivacion, Fallo, Rango};

/// La versión de Python del puesto (`Dockerfile`, etapa `puesto-python`). La
/// sintaxis posterior es Python válido que allí no correría, y se señala
/// ([`Derivacion::version`]).
pub const PYTHON_DEL_PUESTO: (u8, u8) = (3, 12);

/// Lee `fuente` y deriva cada `@function` de su nivel superior. `ruta` es la
/// del fichero desde la carpeta del paquete, con `/`: va al `entrypoint`.
///
/// Nunca falla ni se detiene en el primer problema: un error de sintaxis, un
/// tipo sin traducción o un argumento que no es literal son datos de la
/// [`Derivacion`], todos a la vez y cada uno con su sitio en el fuente.
///
/// # La pila
///
/// `ore-serve` lee código ajeno, y un fichero hecho para tumbarlo es un
/// árbol muy hondo: `[[[[…]]]]`, `-------1`, `1+1+1+…`. Ruff se protege al
/// analizar (`stacker` le crece la pila), pero **soltar** ese árbol es
/// recursivo y no. Por eso todo —analizar, derivar y soltar— corre en una pila
/// propia del tamaño del peor caso para ese fuente: [`PILA_POR_BYTE`] por byte,
/// medido (`examples/hostil.rs`: 168 B/byte en debug, 84 en release, con
/// corchetes y signos de menos). Es memoria virtual: solo se toca la que el
/// árbol usa de verdad, así que el coste normal es nulo. Y por eso también hay
/// un [`LIMITE`] de tamaño.
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
        derivar::derivar(&sintaxis::leer(fuente), ruta)
    })
}

/// Lo más grande que se analiza: 1 MiB. Ningún fichero de funciones se acerca.
pub const LIMITE: usize = 1 << 20;

/// La pila por byte de fuente para el peor árbol posible (ver [`derivar`]).
pub const PILA_POR_BYTE: usize = 256;

/// Si merece la pena analizar el fichero para encontrar funciones: sin el
/// texto `function` no puede haber ningún `@function`. Es un filtro, no una
/// respuesta —con él dentro hay que analizar—, y cuesta un `memchr`.
pub fn puede_tener_funciones(fuente: &str) -> bool {
    fuente.contains("function")
}
