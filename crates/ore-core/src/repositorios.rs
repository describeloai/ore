//! **El repositorio** (0035 ⑥, [`0036`]): la unidad de **trabajo**. Una carpeta
//! del árbol con **nombre** y **clase**.
//!
//! [`0036`]: https://github.com/describeloai/ore/blob/main/docs/decisions/0036-la-clase-del-repositorio.md
//!
//! ```text
//! packages/ventas/churn/README.md
//! ---
//! nombre: New Pipelines Java Transform
//! plantilla: transforms
//! plantillaVersion: 1
//! ---
//! Lo que este repositorio hace, en prosa.
//! ```
//!
//! # Qué lo distingue de una carpeta cualquiera
//!
//! **La clave `plantilla`, y sólo eso.** Un `README.md` sin ella es una carpeta
//! con README —las que `ore init` deja en cada directorio, por ejemplo—; con
//! ella, es un repositorio. No hay registro aparte y no hay `kind` nuevo: el
//! compilador sigue sin verlo (medido, 0035 ⑥ §1: `validate` 0 y el índice
//! igual en demo y victor).
//!
//! # Qué lo distingue de un proyecto
//!
//! Un proyecto es **una lente**: nombra lo suyo, se solapa con otras, y por eso
//! un ítem lleva `proyectos` en **plural**. Un repositorio es **el sitio donde
//! se trabaja**: un ítem está en **uno** —el más hondo que lo contiene— o en
//! ninguno. Anidar repositorios es **hondura, no solape**.
//!
//! Y de él se deriva todo lo acotado (0036): la capa de dependencias, la
//! sesión, la rama, las propuestas y el árbol que el editor abre. **La ruta es
//! la clave de partición**: lo acotado se dice «esto, para este prefijo».
use crate::manifiesto;
use std::path::Path;

/// Un repositorio, leído de su manifiesto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repositorio {
    /// `packages/<paquete>/<carpeta>`: la ruta de la carpeta, y **la clave de partición**.
    pub ruta: String,
    pub paquete: String,
    /// Lo que va del paquete a la carpeta (`churn`, `churn/v2`). Nunca vacío.
    pub carpeta: String,
    /// El nombre para las personas («New Pipelines Java Transform»).
    pub nombre: Option<String>,
    /// La clase: `transforms`, `analytics`, `models`, `functions`, `semantics`.
    pub plantilla: Option<String>,
    /// Con qué versión de la clase se creó. La compara el producto (0036 ⑥).
    pub plantilla_version: Option<i64>,
    /// La ruta del manifiesto.
    pub manifiesto: String,
    /// Por qué no se entiende. Se lista igual: el índice enseña lo que hay.
    pub roto: Option<String>,
}

impl Repositorio {
    /// ¿Este ítem cae dentro? `carpeta` es la del ítem dentro de su paquete.
    pub fn contiene(&self, paquete: &str, carpeta: &str) -> bool {
        paquete == self.paquete
            && (carpeta == self.carpeta || carpeta.starts_with(&format!("{}/", self.carpeta)))
    }
}

/// El repositorio de un ítem: el **más hondo** que lo contiene, si hay alguno.
///
/// Dos repositorios anidados no se reparten un ítem: se lo queda el de dentro,
/// que es donde alguien está trabajando en él.
pub fn de_item<'a>(
    repos: &'a [Repositorio],
    paquete: &str,
    carpeta: &str,
) -> Option<&'a Repositorio> {
    repos
        .iter()
        .filter(|r| r.roto.is_none() && r.contiene(paquete, carpeta))
        .max_by_key(|r| r.carpeta.len())
}

/// Todo lo que cuelga de `dir`, en orden, buscando `README.md`.
fn readmes(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entradas) = std::fs::read_dir(dir) else {
        return;
    };
    let mut hijos: Vec<std::path::PathBuf> =
        entradas.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    hijos.sort();
    for p in hijos {
        if p.is_dir() {
            if p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with('.'))
            {
                continue;
            }
            readmes(&p, out);
        } else if p
            .file_name()
            .is_some_and(|n| n.eq_ignore_ascii_case("README.md"))
        {
            out.push(p);
        }
    }
}

