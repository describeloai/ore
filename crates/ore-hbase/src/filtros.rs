//! **Que se empuja a HBase y que se evalua aqui**, y como se reparte el trabajo. Todo puro (sin servidor).
//!
//! HBase solo empuja con eficacia dos cosas: el **rango de la clave de fila** y el de tiempo (ficha 0072, §7.4).
//! Un filtro sobre `_clave` (`eq`, `in`, `lt`, `le`, `gt`, `ge`, `isNull`, `isNotNull`) se convierte en
//! **intervalos de clave** `[ini, fin)` que se piden al servidor y que, por ser exactos, no se vuelven a evaluar.
//! Cualquier otro (`neq` sobre la clave, los filtros de una clave cuya codificacion no conserva el orden, y todo lo
//! que sea sobre celdas) es un **residual** que se evalua en el conector sobre el **valor ya decodificado** de cada
//! fila, con la semantica de SQL: **un nulo no cumple nada** salvo `isNull`.
//!
//! # Cuando un rango sobre la clave es exacto
//!
//! Solo si el orden de los bytes es el orden de los valores:
//!
//! | clave | ¿se empuja? |
//! |---|---|
//! | `utf8`, `bytes` | si: el orden de bytes es el lexicografico, el de las cadenas UTF-8 |
//! | `u32be`, `u64be` | si: big-endian sin signo ordena como el numero |
//! | `i8be` … `i64be` | **si, partiendo en dos**: en complemento a dos los negativos (`0x80…`) van *despues* de los positivos (`0x00…`), asi que `id < 5` son dos tramos |
//! | `epoch-ms` de `DateTime`/`DateTimeTz` | como `i64be`, convirtiendo microsegundos a milisegundos con el redondeo que toca |
//! | `f32be`, `f64be`, `decimal`, `texto`, `bool`, `epoch-ms` de `Date` | no: es un residual |

use crate::codificacion::{Codificacion, codificar};
use crate::esquema::{CLAVE, Largo, ParteClave, Tipo};
use crate::lote::de_base64;
use ore_base::tipos::Valor;
use ore_driver::Valor as Derecha;
use ore_driver::{Fallo, Filtro};
use std::cmp::Ordering;

/// `[ini, fin)`; un `fin` vacio es «sin limite».
pub type Intervalo = (Vec<u8>, Vec<u8>);

/// Un filtro que se evalua sobre cada fila.
#[derive(Debug, Clone, PartialEq)]
pub struct Residual {
    /// `_clave` o `familia:cualificador`.
    pub columna: String,
    pub operador: String,
    /// Los valores de la derecha, en el tipo de la columna.
    pub valores: Vec<Valor>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// Lo que se pide al servidor: ordenado, sin solapes. Vacio: ninguna fila puede cumplirlo.
    pub intervalos: Vec<Intervalo>,
    pub residuales: Vec<Residual>,
}

/// La clave inmediatamente siguiente en orden de bytes.
pub fn siguiente(clave: &[u8]) -> Vec<u8> {
    let mut s = clave.to_vec();
    s.push(0);
    s
}

fn todo() -> Vec<Intervalo> {
    vec![(Vec::new(), Vec::new())]
}

/// El menor de dos finales exclusivos, donde vacio es «sin limite».
fn menor_fin(a: &[u8], b: &[u8]) -> Vec<u8> {
    match (a.is_empty(), b.is_empty()) {
        (true, _) => b.to_vec(),
        (_, true) => a.to_vec(),
        _ => a.min(b).to_vec(),
    }
}

/// Las partes comunes de cada intervalo de `a` con cada uno de `b`, sin mezclar las que se tocan.
pub fn recortar(a: &[Intervalo], b: &[Intervalo]) -> Vec<Intervalo> {
    let mut sal = Vec::new();
    for (ai, af) in a {
        for (bi, bf) in b {
            let ini = ai.max(bi).clone();
            let fin = menor_fin(af, bf);
            if fin.is_empty() || ini < fin {
                sal.push((ini, fin));
            }
        }
    }
    sal
}

/// Ordena y une los que se solapan o se tocan.
pub fn normalizar(mut v: Vec<Intervalo>) -> Vec<Intervalo> {
    v.sort();
    let mut sal: Vec<Intervalo> = Vec::new();
    for (ini, fin) in v {
        if let Some((_, ult_fin)) = sal.last_mut() {
            // el anterior llega hasta el infinito, o hasta mas alla de donde empieza este
            if ult_fin.is_empty() || ini <= *ult_fin {
                if !ult_fin.is_empty() && (fin.is_empty() || fin > *ult_fin) {
                    *ult_fin = fin;
                }
                continue;
            }
        }
        sal.push((ini, fin));
    }
    sal
}

/// La interseccion de dos conjuntos de intervalos.
pub fn interseccion(a: &[Intervalo], b: &[Intervalo]) -> Vec<Intervalo> {
    normalizar(recortar(a, b))
}

/// Parte los intervalos por los limites de las regiones: cada trozo cae en una sola. Es el reparto de trabajo.
pub fn por_regiones(intervalos: &[Intervalo], regiones: &[Intervalo]) -> Vec<Intervalo> {
    let mut sal = Vec::new();
    for i in intervalos {
        sal.extend(recortar(std::slice::from_ref(i), regiones));
    }
    sal
}

