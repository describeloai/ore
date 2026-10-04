//! **Los datos de partida**: las mismas tablas en cada origen, y lo que cada
//! petición tiene que devolver, calculado aquí sin preguntar a nadie.
//!
//! Tres tablas:
//!
//! | tabla | qué | para |
//! |---|---|---|
//! | `tipos` | una columna por tipo escalar de OOS, con nulos y bordes | casos 1–5 |
//! | `grande` | 10⁶ filas generadas | casos 7, 8, 10 |
//! | `vacia` | las columnas de `tipos`, sin filas | caso 6 |
//!
//! Los valores se escriben en el **texto canónico** de `ore_core::tipos` y se
//! comparan como [`Valor`]: la misma tabla de 0032 que estrecha el almacén. Un
//! conector que devuelve otro tipo, u otro valor, lo hace contra la definición
//! que usa el resto del producto, no contra una segunda que este kit tuviera.

use ore_core::tipos::{Fisico, Valor};
use std::cmp::Ordering;

/// Una tabla de la semilla.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tabla {
    Tipos,
    Grande,
    Vacia,
}

impl Tabla {
    pub fn nombre(self) -> &'static str {
        match self {
            Tabla::Tipos => "tipos",
            Tabla::Grande => "grande",
            Tabla::Vacia => "vacia",
        }
    }

    pub fn columnas(self) -> &'static [Columna] {
        match self {
            Tabla::Tipos | Tabla::Vacia => TIPOS,
            Tabla::Grande => GRANDE_COLUMNAS,
        }
    }
}

/// Una columna: su nombre y su tipo de OOS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Columna {
    pub nombre: &'static str,
    pub tipo: &'static str,
}

impl Columna {
    /// Su físico, por la tabla de 0032.
    pub fn fisico(&self) -> Fisico {
        let t = ore_core::types::parse_type(self.tipo).expect("un tipo de la semilla es de OOS");
        Fisico::de(&t)
    }
}

const fn c(nombre: &'static str, tipo: &'static str) -> Columna {
    Columna { nombre, tipo }
}

/// Las columnas de `tipos` (y de `vacia`). `id` no es nulo nunca: es lo que
/// identifica las filas que vuelven.
pub const TIPOS: &[Columna] = &[
    c("id", "Integer"),
    c("entero", "Integer"),
    c("importe", "Decimal<12, 2>"),
    c("real", "Float"),
    c("texto", "String"),
    c("fecha", "Date"),
    c("local", "DateTime"),
    c("instante", "DateTimeTz"),
    c("logico", "Boolean"),
];

/// Las filas de `tipos`, en el orden de [`TIPOS`] y en texto canónico.
///
/// Lo que prueban: los bordes de un `int64`, la escala exacta de un decimal,
/// la cadena vacía frente al nulo, `_` y `%` dentro de un texto (para `like`),
/// mayúsculas, acentos, una comilla, un salto de línea, microsegundos, el 29
/// de febrero, una fila entera nula. Los `entero` no nulos son distintos entre
/// sí, para que un orden por esa columna tenga una sola respuesta.
pub const FILAS: &[[Option<&str>; 9]] = &[
    [
        Some("1"),
        Some("0"),
        Some("0.00"),
        Some("0.5"),
        Some("ana"),
        Some("2026-10-04"),
        Some("2026-10-04T10:00:00"),
        Some("2026-10-04T10:00:00Z"),
        Some("true"),
    ],
    [
        Some("2"),
        Some("-7"),
        Some("-2.50"),
        Some("-10000000000"),
        Some("Ana"),
        Some("1970-01-01"),
        Some("1970-01-01T00:00:00"),
        Some("1970-01-01T00:00:00Z"),
        Some("false"),
    ],
    [
        Some("3"),
        Some("9223372036854775807"),
        Some("9999999999.99"),
        Some("1.25"),
        Some(""),
        Some("2099-12-31"),
        Some("2099-12-31T23:59:59.999999"),
        Some("2099-12-31T23:59:59.999999Z"),
        Some("true"),
    ],
    [
        Some("4"),
        Some("-9223372036854775808"),
        Some("-9999999999.99"),
        Some("3"),
        Some("a_b"),
        Some("1900-01-01"),
        Some("1900-01-01T00:00:00"),
        Some("1900-01-01T00:00:00Z"),
        Some("false"),
    ],
    [
        Some("5"),
        Some("1"),
        Some("0.01"),
        Some("2.5"),
        Some("a%b"),
        Some("2026-10-03"),
        Some("2026-10-03T23:59:59"),
        Some("2026-10-04T09:59:59.999999Z"),
        Some("true"),
    ],
    [
        Some("6"),
        Some("2"),
        Some("100.00"),
        None,
        Some("o'neil"),
        Some("2026-10-05"),
        Some("2026-10-05T00:00:00.000001"),
        Some("2026-10-05T00:00:00.000001Z"),
        Some("true"),
    ],
    [Some("7"), None, None, None, None, None, None, None, None],
    [
        Some("8"),
        Some("3"),
        Some("1.10"),
        Some("4.75"),
        Some("ñandú"),
        Some("2000-02-29"),
        Some("2000-02-29T12:30:00"),
        Some("2000-02-29T12:30:00.5Z"),
        Some("false"),
    ],
    [
        Some("9"),
        Some("4"),
        None,
        Some("5.5"),
        Some("línea\nnueva"),
        None,
        None,
        None,
        Some("true"),
    ],
    [
        Some("10"),
        Some("10"),
        Some("7.00"),
        Some("-0.25"),
        Some("Jo"),
        Some("2026-01-01"),
        Some("2026-01-01T00:00:00"),
        Some("2026-01-01T00:00:00Z"),
        None,
    ],
];

