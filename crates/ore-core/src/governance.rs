//! Gobierno — la familia `OOS8xxx`.
//!
//! Ni [`flow`](crate::flow) ni [`effect`](crate::effect): el tercero. Aquel
//! gobierna **lo que se puede saber**, el otro **lo que se puede causar**, y
//! este **qué debe sostenerse y quién responde**.
//!
//! > `∀x . L(x) ⊒ n ⟹ ∃r . x ∈ objetivo(r)`
//!
//! # Lo que hace distinta a esta fase
//!
//! Las otras dos razonan documento a documento: una etiqueta mal puesta, una
//! función que no alcanza su destino. Esta razona sobre el paquete **entero**,
//! y su error central —`OOS8001`— no señala una línea equivocada sino **una
//! línea que nadie escribió**. Es el `OOS4001` de este plano: no hay diff donde
//! mirarlo, no lo encuentra un `grep` y no lo encuentra una revisión de código.
//!
//! Por eso corre la última. Calcular una ausencia sobre etiquetas que todavía
//! podrían estar mal produciría el peor diagnóstico posible: señalar algo que
//! no existe por culpa de algo que sí.
//!
//! # La pieza que lo hace gratis
//!
//! No hay lenguaje de objetivos. Un retículo declarado para **comparar** dos
//! elementos —`L ⊑ C`— nombra, leído en la otra dirección, un **conjunto**:
//! `{x : L(x) ⊒ n}`. Todo este módulo es esa lectura, y es decidible al
//! compilar por la misma razón que lo es la regla de flujo.
//!
//! # Cobertura y adecuación
//!
//! `OOS8001` demuestra que **existe** una regla **de la clase que la
//! clasificación exige**. Eso cierra los huecos baratos —lo ilegible, lo que no
//! puede fallar— y el frecuente: el **error de categoría**, cubrir con una
//! comprobación de calidad lo que un paquete de protección de datos pedía como
//! política.
//!
//! Lo que no cierra, y ningún análisis estático cerrará, es si la regla es **la
//! adecuada**: una política que permite todo cubre igual que una que no permite
//! nada, y la diferencia no está en el documento sino en lo que la organización
//! quería. Se responde como todo lo indecidible aquí — **exigiendo que alguien
//! responda**: un `owner`, y cuando hace falta más, un endoso.
//!
//! > El compilador decide la cobertura. El endoso registra la adecuación.
//!
//! Se dice para que nadie lo deduzca de que los casos pasan.

use crate::code::Code;
use crate::diag::{Diagnostic, Pos};
use crate::document::Kind;
use crate::flow::{self, Axis, Lattice};
use crate::link::{Loaded, Package};
use crate::normalize;
use std::collections::{BTreeMap, BTreeSet};

/// Los desclasificadores admisibles **como máscara**.
///
/// Subconjunto del vocabulario cerrado de `04-flow` §5, y falta uno a
/// propósito: `promote` **sube** por un retículo de ciclo de vida y una máscara
/// tiene que bajar. Excluirlo no es retirarlo — sigue siendo un desclasificador
/// para su propio uso.
const MASCARAS: &[&str] = &["mask", "tokenize", "redact", "aggregate"];

/// Un objetivo: retículo → nivel mínimo. La conjunción de sus entradas.
type Objetivo = BTreeMap<String, String>;

/// Etiquetas efectivas: `entidad.propiedad` → retículo → nivel.
type Props = BTreeMap<String, BTreeMap<String, String>>;

pub fn check(pkg: &Package) -> Vec<Diagnostic> {
    let formas = crate::significado::por_forma(pkg);
    let reglas: Vec<&Loaded> = pkg
        .docs
        .iter()
        .filter(|d| d.kind == Kind::Ruleset)
        .collect();
    let lat = flow::lattices(pkg);
    let mut out = Vec::new();

    // 1 · Que los objetivos apunten a algo que existe y del eje correcto.
    //     Antes que nada: seleccionar sobre un retículo que no está, sobre el
    //     eje equivocado o sobre una propiedad que no existe no produce una
    //     selección mala — produce una vacía, y el error saldría como
    //     `OOS8002`, que es el diagnóstico equivocado.
    let props = flow::efectivas(pkg, &lat);
    for r in &reglas {
        objetivos_validos(r, &lat, &props, &formas, &mut out);
    }
    if !out.is_empty() {
        return out;
    }

    // 2 · La selección, de la que depende todo lo demás.
    let mut sel: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for r in &reglas {
        let mut union = BTreeSet::new();
        for (dom, pos) in objetivos(r, &formas) {
            let casan = selecciona(&dom, &props, &lat);
            if casan.is_empty() {
                out.push(vacio(r, &dom, pos));
            }
            union.extend(casan);
        }
        sel.insert(r.qname().unwrap_or_default(), union);
    }
    if !out.is_empty() {
        return out;
    }

    // 3 · Las tres piezas, cada una contra su objetivo.
    let fuentes = fuentes_por_entidad(pkg);
    for r in &reglas {
        let q = r.qname().unwrap_or_default();
        let seleccionadas = sel.get(&q).cloned().unwrap_or_default();
        mascaras(r, &lat, &props, &formas, &mut out);
        aserciones(r, &seleccionadas, &fuentes, &mut out);
        deberes(pkg, r, &mut out);
        ambitos(pkg, r, &props, &mut out);
    }
    mascara_con_sujeto(pkg, &lat, &props, &sel, &mut out);
    ambito_con_sujeto(pkg, &mut out);
    finalidades(pkg, &mut out);
    roles_sin_origen(pkg, &mut out);
    if !out.is_empty() {
        return out;
    }

    // 4 · Y la cobertura, que es una diferencia de conjuntos.
    cobertura(pkg, &lat, &props, &reglas, &sel, &mut out);
    out
}

/// **Quién descarga** una clase de gobierno sobre una propiedad.
///
/// De un `Ruleset` responde su `owner`. De una **política de Cedar** responde el
/// dueño del `ConduitPolicy`, y eso no es una inferencia cómoda: quien eleva la
/// autorización de un conducto y quien escribe un `permit` toman la misma clase
/// de decisión —**quién ve qué**— y son la misma persona.
///
/// El `owner` sigue siendo `Option` porque con **varios** `ConduitPolicy` no se
/// hereda: no habría forma de saber de cuál, y adivinar el dueño de una decisión
/// de seguridad es peor que no tenerlo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Descarga {
    /// El `Ruleset` cualificado, o el `@id` de la política.
    pub regla: String,
    pub owner: Option<String>,
}

