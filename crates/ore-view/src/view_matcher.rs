//! **View Matcher**: ¿contesta esta materialización a este plan, y qué hay que
//! hacerle encima?
//!
//! *View matching* es el nombre que le dan Oracle y SQL Server; Calcite lo llama
//! *query rewriting*. Es el problema de **answering queries using views**
//! (Halevy, VLDB J. 2001), y conviene tenerlo delante porque tiene una frontera
//! de decidibilidad estudiada: la *containment* de consultas conjuntivas es
//! NP-completa y la *determinacy* es indecidible. Así que esta pieza **no
//! implementa «la reescritura»: implementa el subconjunto decidible y dice cuál
//! es.**
//!
//! # El subconjunto de hoy
//!
//! Planes y materializaciones de forma **select-project sobre una hoja**:
//! `Proyecta`, `Filtra` y `Lee`, anidados como sea. Sin juntas, sin agregados,
//! sin unión, sin `Distingue`, sin `Limita`. Lo que no quepa sale con
//! [`NoContesta::FueraDelSubconjunto`] **nombrando el operador**, no con un
//! `false`.
//!
//! Son tres de los cuatro *checks* de Oracle y Calcite:
//!
//! | | Check | Aquí |
//! |---|---|---|
//! | 2 | **data sufficiency** | cada columna que el plan produce se deriva de lo que la materialización expone — o de una **constante** que su predicado fija, o de una **clase de equivalencia** del plan |
//! | 3 | **predicate subsumption** | el predicado del plan **implica** el de la materialización; lo que sobra es la **compensation** |
//! | — | **label seal** | la clasificación de la materialización **se hereda, no se recalcula** |
//!
//! El 1 —*join compatibility*— y el 4 —*aggregate computability*— son los dos
//! siguientes, y cada uno tiene su criterio y su prueba.
//!
//! # La implicación, y hasta dónde llega
//!
//! Un conyunto de la materialización está implicado por el plan si el plan lo
//! contiene **tal cual**, o si es una comparación simple —columna contra
//! literal— y los conyuntos del plan sobre esa columna la **acotan**: `total >=
//! 100` implica `total > 0`. Se razona por columna, con igualdades, desigualdades,
//! cotas e `IN`. Fuera de eso —disyunciones, negaciones compuestas, opacas
//! distintas— **no hay implicación**, y no haberla es la respuesta segura.
//!
//! Y una opaca **idéntica** en los dos lados solo se da por implicada si es
//! **determinista**: un `RANDOM() > 0.5` escrito dos veces son dos filtros
//! distintos.
//!
//! # El label seal, que no lo tiene nadie
//!
//! Una vista que filtró por `nif` produce un resultado `critical` aunque `nif`
//! **no esté entre sus columnas** — lo dice el Lineage Analyzer con una arista
//! `INDIRECT`. Si al reescribir se recalculase el linaje sobre la tabla
//! materializada, esa columna no aparecería y la etiqueta desaparecería con
//! ella. Sería el fallo que M2 y M3 existen para impedir, entrando por aquí.
//!
//! Así que las raíces del plan reescrito son las columnas de la tabla
//! materializada, **y sus etiquetas son las que la materialización trae
//! selladas**. El Flow Checker corre sobre eso: la misma regla, sin una segunda
//! copia. Romper el sello es recalcular.

use crate::filter_tree::Materializacion;
use crate::flow::{Clasificacion, Etiquetas, comprobar};
use crate::lineage::{Raiz, linaje};
use crate::plan::{Comparador, Expr, Lectura, Nodo, Valor};
use std::cmp::Ordering;
use std::collections::BTreeMap;

/// Lo que sale de un cotejo que contesta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rewrite {
    /// Qué materialización contesta.
    pub from: String,
    /// El plan reescrito: lee la tabla materializada, con la compensación encima
    /// y la proyección del plan original traducida.
    pub plan: Nodo,
    /// Los conyuntos que la materialización **no** garantiza y hay que aplicar
    /// encima. Vacío cuando contesta tal cual.
    pub compensation: Vec<Expr>,
    /// La clasificación de cada columna de salida, **heredada del sello** de la
    /// materialización. Nunca recalculada desde los orígenes.
    pub label_seal: BTreeMap<String, Etiquetas>,
}

/// Por qué no contesta. Cada variante es un motivo distinto, y el que importa
/// para una caché parcial es el tercero: dice **qué** falta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoContesta {
    /// Uno de los dos lados usa un operador fuera del subconjunto de hoy.
    FueraDelSubconjunto { operador: &'static str },
    /// Uno de los dos lados todavía nombra una vista.
    SinExpandir { vista: String },
    /// No leen la misma hoja. El Filter Tree lo impide, y aquí se comprueba
    /// igual porque esta pieza también se puede llamar sin él.
    HojaDistinta,
    /// El plan necesita una columna que la materialización no expone, ni fija
    /// como constante, ni el plan iguala a otra que sí.
    ColumnaNoDerivable { columna: String },
    /// La materialización filtra más de lo que el plan pide: le faltarían filas.
    /// Va en caja porque una `Expr` es grande y un `Err` grande se copia en cada
    /// retorno.
    PredicadoNoSubsumido { conyunto: Box<Expr> },
    /// Una expresión opaca no se reescribe: su texto nombra columnas por dentro
    /// y este motor no lo lee.
    OpacaNoReescribible,
}

