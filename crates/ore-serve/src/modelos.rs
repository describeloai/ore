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
//! # La ficha es una fila (0027 E3 ⑥): el cruce se hace aquí
//!
//! Un `Model` declarado no dice si **sirve**. Eso lo sabe el gateway, y sólo
//! la VPC lo alcanza: la consola no. Así que `GET /modelos` le pregunta al
//! gateway una vez por petición —`/admin/health` (qué backends están arriba),
//! `/admin/tenants` (a qué está suscrita esta celda) y `/admin/usage` (lo que
//! la celda gastó este mes)— y devuelve, con cada ficha, `estado {fase,
//! backends, motivo?}`, `uso {hoy, mes}`, `autor` y `desde`. Si el gateway no
//! contesta, la ficha sale igual, con `estado.fase: error` y el motivo: la
//! consola nunca espera al gateway y nunca inventa una píldora.
//!
//!   declarado · un backend `up` sirve su id  → `running`
//!   declarado · ninguno arriba              → `provisioning`
//!   declarado · la celda no está suscrita   → `error` (deriva: el verbo hace las dos)
//!   declarado · el gateway no contesta      → `error` con el motivo
//!   suscrito · sin documento                → `retiring` (deriva; sale como fila)
//!
//! `autor` y `desde` son del commit que escribió el fichero: el verbo firma
//! con el sujeto de la petición (`git.rs`), así que la fila lleva quién la
//! pidió sin ninguna tabla aparte.
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

/// Un nodo tal como vino (JSON o YAML) a `Json`, sin perder: los números que
/// no son enteros se quedan como texto (`Json` no tiene coma flotante, y
/// `0.2765` tampoco la tendría exacta), y `null` es `false`, como en la ficha.
fn json_de(n: &Node) -> Json {
    match n {
        Node::Scalar { raw, style, .. } => {
            if matches!(style, parse::Style::Plain) {
                match raw.as_str() {
                    "true" => return Json::Bool(true),
                    "false" | "null" | "~" => return Json::Bool(false),
                    _ => {}
                }
                if let Ok(i) = raw.parse::<i64>() {
                    return Json::Int(i);
                }
            }
            Json::s(raw.clone())
        }
        Node::Sequence { items, .. } => Json::Arr(items.iter().map(json_de).collect()),
        Node::Mapping { entries, .. } => Json::Obj(
            entries
                .iter()
                .filter_map(|(k, v)| Some((k.as_str()?.to_string(), json_de(v))))
                .collect(),
        ),
    }
}

/// Hoy en UTC como `AAAA-MM-DD`, y el primero del mes. Sin reloj de pared
/// externo: el calendario civil desde los días de la época (Hinnant).
fn hoy_utc() -> (String, String) {
    fecha_de(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
    )
}

fn fecha_de(seg: i64) -> (String, String) {
    let z = seg.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (format!("{y:04}-{m:02}-{d:02}"), format!("{y:04}-{m:02}-01"))
}

/// Una fila de `/admin/usage`: día × modelo, de esta celda.
struct Uso {
    dia: String,
    model: String,
    peticiones: i64,
    tokens: i64,
    usd: f64,
}

/// Lo que el gateway cuenta, preguntado UNA vez por petición (E3 ⑥).
struct Gateway {
    /// `(id, model, up)` de `/admin/health`.
    backends: Vec<(String, String, bool)>,
    /// Los ids a los que esta celda está suscrita (`/admin/tenants`).
    suscritos: Vec<String>,
    /// Las filas de este mes de esta celda (`/admin/usage`).
    uso: Vec<Uso>,
    /// Si no contestó: por qué. Con esto, ninguna ficha dice `running`.
    caido: Option<String>,
}

impl Gateway {
    fn caido(motivo: impl Into<String>) -> Gateway {
        Gateway {
            backends: Vec::new(),
            suscritos: Vec::new(),
            uso: Vec::new(),
            caido: Some(motivo.into()),
        }
    }
}

fn entero(n: &Node, k: &str) -> i64 {
    campo(n, k).and_then(|v| v.parse().ok()).unwrap_or(0)
}

