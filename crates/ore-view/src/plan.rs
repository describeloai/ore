//! El **IR**: álgebra relacional con identidad determinista.
//!
//! Es el órgano del que cuelga todo lo demás. Sin un plan que se pueda mirar no
//! hay reescritura con materializaciones, no hay linaje derivado, no hay reparto
//! por capacidades y no hay incremental — solo hay cadenas de SQL que nadie
//! puede analizar.
//!
//! # Por qué el álgebra está acotada
//!
//! `Lee`, `Proyecta`, `Filtra`, `Une`, `Agrupa`, `Unifica`, `Distingue`,
//! `Limita`. Sin recursión, sin ventanas y sin funciones definidas por el
//! usuario.
//!
//! No es pobreza: es la misma decisión que Cedar tomando un lenguaje sin bucles
//! y que este compilador no teniendo reloj. **La expresividad acotada es lo que
//! hace analizable una cosa**, y lo que se analiza aquí es si una columna
//! `critical` acaba donde no debe. Lo que no quepa entra como [`Opaca`], que
//! cuesta el análisis y **aun así declara qué lee**.
//!
//! # El digest es del significado, no de la escritura
//!
//! Dos personas que escriban la misma vista tienen que obtener el mismo digest,
//! así que la forma canónica **reordena lo que es conmutativo**: los operandos
//! de `Y` y de `O`, las ramas de `Unifica`, las columnas de una proyección y los
//! pares de una junta. Un `Y(a, b)` y un `Y(b, a)` son el mismo predicado, y un
//! plan que dijera lo contrario convertiría el digest en un accidente del orden
//! en que alguien tecleó.
//!
//! Es G1 otra vez, un piso por debajo del bundle, y por eso la forma canónica no
//! se reinventa: se usa la de `ore_core::json`.
//!
//! # No hay literal `Float`, y se dice por qué
//!
//! `OOS6003` prohíbe los decimales sin comillas para que la forma canónica nunca
//! tenga que serializar una coma flotante. Aquí es la misma regla: un decimal es
//! [`Valor::Decimal`] con sus dígitos tal cual se escribieron, y **un `Float`
//! literal no se puede escribir**. Comparar contra un campo `Float` no cabe en
//! v1, y es mejor que no quepa a que quepa dando un digest distinto por máquina.

use ore_core::json::Json;
use ore_core::parse::Node;
use ore_core::types::{Type, parse_type};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

/// Separación de dominio: el digest de un plan no puede coincidir con el de otra
/// cosa que empiece por los mismos bytes.
const MAGIA: &str = "OREPLAN1";

// ── Valores y expresiones ───────────────────────────────────────────────────

/// Un literal. **Sin coma flotante** — está desarrollado en la cabecera.
///
/// `Ord` se deriva y es **estructural**: sirve para que un valor pueda ser clave
/// de un `BTreeMap`, y nada más. El orden **numérico** —el que decide si `10.25 >
/// 9.999`— es [`Valor::comparar`]. Son dos preguntas, y confundirlas haría que un
/// `BTreeMap` ordenara `"10"` antes que `"9"`, que es lo que hace y está bien
/// para lo que es.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Valor {
    Cadena(String),
    Entero(i64),
    /// Los dígitos **tal y como se escribieron**, sin analizar. Es lo mismo que
    /// hace el analizador de YAML conservando el texto crudo de un número: si se
    /// convirtiera a un doble y se volviera a escribir, `0.1` dejaría de ser
    /// `0.1` y el digest dejaría de ser una identidad.
    Decimal(String),
    Booleano(bool),
}

impl Valor {
    pub fn tipo(&self) -> Type {
        Type::Scalar(
            match self {
                Valor::Cadena(_) => "String",
                Valor::Entero(_) => "Integer",
                Valor::Decimal(_) => "Decimal",
                Valor::Booleano(_) => "Boolean",
            }
            .to_string(),
        )
    }

    /// Orden **numérico** entre dos literales del mismo tipo. Tipos distintos no
    /// se comparan, y no compararlos es la respuesta segura: quien lo necesite
    /// tiene que decidir qué hace sin ella.
    ///
    /// Los decimales se comparan **por sus dígitos, sin pasar por un doble**:
    /// `0.10` y `0.1` son iguales, `10.25 > 9.999`, y el signo manda. Lo usan el
    /// View Matcher para implicar predicados y el Delta Compiler para `MIN` y
    /// `MAX`, y por eso vive aquí y no en ninguno de los dos.
    pub fn comparar(&self, otro: &Valor) -> Option<Ordering> {
        Some(match (self, otro) {
            (Valor::Entero(x), Valor::Entero(y)) => x.cmp(y),
            (Valor::Cadena(x), Valor::Cadena(y)) => x.cmp(y),
            (Valor::Booleano(x), Valor::Booleano(y)) => x.cmp(y),
            (Valor::Decimal(x), Valor::Decimal(y)) => decimal(x, y)?,
            _ => return None,
        })
    }

    /// La forma canónica de un decimal: `0.10` → `0.1`, `007` → `7`, `-0` → `0`,
    /// `.5` → `0.5`. Lo demás sale igual.
    ///
    /// **No la aplica el IR**: la forma canónica del plan conserva los dígitos
    /// tal cual se escribieron, porque eso es lo que hace al digest una identidad
    /// de lo escrito. La aplica quien necesite que dos escrituras del mismo número
    /// sean **la misma clave** — agrupar, juntar, deduplicar—, que es el Delta
    /// Compiler.
    pub fn normalizado(self) -> Valor {
        match self {
            Valor::Decimal(d) => Valor::Decimal(match partes_decimal(&d) {
                None => d,
                Some((neg, ent, frac)) => {
                    let ent = if ent.is_empty() { "0" } else { &ent };
                    let cero = ent == "0" && frac.is_empty();
                    let signo = if neg && !cero { "-" } else { "" };
                    if frac.is_empty() {
                        format!("{signo}{ent}")
                    } else {
                        format!("{signo}{ent}.{frac}")
                    }
                }
            }),
            otro => otro,
        }
    }