/// Las filas de `tipos`, ya como valores.
pub fn valores() -> Vec<Vec<Option<Valor>>> {
    FILAS
        .iter()
        .map(|f| {
            f.iter()
                .zip(TIPOS)
                .map(|(v, c)| {
                    v.map(|t| {
                        c.fisico()
                            .analizar(t)
                            .unwrap_or_else(|| panic!("`{t}` no es un {}", c.tipo))
                    })
                })
                .collect()
        })
        .collect()
}

/// Cuántas filas tiene `grande`.
pub const GRANDE: u64 = 1_000_000;

/// Las columnas de `grande`: `id` de 1 a [`GRANDE`], `grupo = id % 100`,
/// `importe = id / 100` con dos decimales, `nota = 'fila-<id>'`.
pub const GRANDE_COLUMNAS: &[Columna] = &[
    c("id", "Integer"),
    c("grupo", "Integer"),
    c("importe", "Decimal<12, 2>"),
    c("nota", "String"),
];

/// Una fila de `grande`, en texto canónico.
pub fn fila_grande(id: u64) -> [String; 4] {
    [
        id.to_string(),
        (id % 100).to_string(),
        format!("{}.{:02}", id / 100, id % 100),
        format!("fila-{id}"),
    ]
}

/// Dos valores del mismo físico, comparados. `None` si no son comparables
/// (de tipos distintos, o un `NaN`).
pub fn comparar(a: &Valor, b: &Valor) -> Option<Ordering> {
    match (a, b) {
        (Valor::Texto(x), Valor::Texto(y)) => Some(x.cmp(y)),
        (Valor::Entero(x), Valor::Entero(y)) => Some(x.cmp(y)),
        (Valor::Real(x), Valor::Real(y)) => x.partial_cmp(y),
        (Valor::Logico(x), Valor::Logico(y)) => Some(x.cmp(y)),
        (Valor::Decimal(x), Valor::Decimal(y)) => Some(x.cmp(y)),
        (Valor::Fecha(x), Valor::Fecha(y)) => Some(x.cmp(y)),
        (Valor::Hora(x), Valor::Hora(y)) => Some(x.cmp(y)),
        (Valor::FechaHora(x), Valor::FechaHora(y)) => Some(x.cmp(y)),
        (Valor::Instante(x), Valor::Instante(y)) => Some(x.cmp(y)),
        _ => None,
    }
}

