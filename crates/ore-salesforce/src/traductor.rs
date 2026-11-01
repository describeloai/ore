//! **De una `Peticion` a SOQL** (ADR 0083 §1.5). Una función pura: sin red y
//! sin org, porque la afirmación «la consulta pide solo las columnas proyectadas»
//! tiene que ser un aserto y no una promesa (0008).
//!
//! ```text
//!   Peticion { proyeccion, filtros, claves, orden, limit, rango }
//!       │   + Objeto (el `describe`: qué campo se filtra, se ordena, de qué tipo)
//!       ▼
//!   Traduccion { consultas: [SOQL…], residuales: [Filtro…], limit, vacia }
//! ```
//!
//! # Lo que el traductor tiene que arreglar de SOQL
//!
//! Medido en X0 (§3.1). El motor que recibe las filas es DuckDB, y cada
//! operador se escribe para que dé las filas que daría SQL:
//!
//! | SQL | SOQL ingenuo | lo que hace mal (medida) | lo que se escribe |
//! |---|---|---|---|
//! | `c <> v` (numérico) | `c != v` | **incluye los nulos** (B3: 112 frente a 75) | `(c != v AND c != NULL)` |
//! | `c = v` (texto) | `c = 'v'` | **no distingue mayúsculas** (B2) | `c = 'v'` **+ residual** exacto |
//! | `c <> v` (texto) | `c != 'v'` | **excluye** `'V'`, que SQL sí devuelve | **sin cláusula** + residual |
//! | `c < v` (texto) | `c < 'v'` | otra intercalación | **sin cláusula** + residual |
//! | `ORDER BY c` | `ORDER BY c` | nulos **primero en ASC y en DESC** (B9) | `ORDER BY c [DESC] NULLS LAST` |
//! | `NOT x` | `AND NOT x` | no compila (B8) | no se emite: `neq` ya lo resuelve |
//! | `c IN (…)` larga | por GET | `431`/`414` a ~16 KB (B14) | se **parte** en consultas de ≤ 8 KB |
//!
//! **Un residual** es un filtro que el servidor aplicó *de más* (o no aplicó):
//! quien lee lo reevalúa exacto sobre las filas que llegan. Es la misma idea
//! que `residual.rs` de Cassandra.
//!
//! Y se **rechaza**, no se ignora, lo que no se sabe escribir: un campo que
//! `describe` marca no filtrable (texto largo, B7; compuestos, B6), un tipo que
//! no se compara (`time`, `multipicklist`), un rango sin cursor.

use crate::esquema::{Campo, Categoria, Objeto};
use ore_driver::{Codigo, Fallo, Filtro, Peticion, Valor};

/// Los operadores del protocolo 2 (`docs/federation.md` §1.2), los diez.
pub const OPERADORES: &[&str] = &[
    "eq",
    "neq",
    "in",
    "lt",
    "le",
    "gt",
    "ge",
    "like",
    "isNull",
    "isNotNull",
];

/// La columna que dice qué fue cada fila de una lectura de cambios. ⚠️ **Provisional**,
/// la misma de `ore-mongodb` (`COLUMNA_OPERACION`): el contrato (0016) define el
/// rango pero no la forma de las filas de un changelog, y el motor todavía no
/// consume uno. **Solo sale si la petición la pide** en su proyección: quien hoy
/// lee por cursor sin pedirla recibe lo de siempre, sin una columna de más.
pub const COLUMNA_OPERACION: &str = "_ore_op";
/// Un registro tal como está ahora (alta o actualización).
pub const OP_UPSERT: &str = "upsert";
/// Un borrado: solo la clave, el resto nulo.
pub const OP_DELETE: &str = "delete";

/// El tope de una consulta por GET: ~10 KB ya codificada (B14: 400 ids pasan,
/// 800 dan `431`).
pub const MAX_SOQL: usize = 8000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Traduccion {
    /// Una o varias consultas SOQL. Varias, solo si una lista `IN` no cabía.
    pub consultas: Vec<String>,
    /// `(propiedad, columna)` en el orden de la petición.
    pub rutas: Vec<(String, String)>,
    /// Lo que se pide al servidor, en orden: las columnas de la proyección y
    /// **después** las que solo hacen falta para reevaluar un residual (el
    /// filtro de `Industry` con la proyección `Id` necesita traer `Industry`).
    /// Quien lee tira las de más al armar el lote.
    pub columnas: Vec<String>,
    /// Lo que el servidor no hizo exacto: se reevalúa al leer.
    pub residuales: Vec<Filtro>,
    /// El tope de filas **del total**: con varias consultas, cada una lleva su
    /// `LIMIT` pero la unión puede pasarse, y quien lee corta aquí.
    pub limit: Option<u64>,
    /// La petición no puede devolver filas (`IN ()`): no hay nada que preguntar.
    pub vacia: bool,
    /// La proyección pide [`COLUMNA_OPERACION`]: las filas de `consultas` son
    /// `upsert` y las de `borrados`, `delete`.
    pub operacion: bool,
    /// Los borrados del rango, para **`queryAll`** (X0 D6): solo `Id`, con el
    /// mismo cursor. `None` si no hay rango, o no se pidió la operación.
    pub borrados: Option<String>,
}

