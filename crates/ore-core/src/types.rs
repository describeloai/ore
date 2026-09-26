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

/// **Si pasar de `de` a `a` ENSANCHA el conjunto de valores aceptados.**
///
/// # Por qué existe, y por qué es normativa
///
/// `diff` trataba cualquier cambio de tipo como `OOS5002`, cuyo texto es *«tipo
/// **estrechado**»*. El veredicto no siempre era falso, pero **la atribución
/// sí**: `Integer → Decimal` no estrecha nada, y un código que dice algo que no
/// pasó es lo que este árbol persigue.
///
/// Y la dirección ya estaba decidida un piso más abajo: la rama de los `enum`
/// dice *«retirar valores de un enum. **Añadirlos no rompe a quien lee**»*.
/// Esto es la misma frase sobre el escalar, y por eso ensanchar **no emite**.
///
/// # El único par, y eso es el hallazgo
///
/// Normativa: `02-entity` §3.4.
///
/// Sobre los diez escalares del conjunto cerrado la relación tiene **un
/// elemento**: `Integer → Decimal`. Todo entero cabe exacto en un decimal, y
/// aquí el decimal es exacto porque no hay coma flotante en ninguna parte.
///
/// Lo que vale de esta función es **lo que deja fuera**, porque son los pares
/// que alguien va a querer añadir «obviamente» algún día:
///
/// | par | por qué NO ensancha |
/// |---|---|
/// | `Integer`/`Decimal` → `Float` | pierde exactitud. `68400.50` no tiene representación exacta en binario, y es la regla más dura de este árbol |
/// | `Date` → `DateTime` | una fecha no es un instante. Ponerle una hora es **inventarla** |
/// | `DateTime` → `DateTimeTz` | ídem con la zona |
/// | cualquiera → `String` | una cadena **representa** el valor, no lo contiene: el contrato de lectura cambia entero |
/// | cualquiera → `Opaque` | `Opaque` es *«no lo modelamos»*. Ir ahí no ensancha el dominio: **retira el gobierno** |
/// | `Boolean` → `Integer` | eso es elegir una codificación, no ampliar un dominio |
///
/// Y **entre decimales con precisión** (02-entity §3.4, 2026-09-26):
/// `Decimal<p₁, s₁>` ensancha a `Decimal<p₂, s₂>` cuando no pierde ninguna
/// cifra por ningún lado —`s₂ ≥ s₁` y `p₂ − s₂ ≥ p₁ − s₁`—, e `Integer` a
/// `Decimal<p, s>` solo con `p − s ≥ 19`, las cifras del entero de 64 bits.
/// Declarar o retirar la precisión (`Decimal` ↔ `Decimal<p, s>`) no ensancha
/// en ninguna dirección.
///
/// Un paramétrico de unidad —`Money<EUR,2>`— lo clasifica `OOS5010`, que es
/// más específico y va antes.
pub fn ensancha(de: &str, a: &str) -> bool {
    match (parse_type(de), parse_type(a)) {
        (
            Ok(Type::Decimal {
                precision: p1,
                escala: s1,
            }),
            Ok(Type::Decimal {
                precision: p2,
                escala: s2,
            }),
        ) => s2 >= s1 && p2 - s2 >= p1 - s1,
        (Ok(Type::Scalar(i)), Ok(Type::Decimal { precision, escala })) if i == "Integer" => {
            precision - escala >= CIFRAS_DEL_ENTERO
        }
        _ => matches!((de, a), ("Integer", "Decimal")),
    }
}

/// Las cifras del mayor entero de 64 bits, `9 223 372 036 854 775 807`: el
/// `Integer` de las copias (0032) visto como decimal es `Decimal<19, 0>`.
pub const CIFRAS_DEL_ENTERO: u8 = 19;

/// El techo de la precisión: el de Iceberg y Parquet (`decimal128`).
pub const PRECISION_MAXIMA: u8 = 38;

/// **El supertipo de dos decimales** (02-entity §3.5): la mayor escala y la
/// mayor parte entera. Una unión, las dos ramas de un `CASE`, los dos lados de
/// una comparación. `None` si no cabe en [`PRECISION_MAXIMA`]: no hay decimal
/// exacto donde quepan los dos, y redondear sería elegir en silencio qué
/// cifras se pierden.
pub fn supertipo_decimal(a: (u8, u8), b: (u8, u8)) -> Option<(u8, u8)> {
    let escala = a.1.max(b.1);
    let enteras = (a.0 - a.1).max(b.0 - b.1);
    let precision = enteras.checked_add(escala)?;
    (precision <= PRECISION_MAXIMA).then_some((precision, escala))
}

