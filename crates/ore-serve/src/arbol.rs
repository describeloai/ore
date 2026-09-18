//! El árbol por ruta (ADR 0030, W0): `GET /arbol` · `GET /arbol/diagnosticos` ·
//! `GET|PUT|DELETE /arbol/{ruta}`.
//!
//! # Por qué existe, si ya hay `/documentos/{kind}`
//!
//! Porque el code workspace abre **el árbol de la celda** tal como es —un
//! fichero por su ruta, un índice con todo— y no una carpeta por kind. Lo que
//! `documentos.rs` sabe de escribir (compilar antes de empujar, no empeorar, el
//! commit del sujeto, 409 si el árbol se movió) vale igual para una ruta: es la
//! misma figura con un nombre menos particular. `PUT /documentos/…` queda como
//! el caso especial que pone el nombre desde la ruta.
//!
//! # Lo que una ruta puede ser
//!
//! Relativa a la raíz, sin `..`, sin `.git/`, y **no** lo gobernado que induce
//! `review` (`discover.*.json`): eso lo escribe el verbo que decide, no un
//! editor. Un fichero que no es YAML también se sirve —`conduits.yaml` lo es;
//! un `README.md` no— porque el árbol es del cliente entero.
//!
//! # Guardar es compilar
//!
//! `PUT` devuelve **los diagnósticos del árbol tras el guardado**, con la
//! posición separada (`fichero`, `linea`, `columna`) para que el editor los
//! pinte donde tocan. Un guardado que empeora es 422 con sólo los nuevos y no
//! se publica; el fichero queda como estaba.

use crate::documentos::{cabeza_de, commit_de, git, relativo};
use crate::rutas::Servidor;
use ore_core::json::Json;
use ore_entrada::http::Respuesta;
use std::path::{Path, PathBuf};

/// La ruta, comprobada: relativa, dentro del árbol, y no lo que se induce.
fn ruta_valida(ruta: &str) -> Result<PathBuf, Respuesta> {
    let limpia = ruta.trim().trim_start_matches('/');
    if limpia.is_empty() {
        return Err(Respuesta::error(422, "la ruta está vacía"));
    }
    if limpia.contains('\\') || limpia.contains("//") {
        return Err(Respuesta::error(422, "la ruta va con `/` y sin dobles"));
    }
    for seg in limpia.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." {
            return Err(Respuesta::error(
                422,
                format!("`{ruta}` no es una ruta dentro del árbol"),
            ));
        }
        if seg == ".git" {
            return Err(Respuesta::error(
                422,
                "`.git/` no es del árbol: es su historia",
            ));
        }
        if !seg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        {
            return Err(Respuesta::error(
                422,
                format!("`{seg}`: un nombre de fichero lleva letras, dígitos, `.`, `_` y `-`"),
            ));
        }
    }
    if let Some(nombre) = limpia.rsplit('/').next()
        && nombre.starts_with("discover.")
        && nombre.ends_with(".json")
    {
        return Err(Respuesta::error(
            422,
            format!("`{nombre}` lo induce `review` (0027): se decide con los verbos, no se edita"),
        ));
    }
    Ok(PathBuf::from(limpia))
}

/// El kind de un YAML, mirando sólo su cabeza.
fn kind_de(texto: &str) -> Option<String> {
    texto
        .lines()
        .take(12)
        .find_map(|l| l.strip_prefix("kind:").map(|k| k.trim().to_string()))
        .filter(|k| !k.is_empty())
}

/// Un diagnóstico de `diagnosticos_de` con la posición separada, para que el
/// editor lo pinte: `donde` es `fichero:línea:columna` (línea y columna
/// opcionales).
pub(crate) fn con_posicion(d: &Json) -> Json {
    let Json::Obj(m) = d else { return d.clone() };
    let mut m = m.clone();
    let donde = match m.get("donde") {
        Some(Json::Str(s)) => s.clone(),
        _ => String::new(),
    };
    let mut partes = donde.rsplitn(3, ':');
    let (c, l, f) = (partes.next(), partes.next(), partes.next());
    let (fichero, linea, columna) = match (f, l, c) {
        (Some(f), Some(l), Some(c)) if l.parse::<i64>().is_ok() && c.parse::<i64>().is_ok() => {
            (f.to_string(), l.parse().ok(), c.parse().ok())
        }
        _ => (donde.clone(), None, None),
    };
    if !fichero.is_empty() {
        m.insert("fichero".into(), Json::s(fichero));
    }
    if let Some(l) = linea {
        m.insert("linea".into(), Json::Int(l));
    }
    if let Some(c) = columna {
        m.insert("columna".into(), Json::Int(c));
    }
    // Hoy `ore` sólo emite errores; el día que emita avisos vendrán marcados.
    m.entry("severidad".to_string())
        .or_insert_with(|| Json::s("error"));
    Json::Obj(m)
}

