//! **La puerta PyPI de los registros de artefactos** (ADR 0074 AR2·2).
//!
//! `uv publish` y `twine upload` publican aquí con su configuración de siempre
//! (la URL y el token, usuario `__token__`), y la capa de Python resuelve con
//! `uv` como contra PyPI. Como la de Maven ([`crate::artefactos`]): **este
//! proceso decide** y los bytes los guarda y sirve `ore-medios`, en flujo.
//!
//! ```text
//! POST /artifacts/<b>/<s>/<registro>/pypi/                       publicar (multipart, un fichero por petición)
//! GET  /artifacts/pypi/simple/<nombre>/                          el índice de la celda (PEP 503)
//! GET  /artifacts/<b>/<s>/<registro>/pypi/simple/<nombre>/       el de un registro
//! GET  /artifacts/<b>/<s>/<registro>/pypi/files/<nombre>/<versión>/<fichero>
//! ```
//!
//! Lo medido en AR2·0 que decide la forma:
//! - **los campos llegan antes que el fichero**: todo se decide antes de pasar
//!   un byte, y el fichero va el último (lo exige [`ore_entrada::multipart`]);
//! - **uv no pasa a PyPI si ORE contesta 401, 403 o 5xx**, y falla la capa
//!   entera, también para lo público: a un nombre que no es de ningún registro,
//!   o que el token no puede leer, **404 y nunca 403**;
//! - uv pide JSON (PEP 691) pero acepta el HTML de PEP 503, y no hace `Range`:
//!   basta el HTML con `#sha256=` y `data-requires-python`.

use crate::artefactos::{self, Carga, error, pide_credencial, texto_plano};
use crate::rutas::Servidor;
use ore_core::json::Json;
use ore_entrada::http::{Bytes, Peticion, Respuesta, Salida, Subida};
use ore_entrada::multipart::{Multipart, Parte};

/// El nombre de PyPI normalizado (PEP 503): minúsculas, y cada tramo de `.`,
/// `_` y `-` vale un `-`. Es la coordenada (D-A14).
pub fn normal(n: &str) -> String {
    let mut out = String::with_capacity(n.len());
    let mut separando = false;
    for c in n.chars() {
        if matches!(c, '.' | '_' | '-') {
            separando = true;
        } else {
            if separando && !out.is_empty() {
                out.push('-');
            }
            separando = false;
            out.push(c.to_ascii_lowercase());
        }
    }
    out
}

/// Un nombre de proyecto válido (PEP 508): letras, dígitos, `.`, `_` y `-`,
/// que empieza y acaba en letra o dígito.
fn nombre_valido(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 200
        && n.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        && n.starts_with(|c: char| c.is_ascii_alphanumeric())
        && n.ends_with(|c: char| c.is_ascii_alphanumeric())
}

/// **La versión para comparar** (PEP 440, lo bastante): `1.0` = `1.0.0` =
/// `v1.0`, `1.0-RC1` = `1.0rc1`, `1.0.alpha2` = `1.0a2`. Sirve para decir
/// «es la misma versión» (D-A14); la del fichero se guarda tal cual.
pub fn version_canonica(v: &str) -> String {
    let v = v.trim().to_ascii_lowercase();
    let v = v.strip_prefix('v').unwrap_or(&v);
    let (publica, local) = match v.split_once('+') {
        Some((p, l)) => (p, Some(l.replace(['-', '_'], "."))),
        None => (v, None),
    };
    let fin = publica
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(publica.len());
    let (release, resto) = publica.split_at(fin);
    let mut numeros: Vec<String> = release
        .split('.')
        .filter(|x| !x.is_empty())
        .map(|x| x.trim_start_matches('0').to_string())
        .map(|x| if x.is_empty() { "0".into() } else { x })
        .collect();
    while numeros.len() > 1 && numeros.last().map(String::as_str) == Some("0") {
        numeros.pop();
    }
    // El resto, por tramos de letras y de dígitos, sin separadores.
    let mut sufijo = String::new();
    let mut tramo = String::new();
    let volcar = |t: &mut String, s: &mut String| {
        if t.is_empty() {
            return;
        }
        if t.chars().all(|c| c.is_ascii_digit()) {
            let n = t.trim_start_matches('0');
            s.push_str(if n.is_empty() { "0" } else { n });
        } else {
            s.push_str(match t.as_str() {
                "alpha" => "a",
                "beta" => "b",
                "c" | "pre" | "preview" => "rc",
                "rev" | "r" => "post",
                otro => otro,
            });
        }
        t.clear();
    };
    for c in resto.chars() {
        if matches!(c, '.' | '-' | '_') {
            volcar(&mut tramo, &mut sufijo);
            continue;
        }
        let cambia = tramo
            .chars()
            .last()
            .is_some_and(|u| u.is_ascii_digit() != c.is_ascii_digit());
        if cambia {
            volcar(&mut tramo, &mut sufijo);
        }
        tramo.push(c);
    }
    volcar(&mut tramo, &mut sufijo);
    let mut out = numeros.join(".") + &sufijo;
    if let Some(l) = local {
        out.push('+');
        out.push_str(&l);
    }
    out
}

