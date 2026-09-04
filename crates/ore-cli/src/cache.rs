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
//! # ⚠️ Y este comando se quedó sin quien le pregunte
//!
//! `ore-exec` se retiró —era el camino de lectura del paradigma de bindings— y
//! con él se fue **el único consumidor de este veredicto** y **el único productor
//! del artefacto `ORETOPO1`** del que salía la versión de topología.
//!
//! O sea que hoy esto contesta bien una pregunta que nadie hace, y su bandera
//! `--topology` transporta una cadena que ya nadie escribe. Se deja en pie y
//! **dicho**, no borrado, porque la pregunta que contesta —*«¿esta caché puede
//! servir esta consulta?»*— es exactamente la que el motor de vistas hace con su
//! View Matcher, y decidir si son una o dos es trabajo aparte.
//!
//! La versión de topología se teclea, y la razón original —*leer `ORETOPO1` es de
//! otro binario, y `ore` no enlaza contra él*— sigue valiendo aunque ese binario
//! ya no exista: transportar una cadena es honesto; leer un artefacto no es de
//! aquí.

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