/// Reparte `trozos` en hasta `n` grupos contiguos y de tamano parecido.
pub fn repartir<T: Clone>(trozos: &[T], n: usize) -> Vec<Vec<T>> {
    let g = n.min(trozos.len());
    (0..g)
        .map(|i| trozos[i * trozos.len() / g..(i + 1) * trozos.len() / g].to_vec())
        .collect()
}

// ── El orden de los bytes de una clave ──────────────────────────────────────────────────────────────────────────

/// Los valores enteros que caben en la clave: de `min` a `max`, en `ancho` bytes, y cuanto vale cada uno en la unidad
/// del tipo (1 si es un entero; 1000 si la clave son milisegundos y el tipo guarda microsegundos).
#[derive(Debug, Clone, Copy)]
struct Dominio {
    min: i128,
    max: i128,
    ancho: usize,
    firmado: bool,
    factor: i128,
}

enum Orden {
    /// El orden de bytes es el lexicografico de la cadena o de los bytes.
    Lexico,
    /// Enteros de ancho fijo.
    Entero(Dominio),
    /// El orden de los bytes no es el de los valores: no se empuja.
    NoEmpuja,
}

fn orden_de(t: &Tipo) -> Orden {
    use Codificacion::*;
    let d = |min: i128, max: i128, ancho, firmado, factor| {
        Orden::Entero(Dominio {
            min,
            max,
            ancho,
            firmado,
            factor,
        })
    };
    match t.cod {
        Utf8 | Bytes => Orden::Lexico,
        I8Be => d(i128::from(i8::MIN), i128::from(i8::MAX), 1, true, 1),
        I16Be => d(i128::from(i16::MIN), i128::from(i16::MAX), 2, true, 1),
        I32Be => d(i128::from(i32::MIN), i128::from(i32::MAX), 4, true, 1),
        I64Be => d(i128::from(i64::MIN), i128::from(i64::MAX), 8, true, 1),
        U32Be => d(0, i128::from(u32::MAX), 4, false, 1),
        // mas alla de i64::MAX no se decodifica (no cabe en un Integer)
        U64Be => d(0, i128::from(i64::MAX), 8, false, 1),
        EpochMs
            if matches!(
                t.fisico,
                ore_base::tipos::Fisico::FechaHora | ore_base::tipos::Fisico::Instante
            ) =>
        {
            d(i128::from(i64::MIN), i128::from(i64::MAX), 8, true, 1000)
        }
        _ => Orden::NoEmpuja,
    }
}

fn piso(a: i128, b: i128) -> i128 {
    a.div_euclid(b)
}

fn techo(a: i128, b: i128) -> i128 {
    -((-a).div_euclid(b))
}

/// Las cotas, inclusivas y en el dominio de la clave, de `operador valor` (con el valor en la unidad del tipo).
/// `None`: ningun valor de la clave lo cumple.
fn cotas(op: &str, l: i128, d: &Dominio) -> Option<(i128, i128)> {
    let k = d.factor;
    let (lo, hi) = match op {
        "eq" if l.rem_euclid(k) == 0 => (l / k, l / k),
        "eq" => return None,
        "ge" => (techo(l, k), d.max),
        "gt" => (piso(l, k) + 1, d.max),
        "le" => (d.min, piso(l, k)),
        "lt" => (d.min, techo(l, k) - 1),
        _ => return None,
    };
    let (lo, hi) = (lo.max(d.min), hi.min(d.max));
    (lo <= hi).then_some((lo, hi))
}

/// Los bytes de un entero del dominio: complemento a dos, big-endian, del ancho de la clave.
fn bytes_de(x: i128, d: &Dominio) -> Vec<u8> {
    x.to_be_bytes()[16 - d.ancho..].to_vec()
}

/// Los intervalos de bytes de los enteros `[lo, hi]` del dominio. En un dominio con signo son dos: los no negativos
/// (`0x00…`) y, **despues**, los negativos (`0x80…`).
fn intervalos_enteros(d: &Dominio, lo: i128, hi: i128) -> Vec<Intervalo> {
    let trozos: Vec<(i128, i128)> = if d.firmado {
        let mut v = Vec::new();
        if hi >= 0 {
            v.push((lo.max(0), hi));
        }
        if lo < 0 {
            v.push((lo, hi.min(-1)));
        }
        v
    } else {
        vec![(lo, hi)]
    };
    trozos
        .into_iter()
        .map(|(a, b)| (bytes_de(a, d), siguiente(&bytes_de(b, d))))
        .collect()
}

// ── El plan ─────────────────────────────────────────────────────────────────────────────────────────────────────

/// El valor de la derecha de un filtro, en el tipo de la columna.
fn literal(t: &Tipo, s: &str, columna: &str) -> Result<Valor, Fallo> {
    if t.opaco {
        return Err(Fallo::operador(format!(
            "la columna `{columna}` es `Opaque`: solo se puede preguntar si es nula"
        )));
    }
    if t.fisico == ore_base::tipos::Fisico::Texto {
        return Ok(Valor::Texto(s.to_string()));
    }
    t.fisico.analizar(s).ok_or_else(|| {
        Fallo::operador(format!(
            "el valor `{s}` no es un `{}` en su forma canonica (la de la columna `{columna}`)",
            t.oos
        ))
    })
}

