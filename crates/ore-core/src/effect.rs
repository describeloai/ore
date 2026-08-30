//! Efectos e integridad — la familia `OOS7xxx`.
//!
//! El dual de [`flow`](crate::flow). Donde aquel gobierna **lo que se puede
//! saber**, este gobierna **lo que se puede causar**, y la regla es la misma
//! frase con el signo cambiado:
//!
//! > Una función no puede declarar un efecto sobre una propiedad cuya
//! > integridad exigida supere la suya, salvo que atraviese un endosante
//! > autorizado.
//!
//! `I(función) ⊒ I(destino)`.
//!
//! # Por qué la regla se escribe sobre la función y no sobre quien invoca
//!
//! La formulación que sale de la intuición es «el actor que invoca alcanza la
//! integridad que el destino exige». Es la correcta en ejecución y es
//! **inservible al compilar**: al compilar no hay actor, y una regla que
//! dependiera de él dejaría todo el régimen fuera de L0.
//!
//! Así que el sujeto es la función, y `I(función)` no es la identidad de nadie:
//! es **el nivel de garantía que la función ha ganado** con sus endosos. Quién
//! invoca lo decide Cedar, en ejecución, y es L3.
//!
//! # Lo que esta fase todavía no hace
//!
//! Un endoso `attested` se toma **por su palabra**: se comprueba que está
//! declarado y que nombra una atestación, no que la firma sea válida. Verificar
//! la firma contra una clave del lock es lo que convierte el endoso en garantía
//! en lugar de declaración, y está pendiente. Se dice aquí para que nadie lo
//! deduzca de que los casos pasan.

use crate::code::Code;
use crate::diag::Diagnostic;
use crate::document::Kind;
use crate::flow::{self, Axis, Lattice};
use crate::link::{Loaded, Package};
use crate::parse::Node;
use std::collections::BTreeMap;

/// El vocabulario **cerrado** de endosantes.
///
/// Dos, no cinco, porque solo hay dos formas de ganar confianza de manera
/// comprobable: demostrarla una vez y dejarlo escrito, o pagarla en cada uso.
/// Los candidatos evidentes se caen o colapsan — una revisión de `CODEOWNERS` y
/// una firma son la misma cosa (lo verificable es la constancia, no el acto), y
/// una suite en verde exige ejecutarla, que es L2.
const ENDOSANTES: &[&str] = &["attested", "humanApproval"];

pub fn check(pkg: &Package) -> Vec<Diagnostic> {
    let lat = flow::lattices(pkg);
    let mut out = Vec::new();

    // 1 · El retículo antes que nada: comparar contra un eje incoherente es
    //     comparar contra la nada.
    reticulos(pkg, &lat, &mut out);
    if !out.is_empty() {
        return out;
    }

    // 2 · Toda etiqueta de integridad tiene que pertenecer a su retículo.
    etiquetas(pkg, &lat, &mut out);
    if !out.is_empty() {
        return out;
    }

    // 3 · La regla, función a función.
    for f in pkg.docs.iter().filter(|d| d.kind == Kind::Function) {
        funcion(pkg, f, &lat, &mut out);
    }

    // 4 · Y el efecto sobre la identidad.
    for r in pkg.docs.iter().filter(|d| d.kind == Kind::Resolution) {
        resolucion(pkg, r, &lat, &mut out);
    }
    out
}

// ── Resolution · OOS7009 · OOS7011 ──────────────────────────────────────────

/// ¿Tiene este documento algún endoso **incondicional**?
///
/// Compartido entre `Function` y `Resolution` a propósito: un régimen con dos
/// formas de decir «que lo mire una persona» acaba con dos semánticas.
fn endosada(d: &Loaded) -> bool {
    d.section("endorsements")
        .map(|n| n.items())
        .unwrap_or(&[])
        .iter()
        .any(|e| {
            e.get("when").is_none()
                && e.get("endorser")
                    .and_then(|(_, v)| v.as_str())
                    .is_some_and(|n| ENDOSANTES.contains(&n))
        })
}

