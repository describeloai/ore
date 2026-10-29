//! **git, como proceso** (como `ore-serve`): el espejo desnudo del árbol del inquilino y lo que se
//! le pregunta. Solo los punteros: los `.json` bajo [`CARPETAS_AL_DIA`].

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use ore_core::punteros::crudo::CARPETAS_AL_DIA;

/// Un puntero que cambió: su ruta y su blob nuevo (`None`, borrado).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cambio {
    pub ruta: String,
    pub blob: Option<String>,
}

/// Un commit de primer padre de `main`, con los punteros que movió respecto a su primer padre.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub sha: String,
    pub autor: String,
    pub cuando: String,
    pub cambios: Vec<Cambio>,
}

/// ¿Es la ruta de un puntero?
pub fn es_puntero(ruta: &str) -> bool {
    ruta.ends_with(".json")
        && CARPETAS_AL_DIA
            .iter()
            .any(|c| ruta.strip_prefix(c).is_some_and(|r| r.starts_with('/')))
}

/// Una línea de `--raw` (`:100644 100644 <viejo> <nuevo> M\truta`) → el cambio, si es un puntero.
fn de_raw(linea: &str) -> Option<Cambio> {
    let (meta, ruta) = linea.strip_prefix(':')?.split_once('\t')?;
    if !es_puntero(ruta) {
        return None;
    }
    let campos: Vec<&str> = meta.split_whitespace().collect();
    let [_, _, _, nuevo, estado] = campos.as_slice() else {
        return None;
    };
    Some(Cambio {
        ruta: ruta.to_string(),
        blob: (!estado.starts_with('D')).then(|| nuevo.to_string()),
    })
}

/// La salida de `log --format=%x01%H%x00%an <%ae>%x00%cI --raw`, commit a commit.
fn de_log(texto: &str) -> Vec<Commit> {
    texto
        .split('\u{1}')
        .filter(|t| !t.trim().is_empty())
        .filter_map(|trozo| {
            let mut lineas = trozo.lines();
            let mut cabecera = lineas.next()?.split('\0');
            let (sha, autor, cuando) = (cabecera.next()?, cabecera.next()?, cabecera.next()?);
            Some(Commit {
                sha: sha.to_string(),
                autor: autor.to_string(),
                cuando: cuando.to_string(),
                cambios: lineas.filter_map(de_raw).collect(),
            })
        })
        .collect()
}

/// La salida de `cat-file --batch`: `<sha> <tipo> <tamaño>\n<contenido>\n`, en el orden pedido.
fn de_lote(salida: &[u8], pedidos: &[String]) -> Result<HashMap<String, String>, String> {
    let mut out = HashMap::new();
    let mut pos = 0;
    for blob in pedidos {
        let fin = salida[pos..]
            .iter()
            .position(|b| *b == b'\n')
            .ok_or("cat-file: salida cortada")?;
        let cabecera = String::from_utf8_lossy(&salida[pos..pos + fin]).into_owned();
        pos += fin + 1;
        let tam: usize = match cabecera.split(' ').collect::<Vec<_>>().as_slice() {
            [_, _, t] => t.parse().map_err(|_| format!("cat-file: `{cabecera}`"))?,
            _ => return Err(format!("cat-file: `{cabecera}` (pedido {blob})")),
        };
        let cuerpo = salida
            .get(pos..pos + tam)
            .ok_or("cat-file: contenido cortado")?;
        out.insert(
            blob.clone(),
            String::from_utf8(cuerpo.to_vec())
                .map_err(|_| format!("el blob {blob} no es UTF-8"))?,
        );
        pos += tam + 1;
    }
    Ok(out)
}

/// El espejo desnudo y la forja de la que se trae.
pub struct Espejo {
    dir: PathBuf,
    origen: String,
    cabecera: Option<String>,
}

impl Espejo {
    /// Abre el espejo en `dir`, creándolo vacío si no existe. `origen` es la URL de la forja o una
    /// ruta (el banco); `testigo`, el token de la forja.
    pub fn abrir(dir: &Path, origen: &str, testigo: Option<&str>) -> Result<Espejo, String> {
        let e = Espejo {
            dir: dir.to_path_buf(),
            origen: origen.to_string(),
            cabecera: testigo.map(|t| format!("Authorization: token {}", t.trim())),
        };
        if !dir.join("HEAD").exists() {
            std::fs::create_dir_all(dir).map_err(|x| format!("{}: {x}", dir.display()))?;
            e.git(&["init", "--quiet", "--bare"])?;
        }
        Ok(e)
    }