/// Un filtro sobre la clave, como intervalos. `None`: no se empuja (se evalua sobre la clave decodificada).
fn de_la_clave(f: &Filtro, clave: &Tipo) -> Result<Option<Vec<Intervalo>>, Fallo> {
    match f.operador.as_str() {
        // la clave de fila nunca es nula
        "isNull" => return Ok(Some(vec![])),
        "isNotNull" => return Ok(Some(todo())),
        "eq" | "in" | "lt" | "le" | "gt" | "ge" => {}
        _ => return Ok(None),
    }
    let orden = orden_de(clave);
    if matches!(orden, Orden::NoEmpuja) {
        return Ok(None);
    }
    // Los valores de la derecha: uno, o una lista para `in`.
    let textos: Vec<&String> = match &f.valor {
        Derecha::Uno(s) => vec![s],
        Derecha::Lista(l) => l.iter().collect(),
        Derecha::Ninguno => {
            return Err(Fallo::operador(format!("`{}` lleva un valor", f.operador)));
        }
    };
    let op = if f.operador == "in" {
        "eq"
    } else {
        f.operador.as_str()
    };
    let mut trozos = Vec::new();
    for s in textos {
        match &orden {
            Orden::Lexico => {
                let v = if clave.opaco {
                    de_base64(s).ok_or_else(|| {
                        Fallo::operador(format!(
                            "el valor `{s}` de la clave (`Opaque`) no es base64"
                        ))
                    })?
                } else {
                    s.as_bytes().to_vec()
                };
                trozos.extend(match op {
                    "eq" => vec![(v.clone(), siguiente(&v))],
                    // «menor que el vacio» no lo cumple nadie; y un `fin` vacio querria decir «sin limite»
                    "lt" if v.is_empty() => vec![],
                    "lt" => vec![(Vec::new(), v)],
                    "le" => vec![(Vec::new(), siguiente(&v))],
                    "gt" => vec![(siguiente(&v), Vec::new())],
                    _ => vec![(v, Vec::new())],
                });
            }
            Orden::Entero(d) => {
                let l = match literal(clave, s, CLAVE)? {
                    Valor::Entero(n) => i128::from(n),
                    Valor::FechaHora(us) | Valor::Instante(us) => i128::from(us),
                    // un tipo que el dominio no sabe: se evalua despues
                    _ => return Ok(None),
                };
                if let Some((lo, hi)) = cotas(op, l, d) {
                    trozos.extend(intervalos_enteros(d, lo, hi));
                }
            }
            Orden::NoEmpuja => unreachable!(),
        }
    }
    Ok(Some(normalizar(trozos)))
}

/// El primer prefijo mayor que todos los que empiezan por `p`: el fin de un recorrido por prefijo. Vacio: sin limite.
fn sucesor_de_prefijo(p: &[u8]) -> Vec<u8> {
    let mut v = p.to_vec();
    while let Some(&u) = v.last() {
        if u == 0xFF {
            v.pop();
        } else {
            *v.last_mut().expect("hay ultimo") += 1;
            return v;
        }
    }
    Vec::new()
}

/// **Los filtros de las partes de una clave compuesta, como un recorrido por prefijo.** Un *superconjunto* de lo que
/// piden: los `eq` de las primeras partes forman el prefijo, y un rango entero sobre la parte siguiente lo recorta mas.
/// Nunca decide el resultado —todos los filtros de las partes se vuelven a evaluar sobre los valores decodificados—,
/// asi que lo que no se sabe empujar se deja y solo cuesta leer de mas. `None`: no recorta nada.
fn intervalos_de_partes(filtros: &[Filtro], partes: &[ParteClave]) -> Option<Vec<Intervalo>> {
    let de_parte = |i: usize| {
        filtros
            .iter()
            .filter(move |f| f.columna == partes[i].columna)
    };
    let mut prefijo: Vec<u8> = Vec::new();
    let mut k = 0;
    while k < partes.len() {
        let p = &partes[k];
        let Some(Derecha::Uno(s)) = de_parte(k).find(|f| f.operador == "eq").map(|f| &f.valor)
        else {
            break;
        };
        let Ok(v) = literal(&p.tipo, s, &p.columna) else {
            break;
        };
        let Ok(mut b) = codificar(p.tipo.cod, &p.tipo.fisico, &v) else {
            break;
        };
        match &p.largo {
            Largo::Fijo(n) if b.len() == *n => {}
            Largo::Hasta(sep) => b.extend_from_slice(sep),
            Largo::Resto => {}
            // un valor que no ocupa lo que la parte: no se empuja
            Largo::Fijo(_) => break,
        }
        prefijo.extend(b);
        k += 1;
    }
    let mut tramos = vec![(prefijo.clone(), sucesor_de_prefijo(&prefijo))];
    if k < partes.len()
        && let Orden::Entero(d) = orden_de(&partes[k].tipo)
        && partes[k].largo == Largo::Fijo(d.ancho)
    {
        let (mut lo, mut hi, mut hay) = (d.min, d.max, false);
        for f in de_parte(k).filter(|f| matches!(f.operador.as_str(), "lt" | "le" | "gt" | "ge")) {
            let Derecha::Uno(s) = &f.valor else { continue };
            let l = match literal(&partes[k].tipo, s, &partes[k].columna) {
                Ok(Valor::Entero(n)) => i128::from(n),
                Ok(Valor::FechaHora(us) | Valor::Instante(us)) => i128::from(us),
                _ => continue,
            };
            let Some((a, b)) = cotas(&f.operador, l, &d) else {
                return Some(Vec::new());
            };
            (lo, hi, hay) = (lo.max(a), hi.min(b), true);
        }
        if hay {
            if lo > hi {
                return Some(Vec::new());
            }
            tramos = intervalos_enteros(&d, lo, hi)
                .into_iter()
                .map(|(a, b)| ([prefijo.clone(), a].concat(), [prefijo.clone(), b].concat()))
                .collect();
        }
    }
    (tramos != todo()).then_some(tramos)
}

