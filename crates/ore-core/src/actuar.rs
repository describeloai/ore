//! v1alpha10 — **actuar**: la superficie de una función en los dos sentidos,
//! y la forma y las referencias de una `Action`.
//!
//! La regla de la versión (`00-scope` §1): `lee(f) ⊆ reads(f)` y
//! `causa(f) ⊆ effects(f)`. Lo que aquí se comprueba es la mitad que se
//! resuelve **por referencia** —`over` y `reads` a vistas, `writes` a
//! propiedades, `call` a una función— y la forma de una acción, que no cabe en
//! «esta clave existe». La integridad (`OOS7001`, `OOS7002`) sigue en
//! `effect.rs`, que ahora también recorre las acciones.
//!
//! # `writes` que no resuelve es `OOS2005`, en todas las versiones
//!
//! `02-function` §8 (v1alpha2) lo decía y el compilador no lo hacía: un
//! efecto sobre una propiedad o una entidad que no existe compilaba, y retirar
//! la entidad con la función puesta también (medido el 2026-09-17,
//! `pruebas-de-fuego/medida-forge-function.py` ②c). `effect::propiedad`
//! devolvía `None` y el efecto se saltaba con el comentario «es `OOS2005`, y ya
//! falló antes». No había fallado. Aquí falla, y para todas las versiones,
//! porque no es una regla nueva: es la de siempre, aplicada.
//!
//! # La lectura no declarada es `OOS7014`, y solo desde v1alpha10
//!
//! Antes no había dónde declararla. Un `over` que no resuelve, un `reads` que
//! no resuelve, o una precondición que mira `target.x` cuando `over` no expone
//! `x`: lo que no está en la superficie no existe para la función, y por eso no
//! fluye ni arrastra.

use crate::code::Code;
use crate::diag::Diagnostic;
use crate::document::{ApiVersion, Kind};
use crate::link::{Loaded, Package};
use crate::parse::Node;

pub fn comprobar(pkg: &Package, out: &mut Vec<Diagnostic>) {
    for f in pkg.of(Kind::Function) {
        escrituras(pkg, f, "effects", out);
        if f.version().is_some_and(|v| v >= ApiVersion::V1Alpha10) {
            toca_algo(f, out);
            lectura(pkg, f, out);
        }
    }
    for a in pkg.of(Kind::Action) {
        forma_de_accion(pkg, a, out);
        escrituras(pkg, a, "sets", out);
        lectura(pkg, a, out);
    }
}

// ── OOS2005 · `writes` resuelve a una propiedad de una entidad ──────────────

fn escrituras(pkg: &Package, d: &Loaded, seccion: &str, out: &mut Vec<Diagnostic>) {
    for e in d.section(seccion).map(|n| n.items()).unwrap_or(&[]) {
        let Some((_, nodo)) = e.get("writes") else {
            continue;
        };
        let Some(referencia) = nodo.as_str() else {
            continue;
        };
        let Some((entidad, prop)) = referencia.rsplit_once('.') else {
            continue; // la forma la comprueba el esquema
        };
        let Some(e) = pkg.resolve_entity(entidad, d) else {
            out.push(
                crate::link::referencia_rota(&d.path, nodo, referencia, "writes").help(format!(
                    "`{entidad}` no es ninguna entidad del árbol. Un efecto nombra \
                     `<ns>.<Entidad>.<propiedad>`, y la entidad tiene que estar: retirarla con \
                     la función puesta deja una escritura sobre nada"
                )),
            );
            continue;
        };
        let existe = e
            .section("properties")
            .is_some_and(|p| p.get(prop).is_some());
        if !existe {
            let qn = e.qname().unwrap_or_default();
            out.push(
                crate::link::referencia_rota(&d.path, nodo, referencia, "writes").help(format!(
                    "`{qn}` existe y no declara la propiedad `{prop}`. Un efecto escribe una \
                     propiedad declarada: la ontología no puede afirmar un hecho sobre un \
                     campo que no significa nada"
                )),
            );
        }
    }
}

// ── OOS1004 · una función toca algo ─────────────────────────────────────────

