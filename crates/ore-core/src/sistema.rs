//! **Las tablas del sistema** (docs/ejecuciones.md): `system.default.executions`.
//!
//! No son documentos del árbol: las escribe `ore-serve` y sus punteros viven en
//! el lago. Pero se leen y se ven como cualquier dataset —desde Query, en el
//! árbol de catálogo—, así que el núcleo sabe sus nombres, sus columnas y el
//! `Package` y el `Dataset` con que el índice las enseña ([`documentos`]).

use std::path::Path;

use crate::document::Kind;
use crate::link::Loaded;

/// La base de las tablas del sistema ([`crate::pertenencia::BASES_RESERVADAS`]).
pub const BASE: &str = "system";

/// `system.default.executions`, en su forma corta (la clave del árbol).
pub const EJECUCIONES: &str = "system.executions";

/// Las columnas de `system.default.executions`, en el vocabulario de OOS: las
/// escribe `ore-serve` (`ejecuciones.rs`) y las declara el `Dataset` del índice.
/// Planas: el lago escribe filas de escalares, y las listas van como JSON.
pub const COLUMNAS_DE_EJECUCIONES: &[(&str, &str, &str)] = &[
    ("id", "String", "the execution: its cell, build or job"),
    ("kind", "String", "query, cell, build or job"),
    ("subject", "String", "who ran it (user:<handle>)"),
    ("organization", "String", ""),
    (
        "source_kind",
        "String",
        "where it ran from: query, sesion, notebook, build",
    ),
    (
        "source_id",
        "String",
        "the sheet, .sql, repository or Transform",
    ),
    ("source_commit", "String", ""),
    ("source_branch", "String", ""),
    ("statement", "String", "what ran, up to 64 KiB"),
    (
        "fingerprint",
        "String",
        "the normalized statement's hash: groups repeats",
    ),
    ("queued_at", "DateTimeTz", ""),
    ("started_at", "DateTimeTz", ""),
    ("ended_at", "DateTimeTz", ""),
    ("wait_ms", "Integer", "time queued"),
    ("duration_ms", "Integer", "time running"),
    ("status", "String", "ok or failed"),
    ("error_name", "String", ""),
    ("error_message", "String", ""),
    ("compute", "String", "L or G"),
    ("worker", "String", "the session that ran it"),
    (
        "cpu_ms",
        "Integer",
        "CPU time, of the session and its engine",
    ),
    (
        "peak_memory_mb",
        "Integer",
        "the session's highest resident memory",
    ),
    ("spill_mb", "Integer", "what the engine spilled to disk"),
    ("read", "String", "JSON: [{name, live}]"),
    (
        "steps",
        "String",
        "JSON: [{statement, status, duration_ms}]",
    ),
    // Elastic Sessions S2b: lo que la sesión creció mientras corría.
    (
        "growth",
        "String",
        "JSON: [{at_ms, from_mb, to_mb, cpu_m, reason}]: how the session grew while it ran",
    ),
    (
        "memory_final_mb",
        "Integer",
        "the session's memory limit when it ended",
    ),
    (
        "growth_refused",
        "String",
        "why it could not grow: infeasible, deferred or quota",
    ),
];

/// El catálogo de DuckDB con que el SDK registra `system`: `system` es un
/// catálogo propio de DuckDB, de solo lectura, y en él no se crea un schema
/// (medido, DuckDB 1.5.4: «Cannot create schema in system catalog»).
pub const CATALOGO_EN_DUCKDB: &str = "__ore_system";

