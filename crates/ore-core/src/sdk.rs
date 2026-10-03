//! **El SDK al que llama el código que ORE genera** (S3, 2026-10-03).
//!
//! ORE escribe Python por la persona en tres sitios: las celdas en que
//! `ore-serve` convierte una sentencia SQL, el arnés de una `Function` y las
//! plantillas con que nace un repositorio. Ese código llama al SDK del puesto
//! por sus nombres públicos —en inglés desde S1— y sólo por los de este
//! módulo, que es la única fuente de lo que el código generado da por hecho:
//!
//! - [`API`]: la versión de la interfaz que exige. El SDK la declara
//!   (`ore.API`) y [`guarda_python`] la comprueba al empezar, en Python llano,
//!   para que un puesto con un SDK anterior falle diciendo qué pasa y qué
//!   hacer, en vez de con un `ImportError`.
//! - [`NOMBRES_DE_ANTES`]: los nombres en español que S1 dejó como alias. El
//!   código generado no lleva ninguno (lo comprueba un test sobre todas las
//!   formas que se generan), para que retirar los alias (S5) no rompa nada.

/// La versión de la interfaz del SDK que el código generado exige: 2, los
/// nombres en inglés (S1). Subirla es decir que el código generado usa algo
/// que un SDK anterior no tiene.
pub const API: u32 = 2;

/// Las primeras líneas de toda celda que el servidor genera y un puesto corre.
/// En Python llano, sin nada del SDK que pueda faltar: funciona con cualquiera,
/// también con uno anterior a `API`.
pub fn guarda_python() -> String {
    format!(
        "import ore as _ore\n\
         if getattr(_ore, \"API\", 1) < {API}:\n    \
         raise RuntimeError(\"this session runs an older ORE SDK (API %s; this cell needs {API}): \
         close the session and open a new one\" % getattr(_ore, \"API\", 1))\n\n"
    )
}

/// Lo que el código generado **no** puede llevar: los nombres, argumentos y
/// claves en español del SDK (S1 los dejó como alias, S5 los retira). Como
/// patrones de código —una llamada, un argumento, una clave leída—, no como
/// palabras: la prosa de un comentario no cuenta.
pub const NOMBRES_DE_ANTES: &[&str] = &[
    "crear_base(",
    "crear_schema(",
    "crear_dataset(",
    "crear_vista(",
    "crear_coleccion(",
    "borrar_vista(",
    "coleccion(",
    ".transaccion(",
    ".aplicar(",
    "leer_varios(",
    "put_varios(",
    "persona(",
    "funcion(",
    "modelo(",
    "tabla(",
    "json_de(",
    "media_de(",
    "medias(",
    "si_no_existe=",
    "si_existe=",
    "o_reemplaza=",
    "como=",
    "modo=",
    "clave=",
    "dueno=",
    "comentario=",
    "etiquetas=",
    "formatos=",
    "columnas=",
    "evolucion=",
    "existe=",
    "anterior=",
    "materializada=",
    "clase=",
    "origen=",
    "incluye=",
    "anclada_a=",
    "salida=",
    "hilos=",
    "[\"filas\"]",
    "[\"repetida\"]",
    "[\"creada\"]",
    "[\"creado\"]",
    "[\"coleccion\"]",
    "[\"vista\"]",
    "[\"estado\"]",
    "[\"clase\"]",
    "[\"base\"]",
    "(\"filas\")",
    "\"sobrescribir\"",
    "\"anexar\"",
];

/// Los fallos de un trozo de código generado contra este módulo: cada nombre de
/// antes que lleve (fuera de los comentarios).
pub fn nombres_de_antes_en(codigo: &str) -> Vec<&'static str> {
    let sin_comentarios: String = codigo
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            !t.starts_with('#') && !t.starts_with("//") && !t.starts_with("--")
        })
        .collect::<Vec<_>>()
        .join("\n");
    // En el límite de un identificador: `existe=` no es `si_no_existe=`, ni
    // `tabla(` es `_como_la_tabla(` (un ayudante privado del SDK).
    let en_su_limite = |n: &str| {
        sin_comentarios.match_indices(n).any(|(i, _)| {
            !sin_comentarios[..i]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
        })
    };
    NOMBRES_DE_ANTES
        .iter()
        .copied()
        .filter(|n| en_su_limite(n))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// La guarda es Python que se entiende sin el SDK, y exige `API`.
    #[test]
    fn la_guarda_exige_la_api() {
        let g = guarda_python();
        assert!(g.starts_with("import ore as _ore\n"), "{g}");
        assert!(g.contains(&format!("< {API}:")), "{g}");
        assert!(nombres_de_antes_en(&g).is_empty());
    }

    #[test]
    fn los_nombres_de_antes_se_ven_en_el_codigo_y_no_en_la_prosa() {
        assert_eq!(
            nombres_de_antes_en("x = crear_vista(\"a\", si_no_existe=True)"),
            ["crear_vista(", "si_no_existe="]
        );
        assert!(
            nombres_de_antes_en("# antes se llamaba crear_vista(\n-- y modo=\nx = 1").is_empty()
        );
        // en el límite de un identificador: los ayudantes privados no cuentan
        assert!(
            nombres_de_antes_en(
                "_como_la_tabla(x)\n_ore._modelos_de_la_funcion(m)\nf(if_not_exists=True)"
            )
            .is_empty()
        );
    }
}
