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
use crate::document::Kind;
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
    tipos_de_conceptos(pkg, &mut out);
    // OOS3006 vive en su propio modulo porque necesita el paquete entero: hay
    // que leer la `primaryKey` de OTRA entidad. Es de esta familia igualmente.
    crate::enlace_compuesto::comprobar(pkg, &mut out);
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

/// El tipo de un **concepto**, que es el que después hereda todo el que lo
/// referencie. Sin esto, un `type` mal escrito en un `Property` se propagaría
/// en silencio a las quince propiedades que lo mapean.
fn tipos_de_conceptos(pkg: &Package, out: &mut Vec<Diagnostic>) {
    for d in pkg.docs.iter().filter(|d| d.kind == Kind::Property) {
        let Some(t) = d.section("type") else { continue };
        let Some(s) = t.as_str() else { continue };
        if parse_type(s).is_err() {
            out.push(
                Diagnostic::new(
                    Code::Oos3001,
                    &d.path,
                    format!("`{s}` no es un tipo de OOS"),
                )
                .at(t.pos()),
            );
        }
    }
}

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
    // mismo destino, y eso solo lo sostiene una clave declarada. Ahora que `via`
    // es una secuencia la condición se puede decir entera: **`via` tiene que
    // CONTENER una clave**, no estar contenida en ella. Un superconjunto de una
    // clave sigue siendo único; un subconjunto no lo es, y la redacción anterior
    // —«que `via` esté en `primaryKey`»— aceptaba justo eso.
    let lista = |n: &Node| -> Vec<String> {
        n.items()
            .iter()
            .filter_map(|i| i.as_str().map(String::from))
            .collect()
    };
    let mut claves: Vec<Vec<String>> = Vec::new();
    if let Some(pk) = e.section("primaryKey") {
        let k = lista(pk);
        if !k.is_empty() {
            claves.push(k);
        }
    }
    if let Some(uk) = e.section("uniqueKeys") {
        for c in uk.items() {
            let k = lista(c);
            if !k.is_empty() {
                claves.push(k);
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
        let via = lista(vianode);
        if !claves.iter().any(|k| k.iter().all(|p| via.contains(p))) {
            out.push(
                Diagnostic::new(
                    Code::Oos3005,
                    &e.path,
                    format!(
                        "`{qn}.{rn}` declara `one_to_one` a través de [{}], que no es única",
                        via.join(", ")
                    ),
                )
                .at(vianode.pos())
                .help(if claves.is_empty() {
                    "`one_to_one` afirma que ninguna otra instancia apunta al mismo \
                     destino, y nada en las claves declaradas lo sostiene. Declara esas \
                     propiedades en `uniqueKeys`, o usa `many_to_one`"
                        .to_string()
                } else {
                    format!(
                        "`via` tiene que CONTENER una clave entera, no una parte de \
                         ella. Sostienen `one_to_one`: {}. De la cardinalidad dependen \
                         la estructura del indice y la deteccion de cambios rompedores",
                        claves
                            .iter()
                            .map(|k| format!("[{}]", k.join(", ")))
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

// ── Guardián ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod acuerdo {
    use std::path::Path;

    fn leer(rel: &str) -> String {
        let p = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../vendor/oos")
            .join(rel);
        std::fs::read_to_string(&p)
            .unwrap_or_else(|e| panic!("no se pudo leer {}: {e}", p.display()))
    }

    /// Los nombres entre acentos graves de `02-entity.md` §3.1 — el texto
    /// normativo, que es quien fija el conjunto.
    fn de_la_prosa() -> Vec<String> {
        let t = leer("spec/v1alpha1/02-entity.md");
        let i = t.find("### 3.1").expect("02-entity.md ya no tiene §3.1");
        let resto = &t[i..];
        let fin = resto[6..].find("###").map(|j| j + 6).unwrap_or(resto.len());
        let seccion = &resto[..fin];
        let mut fuera = Vec::new();
        let mut it = seccion.split('`');
        it.next();
        while let Some(dentro) = it.next() {
            fuera.push(dentro.to_string());
            if it.next().is_none() {
                break;
            }
        }
        fuera
    }

    /// El `enum` de `scalarType`, leído como texto: `ore-core` no lleva
    /// analizador de JSON, y esta comprobación no es motivo para meter uno.
    fn del_esquema() -> Vec<String> {
        let t = leer("schemas/v1alpha1/type/basic.schema.json");
        let i = t
            .find("\"scalarType\"")
            .expect("el esquema ya no declara scalarType");
        let j = t[i..].find("\"enum\"").expect("scalarType sin enum") + i;
        let a = t[j..].find('[').expect("enum sin abrir") + j;
        let b = t[a..].find(']').expect("enum sin cerrar") + a;
        t[a..b]
            .split('"')
            .skip(1)
            .step_by(2)
            .map(str::to_string)
            .collect()
    }

    /// Tres declaraciones del mismo conjunto tienen que decir lo mismo.
    ///
    /// No lo decían. Hasta que v1alpha5 necesitó una tabla de tipos exacta, el
    /// esquema publicaba siete nombres en minúscula que no usaba ni un documento
    /// del repositorio, mientras la prosa y este motor usaban diez capitalizados
    /// — y las 375 propiedades escritas `String` validaban por la puerta de
    /// escape de `qualifiedName`, como «tipo importado» llamado `String`.
    ///
    /// Un `$def` con 375 usuarios y ninguno que lo usara. Esto lo vuelve
    /// imposible de repetir.
    #[test]
    fn el_vocabulario_de_escalares_es_uno_solo() {
        let prosa = de_la_prosa();
        let esquema = del_esquema();
        let motor: Vec<String> = super::ESCALARES.iter().map(|s| s.to_string()).collect();

        assert!(
            prosa.len() >= 8,
            "§3.1 de 02-entity.md solo dio {} nombres: {prosa:?}.              Si la sección cambió de forma, este guardián está leyendo otra cosa.",
            prosa.len()
        );
        assert_eq!(
            prosa, motor,
            "la prosa normativa y el motor discrepan sobre los escalares"
        );
        assert_eq!(
            prosa, esquema,
            "la prosa normativa y `basic.schema.json` discrepan sobre los escalares"
        );
    }

    /// Y la rama de tipo importado tiene que exigir un punto: sin él se traga
    /// cualquier identificador y el `enum` de arriba se queda sin trabajo —
    /// `Blob` pasaría como «tipo importado» en vez de fallar con OOS3001.
    #[test]
    fn la_rama_de_tipo_importado_exige_un_punto() {
        let t = leer("schemas/v1alpha1/type/basic.schema.json");
        let i = t
            .find("\"scalarType\"")
            .expect("el esquema ya no declara scalarType");
        let seccion = &t[i..];
        assert!(
            seccion.contains("iso.CountryAlpha2"),
            "la rama de tipo importado ya no está donde este guardián la busca"
        );
        assert!(
            !seccion.contains("$defs/qualifiedName"),
            "la rama de tipo importado volvió a `qualifiedName`, que acepta un              identificador suelto y deja sin efecto el conjunto cerrado de escalares"
        );
    }
}
