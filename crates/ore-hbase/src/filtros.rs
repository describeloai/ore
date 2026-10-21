//! **Que se empuja a HBase y que se evalua aqui**, y como se reparte el trabajo. Todo puro (sin servidor).
//!
//! HBase solo empuja con eficacia dos cosas: el **rango de la clave de fila** y el de tiempo (ficha 0072, §7.4).
//! Un filtro sobre `_clave` (`eq`, `in`, `lt`, `le`, `gt`, `ge`, `isNull`, `isNotNull`) se convierte en
//! **intervalos de clave** `[ini, fin)` que se piden al servidor y que, por ser exactos, no se vuelven a evaluar.
//! Cualquier otro (`neq` sobre la clave, y todo sobre las celdas) es un **residual** que se evalua en el conector
//! sobre cada fila leida, con la semantica de SQL: **un nulo no cumple nada** salvo `isNull`.
//!
//! El orden de bytes de HBase es el de las cadenas UTF-8 de DuckDB, asi que los rangos sobre una clave `String`
//! son exactos. Sobre una clave `Opaque` los valores llegan en base64 y se comparan como bytes.

use crate::esquema::{CLAVE, Tipo};
use crate::lote::de_base64;
use ore_driver::{Fallo, Filtro, Valor};
use std::cmp::Ordering;

/// `[ini, fin)`; un `fin` vacio es «sin limite».
pub type Intervalo = (Vec<u8>, Vec<u8>);

/// Un filtro que se evalua sobre cada fila.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Residual {
    /// `_clave` o `familia:cualificador`.
    pub columna: String,
    pub operador: String,
    pub valores: Vec<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

fn valor_de(t: Tipo, s: &str, columna: &str) -> Result<Vec<u8>, Fallo> {
    match t {
        Tipo::Texto => Ok(s.as_bytes().to_vec()),
        Tipo::Opaco => de_base64(s).ok_or_else(|| {
            Fallo::operador(format!(
                "el valor `{s}` de la columna `{columna}` (`Opaque`) no es base64"
            ))
        }),
    }
}

/// Un filtro sobre la clave, como intervalos.
fn de_la_clave(f: &Filtro, t: Tipo) -> Result<Option<Vec<Intervalo>>, Fallo> {
    let uno = |f: &Filtro| match &f.valor {
        Valor::Uno(s) => valor_de(t, s, CLAVE),
        _ => Err(Fallo::operador(format!("`{}` lleva un valor", f.operador))),
    };
    Ok(Some(match f.operador.as_str() {
        "eq" => {
            let v = uno(f)?;
            vec![(v.clone(), siguiente(&v))]
        }
        "in" => {
            let Valor::Lista(l) = &f.valor else {
                return Err(Fallo::operador("`in` lleva una lista"));
            };
            let puntos = l
                .iter()
                .map(|s| valor_de(t, s, CLAVE).map(|v| (v.clone(), siguiente(&v))))
                .collect::<Result<Vec<_>, _>>()?;
            normalizar(puntos)
        }
        // «menor que el vacio» no lo cumple nadie; y un `fin` vacio querria decir «sin limite»
        "lt" => {
            let v = uno(f)?;
            if v.is_empty() {
                vec![]
            } else {
                vec![(Vec::new(), v)]
            }
        }
        "le" => vec![(Vec::new(), siguiente(&uno(f)?))],
        "gt" => vec![(siguiente(&uno(f)?), Vec::new())],
        "ge" => vec![(uno(f)?, Vec::new())],
        // la clave de fila nunca es nula
        "isNull" => vec![],
        "isNotNull" => todo(),
        // `neq` y lo que no se sabe: lo evalua el conector sobre cada fila
        _ => return Ok(None),
    }))
}

/// El plan de una peticion: los intervalos de clave y lo que queda por evaluar. `tipo_de` da el tipo de una
/// columna de celdas (lo declarado, o `String`).
pub fn plan(
    filtros: &[Filtro],
    clave: Tipo,
    tipo_de: &dyn Fn(&str) -> Tipo,
    objeto: &str,
) -> Result<Plan, Fallo> {
    let mut intervalos = todo();
    let mut residuales = Vec::new();
    for f in filtros {
        if f.columna == CLAVE
            && let Some(nuevo) = de_la_clave(f, clave)?
        {
            intervalos = interseccion(&intervalos, &nuevo);
            continue;
        }
        let t = if f.columna == CLAVE {
            clave
        } else {
            tipo_de(&f.columna)
        };
        // Una columna de bytes no tiene orden ni igualdad con sentido para quien la ve en base64.
        if t == Tipo::Opaco && !matches!(f.operador.as_str(), "isNull" | "isNotNull") {
            return Err(Fallo::operador(format!(
                "`{objeto}`: el filtro `{}` sobre la columna `{}` (`Opaque`) no se puede evaluar: solo `isNull` e `isNotNull`",
                f.operador, f.columna
            )));
        }
        let valores = match &f.valor {
            Valor::Uno(s) => vec![valor_de(t, s, &f.columna)?],
            Valor::Lista(l) => l
                .iter()
                .map(|s| valor_de(t, s, &f.columna))
                .collect::<Result<_, _>>()?,
            Valor::Ninguno => Vec::new(),
        };
        residuales.push(Residual {
            columna: f.columna.clone(),
            operador: f.operador.clone(),
            valores,
        });
    }
    Ok(Plan {
        intervalos,
        residuales,
    })
}

