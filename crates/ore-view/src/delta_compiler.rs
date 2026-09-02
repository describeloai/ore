//! **Delta Compiler**: el circuito Δ de un plan — qué hay que recomputar cuando
//! llega un cambio, y nada más.
//!
//! La teoría es DBSP (Budiu et al., VLDB'23) y no deja margen:
//!
//! > **`Q^Δ = D ∘ Q ∘ I`** — diferenciar, aplicar la consulta, integrar.
//! > Cualquier circuito se incrementaliza **mecánicamente**.
//!
//! > Aunque `Q` sea una función pura, **`Q^Δ` tiene estado, y ese estado vive
//! > *enteramente* en los operadores de retardo `z⁻¹`**.
//!
//! Esa segunda frase es la que convierte «el estado» de problema difuso en una
//! lista: son los **integradores** —lo acumulado a la entrada de cada operador
//! que no es lineal—, y [`Circuito::estado`] los enumera.
//!
//! # Z-sets
//!
//! Una relación es un multiconjunto con **pesos con signo**: `+1` una fila que
//! está, `+3` una fila repetida tres veces, `-1` una que se retira. Con eso
//! altas y bajas son **lo mismo** —un Z-set que se suma—, y por eso el
//! incremental funciona para cualquier mezcla de cambios y no solo para
//! *appends*. Es lo que los modelos de solo-alta no tienen, y por lo que fallan
//! en la primera baja.
//!
//! # Nuestra álgebra, operador por operador
//!
//! | Operador | Regla | Estado |
//! |---|---|---|
//! | `Proyecta` `Filtra` `Unifica` | **lineales** · `Δσ(R) = σ(ΔR)` | **ninguno** |
//! | `Une` | **bilineal** · `Δ(a⋈b) = Δa⋈Δb + I(a)⋈Δb + Δa⋈I(b)` | `I(a)` e `I(b)` |
//! | `Agrupa` | no lineal · se recomputan **los grupos que el Δ toca** | `I(entrada)` |
//! | `Distingue` | no lineal · se recomputan **las filas que el Δ toca** | `I(entrada)` |
//! | `Limita` | **no incrementalizable en general** | — |
//!
//! Los operadores **sin** estado son exactamente los que el Pushdown Planner
//! empuja al origen. Lo que se queda arriba es lo que cuesta mantener.
//!
//! # Lo que se refusa, y por qué
//!
//! - **`Limita`.** Retirar una fila de dentro del top-N exige conocer la N+1, y
//!   sin orden ni siquiera está definido cuál era el top-N.
//! - **`PROMEDIO`.** Mantenerlo exige `SUMA / CUENTA`, y **el álgebra no tiene
//!   división**. Es la misma razón por la que el View Matcher no lo enrolla, dicha
//!   por segunda vez: guárdense suma y cuenta aparte.
//! - **Una opaca volátil.** El determinismo es precondición de la
//!   incrementalidad — Snowflake lo documenta como *«UDF volátiles → refresco
//!   completo»*—, y `Opaca::determinista` existe para poder saberlo.
//!
//! # Esto no es un motor de ejecución, y hay que decirlo
//!
//! Este módulo **evalúa**: aplica un Δ y produce un Δ, sobre Z-sets en memoria.
//! No contradice *«la ejecución es de otro»*: es la **semántica de referencia**
//! del circuito, la que hace comprobable que aplicar un delta da lo mismo que
//! recomputar. DBSP es una semántica que se puede ejecutar, y esta es su
//! versión sobre nuestra álgebra. Correr esto sobre una tabla del cliente es
//! otra pieza — el Partial State Store y quien lo ejecute—, y no es esta.
//!
//! Por lo mismo, **no evalúa opacas**: su texto es de otro dialecto. Compilan si
//! son deterministas, porque la regla —lineal— es la misma; evaluarlas es
//! [`Evaluacion::Opaca`], y lo dice.
//!
//! # Sin nulos
//!
//! `Valor` no tiene nulo, así que las filas de esta semántica tampoco. `EsNulo`
//! evalúa a falso. Es una limitación de la semántica de referencia, no del IR, y
//! se dice aquí para que nadie la descubra en una prueba.

use crate::filter_tree::Hoja;
use crate::plan::{Agregacion, Agregado, Comparador, Expr, Junta, Nodo, Valor};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

/// Una fila: columna → valor. Los decimales entran **normalizados**, para que
/// `0.10` y `0.1` sean la misma clave al agrupar, juntar o deduplicar.
pub type Fila = BTreeMap<String, Valor>;

/// Un multiconjunto con pesos con signo. Un peso cero **no se guarda**: una fila
/// que se puso y se quitó no está.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Zset(BTreeMap<Fila, i64>);

impl Zset {
    pub fn nuevo() -> Zset {
        Zset::default()
    }

    pub fn de(filas: impl IntoIterator<Item = (Fila, i64)>) -> Zset {
        let mut z = Zset::nuevo();
        for (f, w) in filas {
            z.insertar(f, w);
        }
        z
    }

    /// Suma `peso` a la fila. Normaliza los valores y borra la fila si queda a
    /// cero.
    pub fn insertar(&mut self, fila: Fila, peso: i64) {
        let fila: Fila = fila
            .into_iter()
            .map(|(k, v)| (k, v.normalizado()))
            .collect();
        let w = self.0.entry(fila.clone()).or_insert(0);
        *w += peso;
        if *w == 0 {
            self.0.remove(&fila);
        }
    }

    pub fn sumar(&mut self, otro: &Zset) {
        for (f, w) in &otro.0 {
            self.insertar(f.clone(), *w);
        }
    }