fn identificador(s: &str, que: &str) -> Result<(), Fallo> {
    let mut c = s.chars();
    let ok = c.next().is_some_and(|x| x.is_ascii_alphabetic())
        && s.chars().all(|x| x.is_ascii_alphanumeric() || x == '_');
    if ok {
        Ok(())
    } else {
        Err(Fallo::operador(format!(
            "`{s}` no es un nombre de {que} de Salesforce (letras, cifras y `_`)"
        )))
    }
}

fn columna<'a>(o: &'a Objeto, nombre: &str) -> Result<&'a Campo, Fallo> {
    identificador(nombre, "campo")?;
    o.campo(nombre).ok_or_else(|| {
        Fallo::new(
            Codigo::Esquema,
            format!("`{}.{nombre}`: el objeto no tiene esa columna", o.nombre),
        )
    })
}

/// Una cadena SOQL entre comillas simples.
fn cadena(v: &str) -> Result<String, Fallo> {
    if v.contains('\0') {
        return Err(Fallo::operador("un valor con NUL no se escribe en SOQL"));
    }
    let mut s = String::with_capacity(v.len() + 2);
    s.push('\'');
    for c in v.chars() {
        match c {
            '\\' => s.push_str("\\\\"),
            '\'' => s.push_str("\\'"),
            '\n' => s.push_str("\\n"),
            '\r' => s.push_str("\\r"),
            '\t' => s.push_str("\\t"),
            otro => s.push(otro),
        }
    }
    s.push('\'');
    Ok(s)
}

