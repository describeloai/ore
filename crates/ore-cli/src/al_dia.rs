//! **Una rama con lo que no tocó al día** (0044 C.2 ③), para quien trabaja en
//! un clon: el Job de la copia en una rama (0044 C.2 ④). Es lo que `ore-serve`
//! hace con sus árboles (`Forja::superponer`), con la misma regla
//! ([`ore_core::punteros::manda`]) y sobre el clon que el Job ya tiene.
//!
//! ```text
//! ore overlay . --main origin/main   los punteros que la rama no tocó, los de main de hoy
//! ore overlay . --undo               antes de confirmar: lo no tocado, como estaba
//! ```
//!
//! Lo superpuesto es **invisible para git** —lo que la rama tiene va con
//! `--skip-worktree`, lo que no tiene al `info/exclude` del clon—, así que un
//! `git add -A` no lo confirma; `--undo` deja visible lo que la escritura
//! cambió (un puntero que partió del de `main` y ahora es otro: pasa a ser de
//! la rama) y devuelve lo demás a como está en la rama. La lista, con la huella
//! de cada puntero escrito, queda en `.ore-al-dia.json`: `--undo` no necesita
//! nada más.
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::{Command, ExitCode};

use ore_core::json::Json;
use ore_core::punteros::{CARPETAS_AL_DIA, Manda, manda};

/// El mismo nombre que en `ore-serve` (`git::AL_DIA`).
const AL_DIA: &str = ".ore-al-dia.json";
/// Desde aquí, lo que se añadió al `info/exclude` del clon.
const MARCA: &str = "# ore · al día (0044 C.2 ③): no tocar";

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let o = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .map_err(|e| format!("no se pudo lanzar git: {e}"))?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).into_owned())
    } else {
        Err(format!(
            "`git {}`: {}",
            args.join(" "),
            String::from_utf8_lossy(&o.stderr).trim()
        ))
    }
}

/// Ruta → huella del blob, de los punteros de `commit`.
fn blobs(dir: &Path, commit: &str) -> Result<BTreeMap<String, String>, String> {
    let mut args = vec!["ls-tree", "-r", commit, "--"];
    args.extend(CARPETAS_AL_DIA);
    Ok(git(dir, &args)?
        .lines()
        .filter_map(|l| {
            let (izq, ruta) = l.split_once('\t')?;
            let h = izq.split_whitespace().nth(2)?;
            ruta.ends_with(".json")
                .then(|| (ruta.to_string(), h.to_string()))
        })
        .collect())
}

fn en_head(dir: &Path, ruta: &str) -> bool {
    git(dir, &["cat-file", "-e", &format!("HEAD:{ruta}")]).is_ok()
}

fn fallo(m: String) -> ExitCode {
    eprintln!("error: {m}");
    ExitCode::from(1)
}

/// `ore overlay <dir> --main <ref>`.
pub fn superponer(dir: &Path, main: &str) -> ExitCode {
    match superponer_(dir, main) {
        Ok(n) => {
            println!("al día con `{main}`: {n} puntero(s) de main que la rama no tocó");
            ExitCode::SUCCESS
        }
        Err(m) => fallo(m),
    }
}

