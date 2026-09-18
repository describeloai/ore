//! **Proponer** (ADR 0030 W2): rama por persona, propuesta en la forja, el
//! diff y los diagnósticos de la rama, la revisión de OTRA persona, y el
//! merge — que es lo que Flux mira.
//!
//! # Lo que la medida dictó (`medida-w2-proponer.py`, 2026-09-18)
//!
//! La forja ya tiene ramas, PRs, ficheros, diff y comentarios, y cuestan
//! segundos. Lo que NO tiene es a las personas: todas las PRs las abre
//! `serve-<inquilino>` y la forja no deja aprobar la PR propia. Así que:
//!
//! - **quién propone** va en el cuerpo de la PR (`sub: <persona>`), igual que
//!   el commit lleva a la persona de autor y a este servidor de committer;
//! - **la revisión es nuestra**: `revisar` la deja en la forja como una review
//!   `COMMENT` cuyo cuerpo nombra a la persona y su veredicto, y `fusionar`
//!   exige que quien fusiona NO sea quien propuso y que alguien distinto haya
//!   aprobado — *dos personas, dos ramas, una revisión*;
//! - **la rama no despliega**: Flux sigue mirando `main`; el merge es lo que
//!   despliega, y la forja avisa en segundos (17-el-aviso).
//!
//! # Los códigos
//!
//! - `422` sin API de la forja (un árbol en directorio), nombre de rama malo,
//!   quien propone intenta revisar o fusionar lo suyo, sin revisión, la rama
//!   no compila;
//! - `409` la propuesta ya no está abierta, la rama tiene conflictos, la rama
//!   tiene una propuesta abierta y se quiere retirar;
//! - `404` la rama o la propuesta no está;
//! - lo demás, lo que la forja dijo, con su código.

use crate::forja::{Api, Fallo, booleano, campo, hijo, numero};
use crate::rutas::{Arbol, Servidor};
use ore_core::json::Json;
use ore_entrada::http::Respuesta;
use ore_entrada::identidad::Identidad;
use std::path::Path;

/// La cabecera con la que el editor dice EN QUÉ RAMA lee o escribe el árbol.
/// Sin ella, `main`, que es lo que todo hacía hasta hoy. Va en una cabecera y
/// no en la URL por la misma regla que el resto: ningún dato entra por la URL.
pub const CABECERA_RAMA: &str = "x-ore-rama";

/// Un nombre de rama que git acepta y que no se sale de `refs/heads/`.
pub fn nombre_de_rama_valido(s: &str) -> Result<(), String> {
    if s.is_empty() || s.len() > 120 {
        return Err("el nombre de la rama tiene que tener entre 1 y 120 caracteres".into());
    }
    if s.starts_with('/')
        || s.ends_with('/')
        || s.contains("..")
        || s.contains("//")
        || s.ends_with(".lock")
    {
        return Err(format!(
            "`{s}` no es un nombre de rama: ni empieza o acaba en `/`, ni `..`, ni `//`, ni `.lock`"
        ));
    }
    if !s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
    {
        return Err(format!(
            "`{s}` no es un nombre de rama: letras, cifras, `-`, `_`, `.` y `/`"
        ));
    }
    Ok(())
}

/// El prefijo de rama de una persona: lo que va tras el último `:` de su
/// sujeto, en minúsculas y sin nada que git no quiera. `persona:ana` → `ana`.
pub fn prefijo_de(persona: &str) -> String {
    let base = persona.rsplit(':').next().unwrap_or(persona);
    let limpio: String = base
        .chars()
        .map(|c| c.to_ascii_lowercase())
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    if limpio.is_empty() {
        "persona".into()
    } else {
        limpio
    }
}

/// Un campo de texto del cuerpo JSON de la petición.
fn del_cuerpo(cuerpo: &str, k: &str) -> Option<String> {
    if cuerpo.trim().is_empty() {
        return None;
    }
    let n = ore_core::parse::parse(cuerpo).ok()?;
    n.get(k).and_then(|(_, v)| v.as_str().map(String::from))
}

