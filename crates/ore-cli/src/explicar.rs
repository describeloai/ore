//! **`ore explain`**: qué va al origen y qué queda en el motor (ADR 0053 F5·2).
//!
//! ```text
//! ore explain "SELECT …" [--file q.sql] [--policy DIR] [--from-workspace] [--json] [--path .]
//! ```
//!
//! Es el reparto de `ore_core::reparto` contado: por cada `Table` de un origen
//! que la sentencia lee, las columnas, los filtros y el `limit` que se le piden,
//! lo que evalúa DuckDB, el coste y los avisos. No abre nada: lee el árbol, y la
//! política de `main` si se da (`--policy`, como `ore federate`).
//!
//! Con `--estimate` (Query Q2) también **lo que lee del lago**: por cada dataset
//! del lago que nombra, su esquema vigente (del `metadata.json`), las columnas y
//! los filtros que usa, y lo que eso lee según los manifiestos —filas, ficheros y
//! bytes—, que es con lo que se elige el motor. Eso sí abre algo: el almacén.
//!
//! En texto, un no sale como `error[CÓDIGO]` y el proceso falla; con `--json`,
//! una línea `{"ok": false, "http", "codigo", "tabla", "mensaje"}` y sale 0 (el
//! que llama decide con la línea, como con `federate`).

use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;

use ore_core::json::Json;
use ore_core::reparto::{self, Lectura, Opciones, Reparto};

pub struct Pedido<'a> {
    pub raiz: &'a Path,
    pub sql: String,
    pub politica: Option<&'a Path>,
    pub desde_puesto: bool,
    pub json: bool,
    /// La suite de conformidad: la spec no tiene el interruptor de la fuente.
    pub conformidad: bool,
    /// Query Q2: estimar lo que se lee del lago (abre el almacén).
    pub estimar: bool,
}

pub fn explicar(p: &Pedido) -> ExitCode {
    let (mut pkg, _) = ore_core::validate::cargar_paquete(p.raiz);
    if let Some(dir) = p.politica
        && let Err((http, codigo, mensaje)) = crate::federar::politica_de_main(&mut pkg, dir)
    {
        return no(p.json, http, &codigo, None, &mensaje);
    }
    // 0057 B4·1: lo que sabe cada conector, si hay pasarela.
    let conectores = crate::lector::conectores_de_la_pasarela();
    // Q2: los datasets del lago que nombra, con su esquema vigente.
    let del_lago = if p.estimar {
        tablas_del_lago(p.raiz, &p.sql)
    } else {
        Vec::new()
    };
    let esquemas: BTreeMap<String, Vec<String>> = del_lago
        .iter()
        .filter_map(|(c, _, _, cols)| cols.as_ref().ok().map(|v| (c.clone(), v.clone())))
        .collect();
    let o = Opciones {
        desde_puesto: p.desde_puesto,
        exigir_interruptor: !p.conformidad,
        conectores: conectores.as_ref(),
        copia: false,
        del_lago: p.estimar.then_some(&esquemas),
    };
    match reparto::repartir(&p.sql, &pkg, &o) {
        Ok(r) if p.json => {
            let mut j = r.json();
            if p.estimar
                && let Json::Obj(m) = &mut j
            {
                m.insert("estimacion".into(), estimacion(&r, &del_lago));
            }
            println!("{}", j.jcs());
            ExitCode::SUCCESS
        }
        Ok(r) => {
            print!("{}", texto(&r));
            if p.estimar {
                print!("{}", texto_de_la_estimacion(&estimacion(&r, &del_lago)));
            }
            ExitCode::SUCCESS
        }
        Err(n) => no(p.json, n.http, &n.codigo, n.tabla.as_deref(), &n.mensaje),
    }
}

/// `(corto, dataset, metadata_location, columnas)` de cada nombre de la sentencia
/// que es un dataset del lago con tabla Iceberg. Las columnas pueden fallar (el
/// almacén no contesta): entonces ese dataset no se estima, y se dice.
type DelLago = (String, String, String, Result<Vec<String>, (u8, String)>);

fn tablas_del_lago(raiz: &Path, sql: &str) -> Vec<DelLago> {
    let mut vistos = Vec::new();
    let mut out = Vec::new();
    for n in nombres_con_punto(sql) {
        if let Some((corto, dataset, ml)) = crate::datasets::tabla_del_lago(raiz, &n)
            && !vistos.contains(&corto)
        {
            vistos.push(corto.clone());
            let cols = crate::datasets::columnas_del_lago(&ml);
            out.push((corto, dataset, ml, cols));
        }
    }
    out
}

