//! `ore export --format graphql` — la cuarta superficie.
//!
//! Normativo: [`spec/v1alpha5/01-emision-graphql.md`]. Este módulo es la
//! implementación de referencia de ese documento, y su tesis cabe en una línea:
//!
//! > **La clasificación no se emite. Se ejecuta al emitir.**
//!
//! Un campo cuya etiqueta efectiva excede el techo de `contextSurface` no sale
//! prohibido ni marcado: **sale ausente**. El consumidor no puede pedir lo que
//! el contrato no declara, así que en el momento de la petición no queda nada
//! que aplicar — ya se aplicó al compilar. Es `G2` mirado desde fuera.
//!
//! # Por qué el techo se pide prestado y no se recalcula
//!
//! `flow::clearances` es el mismo techo que usa el chequeo de flujo. Recalcularlo
//! aquí abriría la puerta a que `ore validate` dijera que un dato no puede salir
//! por `contextSurface` mientras el esquema lo declara — dos verdades sobre el
//! mismo hecho, que es el modo de fallo que este proyecto persigue en todas
//! partes.
//!
//! # Lo que este emisor NO hace
//!
//! No mira quién pregunta. `contextSurface` es un conducto, y un conducto es un
//! techo del paquete: la emisión es una función pura del bundle. Lo que dependa
//! del principal se decide en runtime, que es donde ya vive Cedar
//! ([`00-scope`](../../vendor/oos/spec/v1alpha5/00-scope.md) §2.2).

use crate::flow::{self, Lattice};
use crate::link::{Loaded, Package};
use crate::types::{Type, parse_type};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

const CONDUCTO: &str = "contextSurface";

/// Un tipo que sobrevivió al filtro, listo para escribirse.
struct Tipo {
    nombre: String,
    /// `(campo, tipo GraphQL)`, en orden canónico — `properties` es un mapa.
    campos: Vec<(String, String)>,
    /// `(campo, tipo destino)` — las aristas cuyo destino también sobrevivió.
    aristas: Vec<(String, String)>,
    /// Una entrada por `primaryKey` y otra por cada `uniqueKeys`.
    claves: Vec<Vec<String>>,
    /// El nombre del campo de la raíz de consulta: `employee` y `employees`.
    raiz: String,
    interfaces: Vec<String>,
}

/// Una `Function` que sobrevivió al filtro, lista para escribirse.
struct Mutacion {
    nombre: String,
    argumentos: Vec<(String, String)>,
    retorno: String,
    /// El tipo de resultado que hay que declarar, si lo hay.
    resultado: Option<(String, Vec<(String, String)>)>,
    /// Cuántas firmas distintas exige, si exige más de una. Va a la
    /// documentación del campo: es donde un cliente la lee **sin ejecutarlo**.
    quorum: Option<u32>,
}

/// El tipo que devuelve una mutación cuyo endoso exige una firma humana.
///
/// No es política metida en el contrato: es aritmética del tipo de retorno.
/// `01-efectos` §3.2 fija que `humanApproval` es un endosante **dinámico** —el
/// compilador verifica la declaración, el motor verifica el acto—, así que en el
/// instante de la llamada **el acto no ha ocurrido**. No hay resultado que
/// devolver, y tipar la respuesta como si lo hubiera sería mentir.
const APROBACION: &str = "ApprovalRequired";