    pub fn negado(&self) -> Zset {
        Zset(self.0.iter().map(|(f, w)| (f.clone(), -w)).collect())
    }

    pub fn menos(&self, otro: &Zset) -> Zset {
        let mut z = self.clone();
        z.sumar(&otro.negado());
        z
    }

    pub fn es_vacio(&self) -> bool {
        self.0.is_empty()
    }

    pub fn filas(&self) -> impl Iterator<Item = (&Fila, i64)> {
        self.0.iter().map(|(f, w)| (f, *w))
    }

    pub fn peso(&self, fila: &Fila) -> i64 {
        self.0.get(fila).copied().unwrap_or(0)
    }

    /// Solo las filas con peso positivo: lo que **está**, como multiconjunto.
    fn presentes(&self) -> impl Iterator<Item = (&Fila, i64)> {
        self.0.iter().filter(|(_, w)| **w > 0).map(|(f, w)| (f, *w))
    }
}

/// Por qué un plan no se compila a un circuito.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoIncrementalizable {
    SinExpandir {
        vista: String,
    },
    /// Retirar una fila de dentro del top-N exige conocer la N+1.
    Limita,
    /// Mantener un promedio exige dividir, y el álgebra no tiene división.
    Promedio {
        nombre: String,
    },
    /// Una opaca que no declara ser determinista puede dar otro valor la
    /// segunda vez, y entonces `Δ` no significa nada.
    OpacaVolatil,
    /// Una junta externa produce filas con columnas ausentes, y esta semántica
    /// no tiene nulos. La regla bilineal es de la interna.
    JuntaExterna,
}

impl NoIncrementalizable {
    pub fn como_texto(&self) -> String {
        match self {
            NoIncrementalizable::SinExpandir { vista } => {
                format!("todavía nombra a `{vista}`: hay que expandir antes")
            }
            NoIncrementalizable::Limita => "`Limita` no se mantiene incrementalmente: retirar una \
                                            fila de dentro del top-N exige conocer la N+1"
                .into(),
            NoIncrementalizable::Promedio { nombre } => format!(
                "`{nombre}` es un promedio y mantenerlo exige dividir, y el álgebra no tiene \
                 división: guárdense SUMA y CUENTA aparte"
            ),
            NoIncrementalizable::OpacaVolatil => "una opaca que no declara ser determinista puede \
                                                  dar otro valor la segunda vez, y entonces Δ no \
                                                  significa nada"
                .into(),
            NoIncrementalizable::JuntaExterna => "una junta externa produce filas con columnas \
                                                  ausentes, y esta semántica no tiene nulos: la \
                                                  regla bilineal es de la interna"
                .into(),
        }
    }
}

/// Un fallo **al evaluar** con la semántica de referencia. Son fallos de esta
/// semántica, no del plan: un plan que cuadra puede no poder evaluarse aquí.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Evaluacion {
    ColumnaAusente(String),
    /// Comparar o sumar valores de tipos distintos.
    TiposDistintos,
    /// Una opaca: su texto es de otro dialecto y aquí no se lee.
    Opaca,
    /// Un entero se salió de `i64`, o un decimal de `i128`.
    Desborde,
    /// Lo que no tiene semántica de referencia: `Limita`, una junta externa, un
    /// promedio, una referencia sin expandir. Se dice cuál.
    SinSemantica(&'static str),
    /// La condición de un filtro no dio un booleano.
    NoEsBooleano,
}

/// Un integrador: qué operador lo exige y qué guarda.
///
/// Es la enumeración del estado de DBSP hecha lista, y describe lo que un
/// almacén de producción tendría que sostener — no lo que esta semántica de
/// referencia guarda, que es siempre la entrada entera.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Estado {
    pub operador: &'static str,
    pub guarda: String,
}

// ── El circuito ─────────────────────────────────────────────────────────────

enum Op {
    Hoja(Hoja),
    Proyecta {
        entrada: Box<Op>,
        campos: BTreeMap<String, Expr>,
    },
    Filtra {
        entrada: Box<Op>,
        predicado: Expr,
    },
    Une {
        izquierda: Box<Op>,
        derecha: Box<Op>,
        sobre: Vec<(String, String)>,
        i_izquierda: Zset,
        i_derecha: Zset,
    },
    Agrupa {
        entrada: Box<Op>,
        por: BTreeSet<String>,
        agregados: BTreeMap<String, Agregacion>,
        integrado: Zset,
    },
    Unifica(Vec<Op>),
    Distingue {
        entrada: Box<Op>,
        integrado: Zset,
    },
}

/// El circuito Δ de un plan, con su estado dentro.
pub struct Circuito {
    raiz: Op,
}

impl Circuito {
    /// **`Q^Δ`.** Falla por lo que no se puede mantener, y lo dice.
    pub fn compilar(plan: &Nodo) -> Result<Circuito, NoIncrementalizable> {
        Ok(Circuito {
            raiz: compilar(plan)?,
        })
    }

    /// **Un paso**: llega un Δ por hoja, sale el Δ de la salida, y el estado
    /// avanza. Una hoja que no aparece en `deltas` no cambió.
    pub fn paso(&mut self, deltas: &BTreeMap<Hoja, Zset>) -> Result<Zset, Evaluacion> {
        self.raiz.paso(deltas)
    }

    /// Los integradores, enumerados. Vacío para un plan lineal, que es lo que
    /// significa que no cuesta nada mantenerlo.
    pub fn estado(&self) -> Vec<Estado> {
        let mut out = Vec::new();
        self.raiz.estado(&mut out);
        out
    }
}

