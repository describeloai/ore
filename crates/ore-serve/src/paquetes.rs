//! **El paquete de un árbol, analizado una vez por commit** (0081 §7.6, K1 en victor). `loadTable`, sus credenciales
//! y los listados de `/v1` cargaban el árbol entero en cada petición (`cargar_paquete`: 97 paquetes y 1.321 YAML en
//! victor, ~1 s de cada `loadTable` de ~1,4 s). Un árbol de lectura es un worktree del espejo en un commit fijo
//! (desmontado y rehecho si alguien lo toca), así que lo analizado sirve mientras el árbol esté en ese commit.
//!
//! - **La clave** es la ruta del árbol y su commit (el `HEAD` del worktree, leído del fichero: un commit separado). El
//!   paquete guarda su ruta, así que la misma ruta en otro commit, o el mismo commit en otra ruta, es otra entrada.
//! - **Sin commit, no se guarda:** un árbol que no es un worktree con `HEAD` separado (un directorio, una rama) se
//!   analiza en cada petición, como antes.
//! - **Acotado** a [`CUPO`] paquetes, el menos usado fuera.

use ore_core::link::Package;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Cuántos paquetes se guardan: los árboles de lectura vivos son pocos (`main` y las ramas que se leen).
pub(crate) const CUPO: usize = 4;

type Clave = (PathBuf, String);

static GUARDADOS: Mutex<Vec<(Clave, Arc<Package>)>> = Mutex::new(Vec::new());

/// **El paquete de `raiz`**: el ya analizado si el árbol sigue en el mismo commit; si no, lo analiza.
pub(crate) fn de(raiz: &Path) -> Arc<Package> {
    let Some(commit) = commit_de(raiz) else {
        return Arc::new(ore_core::validate::cargar_paquete(raiz).0);
    };
    let clave = (raiz.to_path_buf(), commit);
    {
        let mut g = GUARDADOS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(i) = g.iter().position(|(c, _)| *c == clave) {
            let par = g.remove(i);
            let p = par.1.clone();
            g.push(par);
            return p;
        }
    }
    // Fuera del candado: analizar tarda, y otra petición de otro árbol no espera por ésta.
    let p = Arc::new(ore_core::validate::cargar_paquete(raiz).0);
    let mut g = GUARDADOS.lock().unwrap_or_else(|e| e.into_inner());
    if !g.iter().any(|(c, _)| *c == clave) {
        g.push((clave, p.clone()));
        while g.len() > CUPO {
            g.remove(0);
        }
    }
    p
}

/// El commit separado en que está el worktree `raiz`: su `.git` (un fichero `gitdir:` o un directorio) y el `HEAD`
/// de ese gitdir, si es un commit y no una ref.
fn commit_de(raiz: &Path) -> Option<String> {
    let git = raiz.join(".git");
    let gitdir = if git.is_dir() {
        git
    } else {
        let t = std::fs::read_to_string(&git).ok()?;
        let d = PathBuf::from(t.trim().strip_prefix("gitdir:")?.trim());
        if d.is_absolute() { d } else { raiz.join(d) }
    };
    let head = std::fs::read_to_string(gitdir.join("HEAD")).ok()?;
    let head = head.trim();
    (head.len() == 40 && head.bytes().all(|b| b.is_ascii_hexdigit())).then(|| head.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arbol(nombre: &str, head: Option<&str>) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ore-paquetes-{}-{nombre}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        if let Some(h) = head {
            let gd = d.join("gitdir");
            std::fs::create_dir_all(&gd).unwrap();
            std::fs::write(gd.join("HEAD"), format!("{h}\n")).unwrap();
            std::fs::write(d.join(".git"), format!("gitdir: {}\n", gd.display())).unwrap();
        }
        d
    }

    const A: &str = "4e6a9546859aca6e7aff9ce2899f2698dda575b3";
    const B: &str = "ce14f9aa7c80dbff4fbb27f9f70b133279bed8cf";

    #[test]
    fn el_mismo_commit_no_se_vuelve_a_analizar_y_otro_si() {
        let d = arbol("commit", Some(A));
        let (p, q) = (de(&d), de(&d));
        assert!(Arc::ptr_eq(&p, &q), "el mismo árbol en el mismo commit");
        std::fs::write(d.join("gitdir/HEAD"), format!("{B}\n")).unwrap();
        assert!(
            !Arc::ptr_eq(&p, &de(&d)),
            "el árbol en otro commit se analiza de nuevo"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn sin_un_commit_separado_no_se_guarda() {
        let d = arbol("sin-git", None);
        assert!(!Arc::ptr_eq(&de(&d), &de(&d)));
        let r = arbol("una-ref", Some("x"));
        std::fs::write(r.join("gitdir/HEAD"), "ref: refs/heads/main\n").unwrap();
        assert_eq!(commit_de(&r), None);
        assert!(!Arc::ptr_eq(&de(&r), &de(&r)));
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(&r);
    }
}