/// El plan de una peticion: los intervalos de clave y lo que queda por evaluar. `tipo_de` da el tipo de una
/// columna de celdas (lo declarado, o `String`). `partes`: las de una clave compuesta, si la tabla la declara.
pub fn plan(
    filtros: &[Filtro],
    clave: &Tipo,
    partes: &[ParteClave],
    tipo_de: &dyn Fn(&str) -> Tipo,
    objeto: &str,
) -> Result<Plan, Fallo> {
    let mut intervalos = todo();
    let mut residuales = Vec::new();
    for f in filtros {
        let t = if f.columna == CLAVE {
            clave.clone()
        } else {
            tipo_de(&f.columna)
        };
        // Una columna de bytes no tiene orden ni igualdad con sentido para quien la ve en base64.
        // (La clave `Opaque` si se empuja: son bytes con su orden. Lo que no tiene sentido es filtrar por el valor de una
        // *celda* `Opaque`, que el que pregunta ve en base64.)
        if t.opaco && f.columna != CLAVE && !matches!(f.operador.as_str(), "isNull" | "isNotNull") {
            return Err(Fallo::operador(format!(
                "`{objeto}`: el filtro `{}` sobre la columna `{}` (`Opaque`) no se puede evaluar: solo `isNull` e `isNotNull`",
                f.operador, f.columna
            )));
        }
        if f.columna == CLAVE
            && let Some(nuevo) = de_la_clave(f, clave)?
        {
            intervalos = interseccion(&intervalos, &nuevo);
            continue;
        }
        let valores = match &f.valor {
            Derecha::Uno(s) => vec![literal(&t, s, &f.columna)?],
            Derecha::Lista(l) => l
                .iter()
                .map(|s| literal(&t, s, &f.columna))
                .collect::<Result<_, _>>()?,
            Derecha::Ninguno => Vec::new(),
        };
        residuales.push(Residual {
            columna: f.columna.clone(),
            operador: f.operador.clone(),
            valores,
        });
    }
    if let Some(iv) = intervalos_de_partes(filtros, partes) {
        intervalos = interseccion(&intervalos, &iv);
    }
    Ok(Plan {
        intervalos,
        residuales,
    })
}

// ── La evaluacion ───────────────────────────────────────────────────────────────────────────────────────────────

/// Como compara SQL (y DuckDB, que recibe las filas) dos valores del mismo tipo: `NaN` es igual a `NaN` y mayor
/// que todo, y `-0.0` es igual a `0.0`. `None` si no son del mismo fisico.
pub fn comparar(a: &Valor, b: &Valor) -> Option<Ordering> {
    Some(match (a, b) {
        (Valor::Texto(x), Valor::Texto(y)) => x.as_bytes().cmp(y.as_bytes()),
        (Valor::Entero(x), Valor::Entero(y)) => x.cmp(y),
        (Valor::Real(x), Valor::Real(y)) => match (x.is_nan(), y.is_nan()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            _ => x.partial_cmp(y)?,
        },
        (Valor::Logico(x), Valor::Logico(y)) => x.cmp(y),
        (Valor::Decimal(x), Valor::Decimal(y)) => x.cmp(y),
        (Valor::Fecha(x), Valor::Fecha(y)) => x.cmp(y),
        (Valor::Hora(x), Valor::Hora(y)) => x.cmp(y),
        (Valor::FechaHora(x), Valor::FechaHora(y)) => x.cmp(y),
        (Valor::Instante(x), Valor::Instante(y)) => x.cmp(y),
        _ => return None,
    })
}

