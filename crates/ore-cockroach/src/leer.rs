//! **Leer**: la petición, en Arrow y en flujo, por `salida` (ADR 0080, paso 2).
//!
//! ⚠️ **Adaptado de `ore-read-postgres/src/lectura.rs`** (`columnas`, `planear`, `en_linea`, `sql_de`, `por_lotes`), sin las lecturas en
//! tramos por `ctid`, sin geometría y sin filtros, orden ni límite (pasos 3 a 5). Lo que hay hoy es lo mínimo que el kit pide
//! (casos 6, 1, 7 y, con el texto, el 5): la proyección de una tabla entera.
//!
//! La consulta es `SELECT col::STRING, … FROM (<consulta de ore-sql>) AS ore_q` en una transacción de **solo lectura**; cada valor llega
//! como el texto que escribe CockroachDB y se lee con `ore_base::tipos`, igual que en Postgres.

use crate::lectura::{Sesion, Token, fallo_de};
use crate::tipos::{Aviso, Forma, poner};
use arrow_array::RecordBatch;
use arrow_schema::{Field, Schema};
use ore_arrow::{Columna, Emisor, RecordBatchBuilder};
use ore_driver::{Codigo, Fallo, Peticion};
use std::collections::BTreeMap;
use std::io::Write;
use std::sync::Arc;

/// Las filas que van en cada lote de Arrow.
pub const LOTE: usize = 8192;

