//! **La API de la forja** (0030 W2): ramas y *pull requests* del árbol, por
//! el mismo testigo con el que `git.rs` clona y empuja.
//!
//! # Qué se le pide y qué no
//!
//! Se le pide lo que la forja hace mejor que nadie y ya hace: guardar ramas,
//! abrir una PR de una rama a `main`, contar sus ficheros, dar su diff de
//! líneas, guardar comentarios y fusionar. **No se le pide la revisión.**
//! Medido el 2026-09-18 (`medida-w2-proponer.py`): el autor de TODAS las PRs
//! en la forja es `serve-<inquilino>`, y la forja no deja aprobar la PR propia
//! (422). La persona que propone y la que revisa son sujetos de la plataforma
//! —no usuarios de la forja—, así que quién puede fusionar y que no sea quien
//! propuso lo decide `propuestas.rs` con la identidad de la sesión, y la forja
//! guarda el rastro (una review `COMMENT` que nombra a la persona).
//!
//! # Por qué esto es un cliente y no otro
//!
//! Es `http::pedir`, con su plazo y sin TLS, contra un servicio del clúster —
//! la misma frontera que el custodio y el gateway de modelos. La forja habla
//! HTTP dentro de la celda; lo que hay que decirle está en la API de Gitea 1.22
//! que Forgejo 15 sirve, y cabe en diez verbos.

use ore_core::json::Json;
use ore_entrada::http;

/// Dónde está la API, y de qué repositorio.
pub struct Api {
    /// `host:puerto`, sin esquema.
    pub destino: String,
    pub dueno: String,
    pub repo: String,
    pub testigo: String,
}

/// Lo que la forja contestó y no era lo esperado.
#[derive(Debug)]
pub struct Fallo {
    pub codigo: u16,
    pub mensaje: String,
}

impl std::fmt::Display for Fallo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "la forja contestó {}: {}", self.codigo, self.mensaje)
    }
}

impl Api {
    /// De la URL del repositorio (`http://forja.t-x.svc:3000/t-x/ontologia.git`)
    /// sale el destino de la API y el `dueño/repo`. Con `--forja-api` se dice
    /// el destino aparte (el banco: el árbol en `file://` y una API de mentira).
    pub fn de(url: &str, destino: Option<&str>, testigo: &str) -> Option<Api> {
        let sin_esquema = url.split("://").nth(1)?;
        let (host, camino) = sin_esquema.split_once('/')?;
        let mut partes: Vec<&str> = camino.trim_end_matches('/').rsplit('/').collect();
        let repo = partes.first()?.trim_end_matches(".git").to_string();
        let dueno = partes.get(1)?.to_string();
        partes.clear();
        let destino = match destino {
            Some(d) => d.to_string(),
            None if url.starts_with("file://") => return None,
            None => host.to_string(),
        };
        Some(Api {
            destino,
            dueno,
            repo,
            testigo: testigo.to_string(),
        })
    }

    fn camino(&self, resto: &str) -> String {
        format!("/api/v1/repos/{}/{}{}", self.dueno, self.repo, resto)
    }

    /// Una petición JSON. La forja acepta `Authorization: Bearer <token>` para
    /// sus testigos, que es lo que `pedir` escribe.
    fn json(&self, metodo: &str, resto: &str, cuerpo: Option<&Json>) -> Result<Json, Fallo> {
        let (codigo, texto) = http::pedir(
            metodo,
            &self.destino,
            &self.camino(resto),
            Some(&self.testigo),
            cuerpo,
        )
        .map_err(|m| Fallo {
            codigo: 502,
            mensaje: m,
        })?;
        if !(200..300).contains(&codigo) {
            let mensaje = ore_core::parse::parse(&texto)
                .ok()
                .and_then(|n| campo(&crate::rutas::de_node(&n), "message"))
                .unwrap_or_else(|| texto.trim().chars().take(200).collect());
            return Err(Fallo { codigo, mensaje });
        }
        if texto.trim().is_empty() {
            return Ok(Json::obj([]));
        }
        ore_core::parse::parse(&texto)
            .map(|n| crate::rutas::de_node(&n))
            .map_err(|e| Fallo {
                codigo: 502,
                mensaje: format!("la forja contestó algo que no analiza: {e:?}"),
            })
    }

    // ── Ramas ────────────────────────────────────────────────────────────

    /// `[(nombre, commit)]`, la rama por defecto la dice el repositorio.
    pub fn ramas(&self) -> Result<Vec<(String, String)>, Fallo> {
        let j = self.json("GET", "/branches?limit=100", None)?;
        Ok(lista(&j)
            .iter()
            .filter_map(|r| {
                Some((
                    campo(r, "name")?,
                    hijo(r, "commit")
                        .and_then(|c| campo(c, "id"))
                        .unwrap_or_default(),
                ))
            })
            .collect())
    }

    pub fn rama_por_defecto(&self) -> Result<String, Fallo> {
        let j = self.json("GET", "", None)?;
        Ok(campo(&j, "default_branch").unwrap_or_else(|| "main".into()))
    }

    pub fn crear_rama(&self, nombre: &str, desde: &str) -> Result<(), Fallo> {
        self.json(
            "POST",
            "/branches",
            Some(&Json::obj([
                ("new_branch_name", Json::s(nombre)),
                ("old_branch_name", Json::s(desde)),
            ])),
        )
        .map(|_| ())
    }

    pub fn borrar_rama(&self, nombre: &str) -> Result<(), Fallo> {
        self.json("DELETE", &format!("/branches/{nombre}"), None)
            .map(|_| ())
    }