/// Qué clases de gobierno cubren de hecho a cada propiedad, **y por quién**.
///
/// Se expone para `diff` y para el informe, y por la misma razón que
/// `selecciones`: **dos definiciones de «esta propiedad está gobernada» serían
/// dos semánticas.** La que decide `OOS8001` al compilar tiene que ser la misma
/// que decide si una versión nueva retiró gobierno, y la misma que se imprime.
///
/// Y es lo que permite que el versionado tenga **un código por síntoma y no uno
/// por causa**: quitar un `Ruleset`, estrechar un objetivo, borrar una aserción
/// o cambiar una etiqueta son cambios distintos con un solo efecto observable —
/// esta propiedad ha dejado de tener esta clase de gobierno.
///
/// # El defecto que arregla, y que llevaba desde v1alpha3
///
/// La versión anterior computaba `authorization` desde el conjunto global de
/// **etiquetas** que las políticas mencionan, así que una política que nombra la
/// propiedad **directamente** —`resource == Property::"hr.Employee.nationalId"`—
/// **no contaba**. Medido sobre la ontología de referencia:
///
/// ```text
/// error[OOS8001]: `hr.Employee.nationalId` exige `authorization` y no lo tiene
/// ```
///
/// …con el `forbid` más contundente del fichero apuntándole por su nombre. La
/// proyección tiene **dos mitades** —`Property in [Label, <Entidad>]`— y la
/// cobertura leía una. Es el mismo defecto que `politica::alcance` tenía un piso
/// más abajo, y aquí muerde más fuerte porque **rompe la compilación**: un
/// paquete que gobierne enumerando no compilaba, y el mensaje le decía que le
/// faltaba lo que tenía.
///
/// Se arregla usando `alcance`, que ya mira las dos — en vez de una segunda
/// lectura que volvería a poder divergir.
pub fn cobertura_atribuida(pkg: &Package) -> BTreeMap<String, BTreeMap<&'static str, Vec<Descarga>>> {
    let sel = selecciones(pkg);
    let mut out: BTreeMap<String, BTreeMap<&'static str, Vec<Descarga>>> = BTreeMap::new();

    for r in pkg.docs.iter().filter(|d| d.kind == Kind::Ruleset) {
        let q = r.qname().unwrap_or_default();
        let Some(seleccionadas) = sel.get(&q) else {
            continue;
        };
        let owner = r
            .section("owner")
            .and_then(|o| o.as_str())
            .map(String::from);
        for clase in aporta(pkg, r) {
            for prop in seleccionadas {
                out.entry(prop.clone()).or_default().entry(clase).or_default().push(Descarga {
                    regla: q.clone(),
                    owner: owner.clone(),
                });
            }
        }
    }

    // **De una política de Cedar responde quien responde de los conductos.**
    //
    // No es una inferencia cómoda: quien eleva la autorización de un conducto y
    // quien escribe un `permit` toman la misma clase de decisión —quién ve qué—
    // y en la práctica son la misma persona. El ejemplo de referencia lo llevaba
    // escrito en un comentario antes de que existiera el campo.
    //
    // Con VARIOS `ConduitPolicy` no se hereda: no habría forma de saber de cuál,
    // y adivinar el dueño de una decisión de seguridad es peor que no tenerlo.
    // Ahí la salida es `@oosOwner` en la propia política — el caso raro, no la
    // forma.
    let dueno_de_politicas = {
        let mut cps = pkg.of(Kind::ConduitPolicy);
        match (cps.next(), cps.next()) {
            (Some(cp), None) => cp
                .section("owner")
                .and_then(|o| o.as_str())
                .map(String::from),
            _ => None,
        }
    };

    // `authorization` la descarga una política de Cedar, y **qué alcanza cada
    // política** ya lo computa `alcance`: reutilizarlo es lo que impide que esta
    // lectura y aquella se separen.
    for (politica, props) in crate::politica::alcance(pkg) {
        for prop in props {
            out.entry(prop)
                .or_default()
                .entry("authorization")
                .or_default()
                .push(Descarga {
                    regla: politica.clone(),
                    owner: dueno_de_politicas.clone(),
                });
        }
    }
    out
}

/// La misma cobertura, sin atribución. **Derivada**, no recomputada.
pub fn cobertura_efectiva(pkg: &Package) -> BTreeMap<String, BTreeSet<&'static str>> {
    cobertura_atribuida(pkg)
        .into_iter()
        .map(|(p, m)| (p, m.keys().copied().collect()))
        .filter(|(_, n): &(String, BTreeSet<&str>)| !n.is_empty())
        .collect()
}

/// La selección de cada `Ruleset`: nombre cualificado del documento → las
/// propiedades que casan con alguno de sus objetivos.
///
/// Se expone para la emisión a ODCS, que necesita exactamente esto: una
/// aserción se proyecta a la propiedad que gobierna. Y se expone en vez de
/// recomputarse allí porque **dos selecciones serían dos semánticas** — la
/// única definición de «qué gobierna esta regla» vive en `selecciona`, y las
/// dos rutas la comparten.
pub fn selecciones(pkg: &Package) -> BTreeMap<String, BTreeSet<String>> {
    let formas = crate::significado::por_forma(pkg);
    let lat = flow::lattices(pkg);
    let props = flow::efectivas(pkg, &lat);
    pkg.docs
        .iter()
        .filter(|d| d.kind == Kind::Ruleset)
        .map(|r| {
            let mut union = BTreeSet::new();
            for (dom, _) in objetivos(r, &formas) {
                union.extend(selecciona(&dom, &props, &lat));
            }
            (r.qname().unwrap_or_default(), union)
        })
        .collect()
}

// ── Los objetivos ───────────────────────────────────────────────────────────

/// El índice de formas: `interfaz` → propiedades que la satisfacen.
type Formas = BTreeMap<String, BTreeSet<String>>;

/// Cómo un objetivo nombra su dominio.
///
/// Tres criterios y **un solo sitio donde escribirlos**. `atLeast` lo computa —el
/// retículo leído al revés—; `named` lo escribe. Que la segunda sea admisible
/// es consecuencia de `OOS8001`: enumerar se pudre porque una propiedad nueva
/// se escapa **en silencio**, y con la regla de cobertura eso rompe la
/// compilación. Era el silencio, no la enumeración, lo que hacía daño.
///
/// Prohibirla obligaba a que el caso enumerado viviera colgando de la
/// propiedad, que es una segunda superficie de autoría **sin dueño propio**:
/// movía el problema en vez de resolverlo.
/// El tercero llegó con v1alpha4 y no es una tercera manera de decir lo
/// mismo: `atLeast` nombra un conjunto por **clasificación**, `named` por
/// **identidad**, `implements` por **forma**. Que existan los tres es lo que
/// permite gobernar quince casi-duplicados de quince sistemas con una regla,
/// sin renombrar ninguno — porque la forma se expresa en conceptos y no en
/// nombres.
enum Dominio {
    /// El dominio computado: retículo → nivel mínimo, en conjunción.
    Predicado(Objetivo),
    /// El dominio escrito: nombres cualificados de propiedad.
    Nombres(Vec<String>),
    /// El dominio por forma, **ya resuelto**: se guardan las interfaces para
    /// poder nombrarlas en un diagnóstico y las propiedades para seleccionar.
    ///
    /// Resolverlo aquí y no en cada consumidor es deliberado: si `selecciona`
    /// y `suelo` lo resolvieran cada uno por su lado serían dos lecturas del
    /// mismo objetivo, y la cobertura vería una de las dos.
    Forma {
        interfaces: Vec<String>,
        propiedades: Vec<String>,
    },
}