impl NoContesta {
    pub fn como_texto(&self) -> String {
        match self {
            NoContesta::FueraDelSubconjunto { operador } => format!(
                "`{operador}` está fuera del subconjunto que este cotejo decide: hoy son \
                 proyecciones y filtros sobre una hoja"
            ),
            NoContesta::SinExpandir { vista } => {
                format!("todavía nombra a `{vista}`: hay que expandir antes de cotejar")
            }
            NoContesta::HojaDistinta => "no leen la misma hoja".into(),
            NoContesta::ColumnaNoDerivable { columna } => format!(
                "`{columna}` no se deriva de la materialización: no la expone, su predicado no \
                 la fija y el plan no la iguala a otra que sí"
            ),
            NoContesta::PredicadoNoSubsumido { conyunto } => format!(
                "la materialización filtra por {conyunto:?} y el plan no lo implica: le \
                 faltarían filas"
            ),
            NoContesta::OpacaNoReescribible => {
                "una expresión opaca no se reescribe: su texto nombra columnas por dentro".into()
            }
        }
    }
}

// ── La forma normalizada ────────────────────────────────────────────────────

/// Un plan select-project sobre una hoja, aplanado: qué hoja, qué conyuntos
/// sobre sus columnas, y qué produce en función de sus columnas.
struct Spj<'a> {
    hoja: &'a Lectura,
    conyuntos: Vec<Expr>,
    salida: BTreeMap<String, Expr>,
}

fn spj(n: &Nodo) -> Result<Spj<'_>, NoContesta> {
    match n {
        Nodo::Referencia(v) => Err(NoContesta::SinExpandir { vista: v.clone() }),
        Nodo::Lee(l) => Ok(Spj {
            hoja: l,
            conyuntos: Vec::new(),
            salida: l
                .campos
                .keys()
                .map(|c| (c.clone(), Expr::Campo(c.clone())))
                .collect(),
        }),
        Nodo::Filtra { entrada, predicado } => {
            let mut s = spj(entrada)?;
            let p = sustituir(predicado, &s.salida)?;
            s.conyuntos.extend(conyuntos(&p));
            Ok(s)
        }
        Nodo::Proyecta { entrada, campos } => {
            let s = spj(entrada)?;
            let salida = campos
                .iter()
                .map(|(k, x)| Ok((k.clone(), sustituir(x, &s.salida)?)))
                .collect::<Result<_, _>>()?;
            Ok(Spj {
                hoja: s.hoja,
                conyuntos: s.conyuntos,
                salida,
            })
        }
        Nodo::Une { .. } => Err(NoContesta::FueraDelSubconjunto { operador: "une" }),
        Nodo::Agrupa { .. } => Err(NoContesta::FueraDelSubconjunto { operador: "agrupa" }),
        Nodo::Unifica(_) => Err(NoContesta::FueraDelSubconjunto {
            operador: "unifica",
        }),
        Nodo::Distingue(_) => Err(NoContesta::FueraDelSubconjunto {
            operador: "distingue",
        }),
        Nodo::Limita { .. } => Err(NoContesta::FueraDelSubconjunto { operador: "limita" }),
    }
}

/// Reescribe una expresión con cada `Campo(c)` sustituido por `mapa[c]`.
///
/// Una opaca solo pasa si cada columna que lee se sustituye **por sí misma**: su
/// texto nombra columnas por dentro, y renombrarlas fuera dejaría el texto
/// apuntando a nombres que ya no existen.
fn sustituir(x: &Expr, mapa: &BTreeMap<String, Expr>) -> Result<Expr, NoContesta> {
    let de = |c: &str| -> Result<Expr, NoContesta> {
        mapa.get(c).cloned().ok_or(NoContesta::ColumnaNoDerivable {
            columna: c.to_string(),
        })
    };
    Ok(match x {
        Expr::Campo(c) => de(c)?,
        Expr::Literal(v) => Expr::Literal(v.clone()),
        Expr::Compara {
            op,
            izquierda,
            derecha,
        } => Expr::Compara {
            op: *op,
            izquierda: Box::new(sustituir(izquierda, mapa)?),
            derecha: Box::new(sustituir(derecha, mapa)?),
        },
        Expr::EnConjunto { campo, valores } => match de(campo)? {
            Expr::Campo(c) => Expr::EnConjunto {
                campo: c,
                valores: valores.clone(),
            },
            // `IN` sobre algo que ya no es una columna no cabe en la forma.
            _ => return Err(NoContesta::OpacaNoReescribible),
        },
        Expr::EsNulo(e) => Expr::EsNulo(Box::new(sustituir(e, mapa)?)),
        Expr::Y(v) => Expr::Y(
            v.iter()
                .map(|e| sustituir(e, mapa))
                .collect::<Result<_, _>>()?,
        ),
        Expr::O(v) => Expr::O(
            v.iter()
                .map(|e| sustituir(e, mapa))
                .collect::<Result<_, _>>()?,
        ),
        Expr::No(e) => Expr::No(Box::new(sustituir(e, mapa)?)),
        Expr::Opaca(o) => {
            for c in &o.lee {
                if mapa.get(c) != Some(&Expr::Campo(c.clone())) {
                    return Err(NoContesta::OpacaNoReescribible);
                }
            }
            x.clone()
        }
    })
}

fn conyuntos(x: &Expr) -> Vec<Expr> {
    match x {
        Expr::Y(v) => v.iter().flat_map(conyuntos).collect(),
        otro => vec![otro.clone()],
    }
}

// ── Literales: orden exacto, sin coma flotante ──────────────────────────────

