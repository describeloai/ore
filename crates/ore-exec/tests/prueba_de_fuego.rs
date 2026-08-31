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
//! # Lo que encontró la primera ejecución
//!
//! `ore-exec` está fuera de `default-members`, así que esto **no** corre en la
//! suite local: se ejecuta a mano, como el driver —y en Docker, porque
//! `windows-sys` exige `dlltool`—. **Hoy está en verde.** Las cinco cosas que
//! sacó por el camino, y dónde acabó cada una:
//!
//! | | Hallazgo | Dónde se cerró |
//! |---|---|---|
//! | 1 | `entity Role;` sin `in [Role]`, y un `principal: true` sin `Role` entre sus ancestros — declararlo **desconectaba** a la entidad de toda política `principal in Role::"…"` | `cedar_schema.rs` |
//! | 2 | El esquema comprometido del ejemplo no tenía `context: { purpose: String }`: una decisión entera de retraso, sin que nada se pusiera rojo | `ore-cli/tests/examples.rs`, que ahora lo cobra byte a byte |
//! | 3 | `forbid-agent-without-purpose` es **imposible**: vigila una petición sin `purpose`, y `purpose` es obligatorio — esa petición ya no existe | `05-ejecutor` §6.1: se rechaza **antes** de ①, y la política se borra del ejemplo |
//! | 4 | `resource.owner` no existe **y no puede existir**: el recurso se posiciona por pertenencia, nunca por atributos | `v1alpha3/02-ruleset` §4.2 — el **ámbito de fila** |
//! | 5 | `context.purpose in ["a","b"]` **no es Cedar válido**: `in` es el operador de jerarquía de entidades y `purpose` es un `String` | corregido a `.contains(…)` en el ejemplo, en `diff/widen-purposes` y en el ADR 0003, que lo daba como forma equivalente |
//!
//! El 5 explica por qué esta prueba tenía que existir. La forma inválida vivía
//! en tres sitios desde el principio **sin producir ningún síntoma**, porque
//! `purposes()` extrae las cadenas entrecomilladas y le da igual el operador:
//! `OOS5015` seguía clasificando bien. La lectura estructural leía
//! correctamente una política que Cedar habría rechazado.
//!
//! El 3 y el 4 eran la misma pregunta: **el recurso de esta proyección es una
//! propiedad, no una fila.** *«La compensación de mi departamento»* es un
//! recorte de filas, y un recorte de filas no lo evalúa Cedar fila a fila —eso
//! sería leer la fila para autorizar la fila—: se **traduce a un filtro** que
//! viaja al origen, que es exactamente la ley del ejecutor.
//!
//! # El sexto, que salió de preguntar por qué
//!
//! Declarar `principal: true` sobre `hr.Employee` metía **todas** sus propiedades
//! escalares en el esquema como atributos **obligatorios** del principal — y una
//! de ellas es `nationalId`, clasificada `critical`. La capa de identidad tenía
//! que firmar el DNI en cada petición.
//!
//! > Un atributo del principal es lo que **decide** el acceso. Meter ahí un dato
//! > que el acceso protege es exactamente al revés.
//!
//! No era un defecto de `atributos()`: era que **no había dónde declarar qué es
//! una reclamación de identidad**. OOS declaraba la entrada de datos
//! —`datasources`— y la salida —`ConduitPolicy`— y **no la entrada de identidad**,
//! que es la única que decide en vez de ser gobernada. Lo cierra
//! [`06-request`](../../vendor/oos/spec/v1alpha1/06-request.md), y con él vuelve
//! `OOS4005`.
//!
//! Hoy el esquema del ejemplo emite `Employee { departmentId, employeeId }`.

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
