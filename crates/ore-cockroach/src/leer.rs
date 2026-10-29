//! **Leer**: la petición, en Arrow y en flujo, por `salida` (ADR 0080, pasos 2 y 3).
//!
//! ⚠️ **Adaptado de `ore-read-postgres/src/lectura.rs`** (`columnas`, `planear`, `en_linea`, `sql_de`, `por_lotes`, `en_tramos`), con otro
//! reparto: Postgres parte la tabla por páginas físicas (`ctid`) y exporta una instantánea; CockroachDB la parte por **rangos de la primera
//! columna entera de la clave primaria** y todas las conexiones leen en un mismo **`AS OF SYSTEM TIME`**, que es lo que hace que lo leído
//! sea lo de un instante (ADR 0080 §1.3, §3.1 B5).
//!
//! La consulta es `SELECT col::STRING, … FROM (<consulta de ore-sql>) AS ore_q` en una transacción de **solo lectura**; cada valor llega
//! como el texto que escribe CockroachDB y se lee con `ore_base::tipos`, igual que en Postgres.
//!
//! **Cuándo se reparte** ([`reparto`]): hay más de una conexión (`tramos`, 4 por defecto), no hay `limit`, orden ni claves (una copia, donde
//! el orden no importa), la clave primaria empieza por una columna entera —se salta la oculta de una clave *hash-sharded*— y entre su mínimo y
//! su máximo caben al menos [`CLAVES_PARA_REPARTIR`] valores. Si no, una sola conexión y una transacción: ya es una foto de un instante.
//!
//! **El desequilibrio:** los rangos son de ancho igual sobre `[mínimo, máximo]`. Una clave muy desigual dejaría a una conexión con casi todo,
//! así que se cortan [`TROZOS_POR_TRAMO`] trozos por conexión y cada una va tomando el siguiente que queda.

use crate::lectura::{Sesion, Token, fallo_de};
use crate::tipos::{Aviso, Forma, poner};
use crate::tls::{self, Abierta};
use arrow_array::RecordBatch;
use arrow_schema::{Field, Schema};
use ore_arrow::{Columna, Emisor, RecordBatchBuilder};
use ore_driver::{Codigo, Fallo, Filtro, Peticion, Valor};
use std::collections::BTreeMap;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Las filas que van en cada lote de Arrow.
pub const LOTE: usize = 8192;

/// Desde cuántos valores de clave (máximo − mínimo + 1) se reparte: por debajo, abrir conexiones cuesta más viajes de los que ahorra.
pub const CLAVES_PARA_REPARTIR: i128 = 200_000;

/// Cuántos trozos se cortan por cada conexión (ver arriba, «El desequilibrio»).
pub const TROZOS_POR_TRAMO: usize = 4;

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

/// La petición, ya con su plan: lo que sale, en qué orden y con qué esquema de Arrow.
struct Plan {
    consulta: ore_sql::Consulta,
    /// Por propiedad: su posición en el resultado, su forma y su columna.
    salida: Vec<(String, usize, Forma, String)>,
    esquema: Arc<Schema>,
}

