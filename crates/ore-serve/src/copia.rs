//! La decisión de la copia (0027 P1 I2): `POST /paquetes/{n}/vistas/{v}/copia`.
//!
//! # Por qué es un verbo y no lo escribe `discover`
//!
//! `ore discover` no propone `materialized` a propósito: que una vista tenga
//! copia es una decisión de operación con coste, y proponerla sería inventarla
//! (`inductor.rs`). Pero desde el ADR 0018 la copia no es un acelerador: es **el
//! sistema de registro** —donde una Propuesta aterriza— y sin ella no hay
//! inferencia sobre los datos. Así que la decisión necesita un sitio donde
//! tomarse y firmarse, y es éste: quien pulsa decide, y el commit lleva su `sub`.
//!
//! # Lo que decide, y es más de un campo
//!
//! Medido antes de escribirlo (2026-09-17, sobre `demo/olist`): un `materialized`
//! solo **no compila** (`OOS4011`: el conducto `materialization.payload` no tiene
//! autorización declarada). La decisión son tres escrituras coherentes, o nada:
//!
//! | | dónde | qué |
//! |---|---|---|
//! | la copia | la vista | `materialized: { datasource: <la de su tabla raíz>, table: "copia.<vista>" }` |
//! | la clave (si se pide) | la tabla raíz | `changes.key: [columnas]` y `mode: upsert` — la identidad de la fila que F5 necesita |
//! | el conducto | `conduits.yaml` en la raíz | `materialization.payload` autorizado (vacío si el árbol no tiene retículos) |
//!
//! Y después `ore validate .`: si no compila, **nada se escribe** (422 con los
//! diagnósticos). La clave de la ENTIDAD (`primaryKey`) no se toca aquí: es la
//! decisión `clave` de `ore review`, y sigue abierta donde estaba.
//!
//! # Lo que no hace
//!
//! No copia nada: eso es el Job `copiar-<paquete>` (I3), que lee esta
//! declaración. No elige qué gana cuando el origen y la copia se contradicen
//! (functions.md §7.4): es de F5.
use crate::cola;
use crate::rutas::{Servidor, analizar, de_node, token};
use ore_core::json::Json;
use ore_core::parse::{self, Node};
use ore_entrada::http::Respuesta;
use ore_entrada::identidad::Identidad;
use std::path::{Path, PathBuf};

fn campo(n: &Node, k: &str) -> Option<String> {
    n.get(k).and_then(|(_, v)| v.as_str()).map(str::to_string)
}

/// El fichero de un documento `kind` con ese nombre, bajo `packages/<n>/<dir>/`.
fn fichero_de(dir: &Path, kind: &str, nombre: &str) -> Option<(PathBuf, String, Node)> {
    let es = std::fs::read_dir(dir).ok()?;
    let mut rutas: Vec<PathBuf> = es
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "yaml"))
        .collect();
    rutas.sort();
    for p in rutas {
        let Ok(texto) = std::fs::read_to_string(&p) else {
            continue;
        };
        let Ok(n) = parse::parse(&texto) else {
            continue;
        };
        if campo(&n, "kind").as_deref() != Some(kind) {
            continue;
        }
        let meta = n.get("metadata").map(|(_, m)| m);
        if meta.and_then(|m| campo(m, "name")).as_deref() == Some(nombre) {
            return Some((p, texto, n));
        }
    }
    None
}

/// Lo que se escribió, para deshacerlo si al final no compila (sobre un
/// directorio no hay clon que tirar).
struct Escrito {
    ruta: PathBuf,
    antes: Option<String>,
}

fn deshacer(escritos: &[Escrito]) {
    for e in escritos.iter().rev() {
        match &e.antes {
            Some(t) => {
                let _ = std::fs::write(&e.ruta, t);
            }
            None => {
                let _ = std::fs::remove_file(&e.ruta);
            }
        }
    }
}

