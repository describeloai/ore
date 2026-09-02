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
//! # El subconjunto
//!
//! **Select-project-join sobre juntas internas, con un agregado encima como
//! mucho.** `Lee`, `Une` interna, `Filtra`, `Proyecta`, y un `Agrupa` con sus
//! `Filtra`/`Proyecta` por encima. Lo que no quepa —juntas externas, unión,
//! `Distingue`, `Limita`, agregados anidados— sale con
//! [`NoContesta::FueraDelSubconjunto`] **nombrando el operador**, no con un
//! `false`.
//!
//! Los cuatro *checks* de Oracle y Calcite, y el nuestro:
//!
//! | | Check | Aquí |
//! |---|---|---|
//! | 1 | **join compatibility** | las juntas de la materialización las implica el plan; las del plan que faltan son compensación; una hoja **de más** en la materialización solo vale con **clave única y referencial declaradas** |
//! | 2 | **data sufficiency** | cada columna que el plan produce se deriva de lo expuesto, de una **constante** que el predicado fija, o de una **clase de equivalencia** del plan |
//! | 3 | **predicate subsumption** | el predicado del plan **implica** el de la materialización; lo que sobra es la **compensation** |
//! | 4 | **aggregate computability** | la agrupación del plan es igual o **más gruesa**; los agregados se copian o se **enrollan** — y `AVG` no se enrolla |
//! | — | **label seal** | la clasificación de la materialización **se hereda, no se recalcula** |
//!
//! # La implicación, y hasta dónde llega
//!
//! Un conyunto de la materialización está implicado por el plan si el plan lo
//! contiene **tal cual**, si es una igualdad entre columnas de la **misma clase**
//! del plan, o si es una comparación simple —columna contra literal— y los
//! conyuntos del plan sobre esa columna la **acotan**: `total >= 100` implica
//! `total > 0`. Fuera de eso **no hay implicación**, y no haberla es la respuesta
//! segura: afirmar una que no se sabe demostrar es servir filas que no debían
//! salir.
//!
//! Y una opaca **idéntica** en los dos lados solo se da por implicada si es
//! **determinista**: un `RANDOM() > 0.5` escrito dos veces son dos filtros
//! distintos.
//!
//! # La junta de más, y por qué hacen falta dos restricciones
//!
//! Oracle: *«las restricciones se usan para determinar juntas sin pérdida»*. Una
//! junta interna `X.a = E.b` **no pierde ni duplica** filas de `X` solo si `E.b`
//! es **única** —ninguna fila de `X` casa con dos de `E`— **y** existe una
//! **referencial** de `X.a` a `E.b` —toda fila de `X` casa con una de `E`—. La
//! unicidad sola evita duplicar; la referencial sola evita perder. **Con una de
//! las dos no hay garantía, y se dice cuál falta.** Sin restricciones declaradas
//! no se supone ninguna.
//!
//! # `AVG` no se enrolla, y aquí además no se puede escribir
//!
//! `SUM`, `COUNT`, `MIN` y `MAX` se enrollan a una agrupación más gruesa:
//! `SUM(SUM)`, `SUM(COUNT)`, `MIN(MIN)`, `MAX(MAX)`. `AVG` necesitaría
//! `SUM(sumas) / SUM(cuentas)`, y **el álgebra no tiene división** — no es una
//! omisión: no hay aritmética porque no hay coma flotante. A la misma granularidad
//! `AVG` se copia; a otra, se dice que no.
//!
//! # El label seal, que no lo tiene nadie
//!
//! Una vista que filtró por `nif` produce un resultado `critical` aunque `nif`
//! **no esté entre sus columnas**. Si al reescribir se recalculase el linaje sobre
//! la tabla materializada, la etiqueta desaparecería. Así que las raíces del
//! plan reescrito son las columnas de la tabla, **y sus etiquetas son las
//! selladas**. El Flow Checker corre sobre eso: la misma regla, sin segunda copia.

use crate::filter_tree::{Hoja, Materializacion};
use crate::flow::{Clasificacion, Etiquetas, comprobar};
use crate::lineage::{Raiz, linaje};
use crate::plan::{Agregacion, Agregado, Comparador, Expr, Junta, Lectura, Nodo, Valor};
use crate::schema::esquema;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

/// Lo que sale de un cotejo que contesta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rewrite {
    pub from: String,
    /// El plan reescrito: lee la tabla materializada, con lo que haga falta
    /// encima — compensación, juntas de vuelta, re-agregación, proyección.
    pub plan: Nodo,
    /// Los conyuntos que la materialización **no** garantiza. Vacío cuando
    /// contesta tal cual.
    pub compensation: Vec<Expr>,
    /// La clasificación de cada columna de salida, **heredada del sello**.
    pub label_seal: BTreeMap<String, Etiquetas>,
}

/// Una restricción declarada sobre una hoja. Son **datos**, como el sello: esta
/// pieza no sabe de dónde salen — de un `primaryKey`, de un `uniqueKeys`, de una
/// relación—, solo qué garantizan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Restriccion {
    /// Las columnas identifican una fila como mucho.
    Unica { hoja: Hoja, columnas: Vec<String> },
    /// Todo valor de `desde` existe en `hacia`.
    Referencial {
        desde: (Hoja, Vec<String>),
        hacia: (Hoja, Vec<String>),
    },
}

/// Por qué no contesta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoContesta {
    FueraDelSubconjunto {
        operador: &'static str,
    },
    SinExpandir {
        vista: String,
    },
    /// Uno agrega y el otro no.
    FormasDistintas,
    /// La materialización no lee alguna hoja del plan y no hay forma de traerla.
    HojaAusente {
        hoja: Hoja,
    },
    ColumnaNoDerivable {
        columna: String,
    },
    /// En caja porque una `Expr` es grande y un `Err` grande se copia en cada
    /// retorno, también en el camino bueno.
    PredicadoNoSubsumido {
        conyunto: Box<Expr>,
    },
    OpacaNoReescribible,
    /// La materialización tiene una hoja que el plan no lee, y la junta que la
    /// trae **no está garantizada** sin pérdida ni duplicado. `falta` dice qué
    /// restricción haría falta.
    JuntaDeMasSinGarantia {
        hoja: Hoja,
        falta: &'static str,
    },
    /// El plan agrupa por algo que la materialización ya agregó.
    AgrupacionMasFina {
        columna: String,
    },
    /// El plan pide un agregado que la materialización no calculó.
    AgregadoNoDisponible {
        nombre: String,
    },
    /// El agregado existe pero no se enrolla a la agrupación del plan.
    AgregadoNoEnrollable {
        nombre: String,
        funcion: Agregado,
        porque: &'static str,
    },
    /// Una compensación **por debajo del agregado** sobre una columna que no es
    /// de grupo: esas filas ya se sumaron, y no se pueden separar.
    CompensacionBajoAgregado {
        conyunto: Box<Expr>,
    },
    /// El plan reescrito no cuadra. No debería ocurrir; si ocurre, se dice.
    ReescrituraNoCuadra {
        porque: String,
    },
}

