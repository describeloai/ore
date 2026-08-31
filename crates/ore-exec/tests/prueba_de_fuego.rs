//! La prueba de fuego del evaluador: **las políticas de un paquete contra el
//! esquema que ese mismo paquete proyecta.**
//!
//! Nadie las había enfrentado nunca, y las dos comprobaciones que ya existían
//! explican por qué el hueco pasaba desapercibido: `sync.rs` mira que el esquema
//! comprometido conozca cada nivel de cada retículo, y `politica.rs` que cada
//! etiqueta mencionada exista. Las dos direcciones **de una sola** de las
//! proyecciones.
//!
//! Que una política **entera** sea satisfacible contra el esquema **entero** es
//! otra pregunta, y solo la contesta un validador de Cedar. Es exactamente la
//! misma figura que la prueba de fuego del esquema (`00-overview` §4.1.1), que
//! encontró dos defectos con versiones de antigüedad — pero un peldaño más
//! arriba: allí se comprobó que el esquema fuera *legal*, aquí que las políticas
//! *puedan gobernar algo*.
//!
//! > **Una política imposible no falla: deja de casar con nada.** Y ya sabemos
//! > qué aspecto tiene eso.
//!
//! Se ejecuta contra el ejemplo de referencia porque es el que se enseña: un
//! defecto ahí lo copia todo el mundo.
//!
//! # Estado: en rojo, y lo que encontró
//!
//! `ore-exec` está fuera de `default-members`, así que esto **no** corre en la
//! suite local: se ejecuta a mano, como el driver. Hoy falla, y las cuatro cosas
//! que sacó la primera ejecución son:
//!
//! | | Hallazgo | Estado |
//! |---|---|---|
//! | 1 | `entity Role;` sin `in [Role]`, y un `principal: true` sin `Role` entre sus ancestros — declararlo **desconectaba** a la entidad de toda política `principal in Role::"…"` | **corregido** en `cedar_schema.rs` |
//! | 2 | El esquema comprometido del ejemplo no tenía `context: { purpose: String }`: una decisión entera de retraso, sin que nada se pusiera rojo | **corregido**, y con guardián en `ore-cli/tests/examples.rs` |
//! | 3 | `forbid-agent-without-purpose` es **imposible**: vigila una petición sin `purpose`, y `purpose` es obligatorio en el esquema — esa petición ya no existe. Dos decisiones correctas por separado que se anulan | abierto |
//! | 4 | `resource.owner` y `principal.department` no existen en el esquema: el recurso se posiciona **por pertenencia**, nunca por atributos | abierto — decisión de modelo |
//!
//! El 3 y el 4 son la misma pregunta de fondo: **el recurso de esta proyección
//! es una propiedad, no una fila.** «La compensación de mi departamento» es un
//! recorte de filas, y un recorte de filas no lo evalúa Cedar por fila —eso
//! sería leer la fila para autorizar la fila—: se **traduce a un filtro** que
//! viaja al origen. Que es, exactamente, la ley del ejecutor.

use ore_exec::Motor;
use std::path::Path;

fn ejemplo() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("vendor/oos/examples/acme-retail")
        .leak()
}

#[test]
fn las_politicas_del_ejemplo_validan_contra_su_propio_esquema() {
    let m = Motor::cargar(ejemplo()).expect("el ejemplo de referencia tiene que cargar");
    let errores = m.validar();
    assert!(
        errores.is_empty(),
        "las políticas del ejemplo de referencia no validan contra el esquema que \
         el propio ejemplo proyecta:\n  {}",
        errores.join("\n  ")
    );
}

/// Y el aviso importa tanto como el error, porque el modo de fallo es el mismo.
/// Cedar sabe decir que una política **no puede casar con nada** —una condición
/// imposible, una jerarquía que el esquema no permite—, y eso no es un error de
/// tipos: es una política que no gobierna.
#[test]
fn ninguna_politica_del_ejemplo_es_imposible() {
    let m = Motor::cargar(ejemplo()).expect("el ejemplo de referencia tiene que cargar");
    let avisos = m.avisos();
    assert!(
        avisos.is_empty(),
        "el validador de Cedar avisa de políticas que no pueden gobernar nada:\n  {}",
        avisos.join("\n  ")
    );
}