/// Emite el SDL del paquete, o dice por qué no puede.
///
/// El orden de los pasos es el de §4 y no es intercambiable: podar lo que quedó
/// vacío tiene que ir **después** de podar por clasificación, porque un tipo se
/// queda sin campos justo por eso.
pub fn emit(pkg: &Package) -> Result<String, String> {
    let lat = flow::lattices(pkg);
    let techo = techo(pkg, &lat)?;
    let efectivas = flow::efectivas(pkg, &lat);

    // ── 1 y 2 · descartar por madurez y por clasificación ───────────────────
    let mut tipos: BTreeMap<String, Tipo> = BTreeMap::new();
    for e in pkg.entities() {
        let Some(qn) = e.qname() else { continue };
        let nombre = corto(&qn);
        let props = propiedades_visibles(e, &qn, &efectivas, &techo, &lat)?;
        // 4 · un tipo sin campos no se emite
        if props.is_empty() {
            continue;
        }
        let claves = claves(e);
        // Una propiedad puede ser clave y estar gobernada por encima del techo.
        // No se emite media clave: un `@key` incompleto es una identidad falsa.
        for k in &claves {
            if let Some(f) = k.iter().find(|f| !props.iter().any(|(p, _)| p == *f)) {
                return Err(format!(
                    "`{qn}` declara una clave sobre `{f}`, que el conducto \
                     `{CONDUCTO}` no admite.\n  Emitir la clave sin el campo daría un \
                     esquema inválido, y omitirla afirmaría otra identidad."
                ));
            }
        }
        let campos = props
            .iter()
            .map(|(p, t)| {
                let obligatorio = claves.first().is_some_and(|pk| pk.contains(p));
                (
                    p.clone(),
                    if obligatorio { "ID!".into() } else { t.clone() },
                )
            })
            .collect();
        tipos.insert(
            qn.clone(),
            Tipo {
                nombre: nombre.clone(),
                campos,
                aristas: Vec::new(),
                claves,
                raiz: minuscula_inicial(&nombre),
                interfaces: implementa(e),
            },
        );
    }

    if tipos.is_empty() {
        return Err(format!(
            "el conducto `{CONDUCTO}` no admite una sola propiedad del paquete.\n  \
             Un SDL sin tipos y sin raíz de consulta no es un documento válido de \
             GraphQL: no hay esquema que emitir."
        ));
    }

    // ── sin `Binding` no hay resolver ───────────────────────────────────────
    let huerfanas: Vec<String> = crate::normalize::sin_binding(pkg)
        .into_iter()
        .filter(|q| tipos.contains_key(q))
        .collect();
    if !huerfanas.is_empty() {
        return Err(format!(
            "sin `Binding`, y por tanto sin resolver: {}.\n  Un campo en un SDL es la \
             promesa de que preguntar por él devuelve algo.",
            huerfanas.join(", ")
        ));
    }

    // ── 4 · las aristas hacia un tipo que no se emitió tampoco se emiten ────
    //
    // No es limpieza. Un campo `patient: Diagnosis` revela que existe un tipo
    // `Diagnosis` y que un pedido se relaciona con uno — ninguna de las dos es
    // un dato, y las dos juntas son la fuga por topología de `DESIGN` §4.1.
    let vivos: BTreeSet<String> = tipos.keys().cloned().collect();
    for e in pkg.entities() {
        let Some(qn) = e.qname() else { continue };
        if !vivos.contains(&qn) {
            continue;
        }
        let aristas = relaciones(pkg, e, &vivos);
        if let Some(t) = tipos.get_mut(&qn) {
            t.aristas = aristas;
        }
    }

    let mutaciones = mutaciones(pkg, &tipos);

    Ok(escribir(&tipos, &mutaciones))
}

// ── Las mutaciones ──────────────────────────────────────────────────────────