fn digitos(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// `2026-10-31T20:39:17Z` (con fracción y con zona o sin ella: sin ella, UTC).
fn instante(v: &str) -> Option<String> {
    let v = v.trim().replacen(' ', "T", 1);
    let (f, h) = v.split_once('T')?;
    let mut p = f.split('-');
    let (a, m, d) = (p.next()?, p.next()?, p.next()?);
    if !(a.len() == 4
        && m.len() == 2
        && d.len() == 2
        && digitos(a)
        && digitos(m)
        && digitos(d)
        && p.next().is_none())
    {
        return None;
    }
    let b = h.as_bytes();
    if b.len() < 8
        || !(digitos(&h[0..2])
            && b[2] == b':'
            && digitos(&h[3..5])
            && b[5] == b':'
            && digitos(&h[6..8]))
    {
        return None;
    }
    let mut resto = &h[8..];
    if let Some(r) = resto.strip_prefix('.') {
        let n = r.bytes().take_while(u8::is_ascii_digit).count();
        if n == 0 {
            return None;
        }
        resto = &r[n..];
    }
    let zona_ok = match resto {
        "" | "Z" => true,
        z => {
            let z = z.strip_prefix(['+', '-'])?;
            let z = z.replace(':', "");
            z.len() == 4 && digitos(&z)
        }
    };
    if !zona_ok {
        return None;
    }
    Some(if resto.is_empty() { format!("{v}Z") } else { v })
}

/// El literal de `v` para este campo: lo que SOQL espera de su tipo.
fn literal(c: &Campo, v: &str) -> Result<String, Fallo> {
    let malo = |que: &str| {
        Fallo::operador(format!(
            "`{}`: `{v}` no es {que} (el campo es `{}`)",
            c.nombre, c.tipo
        ))
    };
    match c.categoria() {
        Categoria::Texto => cadena(v),
        Categoria::Id => {
            if (15..=18).contains(&v.len()) && v.bytes().all(|b| b.is_ascii_alphanumeric()) {
                cadena(v)
            } else {
                Err(malo("un Id de Salesforce (15 o 18 caracteres)"))
            }
        }
        Categoria::Numero => {
            let t = v.trim();
            let ok = !t.is_empty()
                && t.bytes()
                    .all(|b| b.is_ascii_digit() || matches!(b, b'+' | b'-' | b'.' | b'e' | b'E'))
                && t.parse::<f64>().is_ok_and(f64::is_finite);
            if ok {
                Ok(t.to_string())
            } else {
                Err(malo("un número"))
            }
        }
        Categoria::Logico => match v.trim().to_ascii_lowercase().as_str() {
            b @ ("true" | "false") => Ok(b.to_string()),
            _ => Err(malo("`true` o `false`")),
        },
        Categoria::Fecha => {
            let t = v.trim();
            let b = t.as_bytes();
            if b.len() == 10
                && b[4] == b'-'
                && b[7] == b'-'
                && digitos(&t[0..4])
                && digitos(&t[5..7])
                && digitos(&t[8..10])
            {
                Ok(t.to_string())
            } else {
                Err(malo("una fecha `AAAA-MM-DD`"))
            }
        }
        Categoria::FechaHora => {
            instante(v).ok_or_else(|| malo("un instante `AAAA-MM-DDThh:mm:ssZ`"))
        }
        Categoria::Ninguna => Err(Fallo::operador(format!(
            "`{}` es de tipo `{}`: se lee, pero no se compara",
            c.nombre, c.tipo
        ))),
    }
}

fn uno(f: &Filtro) -> Result<&str, Fallo> {
    f.valor().ok_or_else(|| {
        Fallo::operador(format!(
            "`{}` sobre `{}` pide un valor",
            f.operador, f.columna
        ))
    })
}

/// Lo que un filtro aporta: cláusulas del servidor, una lista `IN`, o un residual.
#[derive(Default)]
struct Aporte {
    fijas: Vec<String>,
    listas: Vec<(String, Vec<String>)>,
    residuales: Vec<Filtro>,
    vacia: bool,
}

fn filtro(f: &Filtro, o: &Objeto, a: &mut Aporte) -> Result<(), Fallo> {
    let c = columna(o, &f.columna)?;
    if !c.filtrable {
        return Err(Fallo::operador(format!(
            "`{}.{}` no se puede filtrar: `describe` la marca no filtrable ({})",
            o.nombre,
            c.nombre,
            if c.es_compuesto() {
                "un compuesto se filtra por sus subcampos"
            } else {
                "texto largo"
            }
        )));
    }
    let n = c.nombre.as_str();
    let texto = c.categoria() == Categoria::Texto;
    match f.operador.as_str() {
        "isNull" => a.fijas.push(format!("{n} = NULL")),
        "isNotNull" => a.fijas.push(format!("{n} != NULL")),
        "eq" => {
            a.fijas.push(format!("{n} = {}", literal(c, uno(f)?)?));
            if texto {
                a.residuales.push(f.clone());
            }
        }
        "neq" => {
            let lit = literal(c, uno(f)?)?;
            if texto {
                // SOQL no distingue mayúsculas: `!= 'a'` quitaría `'A'`, que
                // SQL sí devuelve. Sin cláusula, y el residual lo hace exacto.
                a.residuales.push(f.clone());
            } else {
                a.fijas.push(format!("({n} != {lit} AND {n} != NULL)"));
            }
        }
        op @ ("lt" | "le" | "gt" | "ge") => {
            let lit = literal(c, uno(f)?)?;
            if c.categoria() == Categoria::Logico {
                return Err(Fallo::operador(format!(
                    "`{op}` sobre `{n}`: un booleano no se ordena"
                )));
            }
            if texto {
                a.residuales.push(f.clone());
            } else {
                let s = match op {
                    "lt" => "<",
                    "le" => "<=",
                    "gt" => ">",
                    _ => ">=",
                };
                a.fijas.push(format!("{n} {s} {lit}"));
            }
        }
        "like" => {
            if !texto {
                return Err(Fallo::operador(format!(
                    "`like` sobre `{n}`: no es un campo de texto"
                )));
            }
            let patron = uno(f)?;
            if patron.contains('\\') {
                return Err(Fallo::operador(format!(
                    "`like` sobre `{n}`: el escape `\\` no está definido en el contrato"
                )));
            }
            a.fijas.push(format!("{n} LIKE {}", cadena(patron)?));
            a.residuales.push(f.clone());
        }
        "in" => {
            let Valor::Lista(vs) = &f.valor else {
                return Err(Fallo::operador(format!("`in` sobre `{n}` pide una lista")));
            };
            if vs.is_empty() {
                a.vacia = true;
                return Ok(());
            }
            let lits = vs
                .iter()
                .map(|v| literal(c, v))
                .collect::<Result<Vec<_>, _>>()?;
            a.listas.push((n.to_string(), lits));
            if texto {
                a.residuales.push(f.clone());
            }
        }
        otro => {
            return Err(Fallo::operador(format!(
                "`{otro}` sobre `{}`: no es uno de los diez operadores del protocolo",
                f.columna
            )));
        }
    }
    Ok(())
}

/// Traduce una petición. `Err` es siempre un [`Fallo`] del conector: quien
/// pidió mandó lo que el conector no declaró o no sabe escribir.
pub fn traducir(p: &Peticion, o: &Objeto) -> Result<Traduccion, Fallo> {
    if p.fichero.is_some() || p.listado.is_some() {
        return Err(Fallo::operador(
            "`fichero` y `listado` son de los orígenes de objetos: no aplican a Salesforce",
        ));
    }
    identificador(&p.objeto, "objeto")?;
    if p.objeto != o.nombre {
        return Err(Fallo::operador(format!(
            "la petición es de `{}` y el esquema, de `{}`",
            p.objeto, o.nombre
        )));
    }
    if !o.consultable {
        return Err(Fallo::operador(format!(
            "`{}` no es consultable por SOQL",
            o.nombre
        )));
    }
    if p.proyeccion.is_empty() {
        return Err(Fallo::operador(
            "la proyección está vacía: no se pide `SELECT *` ni nada (0008: la proyección es una lista cerrada)",
        ));
    }
    let mut cols = Vec::new();
    let mut operacion = false;
    for (_, col) in &p.proyeccion {
        if col == COLUMNA_OPERACION {
            // Virtual: no es un campo de Salesforce, la pone quien lee.
            operacion = true;
            continue;
        }
        let c = columna(o, col)?;
        if !c.seleccionable() {
            return Err(Fallo::operador(format!(
                "`{}.{}` es de tipo `{}` y no sale por SOQL en filas{}",
                o.nombre,
                c.nombre,
                c.tipo,
                if c.es_compuesto() {
                    " (pide sus subcampos)"
                } else {
                    ""
                }
            )));
        }
        cols.push(c.nombre.as_str());
    }
    if operacion {
        if !cols.contains(&"Id") {
            return Err(Fallo::operador(format!(
                "`{COLUMNA_OPERACION}` pide la clave: un borrado no trae más que `Id`, y sin ella no se sabe qué fila retirar"
            )));
        }
        if !p.filtros.is_empty()
            || !p.orden.is_empty()
            || p.limit.is_some()
            || !p.clave_columnas.is_empty()
        {
            return Err(Fallo::operador(format!(
                "una lectura con `{COLUMNA_OPERACION}` devuelve el changelog sin filtrar, ordenar ni recortar (un borrado no trae el documento): eso lo hace el motor"
            )));
        }
    }

    let mut a = Aporte::default();
    if !p.clave_columnas.is_empty() {
        if p.clave_columnas.len() != 1 {
            return Err(Fallo::operador(
                "las claves de más de una columna no se escriben todavía (paso 2)",
            ));
        }
        let vs = p.claves.iter().filter_map(|t| t.first().cloned()).collect();
        let f = Filtro {
            columna: p.clave_columnas[0].clone(),
            operador: "in".into(),
            valor: Valor::Lista(vs),
        };
        filtro(&f, o, &mut a)?;
    }
    for f in &p.filtros {
        filtro(f, o, &mut a)?;
    }

    let mut borrados = None;
    if p.start.is_some() || p.end.is_some() {
        let Some(cur) = &p.cursor else {
            return Err(Fallo::operador(
                "el rango por posición no existe en Salesforce por REST (el CDC es otro paso): hace falta una columna cursor",
            ));
        };
        let c = columna(o, cur)?;
        if !c.filtrable
            || !matches!(
                c.categoria(),
                Categoria::FechaHora | Categoria::Fecha | Categoria::Numero
            )
        {
            return Err(Fallo::operador(format!(
                "`{cur}` no sirve de cursor: no se filtra o no es un instante"
            )));
        }
        let mut rango = Vec::new();
        if let Some(s) = &p.start {
            rango.push(format!("{} > {}", c.nombre, literal(c, s)?));
        }
        if let Some(e) = &p.end {
            rango.push(format!("{} <= {}", c.nombre, literal(c, e)?));
        }
        if operacion {
            // Los borrados no salen de `query` sino de `queryAll` (X0 D6), y su
            // `SystemModstamp` es el de cuando se borraron: el mismo cursor.
            if !o.campo("IsDeleted").is_some_and(|f| f.filtrable) {
                return Err(Fallo::operador(format!(
                    "`{}` no tiene `IsDeleted`: sus borrados no se pueden leer, y no se declara que se mantiene",
                    o.nombre
                )));
            }
            let mut donde = vec!["IsDeleted = true".to_string()];
            donde.extend(rango.iter().cloned());
            borrados = Some(format!(
                "SELECT Id FROM {} WHERE {}",
                o.nombre,
                donde.join(" AND ")
            ));
        }
        a.fijas.extend(rango);
    }

    let rutas = p.proyeccion.clone();
    let mut columnas: Vec<String> = cols.iter().map(|c| c.to_string()).collect();
    for r in &a.residuales {
        let c = columna(o, &r.columna)?;
        if !columnas.contains(&c.nombre) {
            columnas.push(c.nombre.clone());
        }
    }
    if a.vacia {
        return Ok(Traduccion {
            consultas: vec![],
            rutas,
            columnas,
            residuales: vec![],
            limit: p.limit,
            vacia: true,
            operacion,
            borrados: None,
        });
    }

    // Las listas `IN` después de la primera van enteras; la primera se parte si no cabe.
    let mut listas = a.listas.into_iter();
    let partible = listas.next();
    for (c, lits) in listas {
        a.fijas.push(format!("{c} IN ({})", lits.join(", ")));
    }

    let mut cola = String::new();
    if !p.orden.is_empty() {
        let mut partes = Vec::new();
        for ord in &p.orden {
            let c = columna(o, &ord.columna)?;
            if !c.ordenable {
                return Err(Fallo::operador(format!(
                    "`{}.{}` no se puede ordenar: `describe` la marca no ordenable",
                    o.nombre, c.nombre
                )));
            }
            // B9: sin esto, los nulos van primero en los dos sentidos.
            partes.push(format!(
                "{} {} NULLS LAST",
                c.nombre,
                if ord.descendente { "DESC" } else { "ASC" }
            ));
        }
        cola.push_str(&format!(" ORDER BY {}", partes.join(", ")));
    }
    // **El `LIMIT` solo baja al servidor si no hay residuales.** Con uno, el
    // servidor cortaría a `n` filas y el residual quitaría algunas: llegarían
    // menos de las pedidas. Entonces corta quien lee, después de reevaluar.
    if let Some(n) = p.limit
        && a.residuales.is_empty()
    {
        cola.push_str(&format!(" LIMIT {n}"));
    }
    let seleccion = format!("SELECT {} FROM {}", columnas.join(", "), o.nombre);
    let armar = |extra: Option<String>| {
        let mut donde = a.fijas.clone();
        donde.extend(extra);
        let w = if donde.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", donde.join(" AND "))
        };
        format!("{seleccion}{w}{cola}")
    };

    let consultas = match partible {
        None => vec![armar(None)],
        Some((col, lits)) => {
            let base = armar(Some(format!("{col} IN ()"))).len();
            let mut trozos: Vec<Vec<&str>> = vec![vec![]];
            let mut largo = base;
            for l in &lits {
                let costo = l.len() + 2;
                if base + costo > MAX_SOQL {
                    return Err(Fallo::operador(format!(
                        "un valor de `{col}` no cabe en una consulta de {MAX_SOQL} caracteres"
                    )));
                }
                if largo + costo > MAX_SOQL {
                    trozos.push(vec![]);
                    largo = base;
                }
                largo += costo;
                trozos.last_mut().expect("siempre hay un trozo").push(l);
            }
            if trozos.len() > 1 && !p.orden.is_empty() {
                return Err(Fallo::operador(format!(
                    "`orderBy` con una lista `IN` de `{col}` que no cabe en una consulta ({} trozos): no se sabe fusionar",
                    trozos.len()
                )));
            }
            trozos
                .iter()
                .map(|t| armar(Some(format!("{col} IN ({})", t.join(", ")))))
                .collect()
        }
    };
    Ok(Traduccion {
        consultas,
        rutas,
        columnas,
        residuales: a.residuales,
        limit: p.limit,
        vacia: false,
        operacion,
        borrados,
    })
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use crate::esquema::pruebas::cuenta;
    use ore_driver::Orden;

    fn pet(cols: &[&str]) -> Peticion {
        Peticion {
            objeto: "Account".into(),
            proyeccion: cols
                .iter()
                .map(|c| (c.to_string(), c.to_string()))
                .collect(),
            ..Default::default()
        }
    }

    fn una(p: &Peticion) -> String {
        let t = traducir(p, &cuenta()).unwrap();
        assert_eq!(t.consultas.len(), 1, "{t:?}");
        t.consultas[0].clone()
    }

    fn operador(p: &Peticion) -> String {
        let e = traducir(p, &cuenta()).unwrap_err();
        assert_eq!(e.codigo, Codigo::Operador, "{e}");
        e.mensaje
    }

    /// B1: lo que sale es lo que se pidió.
    #[test]
    fn la_consulta_pide_solo_las_columnas_proyectadas() {
        assert_eq!(una(&pet(&["Id", "Name"])), "SELECT Id, Name FROM Account");
    }

    #[test]
    fn una_proyeccion_vacia_se_rechaza() {
        assert!(operador(&pet(&[])).contains("proyección está vacía"));
    }

    /// B3: SOQL incluye los nulos en `!=`; SQL no.
    #[test]
    fn neq_numerico_excluye_los_nulos() {
        let mut p = pet(&["Id"]);
        p.filtros.push(Filtro::uno("NumberOfEmployees", "neq", "5"));
        assert_eq!(
            una(&p),
            "SELECT Id FROM Account WHERE (NumberOfEmployees != 5 AND NumberOfEmployees != NULL)"
        );
    }

    /// B2: `!= 'a'` quitaría `'A'`. Sin cláusula y con residual.
    #[test]
    fn neq_de_texto_no_se_empuja_y_queda_residual() {
        let mut p = pet(&["Id"]);
        p.filtros.push(Filtro::uno("Industry", "neq", "Banking"));
        let t = traducir(&p, &cuenta()).unwrap();
        // Sin cláusula, pero trae `Industry`: hace falta para reevaluar.
        assert_eq!(t.consultas, ["SELECT Id, Industry FROM Account"]);
        assert_eq!(t.columnas, ["Id", "Industry"]);
        assert_eq!(t.residuales.len(), 1);
    }

    /// Un residual quita filas: el `LIMIT` del servidor devolvería de menos.
    #[test]
    fn con_un_residual_el_limit_no_baja_al_servidor() {
        let mut p = pet(&["Id"]);
        p.filtros.push(Filtro::uno("Name", "like", "X0-%"));
        p.limit = Some(5);
        let t = traducir(&p, &cuenta()).unwrap();
        assert_eq!(
            t.consultas,
            ["SELECT Id, Name FROM Account WHERE Name LIKE 'X0-%'"]
        );
        assert_eq!(t.limit, Some(5), "corta quien lee, ya reevaluado");
        // Sin residual, sí baja.
        let mut p = pet(&["Id"]);
        p.filtros.push(Filtro::uno("NumberOfEmployees", "ge", "1"));
        p.limit = Some(5);
        assert_eq!(
            una(&p),
            "SELECT Id FROM Account WHERE NumberOfEmployees >= 1 LIMIT 5"
        );
    }

    /// Si la columna del residual ya se proyecta, no se pide dos veces.
    #[test]
    fn la_columna_de_un_residual_no_se_repite() {
        let mut p = pet(&["Name", "Id"]);
        p.filtros.push(Filtro::uno("Name", "eq", "x"));
        let t = traducir(&p, &cuenta()).unwrap();
        assert_eq!(t.columnas, ["Name", "Id"]);
    }

    /// B2: `=` de texto es un superconjunto: se empuja y se reevalúa.
    #[test]
    fn eq_de_texto_se_empuja_y_queda_residual() {
        let mut p = pet(&["Id"]);
        p.filtros.push(Filtro::uno("Name", "eq", "O'Brien"));
        let t = traducir(&p, &cuenta()).unwrap();
        assert_eq!(
            t.consultas,
            [r"SELECT Id, Name FROM Account WHERE Name = 'O\'Brien'"]
        );
        assert_eq!(t.residuales, [Filtro::uno("Name", "eq", "O'Brien")]);
    }

    #[test]
    fn un_rango_de_texto_no_se_empuja_pero_uno_numerico_si() {
        let mut p = pet(&["Id"]);
        p.filtros.push(Filtro::uno("Name", "gt", "M"));
        p.filtros
            .push(Filtro::uno("NumberOfEmployees", "ge", "100"));
        p.filtros
            .push(Filtro::uno("NumberOfEmployees", "lt", "500"));
        let t = traducir(&p, &cuenta()).unwrap();
        assert_eq!(
            t.consultas,
            [
                "SELECT Id, Name FROM Account WHERE NumberOfEmployees >= 100 AND NumberOfEmployees < 500"
            ]
        );
        assert_eq!(t.residuales.len(), 1);
    }

    #[test]
    fn isnull_e_isnotnull() {
        let mut p = pet(&["Id"]);
        p.filtros.push(Filtro {
            columna: "Industry".into(),
            operador: "isNull".into(),
            valor: Valor::Ninguno,
        });
        p.filtros.push(Filtro {
            columna: "BillingCity".into(),
            operador: "isNotNull".into(),
            valor: Valor::Ninguno,
        });
        assert_eq!(
            una(&p),
            "SELECT Id FROM Account WHERE Industry = NULL AND BillingCity != NULL"
        );
    }

    /// B9: sin `NULLS LAST` los nulos van primero en los dos sentidos.
    #[test]
    fn el_orden_lleva_siempre_nulls_last() {
        let mut p = pet(&["Id"]);
        p.orden = vec![
            Orden {
                columna: "Industry".into(),
                descendente: false,
            },
            Orden {
                columna: "Name".into(),
                descendente: true,
            },
        ];
        p.limit = Some(5);
        assert_eq!(
            una(&p),
            "SELECT Id FROM Account ORDER BY Industry ASC NULLS LAST, Name DESC NULLS LAST LIMIT 5"
        );
    }

    /// B7: el texto largo ni se filtra ni se ordena.
    #[test]
    fn el_texto_largo_no_se_filtra_ni_se_ordena() {
        let mut p = pet(&["Id"]);
        p.filtros.push(Filtro::uno("Description", "eq", "x"));
        assert!(operador(&p).contains("no filtrable"));
        let mut p = pet(&["Id"]);
        p.orden = vec![Orden {
            columna: "Description".into(),
            descendente: false,
        }];
        assert!(operador(&p).contains("no ordenable"));
        // ...pero se lee.
        assert_eq!(
            una(&pet(&["Description"])),
            "SELECT Description FROM Account"
        );
    }

    /// B6: un compuesto no es columna; sus subcampos sí.
    #[test]
    fn el_compuesto_se_pide_por_sus_subcampos() {
        let m = operador(&pet(&["BillingAddress"]));
        assert!(m.contains("pide sus subcampos"), "{m}");
        assert_eq!(
            una(&pet(&["BillingCity"])),
            "SELECT BillingCity FROM Account"
        );
        let mut p = pet(&["Id"]);
        p.filtros.push(Filtro::uno("BillingAddress", "isNull", ""));
        assert!(operador(&p).contains("un compuesto se filtra por sus subcampos"));
    }

    #[test]
    fn like_se_empuja_con_residual_y_rechaza_el_escape() {
        let mut p = pet(&["Id"]);
        p.filtros.push(Filtro::uno("Name", "like", "X0-%"));
        let t = traducir(&p, &cuenta()).unwrap();
        assert_eq!(
            t.consultas,
            ["SELECT Id, Name FROM Account WHERE Name LIKE 'X0-%'"]
        );
        assert_eq!(t.residuales.len(), 1);
        let mut p = pet(&["Id"]);
        p.filtros.push(Filtro::uno("Name", "like", r"a\_b"));
        assert!(operador(&p).contains("escape"));
    }

    /// El literal se valida contra el tipo: un valor no es una inyección.
    #[test]
    fn un_valor_que_no_es_del_tipo_no_se_escribe() {
        for (campo, valor) in [
            ("NumberOfEmployees", "5 OR Name != ''"),
            ("NumberOfEmployees", ""),
            ("IsDeleted", "quizas"),
            ("CreatedDate", "ayer"),
            ("Id", "001' OR Name != '"),
        ] {
            let mut p = pet(&["Id"]);
            p.filtros.push(Filtro::uno(campo, "eq", valor));
            let m = operador(&p);
            assert!(m.contains(&format!("`{campo}`")), "{campo}={valor}: {m}");
        }
    }

    #[test]
    fn una_cadena_se_escapa_entera() {
        let mut p = pet(&["Id"]);
        p.filtros.push(Filtro::uno("Name", "eq", "a\\b'c\nd"));
        assert_eq!(
            una(&p),
            r"SELECT Id, Name FROM Account WHERE Name = 'a\\b\'c\nd'"
        );
    }

    #[test]
    fn los_instantes_se_normalizan_a_soql() {
        let mut p = pet(&["Id"]);
        p.filtros
            .push(Filtro::uno("CreatedDate", "ge", "2026-10-31 20:39:17"));
        p.filtros.push(Filtro::uno(
            "CreatedDate",
            "lt",
            "2026-11-01T00:00:00.000+02:00",
        ));
        assert_eq!(
            una(&p),
            "SELECT Id FROM Account WHERE CreatedDate >= 2026-10-31T20:39:17Z AND CreatedDate < 2026-11-01T00:00:00.000+02:00"
        );
    }

    /// El rango de una copia incremental: `start` exclusivo y `end` inclusivo.
    #[test]
    fn el_rango_va_sobre_la_columna_cursor() {
        let mut p = pet(&["Id"]);
        p.cursor = Some("SystemModstamp".into());
        p.start = Some("2026-10-31T20:39:17Z".into());
        p.end = Some("2026-10-31T21:00:00Z".into());
        assert_eq!(
            una(&p),
            "SELECT Id FROM Account WHERE SystemModstamp > 2026-10-31T20:39:17Z AND SystemModstamp <= 2026-10-31T21:00:00Z"
        );
        p.cursor = None;
        assert!(operador(&p).contains("columna cursor"));
    }

    #[test]
    fn una_columna_que_no_existe_es_esquema() {
        let e = traducir(&pet(&["Nada"]), &cuenta()).unwrap_err();
        assert_eq!(e.codigo, Codigo::Esquema);
        assert!(e.mensaje.contains("`Account.Nada`"), "{}", e.mensaje);
    }

    #[test]
    fn un_nombre_con_sintaxis_no_pasa() {
        let mut p = pet(&["Id"]);
        p.filtros.push(Filtro::uno("Name; DROP", "eq", "x"));
        assert!(operador(&p).contains("no es un nombre de campo"));
    }

    #[test]
    fn in_vacio_no_pregunta_nada() {
        let mut p = pet(&["Id"]);
        p.filtros.push(Filtro {
            columna: "Id".into(),
            operador: "in".into(),
            valor: Valor::Lista(vec![]),
        });
        let t = traducir(&p, &cuenta()).unwrap();
        assert!(t.vacia && t.consultas.is_empty());
    }

    fn ids(n: usize) -> Vec<String> {
        // Un Id de 18 caracteres: 5 + 10 + 3.
        (0..n).map(|i| format!("001fj{i:010}AAA")).collect()
    }

    /// B14: una lista `IN` que no cabe por GET se parte, sin perder ni repetir.
    #[test]
    fn una_lista_larga_se_parte_en_consultas_que_caben() {
        let mut p = pet(&["Id"]);
        let todos = ids(1200);
        p.filtros.push(Filtro {
            columna: "Id".into(),
            operador: "in".into(),
            valor: Valor::Lista(todos.clone()),
        });
        p.filtros.push(Filtro::uno("NumberOfEmployees", "ge", "1"));
        p.limit = Some(10);
        let t = traducir(&p, &cuenta()).unwrap();
        assert!(t.consultas.len() > 1, "1200 ids no caben en una");
        assert_eq!(t.limit, Some(10), "el tope es del total: quien lee corta");
        let mut vistos = Vec::new();
        for q in &t.consultas {
            assert!(q.len() <= MAX_SOQL, "{} caracteres", q.len());
            assert!(q.contains("NumberOfEmployees >= 1 AND Id IN ("), "{q}");
            assert!(q.ends_with("LIMIT 10"), "{q}");
            let dentro = q.split("IN (").nth(1).unwrap().split(')').next().unwrap();
            vistos.extend(dentro.split(", ").map(|s| s.trim_matches('\'').to_string()));
        }
        assert_eq!(vistos, todos, "ni se pierde ni se repite, y en orden");
    }

    #[test]
    fn una_lista_partida_no_se_ordena() {
        let mut p = pet(&["Id"]);
        p.filtros.push(Filtro {
            columna: "Id".into(),
            operador: "in".into(),
            valor: Valor::Lista(ids(1200)),
        });
        p.orden = vec![Orden {
            columna: "Name".into(),
            descendente: false,
        }];
        assert!(operador(&p).contains("no se sabe fusionar"));
    }

    #[test]
    fn la_clave_de_una_columna_es_un_in() {
        let mut p = pet(&["Id"]);
        p.clave_columnas = vec!["Id".into()];
        p.claves = vec![
            vec!["001fj0000000001AAA".into()],
            vec!["001fj0000000002AAA".into()],
        ];
        assert_eq!(
            una(&p),
            "SELECT Id FROM Account WHERE Id IN ('001fj0000000001AAA', '001fj0000000002AAA')"
        );
        p.clave_columnas = vec!["Id".into(), "Name".into()];
        assert!(operador(&p).contains("más de una columna"));
    }

    #[test]
    fn un_operador_que_no_es_del_protocolo_se_rechaza() {
        let mut p = pet(&["Id"]);
        p.filtros.push(Filtro::uno("Name", "regex", "x"));
        assert!(operador(&p).contains("no es uno de los diez"));
    }

    fn con_rango(cols: &[&str]) -> Peticion {
        let mut p = pet(cols);
        p.cursor = Some("SystemModstamp".into());
        p.start = Some("2026-10-31T20:39:17Z".into());
        p.end = Some("2026-10-31T21:00:00Z".into());
        p
    }

    /// ADR 0083 §1.8: la operación es virtual (no sale en el `SELECT`) y los
    /// borrados salen de `queryAll` con el mismo cursor (X0 D6).
    #[test]
    fn la_operacion_no_va_al_select_y_trae_la_consulta_de_borrados() {
        let t = traducir(&con_rango(&["Id", "Name", "_ore_op"]), &cuenta()).unwrap();
        assert!(t.operacion);
        assert_eq!(
            t.consultas,
            [
                "SELECT Id, Name FROM Account WHERE SystemModstamp > 2026-10-31T20:39:17Z AND SystemModstamp <= 2026-10-31T21:00:00Z"
            ]
        );
        assert_eq!(
            t.borrados.as_deref(),
            Some(
                "SELECT Id FROM Account WHERE IsDeleted = true AND SystemModstamp > 2026-10-31T20:39:17Z AND SystemModstamp <= 2026-10-31T21:00:00Z"
            )
        );
        assert_eq!(t.columnas, ["Id", "Name"]);
        assert!(
            t.rutas.iter().any(|(_, c)| c == COLUMNA_OPERACION),
            "sale en el lote"
        );
    }

    #[test]
    fn sin_rango_la_operacion_es_el_estado_y_no_hay_borrados() {
        let t = traducir(&pet(&["Id", "_ore_op"]), &cuenta()).unwrap();
        assert!(t.operacion);
        assert_eq!(
            t.borrados, None,
            "una lectura entera es el estado: nada que retirar"
        );
        assert_eq!(t.consultas, ["SELECT Id FROM Account"]);
    }

    /// Quien lee por cursor **sin pedir la operación** recibe lo de siempre.
    #[test]
    fn sin_pedir_la_operacion_no_hay_ni_columna_ni_borrados() {
        let t = traducir(&con_rango(&["Id", "Name"]), &cuenta()).unwrap();
        assert!(!t.operacion);
        assert_eq!(t.borrados, None);
    }

    #[test]
    fn la_operacion_exige_la_clave_en_la_proyeccion() {
        let m = operador(&con_rango(&["Name", "_ore_op"]));
        assert!(m.contains("pide la clave"), "{m}");
    }

    /// Un changelog no se filtra, ordena ni recorta: un borrado no trae el documento.
    #[test]
    fn un_changelog_no_se_filtra_ni_se_ordena_ni_se_recorta() {
        let mut p = con_rango(&["Id", "_ore_op"]);
        p.filtros.push(Filtro::uno("NumberOfEmployees", "ge", "1"));
        assert!(operador(&p).contains("sin filtrar, ordenar ni recortar"));
        let mut p = con_rango(&["Id", "_ore_op"]);
        p.limit = Some(5);
        assert!(operador(&p).contains("sin filtrar, ordenar ni recortar"));
        let mut p = con_rango(&["Id", "_ore_op"]);
        p.orden = vec![Orden {
            columna: "Name".into(),
            descendente: false,
        }];
        assert!(operador(&p).contains("sin filtrar, ordenar ni recortar"));
    }

    /// Un objeto sin `IsDeleted` no puede decir sus borrados: se niega, no los calla.
    #[test]
    fn un_objeto_sin_isdeleted_no_lee_cambios() {
        let mut o = cuenta();
        o.campos.retain(|c| c.nombre != "IsDeleted");
        let e = traducir(&con_rango(&["Id", "_ore_op"]), &o).unwrap_err();
        assert_eq!(e.codigo, Codigo::Operador);
        assert!(e.mensaje.contains("no tiene `IsDeleted`"), "{}", e.mensaje);
    }

    #[test]
    fn lo_de_los_objetos_de_ficheros_no_aplica() {
        let mut p = pet(&["Id"]);
        p.listado = Some(String::new());
        assert!(operador(&p).contains("no aplican a Salesforce"));
    }
}