/// Compara dos literales **del mismo tipo**. Tipos distintos no se comparan, y
/// no compararlos es la respuesta segura: la implicación que dependiera de ello
/// no se afirma.
fn comparar(a: &Valor, b: &Valor) -> Option<Ordering> {
    Some(match (a, b) {
        (Valor::Entero(x), Valor::Entero(y)) => x.cmp(y),
        (Valor::Cadena(x), Valor::Cadena(y)) => x.cmp(y),
        (Valor::Booleano(x), Valor::Booleano(y)) => x.cmp(y),
        (Valor::Decimal(x), Valor::Decimal(y)) => decimal(x, y)?,
        _ => return None,
    })
}

/// Orden exacto de dos decimales escritos como texto, **sin pasar por un
/// doble**: `0.10` y `0.1` son iguales, `10.25 > 9.999`, y el signo manda.
fn decimal(a: &str, b: &str) -> Option<Ordering> {
    fn partes(s: &str) -> Option<(bool, String, String)> {
        let (neg, s) = match s.strip_prefix('-') {
            Some(r) => (true, r),
            None => (false, s),
        };
        let (ent, frac) = s.split_once('.').unwrap_or((s, ""));
        if ent.is_empty() && frac.is_empty()
            || !ent.chars().all(|c| c.is_ascii_digit())
            || !frac.chars().all(|c| c.is_ascii_digit())
        {
            return None;
        }
        let ent = ent.trim_start_matches('0');
        let frac = frac.trim_end_matches('0');
        Some((neg, ent.to_string(), frac.to_string()))
    }
    let (na, ea, fa) = partes(a)?;
    let (nb, eb, fb) = partes(b)?;
    // Ceros: `-0` y `0` son el mismo número.
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

// ── Implicación por columna ─────────────────────────────────────────────────

/// Un conyunto simple: `columna op literal`, o `columna IN (…)`.
enum Simple<'a> {
    Cmp(&'a str, Comparador, &'a Valor),
    En(&'a str, &'a [Valor]),
}

fn simple(x: &Expr) -> Option<Simple<'_>> {
    match x {
        Expr::Compara {
            op,
            izquierda,
            derecha,
        } => match (&**izquierda, &**derecha) {
            (Expr::Campo(c), Expr::Literal(v)) => Some(Simple::Cmp(c, *op, v)),
            // `lit op col` es `col op' lit` con el comparador vuelto.
            (Expr::Literal(v), Expr::Campo(c)) => Some(Simple::Cmp(
                c,
                match op {
                    Comparador::Menor => Comparador::Mayor,
                    Comparador::MenorIgual => Comparador::MayorIgual,
                    Comparador::Mayor => Comparador::Menor,
                    Comparador::MayorIgual => Comparador::MenorIgual,
                    otro => *otro,
                },
                v,
            )),
            _ => None,
        },
        Expr::EnConjunto { campo, valores } => Some(Simple::En(campo, valores)),
        _ => None,
    }
}

/// Lo que un conjunto de conyuntos afirma sobre **una** columna.
#[derive(Default)]
struct Hechos<'a> {
    igual: Option<&'a Valor>,
    distintos: Vec<&'a Valor>,
    /// `(cota, inclusiva)`.
    inferior: Option<(&'a Valor, bool)>,
    superior: Option<(&'a Valor, bool)>,
    en: Option<&'a [Valor]>,
}

fn hechos<'a>(base: &'a [Expr], columna: &str) -> Hechos<'a> {
    let mut h = Hechos::default();
    for x in base {
        match simple(x) {
            Some(Simple::Cmp(c, op, v)) if c == columna => match op {
                Comparador::Igual => h.igual = Some(v),
                Comparador::Distinto => h.distintos.push(v),
                Comparador::Menor => ajustar(&mut h.superior, v, false, Ordering::Less),
                Comparador::MenorIgual => ajustar(&mut h.superior, v, true, Ordering::Less),
                Comparador::Mayor => ajustar(&mut h.inferior, v, false, Ordering::Greater),
                Comparador::MayorIgual => ajustar(&mut h.inferior, v, true, Ordering::Greater),
            },
            Some(Simple::En(c, vs)) if c == columna => h.en = Some(vs),
            _ => {}
        }
    }
    h
}

/// Se queda con la cota **más apretada**. `mejor` es el orden que hace mejor a
/// una cota: `Less` para superiores, `Greater` para inferiores.
fn ajustar<'a>(cota: &mut Option<(&'a Valor, bool)>, v: &'a Valor, incl: bool, mejor: Ordering) {
    *cota = match *cota {
        None => Some((v, incl)),
        Some((actual, actual_incl)) => match comparar(v, actual) {
            Some(o) if o == mejor => Some((v, incl)),
            Some(Ordering::Equal) => Some((actual, actual_incl && incl)),
            _ => Some((actual, actual_incl)),
        },
    };
}