/// **La sentencia, con `system` como DuckDB lo puede leer**: cada
/// `system.default.executions` (o `system.executions`) que el texto nombra pasa a
/// `__ore_system.…`. Por el tokenizador, no por texto: lo que dice un
/// comentario o una cadena no se toca, y cada trozo se copia del texto por su
/// sitio, sin reimprimirlo. `None` si no nombra ninguna.
pub fn en_duckdb(texto: &str) -> Option<String> {
    use sqlparser::dialect::DuckDbDialect;
    use sqlparser::tokenizer::{Token, Tokenizer};
    let con_sitio = Tokenizer::new(&DuckDbDialect {}, texto)
        .tokenize_with_location()
        .ok()?;
    let lineas: Vec<usize> = std::iter::once(0)
        .chain(texto.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    let byte = |l: u64, c: u64| -> usize {
        let ini = lineas
            .get((l as usize).saturating_sub(1))
            .copied()
            .unwrap_or(texto.len());
        texto[ini..]
            .char_indices()
            .nth((c as usize).saturating_sub(1))
            .map(|(i, _)| ini + i)
            .unwrap_or(texto.len())
    };
    let sitios: Vec<(usize, usize)> = con_sitio
        .iter()
        .map(|t| {
            (
                byte(t.span.start.line, t.span.start.column),
                byte(t.span.end.line, t.span.end.column),
            )
        })
        .collect();
    let toks: Vec<(usize, &Token)> = con_sitio
        .iter()
        .enumerate()
        .filter(|(_, t)| !matches!(t.token, Token::Whitespace(_)))
        .map(|(i, t)| (i, &t.token))
        .collect();
    let palabra = |k: usize| match toks.get(k) {
        Some((_, Token::Word(w))) => Some(w.value.to_ascii_lowercase()),
        _ => None,
    };
    let punto = |k: usize| matches!(toks.get(k), Some((_, Token::Period)));
    let mut cambiar: Vec<usize> = Vec::new();
    for (k, (i, _)) in toks.iter().enumerate() {
        if palabra(k).as_deref() != Some(BASE) || (k > 0 && punto(k - 1)) {
            continue;
        }
        let largo = match (punto(k + 1), palabra(k + 2), punto(k + 3), palabra(k + 4)) {
            (true, Some(s), true, Some(n)) if s == crate::normalize::SCHEMA_POR_DEFECTO => {
                Some((5, format!("{BASE}.{n}")))
            }
            (true, Some(n), false, _) => Some((3, format!("{BASE}.{n}"))),
            _ => None,
        };
        if let Some((l, qn)) = largo
            && !punto(k + l)
            && es_tabla_del_sistema(&qn)
        {
            cambiar.push(*i);
        }
    }
    if cambiar.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(texto.len() + 8 * cambiar.len());
    let mut desde = 0;
    for i in cambiar {
        let (ini, fin) = sitios[i];
        out.push_str(&texto[desde..ini]);
        // con comillas, con comillas: `"system"` → `"__ore_system"`
        let dicho = &texto[ini..fin];
        out.push_str(&dicho.replacen(&dicho.trim_matches('"').to_string(), CATALOGO_EN_DUCKDB, 1));
        desde = fin;
    }
    out.push_str(&texto[desde..]);
    Some(out)
}

/// Si `qn` (en su forma corta, o con `default`) es una tabla del sistema.
pub fn es_tabla_del_sistema(qn: &str) -> bool {
    crate::normalize::a_corto(&qn.to_ascii_lowercase()) == EJECUCIONES
}

/// **Lo que el índice enseña de `system`**: su `Package` y el `Dataset`
/// `executions`, con sus columnas, como documentos de `<raiz>/packages/system/`
/// que no están en el disco. Sólo para proyectar: no se compilan ni se
/// escriben, y un árbol que ya tuviera un paquete `system` (de antes de
/// reservarlo) se queda con el suyo.
pub fn documentos(raiz: &Path) -> Vec<Loaded> {
    let carpeta = raiz.join("packages").join(BASE);
    let paquete = crate::paquetes::documento(BASE, "team:system", "stable", BASE);
    let columnas: String = COLUMNAS_DE_EJECUCIONES
        .iter()
        .map(|(c, t, d)| {
            if d.is_empty() {
                format!("    {c}: {{ type: {t} }}\n")
            } else {
                format!("    {c}: {{ type: {t}, description: \"{d}\" }}\n")
            }
        })
        .collect();
    let dataset = format!(
        "apiVersion: oos.dev/v1alpha12\n\
         kind: Dataset\n\
         metadata:\n  name: executions\n  namespace: {BASE}\n  \
         description: \"One row per execution: what ran, who ran it, from where, how long it took and how it ended. You see your own; an admin sees the organization's.\"\n\
         spec:\n  owner: team:system\n  changes: {{ mode: append }}\n  columns:\n{columnas}"
    );
    [
        (carpeta.join("package.yaml"), Kind::Package, paquete),
        (carpeta.join("executions.yaml"), Kind::Dataset, dataset),
    ]
    .into_iter()
    .filter_map(|(path, kind, texto)| {
        Some(Loaded {
            path,
            kind,
            root: crate::parse::parse(&texto).ok()?,
        })
    })
    .collect()
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn executions_se_reconoce_de_cualquier_forma() {
        assert!(es_tabla_del_sistema("system.executions"));
        assert!(es_tabla_del_sistema("system.default.executions"));
        assert!(es_tabla_del_sistema("System.Default.Executions"));
        assert!(!es_tabla_del_sistema("system.otra"));
        assert!(!es_tabla_del_sistema("ventas.executions"));
    }

    #[test]
    fn duckdb_lee_system_por_su_catalogo() {
        assert_eq!(
            en_duckdb("select kind, count(*) from system.default.executions group by 1").as_deref(),
            Some("select kind, count(*) from __ore_system.default.executions group by 1")
        );
        assert_eq!(
            en_duckdb("select * from System.executions e join \"system\".\"default\".\"executions\" f on true")
                .as_deref(),
            Some("select * from __ore_system.executions e join \"__ore_system\".\"default\".\"executions\" f on true")
        );
        // un comentario, una cadena, otra tabla de system o una columna: igual
        assert_eq!(
            en_duckdb(
                "select 'system.executions' -- system.default.executions\nfrom ventas.pedidos"
            ),
            None
        );
        assert_eq!(en_duckdb("select * from system.otra"), None);
        assert_eq!(en_duckdb("select x.system.executions from t"), None);
    }

    #[test]
    fn el_indice_ve_una_base_con_su_dataset() {
        let docs = documentos(Path::new("/arbol"));
        assert_eq!(docs.len(), 2, "los dos documentos analizan");
        assert_eq!(docs[0].kind, Kind::Package);
        let d = &docs[1];
        assert_eq!(d.qname().as_deref(), Some(EJECUCIONES));
        let cols = d.section("columns").unwrap();
        assert_eq!(cols.entries().len(), COLUMNAS_DE_EJECUCIONES.len());
        assert_eq!(
            cols.get("queued_at")
                .and_then(|(_, c)| c.get("type"))
                .and_then(|(_, t)| t.as_str()),
            Some("DateTimeTz")
        );
    }
}