fn compilar(n: &Nodo) -> Result<Op, NoIncrementalizable> {
    Ok(match n {
        Nodo::Referencia(v) => return Err(NoIncrementalizable::SinExpandir { vista: v.clone() }),
        Nodo::Lee(l) => Op::Hoja((l.datasource.clone(), l.objeto.clone())),
        Nodo::Proyecta { entrada, campos } => {
            for x in campos.values() {
                sin_volatiles(x)?;
            }
            Op::Proyecta {
                entrada: Box::new(compilar(entrada)?),
                campos: campos.clone(),
            }
        }
        Nodo::Filtra { entrada, predicado } => {
            sin_volatiles(predicado)?;
            Op::Filtra {
                entrada: Box::new(compilar(entrada)?),
                predicado: predicado.clone(),
            }
        }
        Nodo::Une {
            izquierda,
            derecha,
            tipo,
            sobre,
        } => {
            if *tipo != Junta::Interna {
                return Err(NoIncrementalizable::JuntaExterna);
            }
            Op::Une {
                izquierda: Box::new(compilar(izquierda)?),
                derecha: Box::new(compilar(derecha)?),
                sobre: sobre.clone(),
                i_izquierda: Zset::nuevo(),
                i_derecha: Zset::nuevo(),
            }
        }
        Nodo::Agrupa {
            entrada,
            por,
            agregados,
        } => {
            if let Some((nombre, _)) = agregados
                .iter()
                .find(|(_, a)| a.funcion == Agregado::Promedio)
            {
                return Err(NoIncrementalizable::Promedio {
                    nombre: nombre.clone(),
                });
            }
            Op::Agrupa {
                entrada: Box::new(compilar(entrada)?),
                por: por.clone(),
                agregados: agregados.clone(),
                integrado: Zset::nuevo(),
            }
        }
        Nodo::Unifica(v) => Op::Unifica(v.iter().map(compilar).collect::<Result<_, _>>()?),
        Nodo::Distingue(e) => Op::Distingue {
            entrada: Box::new(compilar(e)?),
            integrado: Zset::nuevo(),
        },
        Nodo::Limita { .. } => return Err(NoIncrementalizable::Limita),
    })
}

fn sin_volatiles(x: &Expr) -> Result<(), NoIncrementalizable> {
    if x.opacas().iter().any(|o| !o.determinista) {
        return Err(NoIncrementalizable::OpacaVolatil);
    }
    Ok(())
}

impl Op {
    fn paso(&mut self, deltas: &BTreeMap<Hoja, Zset>) -> Result<Zset, Evaluacion> {
        match self {
            Op::Hoja(h) => Ok(deltas.get(h).cloned().unwrap_or_default()),

            // Lineal: el Δ se proyecta, y ya.
            Op::Proyecta { entrada, campos } => proyectar(&entrada.paso(deltas)?, campos),

            // Lineal: el Δ se filtra, y ya.
            Op::Filtra { entrada, predicado } => filtrar(&entrada.paso(deltas)?, predicado),

            // Lineal: los Δ se suman.
            Op::Unifica(ramas) => {
                let mut out = Zset::nuevo();
                for r in ramas {
                    out.sumar(&r.paso(deltas)?);
                }
                Ok(out)
            }

            // **Bilineal.** `Δ(a⋈b) = Δa⋈Δb + I(a)⋈Δb + Δa⋈I(b)`, y después los
            // integradores avanzan. El orden importa: los `I` son los de ANTES.
            Op::Une {
                izquierda,
                derecha,
                sobre,
                i_izquierda,
                i_derecha,
            } => {
                let da = izquierda.paso(deltas)?;
                let db = derecha.paso(deltas)?;
                let mut out = juntar(&da, &db, sobre);
                out.sumar(&juntar(i_izquierda, &db, sobre));
                out.sumar(&juntar(&da, i_derecha, sobre));
                i_izquierda.sumar(&da);
                i_derecha.sumar(&db);
                Ok(out)
            }

            // No lineal: se recomputan **los grupos que el Δ toca**, antes y
            // después, y la diferencia es el Δ de salida.
            Op::Agrupa {
                entrada,
                por,
                agregados,
                integrado,
            } => {
                let d = entrada.paso(deltas)?;
                let tocados: BTreeSet<Vec<Valor>> = d.filas().map(|(f, _)| clave(f, por)).collect();
                let antes = agregar(integrado, por, agregados, Some(&tocados))?;
                integrado.sumar(&d);
                let despues = agregar(integrado, por, agregados, Some(&tocados))?;
                Ok(despues.menos(&antes))
            }

            // No lineal: se recomputan **las filas que el Δ toca**.
            Op::Distingue { entrada, integrado } => {
                let d = entrada.paso(deltas)?;
                let mut out = Zset::nuevo();
                for (f, _) in d.filas() {
                    let antes = i64::from(integrado.peso(f) > 0);
                    let despues = i64::from(integrado.peso(f) + d.peso(f) > 0);
                    if despues != antes {
                        out.insertar(f.clone(), despues - antes);
                    }
                }
                integrado.sumar(&d);
                Ok(out)
            }
        }
    }