fn preparar(p: &Peticion) -> Result<ore_sql::Consulta, Fallo> {
    ore_sql::preparar(p, &ore_sql::dialectos::POSTGRES, &p.objeto, || {
        unreachable!("`POSTGRES` no exige tipos")
    })
    .map_err(Fallo::operador)
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
    let consulta = preparar(p)?;
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
fn sql_de(plan: &Plan, consulta: &ore_sql::Consulta) -> String {
    format!(
        "SELECT {} FROM ({}) AS ore_q",
        plan.salida
            .iter()
            .map(|(_, _, f, col)| f.en_la_consulta(col))
            .collect::<Vec<_>>()
            .join(", "),
        en_linea(consulta)
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

// ── El reparto ───────────────────────────────────────────────────────────────

/// Por dónde se parte una tabla: la columna de clave y los extremos que tiene (ya fijados en un instante).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reparto {
    pub columna: String,
    pub minimo: i128,
    pub maximo: i128,
}

/// **Los trozos** de `[mínimo, máximo]`, de ancho igual: `(desde, hasta)` con `desde` incluido y `hasta` excluido. El primero no tiene
/// cota de abajo y el último no la tiene de arriba, así que juntos cubren toda la clave aunque el mínimo y el máximo se hayan quedado cortos.
pub fn trozos(r: &Reparto, k: usize) -> Vec<(Option<i128>, Option<i128>)> {
    let k = k.max(1);
    let span = (r.maximo - r.minimo + 1).max(1);
    let k = k.min(usize::try_from(span).unwrap_or(usize::MAX));
    let ancho = (span + k as i128 - 1) / k as i128;
    (0..k)
        .map(|i| {
            let desde = r.minimo + i as i128 * ancho;
            (
                (i > 0).then_some(desde),
                (i < k - 1).then_some(desde + ancho),
            )
        })
        .collect()
}

/// `cluster_logical_timestamp()`: el instante en que se lee, tal como lo acepta `AS OF SYSTEM TIME`.
fn instante(c: &mut postgres::Client) -> Result<String, Fallo> {
    c.query_one("SELECT cluster_logical_timestamp()::STRING", &[])
        .map(|f| f.get::<_, String>(0))
        .map_err(|e| fallo_de(&e))
}

/// ¿Se lee en tramos? Si sí, el instante y por dónde se parte. `None`: una sola conexión.
///
/// Mira (1) que la petición sea una copia, (2) la clave primaria de la tabla, saltando las columnas ocultas (la de *shard* de una clave
/// *hash-sharded*), (3) que su primera columna sea entera y (4) cuántos valores caben entre su mínimo y su máximo, **en el instante que
/// luego se lee**. Un objeto sin clave (una vista), de tres partes o con una clave no entera se lee entero con una conexión.
pub fn reparto(c: &mut postgres::Client, p: &Peticion) -> Result<Option<(String, Reparto)>, Fallo> {
    if p.limit.is_some() || !p.orden.is_empty() || !p.claves.is_empty() {
        return Ok(None);
    }
    let partes: Vec<&str> = p.objeto.split('.').collect();
    let (esquema, tabla): (Option<&str>, &str) = match partes[..] {
        [t] => (None, t),
        [e, t] => (Some(e), t),
        _ => return Ok(None),
    };
    let filas = c
        .query_typed(
            "SELECT k.column_name::text, c.data_type::text, c.is_hidden::text \
             FROM information_schema.key_column_usage k \
             JOIN information_schema.columns c ON c.table_schema = k.table_schema \
              AND c.table_name = k.table_name AND c.column_name = k.column_name \
             WHERE k.table_schema = COALESCE($1, current_schema()) AND k.table_name = $2 \
              AND k.constraint_name = (SELECT constraint_name FROM information_schema.table_constraints \
                   WHERE table_schema = COALESCE($1, current_schema()) AND table_name = $2 \
                    AND constraint_type = 'PRIMARY KEY') \
             ORDER BY k.ordinal_position",
            &[
                (
                    &esquema as &(dyn postgres::types::ToSql + Sync),
                    postgres::types::Type::TEXT,
                ),
                (&tabla, postgres::types::Type::TEXT),
            ],
        )
        .map_err(|e| fallo_de(&e))?;
    let primera = filas.iter().find_map(|f| {
        let (nombre, tipo, oculta): (String, String, String) = (f.get(0), f.get(1), f.get(2));
        (oculta != "YES").then_some((nombre, tipo))
    });
    let Some((columna, tipo)) = primera else {
        return Ok(None);
    };
    if !matches!(tipo.as_str(), "bigint" | "integer" | "smallint") {
        return Ok(None);
    }
    let ts = instante(c)?;
    let col = citado(&columna);
    let fila = c
        .query_one(
            &format!(
                "SELECT min({col})::STRING, max({col})::STRING FROM {} AS OF SYSTEM TIME '{ts}'",
                citado(&p.objeto)
            ),
            &[],
        )
        .map_err(|e| fallo_de(&e))?;
    let (minimo, maximo): (Option<String>, Option<String>) = (fila.get(0), fila.get(1));
    let (Some(minimo), Some(maximo)) = (minimo, maximo) else {
        return Ok(None); // una tabla vacía
    };
    let (Ok(minimo), Ok(maximo)) = (minimo.parse::<i128>(), maximo.parse::<i128>()) else {
        return Ok(None);
    };
    if maximo - minimo + 1 < CLAVES_PARA_REPARTIR {
        return Ok(None);
    }
    Ok(Some((
        ts,
        Reparto {
            columna,
            minimo,
            maximo,
        },
    )))
}

// ── Leer ─────────────────────────────────────────────────────────────────────

/// **Leer**: la petición, en Arrow y en flujo, por `salida`. Devuelve las filas.
pub fn leer(
    s: &mut Sesion,
    token: &Token,
    p: &Peticion,
    salida: &mut dyn Write,
) -> Result<u64, Fallo> {
    let plan = planear(&mut s.cliente, p)?;
    token.rearmar();
    token.registrar(s.cliente.cancel_token(), s.tls.clone());
    let mut emisor = Emisor::nuevo(&mut *salida, &plan.esquema)?;
    let mut avisos: BTreeMap<(String, Aviso), u64> = BTreeMap::new();
    let repartido = if s.tramos > 1 {
        reparto(&mut s.cliente, p).unwrap_or_else(|f| {
            // No poder decidir el reparto no es no poder leer: se lee con una conexión.
            eprintln!("ore-read-cockroach: aviso · no se pudo repartir la lectura ({}): se lee con una conexión", f.mensaje);
            None
        })
    } else {
        None
    };
    let hecho = match repartido {
        Some((ts, r)) => en_tramos(&plan, p, token, &ts, &r, s.tramos, &mut avisos, |l| {
            emisor.lote(l)
        }),
        None => en_transaccion(&mut s.cliente, "BEGIN READ ONLY", |c| {
            let mut filas = c
                .query_typed_raw(&sql_de(&plan, &plan.consulta), sin_parametros())
                .map_err(|e| fallo_de(&e))?;
            por_lotes(&mut filas, &plan, token, &mut avisos, |l| emisor.lote(&l))
        }),
    };
    token.terminar();
    let n = hecho?;
    emisor.fin()?;
    for ((columna, aviso), k) in &avisos {
        eprintln!("ore-read-cockroach: aviso · `{columna}`: {k} {aviso}");
    }
    Ok(n)
}

fn sin_parametros() -> std::iter::Empty<(
    &'static (dyn postgres::types::ToSql + Sync),
    postgres::types::Type,
)> {
    std::iter::empty()
}

/// Abre una transacción (`inicio`: de solo lectura, o `AS OF SYSTEM TIME`), hace lo que se pide y la cierra. Si algo falla, `ROLLBACK`: la
/// sesión queda como estaba y puede volver a usarse (`servir`).
fn en_transaccion<T>(
    c: &mut postgres::Client,
    inicio: &str,
    hacer: impl FnOnce(&mut postgres::Client) -> Result<T, Fallo>,
) -> Result<T, Fallo> {
    c.batch_execute(inicio).map_err(|e| fallo_de(&e))?;
    match hacer(c) {
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

/// **Leer en tramos** desde un mismo instante: cada conexión abre `BEGIN AS OF SYSTEM TIME <ts>` y va tomando el siguiente trozo de la
/// clave que queda. Lo leído es lo de un instante, como con una conexión. Los lotes llegan por un canal acotado y se escriben en el orden
/// en que llegan. Si un tramo falla, se cancelan los demás y el error es el suyo.
#[allow(clippy::too_many_arguments)]
fn en_tramos(
    plan: &Plan,
    p: &Peticion,
    token: &Token,
    ts: &str,
    r: &Reparto,
    conexiones: usize,
    avisos: &mut BTreeMap<(String, Aviso), u64>,
    mut escribir: impl FnMut(&RecordBatch) -> Result<(), Fallo>,
) -> Result<u64, Fallo> {
    let cota = |op: &str, v: i128| Filtro {
        columna: r.columna.clone(),
        operador: op.into(),
        valor: Valor::Uno(v.to_string()),
    };
    let consultas: Vec<String> = trozos(r, conexiones * TROZOS_POR_TRAMO)
        .into_iter()
        .map(|(desde, hasta)| {
            let mut q = p.clone();
            if let Some(d) = desde {
                q.filtros.push(cota("ge", d));
            }
            if let Some(h) = hasta {
                q.filtros.push(cota("lt", h));
            }
            preparar(&q).map(|c| sql_de(plan, &c))
        })
        .collect::<Result<_, _>>()?;
    let inicio = format!("BEGIN AS OF SYSTEM TIME '{ts}'");
    let siguiente = AtomicUsize::new(0);
    let (consultas, inicio, siguiente) = (&consultas, &inicio, &siguiente);
    let (dar, lotes) = std::sync::mpsc::sync_channel::<Result<RecordBatch, Fallo>>(2 * conexiones);
    std::thread::scope(|sc| {
        let hilos: Vec<_> = (0..conexiones.min(consultas.len()))
            .map(|_| {
                let dar = dar.clone();
                sc.spawn(move || {
                    let mut suyos = BTreeMap::new();
                    let leido = (|| -> Result<(), Fallo> {
                        let Abierta {
                            mut cliente, tls, ..
                        } = tls::abrir(&p.url)?;
                        token.registrar(cliente.cancel_token(), tls);
                        loop {
                            let i = siguiente.fetch_add(1, Ordering::SeqCst);
                            if i >= consultas.len() {
                                return Ok(());
                            }
                            en_transaccion(&mut cliente, inicio, |c| {
                                let mut filas = c
                                    .query_typed_raw(&consultas[i], sin_parametros())
                                    .map_err(|e| fallo_de(&e))?;
                                por_lotes(&mut filas, plan, token, &mut suyos, |l| {
                                    dar.send(Ok(l))
                                        .map_err(|_| Fallo::origen("la lectura ya terminó"))
                                })
                            })?;
                        }
                    })();
                    if let Err(f) = leido {
                        let _ = dar.send(Err(f));
                    }
                    suyos
                })
            })
            .collect();
        drop(dar);
        let mut n: u64 = 0;
        let mut fallo: Option<Fallo> = None;
        for lote in lotes {
            match lote.and_then(|l| {
                escribir(&l)?;
                Ok(l.num_rows() as u64)
            }) {
                Ok(k) => n += k,
                Err(f) => {
                    // El primero es la causa; los de después, los tramos que se cancelaron por él.
                    if fallo.is_none() {
                        fallo = Some(f);
                        token.parar();
                    }
                }
            }
        }
        for h in hilos {
            for (clave, k) in h.join().unwrap_or_default() {
                *avisos.entry(clave).or_default() += k;
            }
        }
        match fallo {
            Some(f) => Err(f),
            None => Ok(n),
        }
    })
}

/// **Las filas, de [`LOTE`] en [`LOTE`]**, en Arrow, a `dar`. Entre lote y lote mira la marca de cancelar. Devuelve las filas.
fn por_lotes(
    filas: &mut postgres::RowIter<'_>,
    plan: &Plan,
    token: &Token,
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
        if token.cancelada() {
            return Err(Fallo::origen(format!("cancelada con {n} filas leídas")));
        }
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
                            tls::causa(&e)
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

    fn r(minimo: i128, maximo: i128) -> Reparto {
        Reparto {
            columna: "id".into(),
            minimo,
            maximo,
        }
    }

    /// Cada clave de `[minimo, maximo]` cae en exactamente un trozo: sin huecos y sin solapes.
    fn cubre_exacto(r: &Reparto, k: usize) {
        let t = trozos(r, k);
        assert!(!t.is_empty());
        assert_eq!(
            t.first().unwrap().0,
            None,
            "el primero no tiene cota de abajo"
        );
        assert_eq!(
            t.last().unwrap().1,
            None,
            "el último no tiene cota de arriba"
        );
        for par in t.windows(2) {
            assert_eq!(
                par[0].1, par[1].0,
                "el hasta de uno es el desde del siguiente"
            );
        }
        let muestra = [
            r.minimo,
            r.minimo + 1,
            (r.minimo + r.maximo) / 2,
            r.maximo - 1,
            r.maximo,
            r.minimo - 5,
            r.maximo + 5,
        ];
        for clave in muestra {
            let dentro = t
                .iter()
                .filter(|(d, h)| d.is_none_or(|d| clave >= d) && h.is_none_or(|h| clave < h))
                .count();
            assert_eq!(
                dentro, 1,
                "la clave {clave} cae en {dentro} trozos de {t:?}"
            );
        }
    }

    #[test]
    fn los_trozos_cubren_la_clave_sin_huecos_ni_solapes() {
        cubre_exacto(&r(1, 10_000_000), 16);
        cubre_exacto(&r(1, 200_000), 16);
        cubre_exacto(&r(-5_000_000, 5_000_000), 16);
        cubre_exacto(&r(0, 15), 16);
        cubre_exacto(&r(7, 7), 16);
        cubre_exacto(&r(i64::MIN as i128, i64::MAX as i128), 16);
    }

    #[test]
    fn nunca_hay_mas_trozos_que_claves() {
        assert_eq!(trozos(&r(1, 3), 16).len(), 3);
        assert_eq!(trozos(&r(5, 5), 16), vec![(None, None)]);
        assert_eq!(
            trozos(&r(1, 100), 0),
            vec![(None, None)],
            "cero trozos es uno"
        );
    }

    #[test]
    fn los_trozos_son_de_ancho_igual() {
        let t = trozos(&r(1, 1600), 16);
        assert_eq!(t[0], (None, Some(101)));
        assert_eq!(t[1], (Some(101), Some(201)));
        assert_eq!(t[15], (Some(1501), None));
    }
}