    fn json(&self) -> Json {
        match self {
            Valor::Cadena(s) => Json::obj([("s", Json::s(s.as_str()))]),
            Valor::Entero(n) => Json::obj([("i", Json::Int(*n))]),
            Valor::Decimal(d) => Json::obj([("d", Json::s(d.as_str()))]),
            Valor::Booleano(b) => Json::obj([("b", Json::Bool(*b))]),
        }
    }

    fn leer(n: &Node) -> Option<Valor> {
        if let Some((_, v)) = n.get("s") {
            return Some(Valor::Cadena(v.as_str()?.to_string()));
        }
        if let Some((_, v)) = n.get("i") {
            return Some(Valor::Entero(v.as_str()?.parse().ok()?));
        }
        if let Some((_, v)) = n.get("d") {
            return Some(Valor::Decimal(v.as_str()?.to_string()));
        }
        if let Some((_, v)) = n.get("b") {
            return Some(Valor::Booleano(v.as_str()? == "true"));
        }
        None
    }
}

/// `(negativo, parte entera sin ceros a la izquierda, parte decimal sin ceros a
/// la derecha)`, o `None` si el texto no es un decimal.
fn partes_decimal(s: &str) -> Option<(bool, String, String)> {
    let (neg, s) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s),
    };
    let (ent, frac) = s.split_once('.').unwrap_or((s, ""));
    if (ent.is_empty() && frac.is_empty())
        || !ent.chars().all(|c| c.is_ascii_digit())
        || !frac.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    Some((
        neg,
        ent.trim_start_matches('0').to_string(),
        frac.trim_end_matches('0').to_string(),
    ))
}

