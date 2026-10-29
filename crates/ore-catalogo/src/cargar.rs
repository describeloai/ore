//! **El andamio** (ADR 0079 §17): sigue a git y replica en el catálogo cada movimiento de puntero,
//! con la historia entera. Llena el catálogo de un inquilino que ya existe y lo tiene al día hasta
//! su corte; entonces se apaga. Un inquilino nuevo nunca pasa por aquí.
//!
//! - **`main`**, por primer padre: cada commit de git es un commit del catálogo, en orden e
//!   idempotente por su sha. La carga inicial es este mismo camino desde el principio.
//! - **Una rama**: lo propio (el diff contra su merge-base con `main`) sustituye a lo que tenía,
//!   con el punto de salida en el commit del catálogo de esa merge-base.

use std::collections::BTreeMap;

use crate::almacen::Almacen;
use crate::git::{Cambio, Espejo};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Opciones {
    /// Tras cada commit replicado, `main` del catálogo frente al árbol de ese commit.
    pub verificar_pasos: bool,
    /// Parar tras N commits de main (el banco: una caída en un punto conocido).
    pub hasta_n: Option<usize>,
}

/// Lo que hizo una pasada.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Pasada {
    pub main: String,
    pub traida: bool,
    pub commits: usize,
    pub con_cambios: usize,
    pub pasos_verificados: usize,
    /// Los pasos en los que `main` del catálogo no era el árbol de git.
    pub pasos_mal: Vec<String>,
    pub ramas_replicadas: usize,
    pub ramas_borradas: usize,
    /// Se paró en `hasta_n` (el banco).
    pub cortada: bool,
}

fn blobs(cambios: &[Cambio]) -> Vec<String> {
    let mut v: Vec<String> = cambios.iter().filter_map(|c| c.blob.clone()).collect();
    v.sort();
    v.dedup();
    v
}

/// Las diferencias entre dos mapas ruta → blob, contadas.
fn distintos(a: &BTreeMap<String, String>, b: &BTreeMap<String, String>) -> usize {
    a.iter().filter(|(k, v)| b.get(*k) != Some(*v)).count()
        + b.keys().filter(|k| !a.contains_key(*k)).count()
}

/// El primer commit de primer padre de main que contiene `base`: su lista es monótona (una vez
/// contenido, contenido siempre), así que basta una búsqueda binaria.
fn primero_que_contiene(e: &Espejo, base: &str, lista: &[String]) -> Result<usize, String> {
    let (mut lo, mut hi) = (0, lista.len());
    while lo < hi {
        let m = (lo + hi) / 2;
        if e.es_antecesor(base, &lista[m])? {
            hi = m;
        } else {
            lo = m + 1;
        }
    }
    if lo == lista.len() {
        return Err(format!("{base} no está en la historia de main"));
    }
    Ok(lo)
}

/// Una pasada: trae si la forja cambió, replica main y las ramas, y borra las que ya no están.
pub fn pasada(e: &Espejo, a: &mut Almacen, op: &Opciones) -> Result<Pasada, String> {
    let mut p = Pasada::default();
    if e.ramas_remotas()? != e.ramas()? {
        e.traer()?;
        p.traida = true;
    }
    let ramas = e.ramas()?;
    let punta = ramas
        .get("main")
        .cloned()
        .ok_or("la forja no tiene `main`")?;
    p.main = punta.clone();

    // ── main
    let seguido = a.seguido("main")?;
    if seguido.as_deref() != Some(punta.as_str()) {
        let lista = e.primer_padre(seguido.as_deref(), &punta)?;
        let todos: Vec<Cambio> = lista.iter().flat_map(|c| c.cambios.clone()).collect();
        let textos = e.contenidos(&blobs(&todos))?;
        let mut vigente = a.ultimo_commit_main()?;
        for (i, c) in lista.iter().enumerate() {
            if op.hasta_n.is_some_and(|n| i >= n) {
                p.cortada = true;
                return Ok(p);
            }
            vigente = a.replicar_commit_main(c, &textos, vigente)?;
            p.commits += 1;
            p.con_cambios += usize::from(!c.cambios.is_empty());
            if op.verificar_pasos {
                p.pasos_verificados += 1;
                let n = distintos(&e.arbol(&c.sha)?, &a.punteros_main()?);
                if n > 0 {
                    p.pasos_mal
                        .push(format!("{}: {n} punteros distintos", &c.sha[..12]));
                }
            }
        }
    }

    // ── las ramas: se recalculan si cambió su punta o la de main con la que se calcularon
    let en_catalogo = a.ramas()?;
    let mut lista_main: Option<Vec<String>> = None;
    for (r, sha) in ramas.iter().filter(|(r, _)| r.as_str() != "main") {
        if en_catalogo.get(r) == Some(&(Some(sha.clone()), Some(punta.clone()))) {
            continue;
        }
        let base = e.base_comun(&punta, sha)?;
        if lista_main.is_none() {
            lista_main = Some(e.shas_primer_padre(&punta)?);
        }
        let lista = lista_main.as_deref().unwrap_or_default();
        let i = primero_que_contiene(e, &base, lista)?;
        let punto = a
            .punto_de(&lista[i])?
            .ok_or(format!("{} no está en el mapa del andamio", lista[i]))?;
        let propios = e.diferencias(&base, sha)?;
        let textos = e.contenidos(&blobs(&propios))?;
        a.replicar_rama(r, sha, &punta, punto, &propios, &textos)?;
        p.ramas_replicadas += 1;
    }
    for r in en_catalogo.keys().filter(|r| !ramas.contains_key(*r)) {
        a.borrar_rama(r)?;
        p.ramas_borradas += 1;
    }
    Ok(p)
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn contar_lo_distinto() {
        let a = BTreeMap::from([
            ("x".to_string(), "1".to_string()),
            ("y".to_string(), "2".to_string()),
        ]);
        let mut b = a.clone();
        assert_eq!(distintos(&a, &b), 0);
        b.insert("y".into(), "3".into());
        b.insert("z".into(), "4".into());
        assert_eq!(distintos(&a, &b), 2);
        b.remove("x");
        assert_eq!(distintos(&a, &b), 3);
    }

    #[test]
    fn los_blobs_sin_repetir_ni_borrados() {
        let cs = vec![
            Cambio {
                ruta: "a".into(),
                blob: Some("2".into()),
            },
            Cambio {
                ruta: "b".into(),
                blob: Some("1".into()),
            },
            Cambio {
                ruta: "c".into(),
                blob: Some("2".into()),
            },
            Cambio {
                ruta: "d".into(),
                blob: None,
            },
        ];
        assert_eq!(blobs(&cs), vec!["1".to_string(), "2".to_string()]);
    }
}
