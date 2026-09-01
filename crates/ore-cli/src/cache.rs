//! `ore cache check` — **¿esta caché puede servir esta consulta?**
//!
//! La lógica está en `ore_core::cache`; aquí solo se junta lo que la pregunta
//! necesita, y hay un detalle que merece nombrarse: **el digest del bundle no se
//! pide por bandera, se calcula del árbol**.
//!
//! No es comodidad. El campo del que depende todo el módulo es *«¿bajo qué regla
//! se escribieron estas filas?»*, y dejar que quien pregunta lo teclee sería
//! dejarle contestar por su cuenta la única pregunta que la caché no puede
//! contestar sola. Se compila el paquete y se compara — que es exactamente lo
//! que hace `cargar_topologia` con el índice, y por el mismo motivo.
//!
//! # La versión de topología sí se teclea, y se dice por qué
//!
//! Porque leer un artefacto `ORETOPO1` es de `ore-exec`, y `ore` no enlaza contra
//! él. La versión es una cadena que el artefacto reporta, y transportarla es
//! honesto; leerla aquí exigiría meter el ejecutor dentro del compilador, que es
//! justo la frontera que este proyecto no cruza.

use ore_core::cache::{Manifiesto, Pregunta, Veredicto};
use std::path::Path;
use std::process::ExitCode;

pub struct Consulta<'a> {
    pub manifiesto: &'a Path,
    pub entidad: &'a str,
    pub propiedades: Vec<String>,
    pub topologia: Option<&'a str>,
    pub instante: Option<&'a str>,
    pub sla: Option<&'a str>,
}

pub fn check(c: &Consulta, pkg: &ore_core::link::Package) -> ExitCode {
    let texto = match std::fs::read_to_string(c.manifiesto) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: no se pudo leer `{}`: {e}", c.manifiesto.display());
            return ExitCode::from(66); // EX_NOINPUT
        }
    };
    let manifiesto = match Manifiesto::leer(&texto) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(65); // EX_DATAERR
        }
    };

    let bundle = ore_core::digest::bundle(pkg);
    let v = manifiesto.consultar(&Pregunta {
        bundle: &bundle,
        topologia: c.topologia,
        entidad: c.entidad,
        propiedades: &c.propiedades,
        instante: c.instante,
        sla: c.sla,
    });

    println!("{}", v.como_texto());
    if !v.sirve() {
        println!("  remedio: {}", v.remedio());
    }
    salida(&v)
}

/// El código de salida, y **la distinción que transporta**.
///
/// Un guion que llame a esto tiene que poder distinguir *«hoy no sirve»* de
/// *«esta caché no corresponde a esta pregunta»*, porque el remedio es distinto y
/// el segundo no se arregla esperando. Colapsar los dos en un `1` obligaría a
/// leer el texto para saber si hay que reconstruir.
fn salida(v: &Veredicto) -> ExitCode {
    match v {
        Veredicto::Sirve { .. } => ExitCode::SUCCESS,
        // El artefacto se escribió bajo otro significado o con otra
        // correspondencia: es un dato que no vale para esta pregunta.
        Veredicto::ReglaDistinta { .. } | Veredicto::CorrespondenciaDistinta { .. } => {
            ExitCode::from(65) // EX_DATAERR
        }
        // No sirve, y se arregla por la vía que se acaba de decir.
        _ => ExitCode::FAILURE,
    }
}