/// ¿Cumple la fila todos los residuales? `valor_de` da el valor de una columna (`None`: sin celda).
pub fn cumple(residuales: &[Residual], valor_de: &dyn Fn(&str) -> Option<Vec<u8>>) -> bool {
    residuales.iter().all(|r| {
        let v = valor_de(&r.columna);
        match r.operador.as_str() {
            "isNull" => v.is_none(),
            "isNotNull" => v.is_some(),
            // un nulo no cumple ninguna comparacion
            _ => v.is_some_and(|v| match r.operador.as_str() {
                "eq" => r.valores.first().is_some_and(|x| *x == v),
                "neq" => r.valores.first().is_some_and(|x| *x != v),
                "in" => r.valores.contains(&v),
                "lt" => r
                    .valores
                    .first()
                    .is_some_and(|x| v.cmp(x) == Ordering::Less),
                "le" => r
                    .valores
                    .first()
                    .is_some_and(|x| v.cmp(x) != Ordering::Greater),
                "gt" => r
                    .valores
                    .first()
                    .is_some_and(|x| v.cmp(x) == Ordering::Greater),
                "ge" => r
                    .valores
                    .first()
                    .is_some_and(|x| v.cmp(x) != Ordering::Less),
                _ => false,
            }),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(c: &str, op: &str, v: &str) -> Filtro {
        Filtro::uno(c, op, v)
    }
    fn lista(c: &str, op: &str, v: &[&str]) -> Filtro {
        Filtro {
            columna: c.into(),
            operador: op.into(),
            valor: Valor::Lista(v.iter().map(|s| s.to_string()).collect()),
        }
    }
    fn sin(c: &str, op: &str) -> Filtro {
        Filtro {
            columna: c.into(),
            operador: op.into(),
            valor: Valor::Ninguno,
        }
    }
    fn iv(i: &str, f: &str) -> Intervalo {
        (i.as_bytes().to_vec(), f.as_bytes().to_vec())
    }
    fn texto(_: &str) -> Tipo {
        Tipo::Texto
    }
    fn plan_de(fs: &[Filtro]) -> Plan {
        plan(fs, Tipo::Texto, &texto, "e.t").unwrap()
    }

    #[test]
    fn sin_filtros_se_lee_todo() {
        let p = plan_de(&[]);
        assert_eq!(p.intervalos, vec![iv("", "")]);
        assert!(p.residuales.is_empty());
    }

    #[test]
    fn los_operadores_sobre_la_clave_son_intervalos_exactos() {
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
        // y se recorta con una cota
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
        assert_eq!(p.residuales[1].valores, vec![b"x".to_vec()]);
    }

    #[test]
    fn una_columna_opaque_solo_admite_nulidad_y_una_clave_opaque_decodifica_base64() {
        let opaco = |_: &str| Tipo::Opaco;
        let e = plan(&[f("f:raw", "eq", "AAAAKg==")], Tipo::Texto, &opaco, "e.t").unwrap_err();
        assert!(
            e.mensaje.contains("f:raw") && e.mensaje.contains("Opaque"),
            "{}",
            e.mensaje
        );
        assert!(plan(&[sin("f:raw", "isNull")], Tipo::Texto, &opaco, "e.t").is_ok());
        // la clave Opaque: el valor llega en base64 y el intervalo es de bytes
        let p = plan(&[f("_clave", "eq", "AAAAKg==")], Tipo::Opaco, &texto, "e.t").unwrap();
        assert_eq!(
            p.intervalos,
            vec![(vec![0, 0, 0, 42], vec![0, 0, 0, 42, 0])]
        );
        assert!(
            plan(
                &[f("_clave", "eq", "no es base64!")],
                Tipo::Opaco,
                &texto,
                "e.t"
            )
            .is_err()
        );
    }

    fn residual(op: &str, v: &[&str]) -> Vec<Residual> {
        vec![Residual {
            columna: "f:q".into(),
            operador: op.into(),
            valores: v.iter().map(|s| s.as_bytes().to_vec()).collect(),
        }]
    }
    fn con(v: Option<&str>) -> impl Fn(&str) -> Option<Vec<u8>> {
        let v = v.map(|s| s.as_bytes().to_vec());
        move |_| v.clone()
    }

    #[test]
    fn un_nulo_no_cumple_ninguna_comparacion_y_si_isnull() {
        for op in ["eq", "neq", "lt", "le", "gt", "ge"] {
            assert!(!cumple(&residual(op, &["m"]), &con(None)), "{op}");
        }
        assert!(!cumple(&residual("in", &["m"]), &con(None)));
        assert!(cumple(&residual("isNull", &[]), &con(None)));
        assert!(!cumple(&residual("isNotNull", &[]), &con(None)));
        assert!(
            cumple(&residual("isNotNull", &[]), &con(Some(""))),
            "la cadena vacia no es nulo"
        );
    }

    #[test]
    fn las_comparaciones_son_de_bytes() {
        let c = |op, v: &str, x: &str| cumple(&residual(op, &[v]), &con(Some(x)));
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
        assert!(cumple(&residual("in", &["a", "b"]), &con(Some("b"))));
        assert!(!cumple(&residual("in", &[]), &con(Some("b"))));
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
}