/// Orden exacto de dos decimales escritos como texto, sin pasar por un doble.
fn decimal(a: &str, b: &str) -> Option<Ordering> {
    let (na, ea, fa) = partes_decimal(a)?;
    let (nb, eb, fb) = partes_decimal(b)?;
    let cero_a = ea.is_empty() && fa.is_empty();
    let cero_b = eb.is_empty() && fb.is_empty();
    if cero_a && cero_b {
        return Some(Ordering::Equal);
    }
    let magnitud = || {
        ea.len()
            .cmp(&eb.len())
            .then_with(|| ea.cmp(&eb))
            .then_with(|| {
                let n = fa.len().max(fb.len());
                format!("{fa:0<n$}").cmp(&format!("{fb:0<n$}"))
            })
    };
    Some(match (na && !cero_a, nb && !cero_b) {
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
        (false, false) => magnitud(),
        (true, true) => magnitud().reverse(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Comparador {
    Igual,
    Distinto,
    Menor,
    MenorIgual,
    Mayor,
    MayorIgual,
}

impl Comparador {
    const fn texto(self) -> &'static str {
        match self {
            Comparador::Igual => "eq",
            Comparador::Distinto => "ne",
            Comparador::Menor => "lt",
            Comparador::MenorIgual => "le",
            Comparador::Mayor => "gt",
            Comparador::MayorIgual => "ge",
        }
    }

    fn de(s: &str) -> Option<Comparador> {
        Some(match s {
            "eq" => Comparador::Igual,
            "ne" => Comparador::Distinto,
            "lt" => Comparador::Menor,
            "le" => Comparador::MenorIgual,
            "gt" => Comparador::Mayor,
            "ge" => Comparador::MayorIgual,
            _ => return None,
        })
    }
}

/// **La escapatoria, y su precio.**
///
/// Lo que el álgebra acotada no expresa entra aquí: un fragmento en el dialecto
/// de alguien, que este motor **no analiza**. Pero declara dos cosas, y las dos
/// son obligatorias:
///
/// - **`lee`** — qué campos toca. Es lo que permite que las etiquetas fluyan de
///   forma conservadora por algo que no se entiende: si lee una columna
///   `critical`, lo que produzca es `critical`, sin necesidad de mirar dentro.
/// - **`tipo`** — qué promete devolver. Se cree, y el esquema dice que se cree.
/// - **`determinista`** — si dos evaluaciones sobre los mismos valores dan lo
///   mismo. **Por defecto no**, y el porqué está debajo.
///
/// Es la misma figura que `effects:` en una función: **lo que no se puede
/// analizar se puede acotar, si quien lo escribe declara su superficie.**
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opaca {
    pub dialecto: String,
    pub texto: String,
    pub lee: Vec<String>,
    pub tipo: Type,
    /// **Si dos evaluaciones sobre los mismos valores dan lo mismo.**
    ///
    /// El valor por defecto es `false` — se supone volátil— y es P4:
    /// denegación por defecto. Quien escribe la expresión sabe si su dialecto
    /// tiene un `RANDOM()` dentro; este motor no puede saberlo, y suponer que no
    /// lo tiene sería suponer en la dirección insegura.
    ///
    /// # Por qué existe, y por qué se vio tarde
    ///
    /// Faltaba desde que la opaca existe, y se vio al medir el mantenimiento
    /// incremental: **el determinismo es precondición de la incrementalidad.**
    /// La lista de lo que Snowflake **no** mantiene incrementalmente empieza
    /// por *«UDF volátiles»*, y una vista con una dentro **cae a recomputar
    /// entero**.
    ///
    /// El resto del álgebra no necesita el campo porque no puede ser volátil:
    /// no hay reloj, no hay aleatoriedad y no hay literal `Float` — esa última
    /// se decidió por el digest, no por esto. **La opaca es el único agujero**, y
    /// este campo es lo que lo tapa.
    ///
    /// # No es lo mismo que ser analizable
    ///
    /// Son ortogonales, y confundirlos sería relajar una de las dos.
    /// [`Nodo::analizable`] pregunta *«¿se puede razonar sobre lo que
    /// computa?»* — una opaca **nunca** lo es. Esto pregunta *«¿computa lo
    /// mismo dos veces?»*, y una opaca puede perfectamente.
    pub determinista: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    Campo(String),
    Literal(Valor),
    Compara {
        op: Comparador,
        izquierda: Box<Expr>,
        derecha: Box<Expr>,
    },
    /// `campo IN (…)`. Va aparte de una cadena de `O` porque es lo que un
    /// origen sabe empujar de una pieza — `predicatePushdown: [eq, in]`.
    EnConjunto {
        campo: String,
        valores: Vec<Valor>,
    },
    EsNulo(Box<Expr>),
    Y(Vec<Expr>),
    O(Vec<Expr>),
    No(Box<Expr>),
    Opaca(Opaca),
}

impl Expr {
    pub fn campo(n: &str) -> Expr {
        Expr::Campo(n.to_string())
    }

    /// La clave por la que se ordena lo conmutativo: la propia forma canónica
    /// de la subexpresión. Es total y no hay que inventar un orden.
    fn clave(&self) -> String {
        self.json().jcs()
    }

    fn json(&self) -> Json {
        match self {
            Expr::Campo(c) => Json::obj([("op", Json::s("campo")), ("n", Json::s(c.as_str()))]),
            Expr::Literal(v) => Json::obj([("op", Json::s("lit")), ("v", v.json())]),
            Expr::Compara {
                op,
                izquierda,
                derecha,
            } => Json::obj([
                ("op", Json::s("cmp")),
                ("c", Json::s(op.texto())),
                ("i", izquierda.json()),
                ("d", derecha.json()),
            ]),
            Expr::EnConjunto { campo, valores } => Json::obj([
                ("op", Json::s("in")),
                ("n", Json::s(campo.as_str())),
                // El conjunto se ordena y se deduplica: es un conjunto.
                (
                    "v",
                    Json::Arr(
                        valores
                            .iter()
                            .map(|v| (v.json().jcs(), v))
                            .collect::<BTreeMap<_, _>>()
                            .into_values()
                            .map(Valor::json)
                            .collect(),
                    ),
                ),
            ]),
            Expr::EsNulo(e) => Json::obj([("op", Json::s("nulo")), ("e", e.json())]),
            Expr::Y(v) => Json::obj([("op", Json::s("y")), ("e", conmutativa(v))]),
            Expr::O(v) => Json::obj([("op", Json::s("o")), ("e", conmutativa(v))]),
            Expr::No(e) => Json::obj([("op", Json::s("no")), ("e", e.json())]),
            Expr::Opaca(o) => Json::obj(
                [
                    ("op", Json::s("opaca")),
                    ("dialecto", Json::s(o.dialecto.as_str())),
                    ("texto", Json::s(o.texto.as_str())),
                    (
                        "lee",
                        Json::Arr(
                            o.lee
                                .iter()
                                .collect::<BTreeSet<_>>()
                                .into_iter()
                                .map(|c| Json::s(c.as_str()))
                                .collect(),
                        ),
                    ),
                    ("tipo", Json::s(o.tipo.to_string())),
                ]
                .into_iter()
                // Se escribe **solo cuando es cierto**, y no por ahorrar bytes: así
                // el valor por defecto —volátil— es la ausencia, que es lo que un
                // plan escrito antes de que este campo existiera ya dice.
                .chain(o.determinista.then_some(("determinista", Json::Bool(true))))
                .collect::<Vec<_>>(),
            ),
        }
    }

    /// **Los campos que esta expresión lee**, incluida la superficie declarada
    /// de una opaca.
    ///
    /// Es la primitiva de la que vive el linaje: sin ella, «qué columna influye
    /// en qué» habría que deducirlo en cada análisis por separado. Y que la
    /// opaca aporte su `lee` es lo que hace que un trozo que nadie entiende siga
    /// dejando fluir las etiquetas de forma conservadora.
    pub fn campos_leidos(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        self.recorrer(&mut |e| match e {
            Expr::Campo(c) => {
                out.insert(c.clone());
            }
            Expr::EnConjunto { campo, .. } => {
                out.insert(campo.clone());
            }
            Expr::Opaca(o) => out.extend(o.lee.iter().cloned()),
            _ => {}
        });
        out
    }

    /// Las expresiones opacas que hay debajo, para que un plan pueda decir que
    /// no es del todo analizable **en vez de aparentar que sí**.
    pub fn opacas(&self) -> Vec<&Opaca> {
        let mut out = Vec::new();
        self.recorrer(&mut |e| {
            if let Expr::Opaca(o) = e {
                out.push(o);
            }
        });
        out
    }

    fn recorrer<'a>(&'a self, f: &mut impl FnMut(&'a Expr)) {
        f(self);
        match self {
            Expr::Compara {
                izquierda, derecha, ..
            } => {
                izquierda.recorrer(f);
                derecha.recorrer(f);
            }
            Expr::EsNulo(e) | Expr::No(e) => e.recorrer(f),
            Expr::Y(v) | Expr::O(v) => v.iter().for_each(|e| e.recorrer(f)),
            Expr::Campo(_) | Expr::Literal(_) | Expr::EnConjunto { .. } | Expr::Opaca(_) => {}
        }
    }

    fn leer(n: &Node) -> Result<Expr, String> {
        let op = n
            .get("op")
            .and_then(|(_, v)| v.as_str())
            .ok_or("una expresión sin `op`")?;
        let hijo = |k: &str| -> Result<Expr, String> {
            let (_, v) = n.get(k).ok_or(format!("a `{op}` le falta `{k}`"))?;
            Expr::leer(v)
        };
        let cadena = |k: &str| -> Result<String, String> {
            Ok(n.get(k)
                .and_then(|(_, v)| v.as_str())
                .ok_or(format!("a `{op}` le falta `{k}`"))?
                .to_string())
        };
        let lista = |k: &str| -> Result<Vec<Expr>, String> {
            n.get(k)
                .map(|(_, v)| v.items().iter().map(Expr::leer).collect())
                .unwrap_or_else(|| Err(format!("a `{op}` le falta `{k}`")))
        };
        Ok(match op {
            "campo" => Expr::Campo(cadena("n")?),
            "lit" => Expr::Literal(
                n.get("v")
                    .and_then(|(_, v)| Valor::leer(v))
                    .ok_or("un literal que no se sabe leer")?,
            ),
            "cmp" => Expr::Compara {
                op: Comparador::de(&cadena("c")?).ok_or("un comparador desconocido")?,
                izquierda: Box::new(hijo("i")?),
                derecha: Box::new(hijo("d")?),
            },
            "in" => Expr::EnConjunto {
                campo: cadena("n")?,
                valores: n
                    .get("v")
                    .map(|(_, v)| v.items().iter().filter_map(Valor::leer).collect())
                    .unwrap_or_default(),
            },
            "nulo" => Expr::EsNulo(Box::new(hijo("e")?)),
            "y" => Expr::Y(lista("e")?),
            "o" => Expr::O(lista("e")?),
            "no" => Expr::No(Box::new(hijo("e")?)),
            "opaca" => Expr::Opaca(Opaca {
                dialecto: cadena("dialecto")?,
                texto: cadena("texto")?,
                lee: n
                    .get("lee")
                    .map(|(_, v)| {
                        v.items()
                            .iter()
                            .filter_map(|i| i.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
                tipo: parse_type(&cadena("tipo")?).map_err(|_| "un tipo que no es de OOS")?,
                determinista: n
                    .get("determinista")
                    .and_then(|(_, v)| v.as_str())
                    .is_some_and(|v| v == "true"),
            }),
            otro => return Err(format!("`{otro}` no es una expresión")),
        })
    }
}

/// Lo conmutativo se ordena por su propia forma canónica, y se deduplica: en un
/// `Y`, repetir un operando no dice nada nuevo.
fn conmutativa(v: &[Expr]) -> Json {
    Json::Arr(
        v.iter()
            .map(|e| (e.clave(), e))
            .collect::<BTreeMap<_, _>>()
            .into_values()
            .map(Expr::json)
            .collect(),
    )
}

// ── Nodos ───────────────────────────────────────────────────────────────────

/// La hoja: un objeto de una fuente, **con el esquema que se le declara**.
///
/// Declarado y no leído: aquí no se abre nada. Es la misma frontera que separa
/// `discover --from` de `discover --source`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lectura {
    pub datasource: String,
    pub objeto: String,
    pub campos: BTreeMap<String, Type>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Junta {
    Interna,
    Izquierda,
    Derecha,
    Completa,
}

impl Junta {
    const fn texto(self) -> &'static str {
        match self {
            Junta::Interna => "interna",
            Junta::Izquierda => "izquierda",
            Junta::Derecha => "derecha",
            Junta::Completa => "completa",
        }
    }
    fn de(s: &str) -> Option<Junta> {
        Some(match s {
            "interna" => Junta::Interna,
            "izquierda" => Junta::Izquierda,
            "derecha" => Junta::Derecha,
            "completa" => Junta::Completa,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agregado {
    Cuenta,
    Suma,
    Minimo,
    Maximo,
    Promedio,
}

impl Agregado {
    const fn texto(self) -> &'static str {
        match self {
            Agregado::Cuenta => "cuenta",
            Agregado::Suma => "suma",
            Agregado::Minimo => "min",
            Agregado::Maximo => "max",
            Agregado::Promedio => "prom",
        }
    }
    fn de(s: &str) -> Option<Agregado> {
        Some(match s {
            "cuenta" => Agregado::Cuenta,
            "suma" => Agregado::Suma,
            "min" => Agregado::Minimo,
            "max" => Agregado::Maximo,
            "prom" => Agregado::Promedio,
            _ => return None,
        })
    }
}

/// Un agregado y sobre qué. `sobre: None` solo vale para `Cuenta` — es
/// `count(*)`, y sumar la nada no significa nada.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agregacion {
    pub funcion: Agregado,
    pub sobre: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Nodo {
    Lee(Lectura),
    /// Una hoja que nombra a **otra vista**.
    ///
    /// Es lo que hace que *«un pipeline es una cadena de vistas»* sea una
    /// estructura y no una frase: componer no necesita un concepto nuevo, solo
    /// una hoja que en vez de un objeto de una fuente nombra a un vecino.
    ///
    /// Y no es una rareza nuestra: un `TableScan` de Calcite se apoya en una
    /// tabla que puede ser una vista, y el `ReadRel` de Substrait admite una
    /// `NamedTable`. **Una referencia es una exploración con otro nombre.**
    ///
    /// Después de expandir no queda ninguna, y eso se puede afirmar:
    /// [`Nodo::expandido`].
    Referencia(String),
    Proyecta {
        entrada: Box<Nodo>,
        /// `BTreeMap` y no `Vec`: el orden de las columnas de salida no
        /// significa nada cuando todas tienen nombre, y dos escrituras que solo
        /// difieran en él son el mismo plan.
        campos: BTreeMap<String, Expr>,
    },
    Filtra {
        entrada: Box<Nodo>,
        predicado: Expr,
    },
    Une {
        izquierda: Box<Nodo>,
        derecha: Box<Nodo>,
        tipo: Junta,
        /// Pares `(campo de la izquierda, campo de la derecha)`.
        sobre: Vec<(String, String)>,
    },
    Agrupa {
        entrada: Box<Nodo>,
        por: BTreeSet<String>,
        agregados: BTreeMap<String, Agregacion>,
    },
    Unifica(Vec<Nodo>),
    Distingue(Box<Nodo>),
    Limita {
        entrada: Box<Nodo>,
        n: u64,
    },
}

impl Nodo {
    /// **La forma canónica.** RFC 8785, la misma del bundle.
    pub fn canonico(&self) -> String {
        self.json().jcs()
    }

    /// La identidad del plan.
    pub fn digest(&self) -> String {
        let mut b = MAGIA.as_bytes().to_vec();
        b.extend_from_slice(self.canonico().as_bytes());
        ore_core::digest::de_bytes(&b)
    }

    /// **¿Se puede analizar del todo?** Un plan con una expresión opaca dentro
    /// no lo es, y tiene que decirlo: aparentar que sí es cómo un análisis
    /// pierde su valor sin que nadie lo note.
    pub fn analizable(&self) -> bool {
        self.opacas().is_empty()
    }

    /// **¿Computa este plan lo mismo dos veces?**
    ///
    /// Es otra pregunta que [`Nodo::analizable`], y las dos hacen falta. Un plan
    /// que no es determinista no se puede mantener incrementalmente —el
    /// determinismo es precondición de la incrementalidad— y su resultado no es
    /// reproducible, que es la garantía de la que vive el resto del proyecto.
    ///
    /// Solo una opaca puede romperlo: el resto del álgebra no tiene reloj, ni
    /// aleatoriedad, ni coma flotante.
    pub fn deterministico(&self) -> bool {
        self.opacas().iter().all(|o| o.determinista)
    }

    pub fn opacas(&self) -> Vec<&Opaca> {
        let mut out = Vec::new();
        self.recorrer(&mut |n| {
            match n {
                Nodo::Proyecta { campos, .. } => {
                    campos.values().for_each(|e| out.extend(e.opacas()))
                }
                Nodo::Filtra { predicado, .. } => out.extend(predicado.opacas()),
                _ => {}
            };
        });
        out
    }

    /// Las vistas a las que este plan se refiere **sin haberlas incorporado**.
    pub fn referencias(&self) -> Vec<&str> {
        let mut out = Vec::new();
        self.recorrer(&mut |n| {
            if let Nodo::Referencia(v) = n {
                out.push(v.as_str());
            }
        });
        out
    }

    /// **¿Está el plan entero delante?**
    ///
    /// Un plan con referencias no se puede tipar del todo, no se puede repartir
    /// por capacidades y no se puede ejecutar: le faltan trozos. Poder
    /// preguntarlo es lo que impide que un análisis se haga sobre medio plan y
    /// dé un resultado que parece bueno.
    pub fn expandido(&self) -> bool {
        self.referencias().is_empty()
    }

    /// Las lecturas que hay debajo: las hojas del plan.
    pub fn lecturas(&self) -> Vec<&Lectura> {
        let mut out = Vec::new();
        self.recorrer(&mut |n| {
            if let Nodo::Lee(l) = n {
                out.push(l);
            }
        });
        out
    }

    fn recorrer<'a>(&'a self, f: &mut impl FnMut(&'a Nodo)) {
        f(self);
        for h in self.entradas() {
            h.recorrer(f);
        }
    }

    /// Los nodos de los que este depende. Es lo que hace recorrible el plan sin
    /// repetir el `match` en cada análisis.
    pub fn entradas(&self) -> Vec<&Nodo> {
        match self {
            Nodo::Lee(_) | Nodo::Referencia(_) => Vec::new(),
            Nodo::Proyecta { entrada, .. }
            | Nodo::Filtra { entrada, .. }
            | Nodo::Agrupa { entrada, .. }
            | Nodo::Limita { entrada, .. } => vec![entrada],
            Nodo::Distingue(e) => vec![e],
            Nodo::Une {
                izquierda, derecha, ..
            } => vec![izquierda, derecha],
            Nodo::Unifica(v) => v.iter().collect(),
        }
    }

    fn json(&self) -> Json {
        match self {
            Nodo::Lee(l) => Json::obj([
                ("op", Json::s("lee")),
                ("datasource", Json::s(l.datasource.as_str())),
                ("objeto", Json::s(l.objeto.as_str())),
                (
                    "campos",
                    Json::Obj(
                        l.campos
                            .iter()
                            .map(|(k, t)| (k.clone(), Json::s(t.to_string())))
                            .collect(),
                    ),
                ),
            ]),
            Nodo::Proyecta { entrada, campos } => Json::obj([
                ("op", Json::s("proyecta")),
                ("e", entrada.json()),
                (
                    "campos",
                    Json::Obj(campos.iter().map(|(k, x)| (k.clone(), x.json())).collect()),
                ),
            ]),
            Nodo::Filtra { entrada, predicado } => Json::obj([
                ("op", Json::s("filtra")),
                ("e", entrada.json()),
                ("p", predicado.json()),
            ]),
            Nodo::Une {
                izquierda,
                derecha,
                tipo,
                sobre,
            } => Json::obj([
                ("op", Json::s("une")),
                ("i", izquierda.json()),
                ("d", derecha.json()),
                ("tipo", Json::s(tipo.texto())),
                // Los pares se ordenan —son un conjunto— pero los LADOS no se
                // tocan: `izquierda` no es conmutable con `derecha` en una junta
                // externa, y serlo en la interna no justifica dos reglas.
                (
                    "sobre",
                    Json::Arr(
                        sobre
                            .iter()
                            .collect::<BTreeSet<_>>()
                            .into_iter()
                            .map(|(a, b)| {
                                Json::obj([("i", Json::s(a.as_str())), ("d", Json::s(b.as_str()))])
                            })
                            .collect(),
                    ),
                ),
            ]),
            Nodo::Agrupa {
                entrada,
                por,
                agregados,
            } => Json::obj([
                ("op", Json::s("agrupa")),
                ("e", entrada.json()),
                (
                    "por",
                    Json::Arr(por.iter().map(|c| Json::s(c.as_str())).collect()),
                ),
                (
                    "agregados",
                    Json::Obj(
                        agregados
                            .iter()
                            .map(|(k, a)| {
                                let mut campos = vec![("f", Json::s(a.funcion.texto()))];
                                if let Some(s) = &a.sobre {
                                    campos.push(("sobre", Json::s(s.as_str())));
                                }
                                (k.clone(), Json::obj(campos))
                            })
                            .collect(),
                    ),
                ),
            ]),
            Nodo::Unifica(v) => Json::obj([
                ("op", Json::s("unifica")),
                // Las ramas de una unión son un conjunto: se ordenan por su
                // propio digest, que es un orden total y no hay que inventarlo.
                (
                    "e",
                    Json::Arr(
                        v.iter()
                            .map(|n| (n.canonico(), n))
                            .collect::<BTreeMap<_, _>>()
                            .into_values()
                            .map(Nodo::json)
                            .collect(),
                    ),
                ),
            ]),
            Nodo::Referencia(v) => {
                Json::obj([("op", Json::s("ref")), ("vista", Json::s(v.as_str()))])
            }
            Nodo::Distingue(e) => Json::obj([("op", Json::s("distingue")), ("e", e.json())]),
            Nodo::Limita { entrada, n } => Json::obj([
                ("op", Json::s("limita")),
                ("e", entrada.json()),
                ("n", Json::Int(*n as i64)),
            ]),
        }
    }

    /// Lee un plan de su forma canónica.
    ///
    /// Se apoya en el analizador que ya existe —JSON es YAML— en vez de traer un
    /// segundo, que sería un segundo sitio donde envejecer.
    pub fn leer(texto: &str) -> Result<Nodo, String> {
        let n = ore_core::parse::parse(texto).map_err(|e| format!("no se puede leer: {e:?}"))?;
        Nodo::de(&n)
    }

    fn de(n: &Node) -> Result<Nodo, String> {
        let op = n
            .get("op")
            .and_then(|(_, v)| v.as_str())
            .ok_or("un nodo sin `op`")?;
        let hijo = |k: &str| -> Result<Nodo, String> {
            let (_, v) = n.get(k).ok_or(format!("a `{op}` le falta `{k}`"))?;
            Nodo::de(v)
        };
        let cadena = |k: &str| -> Result<String, String> {
            Ok(n.get(k)
                .and_then(|(_, v)| v.as_str())
                .ok_or(format!("a `{op}` le falta `{k}`"))?
                .to_string())
        };
        Ok(match op {
            "lee" => Nodo::Lee(Lectura {
                datasource: cadena("datasource")?,
                objeto: cadena("objeto")?,
                campos: n
                    .get("campos")
                    .map(|(_, v)| {
                        v.entries()
                            .iter()
                            .filter_map(|(k, t)| {
                                Some((k.as_str()?.to_string(), parse_type(t.as_str()?).ok()?))
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            }),
            "proyecta" => Nodo::Proyecta {
                entrada: Box::new(hijo("e")?),
                campos: n
                    .get("campos")
                    .map(|(_, v)| {
                        v.entries()
                            .iter()
                            .map(|(k, x)| {
                                Ok((
                                    k.as_str().ok_or("una columna sin nombre")?.to_string(),
                                    Expr::leer(x)?,
                                ))
                            })
                            .collect::<Result<BTreeMap<_, _>, String>>()
                    })
                    .transpose()?
                    .unwrap_or_default(),
            },
            "filtra" => Nodo::Filtra {
                entrada: Box::new(hijo("e")?),
                predicado: Expr::leer(n.get("p").ok_or("a `filtra` le falta `p`")?.1)?,
            },
            "une" => Nodo::Une {
                izquierda: Box::new(hijo("i")?),
                derecha: Box::new(hijo("d")?),
                tipo: Junta::de(&cadena("tipo")?).ok_or("un tipo de junta desconocido")?,
                sobre: n
                    .get("sobre")
                    .map(|(_, v)| {
                        v.items()
                            .iter()
                            .filter_map(|p| {
                                Some((
                                    p.get("i")?.1.as_str()?.to_string(),
                                    p.get("d")?.1.as_str()?.to_string(),
                                ))
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            },
            "agrupa" => Nodo::Agrupa {
                entrada: Box::new(hijo("e")?),
                por: n
                    .get("por")
                    .map(|(_, v)| {
                        v.items()
                            .iter()
                            .filter_map(|c| c.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
                agregados: n
                    .get("agregados")
                    .map(|(_, v)| {
                        v.entries()
                            .iter()
                            .filter_map(|(k, a)| {
                                Some((
                                    k.as_str()?.to_string(),
                                    Agregacion {
                                        funcion: Agregado::de(a.get("f")?.1.as_str()?)?,
                                        sobre: a
                                            .get("sobre")
                                            .and_then(|(_, s)| s.as_str())
                                            .map(String::from),
                                    },
                                ))
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            },
            "unifica" => Nodo::Unifica(
                n.get("e")
                    .map(|(_, v)| v.items().iter().map(Nodo::de).collect())
                    .unwrap_or_else(|| Err("a `unifica` le falta `e`".to_string()))?,
            ),
            "ref" => Nodo::Referencia(cadena("vista")?),
            "distingue" => Nodo::Distingue(Box::new(hijo("e")?)),
            "limita" => Nodo::Limita {
                entrada: Box::new(hijo("e")?),
                n: cadena("n")?
                    .parse()
                    .map_err(|_| "un límite que no es entero")?,
            },
            otro => return Err(format!("`{otro}` no es un nodo")),
        })
    }
}

// ── Comprobaciones ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Type {
        parse_type(s).expect("un tipo de OOS")
    }

    fn pedidos() -> Nodo {
        Nodo::Lee(Lectura {
            datasource: "lago".into(),
            objeto: "ventas.pedidos".into(),
            campos: [
                ("id".to_string(), t("Integer")),
                ("total".to_string(), t("Decimal")),
                ("pais".to_string(), t("String")),
            ]
            .into(),
        })
    }

    fn eq(campo: &str, v: Valor) -> Expr {
        Expr::Compara {
            op: Comparador::Igual,
            izquierda: Box::new(Expr::campo(campo)),
            derecha: Box::new(Expr::Literal(v)),
        }
    }

    /// **El criterio de M0.** Dos escrituras del mismo plan dan el mismo digest,
    /// y lo conmutativo no cuenta como una escritura distinta.
    #[test]
    fn el_orden_de_lo_conmutativo_no_cambia_el_digest() {
        let filtra = |p: Expr| Nodo::Filtra {
            entrada: Box::new(pedidos()),
            predicado: p,
        };
        let a = filtra(Expr::Y(vec![
            eq("pais", Valor::Cadena("ES".into())),
            eq("id", Valor::Entero(7)),
        ]));
        let b = filtra(Expr::Y(vec![
            eq("id", Valor::Entero(7)),
            eq("pais", Valor::Cadena("ES".into())),
        ]));
        assert_eq!(a.digest(), b.digest());
        assert!(a.digest().starts_with("sha256:"));
    }

    /// Y repetir un operando en un `Y` no dice nada nuevo, así que tampoco
    /// cambia el predicado.
    #[test]
    fn repetir_un_operando_no_es_otro_predicado() {
        let uno = Expr::Y(vec![eq("id", Valor::Entero(7))]);
        let dos = Expr::Y(vec![eq("id", Valor::Entero(7)), eq("id", Valor::Entero(7))]);
        assert_eq!(uno.json().jcs(), dos.json().jcs());
    }

    /// Las ramas de una unión son un conjunto: se ordenan por su propia forma
    /// canónica, que es un orden total y no hay que inventarlo.
    #[test]
    fn el_orden_de_las_ramas_de_una_union_tampoco() {
        let rama = |p: &str| Nodo::Filtra {
            entrada: Box::new(pedidos()),
            predicado: eq("pais", Valor::Cadena(p.into())),
        };
        assert_eq!(
            Nodo::Unifica(vec![rama("ES"), rama("PT")]).digest(),
            Nodo::Unifica(vec![rama("PT"), rama("ES")]).digest()
        );
    }

    /// Pero los lados de una junta **no** son conmutables: en una externa no lo
    /// son, y tener dos reglas según el tipo de junta sería peor que no tener
    /// ninguna.
    #[test]
    fn los_lados_de_una_junta_no_se_conmutan() {
        let lineas = Nodo::Lee(Lectura {
            datasource: "lago".into(),
            objeto: "ventas.lineas".into(),
            campos: [("id_pedido".to_string(), t("Integer"))].into(),
        });
        let une = |i: Nodo, d: Nodo| Nodo::Une {
            izquierda: Box::new(i),
            derecha: Box::new(d),
            tipo: Junta::Izquierda,
            sobre: vec![("id".into(), "id_pedido".into())],
        };
        assert_ne!(
            une(pedidos(), lineas.clone()).digest(),
            une(lineas, pedidos()).digest()
        );
    }

    /// **El criterio de M0.** El plan va y vuelve por su forma canónica.
    #[test]
    fn el_plan_va_y_vuelve() {
        let p = Nodo::Limita {
            entrada: Box::new(Nodo::Distingue(Box::new(Nodo::Proyecta {
                entrada: Box::new(Nodo::Filtra {
                    entrada: Box::new(pedidos()),
                    predicado: Expr::No(Box::new(Expr::EsNulo(Box::new(Expr::campo("pais"))))),
                }),
                campos: [
                    ("pais".to_string(), Expr::campo("pais")),
                    ("cuanto".to_string(), Expr::campo("total")),
                ]
                .into(),
            }))),
            n: 100,
        };
        let vuelta = Nodo::leer(&p.canonico()).expect("se lee");
        assert_eq!(vuelta.canonico(), p.canonico());
        assert_eq!(vuelta.digest(), p.digest());
    }

    /// Y con las formas que más fácil se pierden al serializar: el conjunto, el
    /// agregado sin columna y el decimal con sus dígitos tal cual.
    #[test]
    fn las_formas_dificiles_tambien_van_y_vuelven() {
        let p = Nodo::Agrupa {
            entrada: Box::new(Nodo::Filtra {
                entrada: Box::new(pedidos()),
                predicado: Expr::Y(vec![
                    Expr::EnConjunto {
                        campo: "pais".into(),
                        valores: vec![Valor::Cadena("ES".into()), Valor::Cadena("PT".into())],
                    },
                    Expr::Compara {
                        op: Comparador::Mayor,
                        izquierda: Box::new(Expr::campo("total")),
                        derecha: Box::new(Expr::Literal(Valor::Decimal("0.10".into()))),
                    },
                ]),
            }),
            por: ["pais".to_string()].into(),
            agregados: [
                (
                    "n".to_string(),
                    Agregacion {
                        funcion: Agregado::Cuenta,
                        sobre: None,
                    },
                ),
                (
                    "suma".to_string(),
                    Agregacion {
                        funcion: Agregado::Suma,
                        sobre: Some("total".into()),
                    },
                ),
            ]
            .into(),
        };
        let vuelta = Nodo::leer(&p.canonico()).expect("se lee");
        assert_eq!(vuelta.canonico(), p.canonico());

        // **Lo que la vuelta conserva es la forma canónica, no la escritura.**
        //
        // Lo dijo ejecutarlo: la estructura que sale NO es igual a la que
        // entró, porque los operandos del `Y` salen ordenados. Es lo correcto y
        // es la propiedad entera de este módulo —el digest es del significado,
        // no de la escritura—, así que lo que hay que exigir es que de la forma
        // canónica en adelante ya no se mueva nada.
        assert_eq!(
            Nodo::leer(&vuelta.canonico()).expect("se lee"),
            vuelta,
            "leer la forma canónica tiene que ser un punto fijo"
        );

        // Y el decimal conserva sus dígitos: `0.10` no es `0.1`.
        assert!(p.canonico().contains("0.10"), "{}", p.canonico());
    }

    /// **El criterio de M0.** Un plan con una expresión opaca **lo dice**.
    /// Aparentar que se analiza es cómo un análisis pierde su valor sin que
    /// nadie lo note.
    #[test]
    fn un_plan_con_expresion_opaca_lo_dice() {
        let limpio = Nodo::Filtra {
            entrada: Box::new(pedidos()),
            predicado: eq("pais", Valor::Cadena("ES".into())),
        };
        assert!(limpio.analizable());

        let opaco = Nodo::Proyecta {
            entrada: Box::new(pedidos()),
            campos: [(
                "cuando".to_string(),
                Expr::Opaca(Opaca {
                    dialecto: "bigquery".into(),
                    texto: "PARSE_DATE(fmt, pais)".into(),
                    lee: vec!["pais".into()],
                    tipo: t("Date"),
                    determinista: false,
                }),
            )]
            .into(),
        };
        assert!(!opaco.analizable());
        assert_eq!(opaco.opacas().len(), 1);
        assert_eq!(opaco.opacas()[0].dialecto, "bigquery");
        // Y se ve en la forma canónica: quien lea el plan no tiene que
        // ejecutarlo para saber que hay un trozo que nadie mira.
        assert!(opaco.canonico().contains("opaca"), "{}", opaco.canonico());
        // Y va y vuelve, con su superficie declarada intacta.
        let vuelta = Nodo::leer(&opaco.canonico()).expect("se lee");
        assert_eq!(vuelta.opacas()[0].lee, vec!["pais".to_string()]);
    }

    /// Lo que no es un plan se dice, en vez de leerse a medias.
    #[test]
    fn lo_que_no_es_un_plan_no_se_intenta_interpretar() {
        assert!(Nodo::leer("{}").is_err());
        assert!(Nodo::leer(r#"{"op":"volar"}"#).is_err());
        // Un nodo al que le falta su entrada.
        assert!(Nodo::leer(r#"{"op":"distingue"}"#).is_err());
    }

    /// Las hojas se recuperan: es lo que M4 necesita para repartir el plan por
    /// fuente, y lo que hace que el recorrido no se escriba dos veces.
    #[test]
    fn las_hojas_del_plan_se_recuperan() {
        let lineas = Nodo::Lee(Lectura {
            datasource: "sap".into(),
            objeto: "ventas.lineas".into(),
            campos: [("id_pedido".to_string(), t("Integer"))].into(),
        });
        let p = Nodo::Unifica(vec![pedidos(), lineas]);
        let hojas = p.lecturas();
        assert_eq!(hojas.len(), 2);
        let mut fuentes: Vec<&str> = hojas.iter().map(|l| l.datasource.as_str()).collect();
        fuentes.sort();
        assert_eq!(fuentes, ["lago", "sap"]);
    }

    /// El orden numérico es exacto y no pasa por un doble: `0.10` es `0.1`, el
    /// signo manda, y tipos distintos **no se comparan**.
    #[test]
    fn el_orden_de_los_decimales_es_exacto() {
        let d = |s: &str| Valor::Decimal(s.into());
        assert_eq!(d("0.10").comparar(&d("0.1")), Some(Ordering::Equal));
        assert_eq!(d("9.999").comparar(&d("10.25")), Some(Ordering::Less));
        assert_eq!(d("-3").comparar(&d("2")), Some(Ordering::Less));
        assert_eq!(d("-0").comparar(&d("0.0")), Some(Ordering::Equal));
        assert_eq!(d("-1.5").comparar(&d("-1.25")), Some(Ordering::Less));
        assert_eq!(d("1").comparar(&Valor::Entero(1)), None, "tipos distintos");
        assert_eq!(
            Valor::Entero(3).comparar(&Valor::Entero(7)),
            Some(Ordering::Less)
        );
    }

    /// Y la forma canónica hace que dos escrituras del mismo número sean la
    /// misma clave — sin tocar el IR, que conserva los dígitos como se
    /// escribieron.
    #[test]
    fn la_forma_canonica_de_un_decimal_no_depende_de_como_se_escribio() {
        let d = |s: &str| Valor::Decimal(s.into());
        for (escrito, canonico) in [
            ("0.10", "0.1"),
            ("007", "7"),
            ("-0", "0"),
            ("-0.00", "0"),
            (".5", "0.5"),
            ("-3.50", "-3.5"),
            ("10", "10"),
        ] {
            assert_eq!(d(escrito).normalizado(), d(canonico), "{escrito}");
        }
        assert_eq!(d("abc").normalizado(), d("abc"));
        assert_eq!(Valor::Entero(5).normalizado(), Valor::Entero(5));
    }

    /// **Una opaca se supone volátil mientras no diga lo contrario.**
    ///
    /// Es P4: quien la escribe sabe si su dialecto tiene un `RANDOM()` dentro;
    /// este motor no puede saberlo, y suponer que no lo tiene sería suponer en
    /// la dirección insegura.
    #[test]
    fn una_opaca_se_supone_volatil_mientras_no_diga_lo_contrario() {
        let con = |determinista| Nodo::Proyecta {
            entrada: Box::new(pedidos()),
            campos: [(
                "x".to_string(),
                Expr::Opaca(Opaca {
                    dialecto: "bigquery".into(),
                    texto: "algo(pais)".into(),
                    lee: vec!["pais".into()],
                    tipo: t("String"),
                    determinista,
                }),
            )]
            .into(),
        };
        assert!(!con(false).deterministico(), "sin declarar, volátil");
        assert!(con(true).deterministico());

        // Y sin ninguna opaca, el resto del álgebra no puede ser volátil: no
        // hay reloj, ni aleatoriedad, ni coma flotante.
        assert!(pedidos().deterministico());
    }

    /// **Determinista y analizable son preguntas distintas**, y confundirlas
    /// relajaría una de las dos. Una opaca NUNCA es analizable, y puede
    /// perfectamente ser determinista.
    #[test]
    fn ser_determinista_no_es_ser_analizable() {
        let p = Nodo::Filtra {
            entrada: Box::new(pedidos()),
            predicado: Expr::Opaca(Opaca {
                dialecto: "bigquery".into(),
                texto: "REGEXP_CONTAINS(pais, r'^E')".into(),
                lee: vec!["pais".into()],
                tipo: t("Boolean"),
                determinista: true,
            }),
        };
        assert!(p.deterministico(), "una regex no es volátil");
        assert!(!p.analizable(), "y sigue sin poder razonarse sobre ella");
    }

    /// El campo **no se escribe cuando es falso**, y no por ahorrar bytes: así
    /// el valor por defecto —volátil— es la ausencia, que es lo que un plan
    /// escrito antes de que el campo existiera ya dice.
    ///
    /// Y declarar pureza **sí** cambia el digest: son dos declaraciones
    /// distintas, y una de las dos se puede mantener incrementalmente.
    #[test]
    fn declarar_pureza_cambia_el_plan_y_no_declararla_no_deja_rastro() {
        let con = |determinista| Nodo::Filtra {
            entrada: Box::new(pedidos()),
            predicado: Expr::Opaca(Opaca {
                dialecto: "bigquery".into(),
                texto: "algo(pais)".into(),
                lee: vec!["pais".into()],
                tipo: t("Boolean"),
                determinista,
            }),
        };
        assert!(!con(false).canonico().contains("determinista"));
        assert!(con(true).canonico().contains(r#""determinista":true"#));
        assert_ne!(con(false).digest(), con(true).digest());

        // Y las dos van y vuelven.
        for d in [false, true] {
            let vuelta = Nodo::leer(&con(d).canonico()).expect("se lee");
            assert_eq!(vuelta.opacas()[0].determinista, d);
            assert_eq!(vuelta.canonico(), con(d).canonico());
        }
    }
}