/// Un nombre de fichero de PyPI: su distribución, su versión y su tipo.
/// Rueda: `<dist>-<versión>(-<build>)?-<py>-<abi>-<plat>.whl`; sdist:
/// `<dist>-<versión>.tar.gz` (o `.zip`).
pub fn de_fichero(f: &str) -> Result<(String, String, &'static str), String> {
    if f.is_empty()
        || f.len() > 255
        || f.starts_with('.')
        || !f
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+'))
    {
        return Err(format!("`{f}` is not a file name PyPI accepts"));
    }
    if let Some(stem) = f.strip_suffix(".whl") {
        let partes: Vec<&str> = stem.split('-').collect();
        if !(partes.len() == 5 || partes.len() == 6) || partes.iter().any(|p| p.is_empty()) {
            return Err(format!(
                "`{f}` is not a wheel name: `<name>-<version>-<python>-<abi>-<platform>.whl`"
            ));
        }
        return Ok((partes[0].into(), partes[1].into(), "bdist_wheel"));
    }
    let stem = f
        .strip_suffix(".tar.gz")
        .or_else(|| f.strip_suffix(".zip"))
        .ok_or_else(|| {
            format!("`{f}` is neither a wheel (`.whl`) nor a source distribution (`.tar.gz`)")
        })?;
    match stem.rsplit_once('-') {
        Some((d, v)) if !d.is_empty() && !v.is_empty() => Ok((d.into(), v.into(), "sdist")),
        _ => Err(format!(
            "`{f}` is not a source distribution name: `<name>-<version>.tar.gz`"
        )),
    }
}

/// Lo que pide una ruta de PyPI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RutaPypi {
    Publicar {
        registro: (String, String, String),
    },
    Indice {
        registro: Option<(String, String, String)>,
        nombre: String,
        /// Si la ruta ya es la canónica (`…/simple/<normalizado>/`).
        canonica: bool,
    },
    Fichero {
        registro: (String, String, String),
        nombre: String,
        version: String,
        fichero: String,
    },
}

