//! **El proyecto** (0035 ⑤): un propósito con dueño, un sitio en el árbol y
//! unas ramas donde se trabaja. Una lente, no una caja.
//!
//! El sitio es `proyectos/<nombre>/README.md`, con encabezado:
//!
//! ```text
//! ---
//! nombre: Customer Churn
//! descripcion: Predicción de abandono sobre los pedidos.
//! contiene: [ventas/churn, rrhh/nomina]
//! ---
//! Lo que este proyecto hace, en prosa.
//! ```
//!
//! Tres cosas que este módulo **no** hace, y son la decisión:
//!
//! 1. **No entra en la compilación.** Medido (0035 ⓪): con el manifiesto dentro,
//!    `ore validate` sale 0 y no lo nombra en demo ni en victor, y el índice da
//!    los mismos ítems. Un `README.md` se ignora; un `.yaml` sin kind sería
//!    `OOS1002` y un `kind: Project`, `OOS1003` — el proyecto **no es un
//!    documento de la ontología**.
//! 2. **Un manifiesto roto no rompe el árbol.** Se lee igual, con `roto` puesto:
//!    el catálogo enseña lo que hay, no lo arregla. Es la misma regla que una
//!    relación `rota: true` del índice.
//! 3. **No contiene: nombra.** `contiene` apunta a `<paquete>` o
//!    `<paquete>/<carpeta>`, que es lo que el índice ya sabe de cada ítem
//!    (0034 ④). Dos proyectos pueden nombrar lo mismo —se solapan, y por eso un
//!    ítem lleva `proyectos` en **plural**—, y `contiene` puede no resolver
//!    todavía: un proyecto vacío es un propósito antes de que haya nada.
use crate::parse;
use std::path::Path;

/// Un proyecto, leído de su manifiesto.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proyecto {
    /// El identificador: el nombre de la carpeta (`proyectos/<nombre>/`).
    pub nombre: String,
    /// El título que dice el encabezado; `None` si el manifiesto está roto.
    pub titulo: Option<String>,
    pub descripcion: Option<String>,
    /// Lo que nombra: `<paquete>` o `<paquete>/<carpeta>`.
    pub contiene: Vec<String>,
    /// La ruta del manifiesto, relativa a la raíz del árbol.
    pub ruta: String,
    /// Por qué no se entiende el encabezado, si no se entiende.
    pub roto: Option<String>,
}

impl Proyecto {
    /// ¿Este proyecto nombra a un ítem de este paquete y esta carpeta?
    ///
    /// `ventas` alcanza a todo el paquete; `ventas/churn`, a la carpeta y a lo
    /// que cuelga de ella.
    pub fn alcanza(&self, paquete: &str, carpeta: &str) -> bool {
        self.contiene.iter().any(|c| {
            let c = c.trim_matches('/');
            if c == paquete {
                return true;
            }
            let Some(resto) = c.strip_prefix(paquete).and_then(|r| r.strip_prefix('/')) else {
                return false;
            };
            carpeta == resto || carpeta.starts_with(&format!("{resto}/"))
        })
    }
}

/// El encabezado de un manifiesto: lo que hay entre la primera raya y la
/// segunda, analizado con el analizador del árbol. La prosa se ignora.
fn encabezado(texto: &str) -> Result<parse::Node, String> {
    let mut lineas = texto.lines();
    if lineas.next().map(str::trim) != Some("---") {
        return Err("sin encabezado".into());
    }
    let mut dentro = String::new();
    let mut cerro = false;
    for l in lineas {
        if l.trim() == "---" {
            cerro = true;
            break;
        }
        dentro.push_str(l);
        dentro.push('\n');
    }
    if !cerro {
        return Err("el encabezado no cierra".into());
    }
    let n =
        parse::parse(&dentro).map_err(|e| format!("el encabezado no se analiza: {}", e.message))?;
    if n.entries().is_empty() {
        return Err("el encabezado no es un mapa".into());
    }
    Ok(n)
}