/// ¿Cumple el valor de una columna (`None`: sin celda) este residual?
pub fn cumple_uno(r: &Residual, v: Option<&Valor>) -> bool {
    match r.operador.as_str() {
        "isNull" => v.is_none(),
        "isNotNull" => v.is_some(),
        // un nulo no cumple ninguna comparacion
        _ => v.is_some_and(|v| {
            let con = |x: &Valor| comparar(v, x);
            let primero = r.valores.first();
            match r.operador.as_str() {
                "eq" => primero.is_some_and(|x| con(x) == Some(Ordering::Equal)),
                "neq" => primero.is_some_and(|x| con(x).is_some_and(|o| o != Ordering::Equal)),
                "in" => r.valores.iter().any(|x| con(x) == Some(Ordering::Equal)),
                "lt" => primero.is_some_and(|x| con(x) == Some(Ordering::Less)),
                "le" => primero.is_some_and(|x| con(x).is_some_and(|o| o != Ordering::Greater)),
                "gt" => primero.is_some_and(|x| con(x) == Some(Ordering::Greater)),
                "ge" => primero.is_some_and(|x| con(x).is_some_and(|o| o != Ordering::Less)),
                _ => false,
            }
        }),
    }
}

/// ¿Cumple la fila todos los residuales? `valor_de` da el valor decodificado de una columna (`None`: sin celda).
pub fn cumple(residuales: &[Residual], valor_de: &dyn Fn(&str) -> Option<Valor>) -> bool {
    residuales
        .iter()
        .all(|r| cumple_uno(r, valor_de(&r.columna).as_ref()))
}