/// `uso {hoy, mes}` de un id servido, de las filas de este mes.
fn uso_de(gw: &Gateway, id: &str) -> Json {
    let (hoy, _) = hoy_utc();
    let suma = |f: &dyn Fn(&Uso) -> bool| {
        let (mut p, mut t, mut usd) = (0i64, 0i64, 0f64);
        for u in gw.uso.iter().filter(|u| u.model == id && f(u)) {
            p += u.peticiones;
            t += u.tokens;
            usd += u.usd;
        }
        Json::obj([
            ("peticiones", Json::Int(p)),
            ("tokens", Json::Int(t)),
            ("usd", Json::s(format!("{usd:.4}"))),
        ])
    };
    Json::obj([("hoy", suma(&|u| u.dia == hoy)), ("mes", suma(&|_| true))])
}

/// Lo que se le preguntó al gateway, para que quien lea la lista sepa si las
/// fases son de verdad o del silencio.
fn gateway_json(gw: &Gateway) -> Json {
    Json::obj([
        ("contesta", Json::Bool(gw.caido.is_none())),
        (
            "motivo",
            gw.caido.clone().map(Json::s).unwrap_or(Json::Bool(false)),
        ),
        (
            "backends_arriba",
            Json::Int(gw.backends.iter().filter(|b| b.2).count() as i64),
        ),
    ])
}

/// Quién escribió el fichero y cuándo, del commit que lo trajo. Sobre un
/// directorio sin historia, nada.
fn autor_de(raiz: &Path, fichero: &Path) -> Option<(String, String)> {
    let relativo = fichero.strip_prefix(raiz).ok()?;
    let s = std::process::Command::new("git")
        .current_dir(raiz)
        .args(["log", "-1", "--format=%an%x1f%aI", "--"])
        .arg(relativo)
        .output()
        .ok()?;
    if !s.status.success() {
        return None;
    }
    let texto = String::from_utf8_lossy(&s.stdout);
    let (autor, desde) = texto.trim().split_once('\u{1f}')?;
    if autor.is_empty() {
        return None;
    }
    Some((autor.to_string(), desde.to_string()))
}

