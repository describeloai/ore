//! **`ore migrate proyectos`** — el paquete de un proyecto puede publicar
//! (ORE 0050).
//!
//! Hasta el 2026-10-01 el identificador de un proyecto salía del título con
//! guiones —«Test Project» → `test-project`— y el proyecto nacía con su
//! paquete del mismo nombre (0035 ⑦.1). Un paquete así no puede ser
//! `namespace` (`OOS2030`): lo que se hiciera en sus repositorios —una función,
//! un dataset— no podía tener documento gobernado. Ahora el id es un
//! identificador (`test_project`), y esto pone al día lo de antes:
//!
//! | antes | después |
//! |---|---|
//! | `proyectos/test-project/` | `proyectos/test_project/` |
//! | `packages/test-project/`, `name`/`domain: test-project` | `packages/test_project/`, `test_project` |
//! | `contiene: [test-project, test-project/python]`, en cualquier proyecto | `[test_project, test_project/python]` |
//!
//! Solo se toca el paquete que es **el sitio** del proyecto: el que se llama
//! como él. Un paquete con guion que no es de ningún proyecto (uno importado,
//! `oos.dev`) no es asunto de esta migración.
//!
//! Se ensaya en una copia, y **no se escribe nada** si el árbol da un
//! diagnóstico que antes no daba, o si el nombre nuevo ya está cogido. Las
//! ramas no se tocan: una rama es una foto, y al fusionarla git sigue el
//! renombre de la carpeta.

use crate::migrar::Opciones;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Un proyecto que se renombra, y si su paquete va con él.
#[derive(Debug, Default)]
struct Informe {
    renombrados: Vec<(String, String, bool)>,
    contiene: usize,
    avisos: Vec<String>,
}