/// `sum(Decimal<p, s>)` → `Decimal<38, s>` (02-entity §3.5).
pub fn suma_decimal(t: (u8, u8)) -> (u8, u8) {
    (PRECISION_MAXIMA, t.1)
}

/// `avg(Decimal<p, s>)` → `Decimal<38, max(s, 9)>` (02-entity §3.5): la de
/// BigQuery. La de DuckDB, `DOUBLE`, cambia un exacto por un binario.
pub fn media_decimal(t: (u8, u8)) -> (u8, u8) {
    (PRECISION_MAXIMA, t.1.max(9))
}

/// El conjunto cerrado, para quien tenga que OFRECERLO.
///
/// `ore review` pregunta por el tipo de una columna que el lector no supo
/// traducir, y las opciones que ofrece son estas. Se exponen en vez de copiarse
/// porque una lista escrita a mano en otro fichero envejece en silencio la
/// primera vez que el conjunto crezca — que es la figura que este repositorio
/// lleva encontrando.
pub fn escalares() -> &'static [&'static str] {
    ESCALARES
}

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
    /// `Decimal<p, s>` (02-entity §3.2): el decimal con su precisión y su
    /// escala, `1 ≤ p ≤ 38` y `0 ≤ s ≤ p`. `Decimal` a secas es
    /// `Scalar("Decimal")` y dice otra cosa: la precisión no se declaró.
    Decimal {
        precision: u8,
        escala: u8,
    },
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

/// Cómo se escribe un tipo ya analizado.
///
/// Existía `parse_type` y no existía la vuelta, y eso es un agujero en cuanto
/// alguien tiene que **enseñar** un tipo o meterlo en una forma canónica. La
/// invariante es que `parse_type(t.to_string()) == t` para todo tipo que se
/// haya analizado, y hay una prueba que la ejerce sobre el conjunto cerrado
/// entero — si no, habría dos escrituras del mismo tipo y ninguna diría cuál
/// manda.
impl std::fmt::Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Type::Scalar(s) | Type::Imported(s) => f.write_str(s),
            Type::List(s) => write!(f, "list<{s}>"),
            Type::Decimal { precision, escala } => write!(f, "Decimal<{precision}, {escala}>"),
            Type::Parametric {
                ctor,
                unit,
                precision,
            } => write!(f, "{ctor}<{unit}, {precision}>"),
        }
    }
}

/// Por qué un tipo no es válido.
#[derive(Debug)]
pub enum TypeError {
    Desconocido,
    /// Un paramétrico al que le falta la unidad o la precisión.
    Incompleto(String),
    /// `Decimal<p, s>` con los números fuera de rango (OOS3002).
    DecimalFueraDeRango(String),
}