fn toca_algo(f: &Loaded, out: &mut Vec<Diagnostic>) {
    let toca = ["over", "reads", "effects"]
        .iter()
        .any(|s| f.section(s).is_some());
    if !toca {
        out.push(
            Diagnostic::new(
                Code::Oos1004,
                &f.path,
                format!(
                    "`{}` no toca nada: ni `over`, ni `reads`, ni `effects`",
                    f.qname().unwrap_or_default()
                ),
            )
            .at(f.root.pos())
            .help(
                "una función lee, edita o infiere sobre la copia, y las tres cosas se declaran: \
                 `over` (la vista cuyas filas trabaja), `reads` (las que puede leer) o \
                 `effects` (lo que afirma). Un documento que no toca nada no es una función",
            ),
        );
    }
}

// ── OOS7014 · lo que lee está en la superficie ──────────────────────────────

fn lectura(pkg: &Package, d: &Loaded, out: &mut Vec<Diagnostic>) {
    let qn = d.qname().unwrap_or_default();
    let mut expone: Option<Vec<String>> = None;
    if let Some(nodo) = d.section("over")
        && let Some(r) = nodo.as_str()
    {
        match pkg.resolve_view(r, d) {
            // Lo que la vista expone: sus `fields`, o el contrato de una
            // vista SQL (v1alpha14 §4: `OOS7014` sigue valiendo sobre ella).
            Some(v) => expone = Some(crate::vistas::expone(v).into_keys().collect()),
            None => out.push(no_es_vista(d, nodo, r, "over")),
        }
    }
    for r in d.section("reads").map(|n| n.items()).unwrap_or(&[]) {
        if let Some(s) = r.as_str()
            && pkg.resolve_view(s, d).is_none()
        {
            out.push(no_es_vista(d, r, s, "reads"));
        }
    }
    // Una precondición mira `target.<campo>`, y `target` es una fila de `over`.
    let Some(campos) = expone else {
        return;
    };
    for p in d.section("preconditions").map(|n| n.items()).unwrap_or(&[]) {
        let Some((_, expr)) = p.get("expr") else {
            continue;
        };
        let Some(texto) = expr.as_str() else {
            continue;
        };
        for campo in crate::effect::referencias(texto) {
            if !campos.contains(&campo) {
                out.push(
                    Diagnostic::new(
                        Code::Oos7014,
                        &d.path,
                        format!("`{qn}` lee `target.{campo}`, y `over` no lo expone"),
                    )
                    .at(expr.pos())
                    .help(format!(
                        "`target` es una fila de la vista de `over`, y esa vista expone {}. Lo \
                         que no está en la superficie no existe para la función: añade el campo \
                         a la vista, o mira otro",
                        if campos.is_empty() {
                            "ningún campo".to_string()
                        } else {
                            campos
                                .iter()
                                .map(|c| format!("`{c}`"))
                                .collect::<Vec<_>>()
                                .join(", ")
                        }
                    )),
                );
            }
        }
    }
}

fn no_es_vista(d: &Loaded, nodo: &Node, referencia: &str, campo: &str) -> Diagnostic {
    Diagnostic::new(
        Code::Oos7014,
        &d.path,
        format!(
            "`{}` lee `{referencia}` por `{campo}`, y no es ninguna vista",
            d.qname().unwrap_or_default()
        ),
    )
    .at(nodo.pos())
    .help(
        "la superficie de lectura son VISTAS: lo que entra en el sandbox es lo que una \
         pregunta expone, con sus etiquetas, y así el sandbox es un conducto. Una función \
         no lee tablas ni entidades: lee preguntas",
    )
}

// ── La forma de una acción ──────────────────────────────────────────────────