/// `"esquema"."tabla"`, citado parte a parte: lo que `to_regclass` lee sin que un nombre pueda decir otra cosa.
pub fn citado(objeto: &str) -> String {
    objeto
        .split('.')
        .map(|p| format!("\"{}\"", p.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(".")
}

/// Las columnas del objeto, con su forma, en el orden de la tabla. `objeto` si no existe (o no se puede ver).
pub fn columnas(c: &mut postgres::Client, objeto: &str) -> Result<Vec<(String, Forma)>, Fallo> {
    let citado = citado(objeto);
    let filas = c
        .query_typed(
            "SELECT a.attname::text, format_type(a.atttypid, a.atttypmod), t.typtype::text, \
                    CASE WHEN t.typtype = 'd' THEN format_type(t.typbasetype, t.typtypmod) END, \
                    to_regclass($1) IS NOT NULL \
             FROM (SELECT to_regclass($1) AS r) o \
             LEFT JOIN pg_attribute a ON a.attrelid = o.r AND a.attnum > 0 AND NOT a.attisdropped \
             LEFT JOIN pg_type t ON t.oid = a.atttypid \
             ORDER BY a.attnum",
            &[(
                &citado as &(dyn postgres::types::ToSql + Sync),
                postgres::types::Type::TEXT,
            )],
        )
        .map_err(|e| fallo_de(&e))?;
    if filas.first().is_none_or(|f| !f.get::<_, bool>(4)) {
        return Err(Fallo::new(
            Codigo::Objeto,
            format!("`{objeto}` no existe en el origen (o no se puede ver)"),
        ));
    }
    Ok(filas
        .iter()
        .filter_map(|f| {
            let nombre: Option<String> = f.get(0);
            let formato: Option<String> = f.get(1);
            let familia: Option<String> = f.get(2);
            let base: Option<String> = f.get(3);
            Some((
                nombre?,
                Forma::de(&formato?, &familia.unwrap_or_default(), base.as_deref()),
            ))
        })
        .collect())
}

/// **La tabla está y le falta una columna declarada** (ADR 0063 S5b): el esquema del origen cambió. `esquema` y no `objeto`.
fn deriva(objeto: &str, col: &str, cols: &[(String, Forma)]) -> Fallo {
    Fallo::new(
        Codigo::Esquema,
        format!(
            "the source schema changed: `{objeto}` no longer has the column `{col}` (its columns now: {})",
            cols.iter()
                .map(|(n, _)| n.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    )
}

/// La consulta de la petición, ya traducida, y lo que hace falta para leerla.
struct Plan {
    consulta: ore_sql::Consulta,
    /// Por propiedad: su posición en el resultado, su forma y su columna.
    salida: Vec<(String, usize, Forma, String)>,
    esquema: Arc<Schema>,
}

fn planear(c: &mut postgres::Client, p: &Peticion) -> Result<Plan, Fallo> {
    if let Some(porque) = ore_driver::rango_servible(p, false, false) {
        return Err(Fallo::operador(porque));
    }
    <crate::Cockroach as ore_driver::origen::Origen>::CAPACIDADES.admite(p)?;
    if !p.claves.is_empty() {
        return Err(Fallo::operador(
            "el recorte por clave todavía no se empuja a CockroachDB",
        ));
    }
    let consulta = ore_sql::preparar(p, &ore_sql::dialectos::POSTGRES, &p.objeto, || {
        unreachable!("`POSTGRES` no exige tipos")
    })
    .map_err(Fallo::operador)?;
    let cols = columnas(c, &p.objeto)?;
    let mut salida = Vec::new();
    for (prop, col) in &p.proyeccion {
        let f = cols
            .iter()
            .find(|(n, _)| n == col)
            .ok_or_else(|| deriva(&p.objeto, col, &cols))?
            .1
            .clone();
        let i = consulta
            .columnas
            .iter()
            .position(|x| x == col)
            .ok_or_else(|| Fallo::origen(format!("la consulta no pide `{col}`")))?;
        salida.push((prop.clone(), i, f, col.clone()));
    }
    let esquema = Arc::new(Schema::new(
        salida
            .iter()
            .map(|(prop, _, f, _)| Field::new(prop, ore_arrow::tipo_arrow(&f.ore()), true))
            .collect::<Vec<_>>(),
    ));
    Ok(Plan {
        consulta,
        salida,
        esquema,
    })
}

/// **La consulta con sus parámetros como literales**, para la consulta sin parámetros. Las comillas se doblan; los `$n` dentro de un
/// identificador o de un literal no se tocan.
pub fn en_linea(c: &ore_sql::Consulta) -> String {
    let mut out = String::with_capacity(c.texto.len());
    let mut cs = c.texto.chars().peekable();
    let mut dentro: Option<char> = None;
    while let Some(ch) = cs.next() {
        match dentro {
            Some(q) => {
                out.push(ch);
                if ch == q {
                    dentro = None;
                }
            }
            None if ch == '"' || ch == '\'' => {
                dentro = Some(ch);
                out.push(ch);
            }
            None if ch == '$' && cs.peek().is_some_and(|d| d.is_ascii_digit()) => {
                let mut n = String::new();
                while let Some(d) = cs.peek().filter(|d| d.is_ascii_digit()) {
                    n.push(*d);
                    cs.next();
                }
                match n
                    .parse::<usize>()
                    .ok()
                    .and_then(|k| c.parametros.get(k.wrapping_sub(1)))
                {
                    Some(v) => {
                        out.push('\'');
                        out.push_str(&v.replace('\'', "''"));
                        out.push('\'');
                    }
                    None => {
                        out.push('$');
                        out.push_str(&n);
                    }
                }
            }
            None => out.push(ch),
        }
    }
    out
}

/// La consulta que se manda: sólo lo que sale, en el orden de `salida`, cada columna por su texto.
fn sql_de(plan: &Plan) -> String {
    format!(
        "SELECT {} FROM ({}) AS ore_q",
        plan.salida
            .iter()
            .map(|(_, _, f, col)| f.en_la_consulta(col))
            .collect::<Vec<_>>()
            .join(", "),
        en_linea(&plan.consulta)
    )
}

/// Los bytes de un valor de texto, sin mirarlos: una consulta que pide `::STRING` los recibe en UTF-8.
struct Crudo<'a>(&'a [u8]);

impl<'a> postgres::types::FromSql<'a> for Crudo<'a> {
    fn from_sql(
        _: &postgres::types::Type,
        raw: &'a [u8],
    ) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(Crudo(raw))
    }

    fn accepts(_: &postgres::types::Type) -> bool {
        true
    }
}

/// **Leer**: la petición, en Arrow y en flujo, por `salida`. Devuelve las filas.
pub fn leer(
    s: &mut Sesion,
    token: &Token,
    p: &Peticion,
    salida: &mut dyn Write,
) -> Result<u64, Fallo> {
    let plan = planear(&mut s.cliente, p)?;
    if let Ok(mut en_vuelo) = token.en_vuelo.lock() {
        *en_vuelo = Some((s.cliente.cancel_token(), s.tls.clone()));
    }
    let mut emisor = Emisor::nuevo(&mut *salida, &plan.esquema)?;
    let mut avisos: BTreeMap<(String, Aviso), u64> = BTreeMap::new();
    let hecho = en_transaccion(&mut s.cliente, |c| {
        let mut filas = c
            .query_typed_raw(
                &sql_de(&plan),
                std::iter::empty::<(&(dyn postgres::types::ToSql + Sync), postgres::types::Type)>(),
            )
            .map_err(|e| fallo_de(&e))?;
        por_lotes(&mut filas, &plan, &mut avisos, |l| emisor.lote(&l))
    });
    if let Ok(mut en_vuelo) = token.en_vuelo.lock() {
        *en_vuelo = None;
    }
    let n = hecho?;
    emisor.fin()?;
    for ((columna, aviso), k) in &avisos {
        eprintln!("ore-read-cockroach: aviso · `{columna}`: {k} {aviso}");
    }
    Ok(n)
}

/// Abre una transacción de **solo lectura**, hace lo que se pide y la cierra. Si algo falla, `ROLLBACK`: la sesión queda como estaba y
/// puede volver a usarse (`servir`).
fn en_transaccion<T>(
    c: &mut postgres::Client,
    hacer: impl FnOnce(&mut postgres::Client) -> Result<T, Fallo>,
) -> Result<T, Fallo> {
    c.batch_execute("BEGIN READ ONLY")
        .map_err(|e| fallo_de(&e))?;
    // El texto canónico: instantes en UTC (la sesión de CockroachDB ya lo es; se fija para no depender del servidor).
    let hecho = c
        .batch_execute("SET LOCAL TIME ZONE 'UTC'")
        .map_err(|e| fallo_de(&e))
        .and_then(|()| hacer(c));
    match hecho {
        Ok(t) => {
            c.batch_execute("COMMIT").map_err(|e| fallo_de(&e))?;
            Ok(t)
        }
        Err(f) => {
            let _ = c.batch_execute("ROLLBACK");
            Err(f)
        }
    }
}

/// **Las filas, de [`LOTE`] en [`LOTE`]**, en Arrow, a `dar`. Devuelve las filas.
fn por_lotes(
    filas: &mut postgres::RowIter<'_>,
    plan: &Plan,
    avisos: &mut BTreeMap<(String, Aviso), u64>,
    mut dar: impl FnMut(RecordBatch) -> Result<(), Fallo>,
) -> Result<u64, Fallo> {
    use postgres::fallible_iterator::FallibleIterator;
    let mut n: u64 = 0;
    let columnas: Vec<Columna> = plan
        .salida
        .iter()
        .map(|(prop, _, f, _)| Columna::de_forma(prop, f.ore(), ""))
        .collect();
    let mut lote = RecordBatchBuilder::con_capacidad(&columnas, LOTE)
        .map_err(|e| Fallo::origen(e.to_string()))?;
    loop {
        let mut k: usize = 0;
        let mut fin = false;
        while k < LOTE {
            let Some(fila) = filas.next().map_err(|e| fallo_de(&e))? else {
                fin = true;
                break;
            };
            for (i, (prop, _, f, columna)) in plan.salida.iter().enumerate() {
                let v = fila
                    .try_get::<_, Option<Crudo>>(i)
                    .map_err(|e| {
                        Fallo::origen(format!(
                            "la columna `{columna}` no se pudo leer: {}",
                            crate::tls::causa(&e)
                        ))
                    })?
                    .map(|c| c.0);
                let mut w = lote.col(i);
                let puesto = match v.map(std::str::from_utf8).transpose() {
                    Ok(t) => poner(&mut w, f, t),
                    Err(e) => Err(format!("no es texto: {e}")),
                };
                if let Some(a) = puesto.map_err(|e| {
                    Fallo::origen(format!(
                        "`{prop}` (la columna `{columna}`), fila {}: {e}",
                        n + k as u64 + 1
                    ))
                })? {
                    *avisos.entry((columna.clone(), a)).or_default() += 1;
                }
            }
            lote.fila_escrita();
            k += 1;
        }
        if k > 0 {
            let arrays = lote
                .finish()
                .map_err(|e| Fallo::origen(e.to_string()))?
                .columns()
                .to_vec();
            dar(RecordBatch::try_new(plan.esquema.clone(), arrays)
                .map_err(|e| Fallo::origen(format!("el lote no casa: {e}")))?)?;
            n += k as u64;
        }
        if fin {
            return Ok(n);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn consulta(texto: &str, parametros: &[&str]) -> ore_sql::Consulta {
        ore_sql::Consulta {
            texto: texto.into(),
            parametros: parametros.iter().map(|s| s.to_string()).collect(),
            columnas_de_parametros: Vec::new(),
            columnas: Vec::new(),
        }
    }

    #[test]
    fn el_objeto_se_cita_parte_a_parte() {
        assert_eq!(citado("public.tipos"), "\"public\".\"tipos\"");
        assert_eq!(citado("a\"b.c"), "\"a\"\"b\".\"c\"");
    }

    #[test]
    fn los_parametros_pasan_a_literales_con_la_comilla_doblada() {
        let c = consulta("SELECT 1 WHERE a = $1 AND b = $2", &["x", "o'brien"]);
        assert_eq!(en_linea(&c), "SELECT 1 WHERE a = 'x' AND b = 'o''brien'");
    }

    #[test]
    fn un_dolar_dentro_de_un_literal_o_un_identificador_no_se_toca() {
        let c = consulta("SELECT \"a$1\" FROM t WHERE b = '$1' AND c = $1", &["z"]);
        assert_eq!(
            en_linea(&c),
            "SELECT \"a$1\" FROM t WHERE b = '$1' AND c = 'z'"
        );
    }

    #[test]
    fn un_parametro_que_no_esta_se_deja() {
        assert_eq!(en_linea(&consulta("SELECT $3", &["a"])), "SELECT $3");
    }
}