    fn estado(&self, out: &mut Vec<Estado>) {
        match self {
            Op::Hoja(_) => {}
            Op::Proyecta { entrada, .. } | Op::Filtra { entrada, .. } => entrada.estado(out),
            Op::Unifica(v) => v.iter().for_each(|r| r.estado(out)),
            Op::Une {
                izquierda,
                derecha,
                sobre,
                ..
            } => {
                izquierda.estado(out);
                derecha.estado(out);
                let clave: Vec<&str> = sobre.iter().map(|(a, _)| a.as_str()).collect();
                out.push(Estado {
                    operador: "une",
                    guarda: format!("I(izquierda) indexada por [{}]", clave.join(", ")),
                });
                let clave: Vec<&str> = sobre.iter().map(|(_, b)| b.as_str()).collect();
                out.push(Estado {
                    operador: "une",
                    guarda: format!("I(derecha) indexada por [{}]", clave.join(", ")),
                });
            }
            Op::Agrupa {
                entrada, agregados, ..
            } => {
                entrada.estado(out);
                // Lo que un almacén de producción necesita por agregado — no lo
                // que esta semántica guarda, que es la entrada entera.
                for (nombre, a) in agregados {
                    let que = match a.funcion {
                        Agregado::Suma | Agregado::Cuenta => "un acumulador por grupo",
                        Agregado::Minimo | Agregado::Maximo => {
                            "el multiconjunto del grupo: no es invertible bajo baja"
                        }
                        Agregado::Promedio => "SUMA y CUENTA aparte",
                    };
                    out.push(Estado {
                        operador: "agrupa",
                        guarda: format!("{nombre}: {que}"),
                    });
                }
            }
            Op::Distingue { entrada, .. } => {
                entrada.estado(out);
                out.push(Estado {
                    operador: "distingue",
                    guarda: "una cuenta por fila distinta".into(),
                });
            }
        }
    }
}

// ── La semántica de referencia ──────────────────────────────────────────────

/// **`Q`**, sin incrementalizar: el plan entero sobre las bases enteras. Es
/// contra lo que se comprueba el circuito.
pub fn recomputar(plan: &Nodo, bases: &BTreeMap<Hoja, Zset>) -> Result<Zset, Evaluacion> {
    Ok(match plan {
        Nodo::Referencia(_) => return Err(Evaluacion::SinSemantica("referencia sin expandir")),
        Nodo::Limita { .. } => return Err(Evaluacion::SinSemantica("limita")),
        Nodo::Lee(l) => bases
            .get(&(l.datasource.clone(), l.objeto.clone()))
            .cloned()
            .unwrap_or_default(),
        Nodo::Proyecta { entrada, campos } => proyectar(&recomputar(entrada, bases)?, campos)?,
        Nodo::Filtra { entrada, predicado } => filtrar(&recomputar(entrada, bases)?, predicado)?,
        Nodo::Une {
            izquierda,
            derecha,
            tipo,
            sobre,
        } => {
            if *tipo != Junta::Interna {
                return Err(Evaluacion::SinSemantica("junta externa"));
            }
            juntar(
                &recomputar(izquierda, bases)?,
                &recomputar(derecha, bases)?,
                sobre,
            )
        }
        Nodo::Agrupa {
            entrada,
            por,
            agregados,
        } => agregar(&recomputar(entrada, bases)?, por, agregados, None)?,
        Nodo::Unifica(v) => {
            let mut out = Zset::nuevo();
            for r in v {
                out.sumar(&recomputar(r, bases)?);
            }
            out
        }
        Nodo::Distingue(e) => {
            let z = recomputar(e, bases)?;
            Zset::de(z.presentes().map(|(f, _)| (f.clone(), 1)))
        }
    })
}

fn proyectar(z: &Zset, campos: &BTreeMap<String, Expr>) -> Result<Zset, Evaluacion> {
    let mut out = Zset::nuevo();
    for (f, w) in z.filas() {
        let mut nueva = Fila::new();
        for (k, x) in campos {
            nueva.insert(k.clone(), evaluar(x, f)?);
        }
        out.insertar(nueva, w);
    }
    Ok(out)
}

fn filtrar(z: &Zset, p: &Expr) -> Result<Zset, Evaluacion> {
    let mut out = Zset::nuevo();
    for (f, w) in z.filas() {
        match evaluar(p, f)? {
            Valor::Booleano(true) => out.insertar(f.clone(), w),
            Valor::Booleano(false) => {}
            _ => return Err(Evaluacion::NoEsBooleano),
        }
    }
    Ok(out)
}

fn clave(f: &Fila, cols: &BTreeSet<String>) -> Vec<Valor> {
    cols.iter().filter_map(|c| f.get(c).cloned()).collect()
}

/// Junta interna por igualdad de pares. El peso de una fila juntada es el
/// producto de los pesos: es lo que hace que la regla bilineal cuadre.
fn juntar(a: &Zset, b: &Zset, sobre: &[(String, String)]) -> Zset {
    let ka: BTreeSet<String> = sobre.iter().map(|(x, _)| x.clone()).collect();
    let kb: BTreeSet<String> = sobre.iter().map(|(_, y)| y.clone()).collect();
    // Los pares van en el orden de `sobre`, no en el del BTreeSet, para que la
    // clave de un lado se corresponda posición a posición con la del otro.
    let clave_a = |f: &Fila| -> Vec<Valor> {
        sobre
            .iter()
            .filter_map(|(x, _)| f.get(x).cloned())
            .collect()
    };
    let clave_b = |f: &Fila| -> Vec<Valor> {
        sobre
            .iter()
            .filter_map(|(_, y)| f.get(y).cloned())
            .collect()
    };
    let _ = (&ka, &kb);
    let mut indice: BTreeMap<Vec<Valor>, Vec<(&Fila, i64)>> = BTreeMap::new();
    for (f, w) in b.filas() {
        indice.entry(clave_b(f)).or_default().push((f, w));
    }
    let mut out = Zset::nuevo();
    for (fa, wa) in a.filas() {
        if let Some(casan) = indice.get(&clave_a(fa)) {
            for (fb, wb) in casan {
                let mut fila = fa.clone();
                fila.extend(fb.iter().map(|(k, v)| (k.clone(), v.clone())));
                out.insertar(fila, wa * wb);
            }
        }
    }
    out
}