/// ¿Todo valor que cumple `hechos` cumple también `col op lit`?
fn implica_cmp(h: &Hechos, op: Comparador, lit: &Valor) -> bool {
    use Comparador::*;
    let cmp = |a: &Valor| comparar(a, lit);
    // Con una igualdad, la columna ES ese valor: se evalúa.
    if let Some(v) = h.igual {
        return matches!(
            (op, cmp(v)),
            (Igual, Some(Ordering::Equal))
                | (Distinto, Some(Ordering::Less | Ordering::Greater))
                | (Menor, Some(Ordering::Less))
                | (MenorIgual, Some(Ordering::Less | Ordering::Equal))
                | (Mayor, Some(Ordering::Greater))
                | (MayorIgual, Some(Ordering::Greater | Ordering::Equal))
        );
    }
    // Con un `IN`, todos los valores del conjunto tienen que cumplirlo.
    if let Some(vs) = h.en {
        let todos = |f: &dyn Fn(Ordering) -> bool| {
            !vs.is_empty() && vs.iter().all(|v| cmp(v).is_some_and(f))
        };
        return match op {
            Igual => vs.iter().all(|v| cmp(v) == Some(Ordering::Equal)) && !vs.is_empty(),
            Distinto => todos(&|o| o != Ordering::Equal),
            Menor => todos(&|o| o == Ordering::Less),
            MenorIgual => todos(&|o| o != Ordering::Greater),
            Mayor => todos(&|o| o == Ordering::Greater),
            MayorIgual => todos(&|o| o != Ordering::Less),
        };
    }
    match op {
        Igual => false,
        Distinto => {
            h.distintos.iter().any(|v| cmp(v) == Some(Ordering::Equal))
                || h.inferior.is_some_and(|(v, incl)| {
                    matches!(cmp(v), Some(Ordering::Greater))
                        || (!incl && cmp(v) == Some(Ordering::Equal))
                })
                || h.superior.is_some_and(|(v, incl)| {
                    matches!(cmp(v), Some(Ordering::Less))
                        || (!incl && cmp(v) == Some(Ordering::Equal))
                })
        }
        Menor => h.superior.is_some_and(|(v, incl)| {
            matches!(cmp(v), Some(Ordering::Less)) || (!incl && cmp(v) == Some(Ordering::Equal))
        }),
        MenorIgual => h
            .superior
            .is_some_and(|(v, _)| matches!(cmp(v), Some(Ordering::Less | Ordering::Equal))),
        Mayor => h.inferior.is_some_and(|(v, incl)| {
            matches!(cmp(v), Some(Ordering::Greater)) || (!incl && cmp(v) == Some(Ordering::Equal))
        }),
        MayorIgual => h
            .inferior
            .is_some_and(|(v, _)| matches!(cmp(v), Some(Ordering::Greater | Ordering::Equal))),
    }
}

/// ¿Implica `base` a `objetivo`?
///
/// Igualdad literal primero —con la salvedad de la opaca volátil—, y si no,
/// razonamiento por columna para comparaciones simples. Fuera de eso, **no**, y
/// no es la respuesta segura por casualidad: afirmar una implicación que no se
/// sabe demostrar es servir filas que no debían salir.
fn implica(base: &[Expr], objetivo: &Expr) -> bool {
    if base.contains(objetivo) {
        return match objetivo {
            Expr::Opaca(o) => o.determinista,
            _ => true,
        };
    }
    match simple(objetivo) {
        Some(Simple::Cmp(c, op, v)) => implica_cmp(&hechos(base, c), op, v),
        Some(Simple::En(c, vs)) => {
            let h = hechos(base, c);
            if let Some(v) = h.igual {
                return vs.iter().any(|w| comparar(v, w) == Some(Ordering::Equal));
            }
            if let Some(mios) = h.en {
                return mios
                    .iter()
                    .all(|m| vs.iter().any(|w| comparar(m, w) == Some(Ordering::Equal)));
            }
            false
        }
        None => false,
    }
}

// ── El cotejo ───────────────────────────────────────────────────────────────

/// Con qué se puede escribir cada columna de la hoja **en términos de la tabla
/// materializada**: la columna que la expone tal cual, la constante que su
/// predicado fija, o —vía una igualdad del plan— otra columna que sí se expone.
fn disponibles(m: &Spj, p: &Spj) -> BTreeMap<String, Expr> {
    let mut out: BTreeMap<String, Expr> = BTreeMap::new();
    // 1 · expuestas tal cual.
    for (salida, x) in &m.salida {
        if let Expr::Campo(c) = x {
            out.entry(c.clone()).or_insert(Expr::Campo(salida.clone()));
        }
    }
    // 2 · fijadas por el predicado de la materialización.
    for x in &m.conyuntos {
        if let Some(Simple::Cmp(c, Comparador::Igual, v)) = simple(x) {
            out.entry(c.to_string()).or_insert(Expr::Literal(v.clone()));
        }
    }
    // 3 · igualadas en el plan a algo disponible. Una pasada basta para una
    // igualdad; encadenar dos sería un cierre, y hoy no hace falta.
    for x in &p.conyuntos {
        if let Expr::Compara {
            op: Comparador::Igual,
            izquierda,
            derecha,
        } = x
            && let (Expr::Campo(a), Expr::Campo(b)) = (&**izquierda, &**derecha)
        {
            if !out.contains_key(a)
                && let Some(d) = out.get(b).cloned()
            {
                out.insert(a.clone(), d);
            } else if !out.contains_key(b)
                && let Some(d) = out.get(a).cloned()
            {
                out.insert(b.clone(), d);
            }
        }
    }
    out
}

/// Un conyunto que, traducido, compara una cosa consigo misma no dice nada.
fn tautologia(x: &Expr) -> bool {
    matches!(
        x,
        Expr::Compara {
            op: Comparador::Igual,
            izquierda,
            derecha
        } if izquierda == derecha
    )
}