pub fn parse_type(s: &str) -> Result<Type, TypeError> {
    if let Some(inner) = s.strip_prefix("list<").and_then(|r| r.strip_suffix('>')) {
        return if ESCALARES.contains(&inner) {
            Ok(Type::List(inner.to_string()))
        } else {
            Err(TypeError::Desconocido)
        };
    }

    if let Some(resto) = s.strip_prefix("Decimal<") {
        return decimal(resto);
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

/// Lo que sigue a `Decimal<`. Sin cerrar, o sin los dos números, está
/// incompleto; con los dos pero fuera de rango, también es `OOS3002`, con la
/// causa dicha.
fn decimal(resto: &str) -> Result<Type, TypeError> {
    let Some(args) = resto.strip_suffix('>') else {
        return Err(TypeError::Incompleto("Decimal".into()));
    };
    let partes: Vec<&str> = args.split(',').map(str::trim).collect();
    let [p, e] = partes[..] else {
        return Err(TypeError::Incompleto("Decimal".into()));
    };
    let (Ok(p), Ok(e)) = (p.parse::<u16>(), e.parse::<u16>()) else {
        return Err(TypeError::Incompleto("Decimal".into()));
    };
    if p == 0 || p > u16::from(PRECISION_MAXIMA) {
        return Err(TypeError::DecimalFueraDeRango(format!(
            "la precisión va de 1 a {PRECISION_MAXIMA}, el techo de Iceberg y Parquet \
             (`decimal128`), y es {p}. Lo que no cabe viaja como `String` con su `physicalType`"
        )));
    }
    if e > p {
        return Err(TypeError::DecimalFueraDeRango(format!(
            "la escala ({e}) no puede pasar de la precisión ({p}): son las cifras detrás de la \
             coma, y no hay más que las que hay"
        )));
    }
    Ok(Type::Decimal {
        precision: p as u8,
        escala: e as u8,
    })
}

pub fn check(pkg: &Package) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    tipos_de_conceptos(pkg, &mut out);
    // OOS3006 vive en su propio modulo porque necesita el paquete entero: hay
    // que leer la `primaryKey` de OTRA entidad. Es de esta familia igualmente.
    crate::enlace_compuesto::comprobar(pkg, &mut out);
    // Las columnas de una `Table` y de un `Dataset` también declaran tipo, y un
    // `Decimal<40, 2>` ahí se ignoraba en silencio: `tipos_de_columnas` descarta
    // lo que no analiza, y la columna quedaba sin tipo — texto para quien la
    // leyera. Desde `Decimal<p, s>` (0032 T4) un tipo mal escrito en una columna
    // dice su código igual que en una propiedad.
    for d in pkg
        .docs
        .iter()
        .filter(|d| matches!(d.kind, Kind::Table | Kind::Dataset))
    {
        tipos_de_seccion(d, "columns", &mut out);
    }
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
    for d in pkg.docs.iter().filter(|d| d.kind == Kind::Concept) {
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
    tipos_de_seccion(e, "properties", out);
}

/// OOS3001/OOS3002 sobre el `type` de cada entrada de una sección: las
/// `properties` de una entidad, las `columns` de una tabla o de un dataset.
fn tipos_de_seccion(e: &Loaded, seccion: &str, out: &mut Vec<Diagnostic>) {
    let Some(ps) = e.section(seccion) else {
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
            Err(TypeError::Incompleto(ctor)) if ctor == "Decimal" => out.push(
                Diagnostic::new(
                    Code::Oos3002,
                    &e.path,
                    format!("`{s}` está incompleto: `Decimal<p, s>` lleva precisión y escala"),
                )
                .at(t.pos())
                .help(
                    "escríbelo como `Decimal<10, 2>` —diez cifras, dos detrás de la coma—, o \
                     `Decimal` a secas si la precisión no se sabe. En estilo flow va entre \
                     comillas: la coma lo partiría",
                ),
            ),
            Err(TypeError::DecimalFueraDeRango(porque)) => out.push(
                Diagnostic::new(Code::Oos3002, &e.path, format!("`{s}` · {porque}")).at(t.pos()),
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

    /// **El ensanche, y sobre todo lo que deja fuera.**
    ///
    /// La mitad de arriba es una linea; la que vale es la de abajo, porque son
    /// los pares que alguien va a querer meter «obviamente». Que `Decimal` a
    /// `Float` NO ensanche es la regla mas dura de este arbol dicha en el
    /// sistema de tipos: convertir es perder, y `68400.50` no tiene
    /// representacion exacta en binario.
    #[test]
    fn el_ensanche_tiene_un_par_y_el_resto_esta_argumentado() {
        assert!(ensancha("Integer", "Decimal"));

        // La contraria estrecha, y por eso sigue siendo `OOS5002`.
        assert!(!ensancha("Decimal", "Integer"));
        // Exactitud.
        assert!(!ensancha("Integer", "Float"));
        assert!(!ensancha("Decimal", "Float"));
        // Inventar la hora, e inventar la zona.
        assert!(!ensancha("Date", "DateTime"));
        assert!(!ensancha("DateTime", "DateTimeTz"));
        // Representar no es contener.
        assert!(!ensancha("Integer", "String"));
        // `Opaque` no amplia el dominio: retira el gobierno.
        assert!(!ensancha("Integer", "Opaque"));
        assert!(!ensancha("String", "Opaque"));
        // Una codificacion no es un ensanche.
        assert!(!ensancha("Boolean", "Integer"));
        // Y nada ensancha a si mismo: un cambio que no cambia no es un cambio.
        for e in escalares() {
            assert!(!ensancha(e, e), "{e} ensancha a si mismo");
        }
    }

    /// El censo: la relación solo habla del vocabulario cerrado. Si alguien
    /// añade un escalar y cree que ensancha a otro, tiene que decirlo aquí —y
    /// esta prueba le recuerda que la lista de pares es exhaustiva.
    #[test]
    fn el_ensanche_no_nombra_nada_que_no_sea_un_escalar() {
        let pares: usize = escalares()
            .iter()
            .flat_map(|a| escalares().iter().map(move |b| (a, b)))
            .filter(|(a, b)| ensancha(a, b))
            .count();
        assert_eq!(
            pares, 1,
            "la relación de ensanche cambió de tamaño: dilo en `91-versioning` §5.1 antes"
        );
    }

    /// **`parse_type(t.to_string()) == t`**, sobre el conjunto cerrado entero y
    /// las tres formas compuestas.
    ///
    /// Se ejerce sobre `escalares()` y no sobre una lista escrita a mano aqui
    /// por lo mismo que `escalares()` existe: una copia envejece en silencio la
    /// primera vez que el conjunto crezca.
    #[test]
    fn todo_tipo_que_se_analiza_se_puede_volver_a_escribir() {
        let mut casos: Vec<String> = escalares().iter().map(|s| (*s).to_string()).collect();
        casos.push("list<String>".into());
        casos.push("Money<EUR, 2>".into());
        casos.push("Quantity<km, 1>".into());
        casos.push("Decimal<38, 9>".into());
        casos.push("Decimal<1, 0>".into());
        casos.push("iso.CountryAlpha2".into());
        for c in casos {
            let t = parse_type(&c).unwrap_or_else(|_| panic!("`{c}` tenia que analizar"));
            assert_eq!(t.to_string(), c, "la vuelta no coincide");
            assert_eq!(parse_type(&t.to_string()).unwrap(), t, "no es idempotente");
        }
        // Y la forma laxa converge en la canonica: `Quantity<km,1>` sin espacio
        // se escribe con espacio, que es la unica de las dos que se emite.
        assert_eq!(
            parse_type("Quantity<km,1>").unwrap().to_string(),
            "Quantity<km, 1>"
        );
    }

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

    /// `Decimal<p, s>` (02-entity §3.2): los bordes del rango analizan, y lo
    /// de fuera es OOS3002 —incompleto o fuera de rango—, nunca «desconocido».
    #[test]
    fn el_decimal_con_precision_y_su_rango() {
        assert_eq!(
            parse_type("Decimal<10,2>").unwrap(),
            Type::Decimal {
                precision: 10,
                escala: 2
            }
        );
        assert_eq!(
            parse_type("Decimal<10,2>").unwrap().to_string(),
            "Decimal<10, 2>"
        );
        assert!(parse_type("Decimal<38, 38>").is_ok());
        assert!(parse_type("Decimal<1, 0>").is_ok());
        assert_eq!(
            parse_type("Decimal").unwrap(),
            Type::Scalar("Decimal".into())
        );
        for (t, incompleto) in [
            ("Decimal<10>", true),
            ("Decimal<10, dos>", true),
            ("Decimal<10, 2", true),
            ("Decimal<10, 2, 1>", true),
            ("Decimal<2, 4>", false),
            ("Decimal<0, 0>", false),
            ("Decimal<40, 2>", false),
            ("Decimal<300, 2>", false),
        ] {
            match parse_type(t) {
                Err(TypeError::Incompleto(c)) if incompleto => assert_eq!(c, "Decimal"),
                Err(TypeError::DecimalFueraDeRango(_)) if !incompleto => {}
                otro => panic!("`{t}` dio {otro:?}"),
            }
        }
    }

    /// 02-entity §3.4: sin perder cifras por ningún lado; declarar o retirar la
    /// precisión no ensancha.
    #[test]
    fn el_ensanche_entre_decimales() {
        assert!(ensancha("Decimal<10, 2>", "Decimal<12, 2>"));
        assert!(ensancha("Decimal<10, 2>", "Decimal<12, 4>"));
        assert!(
            !ensancha("Decimal<12, 4>", "Decimal<12, 2>"),
            "pierde decimales"
        );
        assert!(
            !ensancha("Decimal<12, 2>", "Decimal<12, 4>"),
            "pierde enteras"
        );
        assert!(ensancha("Integer", "Decimal<19, 0>"));
        assert!(ensancha("Integer", "Decimal<38, 9>"));
        assert!(!ensancha("Integer", "Decimal<10, 2>"));
        assert!(!ensancha("Decimal", "Decimal<38, 9>"));
        assert!(!ensancha("Decimal<38, 9>", "Decimal"));
        assert!(ensancha("Integer", "Decimal"), "el par de siempre sigue");
    }

    /// 02-entity §3.5, fila a fila.
    #[test]
    fn el_decimal_en_las_operaciones() {
        assert_eq!(supertipo_decimal((10, 2), (12, 4)), Some((12, 4)));
        assert_eq!(supertipo_decimal((10, 2), (5, 5)), Some((13, 5)));
        assert_eq!(
            supertipo_decimal((38, 0), (38, 38)),
            None,
            "76 cifras no caben"
        );
        assert_eq!(supertipo_decimal((38, 9), (19, 0)), Some((38, 9)));
        assert_eq!(suma_decimal((10, 2)), (38, 2));
        assert_eq!(media_decimal((10, 2)), (38, 9));
        assert_eq!(media_decimal((38, 12)), (38, 12));
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
