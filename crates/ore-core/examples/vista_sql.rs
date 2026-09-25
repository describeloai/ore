//! `ore_core::vista_sql` por stdin, para las medidas de ADR 0040
//! (`medida-el-linaje-de-la-vista-sql.py`).
//!
//! Entrada: bloques separados por una línea `-- @@ <id>`.
//! Salida: una línea por bloque, `<id>\t<json>`.

use std::collections::BTreeSet;
use std::io::Read;

use ore_core::vista_sql::{Clase, Ref, analizar};

fn js(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn refs(r: &BTreeSet<Ref>) -> String {
    let v: Vec<String> = r
        .iter()
        .map(|r| format!("[{},{}]", js(&r.fuente), js(&r.columna)))
        .collect();
    format!("[{}]", v.join(","))
}

fn lista(v: impl IntoIterator<Item = String>) -> String {
    format!(
        "[{}]",
        v.into_iter().map(|x| js(&x)).collect::<Vec<_>>().join(",")
    )
}

fn uno(texto: &str) -> String {
    let c = match analizar(texto, &|_| None) {
        Ok(c) => c,
        Err(e) => return format!("{{\"error\":{}}}", js(&e.como_texto())),
    };
    let cols: Vec<String> = c
        .columnas
        .iter()
        .map(|x| {
            format!(
                "[{},{},{},{}]",
                js(&x.nombre),
                refs(&x.directas),
                refs(&x.derivadas),
                refs(&x.indirectas)
            )
        })
        .collect();
    let preds: Vec<String> = c
        .predicados
        .iter()
        .map(|p| {
            format!(
                "[{},{}]",
                js(if p.clase == Clase::Ordena {
                    "ordena"
                } else {
                    "revela"
                }),
                refs(&p.mira)
            )
        })
        .collect();
    let estrellas: Vec<String> = c
        .estrellas_sin_expandir
        .iter()
        .map(|n| format!("[{},[[{},\"*\"]],[],[]]", js("*"), js(n)))
        .collect();
    format!(
        "{{\"lee\":{},\"ind\":{},\"cols\":[{}],\"predicados\":[{}]}}",
        lista(c.lee),
        refs(&c.indirectas),
        cols.into_iter()
            .chain(estrellas)
            .collect::<Vec<_>>()
            .join(","),
        preds.join(",")
    )
}

fn main() {
    let mut entrada = String::new();
    std::io::stdin().read_to_string(&mut entrada).unwrap();
    let mut id: Option<String> = None;
    let mut buf = String::new();
    let volcar = |id: &Option<String>, buf: &str| {
        if let Some(i) = id {
            println!("{i}\t{}", uno(buf));
        }
    };
    for linea in entrada.lines() {
        if let Some(r) = linea.strip_prefix("-- @@ ") {
            volcar(&id, &buf);
            id = Some(r.trim().to_string());
            buf.clear();
        } else {
            buf.push_str(linea);
            buf.push('\n');
        }
    }
    volcar(&id, &buf);
}