/// Los `a.b` y `a.b.c` de la sentencia, fuera de literales y comentarios: los
/// candidatos a dataset del lago (el puntero decide cuál lo es).
pub(crate) fn nombres_con_punto(sql: &str) -> Vec<String> {
    fn cerrar(actual: &mut String, out: &mut Vec<String>) {
        let partes = actual.split('.').filter(|p| !p.is_empty()).count();
        let letra = actual
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_');
        if letra && (2..=3).contains(&partes) && !actual.ends_with('.') {
            let n = actual.to_lowercase();
            if !out.contains(&n) {
                out.push(n);
            }
        }
        actual.clear();
    }
    let mut out = Vec::new();
    let (mut actual, mut en_texto, mut en_comentario) = (String::new(), false, false);
    let b: Vec<char> = sql.chars().collect();
    for (i, &c) in b.iter().enumerate() {
        if en_comentario {
            en_comentario = c != '\n';
        } else if en_texto {
            en_texto = c != '\'';
        } else if c == '-' && b.get(i + 1) == Some(&'-') {
            cerrar(&mut actual, &mut out);
            en_comentario = true;
        } else if c == '\'' {
            cerrar(&mut actual, &mut out);
            en_texto = true;
        } else if c.is_alphanumeric() || c == '_' || c == '.' {
            actual.push(c);
        } else {
            cerrar(&mut actual, &mut out);
        }
    }
    cerrar(&mut actual, &mut out);
    out
}

/// Por cada dataset del lago que lee: lo que el almacén estima, y el total.
fn estimacion(r: &Reparto, del_lago: &[DelLago]) -> Json {
    let (mut filas, mut bytes) = (0i64, 0i64);
    let mut tablas = Vec::new();
    for l in &r.del_lago {
        let Some((_, dataset, ml, cols)) = del_lago.iter().find(|(c, ..)| *c == l.tabla) else {
            continue;
        };
        let filtros: Vec<Json> = l.filtros.iter().map(reparto::Filtro::json).collect();
        let mut m = vec![
            ("tabla", Json::s(l.tabla.as_str())),
            (
                "columnas",
                Json::Arr(l.columnas.iter().map(Json::s).collect()),
            ),
            ("filtros", Json::Arr(filtros.clone())),
        ];
        let e = match cols {
            Err((_, e)) => Err(e.clone()),
            Ok(_) => crate::datasets::estimar_del_lago(dataset, ml, &l.columnas, filtros)
                .map_err(|(_, e)| e),
        };
        match e {
            Ok(n) => {
                let num = |k: &str| {
                    n.get(k)
                        .and_then(|(_, v)| v.as_str())
                        .and_then(|v| v.parse::<i64>().ok())
                        .unwrap_or(0)
                };
                filas += num("filas");
                bytes += num("bytes");
                for k in [
                    "filas",
                    "filas_total",
                    "ficheros",
                    "ficheros_total",
                    "bytes",
                    "bytes_total",
                ] {
                    m.push((k, Json::Int(num(k))));
                }
                for k in ["sin_tamano", "filtros_ignorados"] {
                    m.push((
                        k,
                        n.get(k)
                            .map(|(_, v)| Json::de_node(v))
                            .unwrap_or(Json::Arr(Vec::new())),
                    ));
                }
            }
            Err(e) => m.push(("error", Json::s(e))),
        }
        tablas.push(Json::obj(m));
    }
    Json::obj([
        ("tablas", Json::Arr(tablas)),
        ("filas", Json::Int(filas)),
        ("bytes", Json::Int(bytes)),
    ])
}