/// Agrupa y agrega **lo presente**. Si `solo` está, únicamente esos grupos: es
/// lo que hace que el paso del agregado cueste lo que el Δ toca.
fn agregar(
    z: &Zset,
    por: &BTreeSet<String>,
    agregados: &BTreeMap<String, Agregacion>,
    solo: Option<&BTreeSet<Vec<Valor>>>,
) -> Result<Zset, Evaluacion> {
    let mut grupos: BTreeMap<Vec<Valor>, Vec<(&Fila, i64)>> = BTreeMap::new();
    for (f, w) in z.presentes() {
        let k = clave(f, por);
        if solo.is_none_or(|s| s.contains(&k)) {
            grupos.entry(k).or_default().push((f, w));
        }
    }
    let mut out = Zset::nuevo();
    for (k, filas) in grupos {
        let mut fila: Fila = por.iter().cloned().zip(k).collect();
        for (nombre, a) in agregados {
            let v = match (a.funcion, &a.sobre) {
                (Agregado::Cuenta, _) => Valor::Entero(filas.iter().map(|(_, w)| w).sum()),
                (Agregado::Promedio, _) => return Err(Evaluacion::SinSemantica("promedio")),
                (_, None) => return Err(Evaluacion::ColumnaAusente(nombre.clone())),
                (Agregado::Suma, Some(c)) => {
                    let mut acc: Option<Valor> = None;
                    for (f, w) in &filas {
                        let v = f
                            .get(c)
                            .ok_or_else(|| Evaluacion::ColumnaAusente(c.clone()))?;
                        let v = por_peso(v, *w)?;
                        acc = Some(match acc {
                            None => v,
                            Some(a) => sumar_valores(&a, &v)?,
                        });
                    }
                    acc.ok_or_else(|| Evaluacion::ColumnaAusente(c.clone()))?
                }
                (f, Some(c)) => {
                    let mut mejor: Option<&Valor> = None;
                    for (fila, _) in &filas {
                        let v = fila
                            .get(c)
                            .ok_or_else(|| Evaluacion::ColumnaAusente(c.clone()))?;
                        mejor = Some(match mejor {
                            None => v,
                            Some(m) => {
                                let o = v.comparar(m).ok_or(Evaluacion::TiposDistintos)?;
                                let gana = match f {
                                    Agregado::Minimo => o == Ordering::Less,
                                    _ => o == Ordering::Greater,
                                };
                                if gana { v } else { m }
                            }
                        });
                    }
                    mejor
                        .cloned()
                        .ok_or_else(|| Evaluacion::ColumnaAusente(c.clone()))?
                }
            };
            fila.insert(nombre.clone(), v);
        }
        out.insertar(fila, 1);
    }
    Ok(out)
}

// ── Expresiones ─────────────────────────────────────────────────────────────

fn evaluar(x: &Expr, f: &Fila) -> Result<Valor, Evaluacion> {
    Ok(match x {
        Expr::Campo(c) => f
            .get(c)
            .cloned()
            .ok_or_else(|| Evaluacion::ColumnaAusente(c.clone()))?,
        Expr::Literal(v) => v.clone().normalizado(),
        Expr::Compara {
            op,
            izquierda,
            derecha,
        } => {
            let (a, b) = (evaluar(izquierda, f)?, evaluar(derecha, f)?);
            let o = a.comparar(&b).ok_or(Evaluacion::TiposDistintos)?;
            Valor::Booleano(match op {
                Comparador::Igual => o == Ordering::Equal,
                Comparador::Distinto => o != Ordering::Equal,
                Comparador::Menor => o == Ordering::Less,
                Comparador::MenorIgual => o != Ordering::Greater,
                Comparador::Mayor => o == Ordering::Greater,
                Comparador::MayorIgual => o != Ordering::Less,
            })
        }
        Expr::EnConjunto { campo, valores } => {
            let v = f
                .get(campo)
                .ok_or_else(|| Evaluacion::ColumnaAusente(campo.clone()))?;
            Valor::Booleano(
                valores
                    .iter()
                    .any(|w| v.comparar(w) == Some(Ordering::Equal)),
            )
        }
        // Sin nulos en esta semántica: nada es nulo.
        Expr::EsNulo(e) => {
            evaluar(e, f)?;
            Valor::Booleano(false)
        }
        Expr::Y(v) => {
            for e in v {
                if evaluar(e, f)? != Valor::Booleano(true) {
                    return Ok(Valor::Booleano(false));
                }
            }
            Valor::Booleano(true)
        }
        Expr::O(v) => {
            for e in v {
                if evaluar(e, f)? == Valor::Booleano(true) {
                    return Ok(Valor::Booleano(true));
                }
            }
            Valor::Booleano(false)
        }
        Expr::No(e) => match evaluar(e, f)? {
            Valor::Booleano(b) => Valor::Booleano(!b),
            _ => return Err(Evaluacion::NoEsBooleano),
        },
        Expr::Opaca(_) => return Err(Evaluacion::Opaca),
    })
}

// ── Aritmética exacta, sin coma flotante ────────────────────────────────────

/// Un decimal como `(mantisa, escala)`: `12.30` → `(1230, 2)`.
fn escalado(s: &str) -> Result<(i128, u32), Evaluacion> {
    let (neg, s) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s),
    };
    let (ent, frac) = s.split_once('.').unwrap_or((s, ""));
    let digitos = format!("{ent}{frac}");
    if digitos.is_empty() || !digitos.chars().all(|c| c.is_ascii_digit()) {
        return Err(Evaluacion::TiposDistintos);
    }
    let m: i128 = digitos.parse().map_err(|_| Evaluacion::Desborde)?;
    Ok((if neg { -m } else { m }, frac.len() as u32))
}

