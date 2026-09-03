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
use crate::document::{ApiVersion, Kind};
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

/// Un efecto declarado: qué propiedad escribe, y con qué posición para el error.
///
/// **No lleva la fuente.** Hasta v1alpha7 la traía escrita —`datasourceRef`—;
/// desde v1alpha8 se deriva por el mismo camino que recorre la lectura:
///
/// ```text
/// entidad  →  backedBy  →  vista  →  raíz  →  tabla  →  datasource
/// ```
///
/// Declararla sería un segundo sitio que puede discrepar del primero, que es
/// justo el defecto que `kind: Table` vino a corregir. Y derivarla no es
/// trabajo nuevo: [`crate::vistas::datasources_de`] ya contesta esa pregunta
/// para las dos gramáticas —el `Binding` de antes y el `backedBy` de ahora—,
/// así que la regla no cambia de significado, cambia de dónde saca el dato.
struct Efecto {
    writes: String,
    /// La fuente que el documento **declaró**, si lo hizo. Solo se lee para
    /// rechazarla en v1alpha8: en las versiones donde era obligatoria, quien
    /// contesta es la derivación, igual que en la nueva.
    declarada: Option<String>,
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
                declarada: e
                    .get("datasourceRef")
                    .and_then(|(_, v)| v.as_str())
                    .map(String::from),
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

    // ── OOS1005 · `datasourceRef` se retiró del efecto en v1alpha8 ──────────
    //
    // El mismo trato que `kind: Binding`, y por la misma razón: un documento no
    // caduca por haber sido escrito antes, así que bajo v1alpha2 sigue siendo
    // obligatorio. Lo que no puede pasar es que se acepte en silencio donde ya
    // no lo lee nadie — un campo que se ignora es peor que uno que no existe,
    // porque promete algo.
    if f.version().is_some_and(|v| v >= ApiVersion::V1Alpha8) {
        for ef in efs.iter().filter(|e| e.declarada.is_some()) {
            out.push(
                Diagnostic::new(
                    Code::Oos1005,
                    &f.path,
                    "clave desconocida `datasourceRef` en un efecto".to_string(),
                )
                .at(ef.pos)
                .help(
                    "se retiró en oos.dev/v1alpha8: el destino se deriva —entidad, `backedBy`, \
                     vista, raíz, tabla— y declararlo sería un segundo sitio que puede \
                     discrepar del primero. Bórralo; `writes` se queda tal cual, porque \
                     nombrar la propiedad es correcto",
                ),
            );
        }
    }

