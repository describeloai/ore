//! Los verbos del modelo (0027 E1): `POST /modelos` · `GET /modelos` ·
//! `GET /modelos/{n}` · `DELETE /modelos/{n}`.
//!
//! # La figura
//!
//! La de `POST /fuentes`, sin excepciones: clonar el árbol, escribir el
//! documento, **derivar en el mismo acto** —aquí la derivación es la
//! suscripción en el gateway (0027 ②)— y empujar con quién lo pidió. Con una
//! diferencia deliberada respecto a la credencial de una fuente: **si la
//! suscripción no se hace, el documento no se escribe**. Un `Model` que el
//! gateway no conoce es un documento que promete algo que no existe, y la
//! respuesta es 502 y el árbol intacto — `escribiendo` no publica un clon
//! cuya respuesta es un error.
//!
//! # El encaje (0027 ⑦) se decide en el servidor, contra la lista de perfiles
//!
//! La lista la publica Bastion y la baja el aprovisionador a la cola
//! (`perfiles.json`, junto a `plantilla-catalogo.txt`, E1 I2). Un `profile`
//! que no está → 422 con los que sí están. `tier: dedicated` → 422 hasta que
//! haya cuota de máquina (E5). Un `digest` cuando el perfil no publica el suyo
//! → 422: no se puede prometer lo que nadie ha medido.
//!
//! # `modelo/<n>` se resuelve aquí
//!
//! `GET /modelos/<n>` devuelve la puerta (`url`, el plano de datos del
//! gateway) y el id servido (`model`, del perfil). Es lo que el Job de E0
//! tomaba de variables; quien ejecute una `Function` lo pregunta con su token.
//!
//! # Lo que este proceso sigue sin ganar
//!
//! Habla con el gateway por HTTP llano dentro de la VPC (`--modelos
//! host:puerto`), como con el cofre. No gana internet, ni TLS, ni un cliente
//! de propósito general: es `http::pedir`, con su plazo, y nada más.

use crate::cola;
use crate::mando;
use crate::rutas::{Servidor, analizar, primera_linea, token};
use ore_core::json::Json;
use ore_core::parse::{self, Node};
use ore_entrada::http::{self, Respuesta};
use std::path::Path;

/// Lo que el aprovisionador deja en la cola: la lista de perfiles certificados.
pub const PERFILES: &str = "perfiles.json";

/// Dónde está el gateway, y qué contesta como puerta.
pub struct Modelos {
    /// El plano de control, `host:puerto` sin esquema (VPC, HTTP llano).
    pub admin: String,
    /// El plano de datos que una `Function` llama: `http://host:8000/v1`.
    pub url: String,
}

/// Un perfil de la lista, con lo que aquí se coteja y lo que se devuelve.
struct Perfil {
    profile: String,
    model: String,
    digest: Option<String>,
    status: String,
    usd_per_mtok: String,
}

fn campo(n: &Node, k: &str) -> Option<String> {
    n.get(k).and_then(|(_, v)| v.as_str()).map(str::to_string)
}

fn perfiles_de(texto: &str) -> Result<Vec<Perfil>, String> {
    let n = parse::parse(texto).map_err(|e| format!("`{PERFILES}` no analiza: {e:?}"))?;
    let Some((_, lista)) = n.get("profiles") else {
        return Err(format!("`{PERFILES}` no trae `profiles`"));
    };
    Ok(lista
        .items()
        .iter()
        .filter_map(|p| {
            Some(Perfil {
                profile: campo(p, "profile")?,
                model: campo(p, "model")?,
                digest: campo(p, "digest").filter(|d| d != "null" && !d.is_empty()),
                status: campo(p, "status").unwrap_or_default(),
                usd_per_mtok: campo(p, "usd_per_mtok").unwrap_or_default(),
            })
        })
        .collect())
}

