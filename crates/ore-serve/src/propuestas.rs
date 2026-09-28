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

/// Lo que git dijo, como respuesta: rama que no está 404, choque o
/// adelantamiento 409 (con los ficheros si los hay), lo demás 502.
fn de_git(e: crate::git::Fallo) -> Respuesta {
    use crate::git::Fallo as G;
    match e {
        G::SinRama(r) => Respuesta::error(404, format!("no hay ninguna rama `{r}`")),
        G::Conflicto(fs) => Respuesta {
            codigo: 409,
            cuerpo: Json::obj([
                (
                    "error",
                    Json::s(format!(
                        "lo propuesto choca con lo que `main` cambió después: {}. Trae `main` a la rama, resuélvelo y vuelve a proponer",
                        fs.join(", ")
                    )),
                ),
                ("conflictos", Json::Arr(fs.iter().map(Json::s).collect())),
            ]),
        },
        e @ (G::Adelantado(_) | G::Cruza(_)) => Respuesta::error(409, e.to_string()),
        e => Respuesta::error(502, e.to_string()),
    }
}

pub(crate) fn de_la_forja(e: Fallo) -> Respuesta {
    let codigo = match e.codigo {
        404 => 404,
        409 | 422 => e.codigo,
        _ => 502,
    };
    Respuesta::error(codigo, e.to_string())
}

/// El sujeto que propuso, del cuerpo de la PR: la primera línea `sub: …`.
/// **Quién fusiona, según la política de `main`** (P1.3, `politica.rs`). Una
/// sola regla para la propuesta de rama entera, la de alcance y el detalle que
/// la enseña antes de pulsar. `aprobada_por`: las aprobaciones que valen —de
/// otra persona que la autora y, con alcance, sobre lo que la PR lleva hoy—.
///
/// - **Libre**: fusiona cualquiera, la autora también, con revisión o sin
///   ella. En `main` libre ya se escribe sin propuesta: exigir aquí dos
///   personas no protegía nada y sólo encerraba a quien trabaja solo.
/// - **Protegida**: la de 0030 W2, dos personas y una revisión.
///
/// Que la autora no apruebe lo suyo no es de aquí: vale igual en las dos
/// (`revisar`), porque una aprobación propia no dice nada.
fn quien_fusiona(
    protegida: bool,
    autor: &str,
    quien: &str,
    aprobada_por: &[String],
) -> Result<(), String> {
    if !protegida {
        return Ok(());
    }
    if autor == quien {
        return Err(
            "`main` está protegida: quien propone no fusiona lo suyo, hace falta otra persona (0030 W2: dos personas, una revisión)"
                .into(),
        );
    }
    if aprobada_por.is_empty() {
        return Err(format!(
            "`main` está protegida: nadie distinto de `{autor}` ha aprobado lo que la propuesta lleva ahora"
        ));
    }
    Ok(())
}

/// El mensaje del commit de merge: quién propuso, quién revisó —o que nadie— y
/// quién fusionó. `que`: el alcance, si lo tiene (` (… de rama)`).
fn mensaje_de_fusion(
    n: u64,
    autor: &str,
    que: &str,
    aprobada_por: &[String],
    quien: &str,
    titulo: &str,
) -> String {
    if aprobada_por.is_empty() {
        format!("Propuesta #{n} de {autor}{que}, fusionada sin revisión por {quien}: {titulo}")
    } else {
        format!(
            "Propuesta #{n} de {autor}{que}, revisada por {} y fusionada por {quien}: {titulo}",
            aprobada_por.join(", ")
        )
    }
}

fn autor_de(pr: &Json) -> String {
    campo(pr, "body")
        .and_then(|b| {
            b.lines()
                .next()
                .and_then(|l| l.strip_prefix("sub: ").map(String::from))
        })
        .unwrap_or_default()
}

/// Las líneas de la cabecera del cuerpo de la PR: `sub: …` y, en una
/// propuesta con alcance (0044 A.2 ②), `rama: …` y `alcance: …`.
const CABECERA: [&str; 4] = ["sub: ", "rama: ", "alcance: ", "activos: "];

fn de_la_cabecera(pr: &Json, k: &str) -> Option<String> {
    let cuerpo = campo(pr, "body")?;
    let pre = format!("{k}: ");
    cuerpo
        .lines()
        .take_while(|l| CABECERA.iter().any(|c| l.starts_with(c)))
        .find_map(|l| l.strip_prefix(pre.as_str()).map(|v| v.trim().to_string()))
}

/// La rama de la que sale la PR en la forja: la rama, o su derivada.
fn cabeza_de_pr(pr: &Json) -> String {
    hijo(pr, "head")
        .and_then(|h| campo(h, "ref"))
        .unwrap_or_default()
}

/// La rama contra la que se propone (`main`).
fn base_de(pr: &Json) -> String {
    hijo(pr, "base")
        .and_then(|h| campo(h, "ref"))
        .unwrap_or_else(|| "main".into())
}

/// La rama que se propone: la global, también cuando la PR sale de su derivada.
fn rama_de(pr: &Json) -> String {
    de_la_cabecera(pr, "rama").unwrap_or_else(|| cabeza_de_pr(pr))
}

/// El alcance de una propuesta: la carpeta de un repositorio. `None`: la rama entera.
fn alcance_de(pr: &Json) -> Option<String> {
    de_la_cabecera(pr, "alcance")
}

/// Los activos de una propuesta con alcance de activos (0044 A.2, E2): sus ids.
fn activos_de(pr: &Json) -> Option<Vec<String>> {
    de_la_cabecera(pr, "activos").map(|l| {
        l.split(", ")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect()
    })
}

/// Lo que lleva una propuesta que no es la rama entera.
enum Alcance {
    /// La carpeta de un repositorio (E1).
    Carpeta(String),
    /// Unos activos, por su id (E2).
    Activos(Vec<String>),
}

impl Alcance {
    fn de(pr: &Json) -> Option<Alcance> {
        alcance_de(pr)
            .map(Alcance::Carpeta)
            .or_else(|| activos_de(pr).map(Alcance::Activos))
    }

    /// Cómo se dice en un mensaje: la carpeta, o cuántos activos.
    fn describe(&self) -> String {
        match self {
            Alcance::Carpeta(c) => c.clone(),
            Alcance::Activos(a) if a.len() == 1 => a[0].clone(),
            Alcance::Activos(a) => format!("{} activos", a.len()),
        }
    }
}

/// ¿Lleva la propuesta algo que no es la rama entera?
fn con_alcance(pr: &Json) -> bool {
    Alcance::de(pr).is_some()
}

/// Una lista de textos del cuerpo JSON de la petición.
fn lista_del_cuerpo(cuerpo: &str, k: &str) -> Option<Vec<String>> {
    let n = ore_core::parse::parse(cuerpo).ok()?;
    let (_, v) = n.get(k)?;
    Some(
        v.items()
            .iter()
            .filter_map(|x| x.as_str().map(|s| s.trim().to_string()))
            .filter(|s| !s.is_empty())
            .collect(),
    )
}