    // ── OOS7008 · una función, una fuente ───────────────────────────────────
    //
    // Derivada, no declarada. `datasources_de` va de una entidad a lo físico
    // por los dos caminos —el binding de v1alpha2 y el `backedBy` de v1alpha8—,
    // así que un paquete que violaba esto lo sigue violando y ninguno que no lo
    // violaba empieza a hacerlo.
    let mut fuentes: Vec<String> = efs
        .iter()
        .filter_map(|e| propiedad(pkg, &e.writes))
        .flat_map(|(entidad, _)| crate::vistas::datasources_de(pkg, entidad))
        .collect();
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
        // ── `quorum` · cuantos juicios distintos ────────────────────────────
        //
        // No lleva codigo propio: es forma del documento, y la forma es
        // `OOS1004`. Un endosante mal escrito si tiene el suyo —`OOS7004`—
        // porque el vocabulario es una decision del regimen; que un entero sea
        // un entero, no.
        if let Some((k, v)) = e.get("quorum") {
            if nombre == "attested" {
                out.push(
                    Diagnostic::new(
                        Code::Oos1004,
                        &f.path,
                        "`quorum` sobre un endoso `attested`",
                    )
                    .at(k.pos())
                    .help(
                        "una atestacion es un artefacto firmado, y dos atestaciones son dos \
                         rutas distintas: ya se distinguen sin contar. `quorum` existe porque \
                         dos `humanApproval` sin atestacion colapsan en uno",
                    ),
                );
            } else if v.as_str().and_then(|s| s.parse::<u32>().ok()).unwrap_or(0) < 2 {
                out.push(
                    Diagnostic::new(
                        Code::Oos1004,
                        &f.path,
                        "`quorum` debe ser un entero de 2 en adelante",
                    )
                    .at(v.pos())
                    .help(
                        "ausente es 1, asi que escribir `quorum: 1` seria declarar lo \
                         derivable (P2)",
                    ),
                );
            }
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

        // ── OOS7012 · el objeto no acepta que lo actualicen ─────────────────
        //
        // Esto es lo que la cara `W` paga. Un efecto cambia una propiedad de
        // algo que YA ESTÁ, así que es un `update`; y si la tabla de la que
        // sale la entidad no lo acepta, el paquete promete una escritura que el
        // origen rechazaría — en producción, y no al compilar.
        //
        // La ausencia de `writes` es una negativa, igual que `reads: none`, así
        // que una tabla que se calla también falla aquí. Es la doctrina de esta
        // casa desde v1alpha1: lo que no se declara, no se puede.
        //
        // Solo se comprueba cuando la entidad sale de una `Table`. En una
        // cadena de v1alpha7 no hay objeto que declare caras, así que no hay a
        // quién preguntar y no se inventa la respuesta.
        // ── OOS7013 · escribir es `Q⁻¹`, y no toda `Q` se invierte ──────────
        //
        // Se comprueba la cadena entera y no solo la vista que la entidad
        // nombra: componer no diluye: si un eslabón de abajo agrega, lo que
        // sale de arriba tampoco se puede deshacer.
        //
        // **Hoy no puede fallar por ningún documento**, y está dicho donde se
        // define. Lo que hace es que el constructor que se añada mañana tenga
        // que decidir si es invertible, en vez de heredar un «sí» tácito.
        if let Some(vista) = crate::vistas::respaldo(pkg, entidad)
            && let Ok(cadena) = crate::vistas::cadena(pkg, vista)
            && let Some(no) = cadena
                .iter()
                .find_map(|v| crate::vistas::invertible(v).err())
        {
            let (donde, porque) = match &no {
                crate::vistas::NoInvertible::CampoCalculado { vista, campo } => (
                    vista.clone(),
                    format!("`{campo}` no sale de una columna: sale de calcularla"),
                ),
                crate::vistas::NoInvertible::ConstruccionDesconocida { vista, clave } => (
                    vista.clone(),
                    format!("declara `{clave}`, y nadie ha dicho si eso se invierte"),
                ),
            };
            out.push(
                Diagnostic::new(
                    Code::Oos7013,
                    &f.path,
                    format!("`{}` entra por `{donde}`, que {porque}", ef.writes),
                )
                .at(ef.pos)
                .help(
                    "escribir a través de una vista es deshacer la pregunta que la vista hace, \
                     y no toda pregunta se deshace: renombrar es una biyección y recortar deja \
                     la fila dentro o fuera, pero de una agregación no se vuelve. O la entidad \
                     se respalda de una vista que sí se invierta, o el efecto va a otra \
                     propiedad",
                ),
            );
            continue;
        }

        if let Some(vista) = crate::vistas::respaldo(pkg, entidad)
            && let Ok(raiz) = crate::vistas::raiz(pkg, vista)
            && let Some(tqn) = raiz.tabla.as_deref()
            && let Some(tabla) = pkg.table(tqn)
        {
            let ops = crate::document::escrituras(tabla.section("writes"));
            if !ops.iter().any(|o| o == "update") {
                let dice = if ops.is_empty() {
                    "no declara `writes`".to_string()
                } else {
                    format!("declara `writes: [{}]`", ops.join(", "))
                };
                out.push(
                    Diagnostic::new(
                        Code::Oos7012,
                        &f.path,
                        format!(
                            "`{}` sale de `{tqn}`, que {dice}",
                            ef.writes
                        ),
                    )
                    .at(ef.pos)
                    .help(format!(
                        "un efecto cambia una propiedad de algo que ya está, así que es un \
                         `update`, y `{tqn}` no lo acepta. La ausencia es una negativa —igual \
                         que `reads: none`—, así que si el origen sí lo acepta hay que \
                         decirlo: añade `update` a su `writes`, y con él `changes.key`, que es \
                         lo que dice qué fila se toca"
                    )),
                );
                continue;
            }
        }

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

    /// **`OOS7013` se dispara de verdad.**
    ///
    /// La guarda de invertibilidad ya está probada en `vistas`, pero probar el
    /// clasificador y no el cable deja escapar el fallo que importa: que la
    /// regla esté escrita y no se llame. Aquí se comprueba **el diagnóstico**.
    ///
    /// Ningún documento OOS puede llegar hasta aquí hoy —un `groupBy` muere
    /// antes, en `OOS1005`—, así que el paquete se arma sin pasar por el
    /// validador. Es deliberado, y es lo mismo que hace el IR de `ore-view` con
    /// `Agrupa`: se ejerce el camino que todavía no tiene tráfico, para que el
    /// día que lo tenga ya esté recorrido.
    #[test]
    fn un_efecto_a_traves_de_una_vista_no_invertible_no_compila() {
        use crate::parse::parse;
        use std::path::PathBuf;

        fn doc(kind: Kind, lineas: &[&str]) -> Loaded {
            Loaded {
                path: PathBuf::from("caso.yaml"),
                kind,
                root: parse(&lineas.join("\n")).expect("yaml"),
            }
        }

        // El mismo paquete dos veces, y lo unico que cambia es la vista. Se
        // arma en vez de clonarse porque `Loaded` no es `Clone` — un documento
        // cargado tiene una ruta, y dos copias con la misma ruta serian dos
        // verdades sobre un fichero.
        fn armar(vista: &[&str]) -> Package {
            Package {
                root: PathBuf::from("."),
                docs: vec![
                    doc(
                        Kind::Table,
                        &[
                            "apiVersion: oos.dev/v1alpha8",
                            "kind: Table",
                            "metadata: { name: employees, namespace: erp }",
                            "spec:",
                            "  datasource: erp",
                            "  object: 'public.employees'",
                            "  columns: { employee_id: {}, country: {} }",
                            "  reads: { fullScan: cheap }",
                            "  changes: { mode: retract, witness: log, key: [employee_id] }",
                            "  writes: [insert, update, delete]",
                        ],
                    ),
                    doc(Kind::View, vista),
                    doc(
                        Kind::Entity,
                        &[
                            "apiVersion: oos.dev/v1alpha1",
                            "kind: Entity",
                            "metadata: { name: Pais, namespace: hr }",
                            "spec:",
                            "  nature: entity",
                            "  primaryKey: [pais]",
                            "  backedBy: hr.por_pais",
                            "  properties:",
                            "    pais: { type: String }",
                        ],
                    ),
                    doc(
                        Kind::Function,
                        &[
                            "apiVersion: oos.dev/v1alpha8",
                            "kind: Function",
                            "metadata: { name: renombrar, namespace: hr }",
                            "spec:",
                            "  runtime: wasm",
                            "  entrypoint: dist/r.wasm",
                            "  effects:",
                            "    - writes: hr.Pais.pais",
                            "      to: 'ES'",
                        ],
                    ),
                ],
                cedar: Vec::new(),
                generated: Vec::new(),
                sobres: Vec::new(),
            }
        }

        const CABECERA: &[&str] = &[
            "apiVersion: oos.dev/v1alpha8",
            "kind: View",
            "metadata: { name: por_pais, namespace: hr }",
            "spec:",
            "  owner: team:rrhh",
            "  from: { table: erp.employees }",
            "  fields: { pais: country }",
        ];

        // Con el constructor que la gramatica todavia no tiene.
        let mut con_groupby = CABECERA.to_vec();
        con_groupby.push("  groupBy: [country]");
        let diags = check(&armar(&con_groupby));
        let d = diags
            .iter()
            .find(|d| d.code == Code::Oos7013)
            .unwrap_or_else(|| {
                panic!(
                    "sin OOS7013; salieron {:?}",
                    diags.iter().map(|d| d.code).collect::<Vec<_>>()
                )
            });
        assert!(
            d.message.contains("hr.por_pais") && d.message.contains("groupBy"),
            "el mensaje tiene que nombrar la vista y lo que la hace no invertible: {}",
            d.message
        );

        // Y el gemelo: la misma forma sin el —que es lo unico que hoy se puede
        // escribir— no da `OOS7013`.
        assert!(
            !check(&armar(CABECERA))
                .iter()
                .any(|d| d.code == Code::Oos7013),
            "sin el constructor nuevo no hay nada que invertir mal"
        );
    }
}