fn superponer_(dir: &Path, main: &str) -> Result<usize, String> {
    let c = git(dir, &["rev-parse", "HEAD"])?.trim().to_string();
    let m = git(dir, &["rev-parse", "--verify", main])?
        .trim()
        .to_string();
    if c == m {
        return Ok(0);
    }
    let base = git(dir, &["merge-base", &m, &c])
        .map(|s| s.trim().to_string())
        .ok();
    let (en_rama, en_main) = (blobs(dir, &c)?, blobs(dir, &m)?);
    let en_base = match &base {
        Some(b) => blobs(dir, b)?,
        None => BTreeMap::new(),
    };
    let mut rutas: BTreeSet<&String> = en_rama.keys().collect();
    rutas.extend(en_main.keys());
    rutas.extend(en_base.keys());
    let (mut de, mut huellas) = (BTreeMap::new(), BTreeMap::new());
    let mut excluir = vec![format!("/{AL_DIA}")];
    for ruta in rutas {
        let r = en_rama.get(ruta).map(String::as_str);
        let f = dir.join(ruta);
        let hecho = match manda(
            r,
            en_base.get(ruta).map(String::as_str),
            en_main.get(ruta).map(String::as_str),
        ) {
            Manda::Rama => None,
            Manda::Main(h) => {
                if Some(h) != r {
                    let texto = git(dir, &["cat-file", "blob", h])?;
                    if let Some(p) = f.parent() {
                        let _ = std::fs::create_dir_all(p);
                    }
                    std::fs::write(&f, texto)
                        .map_err(|e| format!("no se pudo escribir `{ruta}`: {e}"))?;
                }
                huellas.insert(ruta.clone(), Json::s(h));
                Some("main")
            }
            Manda::Ninguno if r.is_some() => {
                let _ = std::fs::remove_file(&f);
                Some("ninguno")
            }
            Manda::Ninguno => None,
        };
        if let Some(d) = hecho {
            de.insert(ruta.clone(), Json::s(d));
            if r.is_some() {
                git(dir, &["update-index", "--skip-worktree", "--", ruta])?;
            } else {
                excluir.push(format!("/{ruta}"));
            }
        }
    }
    let n = de.len();
    let j = Json::obj([
        ("main", Json::s(&m)),
        ("base", Json::s(base.unwrap_or_default())),
        ("rutas", Json::Obj(de)),
        ("huellas", Json::Obj(huellas)),
    ]);
    std::fs::write(dir.join(AL_DIA), j.pretty() + "\n")
        .map_err(|e| format!("no se pudo escribir `{AL_DIA}`: {e}"))?;
    let ex = dir.join(git(dir, &["rev-parse", "--git-path", "info/exclude"])?.trim());
    let mut t = std::fs::read_to_string(&ex).unwrap_or_default();
    if !t.is_empty() && !t.ends_with('\n') {
        t.push('\n');
    }
    t.push_str(MARCA);
    t.push('\n');
    for l in excluir {
        t.push_str(&l);
        t.push('\n');
    }
    if let Some(p) = ex.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    std::fs::write(&ex, t).map_err(|e| format!("no se pudo escribir `info/exclude`: {e}"))?;
    Ok(n)
}

/// `ore overlay <dir> --undo`.
pub fn deshacer(dir: &Path) -> ExitCode {
    match deshacer_(dir) {
        Ok((suyos, vuelven)) => {
            println!(
                "sin superponer: {vuelven} puntero(s) como estaban en la rama; {suyos} cambiado(s) por esta pasada, ya de la rama"
            );
            ExitCode::SUCCESS
        }
        Err(m) => fallo(m),
    }
}

fn deshacer_(dir: &Path) -> Result<(usize, usize), String> {
    let Ok(t) = std::fs::read_to_string(dir.join(AL_DIA)) else {
        return Ok((0, 0));
    };
    let n = ore_core::parse::parse(&t).map_err(|e| format!("`{AL_DIA}` no analiza: {e:?}"))?;
    let huella = |ruta: &str| {
        n.get("huellas")
            .and_then(|(_, h)| h.get(ruta))
            .and_then(|(_, v)| v.as_str())
            .map(String::from)
    };
    let rutas: Vec<(String, String)> = n
        .get("rutas")
        .map(|(_, r)| r.entries())
        .unwrap_or(&[])
        .iter()
        .filter_map(|(k, v)| Some((k.as_str()?.to_string(), v.as_str()?.to_string())))
        .collect();
    let (mut suyos, mut vuelven) = (0, 0);
    for (ruta, de) in &rutas {
        let f = dir.join(ruta);
        let intacto = match (de.as_str(), huella(ruta)) {
            ("main", Some(h)) => {
                git(dir, &["hash-object", "--", ruta]).is_ok_and(|s| s.trim() == h)
            }
            _ => !f.exists(),
        };
        if en_head(dir, ruta) {
            git(dir, &["update-index", "--no-skip-worktree", "--", ruta])?;
            if intacto {
                git(dir, &["checkout", "-q", "HEAD", "--", ruta])?;
            }
        } else if intacto {
            let _ = std::fs::remove_file(&f);
        }
        if intacto {
            vuelven += 1;
        } else {
            suyos += 1;
        }
    }
    let _ = std::fs::remove_file(dir.join(AL_DIA));
    let ex = dir.join(git(dir, &["rev-parse", "--git-path", "info/exclude"])?.trim());
    if let Ok(t) = std::fs::read_to_string(&ex)
        && let Some((antes, _)) = t.split_once(MARCA)
    {
        std::fs::write(&ex, antes)
            .map_err(|e| format!("no se pudo escribir `info/exclude`: {e}"))?;
    }
    Ok((suyos, vuelven))
}