pub fn migrar(path: &Path, op: &Opciones) -> std::process::ExitCode {
    let Some(raiz) = crate::raiz_del_repositorio(path) else {
        eprintln!(
            "ore migrate proyectos · `{}` no está en un repositorio",
            path.display()
        );
        return std::process::ExitCode::from(66);
    };
    let tmp = std::env::temp_dir().join(format!(
        "ore-migrate-proyectos-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let ensayo = crate::migrar_v14::copiar_arbol(&raiz, &tmp).and_then(|()| aplicar(&tmp));
    let nuevos = match &ensayo {
        Ok(_) => nuevos_diagnosticos(&codigos(&raiz), &codigos(&tmp)),
        Err(_) => Vec::new(),
    };
    let _ = std::fs::remove_dir_all(&tmp);
    let informe = match ensayo {
        Err(e) => {
            eprintln!(
                "ore migrate proyectos · no se migra, y no se escribe nada:
  · {e}"
            );
            return std::process::ExitCode::from(65);
        }
        Ok(_) if !nuevos.is_empty() => {
            eprintln!(
                "ore migrate proyectos · no se migra, y no se escribe nada: el árbol daría diagnósticos que hoy no da:"
            );
            for x in nuevos {
                eprintln!("  · {x}");
            }
            return std::process::ExitCode::from(65);
        }
        Ok(i) => i,
    };
    let informe = if op.seco {
        informe
    } else {
        match aplicar(&raiz) {
            Ok(i) => i,
            Err(e) => {
                eprintln!("ore migrate proyectos · el ensayo pasó y el árbol no: {e}");
                return std::process::ExitCode::from(74);
            }
        }
    };
    if informe.renombrados.is_empty() {
        println!(
            "ore migrate proyectos · nada que migrar: todo proyecto se llama con un identificador"
        );
        return std::process::ExitCode::SUCCESS;
    }
    println!(
        "ore migrate proyectos{} · {} proyecto(s) · {} `contiene` reescrito(s) · mismos diagnósticos",
        if op.seco {
            " (en seco: no se ha escrito nada)"
        } else {
            ""
        },
        informe.renombrados.len(),
        informe.contiene,
    );
    for (de, a, paquete) in &informe.renombrados {
        let y = if *paquete {
            format!(" y `packages/{de}` → `packages/{a}`")
        } else {
            String::new()
        };
        println!("  · `proyectos/{de}` → `proyectos/{a}`{y}");
    }
    for a in &informe.avisos {
        println!("  · {a}");
    }
    std::process::ExitCode::SUCCESS
}

/// Los códigos de los diagnósticos del árbol, con cuántas veces sale cada uno.
fn codigos(raiz: &Path) -> BTreeMap<String, usize> {
    let mut m = BTreeMap::new();
    for d in ore_core::validate::validate_package(raiz) {
        *m.entry(d.code.as_str().to_string()).or_default() += 1;
    }
    m
}

fn nuevos_diagnosticos(
    antes: &BTreeMap<String, usize>,
    despues: &BTreeMap<String, usize>,
) -> Vec<String> {
    despues
        .iter()
        .filter(|(c, n)| antes.get(*c).copied().unwrap_or(0) < **n)
        .map(|(c, n)| format!("{c}: {} → {n}", antes.get(c).copied().unwrap_or(0)))
        .collect()
}

/// El id nuevo de uno de antes: los guiones, `_`. `None` si ya vale, o si ni
/// así sería un identificador (entonces se dice y no se toca).
fn nuevo_id(id: &str) -> Option<String> {
    if ore_core::pertenencia::puede_ser_namespace(id) {
        return None;
    }
    let n = id.replace('-', "_");
    ore_core::pertenencia::puede_ser_namespace(&n).then_some(n)
}

fn aplicar(raiz: &Path) -> Result<Informe, String> {
    let mut informe = Informe::default();
    let proyectos = raiz.join("proyectos");
    let Ok(es) = std::fs::read_dir(&proyectos) else {
        return Ok(informe);
    };
    let mut ids: Vec<String> = es
        .flatten()
        .filter(|e| e.path().join("README.md").is_file())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    ids.sort();

    let mut renombres: BTreeMap<String, String> = BTreeMap::new();
    for id in &ids {
        if ore_core::pertenencia::puede_ser_namespace(id) {
            continue;
        }
        let Some(nuevo) = nuevo_id(id) else {
            informe.avisos.push(format!(
                "`proyectos/{id}` no tiene un identificador ni cambiando los guiones por `_`: se queda como está"
            ));
            continue;
        };
        for ocupado in [proyectos.join(&nuevo), raiz.join("packages").join(&nuevo)] {
            if ocupado.exists() {
                return Err(format!(
                    "`{id}` pasaría a `{nuevo}`, y `{}` ya existe",
                    ocupado.strip_prefix(raiz).unwrap_or(&ocupado).display()
                ));
            }
        }
        renombres.insert(id.clone(), nuevo);
    }

    for (id, nuevo) in &renombres {
        mover(&proyectos.join(id), &proyectos.join(nuevo))?;
        let de = raiz.join("packages").join(id);
        let paquete = de.join("package.yaml");
        let es_su_sitio = std::fs::read_to_string(&paquete)
            .ok()
            .and_then(|t| ore_core::parse::parse(&t).ok())
            .and_then(|n| {
                n.get("metadata")
                    .and_then(|(_, m)| m.get("name"))
                    .and_then(|(_, v)| v.as_str().map(str::to_string))
            })
            .is_some_and(|n| &n == id);
        if es_su_sitio {
            let texto = std::fs::read_to_string(&paquete).map_err(|e| e.to_string())?;
            std::fs::write(&paquete, renombrar_paquete(&texto, id, nuevo)?)
                .map_err(|e| format!("{}: {e}", paquete.display()))?;
            mover(&de, &raiz.join("packages").join(nuevo))?;
        }
        informe
            .renombrados
            .push((id.clone(), nuevo.clone(), es_su_sitio));
    }

    // `contiene`, en todos los proyectos: lo que nombraba lo de antes.
    if !renombres.is_empty() {
        let mut readmes: Vec<PathBuf> = std::fs::read_dir(&proyectos)
            .map_err(|e| e.to_string())?
            .flatten()
            .map(|e| e.path().join("README.md"))
            .filter(|p| p.is_file())
            .collect();
        readmes.sort();
        for r in readmes {
            let texto = std::fs::read_to_string(&r).map_err(|e| e.to_string())?;
            if let Some(nuevo) = renombrar_contiene(&texto, &renombres) {
                std::fs::write(&r, nuevo).map_err(|e| format!("{}: {e}", r.display()))?;
                informe.contiene += 1;
            }
        }
    }
    Ok(informe)
}

fn mover(de: &Path, a: &Path) -> Result<(), String> {
    std::fs::rename(de, a).map_err(|e| format!("{} → {}: {e}", de.display(), a.display()))
}

/// `name` y `domain` del `package.yaml`, de `id` a `nuevo`, sin tocar lo demás.
fn renombrar_paquete(texto: &str, id: &str, nuevo: &str) -> Result<String, String> {
    let mut out = String::with_capacity(texto.len());
    for linea in texto.split_inclusive('\n') {
        let mut l = linea.to_string();
        for clave in ["name", "domain"] {
            for (viejo, bueno) in [
                (format!("{clave}: {id}"), format!("{clave}: {nuevo}")),
                (
                    format!("{clave}: \"{id}\""),
                    format!("{clave}: \"{nuevo}\""),
                ),
            ] {
                if let Some(i) = l.find(&viejo) {
                    let fin = i + viejo.len();
                    let sigue = l[fin..].chars().next();
                    if sigue.is_none_or(|c| matches!(c, ',' | ' ' | '}' | '\n' | '\r')) {
                        l.replace_range(i..fin, &bueno);
                    }
                }
            }
        }
        out.push_str(&l);
    }
    let n = ore_core::parse::parse(&out).map_err(|e| format!("package.yaml: {e:?}"))?;
    let nombre = n
        .get("metadata")
        .and_then(|(_, m)| m.get("name"))
        .and_then(|(_, v)| v.as_str());
    if nombre != Some(nuevo) {
        return Err(format!(
            "no se supo reescribir el `name` de `packages/{id}/package.yaml`"
        ));
    }
    Ok(out)
}

/// La línea `contiene:` del encabezado, con cada `<id>` o `<id>/…` renombrado.
/// `None` si no nombra nada de lo que cambia.
fn renombrar_contiene(texto: &str, renombres: &BTreeMap<String, String>) -> Option<String> {
    let mut cambio = false;
    let mut out = String::with_capacity(texto.len());
    let mut en_encabezado = false;
    for (i, linea) in texto.split_inclusive('\n').enumerate() {
        let t = linea.trim_end();
        if t == "---" {
            en_encabezado = i == 0;
        }
        let Some(lista) = t
            .strip_prefix("contiene:")
            .map(str::trim)
            .and_then(|l| l.strip_prefix('['))
            .and_then(|l| l.strip_suffix(']'))
            .filter(|_| en_encabezado)
        else {
            out.push_str(linea);
            continue;
        };
        let items: Vec<String> = lista
            .split(',')
            .map(|x| x.trim().trim_matches('"').to_string())
            .filter(|x| !x.is_empty())
            .map(|x| {
                let (cabeza, resto) = x.split_once('/').unwrap_or((&x, ""));
                match renombres.get(cabeza) {
                    Some(n) => {
                        cambio = true;
                        if resto.is_empty() {
                            n.clone()
                        } else {
                            format!("{n}/{resto}")
                        }
                    }
                    None => x.clone(),
                }
            })
            .collect();
        out.push_str(&format!("contiene: [{}]\n", items.join(", ")));
    }
    cambio.then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arbol() -> PathBuf {
        let raiz = std::env::temp_dir().join(format!(
            "ore-migrar-proyectos-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&raiz);
        let escribir = |rel: &str, t: &str| {
            let f = raiz.join(rel);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, t).unwrap();
        };
        escribir(
            "ontology.config.yaml",
            "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\nmetadata: { name: t, version: 0.1.0 }\n",
        );
        escribir(
            "packages/test-project/package.yaml",
            &ore_core::paquetes::documento("test-project", "team:x", "draft", "test-project"),
        );
        escribir(
            "packages/test-project/python/README.md",
            "---\nnombre: python\nplantilla: transforms-python\nplantillaVersion: 4\n---\n",
        );
        escribir(
            "proyectos/test-project/README.md",
            "---\nnombre: \"test_project\"\ncontiene: [test-project]\n---\n\nLo que este proyecto hace, en prosa.\n",
        );
        escribir(
            "proyectos/otro/README.md",
            "---\nnombre: \"Otro\"\ncontiene: [test-project/python, hr]\n---\n\ncontiene: [test-project] en la prosa no se toca\n",
        );
        raiz
    }

    #[test]
    fn el_proyecto_y_su_paquete_pasan_a_un_identificador() {
        let raiz = arbol();
        let i = aplicar(&raiz).unwrap();
        assert_eq!(
            i.renombrados,
            vec![("test-project".into(), "test_project".into(), true)]
        );
        assert_eq!(i.contiene, 2);
        assert!(
            raiz.join("packages/test_project/python/README.md")
                .is_file()
        );
        assert!(!raiz.join("packages/test-project").exists());
        let p = std::fs::read_to_string(raiz.join("packages/test_project/package.yaml")).unwrap();
        assert!(
            p.contains("name: test_project,") && p.contains("domain: test_project }"),
            "{p}"
        );
        let m = std::fs::read_to_string(raiz.join("proyectos/test_project/README.md")).unwrap();
        assert!(m.contains("contiene: [test_project]\n"), "{m}");
        let o = std::fs::read_to_string(raiz.join("proyectos/otro/README.md")).unwrap();
        assert!(o.contains("contiene: [test_project/python, hr]\n"), "{o}");
        assert!(o.contains("contiene: [test-project] en la prosa"), "{o}");
        assert!(ore_core::validate::validate_package(&raiz).is_empty());
        // Y otra vez no hace nada.
        let otra = aplicar(&raiz).unwrap();
        assert!(otra.renombrados.is_empty() && otra.contiene == 0);
        let _ = std::fs::remove_dir_all(&raiz);
    }

    #[test]
    fn si_el_nombre_nuevo_esta_cogido_no_se_toca_nada() {
        let raiz = arbol();
        std::fs::create_dir_all(raiz.join("packages/test_project")).unwrap();
        let e = aplicar(&raiz).unwrap_err();
        assert!(e.contains("ya existe"), "{e}");
        assert!(raiz.join("proyectos/test-project/README.md").is_file());
        let _ = std::fs::remove_dir_all(&raiz);
    }
}