fn texto(n: &parse::Node, clave: &str) -> Option<String> {
    n.get(clave)
        .and_then(|(_, v)| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Los proyectos de un árbol: `proyectos/*/README.md`, por nombre de carpeta.
///
/// No falla nunca: lo que no se entiende se devuelve con `roto`.
pub fn leer(raiz: &Path) -> Vec<Proyecto> {
    let dir = raiz.join("proyectos");
    let Ok(entradas) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut nombres: Vec<String> = entradas
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    nombres.sort();
    nombres
        .into_iter()
        .filter_map(|nombre| {
            let manifiesto = dir.join(&nombre).join("README.md");
            let texto_md = std::fs::read_to_string(&manifiesto).ok()?;
            let ruta = format!("proyectos/{nombre}/README.md");
            Some(match encabezado(&texto_md) {
                Err(e) => Proyecto {
                    nombre,
                    titulo: None,
                    descripcion: None,
                    contiene: Vec::new(),
                    ruta,
                    roto: Some(e),
                },
                Ok(n) => {
                    let titulo = texto(&n, "nombre");
                    let contiene: Vec<String> = n
                        .get("contiene")
                        .map(|(_, v)| {
                            v.items()
                                .iter()
                                .filter_map(|i| i.as_str())
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty())
                                .collect()
                        })
                        .unwrap_or_default();
                    Proyecto {
                        roto: titulo.is_none().then(|| "sin `nombre`".to_string()),
                        nombre,
                        titulo,
                        descripcion: texto(&n, "descripcion"),
                        contiene,
                        ruta,
                    }
                }
            })
        })
        .collect()
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

    fn arbol(caso: &str, manifiestos: &[(&str, &str)]) -> Arbol {
        let d = Arbol(
            std::env::temp_dir().join(format!("ore-proyectos-{}-{caso}", std::process::id())),
        );
        let _ = std::fs::remove_dir_all(d.path());
        for (n, t) in manifiestos {
            let p = d.path().join("proyectos").join(n);
            std::fs::create_dir_all(&p).unwrap();
            std::fs::write(p.join("README.md"), t).unwrap();
        }
        d
    }

    #[test]
    fn un_manifiesto_se_lee_y_la_prosa_se_ignora() {
        let d = arbol(
            "uno",
            &[(
                "churn",
                "---\nnombre: Customer Churn\ndescripcion: Abandono.\ncontiene: [ventas/churn, rrhh]\n---\n# Lo que hace\n\nProsa: contiene: mentira\n",
            )],
        );
        let ps = leer(d.path());
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0].nombre, "churn");
        assert_eq!(ps[0].titulo.as_deref(), Some("Customer Churn"));
        assert_eq!(ps[0].descripcion.as_deref(), Some("Abandono."));
        assert_eq!(ps[0].contiene, ["ventas/churn", "rrhh"]);
        assert_eq!(ps[0].ruta, "proyectos/churn/README.md");
        assert!(ps[0].roto.is_none());
    }

    #[test]
    fn lo_roto_se_lista_igual_y_dice_por_que() {
        let d = arbol(
            "roto",
            &[
                ("a", "---\ndescripcion: sin nombre\n---\n"),
                ("b", "---\nnombre: no cierra\n"),
                ("c", "# Sin encabezado\n"),
                ("d", "---\nnombre: bien\n---\n"),
            ],
        );
        let ps = leer(d.path());
        let dicho: Vec<_> = ps
            .iter()
            .map(|p| (p.nombre.as_str(), p.roto.as_deref()))
            .collect();
        assert_eq!(
            dicho,
            [
                ("a", Some("sin `nombre`")),
                ("b", Some("el encabezado no cierra")),
                ("c", Some("sin encabezado")),
                ("d", None),
            ]
        );
    }

    #[test]
    fn un_proyecto_vacio_es_legal() {
        let d = arbol("vacio", &[("nuevo", "---\nnombre: Nuevo\n---\n")]);
        let ps = leer(d.path());
        assert!(ps[0].roto.is_none());
        assert!(ps[0].contiene.is_empty());
        assert!(!ps[0].alcanza("ventas", ""));
    }

    #[test]
    fn alcanza_el_paquete_la_carpeta_y_lo_que_cuelga() {
        let d = arbol(
            "alcanza",
            &[("x", "---\nnombre: X\ncontiene: [ventas/churn, rrhh]\n---\n")],
        );
        let p = &leer(d.path())[0];
        assert!(p.alcanza("ventas", "churn"));
        assert!(p.alcanza("ventas", "churn/v2"), "lo que cuelga, también");
        assert!(!p.alcanza("ventas", ""), "el paquete entero, no");
        assert!(!p.alcanza("ventas", "churnalot"), "el prefijo no basta");
        assert!(p.alcanza("rrhh", ""), "el paquete nombrado, sí");
        assert!(p.alcanza("rrhh", "nomina"), "y todo lo suyo");
    }

    #[test]
    fn dos_proyectos_pueden_nombrar_lo_mismo() {
        let d = arbol(
            "solape",
            &[
                (
                    "churn",
                    "---\nnombre: Churn\ncontiene: [ventas/churn]\n---\n",
                ),
                (
                    "abandono",
                    "---\nnombre: Abandono\ncontiene: [ventas/churn]\n---\n",
                ),
            ],
        );
        let ps = leer(d.path());
        assert_eq!(ps.len(), 2);
        assert!(ps.iter().all(|p| p.alcanza("ventas", "churn")));
    }
}