/// Una `Function` emite una `Mutation` **si y solo si** cada propiedad que
/// escribe está en el contrato. Si una sola quedó fuera por el conducto, la
/// mutación no se emite: publicar una escritura sobre un campo que el consumidor
/// no puede leer le pediría que confíe en un efecto que no puede comprobar.
fn mutaciones(pkg: &Package, tipos: &BTreeMap<String, Tipo>) -> BTreeMap<String, Mutacion> {
    let mut out = BTreeMap::new();
    for f in pkg
        .docs
        .iter()
        .filter(|d| d.kind == crate::document::Kind::Function)
    {
        let Some(qn) = f.qname() else { continue };
        if !escribe_solo_lo_servido(f, tipos) {
            continue;
        }
        let nombre = corto(&qn);
        let argumentos = parametros(f, "input");
        let salida = parametros(f, "output");

        // El endoso decide el tipo, y gana sobre `output`. Un `when:` cuenta
        // igual que un endoso incondicional: la regla de integridad pregunta si
        // BASTA para escribir y una condición no es una garantía; el contrato
        // pregunta qué recibe QUIEN LLAMA, y una condición puede activarse.
        // Dos preguntas distintas sobre el mismo campo, dos respuestas.
        let firma = firma_humana(f);
        let (retorno, resultado) = if firma.is_some() {
            (format!("{APROBACION}!"), None)
        } else if salida.is_empty() {
            // `effects` es obligatorio, así que siempre hay un hecho que
            // devolver aunque no haya un valor declarado.
            ("Boolean!".to_string(), None)
        } else {
            let tipo = format!("{}Result", mayuscula_inicial(&nombre));
            (format!("{tipo}!"), Some((tipo, salida)))
        };

        out.insert(
            qn,
            Mutacion {
                nombre,
                argumentos,
                retorno,
                resultado,
                quorum: firma.flatten(),
            },
        );
    }
    out
}

fn escribe_solo_lo_servido(f: &Loaded, tipos: &BTreeMap<String, Tipo>) -> bool {
    let efectos = f.section("effects").map(|n| n.items()).unwrap_or(&[]);
    if efectos.is_empty() {
        return false;
    }
    efectos.iter().all(|e| {
        let Some(destino) = e.get("writes").and_then(|(_, v)| v.as_str()) else {
            return false;
        };
        let Some((entidad, propiedad)) = destino.rsplit_once('.') else {
            return false;
        };
        tipos
            .get(entidad)
            .is_some_and(|t| t.campos.iter().any(|(p, _)| p == propiedad))
    })
}

/// El endoso de firma humana, si lo hay, con su quórum. El `when:` no se mira:
/// ver arriba.
///
/// `Some(None)` es una firma; `Some(Some(n))`, `n` firmas distintas. Que el
/// quórum viva dentro del `Option` y no al lado no es estilo: **sin firma no hay
/// quórum**, y dos campos independientes admitirían escribir esa combinación.
fn firma_humana(f: &Loaded) -> Option<Option<u32>> {
    f.section("endorsements")
        .map(|n| n.items())
        .unwrap_or(&[])
        .iter()
        .find(|e| e.get("endorser").and_then(|(_, v)| v.as_str()) == Some("humanApproval"))
        .map(|e| {
            e.get("quorum")
                .and_then(|(_, v)| v.as_str())
                .and_then(|s| s.parse::<u32>().ok())
                .filter(|n| *n > 1)
        })
}

/// `input` y `output` son mapas de `nombre -> { type, required }`.
fn parametros(f: &Loaded, seccion: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = f
        .section(seccion)
        .map(|n| n.entries())
        .unwrap_or(&[])
        .iter()
        .filter_map(|(k, v)| {
            let nombre = k.as_str()?;
            let crudo = v.get("type").and_then(|(_, t)| t.as_str())?;
            let tipo = grafo(crudo).ok()?;
            let obligatorio = v
                .get("required")
                .and_then(|(_, r)| r.as_str())
                .is_some_and(|r| r == "true");
            Some((
                nombre.to_string(),
                if obligatorio {
                    format!("{tipo}!")
                } else {
                    tipo
                },
            ))
        })
        .collect();
    out.sort(); // un mapa, como `properties`
    out
}

// ── El techo ────────────────────────────────────────────────────────────────

