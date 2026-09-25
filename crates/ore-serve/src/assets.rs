//! **`GET /assets`** — el índice de assets (0034 ⑤): el árbol compilado a una
//! cabeza, proyectado por `ore_core::assets::indice`, y servido **de memoria
//! por commit**.
//!
//! | ruta | qué |
//! |---|---|
//! | `GET /assets` | el índice de la rama por defecto; con la cabecera de rama (0030 W2, como `/arbol`), el de esa rama: lo que un workspace propone, antes de fusionar |
//! | `GET /assets/{commit}` | el catálogo como era. Por el camino y no por la consulta: **ningún dato entra por la URL** más allá de lo que ya viaja en `/arbol/version/{hash}` |
//!
//! # La caché por cabeza
//!
//! Medido (0034 «Lo medido»): cada petición al árbol cuesta lo mismo —0,5 s en
//! demo, 0,9 s en victor— porque es **clonar la forja**, no el tamaño. Aquí el
//! clon se hace **una vez por cabeza**: antes de clonar se pregunta a la forja
//! qué commit tiene la rama (`git ls-remote`, milisegundos), y si es el que ya
//! está en memoria se sirve sin tocar el árbol. Un push cambia la cabeza y el
//! siguiente `GET` recalcula. Nada persiste: la caché son las últimas
//! [`CABEZAS`] por proceso, y un directorio (el banco, los tests) no se cachea.
//!
//! # `version`, por fichero
//!
//! El núcleo deja `version: null` (no sabe de git). Aquí, en el clon que ya se
//! tiene, un `git log -1` por fichero rellena `{commit, cuando, sujeto}`. Se
//! mide en el paso 3 del brief.
use crate::rutas::{Arbol, Servidor};
use ore_core::assets::Cabeza;
use ore_core::json::Json;
use ore_entrada::http::Respuesta;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Cuántas cabezas se recuerdan (por proceso, entre todas las ramas).
pub(crate) const CABEZAS: usize = 4;

/// `(rama, cabeza)`: lo que identifica un índice.
type Clave = (String, String);

/// La caché: `(rama, cabeza) → índice`, las últimas [`CABEZAS`].
#[derive(Default)]
pub(crate) struct Cache {
    entradas: Mutex<VecDeque<(Clave, Arc<Json>)>>,
}

impl std::fmt::Debug for Cache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Cache")
    }
}

impl Cache {
    fn lee(&self, clave: &Clave) -> Option<Arc<Json>> {
        let e = self.entradas.lock().ok()?;
        e.iter().find(|(k, _)| k == clave).map(|(_, v)| v.clone())
    }

    fn guarda(&self, clave: Clave, j: Arc<Json>) {
        if let Ok(mut e) = self.entradas.lock() {
            e.retain(|(k, _)| *k != clave);
            e.push_back((clave, j));
            while e.len() > CABEZAS {
                e.pop_front();
            }
        }
    }
}

/// El índice de un árbol en disco: los punteros de `datasets/`, el paquete
/// compilado, la proyección, y `version` por fichero si hay historia.
pub(crate) fn indice_de(raiz: &Path, cabeza: Cabeza) -> Json {
    let (pkg, _) = ore_core::validate::cargar_paquete(raiz);
    let punteros = ore_core::punteros::del_arbol(raiz);
    let mut j = ore_core::assets::indice(&pkg, &punteros, &cabeza);
    // `version` por fichero, sólo si el árbol tiene historia. Un proyecto
    // (0035 ①) tiene ruta como cualquier otra cosa —su manifiesto—, así que
    // quién lo creó y cuándo sale del mismo sitio y sin pedirle nada al árbol.
    if crate::documentos::cabeza_de(raiz).is_some()
        && let Json::Obj(m) = &mut j
    {
        if let Some(Json::Obj(items)) = m.get_mut("items") {
            for it in items.values_mut() {
                if let Json::Obj(it) = it
                    && let Some(Json::Str(ruta)) = it.get("ruta")
                    && let Some(v) = version_de(raiz, ruta)
                {
                    it.insert("version".into(), v);
                }
            }
        }
        if let Some(Json::Arr(ps)) = m.get_mut("proyectos") {
            for p in ps.iter_mut() {
                if let Json::Obj(p) = p
                    && let Some(Json::Str(ruta)) = p.get("ruta")
                    && let Some(v) = version_de(raiz, ruta)
                {
                    p.insert("version".into(), v);
                }
            }
        }
        // Un repositorio es una CARPETA, y `git log -1 -- <carpeta>` dice lo
        // mismo de ella que de un fichero: quien la tocó y cuándo (medido,
        // 0035 ⑥ §2). Es el «last edited by» de la lista, sin inventar nada.
        if let Some(Json::Arr(rs)) = m.get_mut("repositorios") {
            for r in rs.iter_mut() {
                if let Json::Obj(r) = r
                    && let Some(Json::Str(ruta)) = r.get("ruta")
                    && let Some(v) = version_de(raiz, ruta)
                {
                    r.insert("version".into(), v);
                }
            }
        }
    }
    j
}