/// Lee `spec.targets`, con la posición de cada uno para poder señalarlo.
fn objetivos(r: &Loaded, formas: &Formas) -> Vec<(Dominio, Pos)> {
    r.section("targets")
        .map(|n| n.items())
        .unwrap_or(&[])
        .iter()
        .filter_map(|t| {
            if let Some((_, mapa)) = t.get("atLeast") {
                let obj: Objetivo = mapa
                    .entries()
                    .iter()
                    .filter_map(|(k, v)| Some((k.as_str()?.to_string(), v.as_str()?.to_string())))
                    .collect();
                return Some((Dominio::Predicado(obj), mapa.pos()));
            }
            if let Some((_, lista)) = t.get("implements") {
                let interfaces: Vec<String> = lista
                    .items()
                    .iter()
                    .filter_map(|i| i.as_str().map(String::from))
                    .collect();
                let propiedades = interfaces
                    .iter()
                    .filter_map(|i| formas.get(i))
                    .flatten()
                    .cloned()
                    .collect();
                return Some((
                    Dominio::Forma {
                        interfaces,
                        propiedades,
                    },
                    lista.pos(),
                ));
            }
            let (_, lista) = t.get("named")?;
            let nombres = lista
                .items()
                .iter()
                .filter_map(|i| i.as_str().map(String::from))
                .collect();
            Some((Dominio::Nombres(nombres), lista.pos()))
        })
        .collect()
}

/// `OOS4003` · `OOS8006` · `OOS2005` — que el objetivo apunte a algo que existe
/// y, si es un predicado, del eje que gobierna.
fn objetivos_validos(
    r: &Loaded,
    lat: &BTreeMap<String, Lattice>,
    props: &Props,
    formas: &Formas,
    out: &mut Vec<Diagnostic>,
) {
    for (dom, pos) in objetivos(r, formas) {
        match dom {
            Dominio::Forma { ref interfaces, .. } => {
                for i in interfaces {
                    if formas.contains_key(i) {
                        continue;
                    }
                    out.push(
                        Diagnostic::new(
                            Code::Oos2001,
                            &r.path,
                            format!("`{i}` no es una interfaz del paquete"),
                        )
                        .at(pos)
                        .help(
                            "un objetivo por forma apunta a un documento `Interface`. Que no                              case con ninguna entidad es otro fallo distinto —`OOS8002`— y solo                              uno de los dos es una errata",
                        ),
                    );
                }
            }
            Dominio::Nombres(nombres) => {
                for n in nombres {
                    if props.contains_key(&n) {
                        continue;
                    }
                    out.push(
                        Diagnostic::new(
                            Code::Oos2005,
                            &r.path,
                            format!("`{n}` no es una propiedad de ninguna entidad del paquete"),
                        )
                        .at(pos),
                    );
                }
            }
            Dominio::Predicado(obj) => {
                for (ret, nivel) in &obj {
                    let Some(l) = lat.get(ret) else {
                        out.push(
                            Diagnostic::new(
                                Code::Oos4003,
                                &r.path,
                                format!("`{ret}` no es un retículo declarado"),
                            )
                            .at(pos)
                            .help(
                                "un objetivo por predicado se escribe sobre un retículo que ya \
                                 existe: no hay lenguaje de objetivos, hay el orden del retículo \
                                 leído al revés",
                            ),
                        );
                        continue;
                    };
                    if l.index(nivel).is_none() {
                        out.push(
                            Diagnostic::new(
                                Code::Oos4003,
                                &r.path,
                                format!("`{nivel}` no es un nivel de `{ret}`"),
                            )
                            .at(pos)
                            .help(format!("los niveles son: {}", l.levels.join(", "))),
                        );
                        continue;
                    }
                    if l.axis == Axis::Integrity {
                        out.push(
                            Diagnostic::new(
                                Code::Oos8006,
                                &r.path,
                                format!(
                                    "`{ret}` es de eje `integrity` y un objetivo no puede apuntarlo"
                                ),
                            )
                            .at(pos)
                            .help(
                                "la monotonía del gobierno corre al revés en ese eje: en \
                                 confidencialidad se gobierna hacia arriba —más sensible, más \
                                 gobierno— y en integridad se gobernaría hacia abajo —menos \
                                 fiable, más gobierno—, así que `atLeast` selecciona justo lo \
                                 contrario de lo que hace falta. Y antes de añadir `atMost` hay \
                                 una pregunta: el remedio natural de la baja integridad es un \
                                 endoso, que es asunto de `Function` y no de un `Ruleset`",
                            ),
                        );
                    }
                }
            }
        }
    }
}

/// Las propiedades que casan con un objetivo. Dentro de un predicado, con la
/// conjunción: la propiedad debe satisfacer **todas** las entradas.
fn selecciona(dom: &Dominio, props: &Props, lat: &BTreeMap<String, Lattice>) -> BTreeSet<String> {
    match dom {
        // Igual que `Nombres` una vez resuelto, y esa igualdad es el resultado:
        // la forma no trae un mecanismo de selección propio, solo otro criterio
        // para llegar al mismo conjunto de propiedades.
        Dominio::Forma { propiedades, .. } => propiedades
            .iter()
            .filter(|n| props.contains_key(*n))
            .cloned()
            .collect(),
        Dominio::Nombres(ns) => ns
            .iter()
            .filter(|n| props.contains_key(*n))
            .cloned()
            .collect(),
        Dominio::Predicado(obj) => props
            .iter()
            .filter(|(_, etiquetas)| {
                obj.iter().all(|(ret, piso)| {
                    etiquetas
                        .get(ret)
                        .and_then(|nivel| lat.get(ret)?.ge(nivel, piso))
                        .unwrap_or(false)
                })
            })
            .map(|(q, _)| q.clone())
            .collect(),
    }
}

/// `OOS8002` — un objetivo que no casa con nada.
fn vacio(r: &Loaded, dom: &Dominio, pos: Pos) -> Diagnostic {
    let escrito = match dom {
        Dominio::Forma { interfaces, .. } => interfaces.join(", "),
        Dominio::Nombres(ns) => ns.join(", "),
        Dominio::Predicado(o) => o
            .iter()
            .map(|(k, v)| format!("{k}: {v}"))
            .collect::<Vec<_>>()
            .join(", "),
    };
    Diagnostic::new(
        Code::Oos8002,
        &r.path,
        format!("el objetivo `{escrito}` no casa con ninguna propiedad"),
    )
    .at(pos)
    .help(
        "casi nunca significa «no hay nada así»: significa que un nivel está mal escrito, o \
         que el retículo cambió y la regla se quedó apuntando a un nombre que ya no existe. \
         Y una regla que no gobierna nada tiene exactamente el mismo aspecto que una que \
         funciona — es el único fallo de este documento que no produce ningún síntoma",
    )
}

// ── Las máscaras · OOS8003 ──────────────────────────────────────────────────

