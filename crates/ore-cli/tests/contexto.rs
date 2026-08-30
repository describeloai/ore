//! `ore dev` — el servidor de contexto, comprobado por su protocolo.
//!
//! Igual que el runner de conformidad: **invoca el binario y no enlaza contra
//! `ore-core`**. No puede hacer trampa, y lo que comprueba es lo que un cliente
//! MCP vería.
//!
//! # Lo que de verdad se afirma aquí
//!
//! Que el servidor **no puede decir más de lo que el contrato dice**. Es la
//! decisión de [ADR 0005](../../../docs/decisions/0005-la-superficie-de-contexto.md)
//! puesta a prueba: como toda respuesta se deriva del SDL emitido y no del
//! paquete, una propiedad que el conducto podó **no está a su alcance**. No es
//! que el servidor decida callarla: es que no la tiene.
//!
//! Un test que solo comprobara el apretón de manos dejaría esa propiedad sin
//! medir, y es la única que hace del servidor algo distinto de un servidor de
//! ficheros.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn caso(nombre: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/oos/conformance/v1alpha5/emit")
        .join(nombre)
        .join("input")
}

/// Lanza `ore dev`, le escribe las líneas y devuelve las respuestas en crudo.
fn dialogo(entrada: &Path, mensajes: &[&str]) -> Vec<String> {
    let mut hijo = Command::new(env!("CARGO_BIN_EXE_ore"))
        .arg("dev")
        .arg(entrada)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("no se pudo lanzar `ore dev`");

    {
        let mut w = hijo.stdin.take().expect("sin stdin");
        for m in mensajes {
            writeln!(w, "{m}").expect("no se pudo escribir la petición");
        }
        // Cerrar `stdin` es cómo termina la sesión: `dev` es un proceso hijo y
        // muere con su cliente. Si no terminara aquí, este test colgaría — y esa
        // es justamente la propiedad que separa `dev` de `serve`.
    }

    let salida = hijo.wait_with_output().expect("`ore dev` no terminó");
    String::from_utf8_lossy(&salida.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(String::from)
        .collect()
}

/// El valor de una clave de primer nivel, leído sin analizador de JSON: basta
/// con encontrar la cadena, y evita meter una dependencia en un test.
fn contiene(linea: &str, fragmento: &str) -> bool {
    linea.contains(fragmento)
}

#[test]
fn una_notificacion_no_lleva_respuesta() {
    let r = dialogo(
        &caso("entity-emits-type"),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        ],
    );
    assert_eq!(
        r.len(),
        2,
        "tres mensajes, dos con `id`: JSON-RPC dice que una notificación no se \
         contesta, y contestarla cuelga a un cliente estricto.\n{r:#?}"
    );
}

