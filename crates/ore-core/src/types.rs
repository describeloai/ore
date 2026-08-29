//! Sistema de tipos — la familia `OOS3xxx`.
//!
//! Corre **después** del enlazado y solo si este quedó limpio: no se puede
//! comprobar el tipo de una referencia que no resuelve. Es la misma disciplina
//! de fases que impide enlazar un paquete que no analiza.
//!
//! Lo que distingue a esta familia es `OOS3004`. Los demás códigos miran una
//! propiedad; ese mira **tres a la vez** siguiendo `derivedFrom`, y es la primera
//! vez que el compilador razona sobre el grafo de derivación — exactamente la
//! maquinaria que después reutiliza la propagación de etiquetas de `OOS4xxx`.

use crate::code::Code;
use crate::diag::Diagnostic;
use crate::link::{Loaded, Package};
use crate::parse::Node;
use std::collections::BTreeMap;

/// Los escalares del conjunto cerrado. Los nombres se alinean con el enum de
/// Apache Ossie aunque no lo perfilemos: no cuesta nada y convierte la emisión
/// en un mapeo sin renombrados.
const ESCALARES: &[&str] = &[
    "String",
    "Integer",
    "Decimal",
    "Float",
    "Boolean",
    "Date",
    "Time",
    "DateTime",
    "DateTimeTz",
    // `Opaque` es la salida prevista para lo que OOS no modela: un blob existe
    // en la fuente, se puede etiquetar y gobernar, y el sistema de tipos no
    // necesita saber qué hay dentro.
    "Opaque",
];

/// Un tipo ya analizado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
    Scalar(String),
    /// `Money<EUR, 2>` · `Quantity<km, 1>`. La unidad es parte del tipo, no una
    /// anotación: sin ella, sumar euros y dólares no falla — solo da cifras
    /// incorrectas.
    Parametric {
        ctor: String,
        unit: String,
        precision: u32,
    },
    List(String),
    /// `iso.CountryAlpha2`. Su resolución es trabajo de dependencias.
    Imported(String),
}

impl Type {
    /// La unidad, si el tipo la tiene. Es lo único que `OOS3004` necesita.
    pub fn unit(&self) -> Option<&str> {
        match self {
            Type::Parametric { unit, .. } => Some(unit),
            _ => None,
        }
    }
}

/// Por qué un tipo no es válido.
#[derive(Debug)]
pub enum TypeError {
    Desconocido,
    /// Un paramétrico al que le falta la unidad o la precisión.
    Incompleto(String),
}

pub fn parse_type(s: &str) -> Result<Type, TypeError> {
    if let Some(inner) = s.strip_prefix("list<").and_then(|r| r.strip_suffix('>')) {
        return if ESCALARES.contains(&inner) {
            Ok(Type::List(inner.to_string()))
        } else {
            Err(TypeError::Desconocido)
        };
    }

    if let Some((ctor, resto)) = s.split_once('<') {
        if !matches!(ctor, "Money" | "Quantity") {
            return Err(TypeError::Desconocido);
        }
        let Some(args) = resto.strip_suffix('>') else {
            return Err(TypeError::Incompleto(ctor.to_string()));
        };
        let partes: Vec<&str> = args.split(',').map(str::trim).collect();
        if partes.len() != 2 || partes[0].is_empty() {
            return Err(TypeError::Incompleto(ctor.to_string()));
        }
        let Ok(precision) = partes[1].parse::<u32>() else {
            return Err(TypeError::Incompleto(ctor.to_string()));
        };
        return Ok(Type::Parametric {
            ctor: ctor.to_string(),
            unit: partes[0].to_string(),
            precision,
        });
    }

    if ESCALARES.contains(&s) {
        return Ok(Type::Scalar(s.to_string()));
    }
    // Un nombre cualificado es un tipo importado de un paquete de tipos.
    if s.contains('.') && s.split('.').all(|p| !p.is_empty()) {
        return Ok(Type::Imported(s.to_string()));
    }
    Err(TypeError::Desconocido)
}

pub fn check(pkg: &Package) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for e in pkg.entities() {
        tipos_declarados(e, &mut out);
        temporalidad(e, &mut out);
        derivaciones(pkg, e, &mut out);
        cardinalidades(e, &mut out);
    }
    out
}