fn resolucion(
    pkg: &Package,
    r: &Loaded,
    lat: &BTreeMap<String, Lattice>,
    out: &mut Vec<Diagnostic>,
) {
    let qn = r.qname().unwrap_or_default();
    let estrategias = r.section("strategies").map(|n| n.items()).unwrap_or(&[]);

    let mut probabilistica = false;
    for e in estrategias {
        if e.get("type").and_then(|(_, v)| v.as_str()) != Some("probabilistic") {
            continue;
        }
        probabilistica = true;
        let id = e
            .get("id")
            .and_then(|(_, v)| v.as_str())
            .unwrap_or("<sin id>");

        // ── OOS7009 · un emparejador probabilístico es un conducto ──────────
        if e.get("conduit").is_none() {
            out.push(
                Diagnostic::new(
                    Code::Oos7009,
                    &r.path,
                    format!("la estrategia `{id}` de `{qn}` no declara conducto"),
                )
                .at(e.pos())
                .help(
                    "comparar nombres y direcciones a escala no es una operación de rendimiento: \
                     es hacer fluir esos valores hacia un emparejador que tiene que sostenerlos \
                     para compararlos. Eso es un conducto en el sentido literal de `04-flow`, y \
                     un conducto sin autorización declarada no se autoriza solo. Declara \
                     `conduit: materialization.<nombre>` y autorízalo a la etiqueta de cada \
                     propiedad que ponderas",
                ),
            );
        }
    }

    if !probabilistica || endosada(r) {
        return;
    }

    // ── OOS7011 · una coincidencia probable no produce un hecho ─────────────
    let Some(entidad) = r
        .section("entity")
        .and_then(|n| n.as_str())
        .and_then(|q| pkg.entity(q))
    else {
        return;
    };
    let Some((_, meta)) = entidad.root.get("metadata") else {
        return;
    };
    for (ret, nivel) in integridad_de(meta, lat) {
        let l = &lat[&ret];
        // El techo es «no el máximo», y no un nivel con nombre: obligar a cada
        // retículo a declarar cuál de sus niveles significa «inferido» sería
        // vocabulario nuevo para decir algo que la posición ya dice.
        if l.levels.last() != Some(&nivel) || l.levels.len() < 2 {
            continue;
        }
        out.push(
            Diagnostic::new(
                Code::Oos7011,
                &entidad.path,
                format!(
                    "`{}` declara `{ret} = {nivel}` y `{qn}` la resuelve por conjetura",
                    entidad.qname().unwrap_or_default()
                ),
            )
            .at(meta.pos())
            .help(format!(
                "una estrategia probabilística infiere: por bien calibrada que esté produce una \
                 conclusión, no una observación, y `{nivel}` es la cima de `{ret}`. Sea lo que \
                 sea esa cima, una conjetura no es eso — el umbral no cambia la naturaleza del \
                 método. Baja la etiqueta a `{}`, o declara un endoso incondicional: alguien \
                 mira los dos registros y se hace responsable de la fusión",
                l.levels[l.levels.len() - 2]
            )),
        );
    }
}

// ── OOS7007 ─────────────────────────────────────────────────────────────────

fn reticulos(pkg: &Package, lat: &BTreeMap<String, Lattice>, out: &mut Vec<Diagnostic>) {
    for d in pkg.docs.iter().filter(|d| d.kind == Kind::Lattice) {
        let Some(q) = d.qname() else { continue };
        let Some(l) = lat.get(&q) else { continue };
        let Some(nodo) = d.section("join") else {
            continue;
        };
        let declarado = nodo.as_str().unwrap_or_default();
        if declarado == l.axis.combinador() {
            continue;
        }
        out.push(
            Diagnostic::new(
                Code::Oos7007,
                &d.path,
                format!(
                    "`{q}` es de eje `{}` y declara `join: {declarado}`",
                    eje(l.axis)
                ),
            )
            .at(nodo.pos())
            .help(format!(
                "un retículo de eje `{}` combina por `{}`, y el combinador se deriva del \
                 eje: es un campo derivable, luego no declarable (P2). `join` se admite por \
                 compatibilidad con v1alpha1 y se exige que coincida — aceptarlo en silencio \
                 dejaría un documento que dice una cosa y un compilador que hace otra, y en \
                 integridad eso es la diferencia entre propagar confianza y lavarla",
                eje(l.axis),
                l.axis.combinador()
            )),
        );
    }
}