fn desescalar(m: i128, escala: u32) -> Valor {
    let neg = m < 0;
    let s = m.unsigned_abs().to_string();
    let e = escala as usize;
    let s = if s.len() <= e {
        format!("0.{}{s}", "0".repeat(e - s.len()))
    } else if e == 0 {
        s
    } else {
        format!("{}.{}", &s[..s.len() - e], &s[s.len() - e..])
    };
    Valor::Decimal(format!("{}{s}", if neg { "-" } else { "" })).normalizado()
}

fn sumar_valores(a: &Valor, b: &Valor) -> Result<Valor, Evaluacion> {
    Ok(match (a, b) {
        (Valor::Entero(x), Valor::Entero(y)) => {
            Valor::Entero(x.checked_add(*y).ok_or(Evaluacion::Desborde)?)
        }
        (Valor::Decimal(x), Valor::Decimal(y)) => {
            let ((mx, ex), (my, ey)) = (escalado(x)?, escalado(y)?);
            let e = ex.max(ey);
            let mx = mx
                .checked_mul(10i128.pow(e - ex))
                .ok_or(Evaluacion::Desborde)?;
            let my = my
                .checked_mul(10i128.pow(e - ey))
                .ok_or(Evaluacion::Desborde)?;
            desescalar(mx.checked_add(my).ok_or(Evaluacion::Desborde)?, e)
        }
        _ => return Err(Evaluacion::TiposDistintos),
    })
}

/// Un valor repetido `w` veces, para sumarlo de una vez.
fn por_peso(v: &Valor, w: i64) -> Result<Valor, Evaluacion> {
    Ok(match v {
        Valor::Entero(x) => Valor::Entero(x.checked_mul(w).ok_or(Evaluacion::Desborde)?),
        Valor::Decimal(d) => {
            let (m, e) = escalado(d)?;
            desescalar(m.checked_mul(i128::from(w)).ok_or(Evaluacion::Desborde)?, e)
        }
        _ => return Err(Evaluacion::TiposDistintos),
    })
}