impl NoContesta {
    pub fn como_texto(&self) -> String {
        match self {
            NoContesta::FueraDelSubconjunto { operador } => format!(
                "`{operador}` está fuera del subconjunto que este cotejo decide: \
                 select-project-join interno con un agregado encima como mucho"
            ),
            NoContesta::SinExpandir { vista } => {
                format!("todavía nombra a `{vista}`: hay que expandir antes de cotejar")
            }
            NoContesta::FormasDistintas => "uno agrega y el otro no".into(),
            NoContesta::HojaAusente { hoja } => {
                format!("la materialización no lee `{}·{}`", hoja.0, hoja.1)
            }
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
            NoContesta::JuntaDeMasSinGarantia { hoja, falta } => format!(
                "la materialización junta `{}·{}` y el plan no la lee; sin {falta} esa junta \
                 puede perder o duplicar filas",
                hoja.0, hoja.1
            ),
            NoContesta::AgrupacionMasFina { columna } => format!(
                "el plan agrupa por `{columna}` y la materialización ya la agregó: no se puede \
                 volver a separar"
            ),
            NoContesta::AgregadoNoDisponible { nombre } => {
                format!("el agregado `{nombre}` no está en la materialización")
            }
            NoContesta::AgregadoNoEnrollable {
                nombre,
                funcion,
                porque,
            } => format!("`{nombre}` ({funcion:?}) no se enrolla: {porque}"),
            NoContesta::CompensacionBajoAgregado { conyunto } => format!(
                "{conyunto:?} filtra por debajo del agregado sobre una columna que no es de \
                 grupo: esas filas ya se sumaron y no se separan"
            ),
            NoContesta::ReescrituraNoCuadra { porque } => {
                format!("el plan reescrito no cuadra: {porque}")
            }
        }
    }
}

// ── La forma normalizada ────────────────────────────────────────────────────

/// Select-project-join aplanado: hojas, pares de junta interna, conyuntos y
/// salida — todo en términos de columnas de hoja, que son únicas porque el
/// tipador rechaza colisiones.
struct Spj<'a> {
    hojas: BTreeMap<Hoja, &'a Lectura>,
    juntas: Vec<(String, String)>,
    conyuntos: Vec<Expr>,
    salida: BTreeMap<String, Expr>,
}

/// Y con un agregado encima: `encima` son los conyuntos *having* y `salida` la
/// proyección final, los dos sobre los nombres que el agregado produce.
struct Agregada<'a> {
    dentro: Spj<'a>,
    por: BTreeSet<String>,
    agregados: BTreeMap<String, Agregacion>,
    encima: Vec<Expr>,
    salida: BTreeMap<String, Expr>,
}

enum Forma<'a> {
    Spj(Spj<'a>),
    Agregada(Agregada<'a>),
}

fn fuera(operador: &'static str) -> NoContesta {
    NoContesta::FueraDelSubconjunto { operador }
}

fn spj(n: &Nodo) -> Result<Spj<'_>, NoContesta> {
    match n {
        Nodo::Referencia(v) => Err(NoContesta::SinExpandir { vista: v.clone() }),
        Nodo::Lee(l) => Ok(Spj {
            hojas: [((l.datasource.clone(), l.objeto.clone()), l)].into(),
            juntas: Vec::new(),
            conyuntos: Vec::new(),
            salida: l
                .campos
                .keys()
                .map(|c| (c.clone(), Expr::Campo(c.clone())))
                .collect(),
        }),
        Nodo::Une {
            izquierda,
            derecha,
            tipo,
            sobre,
        } => {
            if *tipo != Junta::Interna {
                return Err(fuera("junta externa"));
            }
            let (mut i, d) = (spj(izquierda)?, spj(derecha)?);
            for (a, b) in sobre {
                let (Expr::Campo(a), Expr::Campo(b)) = (
                    sustituir(&Expr::campo(a), &i.salida)?,
                    sustituir(&Expr::campo(b), &d.salida)?,
                ) else {
                    return Err(fuera("junta sobre una columna computada"));
                };
                i.juntas.push((a, b));
            }
            i.hojas.extend(d.hojas);
            i.juntas.extend(d.juntas);
            i.conyuntos.extend(d.conyuntos);
            i.salida.extend(d.salida);
            Ok(i)
        }
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
                hojas: s.hojas,
                juntas: s.juntas,
                conyuntos: s.conyuntos,
                salida,
            })
        }
        Nodo::Agrupa { .. } => Err(fuera("agregado anidado")),
        Nodo::Unifica(_) => Err(fuera("unifica")),
        Nodo::Distingue(_) => Err(fuera("distingue")),
        Nodo::Limita { .. } => Err(fuera("limita")),
    }
}

fn forma(n: &Nodo) -> Result<Forma<'_>, NoContesta> {
    match n {
        Nodo::Agrupa {
            entrada,
            por,
            agregados,
        } => {
            let dentro = spj(entrada)?;
            let salida = por
                .iter()
                .chain(agregados.keys())
                .map(|c| (c.clone(), Expr::Campo(c.clone())))
                .collect();
            Ok(Forma::Agregada(Agregada {
                dentro,
                por: por.clone(),
                agregados: agregados.clone(),
                encima: Vec::new(),
                salida,
            }))
        }
        Nodo::Filtra { entrada, predicado } => match forma(entrada)? {
            Forma::Spj(_) => Ok(Forma::Spj(spj(n)?)),
            Forma::Agregada(mut a) => {
                let p = sustituir(predicado, &a.salida)?;
                a.encima.extend(conyuntos(&p));
                Ok(Forma::Agregada(a))
            }
        },
        Nodo::Proyecta { entrada, campos } => match forma(entrada)? {
            Forma::Spj(_) => Ok(Forma::Spj(spj(n)?)),
            Forma::Agregada(a) => {
                let salida = campos
                    .iter()
                    .map(|(k, x)| Ok((k.clone(), sustituir(x, &a.salida)?)))
                    .collect::<Result<_, _>>()?;
                Ok(Forma::Agregada(Agregada { salida, ..a }))
            }
        },
        otro => Ok(Forma::Spj(spj(otro)?)),
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

fn conjuncion(mut v: Vec<Expr>) -> Option<Expr> {
    match v.len() {
        0 => None,
        1 => Some(v.remove(0)),
        _ => Some(Expr::Y(v)),
    }
}

fn filtrando(n: Nodo, c: Vec<Expr>) -> Nodo {
    match conjuncion(c) {
        None => n,
        Some(p) => Nodo::Filtra {
            entrada: Box::new(n),
            predicado: p,
        },
    }
}

// ── Clases de equivalencia ──────────────────────────────────────────────────

/// Las igualdades entre columnas —juntas y conyuntos `a = b`— como clases.
/// Es lo que hace que `X.a = E.b` de la materialización esté implicado si el
/// plan iguala `a` y `b` por cualquier camino.
struct Clases(BTreeMap<String, String>);

impl Clases {
    fn de(s: &Spj) -> Clases {
        let mut c = Clases(BTreeMap::new());
        for (a, b) in &s.juntas {
            c.unir(a, b);
        }
        for x in &s.conyuntos {
            if let Some((a, b)) = igualdad(x) {
                c.unir(a, b);
            }
        }
        c
    }
    fn raiz(&self, a: &str) -> String {
        let mut x = a.to_string();
        while let Some(p) = self.0.get(&x) {
            x = p.clone();
        }
        x
    }
    fn unir(&mut self, a: &str, b: &str) {
        let (ra, rb) = (self.raiz(a), self.raiz(b));
        if ra != rb {
            // La menor manda, para que el resultado no dependa del orden.
            let (menor, mayor) = if ra < rb { (ra, rb) } else { (rb, ra) };
            self.0.insert(mayor, menor);
        }
    }
    fn iguales(&self, a: &str, b: &str) -> bool {
        self.raiz(a) == self.raiz(b)
    }
}

fn igualdad(x: &Expr) -> Option<(&str, &str)> {
    if let Expr::Compara {
        op: Comparador::Igual,
        izquierda,
        derecha,
    } = x
        && let (Expr::Campo(a), Expr::Campo(b)) = (&**izquierda, &**derecha)
    {
        return Some((a, b));
    }
    None
}

// ── Literales: orden exacto, sin coma flotante ──────────────────────────────

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
        Some((
            neg,
            ent.trim_start_matches('0').to_string(),
            frac.trim_end_matches('0').to_string(),
        ))
    }
    let (na, ea, fa) = partes(a)?;
    let (nb, eb, fb) = partes(b)?;
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

