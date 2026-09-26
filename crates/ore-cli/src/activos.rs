//! **`ore assets`** — el índice de assets de un árbol (0034 ⑤), desde el CLI:
//! para mirarlo sin ore-serve y para medirlo (`--json`). El índice lo proyecta
//! ore-core (`assets::indice`); aquí sólo se leen los punteros de `datasets/`
//! y se imprime un resumen, o el JSON entero. Desde 0035 ① el resumen acaba
//! con los proyectos: cuántos hay, qué nombra cada uno y cuánto queda **fuera**
//! de todos — que es lo normal, porque un proyecto es una lente y no una caja.
//! Y desde 0035 ⑥, con los **repositorios**: dónde se trabaja, con su clase.
use ore_core::json::Json;
use std::collections::BTreeMap;
use std::path::Path;

pub struct Opciones {
    pub json: bool,
}

pub fn assets(path: &Path, op: &Opciones) -> std::process::ExitCode {
    if !path.is_dir() {
        eprintln!("error: `{}` no es un directorio de paquete", path.display());
        return std::process::ExitCode::from(66);
    }
    let (pkg, _) = ore_core::validate::cargar_paquete(path);
    let punteros = ore_core::punteros::del_arbol(path);
    let indice = ore_core::assets::indice(&pkg, &punteros, &ore_core::assets::Cabeza::default());
    if op.json {
        println!("{}", indice.jcs());
        return std::process::ExitCode::SUCCESS;
    }
    let Json::Obj(m) = &indice else {
        return std::process::ExitCode::SUCCESS;
    };
    let items = match m.get("items") {
        Some(Json::Obj(i)) => i,
        _ => return std::process::ExitCode::SUCCESS,
    };
    // Un resumen: ítems por kind, relaciones, paquetes.
    let mut por_kind: BTreeMap<String, usize> = BTreeMap::new();
    let mut relaciones = 0usize;
    let mut rotas = 0usize;
    let mut identidad = 0usize;
    for it in items.values() {
        let Json::Obj(it) = it else { continue };
        if let Some(Json::Str(k)) = it.get("kind") {
            *por_kind.entry(k.clone()).or_default() += 1;
        }
        if let Some(Json::Arr(r)) = it.get("relaciones") {
            relaciones += r.len();
            rotas += r
                .iter()
                .filter(|x| matches!(x, Json::Obj(o) if o.get("rota") == Some(&Json::Bool(true))))
                .count();
        }
        if let Some(Json::Obj(d)) = it.get("define")
            && d.get("identidad") == Some(&Json::Bool(true))
        {
            identidad += 1;
        }
    }
    println!(
        "{} ítems · {}",
        items.len(),
        por_kind
            .iter()
            .map(|(k, n)| format!("{k}={n}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "{relaciones} relaciones (en las dos direcciones) · rotas {rotas} · identidad {identidad}"
    );
    if let Some(Json::Arr(ps)) = m.get("paquetes") {
        for p in ps {
            let Json::Obj(p) = p else { continue };
            let s = |k: &str| match p.get(k) {
                Some(Json::Str(v)) => v.clone(),
                Some(Json::Int(v)) => v.to_string(),
                Some(Json::Bool(v)) => v.to_string(),
                _ => "-".into(),
            };
            let carpetas = match p.get("carpetas") {
                Some(Json::Arr(c)) => c
                    .iter()
                    .map(|x| match x {
                        Json::Str(s) if s.is_empty() => "\"\"".to_string(),
                        Json::Str(s) => s.clone(),
                        _ => "?".into(),
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
                _ => String::new(),
            };
            println!(
                "  {:<28} {:<9} {:>3} ítems · carpetas: {carpetas}{}",
                s("name"),
                s("type"),
                s("items"),
                if p.contains_key("source") {
                    format!(" · de {}", s("source"))
                } else {
                    String::new()
                }
            );
        }
    }
    // Los proyectos (0035 ①): la lente, y cuánto queda fuera de todas.
    if let Some(Json::Arr(ps)) = m.get("proyectos")
        && !ps.is_empty()
    {
        let fuera = items
            .values()
            .filter(|it| matches!(it, Json::Obj(o) if matches!(o.get("proyectos"), Some(Json::Arr(a)) if a.is_empty())))
            .count();
        println!(
            "{} proyectos · {} ítems · {fuera} fuera",
            ps.len(),
            items.len()
        );
        for p in ps {
            let Json::Obj(p) = p else { continue };
            let s = |k: &str| match p.get(k) {
                Some(Json::Str(v)) => v.clone(),
                Some(Json::Int(v)) => v.to_string(),
                _ => "-".into(),
            };
            let contiene = match p.get("contiene") {
                Some(Json::Arr(c)) if !c.is_empty() => c
                    .iter()
                    .filter_map(|x| match x {
                        Json::Str(s) => Some(s.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
                _ => "nada todavía".into(),
            };
            match p.get("roto") {
                Some(Json::Str(r)) => println!("  {:<28} ROTO: {r}", s("nombre")),
                _ => println!("  {:<28} {:>3} ítems · {contiene}", s("nombre"), s("items")),
            }
        }
    }
    // Los repositorios (0035 ⑥): dónde se trabaja, con su clase.
    if let Some(Json::Arr(rs)) = m.get("repositorios")
        && !rs.is_empty()
    {
        println!("{} repositorios", rs.len());
        for r in rs {
            let Json::Obj(r) = r else { continue };
            let s = |k: &str| match r.get(k) {
                Some(Json::Str(v)) => v.clone(),
                Some(Json::Int(v)) => v.to_string(),
                _ => "-".into(),
            };
            match r.get("roto") {
                Some(Json::Str(x)) => println!("  {:<40} ROTO: {x}", s("ruta")),
                _ => println!(
                    "  {:<40} {:<12} {:>3} ítems · {}",
                    s("ruta"),
                    s("plantilla"),
                    s("items"),
                    s("nombre")
                ),
            }
        }
    }
    std::process::ExitCode::SUCCESS
}
