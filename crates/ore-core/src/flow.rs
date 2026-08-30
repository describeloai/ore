//! Flujo de información — la familia `OOS4xxx`.
//!
//! Es la fase que define el producto, y toda ella se apoya en **una regla**:
//!
//! > La información con etiqueta `L` no debe alcanzar un conducto con
//! > autorización `C` salvo que `L ⊑ C`, o que atraviese un desclasificador
//! > autorizado.
//!
//! Todo lo que sigue —retículos, propagación, conductos, desclasificadores— es
//! maquinaria para poder comprobar esa frase. Y se comprueba **sin red, sin
//! credenciales y sin tocar un solo dato**: es lo que hace que un auditor externo
//! pueda verificar la gobernanza clonando el repositorio.
//!
//! El recorrido de `derivedFrom` es el mismo que estrenó `OOS3004` con las
//! unidades. Aquí transporta etiquetas.

use crate::code::Code;
use crate::diag::Diagnostic;
use crate::document::Kind;
use crate::link::{Loaded, Package};
use crate::parse::Node;
use std::collections::{BTreeMap, BTreeSet};

// ── Retículos ───────────────────────────────────────────────────────────────

/// El eje de un retículo. Decide qué se compara y con qué combinador.
///
/// Confidencialidad pregunta *cuánto daño si esto se filtra*; integridad,
/// *cuánto daño si esto es falso*. Son ortogonales: un dato puede ser público y
/// crítico a la vez —el estado de un pedido no es secreto y escribirlo mal
/// cuesta dinero— y por eso hacen falta los dos.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Axis {
    /// Gobierna lecturas. Combina por `join` = máximo.
    #[default]
    Confidentiality,
    /// Gobierna escrituras. Combina por `meet` = mínimo.
    Integrity,
}

impl Axis {
    /// El combinador que el eje implica. **No se declara** — es derivable, y un
    /// campo derivable no es declarable (P2).
    pub const fn combinador(self) -> &'static str {
        match self {
            Axis::Confidentiality => "max",
            Axis::Integrity => "min",
        }
    }
}

/// Un retículo: etiquetas y su orden parcial. El orden **es** la secuencia.
#[derive(Debug, Clone)]
pub struct Lattice {
    pub qname: String,
    pub levels: Vec<String>,
    pub axis: Axis,
    /// Qué exige la clasificación, por nivel: `nivel → naturalezas`.
    ///
    /// Vive en el retículo y no en la regla porque es lo que hace que
    /// **importar la clasificación importe su exigencia**. Y nombra **clases**
    /// de regla, no solo un nivel: sin eso una comprobación de nulos
    /// descargaría lo que un paquete de protección de datos pedía como
    /// política, que es el error de categoría más frecuente
    /// (`v1alpha3/01-gobierno` §6.1).
    pub requires_governance: BTreeMap<String, Vec<String>>,
}

impl Lattice {
    pub fn index(&self, level: &str) -> Option<usize> {
        self.levels.iter().position(|l| l == level)
    }

    /// ¿Está `nivel` en `piso` o por encima?
    ///
    /// `None` si alguno de los dos no pertenece al retículo — que no es lo
    /// mismo que `false`, y confundirlos convertiría una etiqueta mal escrita
    /// en una propiedad que parece no seleccionada.
    pub fn ge(&self, nivel: &str, piso: &str) -> Option<bool> {
        Some(self.index(nivel)? >= self.index(piso)?)
    }
}