/// Índice `propiedad -> (nodo del tipo, tipo analizado)` de una entidad.
fn tipos_de(e: &Loaded) -> BTreeMap<String, (Node, Option<Type>)> {
    let Some(ps) = e.section("properties") else {
        return BTreeMap::new();
    };
    ps.entries()
        .iter()
        .filter_map(|(k, v)| {
            let nombre = k.as_str()?.to_string();
            let (_, t) = v.get("type")?;
            let parsed = t.as_str().and_then(|s| parse_type(s).ok());
            Some((nombre, (t.clone(), parsed)))
        })
        .collect()
}

// ── OOS3001 · OOS3002 ───────────────────────────────────────────────────────

fn tipos_declarados(e: &Loaded, out: &mut Vec<Diagnostic>) {
    let Some(ps) = e.section("properties") else {
        return;
    };
    for (k, v) in ps.entries() {
        let Some((_, t)) = v.get("type") else {
            continue;
        };
        let Some(s) = t.as_str() else { continue };
        match parse_type(s) {
            Ok(_) => {}
            Err(TypeError::Desconocido) => out.push(
                Diagnostic::new(
                    Code::Oos3001,
                    &e.path,
                    format!("`{s}` no es un tipo de OOS v1alpha1"),
                )
                .at(t.pos())
                .help(format!(
                    "escalares: {}. Para lo que OOS no modela —un blob binario, una \
                     estructura opaca— usa `Opaque`: existe en la fuente, se puede etiquetar \
                     y gobernar, y el sistema de tipos no necesita saber qué hay dentro",
                    ESCALARES.join(" · ")
                )),
            ),
            Err(TypeError::Incompleto(ctor)) => out.push(
                Diagnostic::new(
                    Code::Oos3002,
                    &e.path,
                    format!("`{s}` está incompleto: `{ctor}` necesita unidad y precisión"),
                )
                .at(t.pos())
                .help(format!(
                    "escríbelo como `{ctor}<EUR, 2>`. Ni Ossie ni ODCS pueden expresar \
                     «euros con dos decimales», y por eso el tipo lleva los dos: es un error \
                     silencioso — no falla, solo produce cifras incorrectas",
                )),
            ),
        }
        let _ = k;
    }
}

// ── OOS3003 ─────────────────────────────────────────────────────────────────

fn temporalidad(e: &Loaded, out: &mut Vec<Diagnostic>) {
    let Some(t) = e.section("temporal") else {
        return;
    };
    if t.get("validTime").is_none() {
        out.push(
            Diagnostic::new(Code::Oos3003, &e.path, "`temporal` no declara `validTime`")
                .at(t.pos())
                .help(
                    "`validTime` es cuándo fue cierto EN EL MUNDO, y es el obligatorio: sin él \
                     un salario es un número en lugar de una función del tiempo. \
                     `transactionTime` —cuándo lo supo el sistema de origen— es opcional, \
                     porque «qué sabía el agente el martes» lo responden el commit del bundle \
                     y la marca de agua del índice",
                ),
        );
    }
}

// ── OOS3004 ─────────────────────────────────────────────────────────────────