/// Los bytes de un valor en la codificacion de una columna (para quien siembra un banco o escribe una prueba).
pub fn bytes_de_valor(t: &Tipo, v: &Valor) -> Result<Vec<u8>, String> {
    codificar(t.cod, &t.fisico, v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ore_base::tipos::Fisico;

    fn f(c: &str, op: &str, v: &str) -> Filtro {
        Filtro::uno(c, op, v)
    }
    fn lista(c: &str, op: &str, v: &[&str]) -> Filtro {
        Filtro {
            columna: c.into(),
            operador: op.into(),
            valor: Derecha::Lista(v.iter().map(|s| s.to_string()).collect()),
        }
    }
    fn sin(c: &str, op: &str) -> Filtro {
        Filtro {
            columna: c.into(),
            operador: op.into(),
            valor: Derecha::Ninguno,
        }
    }
    fn iv(i: &str, f: &str) -> Intervalo {
        (i.as_bytes().to_vec(), f.as_bytes().to_vec())
    }
    fn texto(_: &str) -> Tipo {
        Tipo::texto()
    }
    fn plan_de(fs: &[Filtro]) -> Plan {
        plan(fs, &Tipo::texto(), &[], &texto, "e.t").unwrap()
    }
    fn entero(cod: &str) -> Tipo {
        Tipo::nuevo(Some("Integer"), Some(cod), "e.t", "_clave").unwrap()
    }
    fn plan_entero(cod: &str, fs: &[Filtro]) -> Plan {
        plan(fs, &entero(cod), &[], &texto, "e.t").unwrap()
    }
    /// Los bytes de `Bytes.toBytes(int)`.
    fn i32b(n: i32) -> Vec<u8> {
        n.to_be_bytes().to_vec()
    }

    #[test]
    fn sin_filtros_se_lee_todo() {
        let p = plan_de(&[]);
        assert_eq!(p.intervalos, vec![iv("", "")]);
        assert!(p.residuales.is_empty());
    }

    #[test]
    fn los_operadores_sobre_una_clave_de_texto_son_intervalos_exactos() {
        assert_eq!(
            plan_de(&[f("_clave", "eq", "m")]).intervalos,
            vec![(b"m".to_vec(), b"m\0".to_vec())]
        );
        assert_eq!(
            plan_de(&[f("_clave", "lt", "m")]).intervalos,
            vec![iv("", "m")]
        );
        assert_eq!(
            plan_de(&[f("_clave", "le", "m")]).intervalos,
            vec![(vec![], b"m\0".to_vec())]
        );
        assert_eq!(
            plan_de(&[f("_clave", "gt", "m")]).intervalos,
            vec![(b"m\0".to_vec(), vec![])]
        );
        assert_eq!(
            plan_de(&[f("_clave", "ge", "m")]).intervalos,
            vec![iv("m", "")]
        );
    }

    #[test]
    fn dos_cotas_se_intersecan_y_una_imposible_no_deja_ningun_intervalo() {
        assert_eq!(
            plan_de(&[f("_clave", "ge", "c"), f("_clave", "lt", "m")]).intervalos,
            vec![iv("c", "m")]
        );
        assert!(
            plan_de(&[f("_clave", "ge", "m"), f("_clave", "lt", "c")])
                .intervalos
                .is_empty()
        );
        assert!(
            plan_de(&[f("_clave", "gt", "m"), f("_clave", "lt", "m")])
                .intervalos
                .is_empty()
        );
        assert!(
            plan_de(&[f("_clave", "lt", "")]).intervalos.is_empty(),
            "nada es menor que el vacio"
        );
    }

    #[test]
    fn un_in_son_puntos_ordenados_sin_repetir() {
        let p = plan_de(&[lista("_clave", "in", &["m", "c", "m"])]);
        assert_eq!(
            p.intervalos,
            vec![
                (b"c".to_vec(), b"c\0".to_vec()),
                (b"m".to_vec(), b"m\0".to_vec())
            ]
        );
        assert!(
            plan_de(&[lista("_clave", "in", &[])]).intervalos.is_empty(),
            "un in vacio no deja pasar nada"
        );
        let p = plan_de(&[
            lista("_clave", "in", &["a", "m", "z"]),
            f("_clave", "gt", "b"),
            f("_clave", "lt", "y"),
        ]);
        assert_eq!(p.intervalos, vec![(b"m".to_vec(), b"m\0".to_vec())]);
    }

    #[test]
    fn la_clave_nunca_es_nula() {
        assert!(plan_de(&[sin("_clave", "isNull")]).intervalos.is_empty());
        assert_eq!(
            plan_de(&[sin("_clave", "isNotNull")]).intervalos,
            vec![iv("", "")]
        );
    }

    #[test]
    fn neq_sobre_la_clave_y_todo_sobre_las_celdas_se_evalua_en_el_conector() {
        let p = plan_de(&[
            f("_clave", "neq", "m"),
            f("info:n", "eq", "x"),
            sin("info:n", "isNotNull"),
        ]);
        assert_eq!(p.intervalos, vec![iv("", "")], "ninguno recorta la clave");
        assert_eq!(p.residuales.len(), 3);
        assert_eq!(p.residuales[1].valores, vec![Valor::Texto("x".into())]);
    }

    // ── claves enteras: el orden de bytes y el de los numeros no coinciden ─────────────────────────────────────

    #[test]
    fn un_rango_sobre_una_clave_entera_con_signo_se_parte_en_los_positivos_y_los_negativos() {
        // id < 5: 0..=4 (bytes 00000000..00000004) y TODOS los negativos (80000000..FFFFFFFF), en ese orden
        let p = plan_entero("i32be", &[f("_clave", "lt", "5")]);
        assert_eq!(
            p.intervalos,
            vec![
                (i32b(0), [i32b(4), vec![0]].concat()),
                (i32b(i32::MIN), [i32b(-1), vec![0]].concat()),
            ]
        );
        // id > -3: -2..=-1 (negativos) y todos los positivos; los positivos van primero en bytes
        let p = plan_entero("i32be", &[f("_clave", "gt", "-3")]);
        assert_eq!(
            p.intervalos,
            vec![
                (i32b(0), [i32b(i32::MAX), vec![0]].concat()),
                (i32b(-2), [i32b(-1), vec![0]].concat())
            ]
        );
    }

    #[test]
    fn un_rango_que_no_cruza_el_cero_es_un_solo_tramo() {
        assert_eq!(
            plan_entero("i32be", &[f("_clave", "ge", "10"), f("_clave", "le", "20")]).intervalos,
            vec![(i32b(10), [i32b(20), vec![0]].concat())]
        );
        assert_eq!(
            plan_entero(
                "i32be",
                &[f("_clave", "ge", "-20"), f("_clave", "lt", "-10")]
            )
            .intervalos,
            vec![(i32b(-20), [i32b(-11), vec![0]].concat())]
        );
    }

    #[test]
    fn eq_e_in_sobre_enteros_son_puntos_y_ordenan_los_positivos_antes_que_los_negativos() {
        assert_eq!(
            plan_entero("i64be", &[f("_clave", "eq", "-1")]).intervalos,
            vec![([0xFF; 8].to_vec(), [[0xFF; 8].to_vec(), vec![0]].concat())]
        );
        let p = plan_entero("i32be", &[lista("_clave", "in", &["-7", "7", "7"])]);
        assert_eq!(
            p.intervalos,
            vec![
                (i32b(7), [i32b(7), vec![0]].concat()),
                (i32b(-7), [i32b(-7), vec![0]].concat())
            ]
        );
    }

    #[test]
    fn un_valor_fuera_del_dominio_se_recorta_y_uno_imposible_no_deja_nada() {
        // i8: 200 no cabe; id < 200 es todo el dominio
        let p = plan_entero("i8be", &[f("_clave", "lt", "200")]);
        assert_eq!(
            p.intervalos,
            vec![(vec![0x00], vec![0x7F, 0]), (vec![0x80], vec![0xFF, 0])]
        );
        assert!(
            plan_entero("i8be", &[f("_clave", "gt", "200")])
                .intervalos
                .is_empty()
        );
        assert!(
            plan_entero("i8be", &[f("_clave", "eq", "200")])
                .intervalos
                .is_empty()
        );
        // sin signo: nada es menor que 0
        assert!(
            plan_entero("u32be", &[f("_clave", "lt", "0")])
                .intervalos
                .is_empty()
        );
        assert_eq!(
            plan_entero("u32be", &[f("_clave", "ge", "0")]).intervalos,
            vec![(vec![0; 4], [vec![0xFF; 4], vec![0]].concat())]
        );
    }

    #[test]
    fn el_orden_de_los_intervalos_es_el_de_los_bytes() {
        // lo que importa: ordenados y sin solapes, para que el reparto por regiones los recorte bien
        let p = plan_entero("i32be", &[f("_clave", "lt", "5")]);
        assert!(
            p.intervalos.windows(2).all(|w| w[0].1 <= w[1].0),
            "{:?}",
            p.intervalos
        );
    }

    #[test]
    fn epoch_ms_sobre_un_instante_redondea_los_microsegundos_en_el_sentido_que_toca() {
        let clave = Tipo::nuevo(Some("DateTimeTz"), Some("epoch-ms"), "e.t", "_clave").unwrap();
        let p = |op: &str, v: &str| {
            plan(&[f("_clave", op, v)], &clave, &[], &texto, "e.t")
                .unwrap()
                .intervalos
        };
        let ms = |n: i64| n.to_be_bytes().to_vec();
        // 1970-01-01 00:00:00.001500+00 son 1 500 us: entre el ms 1 y el 2
        let l = "1970-01-01 00:00:00.001500+00";
        assert!(
            p("eq", l).is_empty(),
            "ningun milisegundo entero es 1 500 us"
        );
        assert_eq!(
            p("ge", l),
            vec![(ms(2), [ms(i64::MAX), vec![0]].concat())],
            "el primero que no es menor es el ms 2"
        );
        assert_eq!(p("gt", l), vec![(ms(2), [ms(i64::MAX), vec![0]].concat())]);
        assert_eq!(
            p("le", l)[0],
            (ms(0), [ms(1), vec![0]].concat()),
            "el ultimo que no pasa es el ms 1"
        );
        // un milisegundo exacto: eq es un punto
        assert_eq!(
            p("eq", "1970-01-01 00:00:00.002000+00"),
            vec![(ms(2), [ms(2), vec![0]].concat())]
        );
    }

    #[test]
    fn las_claves_que_no_conservan_el_orden_no_se_empujan_y_se_evaluan_decodificadas() {
        for (oos, cod) in [
            ("Float", "f64be"),
            ("Decimal<12, 2>", "decimal"),
            ("Boolean", "bool"),
            ("Integer", "texto"),
            ("Date", "epoch-ms"),
        ] {
            let clave = Tipo::nuevo(Some(oos), Some(cod), "e.t", "_clave").unwrap();
            let literal = match oos {
                "Float" => "1.5",
                "Decimal<12, 2>" => "1.50",
                "Boolean" => "true",
                "Date" => "2026-01-01",
                _ => "7",
            };
            let p = plan(&[f("_clave", "ge", literal)], &clave, &[], &texto, "e.t").unwrap();
            assert_eq!(p.intervalos, vec![iv("", "")], "{cod}");
            assert_eq!(p.residuales.len(), 1, "{cod}");
        }
    }

    #[test]
    fn una_clave_opaque_decodifica_base64_y_una_celda_opaque_solo_admite_nulidad() {
        let opaco = |_: &str| Tipo::opaco();
        let e = plan(
            &[f("f:raw", "eq", "AAAAKg==")],
            &Tipo::texto(),
            &[],
            &opaco,
            "e.t",
        )
        .unwrap_err();
        assert!(
            e.mensaje.contains("f:raw") && e.mensaje.contains("Opaque"),
            "{}",
            e.mensaje
        );
        assert!(
            plan(
                &[sin("f:raw", "isNull")],
                &Tipo::texto(),
                &[],
                &opaco,
                "e.t"
            )
            .is_ok()
        );
        let p = plan(
            &[f("_clave", "eq", "AAAAKg==")],
            &Tipo::opaco(),
            &[],
            &texto,
            "e.t",
        )
        .unwrap();
        assert_eq!(
            p.intervalos,
            vec![(vec![0, 0, 0, 42], vec![0, 0, 0, 42, 0])]
        );
        assert!(
            plan(
                &[f("_clave", "eq", "no es base64!")],
                &Tipo::opaco(),
                &[],
                &texto,
                "e.t"
            )
            .is_err()
        );
    }

    #[test]
    fn un_valor_de_la_derecha_que_no_es_del_tipo_se_rechaza_como_operador() {
        let entero_celda = |_: &str| entero("i32be");
        let e = plan(
            &[f("f:edad", "gt", "muchos")],
            &Tipo::texto(),
            &[],
            &entero_celda,
            "e.t",
        )
        .unwrap_err();
        assert!(
            e.mensaje.contains("muchos") && e.mensaje.contains("Integer"),
            "{}",
            e.mensaje
        );
        assert!(
            plan(
                &[f("_clave", "gt", "muchos")],
                &entero("i32be"),
                &[],
                &texto,
                "e.t"
            )
            .is_err()
        );
    }

    // ── la evaluacion ─────────────────────────────────────────────────────────────────────────────────────────────

    fn residual(op: &str, v: &[Valor]) -> Vec<Residual> {
        vec![Residual {
            columna: "f:q".into(),
            operador: op.into(),
            valores: v.to_vec(),
        }]
    }
    fn con(v: Option<Valor>) -> impl Fn(&str) -> Option<Valor> {
        move |_| v.clone()
    }
    fn t(s: &str) -> Valor {
        Valor::Texto(s.into())
    }

    #[test]
    fn un_nulo_no_cumple_ninguna_comparacion_y_si_isnull() {
        for op in ["eq", "neq", "lt", "le", "gt", "ge"] {
            assert!(!cumple(&residual(op, &[t("m")]), &con(None)), "{op}");
        }
        assert!(!cumple(&residual("in", &[t("m")]), &con(None)));
        assert!(cumple(&residual("isNull", &[]), &con(None)));
        assert!(!cumple(&residual("isNotNull", &[]), &con(None)));
        assert!(
            cumple(&residual("isNotNull", &[]), &con(Some(t("")))),
            "la cadena vacia no es nulo"
        );
    }

    #[test]
    fn las_comparaciones_de_texto_son_de_bytes_y_no_de_numeros() {
        let c = |op, v: &str, x: &str| cumple(&residual(op, &[t(v)]), &con(Some(t(x))));
        assert!(c("eq", "m", "m") && !c("eq", "m", "n"));
        assert!(c("neq", "m", "n") && !c("neq", "m", "m"));
        assert!(c("lt", "m", "a") && !c("lt", "m", "m"));
        assert!(c("le", "m", "m") && !c("le", "m", "n"));
        assert!(c("gt", "m", "n") && !c("gt", "m", "m"));
        assert!(c("ge", "m", "m") && !c("ge", "m", "a"));
        assert!(
            c("lt", "10", "09") && !c("gt", "9", "10"),
            "texto, no numero: \"10\" < \"9\""
        );
        assert!(cumple(
            &residual("in", &[t("a"), t("b")]),
            &con(Some(t("b")))
        ));
        assert!(!cumple(&residual("in", &[]), &con(Some(t("b")))));
    }

    #[test]
    fn las_comparaciones_tipadas_son_las_del_tipo() {
        let n = Valor::Entero;
        let c = |op, v: Valor, x: Valor| cumple(&residual(op, &[v]), &con(Some(x)));
        assert!(c("gt", n(9), n(10)), "numero, no texto: 10 > 9");
        assert!(c("lt", n(-1), n(-2)));
        assert!(c("ge", Valor::Decimal(150), Valor::Decimal(150)));
        assert!(c("lt", Valor::Fecha(100), Valor::Fecha(99)));
        assert!(c("eq", Valor::Logico(true), Valor::Logico(true)));
        assert!(c("gt", Valor::Instante(5), Valor::Instante(6)));
        // un valor de otro fisico no se compara con nada
        assert!(!c("eq", n(1), Valor::Texto("1".into())));
    }

    #[test]
    fn nan_es_igual_a_si_mismo_y_mayor_que_todo_y_el_cero_negativo_es_cero() {
        let r = Valor::Real;
        let c = |op, v: Valor, x: Valor| cumple(&residual(op, &[v]), &con(Some(x)));
        assert!(c("eq", r(f64::NAN), r(f64::NAN)));
        assert!(c("gt", r(1e308), r(f64::NAN)), "NaN es mayor que todo");
        assert!(c("lt", r(f64::NAN), r(0.0)), "0 < NaN");
        assert!(!c("gt", r(f64::NAN), r(0.0)), "nada es mayor que NaN");
        assert!(c("eq", r(0.0), r(-0.0)));
        assert!(
            c("lt", r(1.0), r(f64::NEG_INFINITY)),
            "-inf es menor que todo"
        );
    }

    #[test]
    fn el_reparto_parte_por_regiones_y_agrupa_sin_perder_trozos() {
        let regiones = vec![iv("", "d"), iv("d", "h"), iv("h", "p"), iv("p", "")];
        let trozos = por_regiones(&[iv("c", "m")], &regiones);
        assert_eq!(trozos, vec![iv("c", "d"), iv("d", "h"), iv("h", "m")]);
        assert_eq!(por_regiones(&[iv("", "")], &regiones), regiones);
        assert!(por_regiones(&[], &regiones).is_empty());
        let grupos = repartir(&regiones, 3);
        assert_eq!(
            grupos.iter().map(Vec::len).collect::<Vec<_>>(),
            vec![1, 1, 2]
        );
        assert_eq!(
            grupos.concat(),
            regiones,
            "ni se pierde ni se repite ninguno"
        );
        assert_eq!(repartir(&regiones, 16).len(), 4, "no mas grupos que trozos");
        assert!(repartir::<Intervalo>(&[], 4).is_empty());
    }

    #[test]
    fn normalizar_une_los_que_se_tocan_y_respeta_el_infinito() {
        assert_eq!(
            normalizar(vec![iv("c", "f"), iv("a", "c"), iv("e", "g")]),
            vec![iv("a", "g")]
        );
        assert_eq!(
            normalizar(vec![iv("a", "b"), iv("d", "e")]),
            vec![iv("a", "b"), iv("d", "e")]
        );
        assert_eq!(
            normalizar(vec![iv("a", ""), iv("c", "d")]),
            vec![iv("a", "")]
        );
        assert_eq!(
            normalizar(vec![iv("a", "c"), iv("b", "")]),
            vec![iv("a", "")]
        );
    }

    #[test]
    fn el_fisico_de_un_real_y_un_decimal_se_parsea_de_su_forma_canonica() {
        let _ = Fisico::Real;
        let real = Tipo::nuevo(Some("Float"), Some("f64be"), "e.t", "f:x").unwrap();
        assert_eq!(literal(&real, "1.5", "f:x").unwrap(), Valor::Real(1.5));
        let dec = Tipo::nuevo(Some("Decimal<12, 2>"), Some("decimal"), "e.t", "f:x").unwrap();
        assert_eq!(literal(&dec, "12.30", "f:x").unwrap(), Valor::Decimal(1230));
        assert!(literal(&dec, "12.345", "f:x").is_err(), "no se redondea");
    }
}