/// `oos.maturity` es estándar de la especificación y está siempre activo, lo
/// declare el paquete o no.
///
/// El orden es ASCENDENTE POR RESTRICTIVIDAD, igual que todo retículo, y por
/// eso `STABLE` es el fondo: es lo que puede servirse a cualquier consumidor.
/// Tres partes normativas lo fijan en esa dirección y no en la contraria:
///
/// - `ore promote` es un **desclasificador** y BAJA `DRAFT` a `REVIEWED` a
///   `STABLE` (`04-flow.md` §3, §5). Desclasificar es bajar; luego
///   `STABLE ⊑ REVIEWED ⊑ DRAFT`.
/// - La suite —normativa— razona en `diff/downgrade-maturity` que `DRAFT` es
///   **invisible para los consumidores de producción**. Un `contextSurface`
///   que admite `STABLE` y rechaza `DRAFT` solo es expresable con este orden.
/// - Las autorizaciones de ejemplo de `04-flow.md` §4 solo son coherentes así:
///   `cache: STABLE` admite únicamente lo estable, y `log: DEPRECATED` —el
///   techo— lo admite todo. Con el orden inverso, `cache` aceptaría un
///   borrador y `contextSurface` rechazaría lo estable.
fn maturity() -> Lattice {
    Lattice {
        qname: "oos.maturity".into(),
        levels: ["STABLE", "REVIEWED", "DRAFT", "DEPRECATED"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        axis: Axis::Confidentiality,
        // `oos.maturity` no exige gobierno: es el ciclo de vida de un
        // documento, no una clasificación de sensibilidad. Obligar a cubrir
        // todo lo que no sea STABLE convertiría un borrador en un error.
        requires_governance: BTreeMap::new(),
    }
}

pub fn lattices(pkg: &Package) -> BTreeMap<String, Lattice> {
    let mut out = BTreeMap::new();
    let m = maturity();
    out.insert(m.qname.clone(), m);
    for d in pkg.docs.iter().filter(|d| d.kind == Kind::Lattice) {
        let Some(q) = d.qname() else { continue };
        let levels: Vec<String> = d
            .section("levels")
            .map(|n| {
                n.items()
                    .iter()
                    .filter_map(|i| i.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        // Sin `axis`, confidencialidad: es lo que hace que todo retículo de
        // v1alpha1 siga significando lo mismo sin tocar un fichero.
        let axis = match d.section("axis").and_then(|n| n.as_str()) {
            Some("integrity") => Axis::Integrity,
            _ => Axis::Confidentiality,
        };
        let requires_governance: BTreeMap<String, Vec<String>> = d
            .section("requiresGovernance")
            .map(|n| {
                n.entries()
                    .iter()
                    .filter_map(|(k, v)| {
                        let nivel = k.as_str()?.to_string();
                        let naturalezas = v
                            .items()
                            .iter()
                            .filter_map(|i| i.as_str().map(String::from))
                            .collect();
                        Some((nivel, naturalezas))
                    })
                    .collect()
            })
            .unwrap_or_default();
        out.insert(
            q.clone(),
            Lattice {
                qname: q,
                levels,
                axis,
                requires_governance,
            },
        );
    }
    out
}

// ── Etiquetas ───────────────────────────────────────────────────────────────

/// De dónde salió una etiqueta. Es lo único que distingue `OOS4002` de
/// `OOS4001`, y la distinción no es cosmética: la directa la detecta cualquier
/// linter, la computada no la hace nadie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Escrita en la propia propiedad.
    Declared,
    /// Heredada de la entidad, o del datasource al que se enlaza.
    Inherited,
    /// **Computada** por el compilador propagando `join` desde los orígenes de
    /// una derivación. Nadie la escribió en ninguna parte.
    Computed,
}

/// Etiquetas efectivas de **una propiedad**: retículo → (nivel, de dónde salió).
type Labels = BTreeMap<String, (String, Origin)>;

/// Etiquetas efectivas de **una entidad**: propiedad → sus etiquetas.
type EntityLabels = BTreeMap<String, Labels>;

/// Etiquetas efectivas de todo el paquete: `entidad.propiedad` → retículo →
/// nivel.
///
/// Se expone para [`governance`](crate::governance), que necesita exactamente
/// esto y **no debe recalcularlo**: un objetivo que viera solo las etiquetas
/// declaradas dejaría fuera las heredadas de la entidad y las computadas por
/// propagación, que son las dos que nadie escribió y por tanto las que más
/// falta hace gobernar.
pub fn efectivas(
    pkg: &Package,
    lat: &BTreeMap<String, Lattice>,
) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    for e in pkg.entities() {
        let qn = e.qname().unwrap_or_default();
        for (prop, etiquetas) in propagar_solo(pkg, e, lat) {
            out.insert(
                format!("{qn}.{prop}"),
                etiquetas
                    .into_iter()
                    .map(|(ret, (nivel, _))| (ret, nivel))
                    .collect(),
            );
        }
    }
    out
}

fn read_labels(n: &Node) -> Vec<(String, String, crate::diag::Pos)> {
    n.get("labels")
        .map(|(_, l)| {
            l.entries()
                .iter()
                .filter_map(|(k, v)| {
                    Some((k.as_str()?.to_string(), v.as_str()?.to_string(), v.pos()))
                })
                .collect()
        })
        .unwrap_or_default()
}

// ── El chequeo completo ─────────────────────────────────────────────────────

pub fn check(pkg: &Package) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let lat = lattices(pkg);

    // 1 · Toda etiqueta escrita debe pertenecer a un retículo. Sin esto, el
    //     resto de la fase compararía contra la nada.
    etiquetas_conocidas(pkg, &lat, &mut out);
    if !out.is_empty() {
        return out;
    }

    // 2 · Herencia y propagación.
    let mut efectivas: BTreeMap<String, EntityLabels> = BTreeMap::new();
    for e in pkg.entities() {
        let qn = e.qname().unwrap_or_default();
        efectivas.insert(qn, propagar(pkg, e, &lat, &mut out));
    }
    if !out.is_empty() {
        return out;
    }

    // 3 · Los conductos y la regla de flujo.
    let conductos = clearances(pkg, &lat);
    materializaciones(pkg, &lat, &efectivas, &conductos, &mut out);

    // 4 · Desclasificadores y valores de ejemplo.
    desclasificadores(pkg, &mut out);
    ejemplos(pkg, &lat, &efectivas, &mut out);

    out
}

// ── OOS4003 ─────────────────────────────────────────────────────────────────

fn etiquetas_conocidas(pkg: &Package, lat: &BTreeMap<String, Lattice>, out: &mut Vec<Diagnostic>) {
    let mut revisar = |d: &Loaded, n: &Node| {
        for (ret, nivel, pos) in read_labels(n) {
            // Una etiqueta de un retículo de integridad no es asunto de esta
            // fase: la comprueba `effect` con `OOS7003`. Emitir `OOS4003` aquí
            // sería contestar en el eje equivocado.
            if lat.get(&ret).is_some_and(|l| l.axis == Axis::Integrity) {
                continue;
            }
            let malo = match lat.get(&ret) {
                None => Some(format!(
                    "no hay ningún retículo `{ret}` declarado ni importado"
                )),
                Some(l) if l.index(&nivel).is_none() => Some(format!(
                    "`{nivel}` no es un nivel de `{ret}`; sus niveles son {}",
                    l.levels.join(" ⊑ ")
                )),
                Some(_) => None,
            };
            if let Some(m) = malo {
                out.push(
                    Diagnostic::new(
                        Code::Oos4003,
                        &d.path,
                        format!("etiqueta `{ret}:{nivel}`: {m}"),
                    )
                    .at(pos)
                    .help(
                        "el esquema comprueba que la clave es un nombre cualificado y el \
                             valor un identificador, y ahí se acaba lo que puede saber: que ese \
                             nivel exista en ese retículo es una relación entre documentos",
                    ),
                );
            }
        }
    };

    for e in pkg.entities() {
        if let Some((_, m)) = e.root.get("metadata") {
            revisar(e, m);
        }
        if let Some(ps) = e.section("properties") {
            for (_, v) in ps.entries() {
                revisar(e, v);
            }
        }
    }
    for c in pkg.docs.iter().filter(|d| d.kind == Kind::OntologyConfig) {
        for ds in c.section("datasources").map(|n| n.items()).unwrap_or(&[]) {
            revisar(c, ds);
        }
    }
}

// ── OOS4008 · OOS4012 · propagación ─────────────────────────────────────────

fn propagar(
    pkg: &Package,
    e: &Loaded,
    lat: &BTreeMap<String, Lattice>,
    out: &mut Vec<Diagnostic>,
) -> EntityLabels {
    let qn = e.qname().unwrap_or_default();

    // Heredadas de la entidad: lo cierto del conjunto se declara una vez.
    let mut heredadas: Labels = BTreeMap::new();
    if let Some((_, m)) = e.root.get("metadata") {
        for (r, n, _) in read_labels(m) {
            heredadas.insert(r, (n, Origin::Inherited));
        }
    }

    // Heredadas del datasource: la ubicación física es un hecho del mundo, no
    // una decisión de modelado, así que se computa.
    for b in pkg.docs.iter().filter(|d| d.kind == Kind::Binding) {
        if b.section("targetEntity").and_then(|t| t.as_str()) != Some(qn.as_str()) {
            continue;
        }
        let Some(dsref) = b.section("datasourceRef").and_then(|d| d.as_str()) else {
            continue;
        };
        for c in pkg.docs.iter().filter(|d| d.kind == Kind::OntologyConfig) {
            for ds in c.section("datasources").map(|n| n.items()).unwrap_or(&[]) {
                if ds.get("name").and_then(|(_, v)| v.as_str()) != Some(dsref) {
                    continue;
                }
                for (r, n, _) in read_labels(ds) {
                    let subir = match (heredadas.get(&r), lat.get(&r)) {
                        (Some((actual, _)), Some(l)) => l.index(&n) > l.index(actual),
                        (None, _) => true,
                        _ => false,
                    };
                    if subir {
                        heredadas.insert(r, (n, Origin::Inherited));
                    }
                }
            }
        }
    }

    let mut efectivas: EntityLabels = BTreeMap::new();
    let Some(ps) = e.section("properties") else {
        return BTreeMap::new();
    };

    // Primera pasada: declaradas y heredadas.
    for (k, v) in ps.entries() {
        let Some(nombre) = k.as_str() else { continue };
        let derivada = v.get("derivedFrom").is_some();
        let propias = read_labels(v);

        // OOS4008 · una derivada no declara etiqueta. Falla AUNQUE el valor
        // declarado sea el correcto: si se admitiera cuando coincide, el día
        // que alguien rebaje un origen la etiqueta seguiría mintiendo.
        if derivada && !propias.is_empty() {
            out.push(
                Diagnostic::new(
                    Code::Oos4008,
                    &e.path,
                    format!("`{qn}.{nombre}` es derivada y declara etiqueta"),
                )
                .at(propias[0].2)
                .help(
                    "la etiqueta de una derivada la computa el compilador con `join` sobre sus \
                     orígenes. Declararla es un error aunque el valor sea el correcto hoy: una \
                     etiqueta que un humano puede desincronizar del código acaba mintiendo, y \
                     firmarla criptográficamente lo empeora — parece verificada",
                ),
            );
            continue;
        }

        let mut ls = heredadas.clone();
        for (r, n, pos) in propias {
            // OOS4012 · elevar es legítimo; rebajar, no.
            if let (Some((heredado, _)), Some(l)) = (heredadas.get(&r), lat.get(&r))
                && l.index(&n) < l.index(heredado)
            {
                out.push(
                    Diagnostic::new(
                        Code::Oos4012,
                        &e.path,
                        format!("`{qn}.{nombre}` rebaja `{r}` de `{heredado}` a `{n}`"),
                    )
                    .at(pos)
                    .help(
                        "restringir siempre se puede; relajar es una decisión que exige \
                             tomarse donde se declaró la restricción, no en una propiedad suelta",
                    ),
                );
                continue;
            }
            ls.insert(r, (n, Origin::Declared));
        }
        efectivas.insert(nombre.to_string(), ls);
    }

    // Segunda pasada: propagación por derivación. `join` = la más restrictiva.
    for (k, v) in ps.entries() {
        let Some(nombre) = k.as_str() else { continue };
        let Some((_, from)) = v.get("derivedFrom") else {
            continue;
        };
        let mut ls = heredadas.clone();
        for r in from.items() {
            let Some(q) = r.as_str() else { continue };
            let Some((ent, prop)) = q.rsplit_once('.') else {
                continue;
            };
            let origen = if ent == qn {
                efectivas.get(prop).cloned()
            } else {
                pkg.entity(ent).map(|o| {
                    propagar_solo(pkg, o, lat)
                        .get(prop)
                        .cloned()
                        .unwrap_or_default()
                })
            };
            for (ret, (nivel, _)) in origen.unwrap_or_default() {
                let subir = match (ls.get(&ret), lat.get(&ret)) {
                    (Some((actual, _)), Some(l)) => l.index(&nivel) > l.index(actual),
                    (None, _) => true,
                    _ => false,
                };
                if subir {
                    ls.insert(ret, (nivel, Origin::Computed));
                }
            }
        }
        efectivas.insert(nombre.to_string(), ls);
    }

    efectivas
}

/// Propagación sin emitir diagnósticos, para resolver derivaciones que cruzan
/// entidades sin duplicar los errores de la otra.
fn propagar_solo(pkg: &Package, e: &Loaded, lat: &BTreeMap<String, Lattice>) -> EntityLabels {
    let mut descartar = Vec::new();
    propagar(pkg, e, lat, &mut descartar)
}

// ── Conductos ───────────────────────────────────────────────────────────────

/// Autorización efectiva de cada conducto. Varias políticas se combinan
/// tomando la **más restrictiva**: una local nunca afloja lo que una importada
/// restringe.
fn clearances(pkg: &Package, lat: &BTreeMap<String, Lattice>) -> BTreeMap<String, Labels> {
    let mut out: BTreeMap<String, Labels> = BTreeMap::new();
    for cp in pkg.docs.iter().filter(|d| d.kind == Kind::ConduitPolicy) {
        let Some(cs) = cp.section("conduits") else {
            continue;
        };
        for (ck, cv) in cs.entries() {
            let Some(nombre) = ck.as_str() else { continue };
            let entrada = out.entry(nombre.to_string()).or_default();
            for (rk, rv) in cv.entries() {
                let (Some(ret), Some(nivel)) = (rk.as_str(), rv.as_str()) else {
                    continue;
                };
                let bajar = match (entrada.get(ret), lat.get(ret)) {
                    (Some((actual, _)), Some(l)) => l.index(nivel) < l.index(actual),
                    (None, _) => true,
                    _ => false,
                };
                if bajar {
                    entrada.insert(ret.to_string(), (nivel.to_string(), Origin::Declared));
                }
            }
        }
    }
    out
}

// ── OOS4001 · OOS4002 · OOS4011 ─────────────────────────────────────────────

fn materializaciones(
    pkg: &Package,
    lat: &BTreeMap<String, Lattice>,
    efectivas: &BTreeMap<String, EntityLabels>,
    conductos: &BTreeMap<String, Labels>,
    out: &mut Vec<Diagnostic>,
) {
    for b in pkg.docs.iter().filter(|d| d.kind == Kind::Binding) {
        let Some(mat) = b.section("materialization") else {
            continue;
        };
        let modo = mat
            .get("mode")
            .and_then(|(_, v)| v.as_str())
            .unwrap_or("passthrough");
        if modo == "passthrough" {
            continue;
        }
        let conducto = format!("materialization.{modo}");

        // OOS4011 · omitir un conducto no es dejarlo abierto: es cerrarlo.
        let Some(autorizacion) = conductos.get(&conducto) else {
            out.push(
                Diagnostic::new(
                    Code::Oos4011,
                    &b.path,
                    format!("el conducto `{conducto}` no tiene autorización declarada"),
                )
                .at(mat.pos())
                .help(format!(
                    "un conducto sin autorización es ⊥ y no admite nada. Declara `{conducto}` \
                     en la política de conductos, o baja el modo a `passthrough`"
                )),
            );
            continue;
        };

        let Some(target) = b.section("targetEntity").and_then(|t| t.as_str()) else {
            continue;
        };
        let Some(entidad) = pkg.entity(target) else {
            continue;
        };
        let Some(props) = efectivas.get(target) else {
            continue;
        };

        // Qué fluye: en `cache`, lo declarado; en `index`, la topología —clave
        // primaria y propiedades `via`—, que es derivable.
        let fluyen: Vec<String> = if modo == "cache" {
            mat.get("properties")
                .map(|(_, p)| {
                    p.items()
                        .iter()
                        .filter_map(|i| i.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default()
        } else {
            let mut v: Vec<String> = entidad
                .section("primaryKey")
                .map(|k| {
                    k.items()
                        .iter()
                        .filter_map(|i| i.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            if let Some(rels) = entidad.section("relations") {
                for (_, rv) in rels.entries() {
                    if let Some((_, via)) = rv.get("via")
                        && let Some(s) = via.as_str()
                    {
                        v.push(s.to_string());
                    }
                }
            }
            v
        };

        for p in fluyen {
            let Some(labels) = props.get(&p) else {
                continue;
            };
            for (ret, (nivel, origen)) in labels {
                let Some(l) = lat.get(ret) else { continue };
                let permitido = autorizacion
                    .get(ret)
                    .and_then(|(n, _)| l.index(n))
                    .unwrap_or(0);
                let Some(tiene) = l.index(nivel) else {
                    continue;
                };
                if tiene <= permitido {
                    continue;
                }

                let (code, como) = match *origen {
                    Origin::Computed => (Code::Oos4001, "computada por join"),
                    Origin::Declared => (Code::Oos4002, "declarada"),
                    Origin::Inherited => (Code::Oos4002, "heredada"),
                };
                let permitido_txt = autorizacion
                    .get(ret)
                    .map(|(n, _)| n.clone())
                    .unwrap_or_else(|| l.levels[0].clone());

                out.push(
                    Diagnostic::new(
                        code,
                        &b.path,
                        format!(
                            "`{target}.{p}` alcanza `{conducto}` con `{ret} = {nivel}` \
                             ({como}); el conducto admite `{permitido_txt}`"
                        ),
                    )
                    .at(mat.pos())
                    .help(if *origen == Origin::Computed {
                        "nadie escribió esa etiqueta: la computó el compilador propagando \
                         `join` desde los orígenes de la derivación. Baja el modo a \
                         `passthrough`, aplica un desclasificador autorizado, o eleva la \
                         autorización del conducto — lo último exige revisión de CODEOWNERS"
                    } else {
                        "baja el modo a `passthrough`, aplica un desclasificador autorizado, o \
                         eleva la autorización del conducto — lo último exige revisión de \
                         CODEOWNERS"
                    }),
                );
            }
        }
    }
}

// ── OOS4006 · OOS4007 ───────────────────────────────────────────────────────

/// El vocabulario **cerrado** de desclasificadores. Cerrarlo es lo que lo hace
/// analizable: con un conjunto abierto, el compilador tendría que elegir entre
/// suponer que una obligación desconocida desclasifica —inseguro— o suponer que
/// no —lo que haría inútil la extensibilidad que supuestamente ganaba.
const DESCLASIFICADORES: &[&str] = &["mask", "tokenize", "redact", "aggregate", "promote"];

fn desclasificadores(pkg: &Package, out: &mut Vec<Diagnostic>) {
    for (path, texto) in &pkg.cedar {
        for (i, linea) in texto.lines().enumerate() {
            let Some(inicio) = linea.find("@obligation(") else {
                continue;
            };
            let resto = &linea[inicio + 12..];
            let Some(fin) = resto.find(')') else { continue };
            let arg = resto[..fin].trim().trim_matches('"');
            let (nombre, param) = match arg.split_once(':') {
                Some((n, p)) => (n, Some(p)),
                None => (arg, None),
            };
            let pos = crate::diag::Pos {
                line: i + 1,
                col: inicio + 1,
            };

            if !DESCLASIFICADORES.contains(&nombre) {
                out.push(
                    Diagnostic::new(
                        Code::Oos4006,
                        path,
                        format!("`{nombre}` no es un desclasificador de OOS"),
                    )
                    .at(pos)
                    .help(format!(
                        "el vocabulario es cerrado: {}. Cerrarlo es lo que permite demostrar \
                         que ningún dato etiquetado llega sin transformar a un conducto, y que \
                         un regulador pueda leer la lista completa de transformaciones posibles",
                        DESCLASIFICADORES.join(" · ")
                    )),
                );
                continue;
            }

            if nombre == "aggregate" {
                let umbral = param
                    .and_then(|p| p.split_once('='))
                    .filter(|(k, _)| k.trim() == "minGroupSize")
                    .and_then(|(_, v)| v.trim().parse::<u32>().ok());
                if umbral.is_none() {
                    out.push(
                        Diagnostic::new(Code::Oos4007, path, "`aggregate` sin `minGroupSize`")
                            .at(pos)
                            .help(
                                "sin umbral no desclasifica nada: el agregado de un grupo de una \
                             persona es esa persona. Aceptarlo sería peor que rechazarlo — el \
                             paquete compilaría y todo el mundo creería que hay una garantía \
                             de k-anonimato donde solo hay una palabra",
                            ),
                    );
                    continue;
                }
            }
        }
    }
}

// ── OOS4014 ─────────────────────────────────────────────────────────────────

fn ejemplos(
    pkg: &Package,
    lat: &BTreeMap<String, Lattice>,
    efectivas: &BTreeMap<String, EntityLabels>,
    out: &mut Vec<Diagnostic>,
) {
    for e in pkg.entities() {
        let qn = e.qname().unwrap_or_default();
        let Some(ps) = e.section("properties") else {
            continue;
        };
        let Some(labels) = efectivas.get(&qn) else {
            continue;
        };

        for (k, v) in ps.entries() {
            let Some(nombre) = k.as_str() else { continue };
            let Some((_, ex)) = v.get("examples") else {
                continue;
            };
            let sintetico = ex.get("synthetic").and_then(|(_, s)| s.as_str()) == Some("true");
            if sintetico {
                continue;
            }
            // Solo importa si la propiedad está etiquetada por encima de ⊥.
            let etiquetada: BTreeSet<&String> = labels
                .get(nombre)
                .map(|ls| {
                    ls.iter()
                        .filter(|(r, (n, _))| lat.get(*r).and_then(|l| l.index(n)).unwrap_or(0) > 0)
                        .map(|(r, _)| r)
                        .collect()
                })
                .unwrap_or_default();
            if etiquetada.is_empty() {
                continue;
            }
            out.push(
                Diagnostic::new(
                    Code::Oos4014,
                    &e.path,
                    format!("`{qn}.{nombre}` está etiquetada y declara `examples` reales"),
                )
                .at(ex.pos())
                .help(
                    "los valores de ejemplo de una columna de salarios son salarios, y este \
                     fichero se revisa en un pull request, se publica y alcanza la superficie \
                     de contexto de cualquier agente. Declara `synthetic: true` si no proceden \
                     de datos reales — obligar a decirlo convierte un descuido silencioso en \
                     una afirmación consciente",
                ),
            );
        }
    }
}