fn de_la_forja(e: Fallo) -> Respuesta {
    let codigo = match e.codigo {
        404 => 404,
        409 | 422 => e.codigo,
        _ => 502,
    };
    Respuesta::error(codigo, e.to_string())
}

/// El sujeto que propuso, del cuerpo de la PR: la primera línea `sub: …`.
fn autor_de(pr: &Json) -> String {
    campo(pr, "body")
        .and_then(|b| {
            b.lines()
                .next()
                .and_then(|l| l.strip_prefix("sub: ").map(String::from))
        })
        .unwrap_or_default()
}

fn descripcion_de(pr: &Json) -> String {
    campo(pr, "body")
        .map(|b| {
            b.lines()
                .skip(1)
                .collect::<Vec<_>>()
                .join("\n")
                .trim()
                .to_string()
        })
        .unwrap_or_default()
}

fn estado_de(pr: &Json) -> &'static str {
    if booleano(pr, "merged").unwrap_or(false) {
        "fusionada"
    } else if campo(pr, "state").as_deref() == Some("open") {
        "abierta"
    } else {
        "cerrada"
    }
}

/// La propuesta como la consola la quiere: sin la forja dentro.
fn propuesta_de(pr: &Json) -> Json {
    let rama = hijo(pr, "head")
        .and_then(|h| campo(h, "ref"))
        .unwrap_or_default();
    let base = hijo(pr, "base")
        .and_then(|h| campo(h, "ref"))
        .unwrap_or_default();
    Json::obj([
        ("numero", Json::Int(numero(pr, "number").unwrap_or(0))),
        ("titulo", Json::s(campo(pr, "title").unwrap_or_default())),
        ("descripcion", Json::s(descripcion_de(pr))),
        ("estado", Json::s(estado_de(pr))),
        ("autor", Json::s(autor_de(pr))),
        ("rama", Json::s(rama)),
        ("base", Json::s(base)),
        (
            "creada",
            Json::s(campo(pr, "created_at").unwrap_or_default()),
        ),
        (
            "fusionada",
            Json::s(campo(pr, "merged_at").unwrap_or_default()),
        ),
        (
            "mergeable",
            Json::Bool(booleano(pr, "mergeable").unwrap_or(true)),
        ),
    ])
}

/// Una review de la forja como revisión nuestra: `revision: <persona> <veredicto>`
/// en la primera línea del cuerpo; lo demás es el texto.
fn revision_de(r: &Json) -> Option<Json> {
    let cuerpo = campo(r, "body").unwrap_or_default();
    let primera = cuerpo.lines().next().unwrap_or("");
    let resto = primera.strip_prefix("revision: ")?;
    let (persona, veredicto) = resto.rsplit_once(' ')?;
    Some(Json::obj([
        ("por", Json::s(persona)),
        ("veredicto", Json::s(veredicto)),
        (
            "texto",
            Json::s(cuerpo.lines().skip(1).collect::<Vec<_>>().join("\n").trim()),
        ),
        (
            "cuando",
            Json::s(campo(r, "submitted_at").unwrap_or_default()),
        ),
    ]))
}

impl Servidor {
    fn api(&self) -> Result<&Api, Respuesta> {
        self.forja_api.as_ref().ok_or_else(|| {
            Respuesta::error(
                422,
                "este servidor no sabe de ramas ni propuestas: el árbol no está en una forja con API",
            )
        })
    }

    fn forja(&self) -> Option<&crate::git::Forja> {
        match &self.arbol {
            Arbol::Forja(f) => Some(f),
            Arbol::Directorio(_) => None,
        }
    }

    // ── Ramas ──────────────────────────────────────────────────────────────