/// **El cotejo.**
///
/// `sello` es la clasificación que la materialización trae puesta, como
/// [`Clasificacion`] cuyas raíces son **las columnas de su tabla**. Sin sello
/// —`Clasificacion::default()`— el plan reescrito sale sin etiquetas, que es
/// distinto de salir con las etiquetas mal.
pub fn cotejar(
    plan: &Nodo,
    m: &Materializacion,
    sello: &Clasificacion,
) -> Result<Rewrite, NoContesta> {
    let p = spj(plan)?;
    let q = spj(&m.plan)?;
    if p.hoja.datasource != q.hoja.datasource || p.hoja.objeto != q.hoja.objeto {
        return Err(NoContesta::HojaDistinta);
    }

    // ── 3 · predicate subsumption: todo lo que la materialización filtra, el
    // plan también lo filtra. Si no, le faltarían filas.
    for c in &q.conyuntos {
        if !implica(&p.conyuntos, c) {
            return Err(NoContesta::PredicadoNoSubsumido {
                conyunto: Box::new(c.clone()),
            });
        }
    }

    // ── 2 · data sufficiency, a través de lo disponible.
    let disp = disponibles(&q, &p);
    let salida: BTreeMap<String, Expr> = p
        .salida
        .iter()
        .map(|(k, x)| Ok((k.clone(), sustituir(x, &disp)?)))
        .collect::<Result<_, _>>()?;

    // ── La compensation: lo que el plan pide y la materialización no garantiza.
    let mut compensation = Vec::new();
    for c in &p.conyuntos {
        if !implica(&q.conyuntos, c) {
            let t = sustituir(c, &disp)?;
            if !tautologia(&t) {
                compensation.push(t);
            }
        }
    }

    // ── El plan reescrito: la tabla, la compensation, la proyección.
    let mut nodo = Nodo::Lee(m.tabla.clone());
    if !compensation.is_empty() {
        nodo = Nodo::Filtra {
            entrada: Box::new(nodo),
            predicado: if compensation.len() == 1 {
                compensation[0].clone()
            } else {
                Expr::Y(compensation.clone())
            },
        };
    }
    let nodo = Nodo::Proyecta {
        entrada: Box::new(nodo),
        campos: salida,
    };

    // ── El label seal: el Flow Checker sobre el plan reescrito, con las raíces
    // que son las columnas selladas de la tabla. Nada se recalcula desde los
    // orígenes porque los orígenes no están en este plan.
    let label_seal = match linaje(&nodo) {
        Ok(l) => comprobar(&l, sello, &Etiquetas::new()).efectivas,
        Err(_) => BTreeMap::new(),
    };

    Ok(Rewrite {
        from: m.nombre.clone(),
        plan: nodo,
        compensation,
        label_seal,
    })
}

/// El sello de una materialización, construido de su clasificación efectiva
/// **por columna de salida**: es lo que el Flow Checker devolvió cuando se
/// construyó, colgado de las columnas de su tabla.
pub fn sello(
    m: &Materializacion,
    efectivas: &BTreeMap<String, Etiquetas>,
) -> BTreeMap<Raiz, Etiquetas> {
    efectivas
        .iter()
        .map(|(col, e)| {
            (
                Raiz {
                    datasource: m.tabla.datasource.clone(),
                    objeto: m.tabla.objeto.clone(),
                    campo: col.clone(),
                },
                e.clone(),
            )
        })
        .collect()
}