/// `{commit, cuando, sujeto}` del último commit que tocó un fichero.
fn version_de(raiz: &Path, ruta: &str) -> Option<Json> {
    let s = crate::documentos::git(
        raiz,
        &["log", "-1", "--format=%h%x1f%aI%x1f%ae", "--", ruta],
    )?;
    let mut p = s.split('\u{1f}');
    let commit = p.next()?.trim().to_string();
    if commit.is_empty() {
        return None;
    }
    Some(Json::obj([
        ("commit", Json::s(commit)),
        ("cuando", Json::s(p.next().unwrap_or_default().trim())),
        (
            "sujeto",
            Json::s(
                p.next()
                    .unwrap_or_default()
                    .trim()
                    .trim_end_matches("@sujeto.invalid"),
            ),
        ),
    ]))
}

fn ahora() -> String {
    let s = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    // ISO 8601 sin dependencias: lo suficiente para saber cuándo.
    let (d, h) = (s / 86400, s % 86400);
    let (a, m, dia) = civil(d as i64);
    format!(
        "{a:04}-{m:02}-{dia:02}T{:02}:{:02}:{:02}Z",
        h / 3600,
        (h % 3600) / 60,
        h % 60
    )
}

/// Días desde la época → (año, mes, día). Howard Hinnant, `civil_from_days`.
fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