    /// `GET /ramas`: las ramas del árbol, y qué propuesta abierta tiene cada una.
    pub(crate) fn ramas(&self) -> Respuesta {
        let api = match self.api() {
            Ok(a) => a,
            Err(r) => return r,
        };
        let por_defecto = api.rama_por_defecto().unwrap_or_else(|_| "main".into());
        let abiertas = api.pulls("open").unwrap_or_default();
        match api.ramas() {
            Ok(ramas) => Respuesta::ok(Json::obj([
                ("porDefecto", Json::s(&por_defecto)),
                (
                    "ramas",
                    Json::Arr(
                        ramas
                            .into_iter()
                            .map(|(nombre, commit)| {
                                let propuesta = abiertas
                                    .iter()
                                    .find(|pr| {
                                        hijo(pr, "head").and_then(|h| campo(h, "ref")).as_deref()
                                            == Some(nombre.as_str())
                                    })
                                    .and_then(|pr| numero(pr, "number"));
                                Json::obj([
                                    ("nombre", Json::s(&nombre)),
                                    ("porDefecto", Json::Bool(nombre == por_defecto)),
                                    ("commit", Json::s(commit)),
                                    (
                                        "propuesta",
                                        propuesta.map(Json::Int).unwrap_or(Json::Bool(false)),
                                    ),
                                ])
                            })
                            .collect(),
                    ),
                ),
            ])),
            Err(e) => de_la_forja(e),
        }
    }