/// Los repositorios de un árbol: los `packages/<pkg>/<…>/README.md` **con
/// `plantilla`**, por ruta.
///
/// No falla nunca: lo que no se entiende vuelve con `roto`. Un README sin
/// `plantilla` **no es un repositorio** y no se lista — no es un error: es una
/// carpeta con README, que es lo normal en un árbol.
pub fn leer(raiz: &Path) -> Vec<Repositorio> {
    let paquetes = raiz.join("packages");
    let Ok(entradas) = std::fs::read_dir(&paquetes) else {
        return Vec::new();
    };
    let mut nombres: Vec<String> = entradas
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    nombres.sort();
    let mut out = Vec::new();
    for paquete in nombres {
        let dir = paquetes.join(&paquete);
        let mut ficheros = Vec::new();
        readmes(&dir, &mut ficheros);
        for f in ficheros {
            let Ok(texto) = std::fs::read_to_string(&f) else {
                continue;
            };
            // La carpeta del repositorio: lo que va del paquete al README.
            let Some(padre) = f.parent() else { continue };
            let Ok(rel) = padre.strip_prefix(&dir) else {
                continue;
            };
            let carpeta = rel.to_string_lossy().replace('\\', "/");
            // El README del propio paquete no es un repositorio: un repositorio
            // es un sitio DENTRO, y hacer del paquete entero uno borraría la
            // diferencia entre «el paquete» y «donde se trabaja».
            if carpeta.is_empty() {
                continue;
            }
            let ruta = format!("packages/{paquete}/{carpeta}");
            let manifiesto_ruta = format!("{ruta}/README.md");
            match manifiesto::encabezado(&texto) {
                // Sin encabezado que se entienda, no hay forma de saber si
                // pretendía ser un repositorio: se deja pasar como carpeta.
                Err(_) => continue,
                Ok(n) => {
                    let Some(plantilla) = manifiesto::campo(&n, "plantilla") else {
                        continue;
                    };
                    let nombre = manifiesto::campo(&n, "nombre");
                    let version = manifiesto::campo(&n, "plantillaVersion")
                        .and_then(|v| v.parse::<i64>().ok());
                    out.push(Repositorio {
                        roto: nombre.is_none().then(|| "sin `nombre`".to_string()),
                        ruta,
                        paquete: paquete.clone(),
                        carpeta,
                        nombre,
                        plantilla: Some(plantilla),
                        plantilla_version: version,
                        manifiesto: manifiesto_ruta,
                    });
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use std::path::Path;

    /// Un directorio que se borra solo: ore-core no depende de `tempfile`.
    struct Arbol(std::path::PathBuf);
    impl Arbol {
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Arbol {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn arbol(caso: &str, ficheros: &[(&str, &str)]) -> Arbol {
        let d = Arbol(
            std::env::temp_dir().join(format!("ore-repositorios-{}-{caso}", std::process::id())),
        );
        let _ = std::fs::remove_dir_all(d.path());
        for (ruta, texto) in ficheros {
            let p = d.path().join(ruta);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, texto).unwrap();
        }
        d
    }

    const UNO: &str = "---\nnombre: New Pipelines Java Transform\nplantilla: transforms\nplantillaVersion: 2\n---\nProsa.\n";

    #[test]
    fn un_manifiesto_se_lee_entero() {
        let d = arbol("uno", &[("packages/ventas/churn/README.md", UNO)]);
        let rs = leer(d.path());
        assert_eq!(rs.len(), 1);
        assert_eq!(rs[0].ruta, "packages/ventas/churn");
        assert_eq!(rs[0].paquete, "ventas");
        assert_eq!(rs[0].carpeta, "churn");
        assert_eq!(
            rs[0].nombre.as_deref(),
            Some("New Pipelines Java Transform")
        );
        assert_eq!(rs[0].plantilla.as_deref(), Some("transforms"));
        assert_eq!(rs[0].plantilla_version, Some(2));
        assert_eq!(rs[0].manifiesto, "packages/ventas/churn/README.md");
        assert!(rs[0].roto.is_none());
    }

    #[test]
    fn sin_plantilla_es_una_carpeta_con_readme_y_no_un_repositorio() {
        let d = arbol(
            "sin",
            &[
                ("packages/ventas/README.md", "# El paquete\n"),
                ("packages/ventas/notas/README.md", "# Notas\n"),
                (
                    "packages/ventas/mio/README.md",
                    "---\nnombre: Mío\ndescripcion: sin plantilla\n---\n",
                ),
                ("packages/ventas/churn/README.md", UNO),
            ],
        );
        let rs = leer(d.path());
        assert_eq!(
            rs.iter().map(|r| r.ruta.as_str()).collect::<Vec<_>>(),
            ["packages/ventas/churn"],
            "sólo `plantilla:` hace un repositorio"
        );
    }

    #[test]
    fn el_readme_del_paquete_no_es_un_repositorio() {
        let d = arbol("paquete", &[("packages/ventas/README.md", UNO)]);
        assert!(leer(d.path()).is_empty());
    }

    #[test]
    fn lo_roto_se_lista_igual_con_su_porque() {
        let d = arbol(
            "roto",
            &[(
                "packages/ventas/churn/README.md",
                "---\nplantilla: transforms\n---\n",
            )],
        );
        let rs = leer(d.path());
        assert_eq!(rs.len(), 1);
        assert_eq!(rs[0].roto.as_deref(), Some("sin `nombre`"));
        assert_eq!(rs[0].plantilla.as_deref(), Some("transforms"));
    }

    #[test]
    fn anidar_es_hondura_y_no_solape() {
        let d = arbol(
            "anidado",
            &[
                ("packages/ventas/churn/README.md", UNO),
                (
                    "packages/ventas/churn/modelo/README.md",
                    "---\nnombre: Modelo\nplantilla: models\n---\n",
                ),
            ],
        );
        let rs = leer(d.path());
        assert_eq!(rs.len(), 2);
        // El de dentro se queda lo suyo; el de fuera, lo demás.
        assert_eq!(
            de_item(&rs, "ventas", "churn/modelo").map(|r| r.ruta.as_str()),
            Some("packages/ventas/churn/modelo")
        );
        assert_eq!(
            de_item(&rs, "ventas", "churn").map(|r| r.ruta.as_str()),
            Some("packages/ventas/churn")
        );
        assert_eq!(de_item(&rs, "ventas", ""), None, "fuera de todos");
        assert_eq!(de_item(&rs, "rrhh", "churn"), None, "otro paquete, no");
    }

    #[test]
    fn un_repositorio_roto_no_se_queda_ningun_item() {
        let d = arbol(
            "roto-no-alcanza",
            &[(
                "packages/ventas/churn/README.md",
                "---\nplantilla: transforms\n---\n",
            )],
        );
        let rs = leer(d.path());
        assert_eq!(de_item(&rs, "ventas", "churn"), None);
    }
}