/// El suelo que un objetivo impone sobre un retículo, como índice.
///
/// Para un predicado es el nivel declarado. Para un conjunto de nombres es el
/// **mínimo** de sus etiquetas — y solo si **todas** la tienen: si a una le
/// falta, sobre ella la máscara no baja nada y el descenso deja de ser
/// demostrable.
fn suelo(dom: &Dominio, ret: &str, props: &Props, l: &Lattice) -> Option<usize> {
    match dom {
        Dominio::Predicado(o) => l.index(o.get(ret)?),
        Dominio::Forma { propiedades, .. } => {
            suelo(&Dominio::Nombres(propiedades.clone()), ret, props, l)
        }
        Dominio::Nombres(ns) => ns
            .iter()
            .map(|n| {
                props
                    .get(n)
                    .and_then(|e| e.get(ret))
                    .and_then(|v| l.index(v))
            })
            .collect::<Option<Vec<usize>>>()?
            .into_iter()
            .min(),
    }
}

fn mascaras(
    r: &Loaded,
    lat: &BTreeMap<String, Lattice>,
    props: &Props,
    formas: &Formas,
    out: &mut Vec<Diagnostic>,
) {
    let objs = objetivos(r, formas);
    for m in r.section("masks").map(|n| n.items()).unwrap_or(&[]) {
        let Some(nombre) = m.get("declassifier").and_then(|(_, v)| v.as_str()) else {
            continue;
        };
        // Sin `id` una política no puede nombrarla, y nombrarla es lo que
        // mantiene la definición en un solo sitio (`02-ruleset` §4.1).
        if m.get("id").is_none() {
            out.push(
                Diagnostic::new(
                    Code::Oos1004,
                    &r.path,
                    format!("la máscara `{nombre}` no declara `id`"),
                )
                .at(m.pos())
                .help(
                    "el `id` es lo que permite que una política de Cedar la NOMBRE con \
                     `@oosMask(\"<ruleset>#<id>\")` en vez de declarar otra: dos sitios donde \
                     declarar una máscara serían dos semánticas",
                ),
            );
        }
        if !MASCARAS.contains(&nombre) {
            out.push(
                Diagnostic::new(
                    Code::Oos4006,
                    &r.path,
                    format!("`{nombre}` no es un desclasificador admisible como máscara"),
                )
                .at(m.pos())
                .help(format!(
                    "son: {}. `promote` queda fuera porque SUBE por un retículo de ciclo de \
                     vida, y una máscara tiene que bajar",
                    MASCARAS.join(", ")
                )),
            );
            continue;
        }

        // `to` sobre un `redact` es un campo derivable —redactar deja el valor
        // en el ínfimo del retículo, siempre— y el error es de forma, no de
        // gobierno: `OOS1004`. Es la lección de `OOS7010`, y por eso `OOS8003`
        // se quedó con una sola causa.
        let to = m.get("to").map(|(_, v)| v);
        match (nombre, to) {
            ("redact", Some(v)) => {
                out.push(
                    Diagnostic::new(Code::Oos1004, &r.path, "`redact` no admite `to`")
                        .at(v.pos())
                        .help(
                            "redactar hace desaparecer el valor, luego su salida es siempre el \
                             ínfimo del retículo. Un campo derivable no es declarable (P2)",
                        ),
                );
                continue;
            }
            ("redact", None) => continue,
            (_, None) => {
                out.push(
                    Diagnostic::new(
                        Code::Oos1004,
                        &r.path,
                        format!("la máscara `{nombre}` no declara `to`"),
                    )
                    .at(m.pos())
                    .help(
                        "declarar el nivel resultante es lo que hace comprobable el descenso: \
                         sin él la máscara es una función opaca, que es exactamente lo que \
                         hace inauditables a las de un catálogo",
                    ),
                );
                continue;
            }
            _ => {}
        }
        let Some(destino) = to else { continue };

        for (ret, nivel) in destino
            .entries()
            .iter()
            .filter_map(|(k, v)| Some((k.as_str()?, v.as_str()?)))
        {
            let Some(l) = lat.get(ret) else {
                out.push(
                    Diagnostic::new(
                        Code::Oos4003,
                        &r.path,
                        format!("`{ret}` no es un retículo declarado"),
                    )
                    .at(destino.pos()),
                );
                continue;
            };
            let Some(salida) = l.index(nivel) else {
                out.push(
                    Diagnostic::new(
                        Code::Oos4003,
                        &r.path,
                        format!("`{nivel}` no es un nivel de `{ret}`"),
                    )
                    .at(destino.pos()),
                );
                continue;
            };

            // El suelo más bajo que algún objetivo impone sobre este retículo.
            // Si ninguno lo acota, el descenso no es demostrable, que a efectos
            // de gobierno es lo mismo que no bajar.
            let pisos: Vec<usize> = objs
                .iter()
                .filter_map(|(o, _)| suelo(o, ret, props, l))
                .collect();
            let Some(&piso) = pisos.iter().min() else {
                out.push(
                    Diagnostic::new(
                        Code::Oos8003,
                        &r.path,
                        format!(
                            "la máscara declara bajar `{ret}` y ningún objetivo de este \
                             documento acota ese retículo"
                        ),
                    )
                    .at(destino.pos())
                    .help(
                        "el descenso se comprueba contra el suelo del objetivo: el nivel \
                         declarado si apunta por predicado, o el mínimo de las etiquetas si \
                         apunta por nombre. Sin suelo no hay nada contra qué compararlo, y una \
                         máscara que no baja demostrablemente no es una salvaguarda",
                    ),
                );
                continue;
            };
            if salida >= piso {
                out.push(
                    Diagnostic::new(
                        Code::Oos8003,
                        &r.path,
                        format!(
                            "`{nombre}` produce `{ret}: {nivel}` y el objetivo ya selecciona \
                             desde `{}`",
                            l.levels[piso]
                        ),
                    )
                    .at(destino.pos())
                    .help(
                        "un desclasificador que no baja no es una salvaguarda: es teatro con \
                         coste de cómputo. La comprobación es local —dos niveles declarados— \
                         porque ninguna propiedad seleccionada puede estar por debajo del \
                         suelo que la seleccionó",
                    ),
                );
            }
        }
    }
}

// ── El ámbito de fila · OOS2005 · OOS2001 ───────────────────────────────────

