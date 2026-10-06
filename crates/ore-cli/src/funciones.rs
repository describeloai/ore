//! `ore functions generate` (ORE 0050 G1d): el `Function` de cada `@function`
//! del árbol, escrito desde el código. Y `ore transforms generate` (ORE 0055
//! T1·4), el `Transform` de cada `@transform` y de cada sentencia SQL que
//! escribe, en `<repositorio>/pipeline/`: el mismo plan que hace un commit.
//!
//! El plan es de `ore_core::generar`, que no escribe; esto lo enseña y, sin
//! `--check`, lo aplica. Hermético como `validate`: lee ficheros y escribe
//! ficheros, nada más.
//!
//! Sale con 0 si todo quedó al día; con 1 si algo no se pudo generar (un
//! `def` que no se deriva), o si con `--check` algo cambiaría —como `cargo fmt
//! --check`, para el CI—.

use ore_core::generar::{Accion, Plan};
use ore_core::json::Json;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

pub fn generar(
    raiz: &Path,
    comprobar: bool,
    json: bool,
    solo: &[PathBuf],
    transforms: bool,
) -> ExitCode {
    if !raiz.is_dir() {
        eprintln!("error: `{}` no es un árbol", raiz.display());
        return ExitCode::from(66); // EX_NOINPUT
    }
    let (pkg, _) = ore_core::validate::cargar_paquete(raiz);
    // `--solo` como el árbol los ve: desde la raíz, y aunque ya no existan
    // (un `.py` retirado se lleva su documento).
    let solo: BTreeSet<PathBuf> = solo
        .iter()
        .map(|p| {
            if p.is_absolute() {
                p.clone()
            } else {
                raiz.join(p)
            }
        })
        .collect();
    let solo = (!solo.is_empty()).then_some(&solo);
    let plan = if transforms {
        ore_core::generar::plan_de_transforms(&pkg, solo, None)
    } else {
        ore_core::generar::plan_de(&pkg, solo)
    };
    if !comprobar && let Err(e) = ore_core::generar::aplicar(&plan) {
        eprintln!("error: no se pudo escribir: {e}");
        return ExitCode::from(73); // EX_CANTCREAT
    }
    if json {
        println!("{}", a_json(&plan, raiz, comprobar).jcs());
    } else {
        contar(&plan, raiz, comprobar, transforms);
    }
    let pendiente = comprobar && !plan.cambios.is_empty();
    if plan.diagnosticos.is_empty() && !pendiente {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn relativa(p: &Path, raiz: &Path) -> String {
    p.strip_prefix(raiz)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

fn que(a: &Accion) -> &'static str {
    match a {
        Accion::Crear(_) => "crear",
        Accion::Reescribir { a_mano: false, .. } => "reescribir",
        Accion::Reescribir { a_mano: true, .. } => "reescribir-a-mano",
        Accion::Borrar => "borrar",
    }
}

fn contar(plan: &Plan, raiz: &Path, comprobar: bool, transforms: bool) {
    for c in &plan.cambios {
        let (signo, nota) = match &c.accion {
            Accion::Crear(_) => ("+", String::new()),
            Accion::Reescribir { a_mano: false, .. } => ("~", String::new()),
            Accion::Reescribir { a_mano: true, .. } => (
                "~",
                " · estaba escrito a mano: el código es la fuente".to_string(),
            ),
            Accion::Borrar => ("-", " · su `def` ya no es un `@function`".to_string()),
        };
        println!(
            "  {signo} {}  ({}){nota}",
            relativa(&c.ruta, raiz),
            c.entrypoint
        );
    }
    for d in &plan.diagnosticos {
        eprintln!("{}\n", d.render(raiz));
    }
    let n = plan.cambios.len();
    let resumen = match (comprobar, n) {
        (_, 0) => format!(
            "al día · {} {}",
            plan.al_dia,
            if transforms {
                "transforms"
            } else {
                "funciones"
            }
        ),
        (true, _) => {
            format!("{n} documentos no son los que el código da · `generate` los pone al día")
        }
        (false, _) => format!(
            "{n} documentos escritos · {} ya estaban al día",
            plan.al_dia
        ),
    };
    if plan.diagnosticos.is_empty() {
        println!("ok · {resumen}");
    } else {
        println!(
            "{resumen} · {} errores: lo que no se deriva no se escribe",
            plan.diagnosticos.len()
        );
    }
}

fn a_json(plan: &Plan, raiz: &Path, comprobar: bool) -> Json {
    Json::obj([
        ("comprobar", Json::Bool(comprobar)),
        ("alDia", Json::Int(plan.al_dia as i64)),
        (
            "cambios",
            Json::Arr(
                plan.cambios
                    .iter()
                    .map(|c| {
                        Json::obj([
                            ("ruta", Json::s(relativa(&c.ruta, raiz))),
                            ("entrypoint", Json::s(&c.entrypoint)),
                            ("accion", Json::s(que(&c.accion))),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "diagnosticos",
            Json::Arr(
                plan.diagnosticos
                    .iter()
                    .map(|d| {
                        let (linea, columna) = d.pos.map_or((0, 0), |p| (p.line, p.col));
                        Json::obj([
                            ("code", Json::s(d.code.as_str())),
                            ("fichero", Json::s(relativa(&d.file, raiz))),
                            ("linea", Json::Int(linea as i64)),
                            ("columna", Json::Int(columna as i64)),
                            ("mensaje", Json::s(&d.message)),
                            ("ayuda", Json::s(d.help.clone().unwrap_or_default())),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}