fn forma_de_accion(pkg: &Package, a: &Loaded, out: &mut Vec<Diagnostic>) {
    let qn = a.qname().unwrap_or_default();
    let forma = |msg: String, ayuda: &str| {
        Diagnostic::new(Code::Oos1004, &a.path, msg)
            .at(a.root.pos())
            .help(ayuda)
    };
    let sets = a.section("sets");
    let call = a.section("call");
    match (sets.is_some(), call.is_some()) {
        (false, false) => out.push(forma(
            format!("`{qn}` no declara ni `sets` ni `call`"),
            "una acción tiene exactamente una de dos formas: declara lo que causa (`sets`) o \
             llama a una función (`call`)",
        )),
        (true, true) => out.push(forma(
            format!("`{qn}` declara `sets` y `call`"),
            "exactamente una forma: si hace falta computar, es una función y la acción la \
             llama; si no, la acción declara los valores y no hay función",
        )),
        _ => {}
    }

    // ── `sets`: valores fijos o de un parámetro, y ninguna tercera fuente ──
    let parametros: Vec<String> = a
        .section("input")
        .map(|i| i.entries())
        .unwrap_or(&[])
        .iter()
        .filter_map(|(k, _)| k.as_str().map(String::from))
        .collect();
    for s in sets.map(|n| n.items()).unwrap_or(&[]) {
        let destino = s
            .get("writes")
            .and_then(|(_, v)| v.as_str())
            .unwrap_or("<sin writes>");
        let to = s.get("to");
        let from = s.get("from").and_then(|(_, v)| v.as_str());
        match (to.is_some(), from) {
            (false, None) => out.push(
                Diagnostic::new(
                    Code::Oos1004,
                    &a.path,
                    format!("`{qn}` escribe `{destino}` sin decir con qué"),
                )
                .at(s.pos())
                .help("`to: <valor>` fijo, o `from: input.<parámetro>`. Una acción no computa"),
            ),
            (true, Some(_)) => out.push(
                Diagnostic::new(
                    Code::Oos1004,
                    &a.path,
                    format!("`{qn}` escribe `{destino}` con `to` y con `from`"),
                )
                .at(s.pos())
                .help("un valor viene de un sitio: fijo (`to`) o de un parámetro (`from`)"),
            ),
            (false, Some(f)) => {
                let param = f.strip_prefix("input.");
                match param {
                    Some(p) if parametros.iter().any(|x| x == p) => {}
                    Some(p) => out.push(
                        Diagnostic::new(
                            Code::Oos1004,
                            &a.path,
                            format!("`{qn}` escribe `{destino}` desde `input.{p}`, que no declara"),
                        )
                        .at(s.pos())
                        .help("el parámetro tiene que estar en `input`: es lo que la aplicación pide"),
                    ),
                    None => out.push(
                        Diagnostic::new(
                            Code::Oos1004,
                            &a.path,
                            format!("`{qn}` escribe `{destino}` desde `{f}`, que no es un parámetro"),
                        )
                        .at(s.pos())
                        .help("`from` es `input.<parámetro>` y nada más: una acción no lee otra cosa para escribir"),
                    ),
                }
            }
            (true, None) => {}
        }
    }

    // ── sin código no hay atestación ───────────────────────────────────────
    if sets.is_some() {
        for e in a.section("endorsements").map(|n| n.items()).unwrap_or(&[]) {
            if e.get("endorser").and_then(|(_, v)| v.as_str()) == Some("attested") {
                out.push(
                    Diagnostic::new(
                        Code::Oos1004,
                        &a.path,
                        format!("`{qn}` es una acción declarativa con un endoso `attested`"),
                    )
                    .at(e.pos())
                    .help(
                        "una atestación dice que un CÓDIGO es de fiar, y aquí no hay código: hay \
                         una tabla de valores. Lo que cierra la carencia de una acción es quien \
                         la aplica (`humanApproval`), o la función que llama",
                    ),
                );
            }
        }
    }

    // ── `call` resuelve a una función, y el contrato es el suyo ────────────
    if let Some(nodo) = call
        && let Some(nombre) = nodo.as_str()
    {
        let q = crate::link::cualificar(nombre, a);
        let funcion = pkg
            .of(Kind::Function)
            .find(|f| f.qname().as_deref() == Some(q.as_str()));
        let Some(f) = funcion else {
            let otro = pkg
                .docs
                .iter()
                .find(|x| x.qname().as_deref() == Some(q.as_str()))
                .map(|x| x.kind.as_str());
            out.push(
                Diagnostic::new(
                    Code::Oos2001,
                    &a.path,
                    match otro {
                        Some(k) => format!("`{q}` es un `{k}`, no una `Function`"),
                        None => format!("`{q}` no resuelve a ninguna `Function`"),
                    },
                )
                .at(nodo.pos())
                .help(
                    "una acción con `call` es la puerta de una función: la función tiene que \
                     estar, con sus efectos, y la acción pone los parámetros y quién puede",
                ),
            );
            return;
        };
        let exige: Vec<String> = f
            .section("input")
            .map(|i| i.entries())
            .unwrap_or(&[])
            .iter()
            .filter_map(|(k, v)| {
                let obligatorio = v
                    .get("required")
                    .and_then(|(_, r)| r.as_str())
                    .is_some_and(|r| r == "true");
                obligatorio.then(|| k.as_str().map(String::from)).flatten()
            })
            .collect();
        let faltan: Vec<&String> = exige.iter().filter(|p| !parametros.contains(p)).collect();
        if !faltan.is_empty() {
            out.push(
                Diagnostic::new(
                    Code::Oos1004,
                    &a.path,
                    format!(
                        "`{qn}` llama a `{q}` sin pedir {}",
                        faltan
                            .iter()
                            .map(|p| format!("`{p}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                )
                .at(nodo.pos())
                .help(
                    "el contrato es el de la función: lo que ella exige como `required` lo pide \
                     la acción en su `input`, o nadie podría invocarla desde aquí",
                ),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn doc(ruta: &str, kind: Kind, texto: &str) -> Loaded {
        Loaded {
            path: PathBuf::from(ruta),
            kind,
            root: crate::parse::parse(texto).expect("analiza"),
        }
    }

    fn paquete(docs: Vec<Loaded>) -> Package {
        Package {
            root: PathBuf::from("."),
            docs,
            cedar: Vec::new(),
            generated: Vec::new(),
            sobres: Vec::new(),
        }
    }

    const ENTIDAD: &str = "apiVersion: oos.dev/v1alpha1\nkind: Entity\nmetadata: { name: Cliente, namespace: ventas }\nspec:\n  nature: entity\n  primaryKey: [id]\n  backedBy: ventas.clientes\n  properties:\n    id: { type: String }\n    segmento: { type: String }\n";
    const VISTA: &str = "apiVersion: oos.dev/v1alpha8\nkind: View\nmetadata: { name: clientes, namespace: ventas }\nspec:\n  owner: team:v\n  from: { table: t }\n  fields: { id: id, segmento: segmento }\n";

    fn base() -> Vec<Loaded> {
        vec![
            doc("e.yaml", Kind::Entity, ENTIDAD),
            doc("v.yaml", Kind::View, VISTA),
        ]
    }

    fn codigos(pkg: &Package) -> Vec<Code> {
        let mut out = Vec::new();
        comprobar(pkg, &mut out);
        out.iter().map(|d| d.code).collect()
    }

    #[test]
    fn writes_a_una_propiedad_que_no_existe_es_oos2005_en_toda_version() {
        let f = "apiVersion: oos.dev/v1alpha9\nkind: Function\nmetadata: { name: f, namespace: ventas }\nspec:\n  runtime: wasm\n  entrypoint: x\n  effects:\n    - writes: ventas.Cliente.noExiste\n";
        let mut docs = base();
        docs.push(doc("f.yaml", Kind::Function, f));
        assert_eq!(codigos(&paquete(docs)), vec![Code::Oos2005]);
    }

    #[test]
    fn writes_a_una_entidad_que_no_existe_es_oos2005() {
        let f = "apiVersion: oos.dev/v1alpha10\nkind: Function\nmetadata: { name: f, namespace: ventas }\nspec:\n  runtime: wasm\n  entrypoint: x\n  effects:\n    - writes: ventas.Nadie.segmento\n";
        let mut docs = base();
        docs.push(doc("f.yaml", Kind::Function, f));
        assert_eq!(codigos(&paquete(docs)), vec![Code::Oos2005]);
    }

    #[test]
    fn una_funcion_que_no_toca_nada_es_oos1004_y_una_de_lectura_no() {
        let nada = "apiVersion: oos.dev/v1alpha10\nkind: Function\nmetadata: { name: f, namespace: ventas }\nspec:\n  runtime: wasm\n  entrypoint: x\n";
        let mut docs = base();
        docs.push(doc("f.yaml", Kind::Function, nada));
        assert_eq!(codigos(&paquete(docs)), vec![Code::Oos1004]);

        let lee = "apiVersion: oos.dev/v1alpha10\nkind: Function\nmetadata: { name: f, namespace: ventas }\nspec:\n  runtime: wasm\n  entrypoint: x\n  over: ventas.clientes\n  reads: [clientes]\n  output:\n    n: { type: Integer }\n  preconditions:\n    - id: a\n      expr: 'target.segmento == \"pyme\"'\n";
        let mut docs = base();
        docs.push(doc("f.yaml", Kind::Function, lee));
        assert!(codigos(&paquete(docs)).is_empty());
    }

    #[test]
    fn lo_que_no_esta_en_la_superficie_es_oos7014() {
        let f = "apiVersion: oos.dev/v1alpha10\nkind: Function\nmetadata: { name: f, namespace: ventas }\nspec:\n  runtime: wasm\n  entrypoint: x\n  over: ventas.clientes\n  reads: [ventas.nadie]\n  preconditions:\n    - id: a\n      expr: 'target.nombre != \"\"'\n";
        let mut docs = base();
        docs.push(doc("f.yaml", Kind::Function, f));
        assert_eq!(codigos(&paquete(docs)), vec![Code::Oos7014, Code::Oos7014]);
    }

    #[test]
    fn una_accion_tiene_exactamente_una_forma_y_sin_codigo_no_se_atesta() {
        let ok = "apiVersion: oos.dev/v1alpha10\nkind: Action\nmetadata: { name: a, namespace: ventas }\nspec:\n  over: ventas.clientes\n  input:\n    s: { type: String }\n  sets:\n    - writes: ventas.Cliente.segmento\n      from: input.s\n  endorsements:\n    - endorser: humanApproval\n";
        let mut docs = base();
        docs.push(doc("a.yaml", Kind::Action, ok));
        assert!(codigos(&paquete(docs)).is_empty());

        let mal = "apiVersion: oos.dev/v1alpha10\nkind: Action\nmetadata: { name: a, namespace: ventas }\nspec:\n  over: ventas.clientes\n  sets:\n    - writes: ventas.Cliente.segmento\n      from: input.s\n  call: ventas.f\n  endorsements:\n    - endorser: attested\n";
        let mut docs = base();
        docs.push(doc("a.yaml", Kind::Action, mal));
        let c = codigos(&paquete(docs));
        // sets y call · from sin parámetro · attested sin código · call sin función
        assert_eq!(
            c,
            vec![Code::Oos1004, Code::Oos1004, Code::Oos1004, Code::Oos2001],
            "{c:?}"
        );
    }

    #[test]
    fn una_accion_que_llama_pide_lo_que_la_funcion_exige() {
        let f = "apiVersion: oos.dev/v1alpha10\nkind: Function\nmetadata: { name: f, namespace: ventas }\nspec:\n  runtime: wasm\n  entrypoint: x\n  over: ventas.clientes\n  input:\n    cuando: { type: Timestamp, required: true }\n    nota: { type: String }\n";
        let a = "apiVersion: oos.dev/v1alpha10\nkind: Action\nmetadata: { name: a, namespace: ventas }\nspec:\n  over: ventas.clientes\n  call: f\n";
        let mut docs = base();
        docs.push(doc("f.yaml", Kind::Function, f));
        docs.push(doc("a.yaml", Kind::Action, a));
        let out = {
            let mut o = Vec::new();
            comprobar(&paquete(docs), &mut o);
            o
        };
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].code, Code::Oos1004);
        assert!(out[0].message.contains("sin pedir `cuando`"));
    }
}