    // ── Pull requests ─────────────────────────────────────────────────────

    /// `estado`: `open`, `closed` o `all`.
    pub fn pulls(&self, estado: &str) -> Result<Vec<Json>, Fallo> {
        let j = self.json("GET", &format!("/pulls?state={estado}&limit=100"), None)?;
        Ok(lista(&j))
    }

    pub fn pull(&self, n: u64) -> Result<Json, Fallo> {
        self.json("GET", &format!("/pulls/{n}"), None)
    }

    pub fn abrir_pull(
        &self,
        head: &str,
        base: &str,
        titulo: &str,
        cuerpo: &str,
    ) -> Result<Json, Fallo> {
        self.json(
            "POST",
            "/pulls",
            Some(&Json::obj([
                ("head", Json::s(head)),
                ("base", Json::s(base)),
                ("title", Json::s(titulo)),
                ("body", Json::s(cuerpo)),
            ])),
        )
    }

    pub fn cerrar_pull(&self, n: u64) -> Result<(), Fallo> {
        self.json(
            "PATCH",
            &format!("/pulls/{n}"),
            Some(&Json::obj([("state", Json::s("closed"))])),
        )
        .map(|_| ())
    }

    pub fn ficheros(&self, n: u64) -> Result<Vec<Json>, Fallo> {
        Ok(lista(&self.json(
            "GET",
            &format!("/pulls/{n}/files?limit=200"),
            None,
        )?))
    }

    /// El diff de líneas, tal cual lo da la forja (texto).
    pub fn diff(&self, n: u64) -> Result<String, Fallo> {
        let (codigo, texto) = http::pedir(
            "GET",
            &self.destino,
            &self.camino(&format!("/pulls/{n}.diff")),
            Some(&self.testigo),
            None,
        )
        .map_err(|m| Fallo {
            codigo: 502,
            mensaje: m,
        })?;
        if codigo != 200 {
            return Err(Fallo {
                codigo,
                mensaje: texto.trim().chars().take(200).collect(),
            });
        }
        Ok(texto)
    }

    pub fn revisiones(&self, n: u64) -> Result<Vec<Json>, Fallo> {
        Ok(lista(&self.json(
            "GET",
            &format!("/pulls/{n}/reviews?limit=200"),
            None,
        )?))
    }

    /// Una review `COMMENT`: la forja no deja `APPROVED` sobre la PR propia, y
    /// todas son propias de `serve-<n>`. El veredicto va en el cuerpo.
    pub fn comentar_revision(&self, n: u64, cuerpo: &str) -> Result<Json, Fallo> {
        self.json(
            "POST",
            &format!("/pulls/{n}/reviews"),
            Some(&Json::obj([
                ("event", Json::s("COMMENT")),
                ("body", Json::s(cuerpo)),
            ])),
        )
    }

    pub fn fusionar(&self, n: u64, mensaje: &str) -> Result<(), Fallo> {
        self.json(
            "POST",
            &format!("/pulls/{n}/merge"),
            Some(&Json::obj([
                ("Do", Json::s("merge")),
                ("merge_message_field", Json::s(mensaje)),
                ("delete_branch_after_merge", Json::Bool(true)),
            ])),
        )
        .map(|_| ())
    }
}

fn lista(j: &Json) -> Vec<Json> {
    match j {
        Json::Arr(v) => v.clone(),
        _ => Vec::new(),
    }
}

/// Un hijo de un objeto de la forja, si está.
pub fn hijo<'a>(j: &'a Json, nombre: &str) -> Option<&'a Json> {
    match j {
        Json::Obj(m) => m.get(nombre),
        _ => None,
    }
}

/// Un campo de texto de un objeto de la forja, si está (un número también
/// vale como texto: `de_node` los distingue por la forma, no por el sentido).
pub fn campo(j: &Json, nombre: &str) -> Option<String> {
    match hijo(j, nombre)? {
        Json::Str(s) => Some(s.clone()),
        Json::Int(n) => Some(n.to_string()),
        Json::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Un campo numérico (la forja los escribe como números).
pub fn numero(j: &Json, nombre: &str) -> Option<i64> {
    match hijo(j, nombre)? {
        Json::Int(n) => Some(*n),
        Json::Str(s) => s.parse().ok(),
        _ => None,
    }
}

pub fn booleano(j: &Json, nombre: &str) -> Option<bool> {
    match hijo(j, nombre)? {
        Json::Bool(b) => Some(*b),
        Json::Str(s) => Some(s == "true"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_url_del_repositorio_dice_destino_dueno_y_repo() {
        let a = Api::de(
            "http://forja.t-victor.svc.cluster.local:3000/t-victor/ontologia.git",
            None,
            "t",
        )
        .unwrap();
        assert_eq!(a.destino, "forja.t-victor.svc.cluster.local:3000");
        assert_eq!(
            (a.dueno.as_str(), a.repo.as_str()),
            ("t-victor", "ontologia")
        );
        assert_eq!(a.camino("/pulls"), "/api/v1/repos/t-victor/ontologia/pulls");
    }

    #[test]
    fn un_arbol_en_file_no_tiene_api_salvo_que_se_diga() {
        assert!(Api::de("file:///tmp/x/t-demo/ontologia.git", None, "t").is_none());
        let a = Api::de(
            "file:///tmp/x/t-demo/ontologia.git",
            Some("127.0.0.1:9"),
            "t",
        )
        .unwrap();
        assert_eq!((a.dueno.as_str(), a.repo.as_str()), ("t-demo", "ontologia"));
        assert_eq!(a.destino, "127.0.0.1:9");
    }
}