// ── Comprobaciones ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{Lectura, Opaca};
    use ore_core::types::parse_type;

    /// Un generador determinista y pequeño. Sin dependencias: `rand` traería
    /// `getrandom`, y este árbol no enlaza contra el sistema operativo.
    struct Lcg(u64);
    impl Lcg {
        fn nuevo(semilla: u64) -> Lcg {
            Lcg(semilla)
        }
        fn siguiente(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 33
        }
        fn hasta(&mut self, n: usize) -> usize {
            (self.siguiente() % n as u64) as usize
        }
    }

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
            &[("id", "Integer"), ("pais", "String"), ("total", "Decimal")],
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
    const PEDIDOS: (&str, &str) = ("lago", "ventas.pedidos");
    const LINEAS: (&str, &str) = ("sap", "ventas.lineas");
    fn hoja(h: (&str, &str)) -> Hoja {
        (h.0.into(), h.1.into())
    }

    fn fila(pares: &[(&str, Valor)]) -> Fila {
        pares
            .iter()
            .map(|(k, v)| ((*k).to_string(), v.clone()))
            .collect()
    }
    fn s(x: &str) -> Valor {
        Valor::Cadena(x.into())
    }
    fn n(x: i64) -> Valor {
        Valor::Entero(x)
    }
    fn d(x: &str) -> Valor {
        Valor::Decimal(x.into())
    }

    /// Una fila de pedidos al azar, de un dominio pequeño para que haya
    /// repeticiones y bajas de verdad.
    fn pedido_al_azar(g: &mut Lcg) -> Fila {
        let paises = ["ES", "PT", "FR"];
        let totales = ["10", "10.50", "0.5", "-3.25", "100"];
        fila(&[
            ("id", n(g.hasta(6) as i64)),
            ("pais", s(paises[g.hasta(3)])),
            ("total", d(totales[g.hasta(5)])),
        ])
    }
    fn linea_al_azar(g: &mut Lcg) -> Fila {
        let skus = ["A", "B"];
        fila(&[
            ("id_pedido", n(g.hasta(6) as i64)),
            ("sku", s(skus[g.hasta(2)])),
            ("unidades", n(1 + g.hasta(3) as i64)),
        ])
    }

    /// **LA PRUEBA QUE VALE.** Para un plan, `pasos` rondas de altas **y bajas**
    /// mezcladas por hoja; en cada ronda, lo acumulado por el circuito tiene que
    /// ser **igual** a recomputar el plan sobre las bases acumuladas.
    ///
    /// Igual en cada ronda, no solo al final: un error que se compense dos rondas
    /// después seguiría siendo un error.
    fn escenario(plan: &Nodo, semilla: u64, pasos: usize) {
        let mut g = Lcg::nuevo(semilla);
        let mut circuito = Circuito::compilar(plan).expect("se compila");
        let mut bases: BTreeMap<Hoja, Zset> = BTreeMap::new();
        let mut acumulado = Zset::nuevo();

        for ronda in 0..pasos {
            let mut deltas: BTreeMap<Hoja, Zset> = BTreeMap::new();
            for (h, genera) in [
                (hoja(PEDIDOS), pedido_al_azar as fn(&mut Lcg) -> Fila),
                (hoja(LINEAS), linea_al_azar),
            ] {
                let mut delta = Zset::nuevo();
                // Altas: entre 0 y 3.
                for _ in 0..g.hasta(4) {
                    delta.insertar(genera(&mut g), 1);
                }
                // Bajas: entre 0 y 2, de filas que están.
                let presentes: Vec<Fila> = bases
                    .get(&h)
                    .map(|z| z.presentes().map(|(f, _)| f.clone()).collect())
                    .unwrap_or_default();
                for _ in 0..g.hasta(3) {
                    if !presentes.is_empty() {
                        let f = presentes[g.hasta(presentes.len())].clone();
                        delta.insertar(f, -1);
                    }
                }
                if !delta.es_vacio() {
                    bases.entry(h.clone()).or_default().sumar(&delta);
                    deltas.insert(h, delta);
                }
            }
            let salida = circuito.paso(&deltas).expect("se evalúa");
            acumulado.sumar(&salida);
            let esperado = recomputar(plan, &bases).expect("se recomputa");
            assert_eq!(
                acumulado, esperado,
                "ronda {ronda}: el incremental diverge del recómputo\n  plan: {plan:?}"
            );
        }
    }

    fn cmp(campo: &str, op: Comparador, v: Valor) -> Expr {
        Expr::Compara {
            op,
            izquierda: Box::new(Expr::campo(campo)),
            derecha: Box::new(Expr::Literal(v)),
        }
    }
    fn agrupa(e: Nodo, por: &[&str], ag: &[(&str, Agregado, Option<&str>)]) -> Nodo {
        Nodo::Agrupa {
            entrada: Box::new(e),
            por: por.iter().map(|s| (*s).to_string()).collect(),
            agregados: ag
                .iter()
                .map(|(nm, f, sb)| {
                    (
                        (*nm).to_string(),
                        Agregacion {
                            funcion: *f,
                            sobre: sb.map(String::from),
                        },
                    )
                })
                .collect(),
        }
    }

    /// Lineales: filtrar y proyectar. Sin estado, y el circuito lo dice.
    #[test]
    fn filtrar_y_proyectar_son_lineales_y_no_guardan_nada() {
        let plan = Nodo::Proyecta {
            entrada: Box::new(Nodo::Filtra {
                entrada: Box::new(pedidos()),
                predicado: Expr::Y(vec![
                    cmp("pais", Comparador::Distinto, s("FR")),
                    cmp("total", Comparador::Mayor, d("0")),
                ]),
            }),
            campos: [
                ("id".to_string(), Expr::campo("id")),
                ("total".to_string(), Expr::campo("total")),
            ]
            .into(),
        };
        assert!(Circuito::compilar(&plan).unwrap().estado().is_empty());
        for semilla in 1..=5 {
            escenario(&plan, semilla, 40);
        }
    }

    /// **La regla bilineal.** `Δ(a⋈b) = Δa⋈Δb + I(a)⋈Δb + Δa⋈I(b)`, con los `I` de
    /// antes. Es la que se equivoca si se actualiza un integrador antes de tiempo.
    #[test]
    fn la_junta_es_bilineal_y_guarda_los_dos_lados() {
        let plan = Nodo::Une {
            izquierda: Box::new(pedidos()),
            derecha: Box::new(lineas()),
            tipo: Junta::Interna,
            sobre: vec![("id".into(), "id_pedido".into())],
        };
        let e = Circuito::compilar(&plan).unwrap().estado();
        assert_eq!(e.len(), 2, "{e:?}");
        assert!(e[0].guarda.contains("[id]") && e[1].guarda.contains("[id_pedido]"));
        for semilla in 1..=8 {
            escenario(&plan, semilla, 40);
        }
    }

    /// Los agregados: `SUMA` de decimales exacta, `CUENTA` con pesos, `MIN` y
    /// `MAX` que **no son invertibles bajo baja** — y por eso se recomputa el
    /// grupo tocado, que es lo que hace que una baja del mínimo dé el siguiente.
    #[test]
    fn los_agregados_se_mantienen_bajo_altas_y_bajas() {
        let plan = agrupa(
            pedidos(),
            &["pais"],
            &[
                ("suma", Agregado::Suma, Some("total")),
                ("cuantos", Agregado::Cuenta, None),
                ("menor", Agregado::Minimo, Some("total")),
                ("mayor", Agregado::Maximo, Some("total")),
            ],
        );
        let e = Circuito::compilar(&plan).unwrap().estado();
        assert!(e.iter().any(|x| x.guarda.contains("acumulador")));
        assert!(e.iter().any(|x| x.guarda.contains("multiconjunto")));
        for semilla in 1..=8 {
            escenario(&plan, semilla, 40);
        }
    }

    /// Y `SUMA` de enteros, con pesos: una fila repetida tres veces suma tres.
    #[test]
    fn la_suma_de_enteros_cuenta_las_repeticiones() {
        let plan = agrupa(
            lineas(),
            &["sku"],
            &[("unidades", Agregado::Suma, Some("unidades"))],
        );
        for semilla in 1..=5 {
            escenario(&plan, semilla, 40);
        }
    }

    /// `Distingue`: una fila que se pone dos veces está una; se quita una y sigue
    /// estando; se quita la otra y deja de estar.
    #[test]
    fn distinguir_se_mantiene_por_cuentas() {
        let plan = Nodo::Distingue(Box::new(Nodo::Proyecta {
            entrada: Box::new(pedidos()),
            campos: [("pais".to_string(), Expr::campo("pais"))].into(),
        }));
        let e = Circuito::compilar(&plan).unwrap().estado();
        assert_eq!(e.len(), 1);
        for semilla in 1..=5 {
            escenario(&plan, semilla, 40);
        }
    }

    /// Y la composición: un agregado sobre una junta filtrada, que es la forma
    /// de una vista de verdad. Todas las reglas a la vez.
    #[test]
    fn un_agregado_sobre_una_junta_filtrada_se_mantiene() {
        let plan = agrupa(
            Nodo::Filtra {
                entrada: Box::new(Nodo::Une {
                    izquierda: Box::new(pedidos()),
                    derecha: Box::new(lineas()),
                    tipo: Junta::Interna,
                    sobre: vec![("id".into(), "id_pedido".into())],
                }),
                predicado: cmp("pais", Comparador::Distinto, s("FR")),
            },
            &["pais", "sku"],
            &[
                ("unidades", Agregado::Suma, Some("unidades")),
                ("pedidos", Agregado::Cuenta, None),
                ("mayor_total", Agregado::Maximo, Some("total")),
            ],
        );
        for semilla in 1..=8 {
            escenario(&plan, semilla, 30);
        }
    }

    /// Una unión de dos filtros suma sus deltas.
    #[test]
    fn la_union_suma_los_deltas() {
        let rama = |p: &str| Nodo::Filtra {
            entrada: Box::new(pedidos()),
            predicado: cmp("pais", Comparador::Igual, s(p)),
        };
        let plan = Nodo::Unifica(vec![rama("ES"), rama("PT")]);
        for semilla in 1..=5 {
            escenario(&plan, semilla, 30);
        }
    }

    /// **Lo que se refusa, con su motivo.**
    #[test]
    fn lo_que_no_se_mantiene_se_dice() {
        let limita = Nodo::Limita {
            entrada: Box::new(pedidos()),
            n: 5,
        };
        assert_eq!(
            Circuito::compilar(&limita).err(),
            Some(NoIncrementalizable::Limita)
        );

        let media = agrupa(
            pedidos(),
            &["pais"],
            &[("media", Agregado::Promedio, Some("total"))],
        );
        let e = Circuito::compilar(&media).err().unwrap();
        assert_eq!(
            e,
            NoIncrementalizable::Promedio {
                nombre: "media".into()
            }
        );
        assert!(e.como_texto().contains("división"), "{}", e.como_texto());

        let volatil = Nodo::Filtra {
            entrada: Box::new(pedidos()),
            predicado: Expr::Opaca(Opaca {
                dialecto: "bigquery".into(),
                texto: "RAND() > 0.5".into(),
                lee: vec![],
                tipo: parse_type("Boolean").unwrap(),
                determinista: false,
            }),
        };
        assert_eq!(
            Circuito::compilar(&volatil).err(),
            Some(NoIncrementalizable::OpacaVolatil)
        );
        assert_eq!(
            Circuito::compilar(&Nodo::Referencia("v".into())).err(),
            Some(NoIncrementalizable::SinExpandir { vista: "v".into() })
        );
        let externa = Nodo::Une {
            izquierda: Box::new(pedidos()),
            derecha: Box::new(lineas()),
            tipo: Junta::Izquierda,
            sobre: vec![("id".into(), "id_pedido".into())],
        };
        assert_eq!(
            Circuito::compilar(&externa).err(),
            Some(NoIncrementalizable::JuntaExterna)
        );
    }

    /// Una opaca **determinista** compila —la regla es lineal— y no se evalúa:
    /// su texto es de otro dialecto. Las dos cosas, y se distinguen.
    #[test]
    fn una_opaca_determinista_compila_pero_no_se_evalua() {
        let plan = Nodo::Filtra {
            entrada: Box::new(pedidos()),
            predicado: Expr::Opaca(Opaca {
                dialecto: "bigquery".into(),
                texto: "REGEXP_CONTAINS(pais, r'^E')".into(),
                lee: vec!["pais".into()],
                tipo: parse_type("Boolean").unwrap(),
                determinista: true,
            }),
        };
        let mut c = Circuito::compilar(&plan).expect("la regla es lineal");
        let mut deltas = BTreeMap::new();
        deltas.insert(
            hoja(PEDIDOS),
            Zset::de([(
                fila(&[("id", n(1)), ("pais", s("ES")), ("total", d("1"))]),
                1,
            )]),
        );
        assert_eq!(c.paso(&deltas), Err(Evaluacion::Opaca));
    }

    /// La aritmética es exacta: `0.1 + 0.2` es `0.3`, no `0.30000000000000004`.
    #[test]
    fn la_suma_de_decimales_es_exacta() {
        assert_eq!(sumar_valores(&d("0.1"), &d("0.2")).unwrap(), d("0.3"));
        assert_eq!(sumar_valores(&d("10.50"), &d("-3.25")).unwrap(), d("7.25"));
        assert_eq!(sumar_valores(&d("0.5"), &d("-0.5")).unwrap(), d("0"));
        assert_eq!(por_peso(&d("0.5"), 3).unwrap(), d("1.5"));
        assert_eq!(por_peso(&n(7), -2).unwrap(), n(-14));
        assert_eq!(
            sumar_valores(&d("1"), &n(1)),
            Err(Evaluacion::TiposDistintos)
        );
    }

    /// Un Z-set no guarda ceros: lo que se pone y se quita **no está**.
    #[test]
    fn un_zset_no_guarda_ceros() {
        let f = fila(&[("id", n(1))]);
        let mut z = Zset::nuevo();
        z.insertar(f.clone(), 2);
        z.insertar(f.clone(), -2);
        assert!(z.es_vacio());
        // Y `0.10` y `0.1` son la misma fila.
        let mut z = Zset::nuevo();
        z.insertar(fila(&[("t", d("0.10"))]), 1);
        z.insertar(fila(&[("t", d("0.1"))]), 1);
        assert_eq!(z.filas().count(), 1);
        assert_eq!(z.peso(&fila(&[("t", d("0.1"))])), 2);
    }
}