#[derive(Default)]
struct Hechos<'a> {
    igual: Option<&'a Valor>,
    distintos: Vec<&'a Valor>,
    inferior: Option<(&'a Valor, bool)>,
    superior: Option<(&'a Valor, bool)>,
    en: Option<&'a [Valor]>,
}

/// Lo que `base` afirma sobre una columna, **y sobre todas las de su clase**:
/// si el plan iguala `a = b` y acota `b`, también acota `a`.
fn hechos<'a>(base: &'a [Expr], clases: &Clases, columna: &str) -> Hechos<'a> {
    let mut h = Hechos::default();
    for x in base {
        match simple(x) {
            Some(Simple::Cmp(c, op, v)) if clases.iguales(c, columna) => match op {
                Comparador::Igual => h.igual = Some(v),
                Comparador::Distinto => h.distintos.push(v),
                Comparador::Menor => ajustar(&mut h.superior, v, false, Ordering::Less),
                Comparador::MenorIgual => ajustar(&mut h.superior, v, true, Ordering::Less),
                Comparador::Mayor => ajustar(&mut h.inferior, v, false, Ordering::Greater),
                Comparador::MayorIgual => ajustar(&mut h.inferior, v, true, Ordering::Greater),
            },
            Some(Simple::En(c, vs)) if clases.iguales(c, columna) => h.en = Some(vs),
            _ => {}
        }
    }
    h
}

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

fn implica_cmp(h: &Hechos, op: Comparador, lit: &Valor) -> bool {
    use Comparador::*;
    let cmp = |a: &Valor| comparar(a, lit);
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
    if let Some(vs) = h.en {
        let todos = |f: &dyn Fn(Ordering) -> bool| {
            !vs.is_empty() && vs.iter().all(|v| cmp(v).is_some_and(f))
        };
        return match op {
            Igual => todos(&|o| o == Ordering::Equal),
            Distinto => todos(&|o| o != Ordering::Equal),
            Menor => todos(&|o| o == Ordering::Less),
            MenorIgual => todos(&|o| o != Ordering::Greater),
            Mayor => todos(&|o| o == Ordering::Greater),
            MayorIgual => todos(&|o| o != Ordering::Less),
        };
    }
    let excluye = |cota: Option<(&Valor, bool)>, lado: Ordering| {
        cota.is_some_and(|(v, incl)| {
            cmp(v) == Some(lado) || (!incl && cmp(v) == Some(Ordering::Equal))
        })
    };
    match op {
        Igual => false,
        Distinto => {
            h.distintos.iter().any(|v| cmp(v) == Some(Ordering::Equal))
                || excluye(h.inferior, Ordering::Greater)
                || excluye(h.superior, Ordering::Less)
        }
        Menor => excluye(h.superior, Ordering::Less),
        MenorIgual => h
            .superior
            .is_some_and(|(v, _)| matches!(cmp(v), Some(Ordering::Less | Ordering::Equal))),
        Mayor => excluye(h.inferior, Ordering::Greater),
        MayorIgual => h
            .inferior
            .is_some_and(|(v, _)| matches!(cmp(v), Some(Ordering::Greater | Ordering::Equal))),
    }
}