/// `LIKE` de SQL, sin carácter de escape: `%` cualquier secuencia, `_` un
/// carácter, y todo lo demás, literal y distinguiendo mayúsculas.
pub fn como(texto: &str, patron: &str) -> bool {
    fn ir(t: &[char], p: &[char]) -> bool {
        match p.split_first() {
            None => t.is_empty(),
            Some(('%', resto)) => (0..=t.len()).any(|i| ir(&t[i..], resto)),
            Some(('_', resto)) => !t.is_empty() && ir(&t[1..], resto),
            Some((c, resto)) => t.first() == Some(c) && ir(&t[1..], resto),
        }
    }
    let t: Vec<char> = texto.chars().collect();
    let p: Vec<char> = patron.chars().collect();
    ir(&t, &p)
}

/// Un filtro de la petición, con lo que su operador lleva.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Derecha {
    Uno(&'static str),
    Lista(&'static [&'static str]),
    Ninguno,
}

/// Un filtro de prueba sobre `tipos`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prueba {
    pub columna: &'static str,
    pub operador: &'static str,
    pub valor: Derecha,
}

const fn p(columna: &'static str, operador: &'static str, valor: Derecha) -> Prueba {
    Prueba {
        columna,
        operador,
        valor,
    }
}

/// **Los filtros del caso 2**: cada operador, sobre los tipos donde su
/// semántica no depende de nada más (no hay `lt` sobre texto: su orden es la
/// colación del origen, que el contrato no fija).
pub const PRUEBAS: &[Prueba] = &[
    p("texto", "eq", Derecha::Uno("ana")),
    p("texto", "eq", Derecha::Uno("")),
    p("entero", "eq", Derecha::Uno("-9223372036854775808")),
    p("importe", "eq", Derecha::Uno("100.00")),
    p("fecha", "eq", Derecha::Uno("2000-02-29")),
    p("instante", "eq", Derecha::Uno("2000-02-29T12:30:00.5Z")),
    p("logico", "eq", Derecha::Uno("false")),
    p("texto", "neq", Derecha::Uno("ana")),
    p("entero", "neq", Derecha::Uno("0")),
    p("texto", "in", Derecha::Lista(&["ana", "Ana", "ñandú"])),
    p("entero", "in", Derecha::Lista(&["1", "2", "99"])),
    p("texto", "in", Derecha::Lista(&[])),
    p("entero", "lt", Derecha::Uno("1")),
    p("entero", "le", Derecha::Uno("1")),
    p("entero", "gt", Derecha::Uno("3")),
    p("entero", "ge", Derecha::Uno("3")),
    p("importe", "lt", Derecha::Uno("0.01")),
    p("importe", "ge", Derecha::Uno("9999999999.99")),
    p("real", "gt", Derecha::Uno("2.5")),
    p("fecha", "le", Derecha::Uno("1970-01-01")),
    p("fecha", "gt", Derecha::Uno("2026-10-03")),
    p("local", "ge", Derecha::Uno("2026-10-03T23:59:59")),
    p("instante", "lt", Derecha::Uno("2026-10-04T10:00:00Z")),
    p(
        "instante",
        "ge",
        Derecha::Uno("2099-12-31T23:59:59.999999Z"),
    ),
    p("texto", "like", Derecha::Uno("a_b")),
    p("texto", "like", Derecha::Uno("%n%")),
    p("texto", "like", Derecha::Uno("A%")),
    p("importe", "isNull", Derecha::Ninguno),
    p("texto", "isNull", Derecha::Ninguno),
    p("logico", "isNotNull", Derecha::Ninguno),
];

fn indice(columna: &str) -> usize {
    TIPOS
        .iter()
        .position(|c| c.nombre == columna)
        .expect("una columna de la semilla")
}