const fn eje(a: Axis) -> &'static str {
    match a {
        Axis::Confidentiality => "confidentiality",
        Axis::Integrity => "integrity",
    }
}

// ── OOS7003 ─────────────────────────────────────────────────────────────────

/// Las etiquetas de integridad de una propiedad: retículo → nivel.
fn integridad_de(n: &Node, lat: &BTreeMap<String, Lattice>) -> BTreeMap<String, String> {
    n.get("labels")
        .map(|(_, l)| {
            l.entries()
                .iter()
                .filter_map(|(k, v)| {
                    let r = k.as_str()?.to_string();
                    let nivel = v.as_str()?.to_string();
                    lat.get(&r)
                        .filter(|x| x.axis == Axis::Integrity)
                        .map(|_| (r, nivel))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn etiquetas(pkg: &Package, lat: &BTreeMap<String, Lattice>, out: &mut Vec<Diagnostic>) {
    for e in pkg.entities() {
        let qn = e.qname().unwrap_or_default();
        for (k, v) in e.section("properties").map(|n| n.entries()).unwrap_or(&[]) {
            let Some(prop) = k.as_str() else { continue };
            for (ret, nivel) in integridad_de(v, lat) {
                let l = &lat[&ret];
                if l.levels.contains(&nivel) {
                    continue;
                }
                out.push(
                    Diagnostic::new(
                        Code::Oos7003,
                        &e.path,
                        format!("`{qn}.{prop}`: `{nivel}` no es un nivel de `{ret}`"),
                    )
                    .at(v.pos())
                    .help(format!(
                        "los niveles de `{ret}` son {}. En un retículo de integridad esto \
                         importa más que en uno de confidencialidad: comparar contra un nivel \
                         inexistente no produce un error de comparación, produce una \
                         comparación que nadie sabe resolver, y cualquier resolución por \
                         defecto concede o deniega en silencio",
                        l.levels.join(" ⊑ ")
                    )),
                );
            }
        }
    }
}

// ── La regla, función a función ─────────────────────────────────────────────

/// Un efecto declarado: qué escribe, dónde, y con qué posición para el error.
struct Efecto {
    writes: String,
    datasource: String,
    pos: crate::diag::Pos,
}

fn efectos(f: &Loaded) -> Vec<Efecto> {
    f.section("effects")
        .map(|n| n.items())
        .unwrap_or(&[])
        .iter()
        .filter_map(|e| {
            Some(Efecto {
                writes: e.get("writes")?.1.as_str()?.to_string(),
                datasource: e
                    .get("datasourceRef")
                    .and_then(|(_, v)| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                pos: e.pos(),
            })
        })
        .collect()
}

/// Localiza la propiedad `<ns>.<Entidad>.<prop>` que un efecto nombra.
fn propiedad<'a>(pkg: &'a Package, qname: &str) -> Option<(&'a Loaded, &'a Node)> {
    let (entidad, prop) = qname.rsplit_once('.')?;
    let e = pkg.entity(entidad)?;
    let (_, v) = e.section("properties")?.get(prop)?;
    Some((e, v))
}

fn funcion(pkg: &Package, f: &Loaded, lat: &BTreeMap<String, Lattice>, out: &mut Vec<Diagnostic>) {
    let qn = f.qname().unwrap_or_default();
    let efs = efectos(f);

    // ── OOS7008 · una función, una fuente ───────────────────────────────────
    let mut fuentes: Vec<&str> = efs.iter().map(|e| e.datasource.as_str()).collect();
    fuentes.sort_unstable();
    fuentes.dedup();
    if fuentes.len() > 1 {
        out.push(
            Diagnostic::new(
                Code::Oos7008,
                &f.path,
                format!("`{qn}` declara efectos sobre {}", fuentes.join(" y ")),
            )
            .at(efs.first().map(|e| e.pos).unwrap_or(f.root.pos()))
            .help(
                "no hay transacción que abarque dos fuentes, y un régimen que promete lo que \
                 no cumple no gobierna nada. Una implementación que lo aceptara fallaría a \
                 medias en producción —estado escrito, auditoría ausente— y el paquete habría \
                 compilado. Divide la función, o acepta que son dos operaciones",
            ),
        );
        return;
    }

    // ── OOS7004 · el vocabulario cerrado ────────────────────────────────────
    let mut incondicionales = 0usize;
    for e in f.section("endorsements").map(|n| n.items()).unwrap_or(&[]) {
        let nombre = e
            .get("endorser")
            .and_then(|(_, v)| v.as_str())
            .unwrap_or_default();
        if !ENDOSANTES.contains(&nombre) {
            out.push(
                Diagnostic::new(
                    Code::Oos7004,
                    &f.path,
                    format!("`{nombre}` no es un endosante de OOS"),
                )
                .at(e.pos())
                .help(format!(
                    "el vocabulario es cerrado: {}. Un endosante que el motor no sabe \
                     verificar es una promesa, y una promesa no es una garantía — una revisión \
                     de equipo no es verificable sin red, pero la constancia firmada de que \
                     ocurrió sí, y por eso colapsa en `attested`",
                    ENDOSANTES.join(" y ")
                )),
            );
            continue;
        }
        // ── OOS7002 · un endoso condicional no cierra una carencia ──────────
        if e.get("when").is_none() {
            incondicionales += 1;
        }
    }
    if !out.is_empty() {
        return;
    }

    // `I(función)`: sin endoso incondicional, el mínimo del retículo. No es un
    // castigo — es que **no hay nada que la eleve**.
    let atestada = incondicionales > 0;

    for ef in &efs {
        let Some((entidad, def)) = propiedad(pkg, &ef.writes) else {
            continue; // referencia rota: es `OOS2005`, y ya falló antes
        };

        // ── OOS7006 · lo derivado no se escribe ─────────────────────────────
        if def.get("derivedFrom").is_some() {
            out.push(
                Diagnostic::new(
                    Code::Oos7006,
                    &f.path,
                    format!(
                        "`{}` se computa: no puede ser destino de un efecto",
                        ef.writes
                    ),
                )
                .at(ef.pos)
                .help(
                    "declarar que una propiedad derivada además se escribe es afirmar dos \
                     orígenes para el mismo valor, y el compilador no puede saber cuál gana: \
                     si el efecto escribe 121 y la derivación computa 118, el paquete afirma \
                     las dos cosas, y la próxima compilación recomputaría y borraría la \
                     escritura sin decir nada. Es el dual exacto de `OOS4008`",
                ),
            );
            continue;
        }

        let exigido = integridad_de(def, lat);

        // ── OOS7005 · declara o falla ───────────────────────────────────────
        if exigido.is_empty() {
            out.push(
                Diagnostic::new(
                    Code::Oos7005,
                    &f.path,
                    format!(
                        "`{}` no declara integridad y es destino de un efecto",
                        ef.writes
                    ),
                )
                .at(ef.pos)
                .help(
                    "es `OOS4011` con el signo cambiado: un conducto sin autorización \
                     declarada no se autoriza solo. Denegación por defecto no es «asume lo \
                     peor» —eso paralizaría cualquier ontología— sino «declara o falla»: no se \
                     obtiene una escritura sin decir qué integridad exige el destino",
                ),
            );
            continue;
        }

        // ── OOS7001 · lo que la función lee arrastra hacia abajo ────────────
        //
        // `meet`, no `join`: un cómputo no es más fiable que su entrada menos
        // fiable. La atestación dice que el CÓDIGO es de fiar, no que la
        // ENTRADA lo sea, y sin esta cláusula una firma lavaría la procedencia.
        let leidas = lee(f, entidad, lat);

        for (ret, nivel) in &exigido {
            let l = &lat[ret];
            let Some(i_destino) = l.levels.iter().position(|x| x == nivel) else {
                continue;
            };

            // Lo que arrastra: el mínimo de lo leído en este mismo retículo.
            let arrastre = leidas
                .iter()
                .filter(|(r, _, _)| r == ret)
                .filter_map(|(_, n, p)| l.levels.iter().position(|x| x == n).map(|i| (i, n, p)))
                .min_by_key(|(i, _, _)| *i);

            let i_funcion = if atestada { l.levels.len() - 1 } else { 0 };

            if let Some((i_leida, nivel_leido, prop)) = arrastre
                && i_leida < i_destino
                && i_funcion >= i_destino
            {
                out.push(
                    Diagnostic::new(
                        Code::Oos7001,
                        &f.path,
                        format!(
                            "`{}` alcanza `{ret} = {nivel}`, pero `{qn}` lee `{prop}` con \
                             `{nivel_leido}` (computado por meet)",
                            ef.writes
                        ),
                    )
                    .at(ef.pos)
                    .help(
                        "nadie escribió esa integridad: la computó el compilador propagando \
                         `meet` por lo que la función lee. La atestación dice que el código es \
                         de fiar, no que la entrada lo sea, y un promedio no limpia un dato \
                         sucio. Elevarlo exige un endoso, que es la única forma declarada de \
                         decir «me hago responsable»",
                    ),
                );
                continue;
            }

            // ── OOS7002 · la función no llega ───────────────────────────────
            if i_funcion < i_destino {
                out.push(
                    Diagnostic::new(
                        Code::Oos7002,
                        &f.path,
                        format!(
                            "`{}` exige `{ret} = {nivel}` y `{qn}` alcanza `{}`",
                            ef.writes, l.levels[i_funcion]
                        ),
                    )
                    .at(ef.pos)
                    .help(
                        "la integridad de una función se computa de sus endosos, y sin endosos \
                         incondicionales es el mínimo del retículo — un endoso con `when` no \
                         cierra una carencia: si la condición es falsa no hay elevación, y la \
                         carencia sigue abierta justo en el caso que importa. La salida no es \
                         firmarlo todo: es `humanApproval`, y atestar el bundle es lo que lo \
                         convierte de requisito en opción",
                    ),
                );
            }
        }
    }
}

/// Las propiedades que una función **lee**, con su integridad.
///
/// Hoy son las que sus precondiciones nombran como `target.<prop>`, resueltas
/// contra la entidad de sus efectos. Los parámetros de `input` quedan fuera a
/// propósito: vienen de quien invoca, y la integridad de quien invoca es una
/// propiedad de ejecución — L3, no L0.
fn lee(
    f: &Loaded,
    entidad: &Loaded,
    lat: &BTreeMap<String, Lattice>,
) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for p in f.section("preconditions").map(|n| n.items()).unwrap_or(&[]) {
        let Some(expr) = p.get("expr").and_then(|(_, v)| v.as_str()) else {
            continue;
        };
        for prop in referencias(expr) {
            let Some((_, def)) = entidad.section("properties").and_then(|ps| ps.get(&prop)) else {
                continue;
            };
            for (ret, nivel) in integridad_de(def, lat) {
                out.push((ret, nivel, prop.clone()));
            }
        }
    }
    out
}

/// Los `target.<algo>` de una expresión. No es un analizador de CEL: es
/// exactamente lo que hace falta para saber qué lee, y el día que haga falta
/// evaluar una expresión la respuesta será enlazar un motor de CEL, no ampliar
/// esta función.
fn referencias(expr: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut resto = expr;
    while let Some(i) = resto.find("target.") {
        let tras = &resto[i + "target.".len()..];
        let fin = tras
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .unwrap_or(tras.len());
        if fin > 0 {
            out.push(tras[..fin].to_string());
        }
        resto = &tras[fin..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extrae_lo_que_una_precondicion_lee() {
        assert_eq!(
            referencias("target.status == \"OK\" && target.supplierScore < 80"),
            ["status", "supplierScore"]
        );
        assert!(referencias("subject.approvalLimit > 0").is_empty());
    }

    #[test]
    fn el_combinador_sale_del_eje() {
        assert_eq!(Axis::Confidentiality.combinador(), "max");
        assert_eq!(Axis::Integrity.combinador(), "min");
    }
}