/// ¿Implica `base` a `objetivo`?
fn implica(base: &[Expr], clases: &Clases, objetivo: &Expr) -> bool {
    if base.contains(objetivo) {
        return match objetivo {
            Expr::Opaca(o) => o.determinista,
            _ => true,
        };
    }
    if let Some((a, b)) = igualdad(objetivo) {
        return clases.iguales(a, b);
    }
    match simple(objetivo) {
        Some(Simple::Cmp(c, op, v)) => implica_cmp(&hechos(base, clases, c), op, v),
        Some(Simple::En(c, vs)) => {
            let h = hechos(base, clases, c);
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

// ── El cotejo de la parte select-project-join ───────────────────────────────

/// Lo que produce cotejar dos `Spj`: cómo se escribe cada columna de hoja en
/// términos de la tabla, qué compensación queda, y qué hojas del plan hay que
/// traer de vuelta.
struct Base {
    disponibles: BTreeMap<String, Expr>,
    compensation: Vec<Expr>,
    /// Hojas del plan que la materialización no lee, con las juntas que las
    /// enganchan — en términos ya traducidos.
    de_vuelta: Vec<(Lectura, Vec<(String, String)>)>,
}

fn cotejar_spj(p: &Spj, q: &Spj, restricciones: &[Restriccion]) -> Result<Base, NoContesta> {
    let clases_p = Clases::de(p);
    let clases_q = Clases::de(q);

    // ── 1 · hojas de más en la materialización: solo con garantía.
    for (hoja, _) in q.hojas.iter().filter(|(h, _)| !p.hojas.contains_key(*h)) {
        garantizada(hoja, q, restricciones)?;
    }

    // ── 3 · lo que la materialización filtra —y junta—, el plan lo implica.
    for c in &q.conyuntos {
        if !implica(&p.conyuntos, &clases_p, c) {
            return Err(NoContesta::PredicadoNoSubsumido {
                conyunto: Box::new(c.clone()),
            });
        }
    }
    for (a, b) in &q.juntas {
        // Una junta a una hoja de más no tiene con qué compararse en el plan, y
        // ya está garantizada arriba. Las demás, el plan tiene que igualarlas.
        let de_mas = |c: &str| {
            q.hojas
                .iter()
                .any(|(h, l)| !p.hojas.contains_key(h) && l.campos.contains_key(c))
        };
        if !(de_mas(a) || de_mas(b)) && !clases_p.iguales(a, b) {
            return Err(NoContesta::PredicadoNoSubsumido {
                conyunto: Box::new(Expr::Compara {
                    op: Comparador::Igual,
                    izquierda: Box::new(Expr::campo(a)),
                    derecha: Box::new(Expr::campo(b)),
                }),
            });
        }
    }

    // ── 2 · qué columna de hoja se escribe con qué.
    let mut disponibles: BTreeMap<String, Expr> = BTreeMap::new();
    for (salida, x) in &q.salida {
        if let Expr::Campo(c) = x {
            disponibles
                .entry(c.clone())
                .or_insert(Expr::Campo(salida.clone()));
        }
    }
    for x in &q.conyuntos {
        if let Some(Simple::Cmp(c, Comparador::Igual, v)) = simple(x) {
            disponibles
                .entry(c.to_string())
                .or_insert(Expr::Literal(v.clone()));
        }
    }
    // Las hojas del plan que la materialización no lee se traen de vuelta:
    // sus columnas están disponibles **como ellas mismas**.
    let de_vuelta: Vec<(Hoja, Lectura)> = p
        .hojas
        .iter()
        .filter(|(h, _)| !q.hojas.contains_key(*h))
        .map(|(h, l)| (h.clone(), (*l).clone()))
        .collect();
    let cols_de_vuelta: BTreeSet<String> = de_vuelta
        .iter()
        .flat_map(|(_, l)| l.campos.keys().cloned())
        .collect();
    for c in &cols_de_vuelta {
        disponibles.insert(c.clone(), Expr::Campo(c.clone()));
    }
    // Y una igualdad del plan deriva lo que no se expone de lo que sí — **entre
    // columnas de la materialización**. Una columna de una hoja de vuelta no
    // sirve de fuente: lo dijo ejecutarlo. Derivar la clave de junta desde la
    // hoja que hay que traer producía `cid = cid`, una junta consigo misma.
    let pares: Vec<(String, String)> = p
        .juntas
        .iter()
        .cloned()
        .chain(
            p.conyuntos
                .iter()
                .filter_map(|x| igualdad(x).map(|(a, b)| (a.to_string(), b.to_string()))),
        )
        .filter(|(a, b)| !cols_de_vuelta.contains(a) && !cols_de_vuelta.contains(b))
        .collect();
    for _ in 0..pares.len() {
        for (a, b) in &pares {
            if !disponibles.contains_key(a)
                && let Some(d) = disponibles.get(b).cloned()
            {
                disponibles.insert(a.clone(), d);
            } else if !disponibles.contains_key(b)
                && let Some(d) = disponibles.get(a).cloned()
            {
                disponibles.insert(b.clone(), d);
            }
        }
    }

    // ── la compensation: lo que el plan pide y la materialización no da.
    let mut compensation = Vec::new();
    for c in &p.conyuntos {
        if !implica(&q.conyuntos, &clases_q, c) {
            let t = sustituir(c, &disponibles)?;
            if !tautologia(&t) {
                compensation.push(t);
            }
        }
    }

    // ── las juntas del plan: las que engancha una hoja de vuelta van a su
    // junta; las que quedan entre columnas de la tabla van a compensación.
    //
    // Se resuelve **iterando hasta que ninguna progrese**: una hoja de vuelta
    // puede engancharse a otra hoja de vuelta, y el orden en que se declararon
    // no puede decidir si se encuentra. Lo que quede sin enganchar es una hoja
    // ausente, y se dice cuál.
    let mut en_arbol: BTreeSet<String> = q.salida.keys().cloned().collect();
    let mut pendientes = de_vuelta;
    let mut de_vuelta_con_juntas = Vec::new();
    let mut usadas = BTreeSet::new();
    loop {
        let mut progreso = false;
        let mut siguen = Vec::new();
        for (hoja, l) in pendientes {
            let mut enganches = Vec::new();
            for (i, (a, b)) in p.juntas.iter().enumerate() {
                if usadas.contains(&i) {
                    continue;
                }
                let (mio, otro) = if l.campos.contains_key(a) {
                    (a, b)
                } else if l.campos.contains_key(b) {
                    (b, a)
                } else {
                    continue;
                };
                let Ok(Expr::Campo(otro_t)) = sustituir(&Expr::campo(otro), &disponibles) else {
                    continue;
                };
                if !en_arbol.contains(&otro_t) {
                    continue;
                }
                enganches.push((otro_t, mio.clone()));
                usadas.insert(i);
            }
            if enganches.is_empty() {
                siguen.push((hoja, l));
            } else {
                en_arbol.extend(l.campos.keys().cloned());
                de_vuelta_con_juntas.push((l, enganches));
                progreso = true;
            }
        }
        pendientes = siguen;
        if pendientes.is_empty() {
            break;
        }
        if !progreso {
            return Err(NoContesta::HojaAusente {
                hoja: pendientes[0].0.clone(),
            });
        }
    }
    for (i, (a, b)) in p.juntas.iter().enumerate() {
        if usadas.contains(&i) || clases_q.iguales(a, b) {
            continue;
        }
        let t = sustituir(
            &Expr::Compara {
                op: Comparador::Igual,
                izquierda: Box::new(Expr::campo(a)),
                derecha: Box::new(Expr::campo(b)),
            },
            &disponibles,
        )?;
        if !tautologia(&t) {
            compensation.push(t);
        }
    }

    Ok(Base {
        disponibles,
        compensation,
        de_vuelta: de_vuelta_con_juntas,
    })
}

/// Una hoja que la materialización lee y el plan no. La junta que la trae no
/// pierde ni duplica filas **solo con las dos restricciones**: única en el lado
/// de más, y referencial hacia él.
fn garantizada(hoja: &Hoja, q: &Spj, restricciones: &[Restriccion]) -> Result<(), NoContesta> {
    let l = q.hojas[hoja];
    let pertenece = |c: &str| l.campos.contains_key(c);
    let mut cols_e: Vec<String> = Vec::new();
    let mut cols_x: Vec<(Hoja, String)> = Vec::new();
    for (a, b) in &q.juntas {
        let (e, x) = match (pertenece(a), pertenece(b)) {
            (true, false) => (a, b),
            (false, true) => (b, a),
            _ => continue,
        };
        let Some((hx, _)) = q.hojas.iter().find(|(_, lx)| lx.campos.contains_key(x)) else {
            continue;
        };
        cols_e.push(e.clone());
        cols_x.push((hx.clone(), x.clone()));
    }
    if cols_e.is_empty() {
        return Err(NoContesta::JuntaDeMasSinGarantia {
            hoja: hoja.clone(),
            falta: "una junta que la enganche",
        });
    }
    let unica = restricciones.iter().any(|r| {
        matches!(r, Restriccion::Unica { hoja: h, columnas } if h == hoja && *columnas == cols_e)
    });
    if !unica {
        return Err(NoContesta::JuntaDeMasSinGarantia {
            hoja: hoja.clone(),
            falta: "una clave única en el lado de más (evita duplicar)",
        });
    }
    let (hx, _) = &cols_x[0];
    let desde: Vec<String> = cols_x.iter().map(|(_, c)| c.clone()).collect();
    let referencial = restricciones.iter().any(|r| {
        matches!(r, Restriccion::Referencial { desde: (h1, c1), hacia: (h2, c2) }
            if h1 == hx && *c1 == desde && h2 == hoja && *c2 == cols_e)
    });
    if !referencial {
        return Err(NoContesta::JuntaDeMasSinGarantia {
            hoja: hoja.clone(),
            falta: "una referencial hacia el lado de más (evita perder)",
        });
    }
    Ok(())
}

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

/// La tabla, la compensación encima, y las hojas de vuelta enganchadas.
fn cuerpo(m: &Materializacion, base: &Base) -> Nodo {
    let mut n = filtrando(Nodo::Lee(m.tabla.clone()), base.compensation.clone());
    for (l, juntas) in &base.de_vuelta {
        n = Nodo::Une {
            izquierda: Box::new(n),
            derecha: Box::new(Nodo::Lee(l.clone())),
            tipo: Junta::Interna,
            sobre: juntas.clone(),
        };
    }
    n
}

// ── El cotejo ───────────────────────────────────────────────────────────────

/// **El cotejo.**
///
/// `sello` es la clasificación que la materialización trae puesta, con raíces
/// en **las columnas de su tabla**. `restricciones` son las claves declaradas
/// sobre las hojas; sin ellas, ninguna junta de más se supone sin pérdida.
pub fn cotejar(
    plan: &Nodo,
    m: &Materializacion,
    sello: &Clasificacion,
    restricciones: &[Restriccion],
) -> Result<Rewrite, NoContesta> {
    let (nodo, compensation) = match (forma(plan)?, forma(&m.plan)?) {
        (Forma::Spj(p), Forma::Spj(q)) => {
            let base = cotejar_spj(&p, &q, restricciones)?;
            let salida = p
                .salida
                .iter()
                .map(|(k, x)| Ok((k.clone(), sustituir(x, &base.disponibles)?)))
                .collect::<Result<_, _>>()?;
            (
                Nodo::Proyecta {
                    entrada: Box::new(cuerpo(m, &base)),
                    campos: salida,
                },
                base.compensation,
            )
        }
        (Forma::Agregada(p), Forma::Agregada(q)) => cotejar_agregadas(&p, &q, m, restricciones)?,
        _ => return Err(NoContesta::FormasDistintas),
    };

    if let Err(e) = esquema(&nodo) {
        return Err(NoContesta::ReescrituraNoCuadra {
            porque: e.como_texto(),
        });
    }
    let label_seal = linaje(&nodo)
        .map(|l| comprobar(&l, sello, &Etiquetas::new()).efectivas)
        .unwrap_or_default();

    Ok(Rewrite {
        from: m.nombre.clone(),
        plan: nodo,
        compensation,
        label_seal,
    })
}

/// **Check 4.** Dos formas agregadas.
fn cotejar_agregadas(
    p: &Agregada,
    q: &Agregada,
    m: &Materializacion,
    restricciones: &[Restriccion],
) -> Result<(Nodo, Vec<Expr>), NoContesta> {
    if !q.encima.is_empty() {
        return Err(fuera("having en la materialización"));
    }
    let base = cotejar_spj(&p.dentro, &q.dentro, restricciones)?;

    // La columna de hoja detrás de cada nombre de grupo, a los dos lados.
    let hoja_de = |s: &Spj, nombre: &str| -> Option<String> {
        match s.salida.get(nombre) {
            Some(Expr::Campo(c)) => Some(c.clone()),
            _ => None,
        }
    };
    let grupo_q: BTreeMap<String, String> = q
        .por
        .iter()
        .filter_map(|n| hoja_de(&q.dentro, n).map(|h| (h, n.clone())))
        .collect();
    // Qué columna de la TABLA expone cada nombre del agregado de q.
    let tabla_de: BTreeMap<String, String> = q
        .salida
        .iter()
        .filter_map(|(out, x)| match x {
            Expr::Campo(c) => Some((c.clone(), out.clone())),
            _ => None,
        })
        .collect();

    // Una compensación bajo el agregado solo vale sobre columnas de grupo de q:
    // lo demás ya se sumó.
    for c in &base.compensation {
        let toca = c.campos_leidos();
        if !toca.iter().all(|col| {
            tabla_de
                .iter()
                .any(|(agg, out)| out == col && q.por.contains(agg))
        }) {
            return Err(NoContesta::CompensacionBajoAgregado {
                conyunto: Box::new(c.clone()),
            });
        }
    }

    // Agrupación: cada columna de grupo del plan tiene que ser de grupo en q.
    let mut por_plan_hojas = BTreeSet::new();
    let mut por_nuevo = BTreeSet::new();
    let mut nombres: BTreeMap<String, Expr> = BTreeMap::new();
    for n in &p.por {
        let Some(h) = hoja_de(&p.dentro, n) else {
            return Err(fuera("grupo sobre una columna computada"));
        };
        let Some(nq) = grupo_q.get(&h) else {
            return Err(NoContesta::AgrupacionMasFina { columna: h });
        };
        let Some(t) = tabla_de.get(nq) else {
            return Err(NoContesta::ColumnaNoDerivable { columna: h });
        };
        por_plan_hojas.insert(h);
        por_nuevo.insert(t.clone());
        nombres.insert(n.clone(), Expr::Campo(t.clone()));
    }
    let grupo_q_hojas: BTreeSet<&String> = grupo_q.keys().collect();
    let misma_granularidad = por_plan_hojas.iter().collect::<BTreeSet<_>>() == grupo_q_hojas;

    // Agregados: copia a la misma granularidad, enrollado a otra.
    let sobre_hoja = |s: &Spj, a: &Agregacion| -> Result<Option<String>, NoContesta> {
        match &a.sobre {
            None => Ok(None),
            Some(c) => hoja_de(s, c)
                .map(Some)
                .ok_or(fuera("agregado sobre una columna computada")),
        }
    };
    let mut enrollados: BTreeMap<String, Agregacion> = BTreeMap::new();
    for (nombre, a) in &p.agregados {
        let h = sobre_hoja(&p.dentro, a)?;
        let en_q = q
            .agregados
            .iter()
            .find(|(_, b)| b.funcion == a.funcion && sobre_hoja(&q.dentro, b).ok().flatten() == h);
        let Some((nq, _)) = en_q else {
            return Err(NoContesta::AgregadoNoDisponible {
                nombre: nombre.clone(),
            });
        };
        let Some(t) = tabla_de.get(nq) else {
            return Err(NoContesta::AgregadoNoDisponible {
                nombre: nombre.clone(),
            });
        };
        if misma_granularidad {
            nombres.insert(nombre.clone(), Expr::Campo(t.clone()));
        } else {
            let funcion = match a.funcion {
                Agregado::Suma | Agregado::Cuenta => Agregado::Suma,
                Agregado::Minimo => Agregado::Minimo,
                Agregado::Maximo => Agregado::Maximo,
                Agregado::Promedio => {
                    return Err(NoContesta::AgregadoNoEnrollable {
                        nombre: nombre.clone(),
                        funcion: Agregado::Promedio,
                        porque: "harían falta SUMA y CUENTA aparte, y el álgebra no tiene \
                                 división",
                    });
                }
            };
            enrollados.insert(
                nombre.clone(),
                Agregacion {
                    funcion,
                    sobre: Some(t.clone()),
                },
            );
            nombres.insert(nombre.clone(), Expr::Campo(nombre.clone()));
        }
    }

    let mut n = cuerpo(m, &base);
    if !misma_granularidad {
        n = Nodo::Agrupa {
            entrada: Box::new(n),
            por: por_nuevo,
            agregados: enrollados,
        };
    }
    let having = p
        .encima
        .iter()
        .map(|c| sustituir(c, &nombres))
        .collect::<Result<Vec<_>, _>>()?;
    n = filtrando(n, having);
    let salida = p
        .salida
        .iter()
        .map(|(k, x)| Ok((k.clone(), sustituir(x, &nombres)?)))
        .collect::<Result<_, _>>()?;
    Ok((
        Nodo::Proyecta {
            entrada: Box::new(n),
            campos: salida,
        },
        base.compensation,
    ))
}

/// El sello de una materialización: su clasificación efectiva por columna,
/// colgada de las columnas de su tabla.
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
    use crate::plan::Opaca;
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
                ("id_cliente", "Integer"),
                ("pais", "String"),
                ("total", "Decimal"),
                ("nif", "String"),
            ],
        ))
    }
    fn clientes() -> Nodo {
        Nodo::Lee(lectura(
            "crm",
            "ventas.clientes",
            &[("cid", "Integer"), ("segmento", "String")],
        ))
    }
    fn lineas() -> Nodo {
        Nodo::Lee(lectura(
            "sap",
            "ventas.lineas",
            &[
                ("id_pedido", "Integer"),
                ("sku", "String"),
                ("unidades", "Integer"),
            ],
        ))
    }
    fn hoja(ds: &str, o: &str) -> Hoja {
        (ds.into(), o.into())
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
    fn une(i: Nodo, d: Nodo, a: &str, b: &str) -> Nodo {
        Nodo::Une {
            izquierda: Box::new(i),
            derecha: Box::new(d),
            tipo: Junta::Interna,
            sobre: vec![(a.into(), b.into())],
        }
    }
    fn agrupa(e: Nodo, por: &[&str], ag: &[(&str, Agregado, Option<&str>)]) -> Nodo {
        Nodo::Agrupa {
            entrada: Box::new(e),
            por: por.iter().map(|s| (*s).to_string()).collect(),
            agregados: ag
                .iter()
                .map(|(n, f, s)| {
                    (
                        (*n).to_string(),
                        Agregacion {
                            funcion: *f,
                            sobre: s.map(String::from),
                        },
                    )
                })
                .collect(),
        }
    }
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
    fn nada() -> Clasificacion {
        Clasificacion::default()
    }
    fn ok(plan: &Nodo, m: &Materializacion) -> Rewrite {
        let r = cotejar(plan, m, &nada(), &[]).expect("contesta");
        assert_eq!(
            esquema(&r.plan).unwrap(),
            esquema(plan).unwrap(),
            "el reescrito produce otra cosa"
        );
        r
    }

    // ── 2 · 3 · seal (lo que ya pasaba) ─────────────────────────────────────

    #[test]
    fn una_materializacion_identica_contesta_sin_compensacion() {
        let plan = proyecta(filtra(pedidos(), eq_s("pais", "ES")), &["id", "total"]);
        let r = ok(&plan, &mat("es", plan.clone()));
        assert!(r.compensation.is_empty());
        assert_eq!(r.plan.lecturas()[0].objeto, "cache.es");
    }

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
        let r = ok(&plan, &m);
        assert_eq!(
            r.compensation,
            vec![cmp("total", Comparador::Mayor, dec("100"))]
        );
    }

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
            cotejar(&plan, &m, &nada(), &[]),
            Err(NoContesta::PredicadoNoSubsumido {
                conyunto: Box::new(cmp("total", Comparador::Mayor, dec("100")))
            })
        );
    }

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
        let r = ok(&plan, &m);
        assert_eq!(
            r.compensation,
            vec![cmp("total", Comparador::MayorIgual, dec("10.25"))]
        );
        assert!(matches!(
            cotejar(&m.plan, &mat("grandes", plan.clone()), &nada(), &[]),
            Err(NoContesta::PredicadoNoSubsumido { .. })
        ));
        assert_eq!(decimal("0.10", "0.1"), Some(Ordering::Equal));
        assert_eq!(decimal("9.999", "10.25"), Some(Ordering::Less));
        assert_eq!(decimal("-3", "2"), Some(Ordering::Less));
        assert_eq!(decimal("-0", "0.0"), Some(Ordering::Equal));
        assert_eq!(decimal("-1.5", "-1.25"), Some(Ordering::Less));
    }

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
        let r = ok(&plan, &m);
        let Nodo::Proyecta { campos, .. } = &r.plan else {
            panic!()
        };
        assert_eq!(campos["pais"], Expr::Literal(Valor::Cadena("ES".into())));
    }

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
        let r = ok(&plan, &m);
        assert!(r.compensation.is_empty(), "{:?}", r.compensation);
    }

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
        let critical: Etiquetas = [("gdpr.sensitivity".to_string(), "critical".to_string())].into();
        let m = mat(
            "de_un_nif",
            proyecta(filtra(pedidos(), eq_s("nif", "X")), &["id", "total"]),
        );
        let en_origen = Clasificacion {
            reticulos: [("gdpr.sensitivity".to_string(), gdpr)].into(),
            de_raiz: [(
                Raiz {
                    datasource: "lago".into(),
                    objeto: "ventas.pedidos".into(),
                    campo: "nif".into(),
                },
                critical.clone(),
            )]
            .into(),
        };
        let al_construir = comprobar(&linaje(&m.plan).unwrap(), &en_origen, &Etiquetas::new());
        let sellado = Clasificacion {
            reticulos: en_origen.reticulos.clone(),
            de_raiz: sello(&m, &al_construir.efectivas),
        };
        let plan = proyecta(filtra(pedidos(), eq_s("nif", "X")), &["total"]);
        let r = cotejar(&plan, &m, &sellado, &[]).expect("contesta");
        assert_eq!(r.label_seal["total"], critical);
        let recalculado = comprobar(&linaje(&r.plan).unwrap(), &en_origen, &Etiquetas::new());
        assert!(recalculado.efectivas["total"].is_empty());
    }

    #[test]
    fn una_opaca_solo_implica_si_es_determinista() {
        let opaca = |d| {
            Expr::Opaca(Opaca {
                dialecto: "bigquery".into(),
                texto: "REGEXP_CONTAINS(pais, r'^E')".into(),
                lee: vec!["pais".into()],
                tipo: parse_type("Boolean").unwrap(),
                determinista: d,
            })
        };
        let m = mat(
            "regex",
            proyecta(filtra(pedidos(), opaca(true)), &["id", "pais"]),
        );
        assert!(
            ok(
                &proyecta(filtra(pedidos(), opaca(true)), &["id", "pais"]),
                &m
            )
            .compensation
            .is_empty()
        );
        let m_v = mat(
            "azar",
            proyecta(filtra(pedidos(), opaca(false)), &["id", "pais"]),
        );
        assert!(matches!(
            cotejar(
                &proyecta(filtra(pedidos(), opaca(false)), &["id", "pais"]),
                &m_v,
                &nada(),
                &[]
            ),
            Err(NoContesta::PredicadoNoSubsumido { .. })
        ));
    }

    // ── 1 · join compatibility ──────────────────────────────────────────────

    /// **Mismas hojas, misma junta.** La materialización ya juntó pedidos con
    /// líneas por la misma clave: contesta leyendo la tabla, sin volver a juntar.
    #[test]
    fn una_junta_igual_contesta_sin_volver_a_juntar() {
        let plan = proyecta(
            une(pedidos(), lineas(), "id", "id_pedido"),
            &["id", "sku", "total"],
        );
        let r = ok(&plan, &mat("pl", plan.clone()));
        assert!(r.compensation.is_empty());
        assert_eq!(
            r.plan.lecturas().len(),
            1,
            "no vuelve a juntar: {:?}",
            r.plan
        );
    }

    /// **La materialización junta menos que el plan: se junta de vuelta.** Tiene
    /// pedidos⋈líneas; el plan quiere además clientes. Se enganchan sobre la
    /// clave que la tabla expone.
    #[test]
    fn una_hoja_que_falta_se_junta_de_vuelta_sobre_la_clave_expuesta() {
        let m = mat(
            "pl",
            proyecta(
                une(pedidos(), lineas(), "id", "id_pedido"),
                &["id", "id_cliente", "sku", "total"],
            ),
        );
        let plan = proyecta(
            une(
                une(pedidos(), lineas(), "id", "id_pedido"),
                clientes(),
                "id_cliente",
                "cid",
            ),
            &["id", "sku", "segmento"],
        );
        let r = ok(&plan, &m);
        assert_eq!(r.plan.lecturas().len(), 2);
        let hojas: Vec<&str> = r
            .plan
            .lecturas()
            .iter()
            .map(|l| l.objeto.as_str())
            .collect();
        assert!(
            hojas.contains(&"cache.pl") && hojas.contains(&"ventas.clientes"),
            "{hojas:?}"
        );
        assert!(r.compensation.is_empty(), "{:?}", r.compensation);
    }

    /// Y si la clave para engancharla **no está expuesta**, la hoja no se puede
    /// traer: se dice cuál.
    #[test]
    fn sin_la_clave_expuesta_la_hoja_no_se_trae() {
        let m = mat(
            "pl",
            proyecta(
                une(pedidos(), lineas(), "id", "id_pedido"),
                &["id", "sku", "total"],
            ),
        );
        let plan = proyecta(
            une(
                une(pedidos(), lineas(), "id", "id_pedido"),
                clientes(),
                "id_cliente",
                "cid",
            ),
            &["id", "segmento"],
        );
        assert_eq!(
            cotejar(&plan, &m, &nada(), &[]),
            Err(NoContesta::HojaAusente {
                hoja: hoja("crm", "ventas.clientes")
            })
        );
    }

    /// Una junta que el plan hace **entre columnas que la tabla expone** y la
    /// materialización no hizo, va a compensación como una igualdad.
    #[test]
    fn una_junta_del_plan_que_la_materializacion_no_hizo_es_compensacion() {
        // La materialización tiene las dos hojas... sin juntarlas es un producto
        // cartesiano, así que la construimos con una junta distinta y exponemos
        // las dos columnas.
        let m = mat(
            "cruce",
            proyecta(
                une(pedidos(), lineas(), "id", "id_pedido"),
                &["id", "id_legacy", "id_pedido", "sku"],
            ),
        );
        let plan = proyecta(
            filtra(
                une(pedidos(), lineas(), "id", "id_pedido"),
                Expr::Compara {
                    op: Comparador::Igual,
                    izquierda: Box::new(Expr::campo("id_legacy")),
                    derecha: Box::new(Expr::campo("id_pedido")),
                },
            ),
            &["id", "sku"],
        );
        let r = ok(&plan, &m);
        assert_eq!(r.compensation.len(), 1, "{:?}", r.compensation);
    }

    /// **La junta de más, y por qué hacen falta DOS restricciones.** La
    /// materialización junta pedidos con clientes; el plan solo lee pedidos.
    /// Sin restricciones no; con la única sola, dice que falta la referencial;
    /// con las dos, contesta.
    #[test]
    fn una_junta_de_mas_solo_vale_con_clave_unica_y_referencial() {
        let m = mat(
            "pc",
            proyecta(
                une(pedidos(), clientes(), "id_cliente", "cid"),
                &["id", "total", "segmento"],
            ),
        );
        let plan = proyecta(pedidos(), &["id", "total"]);

        assert_eq!(
            cotejar(&plan, &m, &nada(), &[]),
            Err(NoContesta::JuntaDeMasSinGarantia {
                hoja: hoja("crm", "ventas.clientes"),
                falta: "una clave única en el lado de más (evita duplicar)"
            })
        );
        let unica = Restriccion::Unica {
            hoja: hoja("crm", "ventas.clientes"),
            columnas: vec!["cid".into()],
        };
        assert_eq!(
            cotejar(&plan, &m, &nada(), std::slice::from_ref(&unica)),
            Err(NoContesta::JuntaDeMasSinGarantia {
                hoja: hoja("crm", "ventas.clientes"),
                falta: "una referencial hacia el lado de más (evita perder)"
            })
        );
        let referencial = Restriccion::Referencial {
            desde: (hoja("lago", "ventas.pedidos"), vec!["id_cliente".into()]),
            hacia: (hoja("crm", "ventas.clientes"), vec!["cid".into()]),
        };
        let r = cotejar(&plan, &m, &nada(), &[unica, referencial]).expect("con las dos, sí");
        assert_eq!(esquema(&r.plan).unwrap(), esquema(&plan).unwrap());
        assert_eq!(r.plan.lecturas()[0].objeto, "cache.pc");
    }

    /// Y si la materialización **filtra** por la hoja de más, el plan no lo
    /// implica —no la lee— y no contesta, aunque la junta esté garantizada.
    #[test]
    fn un_filtro_sobre_la_hoja_de_mas_no_lo_implica_el_plan() {
        let m = mat(
            "pc_premium",
            proyecta(
                filtra(
                    une(pedidos(), clientes(), "id_cliente", "cid"),
                    eq_s("segmento", "premium"),
                ),
                &["id", "total"],
            ),
        );
        let plan = proyecta(pedidos(), &["id", "total"]);
        let rs = [
            Restriccion::Unica {
                hoja: hoja("crm", "ventas.clientes"),
                columnas: vec!["cid".into()],
            },
            Restriccion::Referencial {
                desde: (hoja("lago", "ventas.pedidos"), vec!["id_cliente".into()]),
                hacia: (hoja("crm", "ventas.clientes"), vec!["cid".into()]),
            },
        ];
        assert!(matches!(
            cotejar(&plan, &m, &nada(), &rs),
            Err(NoContesta::PredicadoNoSubsumido { .. })
        ));
    }

    /// Una junta externa está fuera del subconjunto, y se dice.
    #[test]
    fn una_junta_externa_esta_fuera() {
        let externa = Nodo::Une {
            izquierda: Box::new(pedidos()),
            derecha: Box::new(lineas()),
            tipo: Junta::Izquierda,
            sobre: vec![("id".into(), "id_pedido".into())],
        };
        let m = mat("p", proyecta(pedidos(), &["id"]));
        assert_eq!(
            cotejar(&proyecta(externa, &["id"]), &m, &nada(), &[]),
            Err(fuera("junta externa"))
        );
    }

    // ── 4 · aggregate computability ─────────────────────────────────────────

    fn ventas_por_pais_y_cliente() -> Nodo {
        agrupa(
            pedidos(),
            &["pais", "id_cliente"],
            &[
                ("suma", Agregado::Suma, Some("total")),
                ("n", Agregado::Cuenta, None),
                ("minimo", Agregado::Minimo, Some("total")),
                ("maximo", Agregado::Maximo, Some("total")),
                ("media", Agregado::Promedio, Some("total")),
            ],
        )
    }

    /// **Misma granularidad: se copia**, sin volver a agregar. Incluido `AVG`.
    #[test]
    fn a_la_misma_granularidad_los_agregados_se_copian() {
        let m = mat("vpc", ventas_por_pais_y_cliente());
        let plan = proyecta(
            agrupa(
                pedidos(),
                &["pais", "id_cliente"],
                &[
                    ("total_pais", Agregado::Suma, Some("total")),
                    ("media", Agregado::Promedio, Some("total")),
                ],
            ),
            &["pais", "total_pais", "media"],
        );
        let r = ok(&plan, &m);
        assert!(
            !format!("{:?}", r.plan).contains("Agrupa"),
            "no debía volver a agregar: {:?}",
            r.plan
        );
    }

    /// **Más gruesa: se enrolla.** `SUM(SUM)`, `SUM(COUNT)`, `MIN(MIN)`, `MAX(MAX)`.
    #[test]
    fn a_una_granularidad_mas_gruesa_los_agregados_se_enrollan() {
        let m = mat("vpc", ventas_por_pais_y_cliente());
        let plan = proyecta(
            agrupa(
                pedidos(),
                &["pais"],
                &[
                    ("total_pais", Agregado::Suma, Some("total")),
                    ("pedidos", Agregado::Cuenta, None),
                    ("menor", Agregado::Minimo, Some("total")),
                    ("mayor", Agregado::Maximo, Some("total")),
                ],
            ),
            &["pais", "total_pais", "pedidos", "menor", "mayor"],
        );
        let r = ok(&plan, &m);
        let Nodo::Proyecta { entrada, .. } = &r.plan else {
            panic!()
        };
        let Nodo::Agrupa { agregados, por, .. } = &**entrada else {
            panic!("{entrada:?}")
        };
        assert_eq!(por.len(), 1);
        assert_eq!(agregados["total_pais"].funcion, Agregado::Suma);
        // `COUNT` se enrolla como `SUM` de las cuentas: es la que se equivoca.
        assert_eq!(agregados["pedidos"].funcion, Agregado::Suma);
        assert_eq!(agregados["pedidos"].sobre.as_deref(), Some("n"));
        assert_eq!(agregados["menor"].funcion, Agregado::Minimo);
        assert_eq!(agregados["mayor"].funcion, Agregado::Maximo);
    }

    /// **`AVG` no se enrolla**, y aquí además no se puede escribir: harían
    /// falta `SUMA` y `CUENTA` aparte, y el álgebra no tiene división.
    #[test]
    fn avg_no_se_enrolla() {
        let m = mat("vpc", ventas_por_pais_y_cliente());
        let plan = proyecta(
            agrupa(
                pedidos(),
                &["pais"],
                &[("media", Agregado::Promedio, Some("total"))],
            ),
            &["pais", "media"],
        );
        assert!(matches!(
            cotejar(&plan, &m, &nada(), &[]),
            Err(NoContesta::AgregadoNoEnrollable {
                funcion: Agregado::Promedio,
                ..
            })
        ));
    }

    /// El plan agrupa por algo que la materialización **ya agregó**: no se puede
    /// volver a separar.
    #[test]
    fn una_agrupacion_mas_fina_no_contesta() {
        let m = mat(
            "vp",
            agrupa(
                pedidos(),
                &["pais"],
                &[("suma", Agregado::Suma, Some("total"))],
            ),
        );
        let plan = proyecta(
            agrupa(
                pedidos(),
                &["pais", "id_cliente"],
                &[("suma", Agregado::Suma, Some("total"))],
            ),
            &["pais", "suma"],
        );
        assert_eq!(
            cotejar(&plan, &m, &nada(), &[]),
            Err(NoContesta::AgrupacionMasFina {
                columna: "id_cliente".into()
            })
        );
    }

    /// Un agregado que la materialización no calculó no está.
    #[test]
    fn un_agregado_que_no_se_calculo_no_esta() {
        let m = mat(
            "vp",
            agrupa(
                pedidos(),
                &["pais"],
                &[("suma", Agregado::Suma, Some("total"))],
            ),
        );
        let plan = proyecta(
            agrupa(
                pedidos(),
                &["pais"],
                &[("mayor", Agregado::Maximo, Some("total"))],
            ),
            &["pais", "mayor"],
        );
        assert_eq!(
            cotejar(&plan, &m, &nada(), &[]),
            Err(NoContesta::AgregadoNoDisponible {
                nombre: "mayor".into()
            })
        );
    }

    /// **La compensación por debajo del agregado** solo vale sobre columnas de
    /// grupo: filtrar por `pais` después de agrupar por `pais` es filtrar
    /// grupos; filtrar por `total` es filtrar filas que ya se sumaron.
    #[test]
    fn la_compensacion_bajo_el_agregado_solo_vale_sobre_columnas_de_grupo() {
        let m = mat("vpc", ventas_por_pais_y_cliente());
        // Sobre una columna de grupo: bien.
        let plan_ok = proyecta(
            agrupa(
                filtra(pedidos(), eq_s("pais", "ES")),
                &["pais"],
                &[("suma", Agregado::Suma, Some("total"))],
            ),
            &["pais", "suma"],
        );
        let r = ok(&plan_ok, &m);
        assert_eq!(r.compensation.len(), 1);
        // Sobre una que no lo es: las filas ya se sumaron.
        let plan_mal = proyecta(
            agrupa(
                filtra(pedidos(), cmp("total", Comparador::Mayor, dec("10"))),
                &["pais"],
                &[("suma", Agregado::Suma, Some("total"))],
            ),
            &["pais", "suma"],
        );
        assert!(matches!(
            cotejar(&plan_mal, &m, &nada(), &[]),
            Err(NoContesta::CompensacionBajoAgregado { .. })
        ));
    }

    /// El `HAVING` del plan se aplica encima, traducido a los nombres nuevos.
    #[test]
    fn el_having_del_plan_se_aplica_encima() {
        let m = mat("vpc", ventas_por_pais_y_cliente());
        let plan = proyecta(
            filtra(
                agrupa(pedidos(), &["pais"], &[("pedidos", Agregado::Cuenta, None)]),
                Expr::Compara {
                    op: Comparador::Mayor,
                    izquierda: Box::new(Expr::campo("pedidos")),
                    derecha: Box::new(Expr::Literal(Valor::Entero(10))),
                },
            ),
            &["pais", "pedidos"],
        );
        let r = ok(&plan, &m);
        let Nodo::Proyecta { entrada, .. } = &r.plan else {
            panic!()
        };
        assert!(matches!(**entrada, Nodo::Filtra { .. }), "{entrada:?}");
    }

    /// Un `HAVING` **en la materialización** está fuera: filtra grupos por
    /// valores agregados, y razonar sobre eso es otro peldaño.
    #[test]
    fn un_having_en_la_materializacion_esta_fuera() {
        let m = mat(
            "grandes",
            filtra(
                agrupa(
                    pedidos(),
                    &["pais"],
                    &[("suma", Agregado::Suma, Some("total"))],
                ),
                cmp("suma", Comparador::Mayor, dec("1000")),
            ),
        );
        let plan = proyecta(
            agrupa(
                pedidos(),
                &["pais"],
                &[("suma", Agregado::Suma, Some("total"))],
            ),
            &["pais", "suma"],
        );
        assert_eq!(
            cotejar(&plan, &m, &nada(), &[]),
            Err(fuera("having en la materialización"))
        );
    }

    /// Uno agrega y el otro no: formas distintas.
    #[test]
    fn uno_agrega_y_el_otro_no() {
        let m = mat("p", proyecta(pedidos(), &["id", "total"]));
        let plan = agrupa(
            pedidos(),
            &["pais"],
            &[("suma", Agregado::Suma, Some("total"))],
        );
        assert_eq!(
            cotejar(&plan, &m, &nada(), &[]),
            Err(NoContesta::FormasDistintas)
        );
    }

    /// Lo que está fuera se dice por su operador.
    #[test]
    fn fuera_del_subconjunto_se_dice_por_su_operador() {
        let m = mat("p", proyecta(pedidos(), &["id"]));
        assert_eq!(
            cotejar(
                &Nodo::Limita {
                    entrada: Box::new(pedidos()),
                    n: 5
                },
                &m,
                &nada(),
                &[]
            ),
            Err(fuera("limita"))
        );
        assert_eq!(
            cotejar(&Nodo::Referencia("v".into()), &m, &nada(), &[]),
            Err(NoContesta::SinExpandir { vista: "v".into() })
        );
    }
}
