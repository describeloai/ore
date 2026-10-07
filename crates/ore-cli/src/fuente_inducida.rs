//! `ore source induce <fuente>` — **el único escritor del paquete de la fuente**
//! (ADR 0045 P3′, con la regla de 0046 E5′).
//!
//! El puntero al origen (`Table`, `ObjectTable`) vive una vez, en
//! `packages/<fuente>/`, y las bases lo nombran: una standard con sus
//! `Dataset` y colecciones mantenidas, una foreign con sus `View` y
//! colecciones virtuales.
//!
//! ⭐⭐ 0046 E5′ · **SE ESCRIBE TODO LO CATALOGADO**, no lo que alguna base usa.
//!   El puntero es un hecho del origen —existe, tiene estas columnas, emite
//!   estos cambios— y un hecho no depende de que alguien lo lea: así cada
//!   activo del origen tiene su ficha, entra en el índice y se puede gobernar
//!   antes de que exista una base, y crear una base es solo elegir. Se retira
//!   lo que **desaparece del origen** (sale del catálogo), no lo que deja de
//!   usarse. Medido antes (0046): en victor +88 punteros, en demo +237, todo
//!   compila; un origen de 2000 tablas son 3 s de inducción y 4,8 MB.
//!
//! Lo corre `ore source catalog` al escribir el catálogo en el paquete de la
//! fuente (el Job de catálogo), y —idempotente, por los árboles catalogados
//! antes de esta regla— `discover`, `review`, `model` y `copy` al terminar una
//! base.
//!
//! ⛔ No crea el paquete de la fuente: lo crea el Job de catálogo, y que exista
//!   es «catalogada» para ore-serve.

use crate::inductor::{self, Catalogo};
use crate::revision::Fallo;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Lo que cambió en la fuente, para decirlo.
#[derive(Default, Debug)]
pub struct Informe {
    pub escritas: usize,
    pub retiradas: Vec<String>,
}

fn fallo(codigo: u8, mensaje: impl Into<String>) -> Fallo {
    Fallo {
        codigo,
        mensaje: mensaje.into(),
        ayuda: Vec::new(),
    }
}

/// El paquete de la fuente dentro de un repositorio, si lo es: tiene
/// `package.yaml` y no tiene alcance.
pub fn dir(repo: &Path, fuente: &str) -> Option<PathBuf> {
    let d = repo.join("packages").join(fuente);
    (d.join("package.yaml").is_file() && !crate::alcance::ruta(&d).is_file()).then_some(d)
}

/// Las referencias de una base a los punteros de su fuente, si la fuente tiene
/// paquete y catálogo: objeto del origen → `<fuente>.<schema>.<nombre>`. Del
/// catálogo **de la fuente**, que es el que da los nombres.
pub fn referencias(
    repo: &Path,
    fuente: &str,
) -> Option<std::collections::BTreeMap<String, String>> {
    let d = dir(repo, fuente)?;
    let texto = std::fs::read_to_string(d.join("discover.catalog.json")).ok()?;
    let cat = Catalogo::leer(&texto).ok()?;
    Some(inductor::referencias_a_la_fuente(&cat))
}