fn identificador(x: &str) -> bool {
    !x.is_empty()
        && x.len() <= 100
        && x.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

fn segmentos(ruta: &str) -> Option<Vec<&str>> {
    let resto = ruta.split('?').next()?.strip_prefix("/artifacts/")?;
    Some(resto.split('/').collect())
}

/// Si una ruta de `/artifacts/` es de PyPI.
pub fn es_de_pypi(ruta: &str) -> bool {
    segmentos(ruta).is_some_and(|s| s.first() == Some(&"pypi") || s.get(3) == Some(&"pypi"))
}

/// Si es la de publicar en un registro (`POST`).
pub fn es_publicar(ruta: &str) -> bool {
    matches!(ruta_pypi(ruta), Ok(RutaPypi::Publicar { .. }))
}

pub fn ruta_pypi(ruta: &str) -> Result<RutaPypi, String> {
    let seg = segmentos(ruta).ok_or("not an artifact route")?;
    let (registro, resto): (Option<(String, String, String)>, &[&str]) = match seg.as_slice() {
        ["pypi", r @ ..] => (None, r),
        [b, s, r, "pypi", resto @ ..] => {
            for x in [b, s, r] {
                if !identificador(x) {
                    return Err(format!("`{x}` is not a name of a registry path"));
                }
            }
            (Some((b.to_string(), s.to_string(), r.to_string())), resto)
        }
        _ => return Err("not a PyPI route".into()),
    };
    match resto {
        [] | [""] if registro.is_some() => Ok(RutaPypi::Publicar {
            registro: registro.unwrap_or_default(),
        }),
        ["simple", n] | ["simple", n, ""] if nombre_valido(n) => Ok(RutaPypi::Indice {
            canonica: resto.len() == 3 && normal(n) == *n,
            nombre: normal(n),
            registro,
        }),
        ["files", n, v, f] if registro.is_some() && nombre_valido(n) => {
            let (dist, _, _) = de_fichero(f)?;
            if normal(&dist) != normal(n) || v.is_empty() || v.starts_with('.') {
                return Err(format!("`{f}` does not belong to `{n}`"));
            }
            Ok(RutaPypi::Fichero {
                registro: registro.unwrap_or_default(),
                nombre: normal(n),
                version: v.to_string(),
                fichero: f.to_string(),
            })
        }
        _ => Err(
            "a PyPI path is `…/pypi/simple/<name>/` (to read) or `/artifacts/<base>/<schema>/<registry>/pypi/` (to publish)"
                .into(),
        ),
    }
}

fn escapar(t: &str) -> String {
    t.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// **El índice simple de un nombre** (PEP 503): un enlace por fichero, con su
/// `#sha256=` y su `data-requires-python`.
pub(crate) fn indice_html(
    registro: &str,
    nombre: &str,
    publicados: &[artefactos::Publicado],
) -> String {
    let (b, s, r) = artefactos::partes_del_registro(registro);
    let mut enlaces = String::new();
    for f in publicados {
        let rp = if f.requires_python.is_empty() {
            String::new()
        } else {
            format!(" data-requires-python=\"{}\"", escapar(&f.requires_python))
        };
        enlaces.push_str(&format!(
            "    <a href=\"/artifacts/{b}/{s}/{r}/pypi/files/{nombre}/{}/{}#sha256={}\"{rp}>{}</a><br/>\n",
            escapar(&f.version),
            escapar(&f.nombre),
            f.sha256,
            escapar(&f.nombre)
        ));
    }
    format!(
        "<!DOCTYPE html>\n<html>\n  <head><meta name=\"pypi:repository-version\" content=\"1.0\"><title>Links for {nombre}</title></head>\n  <body>\n    <h1>Links for {nombre}</h1>\n{enlaces}  </body>\n</html>\n"
    )
}

/// Lo que el token de publicar devuelve para PyPI (lo pinta Publish, AR3).
pub fn instrucciones(raiz: &str, b: &str, s: &str, r: &str) -> Json {
    let subir = format!("{raiz}/{b}/{s}/{r}/pypi/");
    let indice = format!("{raiz}/pypi/simple/");
    Json::obj([
        ("upload_url", Json::s(&subir)),
        ("index_url", Json::s(&indice)),
        (
            "registry_index_url",
            Json::s(format!("{raiz}/{b}/{s}/{r}/pypi/simple/")),
        ),
        ("username", Json::s("__token__")),
        (
            "commands",
            Json::Arr(vec![
                Json::s("rm -rf dist && uv build"),
                Json::s(format!(
                    "uv publish --publish-url {subir} --username __token__ --password \"$ORE_TOKEN\""
                )),
                Json::s(format!(
                    "twine upload --repository-url {subir} -u __token__ -p \"$ORE_TOKEN\" dist/*"
                )),
            ]),
        ),
        (
            "note",
            Json::s(
                "ORE installs wheels only: build with `uv build` (or `uv build --wheel`); a source distribution is kept but never installed",
            ),
        ),
    ])
}

fn redirigir(a: &str) -> Salida {
    Salida::Bytes(Bytes {
        codigo: 301,
        cabeceras: vec![("location".into(), a.into())],
        largo: Some(0),
        lector: Box::new(std::io::empty()),
        finales: None,
    })
}

/// El token de una petición que lee: `Err` con la respuesta si no vale.
fn para_leer(p: &Peticion) -> Result<Carga, Salida> {
    match artefactos::de_basic(p).map(|t| artefactos::verificar(&t)) {
        None => Err(pide_credencial(
            "an ORE artifact token is needed (HTTP Basic, as the password)",
        )),
        Some(Err(m)) => Err(pide_credencial(m)),
        Some(Ok(c)) => Ok(c),
    }
}

/// ⛔ Lo que el token no puede leer es **404**, no 403 (AR2·0): un 403 hace que
/// uv falle la capa entera, también para lo público.
fn no_esta(nombre: &str) -> Salida {
    error(404, format!("no registry of this cell has `{nombre}`"))
}

impl Servidor {
    /// **Lo que una petición de PyPI a `/artifacts/` contesta** (las que leen).
    pub(crate) fn atender_pypi(&self, p: &Peticion) -> Salida {
        let ruta = match ruta_pypi(&p.ruta) {
            Ok(r) => r,
            Err(m) => return error(404, m),
        };
        let cabeza = match p.metodo.as_str() {
            "GET" => false,
            "HEAD" => true,
            "POST" => return error(411, "an upload says its `content-length`"),
            otro => {
                return error(
                    405,
                    format!("`{otro}` is not something a PyPI index answers"),
                );
            }
        };
        if let RutaPypi::Publicar { .. } = ruta {
            return error(405, "this is where `uv publish` and `twine upload` POST");
        }
        let carga = match para_leer(p) {
            Ok(c) => c,
            Err(s) => return s,
        };
        let direccion = match artefactos::medios() {
            Ok(d) => d,
            Err(s) => return s,
        };
        match ruta {
            RutaPypi::Publicar { .. } => unreachable!(),
            RutaPypi::Indice {
                registro,
                nombre,
                canonica,
            } => {
                if !canonica {
                    let base = match &registro {
                        Some((b, s, r)) => format!("/artifacts/{b}/{s}/{r}/pypi/simple"),
                        None => "/artifacts/pypi/simple".into(),
                    };
                    return redirigir(&format!("{base}/{nombre}/"));
                }
                let registro = match registro {
                    Some((b, s, r)) => format!("{b}.{s}.{r}"),
                    None => match artefactos::duena(&direccion, "pypi", &nombre) {
                        Ok(Some(r)) => r,
                        Ok(None) => return no_esta(&nombre),
                        Err(s) => return s,
                    },
                };
                if carga.registro != "*" && carga.registro != registro {
                    return no_esta(&nombre);
                }
                let publicados = match artefactos::listar(&direccion, &registro, "pypi", &nombre) {
                    Ok(l) => l,
                    Err(s) => return s,
                };
                if publicados.is_empty() {
                    return no_esta(&nombre);
                }
                let html = indice_html(&registro, &nombre, &publicados);
                texto_plano(200, "text/html", html.into_bytes(), cabeza)
            }
            RutaPypi::Fichero {
                registro: (b, s, r),
                nombre,
                version,
                fichero,
            } => {
                let registro = format!("{b}.{s}.{r}");
                if carga.registro != "*" && carga.registro != registro {
                    return no_esta(&nombre);
                }
                let publicados = match artefactos::listar(&direccion, &registro, "pypi", &nombre) {
                    Ok(l) => l,
                    Err(s) => return s,
                };
                let Some(f) = publicados
                    .iter()
                    .find(|f| f.version == version && f.nombre == fichero)
                else {
                    return error(404, format!("`{fichero}` is not published in `{registro}`"));
                };
                if cabeza {
                    return Salida::Bytes(Bytes {
                        codigo: 200,
                        cabeceras: vec![("content-type".into(), "application/octet-stream".into())],
                        largo: Some(f.tamano),
                        lector: Box::new(std::io::empty()),
                        finales: None,
                    });
                }
                let camino = format!("/artefactos/{b}/{s}/{r}/pypi/{nombre}/{version}/{fichero}");
                match ore_entrada::http::pedir_bajando(
                    "GET",
                    &direccion,
                    &camino,
                    &[],
                    artefactos::PLAZOS,
                ) {
                    Ok(mut bytes) if bytes.codigo == 200 => {
                        bytes.cabeceras.retain(|(k, _)| k == "content-type");
                        Salida::Bytes(bytes)
                    }
                    Ok(bytes) => error(bytes.codigo, "`ore-medios` did not serve it"),
                    Err(e) => error(503, format!("`ore-medios` is not responding: {e}")),
                }
            }
        }
    }

    /// **`POST /artifacts/<b>/<s>/<registro>/pypi/`**: un fichero de `uv
    /// publish` o `twine upload`, con el cuerpo en flujo.
    pub(crate) fn subir_pypi(&self, p: &Peticion, subida: &mut Subida) -> Salida {
        let Ok(RutaPypi::Publicar {
            registro: (b, s, r),
        }) = ruta_pypi(&p.ruta)
        else {
            return error(
                404,
                "publish to the address of a registry: `/artifacts/<base>/<schema>/<registry>/pypi/`",
            );
        };
        let registro = format!("{b}.{s}.{r}");
        let carga = match artefactos::de_basic(p).map(|t| artefactos::verificar(&t)) {
            None => {
                return pide_credencial(
                    "an ORE artifact token is needed (user `__token__`, the token as the password)",
                );
            }
            Some(Err(m)) => return pide_credencial(m),
            Some(Ok(c)) => c,
        };
        if carga.verbo != "publicar" || carga.registro != registro {
            return error(
                403,
                format!(
                    "this token does not publish to `{registro}` (it is for `{}`, to {})",
                    carga.registro, carga.verbo
                ),
            );
        }
        match self.formatos_del_registro(&b, &s, &r) {
            Ok(Some(f)) if f.iter().any(|x| x == "pypi") => {}
            Ok(Some(_)) => {
                return error(
                    400,
                    format!("`{registro}` does not take `pypi`: see its `formats`"),
                );
            }
            Ok(None) => {
                return error(
                    404,
                    format!("there is no artifact registry `{registro}` in this cell"),
                );
            }
            Err(s) => return s,
        }
        let direccion = match artefactos::medios() {
            Ok(d) => d,
            Err(s) => return s,
        };
        let tipo = p.cabeceras.get("content-type").cloned().unwrap_or_default();
        let largo = subida.largo;
        let mut m = match Multipart::nuevo(&mut *subida, &tipo, largo) {
            Ok(m) => m,
            Err(e) => return error(400, e),
        };
        // ── los campos, hasta el fichero (AR2·0: van antes) ──
        let mut campos = std::collections::BTreeMap::<String, String>::new();
        let fichero = loop {
            match m.siguiente() {
                Ok(Some(Parte::Campo { nombre, valor })) => {
                    campos.entry(nombre).or_insert(valor);
                }
                Ok(Some(Parte::Fichero { fichero, .. })) => break fichero,
                Ok(None) => return error(400, "the form has no file (`content`)"),
                Err(e) => return error(400, e),
            }
        };
        let campo = |k: &str| campos.get(k).map(|v| v.trim()).filter(|v| !v.is_empty());
        if let Some(a) = campo(":action")
            && a != "file_upload"
        {
            return error(
                400,
                format!("`:action` `{a}` is not supported: only `file_upload`"),
            );
        }
        let (dist, version_del_fichero, filetype) = match de_fichero(&fichero) {
            Ok(x) => x,
            Err(e) => return error(400, e),
        };
        let nombre = normal(campo("name").unwrap_or(&dist));
        if normal(&dist) != nombre {
            return error(
                400,
                format!(
                    "`{fichero}` is not a file of `{nombre}` (its name normalizes to `{}`)",
                    normal(&dist)
                ),
            );
        }
        if let Some(v) = campo("version")
            && version_canonica(v) != version_canonica(&version_del_fichero)
        {
            return error(
                400,
                format!("`{fichero}` is version `{version_del_fichero}`, not `{v}`"),
            );
        }
        if let Some(t) = campo("filetype")
            && t != filetype
        {
            return error(400, format!("`{fichero}` is a `{filetype}`, not a `{t}`"));
        }
        if version_del_fichero.contains('!') {
            return error(400, "versions with an epoch (`N!`) are not supported");
        }
        let sha256 = match campo("sha256_digest") {
            Some(d) if d.len() == 64 && d.chars().all(|c| c.is_ascii_hexdigit()) => {
                Some(d.to_ascii_lowercase())
            }
            Some(d) => return error(400, format!("`sha256_digest` `{d}` is not a sha256")),
            None => None,
        };
        // ── la versión (D-A14) y si ya estaba: antes de pasar un byte ──
        let publicados = match artefactos::listar(&direccion, &registro, "pypi", &nombre) {
            Ok(l) => l,
            Err(s) => return s,
        };
        if publicados.iter().any(|f| f.nombre == fichero) {
            return error(
                409,
                format!(
                    "File already exists: `{fichero}` is published in `{registro}` and files never change"
                ),
            );
        }
        let canonica = version_canonica(&version_del_fichero);
        let carpeta = publicados
            .iter()
            .find(|f| version_canonica(&f.version) == canonica)
            .map(|f| f.version.clone())
            .unwrap_or(version_del_fichero);
        // ── el fichero, en flujo a `ore-medios` ──
        let Some(largo_del_fichero) = m.largo_del_fichero() else {
            return error(400, "the form ends before its file");
        };
        let mut lector = match m.fichero() {
            Ok(l) => l,
            Err(e) => return error(400, e),
        };
        let camino = format!("/artefactos/{b}/{s}/{r}/pypi/{nombre}/{carpeta}/{fichero}");
        let mut cabeceras = vec![("x-ore-quien", carga.persona.as_str())];
        if let Some(h) = &sha256 {
            cabeceras.push(("x-ore-sha256", h.as_str()));
        }
        let requiere = campo("requires_python").unwrap_or("").to_string();
        if !requiere.is_empty() {
            cabeceras.push(("x-ore-requires-python", requiere.as_str()));
        }
        match ore_entrada::http::pedir_subiendo(
            "PUT",
            &direccion,
            &camino,
            &cabeceras,
            &mut lector,
            largo_del_fichero,
            artefactos::PLAZOS,
        ) {
            Ok((codigo, cuerpo)) => {
                if codigo == 201 {
                    self.huella_de_publicar(
                        &carga,
                        &format!("artifacts/{registro}/pypi:{nombre}/{carpeta}/{fichero}"),
                    );
                }
                // PyPI contesta 200 a una subida buena.
                Salida::Una(Respuesta {
                    codigo: if codigo == 201 { 200 } else { codigo },
                    cuerpo: Json::Crudo(if cuerpo.trim().is_empty() {
                        "{}".into()
                    } else {
                        cuerpo
                    }),
                })
            }
            Err(e) => error(400, format!("the file did not arrive whole: {e}")),
        }
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn los_nombres_se_normalizan_como_en_pep_503() {
        for n in [
            "Banco_Riesgo",
            "banco.riesgo",
            "BANCO--riesgo",
            "banco-_.riesgo",
        ] {
            assert_eq!(normal(n), "banco-riesgo", "{n}");
        }
        assert!(nombre_valido("Banco_Riesgo") && !nombre_valido("-x") && !nombre_valido("a/b"));
    }

    #[test]
    fn la_misma_version_se_escribe_de_muchas_formas() {
        for (a, b) in [
            ("1.0", "1.0.0"),
            ("v1.0", "1"),
            ("1.0-RC1", "1.0rc1"),
            ("1.0.alpha2", "1.0a2"),
            ("1.0-post1", "1.0.post1"),
            ("01.002", "1.2"),
            ("1.0+Ubuntu-1", "1.0+ubuntu.1"),
        ] {
            assert_eq!(version_canonica(a), version_canonica(b), "{a} = {b}");
        }
        for (a, b) in [
            ("1.0", "1.0.1"),
            ("1.0a1", "1.0b1"),
            ("1.0", "1.0.dev1"),
            ("10", "1.0"),
        ] {
            assert_ne!(version_canonica(a), version_canonica(b), "{a} ≠ {b}");
        }
    }

    #[test]
    fn un_fichero_dice_su_nombre_su_version_y_su_tipo() {
        assert_eq!(
            de_fichero("banco_riesgo-1.0-py3-none-any.whl").unwrap(),
            ("banco_riesgo".into(), "1.0".into(), "bdist_wheel")
        );
        assert_eq!(
            de_fichero("banco_riesgo-1.0-1-cp314-cp314-manylinux_2_28_x86_64.whl")
                .unwrap()
                .1,
            "1.0"
        );
        assert_eq!(
            de_fichero("banco_riesgo-1.0.tar.gz").unwrap(),
            ("banco_riesgo".into(), "1.0".into(), "sdist")
        );
        for f in [
            "x.egg",
            "banco-1.0.whl",
            "../x-1.0.tar.gz",
            "a b-1.0.tar.gz",
            "-1.0.tar.gz",
        ] {
            assert!(de_fichero(f).is_err(), "{f}");
        }
    }

    #[test]
    fn las_rutas_de_pypi() {
        let reg = Some(("banco".into(), "riesgo".into(), "libs".into()));
        assert_eq!(
            ruta_pypi("/artifacts/banco/riesgo/libs/pypi/").unwrap(),
            RutaPypi::Publicar {
                registro: reg.clone().unwrap()
            }
        );
        assert!(es_publicar("/artifacts/banco/riesgo/libs/pypi"));
        assert!(!es_publicar("/artifacts/pypi/"));
        assert_eq!(
            ruta_pypi("/artifacts/pypi/simple/banco-riesgo/").unwrap(),
            RutaPypi::Indice {
                registro: None,
                nombre: "banco-riesgo".into(),
                canonica: true
            }
        );
        assert_eq!(
            ruta_pypi("/artifacts/banco/riesgo/libs/pypi/simple/Banco_Riesgo").unwrap(),
            RutaPypi::Indice {
                registro: reg.clone(),
                nombre: "banco-riesgo".into(),
                canonica: false
            }
        );
        assert!(matches!(
            ruta_pypi("/artifacts/banco/riesgo/libs/pypi/files/banco-riesgo/1.0/banco_riesgo-1.0-py3-none-any.whl"),
            Ok(RutaPypi::Fichero { ref version, .. }) if version == "1.0"
        ));
        assert!(
            ruta_pypi(
                "/artifacts/banco/riesgo/libs/pypi/files/otro/1.0/banco_riesgo-1.0-py3-none-any.whl"
            )
            .is_err()
        );
        assert!(es_de_pypi("/artifacts/pypi/simple/x/") && !es_de_pypi("/artifacts/maven/x"));
        assert!(ruta_pypi("/artifacts/Banco/riesgo/libs/pypi/").is_err());
    }

    #[test]
    fn el_indice_simple_enlaza_cada_fichero_con_su_suma() {
        let f = artefactos::Publicado {
            version: "1.0".into(),
            nombre: "banco_riesgo-1.0-py3-none-any.whl".into(),
            sha1: String::new(),
            md5: String::new(),
            sha256: "ab".repeat(32),
            tamano: 3,
            requires_python: ">=3.10".into(),
        };
        let html = indice_html("banco.riesgo.libs", "banco-riesgo", &[f]);
        assert!(html.contains(&format!(
            "href=\"/artifacts/banco/riesgo/libs/pypi/files/banco-riesgo/1.0/banco_riesgo-1.0-py3-none-any.whl#sha256={}\"",
            "ab".repeat(32)
        )), "{html}");
        assert!(
            html.contains("data-requires-python=\"&gt;=3.10\""),
            "{html}"
        );
    }
}