impl Servidor {
    /// `POST /paquetes/{n}/vistas/{v}/copia {key?: [campos]}`.
    pub(crate) fn decidir_copia(
        &self,
        raiz: &Path,
        paquete: &str,
        vista: &str,
        cuerpo: &str,
        sujeto: &Identidad,
    ) -> Respuesta {
        if let Err(m) = token(paquete) {
            return Respuesta::error(422, format!("nombre de paquete: {m}"));
        }
        if let Err(m) = token(vista) {
            return Respuesta::error(422, format!("nombre de vista: {m}"));
        }
        // sin cuerpo = sin clave: la decisión mínima
        let cuerpo = match analizar(if cuerpo.trim().is_empty() {
            "{}"
        } else {
            cuerpo
        }) {
            Ok(n) => n,
            Err(r) => return r,
        };
        let clave: Vec<String> = cuerpo
            .get("key")
            .map(|(_, k)| {
                k.items()
                    .iter()
                    .filter_map(|i| i.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let dir = raiz.join("packages").join(paquete);
        if !dir.is_dir() {
            return Respuesta::error(404, "no hay tal paquete");
        }
        // ── la vista ────────────────────────────────────────────────────────
        let Some((ruta_vista, texto_vista, v)) = fichero_de(&dir.join("views"), "View", vista)
        else {
            return Respuesta::error(404, format!("no hay la vista `{paquete}.{vista}`"));
        };
        let Some((_, spec)) = v.get("spec") else {
            return Respuesta::error(422, "la vista no tiene `spec`");
        };
        if spec.get("materialized").is_some() {
            return Respuesta::error(
                409,
                format!("`{paquete}.{vista}` ya declara `materialized`: la decisión está tomada"),
            );
        }
        let Some((_, from)) = spec.get("from") else {
            return Respuesta::error(422, "la vista no tiene `from`");
        };
        let Some(tabla) = campo(from, "table") else {
            return Respuesta::error(
                422,
                "la copia se declara sobre una vista que lee UNA tabla (`from.table`); una vista sobre otra vista hereda la copia de la de abajo",
            );
        };
        let tabla_corta = tabla.rsplit('.').next().unwrap_or(&tabla).to_string();
        let Some((ruta_tabla, texto_tabla, t)) =
            fichero_de(&dir.join("tables"), "Table", &tabla_corta)
        else {
            return Respuesta::error(
                422,
                format!("la vista lee `{tabla}` y no hay tal tabla en el paquete"),
            );
        };
        let Some((_, tspec)) = t.get("spec") else {
            return Respuesta::error(422, format!("la tabla `{tabla}` no tiene `spec`"));
        };
        let Some(fuente) = campo(tspec, "datasource") else {
            return Respuesta::error(422, format!("la tabla `{tabla}` no dice su `datasource`"));
        };
        // la clave se pide por CAMPOS de la vista; en la tabla van sus columnas
        let mut columnas = Vec::new();
        for c in &clave {
            let Some(col) = spec
                .get("fields")
                .and_then(|(_, f)| f.get(c))
                .and_then(|(_, v)| v.as_str().map(String::from))
            else {
                let hay: Vec<String> = spec
                    .get("fields")
                    .map(|(_, f)| {
                        f.entries()
                            .iter()
                            .filter_map(|(k, _)| k.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                return Respuesta::error(
                    422,
                    format!(
                        "`key` nombra `{c}`, que no es un campo de `{paquete}.{vista}`. Los campos: {}",
                        hay.join(", ")
                    ),
                );
            };
            columnas.push(col);
        }

        let mut escritos = Vec::new();
        // ── ① la vista gana `materialized` ──────────────────────────────────
        let mut nuevo = texto_vista.trim_end().to_string();
        nuevo.push_str(&format!(
            "\n  materialized: {{ datasource: {fuente}, table: \"copia.{vista}\" }}\n"
        ));
        if let Err(e) = std::fs::write(&ruta_vista, &nuevo) {
            return Respuesta::error(500, format!("no se pudo escribir la vista: {e}"));
        }
        escritos.push(Escrito {
            ruta: ruta_vista.clone(),
            antes: Some(texto_vista),
        });
        // ── ② la tabla gana la clave, si se pidió ───────────────────────────
        if !columnas.is_empty() {
            let lista = format!("[{}]", columnas.join(", "));
            let nuevo = match reescribir_changes(&texto_tabla, &lista) {
                Some(t) => t,
                None => {
                    deshacer(&escritos);
                    return Respuesta::error(
                        422,
                        format!(
                            "la tabla `{tabla}` no tiene un bloque `changes` con la forma que el inductor escribe (`mode: …` en su primera línea); la clave se pone a mano"
                        ),
                    );
                }
            };
            if let Err(e) = std::fs::write(&ruta_tabla, &nuevo) {
                deshacer(&escritos);
                return Respuesta::error(500, format!("no se pudo escribir la tabla: {e}"));
            }
            escritos.push(Escrito {
                ruta: ruta_tabla,
                antes: Some(texto_tabla),
            });
        }
        // ── ③ el conducto, autorizado ───────────────────────────────────────
        let conduits = raiz.join("conduits.yaml");
        match std::fs::read_to_string(&conduits) {
            Ok(t) => {
                if !t.contains("materialization.payload") {
                    let nuevo = format!("{}\n    materialization.payload: {{}}\n", t.trim_end());
                    if let Err(e) = std::fs::write(&conduits, &nuevo) {
                        deshacer(&escritos);
                        return Respuesta::error(
                            500,
                            format!("no se pudo escribir `conduits.yaml`: {e}"),
                        );
                    }
                    escritos.push(Escrito {
                        ruta: conduits,
                        antes: Some(t),
                    });
                }
            }
            Err(_) => {
                let owner = std::fs::read_to_string(dir.join("package.yaml"))
                    .ok()
                    .and_then(|t| parse::parse(&t).ok())
                    .and_then(|n| n.get("spec").and_then(|(_, s)| campo(s, "owner")))
                    .unwrap_or_else(|| "team:datos".into());
                let texto = format!(
                    "apiVersion: oos.dev/v1alpha1\nkind: ConduitPolicy\nmetadata: {{ name: {paquete} }}\nspec:\n  owner: {owner}\n  conduits:\n    # 0027 P1: la copia de las vistas que lo declaren. Sin retículos, la\n    # autorización es vacía; con ellos, aquí se dice hasta qué etiqueta.\n    materialization.payload: {{}}\n"
                );
                if let Err(e) = std::fs::write(&conduits, &texto) {
                    deshacer(&escritos);
                    return Respuesta::error(
                        500,
                        format!("no se pudo escribir `conduits.yaml`: {e}"),
                    );
                }
                escritos.push(Escrito {
                    ruta: conduits,
                    antes: None,
                });
            }
        }
        // ── y compila, o nada ───────────────────────────────────────────────
        if let Some(r) = self.no_compila(raiz) {
            deshacer(&escritos);
            return r;
        }
        // ── y el Job de la copia, en la cola, en el mismo acto (I3) ─────────
        //
        // Con TODAS las vistas del árbol que declaran copia, no sólo ésta: el
        // Job las materializa juntas y su nombre lleva el resumen de la lista.
        // Es la misma figura que el catálogo (`encolar_catalogo`).
        let todas = vistas_con_copia(raiz);
        let encolado = self.encolar_copia(&todas, sujeto);
        Respuesta::creado(Json::obj([
            ("encolado", Json::s(encolado)),
            ("package", Json::s(paquete)),
            ("view", Json::s(vista)),
            ("table", Json::s(tabla)),
            ("datasource", Json::s(fuente)),
            ("copia", Json::s(format!("copia.{vista}"))),
            (
                "key",
                Json::Arr(clave.iter().map(|k| Json::s(k.clone())).collect()),
            ),
            (
                "escritos",
                Json::Arr(
                    escritos
                        .iter()
                        .map(|e| {
                            Json::s(
                                e.ruta
                                    .strip_prefix(raiz)
                                    .unwrap_or(&e.ruta)
                                    .to_string_lossy()
                                    .replace('\\', "/"),
                            )
                        })
                        .collect(),
                ),
            ),
        ]))
    }

    /// `GET /paquetes/{n}/copias`: las vistas del paquete que declaran copia, con
    /// su tabla, su fuente y su clave. Lo que el Job de I3 va a copiar.
    pub(crate) fn copias(&self, raiz: &Path, paquete: &str) -> Respuesta {
        if let Err(m) = token(paquete) {
            return Respuesta::error(422, format!("nombre de paquete: {m}"));
        }
        let dir = raiz.join("packages").join(paquete);
        if !dir.is_dir() {
            return Respuesta::error(404, "no hay tal paquete");
        }
        let mut lista = Vec::new();
        if let Ok(es) = std::fs::read_dir(dir.join("views")) {
            let mut rutas: Vec<PathBuf> = es.flatten().map(|e| e.path()).collect();
            rutas.sort();
            for p in rutas {
                let Ok(texto) = std::fs::read_to_string(&p) else {
                    continue;
                };
                let Ok(n) = parse::parse(&texto) else {
                    continue;
                };
                if campo(&n, "kind").as_deref() != Some("View") {
                    continue;
                }
                let Some((_, spec)) = n.get("spec") else {
                    continue;
                };
                let Some((_, mat)) = spec.get("materialized") else {
                    continue;
                };
                let nombre = n
                    .get("metadata")
                    .and_then(|(_, m)| campo(m, "name"))
                    .unwrap_or_default();
                let tabla = spec.get("from").and_then(|(_, f)| campo(f, "table"));
                let clave = tabla
                    .as_deref()
                    .and_then(|t| {
                        fichero_de(
                            &dir.join("tables"),
                            "Table",
                            t.rsplit('.').next().unwrap_or(t),
                        )
                    })
                    .and_then(|(_, _, tn)| {
                        tn.get("spec")
                            .and_then(|(_, s)| s.get("changes"))
                            .and_then(|(_, c)| c.get("key"))
                            .map(|(_, k)| de_node(k))
                    })
                    .unwrap_or(Json::Arr(Vec::new()));
                lista.push(Json::obj([
                    ("view", Json::s(nombre.clone())),
                    ("table", tabla.map(Json::s).unwrap_or(Json::Bool(false))),
                    ("materialized", de_node(mat)),
                    ("key", clave),
                    ("copia", informe_de(raiz, paquete, &nombre)),
                ]));
            }
        }
        Respuesta::ok(Json::obj([("copias", Json::Arr(lista))]))
    }

    /// El Job de la copia a la cola de trabajo, rendido de la plantilla que el
    /// aprovisionador dejó allí. Devuelve una frase que dice qué pasó — nunca
    /// tumba la decisión, que ya está escrita.
    fn encolar_copia(&self, vistas: &[String], sujeto: &Identidad) -> String {
        let Some(forja) = &self.cola else {
            return "NO encolado: este servidor no sabe de ninguna cola (`--cola`); lo rendirá la convergencia".into();
        };
        let prestado = match forja.clonar() {
            Ok(p) => p,
            Err(e) => return format!("NO encolado: {e}"),
        };
        let dir = prestado.ruta();
        let plantilla = match std::fs::read_to_string(dir.join(cola::PLANTILLA_COPIA)) {
            Ok(t) => t,
            Err(_) => {
                return format!(
                    "NO encolado: la cola no trae `{}`; hay que converger este inquilino",
                    cola::PLANTILLA_COPIA
                );
            }
        };
        let (fichero, texto) = match cola::rendir_copia(&plantilla, vistas) {
            Ok(v) => v,
            Err(e) => return format!("NO encolado: {e}"),
        };
        if let Err(e) = std::fs::write(dir.join(&fichero), &texto) {
            return format!("NO encolado: no se pudo escribir `{fichero}`: {e}");
        }
        if !forja.hay_cambios(dir) {
            return format!("ya encolado como `{fichero}`");
        }
        match forja.publicar(dir, sujeto, &format!("Copiar {}", vistas.join(", "))) {
            Ok(c) => format!("encolado como `{fichero}` · commit {c}"),
            Err(e) => format!("NO encolado: {e}"),
        }
    }
}

/// `paquete.vista` de cada vista del árbol que declara `materialized`, en orden.
fn vistas_con_copia(raiz: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(paquetes) = std::fs::read_dir(raiz.join("packages")) else {
        return out;
    };
    let mut dirs: Vec<PathBuf> = paquetes
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    for d in dirs {
        let paquete = d
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let Ok(vistas) = std::fs::read_dir(d.join("views")) else {
            continue;
        };
        let mut rutas: Vec<PathBuf> = vistas.flatten().map(|e| e.path()).collect();
        rutas.sort();
        for p in rutas {
            let Ok(texto) = std::fs::read_to_string(&p) else {
                continue;
            };
            let Ok(n) = parse::parse(&texto) else {
                continue;
            };
            if campo(&n, "kind").as_deref() != Some("View") {
                continue;
            }
            if n.get("spec")
                .and_then(|(_, s)| s.get("materialized"))
                .is_none()
            {
                continue;
            }
            if let Some(v) = n.get("metadata").and_then(|(_, m)| campo(m, "name")) {
                out.push(format!("{paquete}.{v}"));
            }
        }
    }
    out
}

/// Lo que la última pasada del Job dejó en `copias/<paquete>_<vista>.json`, con
/// quién y cuándo (el commit). Sin informe: `pendiente` — se decidió y nadie ha
/// copiado todavía.
fn informe_de(raiz: &Path, paquete: &str, vista: &str) -> Json {
    let rel = format!("copias/{paquete}_{vista}.json");
    let Ok(texto) = std::fs::read_to_string(raiz.join(&rel)) else {
        return Json::obj([("estado", Json::s("pendiente"))]);
    };
    let mut j = match parse::parse(&texto).map(|n| de_node(&n)) {
        Ok(Json::Obj(m)) => m,
        _ => return Json::obj([("estado", Json::s("ilegible")), ("fichero", Json::s(rel))]),
    };
    if let Some((quien, cuando)) = commit_de(raiz, &rel) {
        j.insert("copiado_por".into(), Json::s(quien));
        j.insert("cuando".into(), Json::s(cuando));
    }
    Json::Obj(j)
}

fn commit_de(raiz: &Path, rel: &str) -> Option<(String, String)> {
    let s = std::process::Command::new("git")
        .current_dir(raiz)
        .args(["log", "-1", "--format=%an%x1f%aI", "--", rel])
        .output()
        .ok()?;
    if !s.status.success() {
        return None;
    }
    let texto = String::from_utf8_lossy(&s.stdout);
    let (a, b) = texto.trim().split_once('\u{1f}')?;
    if a.is_empty() {
        return None;
    }
    Some((a.to_string(), b.to_string()))
}

/// `changes:` con la forma del inductor (`mode: <x>` en su primera línea) →
/// `mode: upsert` y `key: [...]` justo debajo. Si ya había `key`, se sustituye.
fn reescribir_changes(texto: &str, lista: &str) -> Option<String> {
    let mut out = Vec::new();
    let mut en_changes = false;
    let mut hecho = false;
    let mut sangria = String::new();
    for l in texto.lines() {
        let t = l.trim_start();
        if !en_changes {
            out.push(l.to_string());
            if t.starts_with("changes:") && t.trim_end() == "changes:" {
                en_changes = true;
            }
            continue;
        }
        let s = &l[..l.len() - t.len()];
        if sangria.is_empty() {
            sangria = s.to_string();
        }
        if s.len() < sangria.len() || t.is_empty() {
            // salimos del bloque sin haber visto `mode`: no es la forma que se sabe reescribir
            if !hecho {
                return None;
            }
            en_changes = false;
            out.push(l.to_string());
            continue;
        }
        if t.starts_with("key:") {
            continue; // la sustituye la de abajo
        }
        if t.starts_with("mode:") {
            out.push(format!("{sangria}mode: upsert"));
            out.push(format!("{sangria}key: {lista}"));
            hecho = true;
            continue;
        }
        out.push(l.to_string());
    }
    if !hecho {
        return None;
    }
    let mut s = out.join("\n");
    s.push('\n');
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_clave_entra_en_el_bloque_changes_del_inductor() {
        let t = "spec:\n  datasource: pg\n  changes:\n    mode: append\n    witness: log\n";
        let r = reescribir_changes(t, "[customer_id]").unwrap();
        assert_eq!(
            r,
            "spec:\n  datasource: pg\n  changes:\n    mode: upsert\n    key: [customer_id]\n    witness: log\n"
        );
        // una clave previa se sustituye, no se duplica
        let r2 = reescribir_changes(&r, "[a, b]").unwrap();
        assert!(
            r2.contains("key: [a, b]") && !r2.contains("customer_id"),
            "{r2}"
        );
        // en línea (`changes: { … }`) no se sabe reescribir, y se dice
        assert!(reescribir_changes("spec:\n  changes: { mode: append }\n", "[x]").is_none());
    }
}