/// `GET /arbol`: el índice, de `git ls-files`, con el kind de cada YAML.
pub(crate) fn indice(raiz: &Path) -> Respuesta {
    let Some(lista) = git(raiz, &["ls-files"]) else {
        return Respuesta::error(500, "no se pudo listar el árbol");
    };
    let mut ficheros = Vec::new();
    for rel in lista.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let p = raiz.join(rel);
        let bytes = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        let mut f = vec![("ruta", Json::s(rel)), ("bytes", Json::Int(bytes as i64))];
        if rel.ends_with(".yaml") || rel.ends_with(".yml") {
            let cabeza = std::fs::read(&p)
                .ok()
                .map(|b| String::from_utf8_lossy(&b[..b.len().min(600)]).into_owned())
                .unwrap_or_default();
            if let Some(k) = kind_de(&cabeza) {
                f.push(("kind", Json::s(k)));
            }
        }
        ficheros.push(Json::obj(f));
    }
    Respuesta::ok(Json::obj([
        (
            "cabeza",
            cabeza_de(raiz).map(Json::s).unwrap_or(Json::Bool(false)),
        ),
        ("ficheros", Json::Arr(ficheros)),
    ]))
}

/// `GET /arbol/{ruta}`: el texto y el commit que lo trajo.
pub(crate) fn leer(raiz: &Path, ruta: &str) -> Respuesta {
    let rel = match ruta_valida(ruta) {
        Ok(r) => r,
        Err(r) => return r,
    };
    let p = raiz.join(&rel);
    let Ok(texto) = std::fs::read_to_string(&p) else {
        return Respuesta::error(404, format!("no hay `{ruta}` en el árbol"));
    };
    Respuesta::ok(Json::obj([
        ("ruta", Json::s(relativo(raiz, &p))),
        (
            "kind",
            kind_de(&texto).map(Json::s).unwrap_or(Json::Bool(false)),
        ),
        ("texto", Json::s(&texto)),
        ("commit", commit_de(raiz, &p).unwrap_or(Json::Bool(false))),
        (
            "cabeza",
            cabeza_de(raiz).map(Json::s).unwrap_or(Json::Bool(false)),
        ),
    ]))
}

impl Servidor {
    /// `GET /arbol/diagnosticos`: lo que el árbol dice tal como está.
    pub(crate) fn diagnosticos_del_arbol(&self, raiz: &Path) -> Respuesta {
        match self.diagnosticos_de(raiz) {
            Ok(d) => Respuesta::ok(Json::obj([
                (
                    "cabeza",
                    cabeza_de(raiz).map(Json::s).unwrap_or(Json::Bool(false)),
                ),
                (
                    "diagnosticos",
                    Json::Arr(d.iter().map(con_posicion).collect()),
                ),
            ])),
            Err(r) => r,
        }
    }

    /// `PUT /arbol/{ruta}` con el texto del fichero como cuerpo. 201 si es
    /// nuevo, 200 si se reescribe, 422 con los diagnósticos nuevos si el
    /// árbol empeora (y nada se escribe), 409 si el árbol se movió
    /// (`If-Match`). Siempre con los diagnósticos del árbol tras el guardado.
    pub(crate) fn escribir_fichero(
        &self,
        raiz: &Path,
        ruta: &str,
        texto: &str,
        si_commit: Option<&str>,
    ) -> Respuesta {
        let rel = match ruta_valida(ruta) {
            Ok(r) => r,
            Err(r) => return r,
        };
        if texto.len() > 2 * 1024 * 1024 {
            return Respuesta::error(413, "un fichero del árbol no pasa de 2 MB");
        }
        if let Some(r) = self.arbol_se_movio(raiz, si_commit) {
            return r;
        }
        let p = raiz.join(&rel);
        let existente = std::fs::read_to_string(&p).ok();
        // el mismo texto (con los finales de línea que sean) no es un cambio
        if existente.as_deref().map(|e| e.replace("\r\n", "\n"))
            == Some(texto.replace("\r\n", "\n"))
        {
            return Respuesta::ok(Json::obj([
                ("ruta", Json::s(relativo(raiz, &p))),
                ("nueva", Json::Bool(false)),
                ("igual", Json::Bool(true)),
            ]));
        }
        let antes = match self.diagnosticos_de(raiz) {
            Ok(a) => a,
            Err(r) => return r,
        };
        if let Some(dir) = p.parent()
            && let Err(e) = std::fs::create_dir_all(dir)
        {
            return Respuesta::error(
                500,
                format!("no se pudo crear `{}`: {e}", relativo(raiz, dir)),
            );
        }
        if let Err(e) = std::fs::write(&p, texto) {
            return Respuesta::error(500, format!("no se pudo escribir `{ruta}`: {e}"));
        }
        // ── compilar antes de empujar: ¿empeora? ────────────────────────────
        if let Err(mut r) = self.empeora(raiz, &antes, &format!("`{ruta}`")) {
            match &existente {
                Some(t) => {
                    let _ = std::fs::write(&p, t);
                }
                None => {
                    let _ = std::fs::remove_file(&p);
                }
            }
            // los diagnósticos nuevos, con posición: es lo que el editor pinta
            if let Json::Obj(m) = &mut r.cuerpo
                && let Some(Json::Arr(ds)) = m.get("diagnosticos").cloned()
            {
                m.insert(
                    "diagnosticos".into(),
                    Json::Arr(ds.iter().map(con_posicion).collect()),
                );
            }
            return r;
        }
        let despues = self.diagnosticos_de(raiz).unwrap_or_default();
        let ficha = Json::obj([
            ("ruta", Json::s(relativo(raiz, &p))),
            (
                "kind",
                kind_de(texto).map(Json::s).unwrap_or(Json::Bool(false)),
            ),
            ("nueva", Json::Bool(existente.is_none())),
            (
                "diagnosticos",
                Json::Arr(despues.iter().map(con_posicion).collect()),
            ),
        ]);
        if existente.is_none() {
            Respuesta::creado(ficha)
        } else {
            Respuesta::ok(ficha)
        }
    }