impl Servidor {
    /// La lista, de donde esté: un fichero (`--perfiles`, el banco) o la cola.
    fn perfiles(&self) -> Result<Vec<Perfil>, Respuesta> {
        if let Some(f) = &self.perfiles {
            let t = std::fs::read_to_string(f)
                .map_err(|e| Respuesta::error(503, format!("no se pudo leer `{}`: {e}", f.display())))?;
            return perfiles_de(&t).map_err(|m| Respuesta::error(503, m));
        }
        let Some(forja) = &self.cola else {
            return Err(Respuesta::error(
                503,
                format!("este servidor no sabe de ninguna cola (`--cola`) ni de un fichero de perfiles (`--perfiles`): sin `{PERFILES}` no hay contra qué encajar un modelo"),
            ));
        };
        let prestado = forja.clonar().map_err(|e| Respuesta::error(502, e.to_string()))?;
        let t = std::fs::read_to_string(prestado.ruta().join(PERFILES)).map_err(|_| {
            Respuesta::error(
                503,
                format!("la cola no trae `{PERFILES}`; hay que converger este inquilino (el aprovisionador la baja de donde Bastion la publica)"),
            )
        })?;
        let _ = cola::PLANTILLA; // la misma cola, el mismo camino
        perfiles_de(&t).map_err(|m| Respuesta::error(503, m))
    }

    fn gateway(&self) -> Result<&Modelos, Respuesta> {
        self.modelos.as_ref().ok_or_else(|| {
            Respuesta::error(
                422,
                "este servidor no sabe de ningún gateway (`--modelos host:puerto`). Un modelo sin \
                 suscripción es un documento que promete algo que no existe: no se escribe",
            )
        })
    }

    fn celda(&self) -> Result<&str, Respuesta> {
        self.organizacion.as_deref().ok_or_else(|| {
            Respuesta::error(422, "este servidor no sabe de quién es este árbol (`--organizacion`): la suscripción es por celda")
        })
    }

