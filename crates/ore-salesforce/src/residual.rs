//! **Los residuales**: lo que el servidor aplicó de más, reevaluado exacto
//! (ADR 0083 §1.5, X0 B2).
//!
//! SOQL no distingue mayúsculas en `=`, `!=`, `LIKE` ni en las comparaciones de
//! texto; el motor que recibe las filas (DuckDB) **sí**. El traductor empuja el
//! superconjunto cuando puede (`=`, `LIKE`, `IN`) y no empuja nada cuando no
//! puede (`!=`, `<`…), y deja el filtro como residual. Aquí se comprueba, fila a
//! fila, con la semántica de SQL:
//!
//! - comparación **binaria y con mayúsculas** (`'a' <> 'A'`);
//! - **un nulo no cumple nada** (`NULL <> 'x'` es desconocido, y SQL lo descarta):
//!   la diferencia con `!=` de SOQL, que lo incluye (B3).
//!
//! Solo hay residuales de **texto**: el traductor no los crea de otro tipo.

use ore_driver::{Filtro, Valor};

/// ¿Cumple este valor el filtro? `None` es el nulo.
pub fn cumple(f: &Filtro, v: Option<&str>) -> bool {
    let Some(v) = v else { return false };
    match (f.operador.as_str(), &f.valor) {
        ("eq", Valor::Uno(x)) => v == x,
        ("neq", Valor::Uno(x)) => v != x,
        ("lt", Valor::Uno(x)) => v < x.as_str(),
        ("le", Valor::Uno(x)) => v <= x.as_str(),
        ("gt", Valor::Uno(x)) => v > x.as_str(),
        ("ge", Valor::Uno(x)) => v >= x.as_str(),
        ("in", Valor::Lista(xs)) => xs.iter().any(|x| x == v),
        ("like", Valor::Uno(p)) => como(v, p),
        // `isNull` e `isNotNull` los pone el servidor, exactos: no llegan aquí.
        _ => false,
    }
}

/// `LIKE` de SQL: `%` es cualquier tramo, `_` un carácter. **Con mayúsculas.**
/// El escape `\` no está definido en el contrato: el traductor lo rechaza.
pub fn como(texto: &str, patron: &str) -> bool {
    let t: Vec<char> = texto.chars().collect();
    let p: Vec<char> = patron.chars().collect();
    let (mut i, mut j) = (0, 0);
    // El último `%` visto y dónde estaba el texto: para volver atrás sin recursión.
    let (mut estrella, mut desde) = (None, 0);
    while i < t.len() {
        match p.get(j) {
            Some('%') => {
                estrella = Some(j);
                desde = i;
                j += 1;
            }
            Some(&c) if c == '_' || c == t[i] => {
                i += 1;
                j += 1;
            }
            _ => match estrella {
                Some(e) => {
                    desde += 1;
                    i = desde;
                    j = e + 1;
                }
                None => return false,
            },
        }
    }
    p[j..].iter().all(|&c| c == '%')
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn f(op: &str, v: &str) -> Filtro {
        Filtro::uno("c", op, v)
    }

    /// X0 B2: SOQL devolvió `X0-Cuenta-001` para `x0-cuenta-001`; SQL, no.
    #[test]
    fn eq_distingue_mayusculas() {
        assert!(cumple(&f("eq", "X0-Cuenta-001"), Some("X0-Cuenta-001")));
        assert!(!cumple(&f("eq", "x0-cuenta-001"), Some("X0-Cuenta-001")));
    }

    /// El `!= 'a'` de SOQL quitaría `'A'`; el de SQL lo devuelve. Y un nulo no pasa.
    #[test]
    fn neq_devuelve_la_variante_y_descarta_el_nulo() {
        assert!(cumple(&f("neq", "Banking"), Some("BANKING")));
        assert!(!cumple(&f("neq", "Banking"), Some("Banking")));
        assert!(
            !cumple(&f("neq", "Banking"), None),
            "NULL <> x es desconocido"
        );
    }

    #[test]
    fn un_nulo_no_cumple_ningun_filtro() {
        for op in ["eq", "neq", "lt", "le", "gt", "ge", "like"] {
            assert!(!cumple(&f(op, "x"), None), "{op}");
        }
        let en = Filtro {
            columna: "c".into(),
            operador: "in".into(),
            valor: Valor::Lista(vec!["a".into()]),
        };
        assert!(!cumple(&en, None));
    }

    #[test]
    fn el_orden_es_binario() {
        assert!(cumple(&f("gt", "M"), Some("N")));
        // Binario: `m` (0x6D) es mayor que `M` (0x4D); con una intercalación
        // sin mayúsculas serían iguales y esto no se cumpliría.
        assert!(cumple(&f("gt", "M"), Some("m")));
        assert!(!cumple(&f("gt", "m"), Some("M")));
    }

    #[test]
    fn in_es_exacto() {
        let en = Filtro {
            columna: "c".into(),
            operador: "in".into(),
            valor: Valor::Lista(vec!["NA".into(), "None".into()]),
        };
        assert!(cumple(&en, Some("NA")));
        assert!(!cumple(&en, Some("na")));
    }

    #[test]
    fn like_con_porcentaje_y_guion_bajo_y_con_mayusculas() {
        assert!(como("X0-Cuenta-001", "X0-%"));
        assert!(como("X0-Cuenta-001", "%001"));
        assert!(como("X0-Cuenta-001", "%Cuenta%"));
        assert!(como("X0-Cuenta-001", "X0-Cuenta-00_"));
        assert!(
            !como("X0-Cuenta-001", "x0-%"),
            "LIKE de SQL distingue mayúsculas"
        );
        assert!(!como("X0-Cuenta-001", "X0-Cuenta-0_"));
        assert!(como("", "%"));
        assert!(!como("", "_"));
        assert!(como("abc", "a%b%c"));
        assert!(!como("abc", "a%b%d"));
        assert!(como("ñandú", "_andú"), "un carácter, no un byte");
    }
}
