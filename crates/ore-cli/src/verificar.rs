//! `ore verify` — **cotejar una propuesta contra el paquete que la autoriza**.
//!
//! El tercer paso del circuito de [`docs/functions.md`](../../../docs/functions.md) §3:
//!
//! ```text
//! Plan ──► función ──► Propuesta ──► (verificar) ──► (aplicar, por la vista)
//!                                         ▲
//!                                       esto
//! ```
//!
//! # No ejecuta nada, y esa es la mitad del asunto
//!
//! No invoca la función, no abre el origen y no escribe. Recibe una propuesta
//! ya hecha —por `ore-invoke`, o por cualquier cosa que hable el protocolo— y
//! contesta si se puede aplicar. Que esto sea contestable **sin runtime** es lo
//! que hace que el simulacro salga gratis: la propuesta **es** el simulacro, y
//! no hay dos caminos que puedan divergir.
//!
//! # Qué distingue esto de `ore validate`
//!
//! `validate` juzga **un paquete**: si compila. `verify` juzga **una propuesta
//! contra un paquete**: si lo que un delegado devolvió cae dentro de lo que ese
//! paquete autorizaba. Un paquete puede compilar y una propuesta sobre él ser
//! inaceptable, que es justo el caso que existe para atrapar.
//!
//! # Y por qué la quinta identidad se recontrasta aquí y no en el núcleo
//!
//! De las cinco, esta es la única que se puede volver a calcular sin abrir
//! nada: basta con expandir la vista otra vez y comparar digests. Pero expandir
//! una vista es del motor, y el motor vive en `ore-view`; el núcleo no lo ve.
//! **Quien tiene los dos es este binario**, que es la misma costura por la que
//! `ore view` contesta. Así que el cotejo de la superficie vive en
//! `ore_core::propuesta` —es gramática— y la comparación de la vista vive aquí
//! —es álgebra—.
//!
//! Las otras tres —topología, marcas de agua, el `Plan`— no las puede
//! contrastar nadie desde aquí, y **se dicen sin verificar en vez de callarse**.

use std::path::Path;
use std::process::ExitCode;

use ore_core::link::Loaded;
use ore_core::propuesta::{Propuesta, cotejar};
use ore_view::catalog::{Catalogo, Vista};

use crate::vista::{Vistas, cuerpo, tipos_de_raiz};

pub fn verificar(paquete: &Path, propuesta: &Path) -> ExitCode {
    let texto = match std::fs::read_to_string(propuesta) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: no se pudo leer `{}`: {e}", propuesta.display());
            return ExitCode::FAILURE;
        }
    };
    let nodo = match ore_core::parse::parse(&texto) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("error: `{}` no analiza: {e:?}", propuesta.display());
            return ExitCode::FAILURE;
        }
    };
    let p = match Propuesta::abrir(&nodo) {
        Ok(p) => p,
        Err(m) => {
            eprintln!("error: {m}");
            return ExitCode::FAILURE;
        }
    };

    let pkg = match crate::cargar_valido(paquete, true) {
        Ok(p) => p,
        Err(c) => return c,
    };

    // La quinta identidad, recontrastada. Se expande la vista por la que la
    // propuesta dice haber entrado y se compara el digest: si cambió, la
    // propuesta se decidió mirando otro recorte, y aplicarla tocaría filas que
    // aquella vista no dejaba ver.
    let vista_ahora = por_donde_entra(&pkg, &p);
    let vista_ok = match &vista_ahora {
        Some(d) => d == &p.bajo.vista,
        // Sin vista que expandir no hay nada que contrastar, y eso NO es un
        // «coincide»: es que no se pudo mirar.
        None => false,
    };

    let rechazos = cotejar(&pkg, &p);

    print!(
        "{}",
        ore_core::propuesta::imprimir(
            &p,
            &[
                ("bundle", p.bajo.bundle == ore_core::digest::bundle(&pkg)),
                ("vista", vista_ok),
            ]
        )
    );
    for (nombre, valor) in [
        ("topologia", &p.bajo.topologia),
        ("plan", &p.bajo.plan),
    ] {
        println!("  {nombre:<9} {valor} · sin verificar aquí");
    }
    if !p.bajo.testigos.is_empty() {
        for (obj, marca) in &p.bajo.testigos {
            println!("  testigo   {obj} = {marca} · sin verificar aquí");
        }
    }
    println!(
        "            —las tres las contesta un delegado; `ore` no abre sockets—"
    );

    if let (Some(ahora), false) = (&vista_ahora, vista_ok) {
        println!("\n  la vista cambió:");
        println!("    se decidió por  {}", p.bajo.vista);
        println!("    hoy se entra por {ahora}");
    }

    if rechazos.is_empty() && vista_ok {
        println!("\nok · la propuesta cae dentro de lo que el paquete autoriza");
        return ExitCode::SUCCESS;
    }
    println!();
    for r in &rechazos {
        eprintln!("rechazada: {}", r.como_texto());
    }
    if !vista_ok {
        eprintln!(
            "rechazada: no se pudo confirmar por dónde entra — la quinta identidad es lo que \
             hace auditable qué filas podía tocar"
        );
    }
    ExitCode::FAILURE
}

/// El digest del plan de la vista por la que esta propuesta entra **hoy**.
///
/// El camino es el mismo que recorre la lectura, leído al revés: el primer edit
/// nombra una propiedad, la propiedad es de una entidad, la entidad nombra su
/// vista con `backedBy`, y la vista se expande. Que sea el mismo camino no es
/// una coincidencia bonita: es lo que garantiza que *por dónde se lee* y *por
/// dónde se escribe* no puedan divergir.
///
/// Todos los edits de una propuesta comparten entidad —si no, el cotejo ya la
/// habría partido por `effects:`— así que basta el primero.
fn por_donde_entra(pkg: &ore_core::link::Package, p: &Propuesta) -> Option<String> {
    let primero = p.edits.first()?;
    let (entidad_qn, _) = primero.escribe.rsplit_once('.')?;
    let e = pkg.docs.iter().find(|d| {
        d.kind == ore_core::document::Kind::Entity && d.qname().as_deref() == Some(entidad_qn)
    })?;
    let v = ore_core::vistas::respaldo(pkg, e)?;
    let qn = v.qname()?;

    let tipos = tipos_de_raiz(pkg);
    let vistas: Vec<&Loaded> = pkg.of_view();
    let catalogo = Catalogo::con(
        vistas
            .iter()
            .filter_map(|v| Some(Vista::nueva(&v.qname()?, cuerpo(pkg, v, &tipos)))),
    );
    catalogo.expandir(&qn).ok().map(|plan| plan.digest())
}