/// **Induce el paquete de la fuente.** Sin paquete de la fuente no hace nada
/// (`Ok(None)`): el árbol es de antes, o el CLI va suelto.
pub fn inducir(repo: &Path, fuente: &str) -> Result<Option<Informe>, Fallo> {
    let Some(d) = dir(repo, fuente) else {
        return Ok(None);
    };
    let ruta = d.join("discover.catalog.json");
    let texto = std::fs::read_to_string(&ruta).map_err(|e| {
        fallo(
            66,
            format!(
                "`{fuente}` no tiene catálogo (`{}`): {e}. Lo escribe el Job de catálogo",
                ruta.display()
            ),
        )
    })?;
    let cat = Catalogo::leer(&texto)
        .map_err(|m| fallo(65, format!("`{}` no analiza: {m}", ruta.display())))?;
    if cat.fuente() != fuente {
        return Err(fallo(
            65,
            format!(
                "el catálogo de `packages/{fuente}` es de `{}`",
                cat.fuente()
            ),
        ));
    }
    // Todo lo catalogado: tablas y conjuntos de objetos.
    let usados: BTreeSet<String> = cat
        .tablas
        .iter()
        .map(|t| t.nombre.clone())
        .chain(cat.objetos.iter().map(|o| o.nombre.clone()))
        .collect();
    let dec = crate::revision::acumuladas(&d)?;
    let manifiesto = std::fs::read_to_string(d.join("package.yaml"))
        .map_err(|e| fallo(66, format!("no se pudo leer el paquete de `{fuente}`: {e}")))?;
    let owner = ore_core::parse::parse(&manifiesto)
        .ok()
        .and_then(|n| {
            n.get("spec")
                .and_then(|(_, s)| s.get("owner"))
                .and_then(|(_, o)| o.as_str().map(String::from))
        })
        .unwrap_or_else(|| "cambiame".into());
    let f = inductor::inducir_la_fuente(&cat, &usados, &dec, &owner);

    let marca = crate::MARCA_INDUCIDO;
    let marcado = |p: &Path| {
        std::fs::read_to_string(p)
            .ok()
            .is_some_and(|t| t.lines().next() == Some(marca))
    };
    let mut informe = Informe::default();

    // Retirar lo inducido que ya no está en el origen: en `tables/` y en la de cada
    // schema, y el `schema.yaml` que se queda sin nada. Sólo lo marcado.
    let mut carpetas = vec![String::new()];
    if let Ok(es) = std::fs::read_dir(&d) {
        for e in es.flatten() {
            if e.path().is_dir()
                && let Some(n) = e.file_name().to_str()
                && !n.starts_with('.')
            {
                carpetas.push(format!("{n}/"));
            }
        }
    }
    for c in &carpetas {
        // v1alpha16: los conjuntos de objetos (`objects/`) son punteros igual.
        for kind in ["tables", "objects"] {
            let Ok(es) = std::fs::read_dir(d.join(format!("{c}{kind}"))) else {
                continue;
            };
            for e in es.flatten() {
                let Some(base) = e.file_name().to_str().map(String::from) else {
                    continue;
                };
                let rel = format!("{c}{kind}/{base}");
                if base.ends_with(".yaml") && !f.ficheros.contains_key(&rel) && marcado(&e.path()) {
                    std::fs::remove_file(e.path())
                        .map_err(|er| fallo(73, format!("no se pudo retirar `{rel}`: {er}")))?;
                    informe.retiradas.push(rel);
                }
            }
            let _ = std::fs::remove_dir(d.join(format!("{c}{kind}")));
        }
        if !c.is_empty() {
            let rel = format!("{c}schema.yaml");
            let vacia = std::fs::read_dir(d.join(c))
                .map(|es| es.count())
                .unwrap_or(0)
                == 1;
            if !f.ficheros.contains_key(&rel) && vacia && marcado(&d.join(&rel)) {
                let _ = std::fs::remove_file(d.join(&rel));
                let _ = std::fs::remove_dir(d.join(c));
                informe.retiradas.push(rel);
            }
        }
    }

    for (rel, contenido) in &f.ficheros {
        let ruta = d.join(rel);
        if let Some(p) = ruta.parent() {
            std::fs::create_dir_all(p)
                .map_err(|e| fallo(73, format!("no se pudo crear `{}`: {e}", p.display())))?;
        }
        let texto = format!("{marca}\n{contenido}");
        if std::fs::read_to_string(&ruta).ok().as_deref() != Some(texto.as_str()) {
            std::fs::write(&ruta, texto)
                .map_err(|e| fallo(73, format!("no se pudo escribir `{}`: {e}", ruta.display())))?;
            informe.escritas += 1;
        }
    }

    // OOS v1alpha28 (ORE 0059): un árbol es un catálogo y sus bases se leen
    // por su nombre; la fuente no abre lo que cataloga con `exports`. Los que
    // ya escribió se quedan: dentro del árbol no estorban ni conceden.
    Ok(Some(informe))
}

/// Tras inducir una base: la fuente, si tiene paquete. Lo que falle se dice y
/// no tumba a la base, que ya está escrita: el árbol lo dirá al compilar.
pub fn tras_la_base(desde: &Path, fuente: &str) -> Option<String> {
    let repo = crate::raiz_del_repositorio(desde)?;
    match inducir(&repo, fuente) {
        Ok(Some(i)) => Some(resumen(fuente, &i)),
        Ok(None) => None,
        Err(f) => {
            eprintln!(
                "aviso: la fuente `{fuente}` no se pudo inducir: {}",
                f.mensaje
            );
            None
        }
    }
}

pub fn resumen(fuente: &str, i: &Informe) -> String {
    let mut s = format!(
        "  ✓ fuente `{fuente}`: {} puntero(s) escrito(s)",
        i.escritas
    );
    if !i.retiradas.is_empty() {
        s.push_str(&format!(
            " · {} retirado(s), ya no están en el origen",
            i.retiradas.len()
        ));
    }
    s.push('\n');
    s
}