    fn orden(&self) -> Command {
        let mut c = Command::new("git");
        c.arg("--git-dir").arg(&self.dir);
        c.env("GIT_TERMINAL_PROMPT", "0");
        if let Some(h) = &self.cabecera {
            // Por el entorno y no por la línea de órdenes: el token no sale en `ps`.
            c.env("GIT_CONFIG_COUNT", "1")
                .env("GIT_CONFIG_KEY_0", "http.extraheader")
                .env("GIT_CONFIG_VALUE_0", h);
        }
        c
    }

    fn salida(&self, args: &[&str]) -> Result<std::process::Output, String> {
        self.orden()
            .args(args)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("git {}: {e}", args.join(" ")))
    }

    fn git(&self, args: &[&str]) -> Result<String, String> {
        let out = self.salida(args)?;
        if !out.status.success() {
            return Err(format!(
                "git {}: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        String::from_utf8(out.stdout).map_err(|e| format!("git {}: {e}", args.join(" ")))
    }

    /// Las ramas de la forja, sin traer nada (60–90 ms en victor).
    pub fn ramas_remotas(&self) -> Result<BTreeMap<String, String>, String> {
        Ok(de_refs(
            &self.git(&["ls-remote", "--heads", &self.origen])?,
            "refs/heads/",
        ))
    }

    /// Las ramas del espejo.
    pub fn ramas(&self) -> Result<BTreeMap<String, String>, String> {
        Ok(de_refs(
            &self.git(&[
                "for-each-ref",
                "--format=%(objectname)\t%(refname)",
                "refs/heads",
            ])?,
            "refs/heads/",
        ))
    }

    /// Trae la forja al espejo: las ramas tal cual, y las borradas fuera.
    pub fn traer(&self) -> Result<(), String> {
        self.git(&[
            "fetch",
            "--quiet",
            "--prune",
            &self.origen,
            "+refs/heads/*:refs/heads/*",
        ])
        .map(|_| ())
    }

    /// Los commits de primer padre de `hasta` después de `desde`, del más viejo al más nuevo, con
    /// los punteros que movió cada uno respecto a su primer padre (una sola llamada a git).
    pub fn primer_padre(&self, desde: Option<&str>, hasta: &str) -> Result<Vec<Commit>, String> {
        let rango = desde.map_or(hasta.to_string(), |d| format!("{d}..{hasta}"));
        if let Some(d) = desde
            && !self.es_antecesor(d, hasta)?
        {
            return Err(format!(
                "main se reescribió: {d} ya no es antecesor de {hasta}"
            ));
        }
        Ok(de_log(&self.git(&[
            "log",
            "--first-parent",
            "--reverse",
            "--diff-merges=first-parent",
            "--raw",
            "--no-renames",
            "--no-abbrev",
            "--format=%x01%H%x00%an <%ae>%x00%cI",
            &rango,
        ])?))
    }

    /// Los shas de primer padre de `punta`, del más viejo al más nuevo.
    pub fn shas_primer_padre(&self, punta: &str) -> Result<Vec<String>, String> {
        Ok(self
            .git(&["rev-list", "--first-parent", "--reverse", punta])?
            .lines()
            .map(str::to_string)
            .collect())
    }

    /// Los punteros que cambian de `de` a `a`.
    pub fn diferencias(&self, de: &str, a: &str) -> Result<Vec<Cambio>, String> {
        let mut args = vec!["diff", "--raw", "--no-renames", "--no-abbrev", de, a, "--"];
        args.extend(CARPETAS_AL_DIA);
        Ok(self.git(&args)?.lines().filter_map(de_raw).collect())
    }

    /// Los punteros de un commit: ruta → blob.
    pub fn arbol(&self, commit: &str) -> Result<BTreeMap<String, String>, String> {
        let mut args = vec!["ls-tree", "-r", commit, "--"];
        args.extend(CARPETAS_AL_DIA);
        Ok(self
            .git(&args)?
            .lines()
            .filter_map(|l| {
                let (meta, ruta) = l.split_once('\t')?;
                let blob = meta.split_whitespace().nth(2)?;
                es_puntero(ruta).then(|| (ruta.to_string(), blob.to_string()))
            })
            .collect())
    }

    pub fn base_comun(&self, a: &str, b: &str) -> Result<String, String> {
        Ok(self.git(&["merge-base", a, b])?.trim().to_string())
    }

    pub fn es_antecesor(&self, a: &str, b: &str) -> Result<bool, String> {
        let out = self.salida(&["merge-base", "--is-ancestor", a, b])?;
        match out.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(format!(
                "git merge-base --is-ancestor: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )),
        }
    }

    /// El contenido de cada blob, en una sola llamada (`cat-file --batch`).
    pub fn contenidos(&self, blobs: &[String]) -> Result<HashMap<String, String>, String> {
        if blobs.is_empty() {
            return Ok(HashMap::new());
        }
        let mut hijo = self
            .orden()
            .args(["cat-file", "--batch"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("git cat-file: {e}"))?;
        let mut entrada = hijo.stdin.take().ok_or("git cat-file: sin stdin")?;
        let peticion: String = blobs.iter().map(|b| format!("{b}\n")).collect();
        // En otro hilo: con muchos blobs, escribir todo antes de leer se atasca en la tubería.
        let escritor = std::thread::spawn(move || entrada.write_all(peticion.as_bytes()));
        let mut salida = Vec::new();
        hijo.stdout
            .take()
            .ok_or("git cat-file: sin stdout")?
            .read_to_end(&mut salida)
            .map_err(|e| format!("git cat-file: {e}"))?;
        escritor
            .join()
            .map_err(|_| "git cat-file: el escritor murió")?
            .map_err(|e| format!("git cat-file: {e}"))?;
        hijo.wait().map_err(|e| format!("git cat-file: {e}"))?;
        de_lote(&salida, blobs)
    }
}

/// `<sha>\t<ref>` → nombre (sin `prefijo`) → sha.
fn de_refs(texto: &str, prefijo: &str) -> BTreeMap<String, String> {
    texto
        .lines()
        .filter_map(|l| {
            let (sha, r) = l.split_once('\t')?;
            Some((r.strip_prefix(prefijo)?.to_string(), sha.trim().to_string()))
        })
        .collect()
}

#[cfg(test)]
mod pruebas {
    use super::*;

    const A: &str = "1111111111111111111111111111111111111111";
    const B: &str = "2222222222222222222222222222222222222222";
    const CERO: &str = "0000000000000000000000000000000000000000";

    #[test]
    fn solo_los_punteros() {
        assert!(es_puntero("datasets/ventas/default/pedidos.json"));
        assert!(es_puntero("copias/a.json"));
        assert!(!es_puntero("datasets/ventas/default/pedidos.yaml"));
        assert!(!es_puntero("packages/a.json"));
        assert!(!es_puntero("datasets_viejos/a.json"));
        assert!(!es_puntero("resultados/a.json"));
    }

    #[test]
    fn una_linea_raw() {
        let m = format!(":100644 100644 {A} {B} M\tdatasets/a/default/t.json");
        assert_eq!(
            de_raw(&m),
            Some(Cambio {
                ruta: "datasets/a/default/t.json".into(),
                blob: Some(B.into())
            })
        );
        let d = format!(":100644 000000 {A} {CERO} D\tcopias/x.json");
        assert_eq!(
            de_raw(&d),
            Some(Cambio {
                ruta: "copias/x.json".into(),
                blob: None
            })
        );
        let fuera = format!(":000000 100644 {CERO} {B} A\tpackages/a.yaml");
        assert_eq!(de_raw(&fuera), None);
    }

    #[test]
    fn un_log_con_y_sin_cambios() {
        let texto = format!(
            "\u{1}{A}\0Ana <a@x>\x002026-10-28T10:00:00+00:00\n\n:000000 100644 {CERO} {B} A\tdatasets/a/default/t.json\n:000000 100644 {CERO} {B} A\tpackages/t.yaml\n\u{1}{B}\0Bea <b@x>\x002026-10-28T11:00:00+00:00\n"
        );
        let cs = de_log(&texto);
        assert_eq!(cs.len(), 2);
        assert_eq!(cs[0].sha, A);
        assert_eq!(cs[0].autor, "Ana <a@x>");
        assert_eq!(cs[0].cambios.len(), 1);
        assert!(cs[1].cambios.is_empty());
    }

    #[test]
    fn un_lote_de_cat_file() {
        let salida = format!("{A} blob 3\nabc\n{B} blob 2\n{{}}\n");
        let m = de_lote(salida.as_bytes(), &[A.to_string(), B.to_string()]).unwrap();
        assert_eq!(m[A], "abc");
        assert_eq!(m[B], "{}");
        let falta = format!("{A} missing\n");
        assert!(de_lote(falta.as_bytes(), &[A.to_string()]).is_err());
    }
}
