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
//! En texto, un no sale como `error[CÓDIGO]` y el proceso falla; con `--json`,
//! una línea `{"ok": false, "http", "codigo", "tabla", "mensaje"}` y sale 0 (el
//! que llama decide con la línea, como con `federate`).

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
}

pub fn explicar(p: &Pedido) -> ExitCode {
    let (mut pkg, _) = ore_core::validate::cargar_paquete(p.raiz);
    if let Some(dir) = p.politica
        && let Err((http, codigo, mensaje)) = crate::federar::politica_de_main(&mut pkg, dir)
    {
        return no(p.json, http, &codigo, None, &mensaje);
    }
    let o = Opciones {
        desde_puesto: p.desde_puesto,
        exigir_interruptor: !p.conformidad,
        conectores: None,
    };
    match reparto::repartir(&p.sql, &pkg, &o) {
        Ok(r) if p.json => {
            println!("{}", r.json().jcs());
            ExitCode::SUCCESS
        }
        Ok(r) => {
            print!("{}", texto(&r));
            ExitCode::SUCCESS
        }
        Err(n) => no(p.json, n.http, &n.codigo, n.tabla.as_deref(), &n.mensaje),
    }
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