/// El techo de `contextSurface`, por retículo.
///
/// Un conducto no listado tiene autorización ⊥ y no admite nada: denegación por
/// defecto (P4). Aquí eso se convierte en un fallo explícito en vez de un
/// esquema vacío — emitir un fichero que ningún motor puede cargar no es servir
/// menos, es entregar algo roto con aspecto de artefacto.
fn techo(
    pkg: &Package,
    lat: &BTreeMap<String, Lattice>,
) -> Result<BTreeMap<String, String>, String> {
    let cl = flow::clearances(pkg, lat);
    match cl.get(CONDUCTO) {
        Some(l) => Ok(l
            .iter()
            .map(|(ret, (nivel, _))| (ret.clone(), nivel.clone()))
            .collect()),
        None => Err(format!(
            "ningún `ConduitPolicy` autoriza `{CONDUCTO}`.\n  Un conducto no listado \
             tiene autorización ⊥: omitirlo no es dejarlo abierto, es cerrarlo (P4)."
        )),
    }
}

/// ¿Cabe esta propiedad bajo el techo, en **todos** los retículos que el techo
/// nombra? Un retículo sin etiqueta efectiva se trata como ⊥ y pasa.
fn cabe(
    etiquetas: Option<&BTreeMap<String, String>>,
    techo: &BTreeMap<String, String>,
    lat: &BTreeMap<String, Lattice>,
) -> bool {
    for (reticulo, tope) in techo {
        let Some(l) = lat.get(reticulo) else { continue };
        let Some(nivel) = etiquetas.and_then(|e| e.get(reticulo)) else {
            continue;
        };
        match (l.index(nivel), l.index(tope)) {
            (Some(i), Some(j)) if i <= j => {}
            // Una etiqueta que el retículo no reconoce no se deja pasar: sería
            // convertir un error de escritura en una propiedad servida.
            _ => return false,
        }
    }
    true
}

// ── La entidad ──────────────────────────────────────────────────────────────

fn propiedades_visibles(
    e: &Loaded,
    qn: &str,
    efectivas: &BTreeMap<String, BTreeMap<String, String>>,
    techo: &BTreeMap<String, String>,
    lat: &BTreeMap<String, Lattice>,
) -> Result<Vec<(String, String)>, String> {
    let mut out = Vec::new();
    let Some(props) = e.section("properties") else {
        return Ok(out);
    };
    for (k, v) in props.entries() {
        let Some(nombre) = k.as_str() else { continue };
        if !cabe(efectivas.get(&format!("{qn}.{nombre}")), techo, lat) {
            continue;
        }
        let crudo = v
            .get("type")
            .and_then(|(_, t)| t.as_str())
            .unwrap_or("String");
        out.push((nombre.to_string(), grafo(crudo)?));
    }
    // `properties` es un MAPA en la forma canónica (`90-canonical-form` §N4), y
    // un mapa se ordena por clave. Emitir en orden de aparición haría que el
    // contrato dependiera de en qué orden tecleó alguien un fichero — y el caso
    // `digest/sdl-ignores-how-it-was-written` lo cazó en rojo antes de que esto
    // llegara a ninguna parte.
    out.sort();
    Ok(out)
}

/// El tipo de OOS, dicho en el vocabulario de GraphQL.
///
/// `Money<EUR, 2>` produce `Money_EUR_2` — **un escalar por combinación**. Un
/// objeto con `currency` convertiría la moneda en un dato que el cliente puede
/// leer e ignorar, y un `Float` haría que sumar euros y dólares dejara de
/// fallar: solo daría cifras incorrectas. La unidad es parte del tipo.
fn grafo(crudo: &str) -> Result<String, String> {
    let t = parse_type(crudo).map_err(|_| format!("tipo `{crudo}` desconocido"))?;
    Ok(match t {
        Type::Scalar(s) => escalar(&s).to_string(),
        Type::Parametric {
            ctor,
            unit,
            precision,
        } => format!("{ctor}_{unit}_{precision}"),
        Type::List(inner) => format!("[{}!]", escalar(&inner)),
        Type::Imported(q) => q.replace('.', "_"),
    })
}