/// Las reclamaciones que la capa de identidad afirma: las claves de `claims` en
/// el `RequestPolicy`.
///
/// Es contra esto —y solo contra esto— que un ámbito puede comparar. Un literal
/// sería un filtro estático, que es lo que hace un `selector`; otra columna
/// sería una comparación entre columnas, que `03-binding` §3.5.1 ya rechazó
/// porque ordena en vez de particionar.
///
/// **Y antes se leían las propiedades de la entidad `principal: true`**, que es
/// el defecto que `06-request` §1.1 ① existe para cerrar: una propiedad la
/// afirma la fuente y está gobernada; una reclamación la firma la identidad y
/// **es lo que gobierna**. Se llaman igual y no son lo mismo.
fn reclamaciones(pkg: &Package) -> BTreeSet<String> {
    pkg.of(Kind::RequestPolicy)
        .next()
        .and_then(|rp| rp.section("claims"))
        .map(|cs| {
            cs.entries()
                .iter()
                .filter_map(|(k, _)| k.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// `OOS4005` · una finalidad que ningún `RequestPolicy` declara.
///
/// # Por qué vuelve un código retirado
///
/// Se retiró razonando que *«la comprobación corresponde al validador de
/// Cedar»*. **Se midió, y Cedar no puede hacerla**: `context.purpose` es un
/// `String`, y un validador comprueba el **tipo**, no el **valor**.
/// `context.purpose == "compenstaion_review"` tipa perfectamente y no casa con
/// nada — el defecto de siempre, en la dirección de siempre.
///
/// Un código retirado por una razón que resulta falsa se reabre. Lo que no se
/// puede es darle otro significado, y no se le da: significa exactamente lo que
/// significaba el día que se escribió.
fn finalidades(pkg: &Package, out: &mut Vec<Diagnostic>) {
    let declaradas: BTreeSet<String> = pkg
        .of(Kind::RequestPolicy)
        .next()
        .and_then(|rp| rp.section("purposes"))
        .map(|ps| {
            ps.items()
                .iter()
                .filter_map(|i| i.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    for (ruta, texto) in pkg.cedar.iter() {
        for pol in crate::cedar::read(texto) {
            for p in &pol.purposes {
                if declaradas.contains(p) {
                    continue;
                }
                out.push(
                    Diagnostic::new(
                        Code::Oos4005,
                        ruta,
                        format!("`{}` limita por la finalidad `{p}`, que nadie declara", pol.id),
                    )
                    .help(if declaradas.is_empty() {
                        "el paquete no declara ningún `RequestPolicy`, así que no hay ninguna \
                         finalidad válida. Una política que limita por una finalidad \
                         inexistente no falla: deja de casar, y el dato queda sin gobernar en \
                         silencio"
                            .to_string()
                    } else {
                        format!(
                            "una finalidad mal escrita no falla: deja de casar. Declaradas: {}",
                            declaradas
                                .iter()
                                .map(|d| format!("`{d}`"))
                                .collect::<Vec<_>>()
                                .join(" · ")
                        )
                    }),
                );
            }
        }
    }
}

/// Un ámbito apunta a una columna que existe y a un atributo que alguien puede
/// rellenar.
///
/// La segunda comprobación es la que importa, y es la de siempre: un ámbito que
/// se compara contra un atributo que **ninguna entidad `principal: true`
/// declara** no falla. El filtro no se construye, o se construye contra la
/// nada, y **la fila queda visible**. Una regla que no recorta tiene exactamente
/// el mismo aspecto que una que recorta.
fn ambitos(pkg: &Package, r: &Loaded, props: &Props, out: &mut Vec<Diagnostic>) {
    let atributos = reclamaciones(pkg);
    for s in r.section("scopes").map(|n| n.items()).unwrap_or(&[]) {
        if let Some((_, v)) = s.get("property")
            && let Some(prop) = v.as_str()
            && !props.contains_key(prop)
        {
            out.push(
                Diagnostic::new(
                    Code::Oos2005,
                    &r.path,
                    format!("el ámbito recorta por `{prop}`, que no es ninguna propiedad"),
                )
                .at(v.pos())
                .help(
                    "`property` es la columna sobre la que se construye el filtro que viaja al \
                     origen. Si no existe, no hay filtro: la fila queda visible",
                ),
            );
        }
        if let Some((_, v)) = s.get("matches")
            && let Some(attr) = v.as_str()
            && !atributos.contains(attr)
        {
            out.push(
                Diagnostic::new(
                    Code::Oos2005,
                    &r.path,
                    format!("`{attr}` no es una reclamación declarada"),
                )
                .at(v.pos())
                .help(if atributos.is_empty() {
                    "el paquete no declara ningún `RequestPolicy`, así que no hay ninguna \
                     reclamación que creer: no hay contra qué comparar. Un ámbito sin lado \
                     derecho no recorta nada, y una fila sin recortar es una fila visible"
                        .to_string()
                } else {
                    format!(
                        "el lado derecho de un ámbito es una reclamación que el principal trae \
                         firmada — es lo que impide que el recorte filtre por algo que él no \
                         sabía. Declaradas: {}",
                        atributos
                            .iter()
                            .map(|a| format!("`{a}`"))
                            .collect::<Vec<_>>()
                            .join(" · ")
                    )
                }),
            );
        }
    }
}

/// `OOS2005` · una política por rol en un paquete donde los roles no pueden
/// llegar.
///
/// Un rol no se declara en ninguna parte —son cadenas que trae la capa de
/// identidad— así que no se puede comprobar **cuál**. Lo que sí se puede, y es
/// lo que importa, es **si pueden llegar**: sin `subject.roles` en el
/// `RequestPolicy`, el principal no pertenece a nada y `principal in
/// Role::"hr_analyst"` **no casa nunca**.
///
/// Está medido, no deducido: un principal sin aristas de padre devuelve `Deny`
/// ante la misma petición que un principal con el rol devuelve `Allow`. Y
/// deniega **en silencio**, que es la forma de fallo de siempre.
fn roles_sin_origen(pkg: &Package, out: &mut Vec<Diagnostic>) {
    let declarada = pkg
        .of(Kind::RequestPolicy)
        .next()
        .and_then(|rp| rp.section("subject"))
        .and_then(|s| s.get("roles"))
        .is_some();
    if declarada {
        return;
    }
    // Y solo aplica si el principal es una ENTIDAD. Sin ninguna `principal:
    // true`, el único principal expresable es `Role`, y entonces la pertenencia
    // **es la identidad**: `Role::"analyst" in Role::"analyst"` es cierto por
    // reflexividad, así que la política casa sin que llegue ninguna
    // reclamación. Es el RBAC degenerado que v1alpha1 siempre admitió.
    //
    // La reclamación hace falta justo cuando el sujeto deja de ser el rol: una
    // `Employee` no es un `Role`, así que su pertenencia tiene que llegar.
    if !pkg
        .entities()
        .any(|e| e.section("principal").and_then(|n| n.as_str()) == Some("true"))
    {
        return;
    }
    for (ruta, texto) in pkg.cedar.iter() {
        for pol in crate::cedar::read(texto) {
            for rol in &pol.roles {
                out.push(
                    Diagnostic::new(
                        Code::Oos2005,
                        ruta,
                        format!("`{}` exige el rol `{rol}`, y nadie declara de dónde vienen los roles", pol.id),
                    )
                    .help(
                        "una pertenencia a rol no es un atributo: llega en una reclamación, y \
                         cuál es lo declara `subject.roles` en el `RequestPolicy`. Sin ella el \
                         principal no pertenece a nada y esta política no casa nunca — no \
                         falla, deniega en silencio",
                    ),
                );
            }
        }
    }
}

/// `@oosScope("<ruleset cualificado>#<id>")` — la misma figura que `@oosMask`,
/// y por la misma razón: la anotación **nombra** un ámbito declarado, no lo
/// declara. Es lo que mantiene la definición en un solo sitio, con dueño.
fn ambito_con_sujeto(pkg: &Package, out: &mut Vec<Diagnostic>) {
    for (ruta, texto) in pkg.cedar.iter() {
        for pol in crate::cedar::read(texto) {
            for referencia in &pol.scopes {
                let resuelve = referencia.split_once('#').is_some_and(|(rs, id)| {
                    pkg.docs
                        .iter()
                        .find(|d| d.kind == Kind::Ruleset && d.qname().as_deref() == Some(rs))
                        .is_some_and(|r| {
                            r.section("scopes")
                                .map(|n| n.items())
                                .unwrap_or(&[])
                                .iter()
                                .any(|s| s.get("id").and_then(|(_, v)| v.as_str()) == Some(id))
                        })
                });
                if !resuelve {
                    out.push(
                        Diagnostic::new(
                            Code::Oos2001,
                            ruta,
                            format!("`@oosScope(\"{referencia}\")` no resuelve a ningún ámbito"),
                        )
                        .help(format!(
                            "la política `{}` nombra un recorte de filas que no existe, así que \
                             no se recorta ninguna: autoriza sobre TODAS las filas de esa \
                             propiedad. La forma es `<ruleset cualificado>#<id de ámbito>`",
                            pol.id
                        )),
                    );
                }
            }
        }
    }
}

// ── La máscara con sujeto · OOS2001 · OOS8003 ───────────────────────────────

/// `@oosMask("<ruleset cualificado>#<id>")` en una política de Cedar.
///
/// # Lo que NO se comprueba, que es la mitad importante
///
/// La cláusula `when` de la política no se evalúa ni se interpreta. Hacerlo
/// sería reimplementar el evaluador de Cedar, que es exactamente lo que **P6**
/// existe para impedir. Lo que se comprueba es estructural —la anotación
/// resuelve, y el ámbito de la política se solapa con el objetivo de la regla—
/// y basta para el fallo que importa: **una máscara que se nombra y no existe,
/// o que se aplica donde su regla no gobierna.**
///
/// Que la política se dispare para el principal correcto lo decide Cedar, en
/// ejecución, y es L3.
fn mascara_con_sujeto(
    pkg: &Package,
    lat: &BTreeMap<String, Lattice>,
    props: &Props,
    sel: &BTreeMap<String, BTreeSet<String>>,
    out: &mut Vec<Diagnostic>,
) {
    for (ruta, texto) in pkg.cedar.iter() {
        for pol in crate::cedar::read(texto) {
            for referencia in &pol.masks {
                let Some((rs, id)) = referencia.split_once('#') else {
                    out.push(
                        Diagnostic::new(
                            Code::Oos2001,
                            ruta,
                            format!(
                                "`@oosMask(\"{referencia}\")` no tiene la forma `<ruleset>#<id>`"
                            ),
                        )
                        .help(
                            "la anotación NOMBRA una máscara declarada en un `Ruleset`; no la \
                             declara. Es lo que mantiene la definición en un solo sitio, con \
                             dueño y con el descenso verificado",
                        ),
                    );
                    continue;
                };
                let regla = pkg
                    .docs
                    .iter()
                    .find(|d| d.kind == Kind::Ruleset && d.qname().as_deref() == Some(rs));
                let existe = regla.is_some_and(|r| {
                    r.section("masks")
                        .map(|n| n.items())
                        .unwrap_or(&[])
                        .iter()
                        .any(|m| m.get("id").and_then(|(_, v)| v.as_str()) == Some(id))
                });
                if !existe {
                    out.push(
                        Diagnostic::new(
                            Code::Oos2001,
                            ruta,
                            format!("`{referencia}` no resuelve a ninguna máscara declarada"),
                        )
                        .help(format!(
                            "la política `{}` nombra una máscara que no existe. Una obligación \
                             que nombra algo inexistente es exactamente lo que mató a XACML",
                            pol.id
                        )),
                    );
                    continue;
                }
                // El ámbito de la política y el objetivo de la regla tienen que
                // solaparse: enmascarar con una regla que no gobierna esa
                // propiedad es una máscara que no se aplica.
                let vacias = sel.get(rs).is_none_or(|ps| {
                    !ps.iter().any(|prop| {
                        props.get(prop).is_some_and(|ets| {
                            pol.labels.iter().any(|g| {
                                g.split_once(':').is_some_and(|(ret, niv)| {
                                    ets.get(ret).is_some_and(|n| {
                                        lat.get(ret).and_then(|l| l.ge(n, niv)) == Some(true)
                                    })
                                })
                            })
                        })
                    })
                });
                if vacias {
                    out.push(
                        Diagnostic::new(
                            Code::Oos8003,
                            ruta,
                            format!("`{referencia}` se aplica donde `{rs}` no gobierna nada"),
                        )
                        .help(
                            "el ámbito de la política y el objetivo de la regla no se solapan: \
                             la máscara se nombra sobre propiedades que esa regla no \
                             selecciona, así que no se aplicaría a ninguna",
                        ),
                    );
                }
            }
        }
    }
}

// ── Las aserciones · OOS8005 ────────────────────────────────────────────────

/// Entidad cualificada → las fuentes físicas a las que se enlaza.
fn fuentes_por_entidad(pkg: &Package) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for b in pkg.docs.iter().filter(|d| d.kind == Kind::Binding) {
        let Some(e) = b.section("targetEntity").and_then(|n| n.as_str()) else {
            continue;
        };
        let Some(ds) = b.section("datasourceRef").and_then(|n| n.as_str()) else {
            continue;
        };
        out.entry(normalize::qualify(
            e,
            b.meta("namespace").and_then(|n| n.as_str()),
        ))
        .or_default()
        .insert(ds.to_string());
    }
    out
}

fn aserciones(
    r: &Loaded,
    seleccionadas: &BTreeSet<String>,
    fuentes: &BTreeMap<String, BTreeSet<String>>,
    out: &mut Vec<Diagnostic>,
) {
    for a in r.section("assertions").map(|n| n.items()).unwrap_or(&[]) {
        if a.get("type").and_then(|(_, v)| v.as_str()) != Some("sql") {
            continue;
        }
        let mut usadas: BTreeSet<&String> = BTreeSet::new();
        for p in seleccionadas {
            let Some((entidad, _)) = p.rsplit_once('.') else {
                continue;
            };
            if let Some(ds) = fuentes.get(entidad) {
                usadas.extend(ds);
            }
        }
        if usadas.len() <= 1 {
            continue;
        }
        let id = a
            .get("id")
            .and_then(|(_, v)| v.as_str())
            .unwrap_or("<sin id>");
        out.push(
            Diagnostic::new(
                Code::Oos8005,
                &r.path,
                format!(
                    "la aserción `sql` `{id}` apunta a propiedades de {} fuentes: {}",
                    usadas.len(),
                    usadas
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )
            .at(a.pos())
            .help(
                "una regla `sql` está atada a un dialecto y el dialecto solo se conoce donde \
                 se declara la fuente: no es portable entre fuentes, y la limitación es de \
                 ODCS, no nuestra. La misma regla escrita como `library` no tiene el problema \
                 porque no tiene dialecto",
            ),
        );
    }
}

// ── Los deberes · OOS2001 ───────────────────────────────────────────────────

fn deberes(pkg: &Package, r: &Loaded, out: &mut Vec<Diagnostic>) {
    let ns = r.meta("namespace").and_then(|n| n.as_str());
    for d in r.section("duties").map(|n| n.items()).unwrap_or(&[]) {
        let Some((_, nodo)) = d.get("call") else {
            continue;
        };
        let Some(nombre) = nodo.as_str() else {
            continue;
        };
        let q = normalize::qualify(nombre, ns);
        let resuelve = pkg
            .docs
            .iter()
            .filter(|x| x.kind == Kind::Function)
            .any(|f| f.qname().as_deref() == Some(q.as_str()));
        if resuelve {
            continue;
        }
        let otro = pkg
            .docs
            .iter()
            .find(|x| x.qname().as_deref() == Some(q.as_str()))
            .map(|x| x.kind.as_str());
        out.push(
            Diagnostic::new(
                Code::Oos2001,
                &r.path,
                match otro {
                    Some(k) => format!("`{q}` es un `{k}`, no una `Function`"),
                    None => format!("`{q}` no resuelve a ninguna `Function`"),
                },
            )
            .at(nodo.pos())
            .help(
                "un deber DEBE nombrar una `Function`, y esa es la única restricción que hace \
                 falta desde el primer día: XACML murió de obligaciones que nombraban deberes \
                 que ningún runtime sabía ejecutar. Una referencia a una función declarada \
                 trae su integridad computada, sus precondiciones y su endoso; un deber en \
                 prosa no trae nada",
            ),
        );
    }
}

// ── La cobertura · OOS8001 ──────────────────────────────────────────────────

/// Las naturalezas que pueden descargar una exigencia.
///
/// `derivation` no está, y no por olvido: **produce contenido, no lo gobierna**.
/// Que la taxonomía de `00-scope` §2 aparezca aquí como vocabulario cerrado es
/// lo que la convierte de descriptiva en normativa.
pub(crate) const NATURALEZAS: &[&str] = &[
    "constraint",
    "authorization",
    "obligation",
    "transformation",
];

/// Qué naturalezas aporta un `Ruleset` a lo que selecciona.
///
/// > Solo cuenta lo que el compilador **puede leer** y lo que **puede fallar**.
///
/// Un aviso no cuenta porque es, por definición, «lo vimos y no paramos nada»;
/// `text` y `custom` no cuentan porque se transportan sin interpretar. Un deber
/// **sí** aparece aquí, y eso cambió al tipar la cobertura: no descarga una
/// exigencia genérica —no puede fallar al compilar— pero **sí** una que pida
/// `obligation` explícitamente, porque entonces lo comprobable es que exista y
/// nombre una `Function`.
fn aporta(pkg: &Package, r: &Loaded) -> BTreeSet<&'static str> {
    let mut out = BTreeSet::new();
    if r.section("assertions")
        .map(|n| n.items())
        .unwrap_or(&[])
        .iter()
        .any(|a| {
            let tipo = a
                .get("type")
                .and_then(|(_, v)| v.as_str())
                .unwrap_or("library");
            let severidad = a
                .get("severity")
                .and_then(|(_, v)| v.as_str())
                .unwrap_or("error");
            asercion_cuenta(tipo, severidad)
        })
    {
        out.insert("constraint");
    }
    if !r
        .section("masks")
        .map(|n| n.items())
        .unwrap_or(&[])
        .is_empty()
    {
        out.insert("transformation");
    }
    let ns = r.meta("namespace").and_then(|n| n.as_str());
    if r.section("duties")
        .map(|n| n.items())
        .unwrap_or(&[])
        .iter()
        .any(|d| {
            d.get("call")
                .and_then(|(_, v)| v.as_str())
                .map(|n| normalize::qualify(n, ns))
                .is_some_and(|q| {
                    pkg.docs
                        .iter()
                        .any(|f| f.kind == Kind::Function && f.qname().as_deref() == Some(&q))
                })
        })
    {
        out.insert("obligation");
    }
    out
}

/// El criterio de una aserción, aislado para poder probarlo: **legible y capaz
/// de fallar**.
fn asercion_cuenta(tipo: &str, severidad: &str) -> bool {
    matches!(tipo, "library" | "sql") && severidad == "error"
}

fn cobertura(
    pkg: &Package,
    lat: &BTreeMap<String, Lattice>,
    props: &Props,
    reglas: &[&Loaded],
    sel: &BTreeMap<String, BTreeSet<String>>,
    out: &mut Vec<Diagnostic>,
) {
    // v1alpha4 · el **tercer origen** de exigencia, junto a los retículos.
    //
    // Un retículo exige por NIVEL —*todo lo que esté en `high` o por encima*—
    // y un concepto exige por CATEGORÍA: por ser lo que es, independientemente
    // de lo que pese. Es la forma que tiene la regulación de clasificar, y sin
    // ella la exigencia depende de que alguien acertara a etiquetar — que es
    // justo el hueco que v1alpha4 existe para cerrar.
    let conceptos = crate::significado::conceptos(pkg);
    let mapeos = crate::significado::mapeos(pkg);
    // **La misma cobertura que se expone**, no una segunda.
    //
    // Esto recomputaba lo suyo aparte, y `cobertura_efectiva` avisaba por
    // escrito de por qué eso no puede ser: *«dos definiciones de "esta propiedad
    // está gobernada" serían dos semánticas»*. Lo eran — la que ROMPE LA
    // COMPILACIÓN y la que se expone para el diff habían divergido, y por eso
    // una política que nombra la propiedad directamente descargaba en una y no
    // en la otra.
    //
    // Y el aviso estaba escrito desde v1alpha3, en el sitio correcto, mirando a
    // la función equivocada.
    let cubierto = cobertura_atribuida(pkg);
    let _ = (reglas, sel);

    for (prop, etiquetas) in props {
        // Lo que exigen todas las clasificaciones que alcanza, en conjunción:
        // si bastara satisfacer una, importar un paquete laxo sería la forma de
        // escapar de uno estricto.
        let mut exigidas: BTreeSet<&str> = BTreeSet::new();
        let mut porque: Vec<String> = Vec::new();
        for (ret, nivel) in etiquetas {
            let Some(l) = lat.get(ret) else { continue };
            for (piso, naturalezas) in &l.requires_governance {
                if l.ge(nivel, piso) != Some(true) {
                    continue;
                }
                porque.push(format!("{ret}:{piso}"));
                for n in naturalezas {
                    if let Some(v) = NATURALEZAS.iter().find(|x| *x == n) {
                        exigidas.insert(v);
                    }
                }
            }
        }
        // Y lo que exige su concepto, si mapea a uno. Se compone con UNIÓN
        // igual que entre retículos: asociativa, conmutativa e idempotente,
        // luego el orden de los orígenes no puede cambiar el resultado. Y solo
        // puede exigir MÁS — no hay forma de descargar una obligación desde
        // aquí, que es lo que impide que importar vocabulario laxo afloje una
        // exigencia local.
        if let Some(q) = mapeos.get(prop)
            && let Some(c) = conceptos.get(q)
            && !c.requiere.is_empty()
        {
            porque.push(format!("el concepto `{q}`"));
            for n in &c.requiere {
                if let Some(v) = NATURALEZAS.iter().find(|x| *x == n) {
                    exigidas.insert(v);
                }
            }
        }

        if exigidas.is_empty() {
            continue;
        }

        let cubiertas: BTreeSet<&str> = cubierto
            .get(prop)
            .map(|m| m.keys().copied().collect())
            .unwrap_or_default();

        let faltan: Vec<&str> = exigidas.difference(&cubiertas).copied().collect();
        if faltan.is_empty() {
            continue;
        }
        let (fichero, pos) = donde(pkg, prop);
        let mut d = Diagnostic::new(
            Code::Oos8001,
            fichero,
            format!(
                "`{prop}` exige {} y no lo tiene",
                faltan
                    .iter()
                    .map(|f| format!("`{f}`"))
                    .collect::<Vec<_>>()
                    .join(" y ")
            ),
        )
        .help(format!(
            "lo exige {}. Y la clase importa: una comprobación de calidad NO descarga lo que              una clasificación pide como política — el fallo no sería que falta una regla,              sería que sobra la equivocada. Ojo también con la salida barata: una aserción              `severity: warning` no cuenta, porque un aviso no descarga la obligación de              gobernar",
            porque.join(", ")
        ));
        if let Some(p) = pos {
            d = d.at(p);
        }
        out.push(d);
    }
}

/// Dónde señalar una propiedad. El fichero de su entidad, y la propiedad si se
/// encuentra — que es lo más cerca que se puede estar de un defecto que no está
/// escrito en ninguna parte.
fn donde(pkg: &Package, prop: &str) -> (std::path::PathBuf, Option<Pos>) {
    let Some((entidad, nombre)) = prop.rsplit_once('.') else {
        return (pkg.root.clone(), None);
    };
    let Some(e) = pkg.entity(entidad) else {
        return (pkg.root.clone(), None);
    };
    let pos = e
        .section("properties")
        .and_then(|n| n.get(nombre))
        .map(|(k, _)| k.pos());
    (e.path.clone(), pos)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reticulo(qname: &str, niveles: &[&str]) -> (String, Lattice) {
        (
            qname.to_string(),
            Lattice {
                qname: qname.to_string(),
                levels: niveles.iter().map(|s| s.to_string()).collect(),
                axis: Axis::Confidentiality,
                requires_governance: BTreeMap::new(),
            },
        )
    }

    fn props(entradas: &[(&str, &[(&str, &str)])]) -> Props {
        entradas
            .iter()
            .map(|(p, ets)| {
                (
                    p.to_string(),
                    ets.iter()
                        .map(|(k, v)| (k.to_string(), v.to_string()))
                        .collect(),
                )
            })
            .collect()
    }

    /// La propiedad de la que sale que solo haga falta un operador: si una
    /// regla se aplica a `high`, **tiene** que aplicarse a `critical`. Una que
    /// dejara de aplicarse cuando el dato es más sensible sería un defecto.
    #[test]
    fn el_gobierno_es_monotono() {
        let lat = BTreeMap::from([reticulo(
            "gdpr.sensitivity",
            &["none", "low", "medium", "high", "critical"],
        )]);
        let p = props(&[
            ("E.baja", &[("gdpr.sensitivity", "low")]),
            ("E.alta", &[("gdpr.sensitivity", "high")]),
            ("E.critica", &[("gdpr.sensitivity", "critical")]),
        ]);
        let obj = Objetivo::from([("gdpr.sensitivity".into(), "medium".into())]);
        let sel = selecciona(&Dominio::Predicado(obj), &p, &lat);
        assert!(sel.contains("E.alta") && sel.contains("E.critica"));
        assert!(!sel.contains("E.baja"));
    }

    /// Dentro de un objetivo, las entradas se conjugan. Es la semántica de
    /// `matchLabels`, y la razón de que sea un mapa y no una lista: una lista
    /// admitiría dos suelos para el mismo retículo, que no significa nada.
    #[test]
    fn el_objetivo_conjuga_sus_entradas() {
        let lat = BTreeMap::from([
            reticulo("gdpr.sensitivity", &["none", "medium", "high"]),
            reticulo("acme.residency", &["global", "eu_only"]),
        ]);
        let p = props(&[
            (
                "E.ambas",
                &[("gdpr.sensitivity", "high"), ("acme.residency", "eu_only")],
            ),
            ("E.solo_una", &[("gdpr.sensitivity", "high")]),
        ]);
        let obj = Objetivo::from([
            ("gdpr.sensitivity".into(), "high".into()),
            ("acme.residency".into(), "eu_only".into()),
        ]);
        let sel = selecciona(&Dominio::Predicado(obj), &p, &lat);
        assert!(sel.contains("E.ambas"));
        assert!(!sel.contains("E.solo_una"));
    }

    /// Una etiqueta que no pertenece al retículo NO selecciona. `ge` devuelve
    /// `None` y no `false` a propósito, y aquí se comprueba que el `None` se
    /// trate como «no casa» en vez de propagarse como un acierto.
    #[test]
    fn un_nivel_desconocido_no_selecciona() {
        let lat = BTreeMap::from([reticulo("gdpr.sensitivity", &["none", "high"])]);
        let p = props(&[("E.p", &[("gdpr.sensitivity", "inventado")])]);
        let obj = Objetivo::from([("gdpr.sensitivity".into(), "none".into())]);
        assert!(selecciona(&Dominio::Predicado(obj), &p, &lat).is_empty());
    }

    /// El dominio escrito selecciona lo que nombra, y solo lo que existe.
    /// Enumerar es seguro porque `OOS8001` impide el silencio, no porque la
    /// enumeración haya dejado de ser enumeración.
    #[test]
    fn el_dominio_escrito_selecciona_lo_que_nombra() {
        let lat = BTreeMap::from([reticulo("gdpr.sensitivity", &["none", "high"])]);
        let p = props(&[
            ("E.a", &[("gdpr.sensitivity", "high")]),
            ("E.b", &[("gdpr.sensitivity", "high")]),
        ]);
        let dom = Dominio::Nombres(vec!["E.a".into(), "E.inexistente".into()]);
        let sel = selecciona(&dom, &p, &lat);
        assert!(sel.contains("E.a"));
        assert!(!sel.contains("E.b"));
        assert_eq!(
            sel.len(),
            1,
            "lo que no existe no se selecciona: es OOS2005"
        );
    }

    /// La regla que impide que la cobertura se vuelva decorativa. Un solo
    /// carácter —`warning` en vez de `error`— separa gobernar de aparentarlo.
    #[test]
    fn un_aviso_no_descarga_la_obligacion() {
        assert!(asercion_cuenta("library", "error"));
        assert!(asercion_cuenta("sql", "error"));
        assert!(!asercion_cuenta("library", "warning"));
        // Se transportan sin interpretar: el compilador no sabe qué afirman.
        assert!(!asercion_cuenta("text", "error"));
        assert!(!asercion_cuenta("custom", "error"));
    }
}