    /// `POST /ramas {nombre?, desde?}`: una rama de la persona, `<persona>/<nombre>`,
    /// desde `main` (o `desde`). Sin nombre, la fecha.
    pub(crate) fn crear_rama(&self, sujeto: &Identidad, cuerpo: &str) -> Respuesta {
        let api = match self.api() {
            Ok(a) => a,
            Err(r) => return r,
        };
        let sufijo = del_cuerpo(cuerpo, "nombre")
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| {
                let t = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                format!("propuesta-{t}")
            });
        let nombre = format!("{}/{}", prefijo_de(&sujeto.persona), sufijo.trim());
        if let Err(m) = nombre_de_rama_valido(&nombre) {
            return Respuesta::error(422, m);
        }
        let desde = del_cuerpo(cuerpo, "desde")
            .unwrap_or_else(|| api.rama_por_defecto().unwrap_or_else(|_| "main".into()));
        match api.crear_rama(&nombre, &desde) {
            Ok(()) => Respuesta::creado(Json::obj([
                ("rama", Json::s(&nombre)),
                ("desde", Json::s(&desde)),
                ("de", Json::s(&sujeto.persona)),
            ])),
            Err(e) => de_la_forja(e),
        }
    }

    /// `DELETE /ramas/{nombre}`: fuera, salvo que tenga una propuesta abierta o sea `main`.
    pub(crate) fn retirar_rama(&self, nombre: &str) -> Respuesta {
        let api = match self.api() {
            Ok(a) => a,
            Err(r) => return r,
        };
        if let Err(m) = nombre_de_rama_valido(nombre) {
            return Respuesta::error(422, m);
        }
        let por_defecto = api.rama_por_defecto().unwrap_or_else(|_| "main".into());
        if nombre == por_defecto {
            return Respuesta::error(
                422,
                format!("`{nombre}` es la rama por defecto: es lo que Flux mira, y no se retira"),
            );
        }
        if let Some(pr) =
            api.pulls("open").unwrap_or_default().iter().find(|pr| {
                hijo(pr, "head").and_then(|h| campo(h, "ref")).as_deref() == Some(nombre)
            })
        {
            return Respuesta::error(
                409,
                format!(
                    "la rama `{nombre}` tiene la propuesta #{} abierta: ciérrala o fusiónala antes",
                    numero(pr, "number").unwrap_or(0)
                ),
            );
        }
        match api.borrar_rama(nombre) {
            Ok(()) => Respuesta::ok(Json::obj([
                ("rama", Json::s(nombre)),
                ("retirada", Json::Bool(true)),
            ])),
            Err(e) => de_la_forja(e),
        }
    }

    // ── Propuestas ─────────────────────────────────────────────────────────

    /// `GET /propuestas`: todas, con su estado (`abierta`, `fusionada`, `cerrada`).
    pub(crate) fn propuestas(&self) -> Respuesta {
        let api = match self.api() {
            Ok(a) => a,
            Err(r) => return r,
        };
        match api.pulls("all") {
            Ok(prs) => Respuesta::ok(Json::obj([(
                "propuestas",
                Json::Arr(prs.iter().map(propuesta_de).collect()),
            )])),
            Err(e) => de_la_forja(e),
        }
    }

    /// `POST /propuestas {rama, titulo, descripcion?}`: la PR de la rama a `main`,
    /// con la persona en el cuerpo.
    pub(crate) fn proponer(&self, sujeto: &Identidad, cuerpo: &str) -> Respuesta {
        let api = match self.api() {
            Ok(a) => a,
            Err(r) => return r,
        };
        let Some(rama) = del_cuerpo(cuerpo, "rama").filter(|r| !r.is_empty()) else {
            return Respuesta::error(422, "falta `rama`: la rama que se propone");
        };
        if let Err(m) = nombre_de_rama_valido(&rama) {
            return Respuesta::error(422, m);
        }
        let base = api.rama_por_defecto().unwrap_or_else(|_| "main".into());
        if rama == base {
            return Respuesta::error(
                422,
                format!("`{rama}` es la rama por defecto: una propuesta sale de otra rama"),
            );
        }
        let titulo = del_cuerpo(cuerpo, "titulo")
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| format!("Propuesta de {}: {rama}", sujeto.persona));
        let descripcion = del_cuerpo(cuerpo, "descripcion").unwrap_or_default();
        let cuerpo_pr = format!("sub: {}\n\n{}", sujeto.persona, descripcion.trim());
        match api.abrir_pull(&rama, &base, titulo.trim(), &cuerpo_pr) {
            Ok(pr) => Respuesta::creado(propuesta_de(&pr)),
            Err(e) => de_la_forja(e),
        }
    }

    /// `GET /propuestas/{n}`: la propuesta con sus ficheros, el diff de líneas
    /// (la forja), el diff de significado (`ore diff main rama`), los
    /// diagnósticos de la rama y las revisiones.
    pub(crate) fn propuesta(&self, n: u64) -> Respuesta {
        let api = match self.api() {
            Ok(a) => a,
            Err(r) => return r,
        };
        let pr = match api.pull(n) {
            Ok(p) => p,
            Err(e) => return de_la_forja(e),
        };
        let mut ficha = propuesta_de(&pr);
        let rama = hijo(&pr, "head")
            .and_then(|h| campo(h, "ref"))
            .unwrap_or_default();
        let base = hijo(&pr, "base")
            .and_then(|h| campo(h, "ref"))
            .unwrap_or_else(|| "main".into());
        let ficheros: Vec<Json> = api
            .ficheros(n)
            .unwrap_or_default()
            .iter()
            .map(|f| {
                Json::obj([
                    ("ruta", Json::s(campo(f, "filename").unwrap_or_default())),
                    ("estado", Json::s(campo(f, "status").unwrap_or_default())),
                    ("mas", Json::Int(numero(f, "additions").unwrap_or(0))),
                    ("menos", Json::Int(numero(f, "deletions").unwrap_or(0))),
                ])
            })
            .collect();
        let diff = api.diff(n).unwrap_or_default();
        let revisiones: Vec<Json> = api
            .revisiones(n)
            .unwrap_or_default()
            .iter()
            .filter_map(revision_de)
            .collect();
        // El significado y los diagnósticos salen de la rama misma, no de la forja.
        let (semantico, diagnosticos) = if estado_de(&pr) == "abierta" {
            self.rama_frente_a(&rama, &base)
        } else {
            (Json::Bool(false), Json::Arr(vec![]))
        };
        if let Json::Obj(m) = &mut ficha {
            m.insert("ficheros".into(), Json::Arr(ficheros));
            m.insert("diff".into(), Json::s(diff));
            m.insert("semantico".into(), semantico);
            m.insert("diagnosticos".into(), diagnosticos);
            m.insert("revisiones".into(), Json::Arr(revisiones));
        }
        Respuesta::ok(ficha)
    }

    /// `ore diff <base> <rama>` y `ore validate` de la rama: `(semantico, diagnosticos)`.
    /// Dos clones; medido, ~1 s cada uno.
    fn rama_frente_a(&self, rama: &str, base: &str) -> (Json, Json) {
        let Some(forja) = self.forja() else {
            return (Json::Bool(false), Json::Arr(vec![]));
        };
        let (Ok(de_rama), Ok(de_base)) =
            (forja.clonar_rama(Some(rama)), forja.clonar_rama(Some(base)))
        else {
            return (Json::Bool(false), Json::Arr(vec![]));
        };
        let diagnosticos = self
            .diagnosticos_de(de_rama.ruta())
            .map(|ds| Json::Arr(ds.iter().map(crate::arbol::con_posicion).collect()))
            .unwrap_or(Json::Arr(vec![]));
        let semantico = match crate::mando::correr(
            &self.binario,
            de_rama.ruta(),
            &[
                "diff".into(),
                de_base.ruta().to_string_lossy().into_owned(),
                de_rama.ruta().to_string_lossy().into_owned(),
            ],
        ) {
            // `ore diff` sale 1 cuando hay cambios, como `diff`: no es un fallo.
            Ok(s) if s.codigo == 0 || s.codigo == 1 => ore_core::parse::parse(&s.stdout)
                .map(|n| crate::rutas::de_node(&n))
                .unwrap_or(Json::Bool(false)),
            _ => Json::Bool(false),
        };
        (semantico, diagnosticos)
    }

    /// `POST /propuestas/{n}/revisar {veredicto, texto?}`: `aprobar`,
    /// `pedir-cambios` o `comentar`. Quien propuso no se aprueba a sí mismo.
    pub(crate) fn revisar(&self, sujeto: &Identidad, n: u64, cuerpo: &str) -> Respuesta {
        let api = match self.api() {
            Ok(a) => a,
            Err(r) => return r,
        };
        let veredicto = match del_cuerpo(cuerpo, "veredicto").as_deref() {
            Some("aprobar") => "aprueba",
            Some("pedir-cambios") => "pide-cambios",
            Some("comentar") | None => "comenta",
            Some(otro) => {
                return Respuesta::error(
                    422,
                    format!("`veredicto` es `aprobar`, `pedir-cambios` o `comentar`, no `{otro}`"),
                );
            }
        };
        let pr = match api.pull(n) {
            Ok(p) => p,
            Err(e) => return de_la_forja(e),
        };
        if estado_de(&pr) != "abierta" {
            return Respuesta::error(
                409,
                format!("la propuesta #{n} está {}: ya no se revisa", estado_de(&pr)),
            );
        }
        if veredicto == "aprueba" && autor_de(&pr) == sujeto.persona {
            return Respuesta::error(
                422,
                "quien propone no aprueba lo suyo: la revisión es de otra persona",
            );
        }
        let texto = del_cuerpo(cuerpo, "texto").unwrap_or_default();
        let cuerpo_review = format!(
            "revision: {} {veredicto}\n\n{}",
            sujeto.persona,
            texto.trim()
        );
        match api.comentar_revision(n, &cuerpo_review) {
            Ok(_) => Respuesta::creado(Json::obj([
                ("numero", Json::Int(n as i64)),
                ("por", Json::s(&sujeto.persona)),
                ("veredicto", Json::s(veredicto)),
            ])),
            Err(e) => de_la_forja(e),
        }
    }

    /// `POST /propuestas/{n}/fusionar`: dos personas, una revisión, la rama
    /// compila, sin conflictos — y entonces `main`, que es lo que Flux mira.
    pub(crate) fn fusionar(&self, sujeto: &Identidad, n: u64) -> Respuesta {
        let api = match self.api() {
            Ok(a) => a,
            Err(r) => return r,
        };
        let pr = match api.pull(n) {
            Ok(p) => p,
            Err(e) => return de_la_forja(e),
        };
        if estado_de(&pr) != "abierta" {
            return Respuesta::error(409, format!("la propuesta #{n} está {}", estado_de(&pr)));
        }
        let autor = autor_de(&pr);
        if autor == sujeto.persona {
            return Respuesta::error(
                422,
                "quien propone no fusiona lo suyo: hace falta otra persona (0030 W2: dos personas, una revisión)",
            );
        }
        let aprobada_por: Vec<String> = api
            .revisiones(n)
            .unwrap_or_default()
            .iter()
            .filter_map(revision_de)
            .filter(|r| campo(r, "veredicto").as_deref() == Some("aprueba"))
            .filter_map(|r| campo(&r, "por"))
            .filter(|p| *p != autor)
            .collect();
        if aprobada_por.is_empty() {
            return Respuesta::error(
                422,
                format!(
                    "la propuesta #{n} no tiene revisión: nadie distinto de `{autor}` la ha aprobado"
                ),
            );
        }
        if !booleano(&pr, "mergeable").unwrap_or(true) {
            return Respuesta::error(
                409,
                format!(
                    "la rama de la propuesta #{n} tiene conflictos con la base: hay que rehacerla"
                ),
            );
        }
        let rama = hijo(&pr, "head")
            .and_then(|h| campo(h, "ref"))
            .unwrap_or_default();
        let base = hijo(&pr, "base")
            .and_then(|h| campo(h, "ref"))
            .unwrap_or_else(|| "main".into());
        let (_, diagnosticos) = self.rama_frente_a(&rama, &base);
        if let Json::Arr(ds) = &diagnosticos
            && !ds.is_empty()
        {
            return Respuesta {
                codigo: 422,
                cuerpo: Json::obj([
                    (
                        "error",
                        Json::s(format!("la rama `{rama}` no compila: no se fusiona")),
                    ),
                    ("diagnosticos", diagnosticos.clone()),
                ]),
            };
        }
        let mensaje = format!(
            "Propuesta #{n} de {autor}, revisada por {} y fusionada por {}: {}",
            aprobada_por.join(", "),
            sujeto.persona,
            campo(&pr, "title").unwrap_or_default()
        );
        match api.fusionar(n, &mensaje) {
            Ok(()) => Respuesta::ok(Json::obj([
                ("numero", Json::Int(n as i64)),
                ("fusionada", Json::Bool(true)),
                ("por", Json::s(&sujeto.persona)),
                (
                    "revisada_por",
                    Json::Arr(aprobada_por.iter().map(Json::s).collect()),
                ),
                ("rama", Json::s(&rama)),
            ])),
            Err(e) => de_la_forja(e),
        }
    }

    /// `DELETE /propuestas/{n}`: cerrada sin fusionar. La rama se queda.
    pub(crate) fn cerrar_propuesta(&self, n: u64) -> Respuesta {
        let api = match self.api() {
            Ok(a) => a,
            Err(r) => return r,
        };
        let pr = match api.pull(n) {
            Ok(p) => p,
            Err(e) => return de_la_forja(e),
        };
        if estado_de(&pr) != "abierta" {
            return Respuesta::error(409, format!("la propuesta #{n} ya está {}", estado_de(&pr)));
        }
        match api.cerrar_pull(n) {
            Ok(()) => Respuesta::ok(Json::obj([
                ("numero", Json::Int(n as i64)),
                ("estado", Json::s("cerrada")),
            ])),
            Err(e) => de_la_forja(e),
        }
    }

    // ── El árbol EN una rama ────────────────────────────────────────────────

    /// Como `leyendo`, en la rama que diga la cabecera. Sin cabecera, `main`.
    pub(crate) fn leyendo_en(
        &self,
        rama: Option<&str>,
        f: impl FnOnce(&Path) -> Respuesta,
    ) -> Respuesta {
        match (rama, &self.arbol) {
            (None, _) => self.leyendo(f),
            (Some(r), Arbol::Directorio(_)) => Respuesta::error(
                422,
                format!("este árbol es un directorio, no una forja: no hay rama `{r}` que leer"),
            ),
            (Some(r), Arbol::Forja(forja)) => {
                if let Err(m) = nombre_de_rama_valido(r) {
                    return Respuesta::error(422, m);
                }
                match forja.clonar_rama(Some(r)) {
                    Err(crate::git::Fallo::SinRama(r)) => {
                        Respuesta::error(404, format!("no hay ninguna rama `{r}`"))
                    }
                    Err(e) => Respuesta::error(502, e.to_string()),
                    Ok(prestado) => f(prestado.ruta()),
                }
            }
        }
    }

    /// Como `escribiendo`, en la rama que diga la cabecera: el clon ES la rama,
    /// así que `publicar` empuja a ella. Sin cabecera, `main`.
    pub(crate) fn escribiendo_en(
        &self,
        rama: Option<&str>,
        sujeto: &Identidad,
        mensaje: &str,
        f: impl FnOnce(&Path) -> Respuesta,
    ) -> Respuesta {
        match (rama, &self.arbol) {
            (None, _) => self.escribiendo(sujeto, mensaje, f),
            (Some(r), Arbol::Directorio(_)) => Respuesta::error(
                422,
                format!(
                    "este árbol es un directorio, no una forja: no hay rama `{r}` que escribir"
                ),
            ),
            (Some(r), Arbol::Forja(forja)) => {
                if let Err(m) = nombre_de_rama_valido(r) {
                    return Respuesta::error(422, m);
                }
                let prestado = match forja.clonar_rama(Some(r)) {
                    Err(crate::git::Fallo::SinRama(r)) => {
                        return Respuesta::error(404, format!("no hay ninguna rama `{r}`"));
                    }
                    Err(e) => return Respuesta::error(502, e.to_string()),
                    Ok(p) => p,
                };
                let mut resp = f(prestado.ruta());
                if resp.codigo >= 300 || !forja.hay_cambios(prestado.ruta()) {
                    return resp;
                }
                match forja.publicar(prestado.ruta(), sujeto, mensaje) {
                    Ok(commit) => {
                        if let Json::Obj(m) = &mut resp.cuerpo {
                            m.insert("commit".into(), Json::s(commit));
                            m.insert("rama".into(), Json::s(r));
                        }
                        resp
                    }
                    Err(crate::git::Fallo::Adelantado(m)) => {
                        Respuesta::error(409, crate::git::Fallo::Adelantado(m).to_string())
                    }
                    Err(e) => Respuesta::error(502, e.to_string()),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_prefijo_es_la_persona_limpia() {
        assert_eq!(prefijo_de("persona:ana"), "ana");
        assert_eq!(prefijo_de("Ana García"), "anagarca");
        assert_eq!(prefijo_de("::"), "persona");
        assert_eq!(prefijo_de("8f0c-4a"), "8f0c-4a");
    }

    #[test]
    fn los_nombres_de_rama_que_git_no_quiere_no_pasan() {
        assert!(nombre_de_rama_valido("ana/vistas-hr").is_ok());
        assert!(nombre_de_rama_valido("main").is_ok());
        assert!(nombre_de_rama_valido("").is_err());
        assert!(nombre_de_rama_valido("/x").is_err());
        assert!(nombre_de_rama_valido("a..b").is_err());
        assert!(nombre_de_rama_valido("a b").is_err());
        assert!(nombre_de_rama_valido("x.lock").is_err());
    }

    #[test]
    fn la_persona_y_el_veredicto_viajan_en_el_cuerpo_de_la_review() {
        let r = Json::obj([
            (
                "body",
                Json::s("revision: persona:bea aprueba\n\nbien visto"),
            ),
            ("submitted_at", Json::s("2026-09-18T20:00:00Z")),
        ]);
        let v = revision_de(&r).unwrap();
        assert_eq!(campo(&v, "por").as_deref(), Some("persona:bea"));
        assert_eq!(campo(&v, "veredicto").as_deref(), Some("aprueba"));
        assert_eq!(campo(&v, "texto").as_deref(), Some("bien visto"));
        assert!(revision_de(&Json::obj([("body", Json::s("un comentario suelto"))])).is_none());
    }

    #[test]
    fn el_autor_es_la_primera_linea_del_cuerpo_de_la_pr() {
        let pr = Json::obj([("body", Json::s("sub: persona:ana\n\nla vista de hr"))]);
        assert_eq!(autor_de(&pr), "persona:ana");
        assert_eq!(descripcion_de(&pr), "la vista de hr");
    }
}