// ── Comprobaciones ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::Clasificacion;
    use crate::plan::{Junta, Opaca};
    use crate::schema::esquema;
    use ore_core::flow::{Axis, Lattice};
    use ore_core::types::parse_type;

    fn lectura(ds: &str, objeto: &str, campos: &[(&str, &str)]) -> Lectura {
        Lectura {
            datasource: ds.into(),
            objeto: objeto.into(),
            campos: campos
                .iter()
                .map(|(n, t)| ((*n).to_string(), parse_type(t).unwrap()))
                .collect(),
        }
    }

    fn pedidos() -> Nodo {
        Nodo::Lee(lectura(
            "lago",
            "ventas.pedidos",
            &[
                ("id", "Integer"),
                ("id_legacy", "Integer"),
                ("pais", "String"),
                ("total", "Decimal"),
                ("nif", "String"),
            ],
        ))
    }

    fn cmp(campo: &str, op: Comparador, v: Valor) -> Expr {
        Expr::Compara {
            op,
            izquierda: Box::new(Expr::campo(campo)),
            derecha: Box::new(Expr::Literal(v)),
        }
    }
    fn eq_s(campo: &str, v: &str) -> Expr {
        cmp(campo, Comparador::Igual, Valor::Cadena(v.into()))
    }
    fn dec(s: &str) -> Valor {
        Valor::Decimal(s.into())
    }

    fn filtra(e: Nodo, p: Expr) -> Nodo {
        Nodo::Filtra {
            entrada: Box::new(e),
            predicado: p,
        }
    }
    fn proyecta(e: Nodo, cols: &[&str]) -> Nodo {
        Nodo::Proyecta {
            entrada: Box::new(e),
            campos: cols
                .iter()
                .map(|c| ((*c).to_string(), Expr::campo(c)))
                .collect(),
        }
    }

    /// Una materialización cuya tabla produce lo mismo que su plan.
    fn mat(nombre: &str, plan: Nodo) -> Materializacion {
        Materializacion {
            nombre: nombre.into(),
            tabla: Lectura {
                datasource: "lago".into(),
                objeto: format!("cache.{nombre}"),
                campos: esquema(&plan).expect("cuadra"),
            },
            plan,
        }
    }

    fn sin_sello() -> Clasificacion {
        Clasificacion::default()
    }

    /// Una materialización idéntica al plan contesta **tal cual**: sin
    /// compensación, leyendo la tabla, y produciendo el mismo esquema.
    #[test]
    fn una_materializacion_identica_contesta_sin_compensacion() {
        let plan = proyecta(filtra(pedidos(), eq_s("pais", "ES")), &["id", "total"]);
        let m = mat("es", plan.clone());
        let r = cotejar(&plan, &m, &sin_sello()).expect("contesta");

        assert_eq!(r.from, "es");
        assert!(r.compensation.is_empty(), "{:?}", r.compensation);
        assert_eq!(
            r.plan.lecturas()[0].objeto,
            "cache.es",
            "lee la tabla, no el origen"
        );
        assert_eq!(esquema(&r.plan).unwrap(), esquema(&plan).unwrap());
    }

    /// **La compensation es lo que el plan pide de más.** La materialización
    /// garantiza `pais = 'ES'`; el plan además quiere `total > 100`. Eso, y solo
    /// eso, va encima.
    #[test]
    fn la_compensation_es_lo_que_el_plan_pide_de_mas() {
        let m = mat(
            "es",
            proyecta(
                filtra(pedidos(), eq_s("pais", "ES")),
                &["id", "pais", "total"],
            ),
        );
        let plan = proyecta(
            filtra(
                pedidos(),
                Expr::Y(vec![
                    eq_s("pais", "ES"),
                    cmp("total", Comparador::Mayor, dec("100")),
                ]),
            ),
            &["id", "total"],
        );
        let r = cotejar(&plan, &m, &sin_sello()).expect("contesta");
        assert_eq!(
            r.compensation,
            vec![cmp("total", Comparador::Mayor, dec("100"))]
        );
        // Y el plan reescrito la aplica sobre la tabla.
        let Nodo::Proyecta { entrada, .. } = &r.plan else {
            panic!()
        };
        assert!(matches!(**entrada, Nodo::Filtra { .. }));
        assert_eq!(esquema(&r.plan).unwrap(), esquema(&plan).unwrap());
    }

    /// **Una materialización más estrecha no contesta**: le faltarían filas. Y
    /// dice qué conyunto es el que el plan no implica.
    #[test]
    fn una_materializacion_mas_estrecha_no_contesta() {
        let m = mat(
            "es_grandes",
            proyecta(
                filtra(
                    pedidos(),
                    Expr::Y(vec![
                        eq_s("pais", "ES"),
                        cmp("total", Comparador::Mayor, dec("100")),
                    ]),
                ),
                &["id", "total"],
            ),
        );
        let plan = proyecta(filtra(pedidos(), eq_s("pais", "ES")), &["id", "total"]);
        assert_eq!(
            cotejar(&plan, &m, &sin_sello()),
            Err(NoContesta::PredicadoNoSubsumido {
                conyunto: Box::new(cmp("total", Comparador::Mayor, dec("100")))
            })
        );
    }

    /// **La implicación razona por rangos, y con decimales exactos.** `total >=
    /// 10.25` implica `total > 0.5`, así que una materialización con lo segundo
    /// contesta a un plan con lo primero — con lo primero de compensation.
    #[test]
    fn la_implicacion_razona_por_rangos_sin_coma_flotante() {
        let m = mat(
            "positivos",
            proyecta(
                filtra(pedidos(), cmp("total", Comparador::Mayor, dec("0.5"))),
                &["id", "total"],
            ),
        );
        let plan = proyecta(
            filtra(
                pedidos(),
                cmp("total", Comparador::MayorIgual, dec("10.25")),
            ),
            &["id", "total"],
        );
        let r = cotejar(&plan, &m, &sin_sello()).expect("10.25 >= implica > 0.5");
        assert_eq!(
            r.compensation,
            vec![cmp("total", Comparador::MayorIgual, dec("10.25"))]
        );

        // Y al revés no: `total > 0.5` no implica `total >= 10.25`.
        assert!(matches!(
            cotejar(&m.plan, &mat("grandes", plan.clone()), &sin_sello()),
            Err(NoContesta::PredicadoNoSubsumido { .. })
        ));

        // El orden decimal es exacto: `0.10` es `0.1`, y `9.999 < 10.25`.
        assert_eq!(decimal("0.10", "0.1"), Some(Ordering::Equal));
        assert_eq!(decimal("9.999", "10.25"), Some(Ordering::Less));
        assert_eq!(decimal("-3", "2"), Some(Ordering::Less));
        assert_eq!(decimal("-0", "0.0"), Some(Ordering::Equal));
        assert_eq!(decimal("-1.5", "-1.25"), Some(Ordering::Less));
    }

    /// Una igualdad en el plan lo dice todo sobre la columna: `pais = 'ES'`
    /// implica `pais IN ('ES','PT')` y `pais != 'FR'`. Y un `IN` del plan
    /// contenido en el `IN` de la materialización también.
    #[test]
    fn una_igualdad_o_un_in_del_plan_implican_lo_que_los_contiene() {
        let en = |vs: &[&str]| Expr::EnConjunto {
            campo: "pais".into(),
            valores: vs.iter().map(|s| Valor::Cadena((*s).into())).collect(),
        };
        let m = mat(
            "iberia",
            proyecta(filtra(pedidos(), en(&["ES", "PT"])), &["id", "pais"]),
        );
        let plan_eq = proyecta(filtra(pedidos(), eq_s("pais", "ES")), &["id", "pais"]);
        assert!(cotejar(&plan_eq, &m, &sin_sello()).is_ok());

        let plan_in = proyecta(filtra(pedidos(), en(&["PT"])), &["id", "pais"]);
        assert!(cotejar(&plan_in, &m, &sin_sello()).is_ok());

        let plan_fuera = proyecta(filtra(pedidos(), en(&["ES", "FR"])), &["id", "pais"]);
        assert!(matches!(
            cotejar(&plan_fuera, &m, &sin_sello()),
            Err(NoContesta::PredicadoNoSubsumido { .. })
        ));

        let m_ne = mat(
            "no_fr",
            proyecta(
                filtra(
                    pedidos(),
                    cmp("pais", Comparador::Distinto, Valor::Cadena("FR".into())),
                ),
                &["id", "pais"],
            ),
        );
        assert!(cotejar(&plan_eq, &m_ne, &sin_sello()).is_ok());
    }

    /// **Data sufficiency.** El plan produce `total` y la materialización no lo
    /// expone, ni lo fija, ni el plan lo iguala a nada. Se dice cuál.
    #[test]
    fn una_columna_que_no_se_expone_ni_se_fija_no_se_deriva() {
        let m = mat("solo_ids", proyecta(pedidos(), &["id", "pais"]));
        let plan = proyecta(pedidos(), &["id", "total"]);
        assert_eq!(
            cotejar(&plan, &m, &sin_sello()),
            Err(NoContesta::ColumnaNoDerivable {
                columna: "total".into()
            })
        );
    }

    /// **Una columna fijada por el predicado es una constante**, y no hace falta
    /// exponerla — es el truco de Goldstein–Larson. La materialización filtra
    /// `pais = 'ES'` y no guarda `pais`; el plan lo pide y lo obtiene como
    /// literal.
    #[test]
    fn una_columna_fijada_por_el_predicado_se_deriva_como_constante() {
        let m = mat(
            "es_sin_pais",
            proyecta(filtra(pedidos(), eq_s("pais", "ES")), &["id", "total"]),
        );
        let plan = proyecta(
            filtra(pedidos(), eq_s("pais", "ES")),
            &["id", "pais", "total"],
        );
        let r = cotejar(&plan, &m, &sin_sello()).expect("contesta");
        let Nodo::Proyecta { campos, .. } = &r.plan else {
            panic!()
        };
        assert_eq!(campos["pais"], Expr::Literal(Valor::Cadena("ES".into())));
        assert!(r.compensation.is_empty(), "{:?}", r.compensation);
        assert_eq!(esquema(&r.plan).unwrap(), esquema(&plan).unwrap());
    }

    /// **Clase de equivalencia.** El plan iguala `id_legacy = id`; la
    /// materialización solo expone `id`. `id_legacy` se deriva de `id`, y el
    /// conyunto que lo igualaba se vuelve `id = id` y se cae.
    #[test]
    fn una_igualdad_del_plan_deriva_lo_que_no_se_expone() {
        let m = mat("ids", proyecta(pedidos(), &["id", "total"]));
        let plan = proyecta(
            filtra(
                pedidos(),
                Expr::Compara {
                    op: Comparador::Igual,
                    izquierda: Box::new(Expr::campo("id_legacy")),
                    derecha: Box::new(Expr::campo("id")),
                },
            ),
            &["id_legacy", "total"],
        );
        let r = cotejar(&plan, &m, &sin_sello()).expect("contesta");
        let Nodo::Proyecta { campos, .. } = &r.plan else {
            panic!()
        };
        assert_eq!(campos["id_legacy"], Expr::campo("id"));
        assert!(
            r.compensation.is_empty(),
            "la tautología se cae: {:?}",
            r.compensation
        );
    }

    /// **EL LABEL SEAL — lo que no tiene nadie.**
    ///
    /// La materialización filtró por `nif` (critical) y no lo expone. Su
    /// clasificación sellada dice que `total` es `critical` por influencia. El
    /// plan reescrito lee la tabla, donde `nif` **no existe**: si se recalculase
    /// el linaje desde los orígenes, `total` saldría sin etiqueta. Con el sello,
    /// hereda `critical`.
    #[test]
    fn el_label_seal_se_hereda_y_no_se_recalcula() {
        let gdpr = Lattice {
            qname: "gdpr.sensitivity".into(),
            levels: ["low", "medium", "high", "critical"]
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            axis: Axis::Confidentiality,
            requires_governance: BTreeMap::new(),
        };
        let raiz = |campo: &str| Raiz {
            datasource: "lago".into(),
            objeto: "ventas.pedidos".into(),
            campo: campo.into(),
        };
        let critical: Etiquetas = [("gdpr.sensitivity".to_string(), "critical".to_string())].into();

        // La materialización, y su clasificación al construirse: el Flow Checker
        // sobre SU plan, con las etiquetas de los orígenes.
        let m = mat(
            "de_un_nif",
            proyecta(filtra(pedidos(), eq_s("nif", "X")), &["id", "total"]),
        );
        let en_origen = Clasificacion {
            reticulos: [("gdpr.sensitivity".to_string(), gdpr.clone())].into(),
            de_raiz: [(raiz("nif"), critical.clone())].into(),
        };
        let al_construir = comprobar(&linaje(&m.plan).unwrap(), &en_origen, &Etiquetas::new());
        assert_eq!(
            al_construir.efectivas["total"], critical,
            "total es critical por influencia del filtro"
        );

        // El sello: esas etiquetas, colgadas de las columnas de la tabla.
        let sellado = Clasificacion {
            reticulos: en_origen.reticulos.clone(),
            de_raiz: sello(&m, &al_construir.efectivas),
        };

        let plan = proyecta(filtra(pedidos(), eq_s("nif", "X")), &["total"]);
        let r = cotejar(&plan, &m, &sellado).expect("contesta");
        assert_eq!(
            r.label_seal["total"], critical,
            "el sello se hereda: {:?}",
            r.label_seal
        );

        // Y la prueba de que hacía falta: recalcular desde los orígenes sobre el
        // plan reescrito da NADA, porque `nif` no está en la tabla.
        let recalculado = comprobar(&linaje(&r.plan).unwrap(), &en_origen, &Etiquetas::new());
        assert!(
            recalculado.efectivas["total"].is_empty(),
            "recalcular habría borrado la etiqueta: {:?}",
            recalculado.efectivas
        );
    }

    /// Una opaca en la proyección del plan no se reescribe. Y una opaca
    /// idéntica a los dos lados del filtro solo se da por implicada si es
    /// determinista: `RANDOM() > 0.5` dos veces son dos filtros distintos.
    #[test]
    fn una_opaca_no_se_reescribe_y_solo_implica_si_es_determinista() {
        let opaca = |determinista| {
            Expr::Opaca(Opaca {
                dialecto: "bigquery".into(),
                texto: "REGEXP_CONTAINS(pais, r'^E')".into(),
                lee: vec!["pais".into()],
                tipo: parse_type("Boolean").unwrap(),
                determinista,
            })
        };
        // Determinista e idéntica: implicada, y como está en las dos, sin
        // compensación.
        let m = mat(
            "regex",
            proyecta(filtra(pedidos(), opaca(true)), &["id", "pais"]),
        );
        let plan = proyecta(filtra(pedidos(), opaca(true)), &["id", "pais"]);
        let r = cotejar(&plan, &m, &sin_sello()).expect("contesta");
        assert!(r.compensation.is_empty());

        // Volátil e idéntica: NO implicada.
        let m_v = mat(
            "azar",
            proyecta(filtra(pedidos(), opaca(false)), &["id", "pais"]),
        );
        let plan_v = proyecta(filtra(pedidos(), opaca(false)), &["id", "pais"]);
        assert!(matches!(
            cotejar(&plan_v, &m_v, &sin_sello()),
            Err(NoContesta::PredicadoNoSubsumido { .. })
        ));

        // Y una opaca en la proyección, sobre una columna que la tabla renombra,
        // no se reescribe.
        let m_r = mat(
            "renombrada",
            Nodo::Proyecta {
                entrada: Box::new(pedidos()),
                campos: [("p".to_string(), Expr::campo("pais"))].into(),
            },
        );
        let plan_o = Nodo::Proyecta {
            entrada: Box::new(pedidos()),
            campos: [("es".to_string(), opaca(true))].into(),
        };
        assert_eq!(
            cotejar(&plan_o, &m_r, &sin_sello()),
            Err(NoContesta::OpacaNoReescribible)
        );
    }

    /// Lo que está fuera del subconjunto se dice **por su operador**, no con un
    /// `false`. Y una hoja distinta también.
    #[test]
    fn fuera_del_subconjunto_se_dice_por_su_operador() {
        let m = mat("p", proyecta(pedidos(), &["id"]));
        let junta = Nodo::Une {
            izquierda: Box::new(pedidos()),
            derecha: Box::new(Nodo::Lee(lectura(
                "sap",
                "ventas.lineas",
                &[("id_pedido", "Integer")],
            ))),
            tipo: Junta::Interna,
            sobre: vec![("id".into(), "id_pedido".into())],
        };
        assert_eq!(
            cotejar(&junta, &m, &sin_sello()),
            Err(NoContesta::FueraDelSubconjunto { operador: "une" })
        );
        assert_eq!(
            cotejar(
                &Nodo::Limita {
                    entrada: Box::new(pedidos()),
                    n: 5
                },
                &m,
                &sin_sello()
            ),
            Err(NoContesta::FueraDelSubconjunto { operador: "limita" })
        );
        let otra = Nodo::Lee(lectura("lago", "ventas.otra", &[("id", "Integer")]));
        assert_eq!(
            cotejar(&proyecta(otra, &["id"]), &m, &sin_sello()),
            Err(NoContesta::HojaDistinta)
        );
        assert_eq!(
            cotejar(&Nodo::Referencia("v".into()), &m, &sin_sello()),
            Err(NoContesta::SinExpandir { vista: "v".into() })
        );
    }

    /// Un filtro anidado bajo una proyección que renombra se sigue traduciendo:
    /// la forma se normaliza a la hoja, sea cual sea el anidamiento.
    #[test]
    fn el_anidamiento_no_importa_porque_todo_se_lleva_a_la_hoja() {
        let m = mat(
            "es",
            proyecta(filtra(pedidos(), eq_s("pais", "ES")), &["id", "total"]),
        );
        // Proyecta → Filtra (sobre el nombre nuevo) → Proyecta (renombra) → Lee.
        let plan = Nodo::Proyecta {
            entrada: Box::new(Nodo::Filtra {
                entrada: Box::new(Nodo::Proyecta {
                    entrada: Box::new(pedidos()),
                    campos: [
                        ("donde".to_string(), Expr::campo("pais")),
                        ("id".to_string(), Expr::campo("id")),
                        ("total".to_string(), Expr::campo("total")),
                    ]
                    .into(),
                }),
                predicado: eq_s("donde", "ES"),
            }),
            campos: [("id".to_string(), Expr::campo("id"))].into(),
        };
        let r = cotejar(&plan, &m, &sin_sello()).expect("contesta");
        assert!(r.compensation.is_empty());
        assert_eq!(esquema(&r.plan).unwrap(), esquema(&plan).unwrap());
    }
}
