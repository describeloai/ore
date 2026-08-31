//! `OOS3006` — que `via` case la clave del destino.
//!
//! # Por qué esto no es `OOS2005` ni `OOS3005`
//!
//! Cuando `via` nombra una propiedad que no existe, eso es `OOS2005`: una
//! referencia rota. Cuando la cardinalidad afirma una unicidad que las claves
//! locales no sostienen, eso es `OOS3005`. Esta comprobación es la tercera, y
//! solo salta cuando **todo resuelve**: las propiedades existen, el destino
//! existe, la cardinalidad es coherente… y el enlace **une de menos**.
//!
//! Es la forma de fallo que este proyecto persigue en todas partes. Un `via` de
//! una propiedad contra una `primaryKey` de dos produce filas de más, en
//! silencio, y quien lea el documento no verá nada raro: **una relación que une
//! de menos tiene exactamente el mismo aspecto que una correcta.**
//!
//! # Por qué no lo alcanza un esquema JSON
//!
//! Porque hay que resolver `target` y leer la `primaryKey` de **otro documento**.
//! Es la misma razón que puso `OOS2005` aquí y no en el esquema: resolver un
//! nombre exige el paquete entero.
//!
//! # Lo que se comprueba, y lo que no
//!
//! Aridad y tipos, posición a posición. El emparejamiento posicional es lo que
//! permite decir una clave foránea compuesta sin declarar el lado del padre —
//! `target` ya publica su clave (P2)— y también es su pie de banco: con
//! `via: [codPais, id]` contra `primaryKey: [id, codPais]` el enlace une por los
//! pares cambiados.
//!
//! Los tipos cazan esa transposición **cuando difieren**, que es el caso común:
//! `(Integer, String)` frente a `(String, Integer)` no casa y salta. Cuando no
//! difieren —dos `String`— **no la caza**, y eso se dice aquí en vez de fingir
//! que la regla es total. Una comprobación con una excepción no anunciada es
//! peor que una comprobación que declara su límite.

use crate::code::Code;
use crate::diag::Diagnostic;
use crate::link::Package;
use crate::parse::Node;

pub fn comprobar(pkg: &Package, out: &mut Vec<Diagnostic>) {
    for e in pkg.entities() {
        let qn = e.qname().unwrap_or_default();
        let Some(rels) = e.section("relations") else {
            continue;
        };
        for (rk, rv) in rels.entries() {
            let rn = rk.as_str().unwrap_or("");
            let (Some((_, tnodo)), Some((_, vianodo))) = (rv.get("target"), rv.get("via")) else {
                continue;
            };
            let destino = tnodo.as_str().unwrap_or("");
            // Si no resuelve, ya lo dijo OOS2005 y repetirlo sería ruido.
            let Some(otra) = pkg.resolve_entity(destino, e) else {
                continue;
            };
            let via = lista(vianodo);
            let clave = otra.section("primaryKey").map(lista).unwrap_or_default();
            if via.is_empty() || clave.is_empty() {
                continue;
            }

            let dqn = otra.qname().unwrap_or_default();
            if via.len() != clave.len() {
                out.push(
                    Diagnostic::new(
                        Code::Oos3006,
                        &e.path,
                        format!(
                            "`{qn}.{rn}` enlaza por {} propiedad{} y `{dqn}` se identifica con {}",
                            via.len(),
                            if via.len() == 1 { "" } else { "es" },
                            clave.len()
                        ),
                    )
                    .at(vianodo.pos())
                    .help(format!(
                        "`via` se empareja posición a posición con la `primaryKey` del \
                         destino, que es [{}]. Un enlace más corto une de menos y produce \
                         filas de más sin que nada lo delate; uno más largo une por algo \
                         que el destino no usa para identificarse",
                        clave.join(", ")
                    )),
                );
                continue;
            }

            // Misma aridad: quedan los tipos, posición a posición.
            for (i, (local, remota)) in via.iter().zip(clave.iter()).enumerate() {
                let (Some(tl), Some(tr)) = (tipo(e, local), tipo(otra, remota)) else {
                    continue;
                };
                if tl != tr {
                    out.push(
                        Diagnostic::new(
                            Code::Oos3006,
                            &e.path,
                            format!(
                                "`{qn}.{rn}` empareja `{local}: {tl}` con `{dqn}.{remota}: {tr}`"
                            ),
                        )
                        .at(vianodo.pos())
                        .help(format!(
                            "en la posición {} de `via`. El orden es semántico: se empareja \
                             con [{}] en ese mismo orden, así que dos propiedades \
                             intercambiadas se ven aquí — siempre que sus tipos difieran",
                            i + 1,
                            clave.join(", ")
                        )),
                    );
                }
            }
        }
    }
}

fn lista(n: &Node) -> Vec<String> {
    n.items()
        .iter()
        .filter_map(|i| i.as_str().map(String::from))
        .collect()
}

fn tipo(e: &crate::link::Loaded, prop: &str) -> Option<String> {
    e.section("properties")?
        .get(prop)?
        .1
        .get("type")?
        .1
        .as_str()
        .map(String::from)
}