    /// `POST /modelos {name, profile, tier?, task?, digest?, description?}`.
    pub(crate) fn alta_de_modelo(&self, raiz: &Path, cuerpo: &str) -> Respuesta {
        let cuerpo = match analizar(cuerpo) {
            Ok(n) => n,
            Err(r) => return r,
        };
        let Some(nombre) = campo(&cuerpo, "name") else {
            return Respuesta::error(422, "falta `name`");
        };
        if let Err(m) = token(&nombre) {
            return Respuesta::error(422, format!("`name`: {m}"));
        }
        let Some(profile) = campo(&cuerpo, "profile") else {
            return Respuesta::error(422, "falta `profile` (`<maquina>/<modelo>`, uno de la lista de certificación)");
        };
        let tier = campo(&cuerpo, "tier").unwrap_or_else(|| "shared".into());
        let task = campo(&cuerpo, "task").unwrap_or_else(|| "chat".into());
        let digest = campo(&cuerpo, "digest");
        let descripcion = campo(&cuerpo, "description");

        let puerta = match self.gateway() {
            Ok(g) => g,
            Err(r) => return r,
        };
        let celda = match self.celda() {
            Ok(c) => c,
            Err(r) => return r,
        };
        // ── ⑦ · el encaje: hay perfil, y es el que se pide ──────────────────
        let perfiles = match self.perfiles() {
            Ok(p) => p,
            Err(r) => return r,
        };
        let Some(perfil) = perfiles.iter().find(|p| p.profile == profile) else {
            let hay: Vec<String> = perfiles.iter().map(|p| format!("`{}` ({})", p.profile, p.status)).collect();
            return Respuesta::error(
                422,
                format!(
                    "`profile: {profile}` no está en la lista de certificación. Un perfil sin número no \
                     existe. Los que hay: {}",
                    if hay.is_empty() { "ninguno".to_string() } else { hay.join(", ") }
                ),
            );
        };
        if tier == "dedicated" {
            return Respuesta::error(
                422,
                "`tier: dedicated` necesita cuota de máquina para esta organización, y no la hay (0027 E5). Hoy: `shared`",
            );
        }
        if let Some(d) = &digest {
            match &perfil.digest {
                None => {
                    return Respuesta::error(
                        422,
                        format!("`digest` no se puede prometer: el perfil `{profile}` todavía no publica el suyo (B4). Déjalo fuera"),
                    );
                }
                Some(suyo) if suyo != d => {
                    return Respuesta::error(422, format!("`digest` no es el del perfil `{profile}`: el perfil sirve `{suyo}`"));
                }
                _ => {}
            }
        }
        // ── el documento ────────────────────────────────────────────────────
        let dir = raiz.join("modelos");
        let fichero = dir.join(format!("{nombre}.yaml"));
        if fichero.exists() {
            return Respuesta::error(409, format!("ya hay un modelo `{nombre}`"));
        }
        let mut texto = format!(
            "apiVersion: oos.dev/v1alpha9\nkind: Model\nmetadata:\n  name: {nombre}\n"
        );
        if let Some(d) = &descripcion {
            texto.push_str(&format!("  description: {}\n", Json::s(d.clone()).jcs()));
        }
        texto.push_str(&format!("spec:\n  profile: {profile}\n"));
        if let Some(d) = &digest {
            texto.push_str(&format!("  digest: {d}\n"));
        }
        texto.push_str(&format!("  tier: {tier}\n  task: {task}\n"));
        if let Err(e) = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&fichero, &texto)) {
            return Respuesta::error(500, format!("no se pudo escribir `modelos/{nombre}.yaml`: {e}"));
        }
        // la gramática decide la forma (tier, task, digest…), no este proceso.
        // (Sobre un directorio no hay clon que tirar: lo escrito se retira.)
        if let Some(r) = self.no_compila(raiz) {
            let _ = std::fs::remove_file(&fichero);
            return r;
        }
        // ── y la suscripción, en el mismo acto (②) ──────────────────────────
        let suscripcion = match http::pedir(
            "POST",
            &puerta.admin,
            &format!("/admin/tenants/{celda}/models"),
            None,
            Some(&Json::obj([("model", Json::s(perfil.model.clone()))])),
        ) {
            Ok((201, _)) | Ok((200, _)) => format!("provisionada en {} para la celda `{celda}`", puerta.admin),
            Ok((cod, cuerpo)) => {
                let _ = std::fs::remove_file(&fichero);
                return Respuesta::error(
                    502,
                    format!("el gateway contestó {cod} a la suscripción ({}); no se escribió nada", cuerpo.trim().chars().take(160).collect::<String>()),
                );
            }
            Err(e) => {
                let _ = std::fs::remove_file(&fichero);
                return Respuesta::error(502, format!("no se pudo suscribir a `{celda}` en el gateway: {e}; no se escribió nada"));
            }
        };
        Respuesta::creado(Json::obj([
            ("name", Json::s(nombre)),
            ("profile", Json::s(profile)),
            ("model", Json::s(perfil.model.clone())),
            ("tier", Json::s(tier)),
            ("task", Json::s(task)),
            ("url", Json::s(puerta.url.clone())),
            ("usd_per_mtok", Json::s(perfil.usd_per_mtok.clone())),
            ("suscripcion", Json::s(suscripcion)),
        ]))
    }

    /// `DELETE /modelos/{n}`: el fichero fuera, la suscripción fuera, o nada.
    pub(crate) fn retirar_modelo(&self, raiz: &Path, nombre: &str) -> Respuesta {
        if let Err(m) = token(nombre) {
            return Respuesta::error(422, format!("`name`: {m}"));
        }
        let fichero = raiz.join("modelos").join(format!("{nombre}.yaml"));
        let Ok(texto) = std::fs::read_to_string(&fichero) else {
            return Respuesta::error(404, format!("no hay ningún modelo `{nombre}`"));
        };
        let puerta = match self.gateway() {
            Ok(g) => g,
            Err(r) => return r,
        };
        let celda = match self.celda() {
            Ok(c) => c,
            Err(r) => return r,
        };
        let profile = parse::parse(&texto)
            .ok()
            .and_then(|n| n.get("spec").and_then(|(_, s)| campo(s, "profile")))
            .unwrap_or_default();
        if let Err(e) = std::fs::remove_file(&fichero) {
            return Respuesta::error(500, format!("no se pudo retirar `modelos/{nombre}.yaml`: {e}"));
        }
        // una función que lo nombre deja de resolver: se dice y no se retira.
        // (Se vuelve a escribir: sobre un directorio no hay clon que tirar.)
        if let Some(mut r) = self.no_compila(raiz) {
            let _ = std::fs::write(&fichero, &texto);
            r.codigo = 409;
            return r;
        }
        // el id servido, del perfil; si la lista no está, del gateway no se puede retirar por id
        let id = match self.perfiles() {
            Ok(p) => p.into_iter().find(|p| p.profile == profile).map(|p| p.model),
            Err(_) => None,
        };
        let Some(id) = id else {
            return Respuesta::error(
                503,
                format!("no sé qué id sirve el perfil `{profile}` (sin `{PERFILES}`): la suscripción no se puede retirar, y sin eso el modelo tampoco"),
            );
        };
        let camino = format!("/admin/tenants/{celda}/models/{}", id.replace('/', "%2F"));
        match http::pedir("DELETE", &puerta.admin, &camino, None, None) {
            Ok((204, _)) | Ok((200, _)) | Ok((404, _)) => {}
            Ok((cod, cuerpo)) => {
                return Respuesta::error(502, format!("el gateway contestó {cod} al retirar la suscripción ({}); el modelo se queda", cuerpo.trim().chars().take(160).collect::<String>()));
            }
            Err(e) => return Respuesta::error(502, format!("no se pudo retirar la suscripción en el gateway: {e}; el modelo se queda")),
        }
        Respuesta::ok(Json::obj([
            ("name", Json::s(nombre)),
            ("retirado", Json::Bool(true)),
            ("suscripcion", Json::s(format!("retirada en {} para la celda `{celda}`", puerta.admin))),
        ]))
    }

    /// `GET /modelos`.
    pub(crate) fn modelos(&self, raiz: &Path) -> Respuesta {
        let perfiles = self.perfiles().ok();
        let mut lista = Vec::new();
        if let Ok(es) = std::fs::read_dir(raiz.join("modelos")) {
            let mut rutas: Vec<_> = es.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "yaml")).collect();
            rutas.sort();
            for p in rutas {
                if let Some(j) = self.ficha(&p, perfiles.as_deref()) {
                    lista.push(j);
                }
            }
        }
        Respuesta::ok(Json::obj([("modelos", Json::Arr(lista))]))
    }

    /// `GET /modelos/{n}`: la ficha con la resolución, o 404.
    pub(crate) fn modelo(&self, raiz: &Path, nombre: &str) -> Respuesta {
        let p = raiz.join("modelos").join(format!("{nombre}.yaml"));
        match self.ficha(&p, self.perfiles().ok().as_deref()) {
            Some(j) => Respuesta::ok(j),
            None => Respuesta::error(404, format!("no hay ningún modelo `{nombre}`")),
        }
    }

    /// Un `Model` del árbol, resuelto: `modelo/<n>` → `(url, model)`.
    fn ficha(&self, fichero: &Path, perfiles: Option<&[Perfil]>) -> Option<Json> {
        let texto = std::fs::read_to_string(fichero).ok()?;
        let n = parse::parse(&texto).ok()?;
        if n.get("kind").and_then(|(_, k)| k.as_str()) != Some("Model") {
            return None;
        }
        let meta = n.get("metadata").map(|(_, m)| m)?;
        let spec = n.get("spec").map(|(_, s)| s)?;
        let profile = campo(spec, "profile").unwrap_or_default();
        let perfil = perfiles.and_then(|ps| ps.iter().find(|p| p.profile == profile));
        let opt = |v: Option<String>| v.map(Json::s).unwrap_or(Json::Bool(false));
        Some(Json::obj([
            ("name", Json::s(campo(meta, "name").unwrap_or_default())),
            ("description", opt(campo(meta, "description"))),
            ("profile", Json::s(profile)),
            ("digest", opt(campo(spec, "digest"))),
            ("tier", Json::s(campo(spec, "tier").unwrap_or_default())),
            ("task", Json::s(campo(spec, "task").unwrap_or_default())),
            // la resolución: lo que una Function necesita para llamar
            ("model", opt(perfil.map(|p| p.model.clone()))),
            ("url", opt(self.modelos.as_ref().map(|m| m.url.clone()))),
            ("usd_per_mtok", opt(perfil.map(|p| p.usd_per_mtok.clone()))),
            ("certificado", Json::Bool(perfil.is_some())),
        ]))
    }

    /// `ore validate` sobre el clon: `None` si compila, la respuesta 422 si no.
    fn no_compila(&self, raiz: &Path) -> Option<Respuesta> {
        match mando::correr(&self.binario, raiz, &["validate".into(), ".".into()]) {
            Err(e) => Some(Respuesta::error(500, e.to_string())),
            Ok(s) if !s.bien() => Some(Respuesta::error(
                422,
                format!("el árbol no compila con ese modelo: {}", primera_linea(&s.stdout, &s.stderr)),
            )),
            Ok(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_lista_se_lee_con_sus_numeros_y_su_digest_nulo() {
        let t = r#"{"v": 1, "profiles": [{"profile": "g1/deepseek-v2-lite", "model": "deepseek-ai/DeepSeek-V2-Lite", "status": "validated", "usd_per_mtok": 0.2765, "digest": null, "tok_s": {"32": 1507.0}}]}"#;
        let p = perfiles_de(t).unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].model, "deepseek-ai/DeepSeek-V2-Lite");
        assert_eq!(p[0].digest, None, "null no es un digest");
        assert_eq!(p[0].usd_per_mtok, "0.2765");
        assert!(perfiles_de("{}").is_err());
    }
}