impl Servidor {
    /// `GET /assets` (la rama, por la cabecera) y `GET /assets/{commit}`.
    pub(crate) fn assets(&self, rama: Option<&str>, commit: Option<&str>) -> Respuesta {
        let rama = rama.map(str::to_string);
        let commit = commit.map(str::to_string);
        if let Some(r) = &rama
            && let Err(m) = crate::propuestas::nombre_de_rama_valido(r)
        {
            return Respuesta::error(422, m);
        }
        if let Some(c) = &commit
            && (c.len() < 4 || !c.chars().all(|x| x.is_ascii_hexdigit()))
        {
            return Respuesta::error(422, format!("`{c}` no es un commit"));
        }
        match &self.arbol {
            // Un directorio: sin caché, sin ramas.
            Arbol::Directorio(d) => {
                if rama.is_some() || commit.is_some() {
                    return Respuesta::error(
                        422,
                        "este árbol es un directorio, no una forja: no hay rama ni commit que leer",
                    );
                }
                let cabeza = Cabeza {
                    cabeza: crate::documentos::cabeza_de(d),
                    rama: None,
                    generado: Some(ahora()),
                };
                let mut j = indice_de(d, cabeza);
                if let Json::Obj(m) = &mut j {
                    m.insert("desde_cache".into(), Json::Bool(false));
                }
                Respuesta::ok(j)
            }
            Arbol::Forja(forja) => {
                let rama_nombre = rama.clone().unwrap_or_else(|| "main".to_string());
                // La cabeza, sin clonar: es lo que decide si se sirve de memoria.
                let cabeza = match &commit {
                    Some(c) => Some(c.clone()),
                    None => forja.cabeza_de(&rama_nombre),
                };
                let clave = cabeza.clone().map(|c| (rama_nombre.clone(), c));
                if let Some(k) = &clave
                    && let Some(j) = self.assets_cache.lee(k)
                {
                    let mut j = (*j).clone();
                    if let Json::Obj(m) = &mut j {
                        m.insert("desde_cache".into(), Json::Bool(true));
                    }
                    return Respuesta::ok(j);
                }
                let prestado = match forja.clonar_rama(rama.as_deref()) {
                    Err(crate::git::Fallo::SinRama(r)) => {
                        return Respuesta::error(404, format!("no hay ninguna rama `{r}`"));
                    }
                    Err(e) => return Respuesta::error(502, e.to_string()),
                    Ok(p) => p,
                };
                if let Some(c) = &commit
                    && crate::documentos::git(prestado.ruta(), &["checkout", "-q", c]).is_none()
                {
                    return Respuesta::error(
                        404,
                        format!("no hay ningún commit `{c}` en `{rama_nombre}`"),
                    );
                }
                let cabeza_real = crate::documentos::cabeza_de(prestado.ruta()).or(cabeza);
                let j = indice_de(
                    prestado.ruta(),
                    Cabeza {
                        cabeza: cabeza_real.clone(),
                        rama: Some(rama_nombre.clone()),
                        generado: Some(ahora()),
                    },
                );
                let j = Arc::new(j);
                if let Some(c) = cabeza_real {
                    self.assets_cache.guarda((rama_nombre, c), j.clone());
                }
                let mut j = (*j).clone();
                if let Json::Obj(m) = &mut j {
                    m.insert("desde_cache".into(), Json::Bool(false));
                }
                Respuesta::ok(j)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arbol() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("ore-assets-serve-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        for sub in ["packages/v/tables", "packages/v/datasets", "datasets"] {
            std::fs::create_dir_all(d.join(sub)).unwrap();
        }
        std::fs::write(
            d.join("ontology.config.yaml"),
            "apiVersion: oos.dev/v1alpha1\nkind: OntologyConfig\nmetadata: { name: t, version: 0.1.0 }\ndatasources:\n  - { name: pg, type: postgres, connectionEnv: PG_URL }\n",
        )
        .unwrap();
        std::fs::write(
            d.join("packages/v/package.yaml"),
            "apiVersion: oos.dev/v1alpha1\nkind: Package\nmetadata: { name: v, version: 0.1.0, status: active, domain: v }\nspec: { owner: team:v }\n",
        )
        .unwrap();
        std::fs::write(
            d.join("packages/v/tables/origen.yaml"),
            "apiVersion: oos.dev/v1alpha8\nkind: Table\nmetadata: { name: origen, namespace: v }\nspec:\n  datasource: pg\n  object: public.origen\n  columns: { id: { type: Integer } }\n  reads: { fullScan: cheap }\n  changes: { mode: append, witness: snapshot }\n",
        )
        .unwrap();
        std::fs::write(
            d.join("packages/v/datasets/pedidos.yaml"),
            "apiVersion: oos.dev/v1alpha12\nkind: Dataset\nmetadata: { name: pedidos, namespace: v }\nspec:\n  owner: team:v\n  from: { table: v.origen }\n",
        )
        .unwrap();
        std::fs::write(
            d.join("datasets/v_pedidos.json"),
            "{\"estado\": \"copiada\", \"filas\": 2, \"dataset\": \"datasets/v_pedidos\"}\n",
        )
        .unwrap();
        d
    }

    #[test]
    fn el_indice_de_un_directorio_lleva_el_puntero_y_sin_historia_no_lleva_version() {
        let d = arbol();
        let j = indice_de(&d, Cabeza::default());
        let Json::Obj(m) = &j else { panic!() };
        let Json::Obj(items) = &m["items"] else {
            panic!()
        };
        assert_eq!(items.len(), 2, "{:?}", items.keys().collect::<Vec<_>>());
        let Json::Obj(ds) = &items["dataset:v.pedidos"] else {
            panic!()
        };
        let Json::Obj(p) = &ds["puntero"] else {
            panic!()
        };
        assert_eq!(p["estado"], Json::s("copiada"));
        assert_eq!(ds["version"], Json::Crudo("null".into()));
        let Json::Obj(t) = &items["table:v.origen"] else {
            panic!()
        };
        let Json::Arr(r) = &t["relaciones"] else {
            panic!()
        };
        assert_eq!(r.len(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn la_cache_recuerda_las_ultimas_cabezas_y_olvida_la_mas_vieja() {
        let c = Cache::default();
        for i in 0..(CABEZAS + 1) {
            c.guarda(
                ("main".into(), format!("c{i}")),
                Arc::new(Json::Int(i as i64)),
            );
        }
        assert!(c.lee(&("main".into(), "c0".into())).is_none());
        assert_eq!(
            *c.lee(&("main".into(), format!("c{CABEZAS}"))).unwrap(),
            Json::Int(CABEZAS as i64)
        );
        // la misma clave otra vez no duplica
        c.guarda(("main".into(), "c1".into()), Arc::new(Json::Int(99)));
        assert_eq!(c.entradas.lock().unwrap().len(), CABEZAS);
    }

    #[test]
    fn la_fecha_civil_cuadra() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(20717), (2026, 9, 21));
    }
}