#[test]
fn el_id_vuelve_con_el_tipo_con_el_que_llego() {
    let r = dialogo(
        &caso("entity-emits-type"),
        &[
            r#"{"jsonrpc":"2.0","id":7,"method":"ping"}"#,
            r#"{"jsonrpc":"2.0","id":"siete","method":"ping"}"#,
        ],
    );
    assert!(
        contiene(&r[0], r#""id":7"#),
        "un número volvió como otra cosa: {}",
        r[0]
    );
    assert!(
        contiene(&r[1], r#""id":"siete""#),
        "una cadena volvió como otra cosa: {}",
        r[1]
    );
}

/// La versión se devuelve, no se impone: esta superficie es de solo lectura y
/// rechazar a un cliente por su fecha sería rechazarlo por nada.
#[test]
fn la_version_del_protocolo_es_la_que_pidio_el_cliente() {
    let r = dialogo(
        &caso("entity-emits-type"),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2031-01-01"}}"#,
        ],
    );
    assert!(
        contiene(&r[0], r#""protocolVersion":"2031-01-01""#),
        "no devolvió la versión pedida: {}",
        r[0]
    );
}

/// **La afirmación que sostiene el diseño.**
///
/// `nationalId` está en `critical` y el conducto admite hasta `medium`, así que
/// no está en el contrato. El servidor no puede nombrarlo **en ninguna
/// respuesta**, ni siquiera cuando se le pregunta por él directamente: no lo
/// tiene delante.
#[test]
fn el_servidor_no_puede_nombrar_lo_que_el_conducto_podo() {
    let r = dialogo(
        &caso("ceiling-prunes-the-classified"),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"resources/read","params":{"uri":"oos://schema.graphql"}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"ontology_schema","arguments":{}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"ontology_describe","arguments":{"type":"Customer"}}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"ontology_describe","arguments":{"type":"nationalId"}}}"#,
        ],
    );
    assert_eq!(r.len(), 4);
    for (i, linea) in r.iter().enumerate() {
        assert!(
            !contiene(linea, "nationalId"),
            "la respuesta {} nombra una propiedad que el conducto podó:\n{linea}",
            i + 1
        );
    }
    // Y lo que sí está, está: si no apareciera, el test de arriba pasaría por
    // servir un esquema vacío.
    assert!(
        contiene(&r[0], "email"),
        "no sirvió lo que sí admite el conducto"
    );
    assert!(contiene(&r[2], "customerId"), "describe no describió nada");
}

/// Un tipo podado y un tipo que nunca existió dan **la misma respuesta**.
/// Distinguirlos revelaría que existe algo que el contrato no declara, que es
/// exactamente lo que el conducto acaba de impedir.
#[test]
fn lo_podado_y_lo_inexistente_se_contestan_igual() {
    let r = dialogo(
        &caso("orphan-relation-is-pruned"),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"ontology_describe","arguments":{"type":"Diagnosis"}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"ontology_describe","arguments":{"type":"Inexistente"}}}"#,
        ],
    );
    let sin_id = |s: &str| s.replace(r#""id":1"#, "").replace(r#""id":2"#, "");
    assert_eq!(
        sin_id(&r[0]).replace("Diagnosis", "X"),
        sin_id(&r[1]).replace("Inexistente", "X"),
        "un tipo podado y uno inexistente se distinguen, y no deben"
    );
}

/// Cada resultado lleva el digest del bundle del que salió. `DESIGN` §3.4
/// promete que *«¿qué sabía el agente el martes a las 14:32?»* se contesta con
/// un commit y una marca de agua; esto es la marca de agua.
#[test]
fn cada_respuesta_lleva_su_digest() {
    let r = dialogo(
        &caso("entity-emits-type"),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"ontology_schema","arguments":{}}}"#,
        ],
    );
    assert!(
        contiene(&r[0], r#""bundle":"sha256:"#),
        "la respuesta no dice de qué bundle salió: {}",
        r[0]
    );
}

/// El contrato que sirve es **exactamente** el que emite `export`. Si divergiera
/// habría dos verdades sobre lo mismo, que es el modo de fallo que este proyecto
/// persigue en todas partes.
#[test]
fn sirve_el_mismo_contrato_que_emite_export() {
    let entrada = caso("entity-emits-type");
    let export = Command::new(env!("CARGO_BIN_EXE_ore"))
        .args(["export", "--format", "graphql"])
        .arg(&entrada)
        .output()
        .expect("no se pudo invocar `ore export`");
    let sdl = String::from_utf8_lossy(&export.stdout);

    let r = dialogo(
        &entrada,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"resources/read","params":{"uri":"oos://schema.graphql"}}"#,
        ],
    );
    for linea in sdl.lines().filter(|l| !l.trim().is_empty()) {
        // El SDL viaja dentro de una cadena JSON, asi que las comillas de un
        // `@key(fields: "...")` llegan escapadas. Se escapa la esperada igual.
        let escapada = linea.trim_end().replace('"', "\\\"");
        assert!(
            contiene(&r[0], &escapada),
            "el servidor no sirve la línea `{linea}` que `export` sí emite"
        );
    }
}