/// Los cuatro que GraphQL ya tiene se traducen; el resto viaja como escalar
/// propio y **se declara** en el mismo SDL — un esquema que referencia un
/// escalar que no declara no es válido.
fn escalar(s: &str) -> &str {
    match s {
        "String" => "String",
        "Integer" => "Int",
        "Float" => "Float",
        "Boolean" => "Boolean",
        otro => otro,
    }
}

fn claves(e: &Loaded) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    if let Some(pk) = e.section("primaryKey") {
        let campos: Vec<String> = pk
            .items()
            .iter()
            .filter_map(|i| i.as_str())
            .map(String::from)
            .collect();
        if !campos.is_empty() {
            out.push(campos);
        }
    }
    if let Some(uk) = e.section("uniqueKeys") {
        let mut alternativas: Vec<Vec<String>> = uk
            .items()
            .iter()
            .map(|clave| {
                clave
                    .items()
                    .iter()
                    .filter_map(|i| i.as_str())
                    .map(String::from)
                    .collect::<Vec<String>>()
            })
            .filter(|c| !c.is_empty())
            .collect();
        // `uniqueKeys` es un CONJUNTO de claves y cada clave una SECUENCIA: se
        // ordena la lista de fuera y se conserva la de dentro. Ordenar la de
        // dentro convertiría una clave compuesta en otra.
        alternativas.sort();
        out.extend(alternativas);
    }
    out
}

fn implementa(e: &Loaded) -> Vec<String> {
    let mut out: Vec<String> = e
        .section("implements")
        .map(|n| {
            n.items()
                .iter()
                .filter_map(|i| i.as_str())
                .map(corto)
                .collect()
        })
        .unwrap_or_default();
    out.sort(); // conjunto
    out
}

/// `cardinality` y `required` de OOS y el par (lista, nulabilidad) de GraphQL
/// tienen **los mismos cuatro estados**, así que no hay nada que decidir.
fn relaciones(pkg: &Package, e: &Loaded, vivos: &BTreeSet<String>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Some(rels) = e.section("relations") else {
        return out;
    };
    for (k, v) in rels.entries() {
        let (Some(nombre), Some(destino)) =
            (k.as_str(), v.get("target").and_then(|(_, t)| t.as_str()))
        else {
            continue;
        };
        let Some(cualificado) = pkg.resolve_entity(destino, e).and_then(|d| d.qname()) else {
            continue;
        };
        if !vivos.contains(&cualificado) {
            continue;
        }
        let tipo = corto(&cualificado);
        let card = v
            .get("cardinality")
            .and_then(|(_, c)| c.as_str())
            .unwrap_or("");
        let obligatoria = v
            .get("required")
            .and_then(|(_, r)| r.as_str())
            .is_some_and(|r| r == "true");
        let muchos = matches!(card, "one_to_many" | "many_to_many");
        let firma = match (muchos, obligatoria) {
            (true, true) => format!("[{tipo}!]!"),
            (true, false) => format!("[{tipo}!]"),
            (false, true) => format!("{tipo}!"),
            (false, false) => tipo,
        };
        out.push((nombre.to_string(), firma));
    }
    out.sort(); // `relations` es un mapa
    out
}

// ── El SDL ──────────────────────────────────────────────────────────────────