/// Los nombres entre comillas invertidas de un diagnóstico: `hr.cons`, `pais`…
fn nombrados(mensaje: &str) -> Vec<String> {
    mensaje
        .split('`')
        .enumerate()
        .filter(|(i, _)| i % 2 == 1)
        .map(|(_, s)| s.to_string())
        .collect()
}

/// Un nombre corto y estable para la derivada de unos activos: FNV-1a de sus ids.
fn huella_de_ids(ids: &[String]) -> String {
    let mut h: u32 = 0x811c_9dc5;
    for b in ids.join("\n").bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    format!("{h:08x}")
}

/// La derivada de `rama` para `alcance` (0044 A.2 ③): una rama técnica, fuera
/// del selector, que sólo escribe este servidor.
fn derivada_de(rama: &str, alcance: &str) -> String {
    format!(
        "{PREFIJO_DERIVADA}{rama}/{}",
        alcance.strip_prefix("packages/").unwrap_or(alcance)
    )
}

pub(crate) const PREFIJO_DERIVADA: &str = "alcance/";

fn descripcion_de(pr: &Json) -> String {
    campo(pr, "body")
        .map(|b| {
            b.lines()
                .skip_while(|l| CABECERA.iter().any(|c| l.starts_with(c)))
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
pub(crate) fn propuesta_de(pr: &Json) -> Json {
    let rama = rama_de(pr);
    let alcance = alcance_de(pr);
    let activos = activos_de(pr);
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
            "alcance",
            alcance.as_ref().map(Json::s).unwrap_or(Json::Bool(false)),
        ),
        (
            "activos",
            activos
                .as_ref()
                .map(|a| Json::Arr(a.iter().map(Json::s).collect()))
                .unwrap_or(Json::Bool(false)),
        ),
        (
            "derivada",
            if con_alcance(pr) {
                Json::s(cabeza_de_pr(pr))
            } else {
                Json::Bool(false)
            },
        ),
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
    let huella = cuerpo
        .lines()
        .nth(1)
        .and_then(|l| l.strip_prefix("huella: "))
        .map(|h| h.trim().to_string());
    let salta = if huella.is_some() { 2 } else { 1 };
    Some(Json::obj([
        ("por", Json::s(persona)),
        ("veredicto", Json::s(veredicto)),
        (
            "texto",
            Json::s(
                cuerpo
                    .lines()
                    .skip(salta)
                    .collect::<Vec<_>>()
                    .join("\n")
                    .trim(),
            ),
        ),
        ("huella", huella.map(Json::s).unwrap_or(Json::Bool(false))),
        (
            "cuando",
            Json::s(campo(r, "submitted_at").unwrap_or_default()),
        ),
    ]))
}

impl Servidor {
    pub(crate) fn api(&self) -> Result<&Api, Respuesta> {
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

    /// La política de las ramas (`politica.rs`), **siempre de la rama por
    /// defecto**. Por la API de la forja (un fichero, sin clonar) o del
    /// directorio del banco.
    pub(crate) fn politica_de_main(
        &self,
        por_defecto: &str,
    ) -> Result<crate::politica::Politica, Fallo> {
        use crate::politica::{Politica, RUTA};
        match (&self.arbol, self.forja_api.as_ref()) {
            (Arbol::Directorio(d), _) => Ok(Politica::de_raiz(d)),
            (Arbol::Forja(_), Some(api)) => Ok(api
                .fichero(RUTA, por_defecto)?
                .map(|t| Politica::de_texto(&t))
                .unwrap_or_default()),
            (Arbol::Forja(_), None) => Err(Fallo {
                codigo: 422,
                mensaje: "el árbol no está en una forja con API".into(),
            }),
        }
    }

    /// `PUT /ramas/{rama}/proteccion {protegida}` (P1.4): proteger `main`, o
    /// dejarla libre. Sólo la rama por defecto: la política es suya.
    ///
    /// - **Proteger una `main` libre**: se escribe `.arbol/ramas.yaml` en `main`
    ///   y ya. Con `main` libre cualquiera escribe en ella, así que no hay nada
    ///   que pedir. `200`.
    /// - **Liberar una `main` protegida** (regla (b)): NO se escribe. Se abre una
    ///   propuesta con sólo ese fichero —una rama `<persona>/libera-main` desde
    ///   `main`— que fusionará otra persona con su aprobación, como cualquier
    ///   cambio de una `main` protegida. Si no, protegerla no protegería nada:
    ///   quien quisiera saltársela la quitaría primero. `202` con la propuesta.
    /// - Pedir lo que ya es: `200`, nada escrito.
    ///
    /// ✏️ Hasta P2 lo puede pedir cualquiera con sesión (igual que hoy cualquiera
    ///   escribe en una `main` libre); con P2, sólo quien tenga la potestad.
    pub(crate) fn proteger(&self, sujeto: &Identidad, rama: &str, cuerpo: &str) -> Respuesta {
        use crate::politica::RUTA;
        let api = match self.api() {
            Ok(a) => a,
            Err(r) => return r,
        };
        let por_defecto = api.rama_por_defecto().unwrap_or_else(|_| "main".into());
        if rama != por_defecto {
            return Respuesta::error(
                422,
                format!("sólo `{por_defecto}` se protege: `{rama}` es una rama de trabajo"),
            );
        }
        let quiere = match del_cuerpo(cuerpo, "protegida").as_deref() {
            Some("true") => true,
            Some("false") => false,
            _ => return Respuesta::error(422, "`protegida` es `true` o `false`"),
        };
        let esta = match self.politica_de_main(&por_defecto) {
            Ok(p) => p.protegida,
            Err(e) => return de_la_forja(e),
        };
        let texto = format!(
            "# Cómo se trabaja en las ramas de este árbol. Lo lee ore-serve, siempre de `{por_defecto}`.\n# Protegida: no se escribe en ella sin propuesta, y fusionar exige la aprobación de otra persona.\n{por_defecto}:\n  protegida: {quiere}\n"
        );
        let hecho = |extra: Vec<(&'static str, Json)>| {
            let mut campos = vec![
                ("rama", Json::s(&por_defecto)),
                ("protegida", Json::Bool(quiere)),
            ];
            campos.extend(extra);
            Respuesta::ok(Json::obj(campos))
        };
        if esta == quiere {
            return hecho(vec![("cambiada", Json::Bool(false))]);
        }
        if quiere {
            // Libre → protegida: con `main` libre se escribe en ella (y
            // `escribiendo` a secas: la regla de `escribiendo_en` no aplica
            // a lo que la crea).
            let r = self.escribiendo(sujeto, &format!("Proteger `{por_defecto}`"), |raiz| {
                let f = raiz.join(RUTA);
                if let Some(d) = f.parent()
                    && std::fs::create_dir_all(d).is_err()
                {
                    return Respuesta::error(500, "no se pudo crear `.arbol/`");
                }
                match std::fs::write(&f, &texto) {
                    Ok(()) => Respuesta::ok(Json::obj([])),
                    Err(e) => Respuesta::error(500, format!("no se pudo escribir `{RUTA}`: {e}")),
                }
            });
            if r.codigo >= 300 {
                return r;
            }
            let commit = campo(&r.cuerpo, "commit")
                .map(Json::s)
                .unwrap_or(Json::Bool(false));
            return hecho(vec![("cambiada", Json::Bool(true)), ("commit", commit)]);
        }
        // Protegida → libre: una propuesta con sólo la política.
        let libera = format!("{}/libera-{por_defecto}", prefijo_de(&sujeto.persona));
        let abiertas = api.pulls("open").unwrap_or_default();
        if let Some(n) = abiertas
            .iter()
            .find(|pr| rama_de(pr) == libera)
            .and_then(|pr| numero(pr, "number"))
        {
            return Respuesta::error(
                409,
                format!("ya hay una propuesta para dejar `{por_defecto}` libre: la #{n}"),
            );
        }
        // Una rama vieja con ese nombre y sin propuesta se rehace desde `main`.
        let _ = api.borrar_rama(&libera);
        if let Err(e) = api.crear_rama(&libera, &por_defecto) {
            return de_la_forja(e);
        }
        let r = self.escribiendo_en(
            Some(&libera),
            sujeto,
            &format!("Dejar `{por_defecto}` libre"),
            |raiz| match std::fs::write(raiz.join(RUTA), &texto) {
                Ok(()) => Respuesta::ok(Json::obj([])),
                Err(e) => Respuesta::error(500, format!("no se pudo escribir `{RUTA}`: {e}")),
            },
        );
        if r.codigo >= 300 {
            return r;
        }
        let pedida = Json::obj([
            ("rama", Json::s(&libera)),
            (
                "titulo",
                Json::s(format!("Leave `{por_defecto}` unprotected")),
            ),
            (
                "descripcion",
                Json::s(format!(
                    "`{por_defecto}` is protected: leaving it unprotected needs the approval of another person, like any other change to it."
                )),
            ),
        ]);
        let mut p = self.proponer(sujeto, &pedida.jcs());
        if p.codigo == 201 {
            p.codigo = 202;
            if let Json::Obj(m) = &mut p.cuerpo {
                m.insert("protegida".into(), Json::Bool(true));
                m.insert("liberar".into(), Json::Bool(true));
            }
        }
        p
    }

    // ── Ramas ──────────────────────────────────────────────────────────────

    /// `GET /ramas`: las ramas del árbol, qué propuesta abierta tiene cada una,
    /// y si la de por defecto está protegida (`politica.rs`).
    pub(crate) fn ramas(&self) -> Respuesta {
        let api = match self.api() {
            Ok(a) => a,
            Err(r) => return r,
        };
        let por_defecto = api.rama_por_defecto().unwrap_or_else(|_| "main".into());
        let abiertas = api.pulls("open").unwrap_or_default();
        // ⛔ Una política que no se pudo leer NO se pinta libre: sería decir que
        //   se puede escribir en `main` sin saberlo. Se dice que falló.
        let protegida = match self.politica_de_main(&por_defecto) {
            Ok(p) => Json::Bool(p.protegida),
            Err(e) => return de_la_forja(e),
        };
        match api.ramas() {
            Ok(ramas) => Respuesta::ok(Json::obj([
                ("porDefecto", Json::s(&por_defecto)),
                (
                    "ramas",
                    Json::Arr(
                        ramas
                            .into_iter()
                            .filter(|(nombre, _)| !nombre.starts_with(PREFIJO_DERIVADA))
                            .map(|(nombre, commit)| {
                                let propuesta = abiertas
                                    .iter()
                                    .find(|pr| rama_de(pr) == nombre)
                                    .and_then(|pr| numero(pr, "number"));
                                Json::obj([
                                    ("nombre", Json::s(&nombre)),
                                    ("porDefecto", Json::Bool(nombre == por_defecto)),
                                    (
                                        "protegida",
                                        if nombre == por_defecto {
                                            protegida.clone()
                                        } else {
                                            Json::Bool(false)
                                        },
                                    ),
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
        if let Some(pr) = api
            .pulls("open")
            .unwrap_or_default()
            .iter()
            .find(|pr| rama_de(pr) == nombre)
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

    /// `POST /ramas/{rama}/fusionar {desde}`: traer OTRA rama a ésta (el «Merge»
    /// del menú del workspace). Es `git merge --no-ff` en un clon de la rama,
    /// con el gate de siempre —el árbol resultante no empeora, o nada se
    /// empuja— y el empujón. **`main` no**: lo que llega a `main` llega por una
    /// propuesta revisada por otra persona; aquí se contesta 422 y se dice.
    pub(crate) fn fusionar_en_rama(
        &self,
        sujeto: &Identidad,
        rama: &str,
        cuerpo: &str,
    ) -> Respuesta {
        let Some(forja) = self.forja() else {
            return Respuesta::error(
                422,
                "este árbol es un directorio, no una forja: no hay ramas que fusionar",
            );
        };
        if let Err(m) = nombre_de_rama_valido(rama) {
            return Respuesta::error(422, m);
        }
        let Some(desde) = del_cuerpo(cuerpo, "desde").filter(|d| !d.is_empty()) else {
            return Respuesta::error(422, "falta `desde`: la rama que se trae");
        };
        if let Err(m) = nombre_de_rama_valido(&desde) {
            return Respuesta::error(422, m);
        }
        if desde == rama {
            return Respuesta::error(
                422,
                format!("`{rama}` ya es `{rama}`: no hay nada que traer"),
            );
        }
        let por_defecto = self
            .api()
            .ok()
            .and_then(|a| a.rama_por_defecto().ok())
            .unwrap_or_else(|| "main".into());
        if rama == por_defecto {
            return Respuesta::error(
                422,
                format!(
                    "a `{por_defecto}` no se le trae una rama a mano: se propone, otra persona la revisa y se fusiona (Pull requests)"
                ),
            );
        }
        let prestado = match forja.clonar_rama(Some(rama)) {
            Err(crate::git::Fallo::SinRama(r)) => {
                return Respuesta::error(404, format!("no hay ninguna rama `{r}`"));
            }
            Err(e) => return Respuesta::error(502, e.to_string()),
            Ok(p) => p,
        };
        let raiz = prestado.ruta();
        let antes = match self.diagnosticos_de(raiz) {
            Ok(a) => a,
            Err(r) => return r,
        };
        let mensaje = format!("Traer `{desde}` a `{rama}` ({})", sujeto.persona);
        match forja.traer(raiz, &desde, sujeto, &mensaje) {
            Ok(false) => {
                return Respuesta::ok(Json::obj([
                    ("rama", Json::s(rama)),
                    ("desde", Json::s(&desde)),
                    ("fusionada", Json::Bool(false)),
                    (
                        "dice",
                        Json::s(format!("`{rama}` ya tiene todo lo de `{desde}`")),
                    ),
                ]));
            }
            Ok(true) => {}
            Err(crate::git::Fallo::SinRama(r)) => {
                return Respuesta::error(404, format!("no hay ninguna rama `{r}`"));
            }
            Err(e @ crate::git::Fallo::Conflicto(_)) => {
                return Respuesta::error(409, e.to_string());
            }
            Err(e) => return Respuesta::error(502, e.to_string()),
        }
        if let Err(mut r) = self.empeora(raiz, &antes, &format!("traer `{desde}`")) {
            if let Json::Obj(m) = &mut r.cuerpo
                && let Some(Json::Arr(ds)) = m.get("diagnosticos").cloned()
            {
                m.insert(
                    "diagnosticos".into(),
                    Json::Arr(ds.iter().map(crate::arbol::con_posicion).collect()),
                );
            }
            // El merge se queda en el clon, y el clon se tira.
            return r;
        }
        match forja.empujar(raiz) {
            Ok(commit) => Respuesta::ok(Json::obj([
                ("rama", Json::s(rama)),
                ("desde", Json::s(&desde)),
                ("fusionada", Json::Bool(true)),
                ("commit", Json::s(commit)),
                ("por", Json::s(&sujeto.persona)),
            ])),
            Err(crate::git::Fallo::Adelantado(m)) => {
                Respuesta::error(409, crate::git::Fallo::Adelantado(m).to_string())
            }
            Err(e) => Respuesta::error(502, e.to_string()),
        }
    }

    // ── Propuestas ─────────────────────────────────────────────────────────

    /// `GET /propuestas`: todas, con su estado (`abierta`, `fusionada`, `cerrada`).
    /// `GET /propuestas`, y con `alcance` (0036 ④) **sólo las de un
    /// repositorio**: las que tocan algún fichero bajo su carpeta.
    ///
    /// El filtro mira **los ficheros**, no el nombre de la rama: una rama se
    /// llama como quien la abrió quiera, y la pregunta es «¿esta propuesta
    /// cambia lo mío?». Cuesta una llamada por propuesta abierta, así que sólo
    /// se paga cuando se pide, y se dice en la respuesta (`alcance`).
    pub(crate) fn propuestas(&self, alcance: Option<&str>) -> Respuesta {
        let api = match self.api() {
            Ok(a) => a,
            Err(r) => return r,
        };
        let prs = match api.pulls("all") {
            Ok(p) => p,
            Err(e) => return de_la_forja(e),
        };
        let Some(dentro) = alcance.map(str::trim).filter(|s| !s.is_empty()) else {
            return Respuesta::ok(Json::obj([(
                "propuestas",
                Json::Arr(prs.iter().map(propuesta_de).collect()),
            )]));
        };
        let prefijo = format!("{}/", dentro.trim_matches('/'));
        let mut suyas = Vec::new();
        for pr in &prs {
            let n = numero(pr, "number").unwrap_or(0) as u64;
            // Si la forja no sabe decir qué ficheros toca, la propuesta NO se
            // esconde: es mejor enseñar de más que callar un cambio que sí es
            // tuyo. Se dice cuál no se pudo mirar.
            let toca = match api.ficheros(n) {
                Ok(fs) => fs.iter().any(|f| {
                    campo(f, "filename")
                        .is_some_and(|r| r.starts_with(&prefijo) || r == dentro.trim_matches('/'))
                }),
                Err(_) => true,
            };
            if toca {
                suyas.push(propuesta_de(pr));
            }
        }
        Respuesta::ok(Json::obj([
            ("propuestas", Json::Arr(suyas)),
            ("alcance", Json::s(dentro.trim_matches('/'))),
        ]))
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
        let alcance = match crate::entorno::alcance_valido(del_cuerpo(cuerpo, "alcance").as_deref())
        {
            Ok(a) => a,
            Err(r) => return r,
        };
        let activos = lista_del_cuerpo(cuerpo, "activos");
        let alcance = match (alcance, activos) {
            (Some(_), Some(_)) => {
                return Respuesta::error(
                    422,
                    "`alcance` (una carpeta) o `activos`, no los dos: son dos propuestas",
                );
            }
            (_, Some(a)) if a.is_empty() => {
                return Respuesta::error(422, "`activos` vacío: no hay qué proponer");
            }
            (Some(c), None) => Some(Alcance::Carpeta(c)),
            (None, Some(a)) => Some(Alcance::Activos(a)),
            (None, None) => None,
        };
        let abiertas = api.pulls("open").unwrap_or_default();
        if let Some(alcance) = alcance {
            return self.proponer_alcance(
                sujeto,
                &rama,
                &base,
                alcance,
                titulo.trim(),
                descripcion.trim(),
                &abiertas,
            );
        }
        // La rama entera lleva lo que ya va en sus propuestas con alcance: una
        // unidad, una propuesta abierta (0044 A.2 ②).
        if let Some(pr) = abiertas
            .iter()
            .find(|pr| con_alcance(pr) && rama_de(pr) == rama)
        {
            return Respuesta::error(
                409,
                format!(
                    "`{rama}` ya tiene la propuesta #{} con alcance `{}`: proponer la rama entera lo llevaría dos veces",
                    numero(pr, "number").unwrap_or(0),
                    Alcance::de(pr).map(|a| a.describe()).unwrap_or_default()
                ),
            );
        }
        let cuerpo_pr = format!("sub: {}\n\n{}", sujeto.persona, descripcion.trim());
        match api.abrir_pull(&rama, &base, titulo.trim(), &cuerpo_pr) {
            Ok(pr) => Respuesta::creado(propuesta_de(&pr)),
            Err(e) => de_la_forja(e),
        }
    }

    /// **Lo que un alcance lleva, en ficheros** (0044 A.2): una carpeta tal
    /// cual; unos activos, por `GET /ramas/{r}/cambios` —su `ruta` y, si se
    /// movieron, su `rutaAntes`—, más **lo que va con ellos sin remedio**:
    /// otro documento cambiado que comparte fichero, y el `package.yaml` de su
    /// base si la rama lo cambió (la versión del paquete va con sus activos).
    ///
    /// Devuelve el alcance para git, los ids que lleva (los pedidos y los que
    /// van con ellos) y cuáles se añadieron. `estricto` es proponer: un id que
    /// la rama no cambia es un `422`, y se añade lo que va con ellos. Si no (una
    /// propuesta ya abierta), los ids son los guardados, y uno que la rama ya
    /// no cambia se ignora (lo deshizo).
    fn en_ficheros(
        &self,
        rama: &str,
        alcance: &Alcance,
        estricto: bool,
    ) -> Result<(crate::git::Alcance, Vec<String>, Vec<String>), Respuesta> {
        let pedidos = match alcance {
            Alcance::Carpeta(c) => {
                return Ok((
                    crate::git::Alcance::Carpeta(c.clone()),
                    Vec::new(),
                    Vec::new(),
                ));
            }
            Alcance::Activos(a) => a,
        };
        let cambios = self.cambios_de(rama)?;
        let id_de = |c: &Json| campo(c, "id").unwrap_or_default();
        let rutas_de = |c: &Json| -> Vec<String> {
            [campo(c, "ruta"), campo(c, "rutaAntes")]
                .into_iter()
                .flatten()
                .collect()
        };
        let mut llevo: Vec<String> = Vec::new();
        for x in pedidos {
            match cambios
                .iter()
                .find(|c| id_de(c) == *x || campo(c, "ref").as_deref() == Some(x.as_str()))
            {
                Some(c) => {
                    if !llevo.contains(&id_de(c)) {
                        llevo.push(id_de(c));
                    }
                }
                None if estricto => {
                    return Err(Respuesta::error(
                        422,
                        format!(
                            "`{x}` no es un activo que `{rama}` cambie: no hay qué proponer de él"
                        ),
                    ));
                }
                None => {}
            }
        }
        let pedidos_ids = llevo.clone();
        // Lo que va con ellos, hasta que no se añade nada. SÓLO al proponer: la
        // propuesta guarda el conjunto ya completo, y lo que la rama cambie
        // después (otra versión del paquete) no se cuela en lo ya revisado.
        if estricto {
            loop {
                let rutas: Vec<String> = cambios
                    .iter()
                    .filter(|c| llevo.contains(&id_de(c)))
                    .flat_map(rutas_de)
                    .collect();
                let bases: Vec<String> = cambios
                    .iter()
                    .filter(|c| llevo.contains(&id_de(c)))
                    .filter_map(|c| campo(c, "nombre"))
                    .filter_map(|n| n.split('.').next().map(str::to_string))
                    .collect();
                let nuevos: Vec<String> = cambios
                    .iter()
                    .filter(|c| !llevo.contains(&id_de(c)))
                    .filter(|c| {
                        rutas_de(c).iter().any(|r| rutas.contains(r))
                            || (campo(c, "kind").as_deref() == Some("Package")
                                && campo(c, "nombre").is_some_and(|n| bases.contains(&n)))
                    })
                    .map(id_de)
                    .collect();
                if nuevos.is_empty() {
                    break;
                }
                llevo.extend(nuevos);
            }
        }
        let mut rutas: Vec<String> = cambios
            .iter()
            .filter(|c| llevo.contains(&id_de(c)))
            .flat_map(rutas_de)
            .collect();
        rutas.sort();
        rutas.dedup();
        let anadidos = llevo
            .iter()
            .filter(|i| !pedidos_ids.contains(i))
            .cloned()
            .collect();
        llevo.sort();
        Ok((crate::git::Alcance::Rutas(rutas), llevo, anadidos))
    }

    /// Los cambios de la rama frente a `main` (`GET /ramas/{r}/cambios`), la lista.
    fn cambios_de(&self, rama: &str) -> Result<Vec<Json>, Respuesta> {
        let r = self.cambios(rama);
        if r.codigo >= 300 {
            return Err(r);
        }
        Ok(match hijo(&r.cuerpo, "cambios") {
            Some(Json::Arr(v)) => v.clone(),
            _ => Vec::new(),
        })
    }

    /// **Lo que le falta al alcance** para compilar sobre `main`: los activos
    /// que un diagnóstico nombra, que la rama cambia y que el alcance no lleva.
    /// Es la sugerencia de «añádelo» (0044 A.2 ④). Un nombre que la rama NO
    /// cambia no está aquí: eso es que el alcance rompería `main`.
    fn faltan(&self, rama: &str, diagnosticos: &[Json], lleva: &[String]) -> Vec<String> {
        let Ok(cambios) = self.cambios_de(rama) else {
            return Vec::new();
        };
        let mut faltan: Vec<String> = Vec::new();
        for d in diagnosticos {
            for nombre in nombrados(&campo(d, "mensaje").unwrap_or_default()) {
                for c in &cambios {
                    let id = campo(c, "id").unwrap_or_default();
                    let es = campo(c, "nombre").as_deref() == Some(nombre.as_str())
                        || id.ends_with(&format!(":{nombre}"));
                    if es && !lleva.contains(&id) && !faltan.contains(&id) {
                        faltan.push(id);
                    }
                }
            }
        }
        faltan
    }

    /// **Proponer sólo una parte de la rama** (0044 A.2): lo de un repositorio
    /// (E1) o unos activos (E2). La PR sale de la derivada —`base` de hoy más
    /// esa parte—, así que la forja enseña, valida y fusiona justo eso; lo
    /// demás sigue en la rama.
    #[allow(clippy::too_many_arguments)]
    fn proponer_alcance(
        &self,
        sujeto: &Identidad,
        rama: &str,
        base: &str,
        alcance: Alcance,
        titulo: &str,
        descripcion: &str,
        abiertas: &[Json],
    ) -> Respuesta {
        let (Ok(api), Some(forja)) = (self.api(), self.forja()) else {
            return Respuesta::error(422, "este árbol no está en una forja: no hay propuestas");
        };
        let (en_git, lleva, anadidos) = match self.en_ficheros(rama, &alcance, true) {
            Ok(x) => x,
            Err(r) => return r,
        };
        // Una cosa, una propuesta abierta: la rama entera choca con todo; una
        // carpeta, con la misma carpeta; unos activos, con los que comparta.
        let choca = |pr: &&Json| {
            rama_de(pr) == rama
                && match (Alcance::de(pr), &alcance) {
                    (None, _) => true,
                    (Some(Alcance::Carpeta(a)), Alcance::Carpeta(b)) => a == *b,
                    (Some(Alcance::Activos(a)), Alcance::Activos(_)) => {
                        a.iter().any(|x| lleva.contains(x))
                    }
                    _ => false,
                }
        };
        if let Some(pr) = abiertas.iter().find(choca) {
            return Respuesta::error(
                409,
                format!(
                    "`{}` de `{rama}` ya va en la propuesta #{}: una cosa, una propuesta abierta",
                    alcance.describe(),
                    numero(pr, "number").unwrap_or(0)
                ),
            );
        }
        let derivada = match &alcance {
            Alcance::Carpeta(c) => derivada_de(rama, c),
            Alcance::Activos(_) => {
                format!("{PREFIJO_DERIVADA}{rama}/activos-{}", huella_de_ids(&lleva))
            }
        };
        if let Err(m) = nombre_de_rama_valido(&derivada) {
            return Respuesta::error(422, m);
        }
        let hecha = match forja.derivar(base, rama, &en_git, sujeto, titulo) {
            Ok(Some(d)) => d,
            Ok(None) => {
                return Respuesta::error(
                    422,
                    format!(
                        "`{rama}` no cambia nada en `{}`: no hay qué proponer",
                        alcance.describe()
                    ),
                );
            }
            Err(e) => return de_git(e),
        };
        // Lo que ya se sabe: si `main` + esto compila, y si no, qué le falta.
        let diagnosticos = self.diagnosticos_de(hecha.clon.ruta()).unwrap_or_default();
        let faltan = if diagnosticos.is_empty() {
            Vec::new()
        } else {
            self.faltan(rama, &diagnosticos, &lleva)
        };
        if let Err(e) = forja.empujar_a(hecha.clon.ruta(), &derivada) {
            return de_git(e);
        }
        let linea = match &alcance {
            Alcance::Carpeta(c) => format!("alcance: {c}"),
            Alcance::Activos(_) => format!("activos: {}", lleva.join(", ")),
        };
        let cuerpo_pr = format!(
            "sub: {}\nrama: {rama}\n{linea}\n\n{descripcion}",
            sujeto.persona
        );
        match api.abrir_pull(&derivada, base, titulo, &cuerpo_pr) {
            Ok(pr) => {
                let mut j = propuesta_de(&pr);
                if let Json::Obj(m) = &mut j {
                    m.insert(
                        "ficherosDelAlcance".into(),
                        Json::Arr(hecha.ficheros.iter().map(Json::s).collect()),
                    );
                    // Lo del catálogo que la rama cambia en la carpeta se queda
                    // en la rama: se propone desde el catálogo (0044 A.2 ①).
                    m.insert(
                        "documentosFuera".into(),
                        Json::Arr(hecha.fuera.iter().map(Json::s).collect()),
                    );
                    // Lo que va con los activos pedidos sin remedio, dicho.
                    m.insert(
                        "anadidos".into(),
                        Json::Arr(anadidos.iter().map(Json::s).collect()),
                    );
                    m.insert(
                        "diagnosticos".into(),
                        Json::Arr(
                            diagnosticos
                                .iter()
                                .map(crate::arbol::con_posicion)
                                .collect(),
                        ),
                    );
                    m.insert(
                        "faltan".into(),
                        Json::Arr(faltan.iter().map(Json::s).collect()),
                    );
                }
                Respuesta::creado(j)
            }
            Err(e) => {
                let _ = api.borrar_rama(&derivada);
                de_la_forja(e)
            }
        }
    }

    /// `GET /propuestas/{n}`: la propuesta con sus ficheros, el diff de líneas
    /// (la forja), el diff de significado (`ore diff main rama`), los
    /// diagnósticos de la rama y las revisiones. Y `fusion: {puede, porque}`:
    /// si QUIEN MIRA puede fusionarla según la política de `main`, dicho antes
    /// de pulsar con la misma regla que `fusionar` (`quien_fusiona`). Sólo las
    /// personas: conflictos y diagnósticos van aparte, en sus campos.
    pub(crate) fn propuesta(&self, sujeto: &Identidad, n: u64) -> Respuesta {
        let api = match self.api() {
            Ok(a) => a,
            Err(r) => return r,
        };
        let pr = match api.pull(n) {
            Ok(p) => p,
            Err(e) => return de_la_forja(e),
        };
        let mut ficha = propuesta_de(&pr);
        // Lo que la forja compara y valida es la cabeza de la PR: la rama, o
        // su derivada.
        let rama = cabeza_de_pr(&pr);
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
        // Con alcance: ¿la derivada lleva lo que la rama tiene HOY en el
        // alcance?, y una revisión vale si se hizo sobre lo que hay ahora.
        let alcance = Alcance::de(&pr).filter(|_| estado_de(&pr) == "abierta");
        let en_git = alcance
            .as_ref()
            .and_then(|a| self.en_ficheros(&rama_de(&pr), a, false).ok());
        let hecha = match &en_git {
            Some((g, _, _)) => self
                .forja()
                .and_then(|f| f.huellas(&base, &rama_de(&pr), &rama, g).ok()),
            None => None,
        };
        let revisiones: Vec<Json> = api
            .revisiones(n)
            .unwrap_or_default()
            .iter()
            .filter_map(revision_de)
            .map(|mut r| {
                if let (Some((_, h)), Json::Obj(m)) = (&hecha, &mut r) {
                    let vale =
                        campo(&Json::Obj(m.clone()), "huella").as_deref() == Some(h.as_str());
                    m.insert("vigente".into(), Json::Bool(vale));
                }
                r
            })
            .collect();
        if let (Some((hoy, h)), Json::Obj(m)) = (&hecha, &mut ficha) {
            m.insert("alDia".into(), Json::Bool(hoy == h));
        }
        // El significado y los diagnósticos salen de la rama misma, no de la forja.
        let (semantico, diagnosticos) = if estado_de(&pr) == "abierta" {
            self.rama_frente_a(&rama, &base)
        } else {
            (Json::Bool(false), Json::Arr(vec![]))
        };
        // Con activos: lo que le falta para compilar sobre `main`, si algo.
        if let (Some((_, lleva, _)), Json::Arr(ds), Json::Obj(m)) =
            (&en_git, &diagnosticos, &mut ficha)
            && matches!(alcance, Some(Alcance::Activos(_)))
        {
            let faltan = self.faltan(&rama_de(&pr), ds, lleva);
            m.insert(
                "faltan".into(),
                Json::Arr(faltan.iter().map(Json::s).collect()),
            );
        }
        let fusion = if estado_de(&pr) != "abierta" {
            Err(format!("la propuesta está {}", estado_de(&pr)))
        } else {
            let autor = autor_de(&pr);
            let aprobada_por: Vec<String> = revisiones
                .iter()
                .filter(|r| campo(r, "veredicto").as_deref() == Some("aprueba"))
                .filter(|r| booleano(r, "vigente") != Some(false))
                .filter_map(|r| campo(r, "por"))
                .filter(|p| *p != autor)
                .collect();
            self.politica_de_main(&base)
                .map_err(|e| format!("no se pudo leer la política de `{base}`: {e}"))
                .and_then(|p| {
                    quien_fusiona(p.protegida, &autor, &sujeto.persona, &aprobada_por)
                        .map(|()| aprobada_por.is_empty())
                })
        };
        if let Json::Obj(m) = &mut ficha {
            m.insert(
                "fusion".into(),
                match fusion {
                    Ok(sin_revision) => Json::obj([
                        ("puede", Json::Bool(true)),
                        ("sinRevision", Json::Bool(sin_revision)),
                    ]),
                    Err(porque) => {
                        Json::obj([("puede", Json::Bool(false)), ("porque", Json::s(porque))])
                    }
                },
            );
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
        // Con alcance, la revisión dice SOBRE QUÉ se hizo: la huella de la
        // derivada. Si la rama cambia lo propuesto, esa aprobación deja de valer.
        let huella = match (Alcance::de(&pr), self.forja()) {
            (Some(a), Some(f)) => self
                .en_ficheros(&rama_de(&pr), &a, false)
                .ok()
                .and_then(|(g, _, _)| {
                    f.huellas(&base_de(&pr), &rama_de(&pr), &cabeza_de_pr(&pr), &g)
                        .ok()
                })
                .map(|(_, h)| format!("huella: {h}\n")),
            _ => None,
        }
        .unwrap_or_default();
        let cuerpo_review = format!(
            "revision: {} {veredicto}\n{huella}\n{}",
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
        // ⛔ La política de `main`, y si no se puede leer no se fusiona: tomarla
        //   por libre sería saltarse una protección que quizá está.
        let protegida = match self.politica_de_main(&base_de(&pr)) {
            Ok(p) => p.protegida,
            Err(e) => return de_la_forja(e),
        };
        if let Some(alcance) = Alcance::de(&pr) {
            return self.fusionar_alcance(sujeto, n, &pr, &alcance, protegida);
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
        if let Err(m) = quien_fusiona(protegida, &autor, &sujeto.persona, &aprobada_por) {
            return Respuesta::error(422, m);
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
        let mensaje = mensaje_de_fusion(
            n,
            &autor,
            "",
            &aprobada_por,
            &sujeto.persona,
            &campo(&pr, "title").unwrap_or_default(),
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
                ("sinRevision", Json::Bool(aprobada_por.is_empty())),
                ("rama", Json::s(&rama)),
            ])),
            Err(e) => de_la_forja(e),
        }
    }

    /// **Fusionar una propuesta con alcance** (0044 A.2 ④ y ⑥):
    ///
    /// 1. si la rama cambió lo propuesto desde que se hizo la derivada, la
    ///    derivada se regenera y se contesta `409`: lo nuevo no lo ha visto nadie;
    /// 2. vale la aprobación de otra persona **sobre lo que hay ahora** (su huella);
    /// 3. se regenera sobre el `main` de hoy y se valida ESO —`main` más el
    ///    alcance, no la rama—; un `OOS2018` dice qué falta en el alcance;
    /// 4. la forja fusiona la derivada (un `merge`: atómico) y la borra;
    /// 5. `main` se trae a la rama, para que lo fusionado deje de contar como
    ///    suyo. Si eso choca, la fusión ya es buena y se dice.
    fn fusionar_alcance(
        &self,
        sujeto: &Identidad,
        n: u64,
        pr: &Json,
        alcance: &Alcance,
        protegida: bool,
    ) -> Respuesta {
        let (Ok(api), Some(forja)) = (self.api(), self.forja()) else {
            return Respuesta::error(422, "este árbol no está en una forja: no hay propuestas");
        };
        let autor = autor_de(pr);
        let rama = rama_de(pr);
        let derivada = cabeza_de_pr(pr);
        let base = hijo(pr, "base")
            .and_then(|h| campo(h, "ref"))
            .unwrap_or_else(|| "main".into());
        let titulo = campo(pr, "title").unwrap_or_default();
        let (en_git, lleva, _) = match self.en_ficheros(&rama, alcance, false) {
            Ok(x) => x,
            Err(r) => return r,
        };
        let alcance = alcance.describe();
        let (hoy, hecha) = match forja.huellas(&base, &rama, &derivada, &en_git) {
            Ok(h) => h,
            Err(e) => return de_git(e),
        };
        let nueva = match forja.derivar(&base, &rama, &en_git, sujeto, &titulo) {
            Ok(Some(d)) => d,
            Ok(None) => {
                return Respuesta::error(
                    409,
                    format!(
                        "`{rama}` ya no cambia nada en `{alcance}`: la propuesta #{n} se cierra, no se fusiona"
                    ),
                );
            }
            Err(e) => return de_git(e),
        };
        if hoy != hecha {
            if let Err(e) = forja.empujar_a(nueva.clon.ruta(), &derivada) {
                return de_git(e);
            }
            return Respuesta::error(
                409,
                format!(
                    "`{rama}` cambió `{alcance}` desde que se propuso: la propuesta #{n} ya lleva lo de ahora, y hay que revisarla otra vez"
                ),
            );
        }
        let aprobada_por: Vec<String> = api
            .revisiones(n)
            .unwrap_or_default()
            .iter()
            .filter_map(revision_de)
            .filter(|r| campo(r, "veredicto").as_deref() == Some("aprueba"))
            .filter(|r| campo(r, "huella").as_deref() == Some(hoy.as_str()))
            .filter_map(|r| campo(&r, "por"))
            .filter(|p| *p != autor)
            .collect();
        if let Err(m) = quien_fusiona(protegida, &autor, &sujeto.persona, &aprobada_por) {
            return Respuesta::error(422, m);
        }
        match self.diagnosticos_de(nueva.clon.ruta()) {
            Ok(ds) if !ds.is_empty() => {
                let faltan = self.faltan(&rama, &ds, &lleva);
                return Respuesta {
                    codigo: 422,
                    cuerpo: Json::obj([
                        ("faltan", Json::Arr(faltan.iter().map(Json::s).collect())),
                        (
                            "error",
                            Json::s(format!(
                                "`{base}` con `{alcance}` de `{rama}` no compila: no se fusiona. Si lee algo que sólo está en la rama, eso también tiene que ir en la propuesta"
                            )),
                        ),
                        (
                            "diagnosticos",
                            Json::Arr(ds.iter().map(crate::arbol::con_posicion).collect()),
                        ),
                    ]),
                };
            }
            Ok(_) => {}
            Err(r) => return r,
        }
        if let Err(e) = forja.empujar_a(nueva.clon.ruta(), &derivada) {
            return de_git(e);
        }
        let mensaje = mensaje_de_fusion(
            n,
            &autor,
            &format!(" ({alcance} de {rama})"),
            &aprobada_por,
            &sujeto.persona,
            &titulo,
        );
        if let Err(e) = api.fusionar(n, &mensaje) {
            return de_la_forja(e);
        }
        let al_dia = self.poner_al_dia(sujeto, &rama, &base);
        Respuesta::ok(Json::obj([
            ("numero", Json::Int(n as i64)),
            ("fusionada", Json::Bool(true)),
            ("por", Json::s(&sujeto.persona)),
            (
                "revisada_por",
                Json::Arr(aprobada_por.iter().map(Json::s).collect()),
            ),
            ("sinRevision", Json::Bool(aprobada_por.is_empty())),
            ("rama", Json::s(&rama)),
            ("alcance", Json::s(&alcance)),
            ("ramaAlDia", al_dia),
        ]))
    }

    /// Trae `base` a la rama tras fusionar una parte de ella: `true`, o lo que
    /// lo impidió (la fusión ya ocurrió; esto sólo ordena la rama).
    fn poner_al_dia(&self, sujeto: &Identidad, rama: &str, base: &str) -> Json {
        let Some(forja) = self.forja() else {
            return Json::Bool(false);
        };
        let hecho = forja.clonar_rama(Some(rama)).and_then(|clon| {
            let mensaje = format!("Traer `{base}` a `{rama}` tras fusionar parte de ella");
            if forja.traer(clon.ruta(), base, sujeto, &mensaje)? {
                forja.empujar(clon.ruta())?;
            }
            Ok(())
        });
        match hecho {
            Ok(()) => Json::Bool(true),
            Err(e) => Json::s(e.to_string()),
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
        let res = api.cerrar_pull(n);
        // La derivada es técnica: cerrada su propuesta, sobra.
        if res.is_ok() && con_alcance(&pr) {
            let _ = api.borrar_rama(&cabeza_de_pr(&pr));
        }
        match res {
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

    /// **Lo que mueve DATOS, sólo en la rama por defecto** (ramas globales, fase
    /// 1). Copiar, ascender, rehacer la copia, decidir o dar de alta una fuente
    /// escriben la cola, y los Jobs que encolan leen la rama por defecto: hecho
    /// desde otra rama, escribiría allí sin decirlo. Los datos de una rama
    /// llegan después; hasta entonces se niega con el porqué. `None` = adelante.
    pub(crate) fn solo_en_la_de_por_defecto(
        &self,
        rama: Option<&str>,
        que: &str,
    ) -> Option<Respuesta> {
        let r = rama?;
        let por_defecto = self
            .api()
            .ok()
            .and_then(|a| a.rama_por_defecto().ok())
            .unwrap_or_else(|| "main".into());
        (r != por_defecto).then(|| {
            Respuesta::error(
                409,
                format!(
                    "{que} mueve datos, y una rama todavía no tiene datos propios: los Jobs \
                     leen `{por_defecto}`. Hazlo en `{por_defecto}` (estás en `{r}`)"
                ),
            )
        })
    }

    /// Como `escribiendo`, en la rama que diga la cabecera: el clon ES la rama,
    /// así que `publicar` empuja a ella. Sin cabecera, `main`.
    ///
    /// ⛔ **Con `main` protegida (`politica.rs`), sin cabecera es `423`.** Éste
    /// es el único sitio por el que se ESCRIBE EL ÁRBOL en `main` —editor,
    /// commit, documentos, el `CREATE VIEW` del puesto, bases, proyectos,
    /// repositorios—, así que la regla vive aquí una vez. Lo que es de la celda
    /// (fuentes, modelos, datasets, copiar a la celda) va por `escribiendo` a
    /// secas y no pasa por aquí: dar de alta una fuente sólo existe en `main`, y
    /// protegerla no puede dejar la celda sin fuentes. La política se lee del
    /// mismo clon que se va a escribir: es la de `main` en ese instante.
    pub(crate) fn escribiendo_en(
        &self,
        rama: Option<&str>,
        sujeto: &Identidad,
        mensaje: &str,
        f: impl FnOnce(&Path) -> Respuesta,
    ) -> Respuesta {
        match (rama, &self.arbol) {
            (None, _) => self.escribiendo(sujeto, mensaje, |raiz| {
                if crate::politica::Politica::de_raiz(raiz).protegida {
                    return Respuesta::error(
                        423,
                        "`main` está protegida: nada se escribió. Se trabaja en una rama y se propone",
                    );
                }
                f(raiz)
            }),
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
    #[test]
    fn quien_fusiona_segun_la_politica() {
        use super::quien_fusiona;
        let nadie: Vec<String> = vec![];
        let bea = vec!["bea".to_string()];
        // libre: la autora fusiona lo suyo, sin revisión; y cualquiera
        assert!(quien_fusiona(false, "ana", "ana", &nadie).is_ok());
        assert!(quien_fusiona(false, "ana", "bea", &nadie).is_ok());
        // protegida: dos personas y una revisión
        assert!(quien_fusiona(true, "ana", "ana", &bea).is_err());
        assert!(quien_fusiona(true, "ana", "bea", &nadie).is_err());
        assert!(quien_fusiona(true, "ana", "bea", &bea).is_ok());
    }

    #[test]
    fn el_merge_dice_si_hubo_revision() {
        use super::mensaje_de_fusion;
        assert_eq!(
            mensaje_de_fusion(3, "ana", "", &[], "ana", "t"),
            "Propuesta #3 de ana, fusionada sin revisión por ana: t"
        );
        assert_eq!(
            mensaje_de_fusion(3, "ana", " (x de r)", &["bea".into()], "cai", "t"),
            "Propuesta #3 de ana (x de r), revisada por bea y fusionada por cai: t"
        );
    }

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

    #[test]
    fn una_propuesta_con_alcance_dice_su_rama_y_su_carpeta_en_la_cabecera() {
        let pr = Json::obj([
            (
                "body",
                Json::s(
                    "sub: persona:ana
rama: ana/mixta
alcance: packages/hr/etl

el transform
rama: no es cabecera",
                ),
            ),
            (
                "head",
                Json::obj([("ref", Json::s("alcance/ana/mixta/hr/etl"))]),
            ),
        ]);
        assert_eq!(autor_de(&pr), "persona:ana");
        assert_eq!(rama_de(&pr), "ana/mixta");
        assert_eq!(alcance_de(&pr).as_deref(), Some("packages/hr/etl"));
        assert_eq!(
            descripcion_de(&pr),
            "el transform
rama: no es cabecera"
        );
        assert_eq!(
            derivada_de("ana/mixta", "packages/hr/etl"),
            "alcance/ana/mixta/hr/etl"
        );
        // sin alcance, la rama es la cabeza de la PR, como siempre
        let entera = Json::obj([
            (
                "body",
                Json::s(
                    "sub: persona:ana

x",
                ),
            ),
            ("head", Json::obj([("ref", Json::s("ana/x"))])),
        ]);
        assert_eq!(rama_de(&entera), "ana/x");
        assert!(alcance_de(&entera).is_none());
    }

    #[test]
    fn una_propuesta_de_activos_los_dice_en_la_cabecera() {
        let pr = Json::obj([
            (
                "body",
                Json::s(
                    "sub: persona:ana
rama: ana/x
activos: Package:hr, View:hr.default.a1

los dos",
                ),
            ),
            (
                "head",
                Json::obj([("ref", Json::s("alcance/ana/x/activos-0badcafe"))]),
            ),
        ]);
        assert_eq!(rama_de(&pr), "ana/x");
        assert!(alcance_de(&pr).is_none());
        assert_eq!(
            activos_de(&pr).unwrap(),
            vec!["Package:hr".to_string(), "View:hr.default.a1".to_string()]
        );
        assert!(con_alcance(&pr));
        assert_eq!(Alcance::de(&pr).unwrap().describe(), "2 activos");
        assert_eq!(descripcion_de(&pr), "los dos");
    }

    #[test]
    fn un_diagnostico_nombra_entre_comillas_invertidas() {
        assert_eq!(
            nombrados("`hr.cons` lee `pais` de `hr.pub`, que no la tiene"),
            vec!["hr.cons", "pais", "hr.pub"]
        );
        let ids = vec!["View:hr.default.a1".to_string()];
        assert_eq!(huella_de_ids(&ids), huella_de_ids(&ids.clone()));
        assert_eq!(huella_de_ids(&ids).len(), 8);
    }

    #[test]
    fn la_huella_de_una_revision_no_es_su_texto() {
        let r = Json::obj([(
            "body",
            Json::s(
                "revision: persona:bea aprueba
huella: abc

bien",
            ),
        )]);
        let v = revision_de(&r).unwrap();
        assert_eq!(campo(&v, "huella").as_deref(), Some("abc"));
        assert_eq!(campo(&v, "texto").as_deref(), Some("bien"));
    }
}
