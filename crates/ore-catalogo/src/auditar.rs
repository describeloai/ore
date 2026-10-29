//! **La auditoría**: lo que el catálogo ve frente a una referencia. En la sombra, la referencia es
//! git con LA MISMA regla que sirve ore-serve ([`ore_core::punteros::manda`]); tras el corte será
//! GCS y la coherencia propia del catálogo (ADR 0079 §18).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use ore_core::punteros::{Manda, manda};

use crate::almacen::{Almacen, Diferencia, Esperado};
use crate::git::Espejo;

/// Lo que una ref debe ver según git: activo → (de dónde, blob).
pub fn esperado_en_git(
    en_rama: &BTreeMap<String, String>,
    en_base: &BTreeMap<String, String>,
    en_main: &BTreeMap<String, String>,
) -> BTreeMap<String, (&'static str, String)> {
    let rutas: BTreeSet<&String> = en_rama
        .keys()
        .chain(en_base.keys())
        .chain(en_main.keys())
        .collect();
    rutas
        .into_iter()
        .filter_map(|r| {
            let (rama, base, main) = (en_rama.get(r), en_base.get(r), en_main.get(r));
            match manda(
                rama.map(String::as_str),
                base.map(String::as_str),
                main.map(String::as_str),
            ) {
                Manda::Rama => Some((r.clone(), ("rama", rama?.clone()))),
                Manda::Main(b) => Some((r.clone(), ("main", b.to_string()))),
                Manda::Ninguno => None,
            }
        })
        .collect()
}

/// Audita main y cada rama del espejo. Devuelve cuántos (ref, activo) se compararon y lo distinto.
pub fn auditar(e: &Espejo, a: &mut Almacen) -> Result<(usize, Vec<Diferencia>), String> {
    let ramas = e.ramas()?;
    let punta = ramas.get("main").ok_or("la forja no tiene `main`")?;
    let en_main = e.arbol(punta)?;
    let mut textos: HashMap<String, String> = HashMap::new();
    let (mut comparados, mut difs) = (0, Vec::new());
    for (r, sha) in &ramas {
        let esperado: BTreeMap<String, (&'static str, String)> = if r == "main" {
            en_main
                .iter()
                .map(|(k, b)| (k.clone(), ("main", b.clone())))
                .collect()
        } else {
            let base = e.base_comun(punta, sha)?;
            esperado_en_git(&e.arbol(sha)?, &e.arbol(&base)?, &en_main)
        };
        let faltan: Vec<String> = esperado
            .values()
            .map(|(_, b)| b.clone())
            .filter(|b| !textos.contains_key(b))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        textos.extend(e.contenidos(&faltan)?);
        let lista: Vec<Esperado> = esperado
            .into_iter()
            .map(|(activo, (de, b))| Esperado {
                activo,
                de,
                texto: textos[&b].clone(),
            })
            .collect();
        comparados += lista.len();
        difs.extend(a.auditar_ref(r, &lista)?);
    }
    Ok((comparados, difs))
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn m(pares: &[(&str, &str)]) -> BTreeMap<String, String> {
        pares
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    #[test]
    fn la_regla_de_hoy() {
        let base = m(&[
            ("cambia", "c0"),
            ("borra", "b0"),
            ("intacta", "i0"),
            ("retira", "r0"),
        ]);
        let rama = m(&[
            ("cambia", "c1"),
            ("intacta", "i0"),
            ("retira", "r0"),
            ("nueva", "n1"),
        ]);
        let main = m(&[
            ("cambia", "c0"),
            ("borra", "b0"),
            ("intacta", "i2"),
            ("posterior", "p1"),
        ]);
        let e = esperado_en_git(&rama, &base, &main);
        assert_eq!(e["cambia"], ("rama", "c1".to_string()));
        assert_eq!(e["nueva"], ("rama", "n1".to_string()));
        assert!(!e.contains_key("borra"));
        assert_eq!(e["intacta"], ("main", "i2".to_string()));
        assert_eq!(e["posterior"], ("main", "p1".to_string()));
        assert!(!e.contains_key("retira"));
    }
}