    /// `DELETE /arbol/{ruta}`: fuera si el árbol no empeora.
    pub(crate) fn retirar_fichero(
        &self,
        raiz: &Path,
        ruta: &str,
        si_commit: Option<&str>,
    ) -> Respuesta {
        let rel = match ruta_valida(ruta) {
            Ok(r) => r,
            Err(r) => return r,
        };
        let p = raiz.join(&rel);
        let Ok(texto) = std::fs::read_to_string(&p) else {
            return Respuesta::error(404, format!("no hay `{ruta}` en el árbol"));
        };
        if let Some(r) = self.arbol_se_movio(raiz, si_commit) {
            return r;
        }
        let antes = match self.diagnosticos_de(raiz) {
            Ok(a) => a,
            Err(r) => return r,
        };
        if let Err(e) = std::fs::remove_file(&p) {
            return Respuesta::error(500, format!("no se pudo retirar `{ruta}`: {e}"));
        }
        if let Err(mut r) = self.empeora(raiz, &antes, &format!("retirar `{ruta}`")) {
            let _ = std::fs::write(&p, &texto);
            if let Json::Obj(m) = &mut r.cuerpo
                && let Some(Json::Arr(ds)) = m.get("diagnosticos").cloned()
            {
                m.insert(
                    "diagnosticos".into(),
                    Json::Arr(ds.iter().map(con_posicion).collect()),
                );
            }
            return r;
        }
        Respuesta::ok(Json::obj([
            ("ruta", Json::s(relativo(raiz, &p))),
            ("retirado", Json::Bool(true)),
        ]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_ruta_se_comprueba() {
        assert!(ruta_valida("packages/olist/views/customers.yaml").is_ok());
        assert!(
            ruta_valida("/conduits.yaml").is_ok(),
            "la barra inicial se tolera"
        );
        for mala in [
            "",
            "..",
            "packages/../x.yaml",
            ".git/config",
            "a//b",
            "a\\b",
            "packages/olist/discover.scope.json",
            "con espacio.yaml",
        ] {
            assert!(ruta_valida(mala).is_err(), "{mala}");
        }
    }

    #[test]
    fn el_diagnostico_gana_posicion() {
        let d = Json::obj([
            ("codigo", Json::s("OOS2022")),
            ("mensaje", Json::s("x")),
            ("donde", Json::s("packages/hr/entities/Employee.yaml:111:5")),
        ]);
        let Json::Obj(m) = con_posicion(&d) else {
            panic!()
        };
        assert_eq!(
            m.get("fichero"),
            Some(&Json::s("packages/hr/entities/Employee.yaml"))
        );
        assert_eq!(m.get("linea"), Some(&Json::Int(111)));
        assert_eq!(m.get("columna"), Some(&Json::Int(5)));
        assert_eq!(m.get("severidad"), Some(&Json::s("error")));
        let sin = Json::obj([
            ("codigo", Json::s("OOS1004")),
            ("donde", Json::s("packages/x/f.yaml")),
        ]);
        let Json::Obj(m) = con_posicion(&sin) else {
            panic!()
        };
        assert_eq!(m.get("fichero"), Some(&Json::s("packages/x/f.yaml")));
        assert!(!m.contains_key("linea"));
    }

    #[test]
    fn el_kind_sale_de_la_cabeza() {
        assert_eq!(
            kind_de("apiVersion: x\nkind: View\nmetadata: {}\n").as_deref(),
            Some("View")
        );
        assert_eq!(kind_de("# nada\n"), None);
    }
}