fn derivaciones(pkg: &Package, e: &Loaded, out: &mut Vec<Diagnostic>) {
    let Some(ps) = e.section("properties") else {
        return;
    };
    let propios = tipos_de(e);
    let qn = e.qname().unwrap_or_default();

    for (k, v) in ps.entries() {
        let Some(nombre) = k.as_str() else { continue };
        let Some((_, from)) = v.get("derivedFrom") else {
            continue;
        };

        // Unidades de los orígenes, cada una con dónde se declaró.
        let mut unidades: Vec<(String, String)> = Vec::new();
        for r in from.items() {
            let Some(qref) = r.as_str() else { continue };
            let Some((ent, prop)) = qref.rsplit_once('.') else {
                continue;
            };
            let tabla = if ent == qn {
                propios.clone()
            } else if let Some(otra) = pkg.entity(ent) {
                tipos_de(otra)
            } else {
                continue;
            };
            if let Some((_, Some(t))) = tabla.get(prop)
                && let Some(u) = t.unit()
            {
                unidades.push((u.to_string(), qref.to_string()));
            }
        }

        // El resultado cuenta como una unidad más: derivar euros de euros y
        // declararlo en dólares es el mismo error.
        if let Some((_, Some(t))) = propios.get(nombre)
            && let Some(u) = t.unit()
        {
            unidades.push((u.to_string(), format!("{qn}.{nombre}")));
        }

        let distintas: Vec<&(String, String)> = {
            let mut v: Vec<&(String, String)> = Vec::new();
            for u in &unidades {
                if !v.iter().any(|(x, _)| x == &u.0) {
                    v.push(u);
                }
            }
            v
        };

        if distintas.len() > 1 {
            out.push(
                Diagnostic::new(
                    Code::Oos3004,
                    &e.path,
                    format!(
                        "`{qn}.{nombre}` mezcla unidades incompatibles: {}",
                        distintas
                            .iter()
                            .map(|(u, d)| format!("{u} en `{d}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                )
                .at(from.pos())
                .help(
                    "comprobar esto exige comparar los tipos de tres propiedades entre sí, \
                     así que ningún esquema JSON lo alcanza — y con `datatype: Decimal` en \
                     ambos lados esto sumaría sin protestar. El compilador no ejecuta la \
                     expresión: comprueba `derivedFrom`, la misma información que usa para \
                     propagar etiquetas",
                ),
            );
        }
    }
}

// ── OOS3005 ─────────────────────────────────────────────────────────────────

fn cardinalidades(e: &Loaded, out: &mut Vec<Diagnostic>) {
    let Some(rels) = e.section("relations") else {
        return;
    };
    let qn = e.qname().unwrap_or_default();

    // Una relación `one_to_one` afirma que ninguna otra instancia apunta al
    // mismo destino. Solo una clave de UNA propiedad sostiene esa afirmación:
    // con `[a, b]` compuesta, `a` por sí sola no es única.
    let mut unicas: Vec<String> = Vec::new();
    if let Some(pk) = e.section("primaryKey")
        && pk.items().len() == 1
        && let Some(s) = pk.items()[0].as_str()
    {
        unicas.push(s.to_string());
    }
    if let Some(uk) = e.section("uniqueKeys") {
        for clave in uk.items() {
            if clave.items().len() == 1
                && let Some(s) = clave.items()[0].as_str()
            {
                unicas.push(s.to_string());
            }
        }
    }

    for (rk, rv) in rels.entries() {
        let Some(rn) = rk.as_str() else { continue };
        let card = rv
            .get("cardinality")
            .and_then(|(_, v)| v.as_str())
            .unwrap_or("");
        if card != "one_to_one" {
            continue;
        }
        let Some((_, vianode)) = rv.get("via") else {
            continue;
        };
        let via = vianode.as_str().unwrap_or("");
        if !unicas.contains(&via.to_string()) {
            out.push(
                Diagnostic::new(
                    Code::Oos3005,
                    &e.path,
                    format!(
                        "`{qn}.{rn}` declara `one_to_one` a través de `{via}`, que no es única"
                    ),
                )
                .at(vianode.pos())
                .help(if unicas.is_empty() {
                    "`one_to_one` afirma que ninguna otra instancia apunta al mismo destino, y \
                     nada en las claves declaradas lo sostiene. Declara `{via}` en `uniqueKeys`, \
                     o usa `many_to_one`"
                        .to_string()
                } else {
                    format!(
                        "sostienen `one_to_one`: {}. Con `{via}` la cardinalidad afirma algo \
                         que las claves no respaldan, y de ella dependen la estructura del \
                         índice y la detección de cambios rompedores",
                        unicas
                            .iter()
                            .map(|u| format!("`{u}`"))
                            .collect::<Vec<_>>()
                            .join(" · ")
                    )
                }),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escalares_y_paramtericos() {
        assert!(matches!(parse_type("String"), Ok(Type::Scalar(_))));
        assert!(matches!(parse_type("Opaque"), Ok(Type::Scalar(_))));
        assert!(matches!(parse_type("list<String>"), Ok(Type::List(_))));
        assert!(matches!(
            parse_type("iso.CountryAlpha2"),
            Ok(Type::Imported(_))
        ));
        assert_eq!(parse_type("Money<EUR, 2>").unwrap().unit(), Some("EUR"));
        assert_eq!(parse_type("Quantity<km,1>").unwrap().unit(), Some("km"));
    }

    #[test]
    fn tipo_desconocido() {
        assert!(matches!(parse_type("Blob"), Err(TypeError::Desconocido)));
        assert!(matches!(
            parse_type("list<Blob>"),
            Err(TypeError::Desconocido)
        ));
    }

    /// La divisa sin precisión y la precisión sin divisa son el mismo error.
    #[test]
    fn paramtrico_incompleto() {
        assert!(matches!(
            parse_type("Money<EUR>"),
            Err(TypeError::Incompleto(_))
        ));
        assert!(matches!(
            parse_type("Money<, 2>"),
            Err(TypeError::Incompleto(_))
        ));
        assert!(matches!(
            parse_type("Money<EUR, dos>"),
            Err(TypeError::Incompleto(_))
        ));
    }
}