fn texto_de_la_estimacion(e: &Json) -> String {
    let Json::Obj(m) = e else {
        return String::new();
    };
    let mut s = String::new();
    let tablas = match m.get("tablas") {
        Some(Json::Arr(v)) => v.clone(),
        _ => Vec::new(),
    };
    for t in tablas {
        let Json::Obj(t) = t else { continue };
        let txt = |k: &str| match t.get(k) {
            Some(Json::Str(v)) => v.clone(),
            Some(Json::Int(v)) => v.to_string(),
            _ => String::new(),
        };
        s.push_str(&format!("{}  (del lago)\n", txt("tabla")));
        if t.contains_key("error") {
            s.push_str(&format!("  sin estimar  {}\n", txt("error")));
            continue;
        }
        s.push_str(&format!(
            "  lee          {} de {} filas · {} de {} ficheros · {} de {} bytes\n",
            txt("filas"),
            txt("filas_total"),
            txt("ficheros"),
            txt("ficheros_total"),
            txt("bytes"),
            txt("bytes_total")
        ));
    }
    let n = |k: &str| match m.get(k) {
        Some(Json::Int(v)) => *v,
        _ => 0,
    };
    if n("bytes") > 0 || n("filas") > 0 {
        s.push_str(&format!(
            "del lago, en total: {} filas · {:.3} GB\n",
            n("filas"),
            n("bytes") as f64 / 1e9
        ));
    }
    s
}

fn no(json: bool, http: u16, codigo: &str, tabla: Option<&str>, mensaje: &str) -> ExitCode {
    if json {
        let mut m = vec![
            ("ok", Json::Bool(false)),
            ("http", Json::Int(i64::from(http))),
            ("codigo", Json::s(codigo)),
            ("mensaje", Json::s(mensaje)),
        ];
        if let Some(t) = tabla {
            m.push(("tabla", Json::s(t)));
        }
        println!("{}", Json::obj(m).jcs());
        return ExitCode::SUCCESS;
    }
    eprintln!("error[{codigo}]: {mensaje}");
    ExitCode::FAILURE
}

/// El reparto, para leerlo.
pub fn texto(r: &Reparto) -> String {
    let mut s = String::new();
    if r.lecturas.is_empty() {
        s.push_str("no lee ningún origen: todo lo que lee es del lago (o de fuera del árbol)\n");
    }
    for l in &r.lecturas {
        s.push_str(&lectura(l));
    }
    for a in &r.avisos {
        s.push_str(&format!("⚠ {a}\n"));
    }
    s
}

fn lectura(l: &Lectura) -> String {
    let mut s = format!(
        "{}  ({} · fuente {} · {})\n",
        l.tabla, l.tipo, l.fuente, l.objeto
    );
    s.push_str(&format!(
        "  al origen    columnas  {}\n",
        l.columnas.join(", ")
    ));
    let filtros: Vec<String> = l.empujados.iter().map(reparto::describir).collect();
    s.push_str(&format!(
        "               filtros   {}\n",
        if filtros.is_empty() {
            "ninguno".to_string()
        } else {
            filtros.join(" AND ")
        }
    ));
    if let Some(n) = l.limit {
        let orden: Vec<String> = l
            .orden
            .iter()
            .map(|(c, d)| format!("{c}{}", if *d { " DESC" } else { "" }))
            .collect();
        s.push_str(&format!(
            "               limit     {n}{}\n",
            if orden.is_empty() {
                String::new()
            } else {
                format!(" (ORDER BY {})", orden.join(", "))
            }
        ));
    }
    if !l.en_el_motor.is_empty() {
        s.push_str(&format!(
            "  en DuckDB    {}\n",
            l.en_el_motor.join("\n               ")
        ));
    }
    s.push_str(&format!(
        "  coste        fullScan: {}{}\n",
        l.full_scan,
        if l.presupuesto {
            " · se lee con presupuesto"
        } else {
            ""
        }
    ));
    for a in &l.avisos {
        s.push_str(&format!("  ⚠ {a}\n"));
    }
    s
}

#[cfg(test)]
mod pruebas_q2 {
    use super::nombres_con_punto;

    /// Query Q2 · los candidatos a dataset del lago: dos o tres partes, fuera de
    /// literales y comentarios, sin repetir.
    #[test]
    fn los_nombres_con_punto_de_una_sentencia() {
        let sql = "SELECT t.a, geo.trip.b FROM geo.trip t JOIN Ventas.Default.Pedidos p ON t.x = p.y \
                   WHERE s = 'no.es.un.nombre' -- ni.esto\n AND f(geo.trip.c) > 1.5";
        assert_eq!(
            nombres_con_punto(sql),
            vec![
                "t.a",
                "geo.trip.b",
                "geo.trip",
                "ventas.default.pedidos",
                "t.x",
                "p.y",
                "geo.trip.c"
            ]
        );
    }
}