impl Servidor {
    /// Pregunta al gateway lo que hace de una ficha una fila. Nunca falla: si
    /// no contesta, devuelve por qué, y las fichas salen con `error`.
    fn preguntar_al_gateway(&self) -> Gateway {
        let Some(puerta) = self.modelos.as_ref() else {
            return Gateway::caido(
                "este servidor no sabe de ningún gateway (`--modelos host:puerto`)",
            );
        };
        let celda = self.organizacion.as_deref().unwrap_or_default();
        let pide = |camino: &str| -> Result<Node, String> {
            match http::pedir("GET", &puerta.admin, camino, None, None) {
                Ok((200, cuerpo)) => {
                    parse::parse(&cuerpo).map_err(|e| format!("`{camino}` no analiza: {e:?}"))
                }
                Ok((cod, _)) => Err(format!("el gateway contestó {cod} a `{camino}`")),
                Err(e) => Err(e),
            }
        };
        let salud = match pide("/admin/health") {
            Ok(n) => n,
            Err(e) => return Gateway::caido(e),
        };
        let backends = salud
            .get("backends")
            .map(|(_, b)| b.items())
            .unwrap_or_default()
            .iter()
            .filter_map(|b| {
                Some((
                    campo(b, "id")?,
                    campo(b, "model")?,
                    campo(b, "up").as_deref() == Some("true"),
                ))
            })
            .collect();
        let suscritos = pide("/admin/tenants")
            .map(|n| {
                n.items()
                    .iter()
                    .find(|t| campo(t, "id").as_deref() == Some(celda))
                    .and_then(|t| t.get("allowed_models").map(|(_, m)| m.items().to_vec()))
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|m| m.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let (_, mes) = hoy_utc();
        let uso = pide(&format!("/admin/usage?tenant={celda}&from={mes}"))
            .map(|n| {
                n.items()
                    .iter()
                    .filter_map(|f| {
                        Some(Uso {
                            dia: campo(f, "day")?,
                            model: campo(f, "model")?,
                            peticiones: entero(f, "requests"),
                            tokens: entero(f, "prompt_tokens") + entero(f, "completion_tokens"),
                            usd: campo(f, "usd").and_then(|v| v.parse().ok()).unwrap_or(0.0),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Gateway {
            backends,
            suscritos,
            uso,
            caido: None,
        }
    }

    /// `GET /perfiles`: la lista de certificación tal como Bastion la publica.
    pub(crate) fn perfiles_publicados(&self) -> Respuesta {
        let texto = match self.texto_de_perfiles() {
            Ok(t) => t,
            Err(r) => return r,
        };
        match parse::parse(&texto) {
            Ok(n) => Respuesta::ok(json_de(&n)),
            Err(e) => Respuesta::error(503, format!("`{PERFILES}` no analiza: {e:?}")),
        }
    }

    /// El texto de la lista, de donde esté: un fichero (`--perfiles`, el
    /// banco) o la cola.
    fn texto_de_perfiles(&self) -> Result<String, Respuesta> {
        if let Some(f) = &self.perfiles {
            return std::fs::read_to_string(f).map_err(|e| {
                Respuesta::error(503, format!("no se pudo leer `{}`: {e}", f.display()))
            });
        }
        let Some(forja) = &self.cola else {
            return Err(Respuesta::error(
                503,
                format!(
                    "este servidor no sabe de ninguna cola (`--cola`) ni de un fichero de perfiles (`--perfiles`): sin `{PERFILES}` no hay contra qué encajar un modelo"
                ),
            ));
        };
        let prestado = forja
            .clonar()
            .map_err(|e| Respuesta::error(502, e.to_string()))?;
        let _ = cola::PLANTILLA; // la misma cola, el mismo camino
        std::fs::read_to_string(prestado.ruta().join(PERFILES)).map_err(|_| {
            Respuesta::error(
                503,
                format!("la cola no trae `{PERFILES}`; hay que converger este inquilino (el aprovisionador la baja de donde Bastion la publica)"),
            )
        })
    }

    /// La lista, de donde esté: un fichero (`--perfiles`, el banco) o la cola.
    fn perfiles(&self) -> Result<Vec<Perfil>, Respuesta> {
        let t = self.texto_de_perfiles()?;
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
            return Respuesta::error(
                422,
                "falta `profile` (`<maquina>/<modelo>`, uno de la lista de certificación)",
            );
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
            let hay: Vec<String> = perfiles
                .iter()
                .map(|p| format!("`{}` ({})", p.profile, p.status))
                .collect();
            return Respuesta::error(
                422,
                format!(
                    "`profile: {profile}` no está en la lista de certificación. Un perfil sin número no \
                     existe. Los que hay: {}",
                    if hay.is_empty() {
                        "ninguno".to_string()
                    } else {
                        hay.join(", ")
                    }
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
                        format!(
                            "`digest` no se puede prometer: el perfil `{profile}` todavía no publica el suyo (B4). Déjalo fuera"
                        ),
                    );
                }
                Some(suyo) if suyo != d => {
                    return Respuesta::error(
                        422,
                        format!(
                            "`digest` no es el del perfil `{profile}`: el perfil sirve `{suyo}`"
                        ),
                    );
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
        let mut texto =
            format!("apiVersion: oos.dev/v1alpha9\nkind: Model\nmetadata:\n  name: {nombre}\n");
        if let Some(d) = &descripcion {
            texto.push_str(&format!("  description: {}\n", Json::s(d.clone()).jcs()));
        }
        texto.push_str(&format!("spec:\n  profile: {profile}\n"));
        if let Some(d) = &digest {
            texto.push_str(&format!("  digest: {d}\n"));
        }
        texto.push_str(&format!("  tier: {tier}\n  task: {task}\n"));
        if let Err(e) = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&fichero, &texto))
        {
            return Respuesta::error(
                500,
                format!("no se pudo escribir `modelos/{nombre}.yaml`: {e}"),
            );
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
            Ok((201, _)) | Ok((200, _)) => {
                format!("provisionada en {} para la celda `{celda}`", puerta.admin)
            }
            Ok((cod, cuerpo)) => {
                let _ = std::fs::remove_file(&fichero);
                return Respuesta::error(
                    502,
                    format!(
                        "el gateway contestó {cod} a la suscripción ({}); no se escribió nada",
                        cuerpo.trim().chars().take(160).collect::<String>()
                    ),
                );
            }
            Err(e) => {
                let _ = std::fs::remove_file(&fichero);
                return Respuesta::error(
                    502,
                    format!(
                        "no se pudo suscribir a `{celda}` en el gateway: {e}; no se escribió nada"
                    ),
                );
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
            return Respuesta::error(
                500,
                format!("no se pudo retirar `modelos/{nombre}.yaml`: {e}"),
            );
        }
        // una función que lo nombre deja de resolver: se dice y no se retira.
        // (Se vuelve a escribir: sobre un directorio no hay clon que tirar.)
        if let Some(r) = self.no_compila(raiz) {
            let _ = std::fs::write(&fichero, &texto);
            let motivo = match &r.cuerpo {
                Json::Obj(m) => m
                    .get("error")
                    .and_then(|e| {
                        if let Json::Str(s) = e {
                            Some(s.clone())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_default(),
                _ => String::new(),
            };
            return Respuesta::error(
                409,
                format!(
                    "no se retira `{nombre}`: alguien del árbol lo nombra y sin él no compila — {motivo}. Retira primero la Function que lo invoca"
                ),
            );
        }
        // el id servido, del perfil; si la lista no está, del gateway no se puede retirar por id
        let id = match self.perfiles() {
            Ok(p) => p
                .into_iter()
                .find(|p| p.profile == profile)
                .map(|p| p.model),
            Err(_) => None,
        };
        let Some(id) = id else {
            return Respuesta::error(
                503,
                format!(
                    "no sé qué id sirve el perfil `{profile}` (sin `{PERFILES}`): la suscripción no se puede retirar, y sin eso el modelo tampoco"
                ),
            );
        };
        let camino = format!("/admin/tenants/{celda}/models/{}", id.replace('/', "%2F"));
        match http::pedir("DELETE", &puerta.admin, &camino, None, None) {
            Ok((204, _)) | Ok((200, _)) | Ok((404, _)) => {}
            Ok((cod, cuerpo)) => {
                return Respuesta::error(
                    502,
                    format!(
                        "el gateway contestó {cod} al retirar la suscripción ({}); el modelo se queda",
                        cuerpo.trim().chars().take(160).collect::<String>()
                    ),
                );
            }
            Err(e) => {
                return Respuesta::error(
                    502,
                    format!(
                        "no se pudo retirar la suscripción en el gateway: {e}; el modelo se queda"
                    ),
                );
            }
        }
        Respuesta::ok(Json::obj([
            ("name", Json::s(nombre)),
            ("retirado", Json::Bool(true)),
            (
                "suscripcion",
                Json::s(format!(
                    "retirada en {} para la celda `{celda}`",
                    puerta.admin
                )),
            ),
        ]))
    }

    /// `GET /modelos`: las filas (E3). Las del árbol, con lo que el gateway
    /// cuenta de cada una; y, si la celda está suscrita a algo que el árbol no
    /// declara, esa deriva como fila `retiring`.
    pub(crate) fn modelos(&self, raiz: &Path) -> Respuesta {
        let perfiles = self.perfiles().ok();
        let gw = self.preguntar_al_gateway();
        let mut lista = Vec::new();
        let mut servidos = Vec::new();
        if let Ok(es) = std::fs::read_dir(raiz.join("modelos")) {
            let mut rutas: Vec<_> = es
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "yaml"))
                .collect();
            rutas.sort();
            for p in rutas {
                if let Some((j, id)) = self.ficha(raiz, &p, perfiles.as_deref(), &gw) {
                    servidos.extend(id);
                    lista.push(j);
                }
            }
        }
        for id in gw.suscritos.iter().filter(|s| !servidos.contains(s)) {
            lista.push(Json::obj([
                ("name", Json::Bool(false)),
                ("model", Json::s(id.clone())),
                ("declarado", Json::Bool(false)),
                (
                    "estado",
                    Json::obj([
                        ("fase", Json::s("retiring")),
                        ("backends", Json::Arr(Vec::new())),
                        (
                            "motivo",
                            Json::s(format!(
                                "la celda está suscrita a `{id}` en el gateway y ningún documento del árbol lo declara: deriva, se retira"
                            )),
                        ),
                    ]),
                ),
                ("uso", uso_de(&gw, id)),
            ]));
        }
        Respuesta::ok(Json::obj([
            ("modelos", Json::Arr(lista)),
            ("gateway", gateway_json(&gw)),
        ]))
    }

    /// `GET /modelos/{n}`: la ficha con la resolución y su estado, o 404.
    pub(crate) fn modelo(&self, raiz: &Path, nombre: &str) -> Respuesta {
        let p = raiz.join("modelos").join(format!("{nombre}.yaml"));
        let gw = self.preguntar_al_gateway();
        match self.ficha(raiz, &p, self.perfiles().ok().as_deref(), &gw) {
            Some((j, _)) => Respuesta::ok(j),
            None => Respuesta::error(404, format!("no hay ningún modelo `{nombre}`")),
        }
    }

    /// Un `Model` del árbol, resuelto y cruzado con el gateway: la ficha y el
    /// id servido (si el perfil está).
    fn ficha(
        &self,
        raiz: &Path,
        fichero: &Path,
        perfiles: Option<&[Perfil]>,
        gw: &Gateway,
    ) -> Option<(Json, Option<String>)> {
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
        let id = perfil.map(|p| p.model.clone());
        // ── ⑥ el cruce ──────────────────────────────────────────────────────
        let (fase, arriba, motivo): (&str, Vec<String>, Option<String>) = match (&id, &gw.caido) {
            (None, _) => (
                "error",
                Vec::new(),
                Some(format!(
                    "el perfil `{profile}` no está en la lista de certificación: no se sabe qué id sirve"
                )),
            ),
            (Some(_), Some(m)) => ("error", Vec::new(), Some(m.clone())),
            (Some(id), None) => {
                let arriba: Vec<String> = gw
                    .backends
                    .iter()
                    .filter(|(_, m, up)| m == id && *up)
                    .map(|(b, _, _)| b.clone())
                    .collect();
                if !gw.suscritos.contains(id) {
                    (
                        "error",
                        arriba,
                        Some(format!(
                            "la celda no está suscrita a `{id}` en el gateway: el documento promete lo que el gateway niega (deriva)"
                        )),
                    )
                } else if arriba.is_empty() {
                    (
                        "provisioning",
                        arriba,
                        Some(format!("ningún backend sirve `{id}` todavía")),
                    )
                } else {
                    ("running", arriba, None)
                }
            }
        };
        let (autor, desde) = autor_de(raiz, fichero).unzip();
        let ficha = Json::obj([
            ("name", Json::s(campo(meta, "name").unwrap_or_default())),
            ("description", opt(campo(meta, "description"))),
            ("profile", Json::s(profile)),
            ("digest", opt(campo(spec, "digest"))),
            ("tier", Json::s(campo(spec, "tier").unwrap_or_default())),
            ("task", Json::s(campo(spec, "task").unwrap_or_default())),
            // la resolución: lo que una Function necesita para llamar
            ("model", opt(id.clone())),
            ("url", opt(self.modelos.as_ref().map(|m| m.url.clone()))),
            ("usd_per_mtok", opt(perfil.map(|p| p.usd_per_mtok.clone()))),
            ("certificado", Json::Bool(perfil.is_some())),
            ("declarado", Json::Bool(true)),
            // la fila (E3)
            (
                "estado",
                Json::obj([
                    ("fase", Json::s(fase)),
                    (
                        "backends",
                        Json::Arr(arriba.into_iter().map(Json::s).collect()),
                    ),
                    ("motivo", opt(motivo)),
                ]),
            ),
            (
                "uso",
                id.as_deref()
                    .map(|i| uso_de(gw, i))
                    .unwrap_or(Json::Bool(false)),
            ),
            ("autor", opt(autor)),
            ("desde", opt(desde)),
        ]);
        Some((ficha, id))
    }

    /// `ore validate` sobre el clon: `None` si compila, la respuesta 422 si no.
    fn no_compila(&self, raiz: &Path) -> Option<Respuesta> {
        match mando::correr(&self.binario, raiz, &["validate".into(), ".".into()]) {
            Err(e) => Some(Respuesta::error(500, e.to_string())),
            Ok(s) if !s.bien() => Some(Respuesta::error(
                422,
                format!(
                    "el árbol no compila con ese modelo: {}",
                    primera_linea(&s.stdout, &s.stderr)
                ),
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

    #[test]
    fn el_calendario_civil_sale_de_los_segundos() {
        assert_eq!(fecha_de(0).0, "1970-01-01");
        assert_eq!(fecha_de(951_782_400).0, "2000-02-29"); // bisiesto de siglo
        assert_eq!(
            fecha_de(1_758_067_200),
            ("2025-09-17".into(), "2025-09-01".into())
        );
        assert_eq!(fecha_de(1_767_225_599).0, "2025-12-31"); // el ultimo segundo del ano
    }

    #[test]
    fn la_lista_publicada_sale_tal_cual_y_null_es_false() {
        let n = parse::parse(r#"{"v": 1, "profiles": [{"profile": "g1/x", "gpus": 1, "usd_per_mtok": 0.2765, "digest": null, "tok_s": {"1": 209.0}}]}"#).unwrap();
        assert_eq!(
            json_de(&n).jcs(),
            r#"{"profiles":[{"digest":false,"gpus":1,"profile":"g1/x","tok_s":{"1":"209.0"},"usd_per_mtok":"0.2765"}],"v":1}"#
        );
    }
}