/// El orden es el de la forma canónica —`BTreeMap` por nombre cualificado— y no
/// el de aparición en el fichero. Es lo que hace que dos escrituras del mismo
/// paquete emitan el mismo SDL byte a byte (`01-emision-graphql` §6.3).
fn escribir(tipos: &BTreeMap<String, Tipo>, mutaciones: &BTreeMap<String, Mutacion>) -> String {
    let mut s = String::new();

    let mut escalares: BTreeSet<String> = BTreeSet::new();
    for t in tipos.values() {
        for (_, tipo) in &t.campos {
            if let Some(e) = propio(tipo) {
                escalares.insert(e);
            }
        }
    }
    for m in mutaciones.values() {
        for (_, tipo) in m.argumentos.iter().chain(
            m.resultado
                .as_ref()
                .map(|(_, campos)| campos.iter())
                .unwrap_or_default(),
        ) {
            if let Some(e) = propio(tipo) {
                escalares.insert(e);
            }
        }
    }
    escalares.remove(APROBACION);
    for e in &escalares {
        let _ = writeln!(s, "scalar {e}");
    }
    if !escalares.is_empty() {
        s.push('\n');
    }

    for t in tipos.values() {
        let mut cabecera = format!("type {}", t.nombre);
        if !t.interfaces.is_empty() {
            let _ = write!(cabecera, " implements {}", t.interfaces.join(" & "));
        }
        for clave in &t.claves {
            let _ = write!(cabecera, " @key(fields: \"{}\")", clave.join(" "));
        }
        let _ = writeln!(s, "{cabecera} {{");
        for (campo, tipo) in &t.campos {
            let _ = writeln!(s, "  {campo}: {tipo}");
        }
        for (campo, tipo) in &t.aristas {
            let _ = writeln!(s, "  {campo}: {tipo}");
        }
        s.push_str("}\n\n");
    }

    // Uno por clave y uno por colección. Ningún campo de filtrado arbitrario:
    // filtrar es consultar datos, y qué se puede consultar es del protocolo de
    // servicio, que la especificación pone fuera de alcance.
    s.push_str("type Query {\n");
    for t in tipos.values() {
        let args = t
            .claves
            .first()
            .map(|k| {
                k.iter()
                    .map(|f| format!("{f}: ID!"))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let _ = writeln!(s, "  {}({args}): {}", t.raiz, t.nombre);
        let _ = writeln!(
            s,
            "  {}s(first: Int, after: String): [{}!]!",
            t.raiz, t.nombre
        );
    }
    s.push_str("}\n");

    if mutaciones.is_empty() {
        return s;
    }

    for m in mutaciones.values() {
        if let Some((tipo, campos)) = &m.resultado {
            let _ = write!(s, "\ntype {tipo} {{\n");
            for (campo, t) in campos {
                let _ = writeln!(s, "  {campo}: {t}");
            }
            s.push_str("}\n");
        }
    }

    if mutaciones
        .values()
        .any(|m| m.retorno.starts_with(APROBACION))
    {
        s.push_str(
            "\n\"La invocacion quedo propuesta y espera las firmas que su endoso declara.\"\n",
        );
        let _ = write!(
            s,
            "type {APROBACION} {{\n  request: ID!\n  endorsement: String!\n  quorum: Int!\n}}\n"
        );
    }

    s.push_str("\ntype Mutation {\n");
    for m in mutaciones.values() {
        let args = m
            .argumentos
            .iter()
            .map(|(n, t)| format!("{n}: {t}"))
            .collect::<Vec<_>>()
            .join(", ");
        // La exigencia va a la documentacion del campo y no a una directiva:
        // lo que se emite es DESCRIPTIVO, nunca directivo, y una directiva es
        // una instruccion a la herramienta.
        if let Some(n) = m.quorum {
            let _ = writeln!(s, "  \"Requiere {n} firmas humanas distintas.\"");
        }
        let _ = writeln!(s, "  {}({args}): {}", m.nombre, m.retorno);
    }
    s.push_str("}\n");
    s
}

/// Un escalar que GraphQL no trae de serie y que el esquema tiene que declarar.
fn propio(tipo: &str) -> Option<String> {
    let base = tipo.trim_start_matches('[').trim_end_matches(['!', ']']);
    match base {
        "String" | "Int" | "Float" | "Boolean" | "ID" => None,
        otro => Some(otro.to_string()),
    }
}

fn corto(qname: &str) -> String {
    qname.rsplit('.').next().unwrap_or(qname).to_string()
}

fn mayuscula_inicial(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(p) => p.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn minuscula_inicial(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(p) => p.to_lowercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}
