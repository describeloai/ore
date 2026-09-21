//! **`ore assets`** — el índice de assets de un árbol (0034 ⑤), desde el CLI:
//! para mirarlo sin ore-serve y para medirlo (`--json`). El índice lo proyecta
//! ore-core (`assets::indice`); aquí sólo se leen los punteros de `datasets/`
//! y se imprime un resumen, o el JSON entero.
use ore_core::json::Json;
use std::collections::BTreeMap;
use std::path::Path;

pub struct Opciones {
    pub json: bool,
}

/// Los punteros de `datasets/`, por su nombre de fichero sin extensión.
pub(crate) fn punteros_de(raiz: &Path) -> BTreeMap<String, Json> {
    let mut out = BTreeMap::new();
    let Ok(entradas) = std::fs::read_dir(raiz.join("datasets")) else {
        return out;
    };
    for e in entradas.flatten() {
        let p = e.path();
        if p.extension().is_none_or(|x| x != "json") {
            continue;
        }
        let Some(nombre) = p.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
            continue;
        };
        if let Ok(texto) = std::fs::read_to_string(&p)
            && let Ok(n) = ore_core::parse::parse(&texto)
        {
            out.insert(nombre, Json::de_node(&n));
        }
    }
    out
}

pub fn assets(path: &Path, op: &Opciones) -> std::process::ExitCode {
    if !path.is_dir() {
        eprintln!("error: `{}` no es un directorio de paquete", path.display());
        return std::process::ExitCode::from(66);
    }
    let (pkg, _) = ore_core::validate::cargar_paquete(path);
    let punteros = punteros_de(path);
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
    let mut inducidas = 0usize;
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
        if let Some(Json::Obj(d)) = it.get("detalle")
            && d.contains_key("vistaInducida")
        {
            inducidas += 1;
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
        "{relaciones} relaciones (en las dos direcciones) · rotas {rotas} · identidad {identidad} · tablas con vista inducida {inducidas}"
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
    std::process::ExitCode::SUCCESS
}