/// **Los `id` que la prueba deja pasar**, calculados sobre la semilla con la
/// semántica de SQL: un nulo no cumple nada salvo `isNull`.
pub fn esperado(prueba: &Prueba) -> Vec<i64> {
    let i = indice(prueba.columna);
    let fisico = TIPOS[i].fisico();
    let analizar = |t: &str| {
        fisico
            .analizar(t)
            .unwrap_or_else(|| panic!("`{t}` no es un {}", TIPOS[i].tipo))
    };
    let mut ids = Vec::new();
    for fila in valores() {
        let v = &fila[i];
        let pasa = match (prueba.operador, &prueba.valor, v) {
            ("isNull", _, v) => v.is_none(),
            ("isNotNull", _, v) => v.is_some(),
            (_, _, None) => false,
            ("in", Derecha::Lista(l), Some(v)) => l.iter().any(|x| &analizar(x) == v),
            ("like", Derecha::Uno(pat), Some(Valor::Texto(t))) => como(t, pat),
            (op, Derecha::Uno(x), Some(v)) => {
                let o = comparar(v, &analizar(x));
                match op {
                    "eq" => o == Some(Ordering::Equal),
                    "neq" => o.is_some_and(|o| o != Ordering::Equal),
                    "lt" => o == Some(Ordering::Less),
                    "le" => o.is_some_and(|o| o != Ordering::Greater),
                    "gt" => o == Some(Ordering::Greater),
                    "ge" => o.is_some_and(|o| o != Ordering::Less),
                    otro => panic!("`{otro}` no tiene semántica en la semilla"),
                }
            }
            (op, d, _) => panic!("`{op}` con {d:?} no es una prueba"),
        };
        if pasa {
            let Some(Valor::Entero(id)) = &fila[0] else {
                unreachable!("`id` es un entero no nulo")
            };
            ids.push(*id);
        }
    }
    ids.sort();
    ids
}

/// **Los `id` de un `ORDER BY columna [DESC] NULLS LAST LIMIT n`**, en su
/// orden.
pub fn primeros(columna: &str, descendente: bool, n: usize) -> Vec<i64> {
    let i = indice(columna);
    let mut filas = valores();
    filas.sort_by(|a, b| match (&a[i], &b[i]) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => {
            let o = comparar(x, y).unwrap_or(Ordering::Equal);
            if descendente { o.reverse() } else { o }
        }
    });
    filas
        .iter()
        .take(n)
        .map(|f| match &f[0] {
            Some(Valor::Entero(id)) => *id,
            _ => unreachable!("`id` es un entero no nulo"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Toda fila de la semilla se lee en su tipo: un valor mal escrito aquí
    /// haría fallar a todos los conectores por culpa del kit.
    #[test]
    fn la_semilla_esta_en_su_tipo() {
        assert_eq!(valores().len(), FILAS.len());
        for id in [1, GRANDE / 2, GRANDE] {
            let f = fila_grande(id);
            for (v, c) in f.iter().zip(GRANDE_COLUMNAS) {
                assert!(c.fisico().analizar(v).is_some(), "{v} no es {}", c.tipo);
            }
        }
    }

    /// Cada prueba deja pasar algo y no todo —salvo la que prueba eso—, así
    /// que un conector que ignorara el filtro o lo negara no pasaría.
    #[test]
    fn cada_prueba_discrimina() {
        let todas = FILAS.len();
        for p in PRUEBAS {
            let e = esperado(p);
            if p.valor == Derecha::Lista(&[]) {
                assert!(e.is_empty(), "{p:?}");
                continue;
            }
            assert!(!e.is_empty() && e.len() < todas, "{p:?} → {e:?}");
        }
        assert!(
            crate::OPERADORES_PROBADOS
                .iter()
                .all(|op| PRUEBAS.iter().any(|p| p.operador == *op)),
            "falta un operador"
        );
    }

    #[test]
    fn la_semantica_de_la_semilla() {
        assert_eq!(esperado(&p("texto", "eq", Derecha::Uno(""))), [3]);
        assert_eq!(esperado(&p("texto", "like", Derecha::Uno("a_b"))), [4, 5]);
        // `neq` no deja pasar el nulo.
        assert!(!esperado(&p("texto", "neq", Derecha::Uno("ana"))).contains(&7));
        assert_eq!(
            esperado(&p("instante", "lt", Derecha::Uno("2026-10-04T10:00:00Z"))),
            [2, 4, 5, 8, 10]
        );
        // Los nulos al final también en `DESC`.
        assert_eq!(primeros("entero", true, 3), [3, 10, 9]);
        assert_eq!(primeros("entero", false, 2), [4, 2]);
        assert_eq!(primeros("importe", true, 11).last(), Some(&9));
        assert!(como("ñandú", "%n%") && !como("Ana", "a%"));
    }
}
