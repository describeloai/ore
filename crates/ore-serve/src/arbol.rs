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

/// Todo lo que cuelga de un directorio, con sus bytes: lo que se va, y lo que
/// habría que devolver si la puerta dijera que no.
fn recoger(dir: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) -> std::io::Result<()> {
    for e in std::fs::read_dir(dir)? {
        let p = e?.path();
        if p.is_dir() {
            recoger(&p, out)?;
        } else {
            let bytes = std::fs::read(&p)?;
            out.push((p, bytes));
        }
    }
    Ok(())
}

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

/// **`GET /arbol`: el índice, de `git ls-files`, con el kind de cada YAML** — y,
/// desde 0036 ④, el de un alcance** (0036 ④): lo que el editor abre cuando está
/// dentro de un repositorio. `raiz_rel` es su carpeta
/// (`packages/<p>/<carpeta>`, de `X-Ore-Raiz`); sin ella, la celda entera,
/// como siempre.
///
/// Medido antes (0035 ⑥ §4): el editor pedía el árbol de la celda —24 ficheros
/// en acme-retail— para enseñar **uno** del repositorio. Acotar no es una
/// comodidad de pantalla: es dejar de traer lo que no es tuyo.
///
/// La **cabeza no cambia**: es la del árbol, porque el árbol es uno. Lo que se
/// acota es qué ficheros se listan, no de qué commit se habla.
pub(crate) fn indice_en(raiz: &Path, raiz_rel: Option<&str>) -> Respuesta {
    let Some(lista) = git(raiz, &["ls-files"]) else {
        return Respuesta::error(500, "no se pudo listar el árbol");
    };
    let dentro = raiz_rel
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| format!("{}/", s.trim_matches('/')));
    let mut ficheros = Vec::new();
    for rel in lista.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(p) = &dentro
            && !rel.starts_with(p.as_str())
        {
            continue;
        }
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
    let mut m = vec![
        (
            "cabeza",
            cabeza_de(raiz).map(Json::s).unwrap_or(Json::Bool(false)),
        ),
        ("ficheros", Json::Arr(ficheros)),
    ];
    if let Some(p) = &dentro {
        m.push(("raiz", Json::s(p.trim_end_matches('/'))));
    }
    Respuesta::ok(Json::obj(m))
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
    ///
    /// **Y una CARPETA también** (0035 ③b). Medido antes: en un árbol no hay
    /// carpetas vacías —git no las guarda—, así que una carpeta **es lo que
    /// tiene dentro**, y borrarla desde la consola era una racha de N llamadas
    /// con N commits, cualquiera de los cuales podía quedarse a medias. Aquí es
    /// **un commit**, pasa por la misma puerta («el árbol no empeora») y la
    /// respuesta dice **qué ficheros se llevó**: borrar sin decir qué es
    /// exactamente lo que nadie puede revisar después.
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
        if p.is_dir() {
            return self.retirar_carpeta(raiz, ruta, &p, si_commit);
        }
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

    /// Una carpeta entera, en un commit (0035 ③b). Lo que hay dentro se guarda
    /// en memoria para poder devolverlo si la puerta la rechaza: un `DELETE`
    /// que empeora el árbol **no deja nada a medias**.
    fn retirar_carpeta(
        &self,
        raiz: &Path,
        ruta: &str,
        dir: &Path,
        si_commit: Option<&str>,
    ) -> Respuesta {
        let mut dentro: Vec<(PathBuf, Vec<u8>)> = Vec::new();
        if let Err(e) = recoger(dir, &mut dentro) {
            return Respuesta::error(500, format!("no se pudo leer `{ruta}`: {e}"));
        }
        // Una carpeta sin ficheros no existe para git: no hay nada que retirar.
        if dentro.is_empty() {
            return Respuesta::error(404, format!("no hay `{ruta}` en el árbol"));
        }
        // Lo que `.git/` guarda no se toca, y `ruta_valida` ya lo negó arriba.
        if let Some(r) = self.arbol_se_movio(raiz, si_commit) {
            return r;
        }
        let antes = match self.diagnosticos_de(raiz) {
            Ok(a) => a,
            Err(r) => return r,
        };
        let mut ficheros: Vec<String> = dentro.iter().map(|(f, _)| relativo(raiz, f)).collect();
        ficheros.sort();
        if let Err(e) = std::fs::remove_dir_all(dir) {
            return Respuesta::error(500, format!("no se pudo retirar `{ruta}`: {e}"));
        }
        if let Err(mut r) = self.empeora(raiz, &antes, &format!("retirar `{ruta}/`")) {
            for (f, bytes) in &dentro {
                if let Some(padre) = f.parent() {
                    let _ = std::fs::create_dir_all(padre);
                }
                let _ = std::fs::write(f, bytes);
            }
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
            ("ruta", Json::s(ruta.trim_matches('/'))),
            ("retirado", Json::Bool(true)),
            ("carpeta", Json::Bool(true)),
            (
                "ficheros",
                Json::Arr(ficheros.iter().map(Json::s).collect()),
            ),
        ]))
    }

    /// **`POST /arbol/commit`** (0030 W2): VARIOS ficheros en UN commit, con el
    /// mensaje de la persona — y en seco, lo que ese commit sería.
    ///
    /// `PUT /arbol/{ruta}` es un commit por fichero, sin mensaje: es lo que un
    /// *Save* hace. El panel de *Commit* del workspace pide otra cosa: lo que
    /// la sesión tiene sin commitear —N ficheros—, con qué es cada cambio
    /// (`A`/`M`/`D`, +/− líneas) y un mensaje para todos. Lo que dice cada
    /// cambio lo dice **git**, no un contador nuestro: se escriben los ficheros
    /// en el clon, `git status --porcelain` y `git diff --cached --numstat`.
    ///
    /// ```text
    /// {"seco": true|false, "mensaje": "…", "forzar": true|false,
    ///  "ficheros": [{"ruta": "packages/hr/views/x.yaml", "texto": "…"}],
    ///  "retirar": ["packages/hr/views/y.yaml"]}
    /// ```
    ///
    /// - `seco: true` (o sin `mensaje`) **no escribe nada**: contesta `cambios`
    ///   —un objeto por fichero con `estado`, `mas`, `menos`— y los
    ///   diagnósticos que el árbol tendría. Es lo que el panel enseña al abrirse;
    /// - `seco: false` con `mensaje`: el mismo gate que un `PUT` —el árbol no
    ///   empeora, o 422 con los diagnósticos nuevos y nada escrito— y entonces
    ///   un commit del sujeto con ese mensaje, en la rama de `X-Ore-Rama`;
    /// - ⭐ `forzar: true` (2026-09-19): **el árbol es de quien lo escribe**. El
    ///   gate avisa (el 422 de arriba, con los diagnósticos nuevos) pero no
    ///   manda: con `forzar` el commit se hace igual, en cualquier rama —`main`
    ///   también—, y la respuesta lo dice: `forzado: true` y `nuevos` (cuántos
    ///   diagnósticos que antes no estaban entran con él). Git deja commitear lo
    ///   que sea; lo que no compila lo dicen los diagnósticos, *Run* y los
    ///   checks de una propuesta, no una negativa a escribir.
    ///
    /// `If-Match` vale como en el `PUT`: 409 si el árbol se movió.
    pub(crate) fn commit_del_arbol(
        &self,
        raiz: &Path,
        cuerpo: &str,
        si_commit: Option<&str>,
    ) -> Respuesta {
        let n = match ore_core::parse::parse(cuerpo) {
            Ok(n) => n,
            Err(_) => return Respuesta::error(400, "el cuerpo no es JSON"),
        };
        let seco = n
            .get("seco")
            .and_then(|(_, v)| v.as_str())
            .is_some_and(|s| s == "true");
        let mensaje = n
            .get("mensaje")
            .and_then(|(_, v)| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let forzar = n
            .get("forzar")
            .and_then(|(_, v)| v.as_str())
            .is_some_and(|s| s == "true");
        if !seco && mensaje.is_none() {
            return Respuesta::error(
                422,
                "un commit lleva `mensaje`; sin él, `seco: true` para ver los cambios",
            );
        }
        let mut escribir: Vec<(PathBuf, String, String)> = Vec::new();
        if let Some((_, fs)) = n.get("ficheros") {
            for f in fs.items() {
                let Some(ruta) = f.get("ruta").and_then(|(_, v)| v.as_str()) else {
                    return Respuesta::error(422, "cada fichero lleva `ruta` y `texto`");
                };
                let texto = f
                    .get("texto")
                    .and_then(|(_, v)| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if texto.len() > 2 * 1024 * 1024 {
                    return Respuesta::error(413, format!("`{ruta}` pasa de 2 MB"));
                }
                match ruta_valida(ruta) {
                    Ok(rel) => escribir.push((rel, ruta.to_string(), texto)),
                    Err(r) => return r,
                }
            }
        }
        let mut retirar: Vec<(PathBuf, String)> = Vec::new();
        if let Some((_, rs)) = n.get("retirar") {
            for r in rs.items() {
                let Some(ruta) = r.as_str() else { continue };
                match ruta_valida(ruta) {
                    Ok(rel) => retirar.push((rel, ruta.to_string())),
                    Err(r) => return r,
                }
            }
        }
        if escribir.is_empty() && retirar.is_empty() {
            return Respuesta::ok(Json::obj([
                ("cambios", Json::Arr(vec![])),
                ("diagnosticos", Json::Arr(vec![])),
                ("seco", Json::Bool(seco)),
            ]));
        }
        if let Some(r) = self.arbol_se_movio(raiz, si_commit) {
            return r;
        }
        let antes = match self.diagnosticos_de(raiz) {
            Ok(a) => a,
            Err(r) => return r,
        };
        // ── Se escribe en el clon: lo que diga git de aquí en adelante es real ──
        for (rel, ruta, texto) in &escribir {
            let p = raiz.join(rel);
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
        }
        for (rel, ruta) in &retirar {
            let p = raiz.join(rel);
            if p.is_file()
                && let Err(e) = std::fs::remove_file(&p)
            {
                return Respuesta::error(500, format!("no se pudo retirar `{ruta}`: {e}"));
            }
        }
        // `git add -A` y el índice dicen QUÉ cambió y CUÁNTO. Sin git (un
        // directorio sin historia) no hay estados: los ficheros salen con `?`.
        let _ = git(raiz, &["add", "-A"]);
        let estados: std::collections::BTreeMap<String, String> =
            git(raiz, &["status", "--porcelain"])
                .map(|s| {
                    s.lines()
                        .filter(|l| l.len() > 3)
                        .map(|l| {
                            (
                                l[3..].trim().trim_matches('"').to_string(),
                                l[..2].trim().to_string(),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
        let lineas: std::collections::BTreeMap<String, (i64, i64)> =
            git(raiz, &["diff", "--cached", "--numstat"])
                .map(|s| {
                    s.lines()
                        .filter_map(|l| {
                            let mut p = l.split('\t');
                            let mas = p.next()?.parse().unwrap_or(0);
                            let menos = p.next()?.parse().unwrap_or(0);
                            Some((p.next()?.trim().trim_matches('"').to_string(), (mas, menos)))
                        })
                        .collect()
                })
                .unwrap_or_default();
        let cambios: Vec<Json> = escribir
            .iter()
            .map(|(_, ruta, _)| ruta.clone())
            .chain(retirar.iter().map(|(_, ruta)| ruta.clone()))
            .map(|ruta| {
                let estado = estados.get(&ruta).cloned().unwrap_or_default();
                let (mas, menos) = lineas.get(&ruta).copied().unwrap_or((0, 0));
                Json::obj([
                    ("ruta", Json::s(&ruta)),
                    // `A` nuevo, `M` cambiado, `D` retirado, `` igual que estaba
                    (
                        "estado",
                        Json::s(
                            estado
                                .chars()
                                .next()
                                .map(|c| c.to_string())
                                .unwrap_or_default(),
                        ),
                    ),
                    ("mas", Json::Int(mas)),
                    ("menos", Json::Int(menos)),
                ])
            })
            .collect();
        let cambiados = cambios
            .iter()
            .filter(|c| matches!(c, Json::Obj(m) if m.get("estado") != Some(&Json::s(""))))
            .count();
        // ── El gate de siempre: el árbol no empeora, o nada se escribe… ──
        //    …salvo que la persona lo fuerce: entonces se escribe y se dice.
        let que = if cambiados == 1 {
            "1 fichero".to_string()
        } else {
            format!("{cambiados} ficheros")
        };
        let mut nuevos = 0;
        if let Err(mut r) = self.empeora(raiz, &antes, &que) {
            let ds = match &r.cuerpo {
                Json::Obj(m) => match m.get("diagnosticos") {
                    Some(Json::Arr(ds)) => ds.clone(),
                    _ => vec![],
                },
                _ => vec![],
            };
            if !forzar || seco {
                if let Json::Obj(m) = &mut r.cuerpo {
                    m.insert(
                        "diagnosticos".into(),
                        Json::Arr(ds.iter().map(con_posicion).collect()),
                    );
                    m.insert("cambios".into(), Json::Arr(cambios));
                    // Que quien lee sepa que puede: el árbol es suyo.
                    m.insert("forzable".into(), Json::Bool(!seco));
                }
                // En seco el clon se tira; con commit, `escribiendo` no publica un 422.
                return r;
            }
            nuevos = ds.len();
        }
        let despues = self.diagnosticos_de(raiz).unwrap_or_default();
        let ficha = Json::obj([
            ("seco", Json::Bool(seco)),
            ("cambiados", Json::Int(cambiados as i64)),
            ("cambios", Json::Arr(cambios)),
            (
                "diagnosticos",
                Json::Arr(despues.iter().map(con_posicion).collect()),
            ),
            ("mensaje", Json::s(mensaje.clone().unwrap_or_default())),
            ("forzado", Json::Bool(nuevos > 0)),
            ("nuevos", Json::Int(nuevos as i64)),
        ]);
        if seco {
            // Lo escrito se queda en el clon, que se tira: `leyendo` no publica.
            return Respuesta::ok(ficha);
        }
        if cambiados == 0 {
            return Respuesta::ok(ficha);
        }
        // `git add -A` ya está hecho; `publicar` vuelve a hacerlo y commitea con
        // el mensaje de la persona (el que `escribiendo` recibió).
        Respuesta::creado(ficha)
    }

    /// **`GET /arbol/historia/{ruta}`** (0030 W2, *Version history*): las
    /// versiones de un fichero, que git ya tiene. Medido en victor el
    /// 2026-09-19: `ontology.config.yaml` lleva 12; `git log --follow` sobre un
    /// fichero cuesta menos de 10 ms tras el clon. El autor es la persona (el
    /// `sub`) y el committer este servidor, como en cada commit.
    pub(crate) fn historia_del_fichero(&self, raiz: &Path, ruta: &str) -> Respuesta {
        let rel = match ruta_valida(ruta) {
            Ok(r) => r,
            Err(r) => return r,
        };
        if !raiz.join(&rel).is_file() {
            // Puede haber existido: la historia de un fichero retirado también es historia.
            let habia = git(raiz, &["log", "-1", "--format=%h", "--", ruta]).unwrap_or_default();
            if habia.is_empty() {
                return Respuesta::error(404, format!("no hay `{ruta}` en el árbol, ni lo hubo"));
            }
        }
        let Some(s) = git(
            raiz,
            &[
                "log",
                "--follow",
                "--format=%H%x1f%h%x1f%an%x1f%ae%x1f%cn%x1f%aI%x1f%s",
                "--",
                ruta,
            ],
        ) else {
            return Respuesta::ok(Json::obj([
                ("ruta", Json::s(ruta)),
                ("versiones", Json::Arr(vec![])),
            ]));
        };
        let versiones: Vec<Json> = s
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| {
                let mut p = l.split('\u{1f}');
                Json::obj([
                    ("hash", Json::s(p.next().unwrap_or_default())),
                    ("corto", Json::s(p.next().unwrap_or_default())),
                    ("autor", Json::s(p.next().unwrap_or_default())),
                    // El sujeto, que va en el correo del autor (`<sub>@sujeto.invalid`).
                    (
                        "sujeto",
                        Json::s(
                            p.next()
                                .unwrap_or_default()
                                .trim_end_matches("@sujeto.invalid"),
                        ),
                    ),
                    ("committer", Json::s(p.next().unwrap_or_default())),
                    ("cuando", Json::s(p.next().unwrap_or_default())),
                    ("mensaje", Json::s(p.next().unwrap_or_default())),
                ])
            })
            .collect();
        Respuesta::ok(Json::obj([
            ("ruta", Json::s(ruta)),
            ("versiones", Json::Arr(versiones)),
        ]))
    }

    /// **`GET /arbol/version/{hash}/{ruta}`**: el texto de UNA versión
    /// (`git show hash:ruta`). Restaurarla no necesita verbo: es un `PUT` con
    /// ese texto, un commit nuevo de la persona.
    pub(crate) fn version_del_fichero(&self, raiz: &Path, hash: &str, ruta: &str) -> Respuesta {
        if hash.is_empty() || hash.len() > 64 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
            return Respuesta::error(422, format!("`{hash}` no es un commit"));
        }
        if let Err(r) = ruta_valida(ruta) {
            return r;
        }
        // Sin `trim`: el texto de una versión es el fichero byte a byte, con su
        // salto de línea final — si no, restaurarla sería un cambio.
        let salida = std::process::Command::new("git")
            .current_dir(raiz)
            .args(["show", &format!("{hash}:{ruta}")])
            .output();
        match salida {
            Ok(s) if s.status.success() => Respuesta::ok(Json::obj([
                ("ruta", Json::s(ruta)),
                ("hash", Json::s(hash)),
                (
                    "texto",
                    Json::s(String::from_utf8_lossy(&s.stdout).into_owned()),
                ),
            ])),
            _ => Respuesta::error(404, format!("no hay `{ruta}` en el commit `{hash}`")),
        }
    }
}

/// Lo que un `POST /arbol/commit` quiere, antes de clonar: si es en seco y con
/// qué mensaje. `escribiendo` necesita el mensaje ANTES de correr el cuerpo.
pub(crate) fn intencion_del_commit(cuerpo: &str) -> (bool, String) {
    let Ok(n) = ore_core::parse::parse(cuerpo) else {
        return (true, String::new());
    };
    let seco = n
        .get("seco")
        .and_then(|(_, v)| v.as_str())
        .is_some_and(|s| s == "true");
    let mensaje = n
        .get("mensaje")
        .and_then(|(_, v)| v.as_str())
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    (seco || mensaje.is_empty(), mensaje)
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
