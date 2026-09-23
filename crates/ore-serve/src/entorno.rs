//! **El entorno** (ADR 0031 §3, W3.2): lo que el árbol declara que una sesión
//! Python necesita, y la capa que lo resuelve.
//!
//! # La declaración
//!
//! `pyproject.toml` en la raíz del árbol, en `packages/<p>/pyproject.toml` y
//! —desde 0036 ③— en el del **repositorio**, con lo de siempre:
//!
//! ```text
//! [project]
//! dependencies = ["polars>=1.40", "scikit-learn"]
//! ```
//!
//! Se lee **sólo** `[project].dependencies` (una lista de cadenas); lo demás
//! del fichero se ignora. La unión, ordenada y sin repetidos, es la
//! declaración, y su digest nombra la capa: `capa-<12 hex>`.
//!
//! # Y en la JVM (0037 ③c): el mismo camino, otro fichero
//!
//! Hasta ③c un repositorio de Java **no podía usar ni una biblioteca**: no era
//! una limitación del lenguaje, era que esto entendía sólo `pyproject.toml`.
//! Ahora el entorno tiene **lenguaje**, y cada uno declara donde su mundo
//! declara —`pom.xml`, `<dependencies>`, `groupId:artifactId:version`—:
//!
//! | entorno | fichero | qué se lee |
//! |---|---|---|
//! | `python` | `pyproject.toml` | `[project].dependencies` |
//! | `jvm` | `pom.xml` | `<dependencies>` de `<project>` |
//!
//! Lo demás **no cambia**: el mismo alcance (la raíz, el paquete y el
//! repositorio), la misma unión ordenada y sin repetidos, el mismo
//! `capa-<12 hex>` y el mismo informe por digest. **El entorno entra en el
//! digest** —salvo en Python, para que las capas ya resueltas sigan
//! llamándose igual—, de modo que dos lenguajes nunca comparten capa ni
//! informe aunque declararan lo mismo.
//!
//! ⛔ Y lo que no se honra, no se finge: de un `pom.xml` se lee
//!   `<dependencies>` **y nada más** —ni `<dependencyManagement>`, ni
//!   `<build>`, ni `<plugins>`, ni `<profiles>`—, exactamente como de un
//!   `pyproject.toml` no se honra nada fuera de `[project].dependencies`. Y lo
//!   dice el propio fichero sembrado.
//!
//! # El alcance (0036 ③): la capa es del repositorio, no de la celda
//!
//! Hasta 0036, la declaración era **la unión del árbol entero** —la raíz y
//! todos los paquetes—, de modo que **una** capa servía a todas las sesiones
//! del cliente. Medido el efecto: el día que un repositorio de modelos
//! declarara `torch`, **todas** las sesiones —las de análisis, las de
//! funciones— arrancarían bajándolo. El entorno único no es una incomodidad de
//! pantalla: es un acoplamiento que crece con el cliente.
//!
//! Desde 0036 la declaración se resuelve **por alcance**:
//!
//! | alcance | qué suma |
//! |---|---|
//! | ninguno (la celda) | la raíz **y todos** los paquetes — como siempre |
//! | `packages/<p>/<carpeta>` | la raíz, **su** paquete y **su** repositorio |
//!
//! La raíz y el paquete siguen siendo comunes **a propósito**: lo de todos,
//! para todos. Lo que deja de ser común es lo de al lado.
//!
//! El alcance viaja por **cabecera** (`X-Ore-Raiz`), no por la URL: ningún dato
//! entra por la URL (`ore-entrada` la descarta). Y el informe deja de ser uno
//! solo: es **uno por digest** (`entorno/<digest>.json`), porque dos alcances
//! son dos capas y un único fichero haría que la segunda borrara a la primera.
//!
//! # La capa
//!
//! Medido el 2026-09-19 (`medida-w3-la-capa.py`, victor, `polars`): resolver
//! con la red del driver cuesta 1,8 s y sube 51 MB de ruedas al bucket en 1 s;
//! el puesto, sin internet, las baja en 0,9 s y las instala en 3,2 s; `import
//! polars` 160 ms. Por eso la capa **no es una imagen en el registro**: es una
//! **caja de ruedas en el bucket** (`ore/puesto/<capa>/`) que el puesto instala
//! al arrancar en un `emptyDir` (`/capa`, en el `PYTHONPATH`). Hermética
//! —el puesto sigue sin alcanzar PyPI—, reproducible —el lock es la lista de
//! ruedas, resueltas para el intérprete del entorno 1— y sin un builder de
//! imágenes ni permisos de registro por inquilino.
//!
//! Quien resuelve es un Job de la cola (`52-la-capa.yaml`, rol driver: alcanza
//! PyPI y escribe el bucket) que deja en el árbol el informe
//! `entorno/python.json` —como la copia deja `copias/<v>.json`—: qué se
//! declaró, con qué digest, qué ruedas, cuántos MB, y `estado: lista | error`.
//!
//! # Los verbos
//!
//! - `GET /entorno`: la declaración, su digest y el informe; `estado`:
//!   `sin-dependencias` · `pendiente` (no hay informe, o es de otra
//!   declaración) · `lista` · `error`;
//! - `POST /entorno`: encolar la capa (202 con el Job), o 200 si ya está lista.
//!
//! Y desde ③c hay una forma con lenguaje —`GET /entorno/jvm`,
//! `POST /entorno/jvm`—: sin él, `python`, como siempre.
//!
//! `POST /puestos` la usa: con la capa lista, el puesto nace con ella; con la
//! capa pendiente, la encola y contesta 409 para que la consola espere.

use crate::cola;
use crate::rutas::Servidor;
use ore_core::json::Json;
use ore_entrada::http::Respuesta;
use ore_entrada::identidad::Identidad;
use std::path::Path;

/// El informe de antes de 0036, cuando la capa era una sola. Se sigue leyendo
/// —un árbol ya resuelto no tiene por qué volver a resolverse— pero no se
/// escribe: los nuevos van a `entorno/<digest>.json`.
pub(crate) const INFORME: &str = "entorno/python.json";

/// El informe de una capa: uno por digest, que es lo que permite que dos
/// alcances convivan.
pub(crate) fn informe_ruta(digest: &str) -> String {
    format!("entorno/{digest}.json")
}

/// Los entornos que declaran capa. `node` no está: su sesión nace con lo que
/// trae su imagen, y sembrar un `package.json` que nadie resuelve sería
/// sembrar una promesa.
pub(crate) const PYTHON: &str = "python";
pub(crate) const JVM: &str = "jvm";

/// Dónde declara cada entorno. Uno por entorno y ninguno más: un segundo
/// formato para el mismo —`build.gradle`— sería un formato antes de haber
/// probado el primero.
pub(crate) fn fichero_de(entorno: &str) -> &'static str {
    match entorno {
        JVM => "pom.xml",
        _ => "pyproject.toml",
    }
}

/// Cómo se nombra, en el idioma de cada entorno, el sitio donde declarar: para
/// que el «no declaras nada» diga dónde declararlo.
pub(crate) fn donde_de(entorno: &str) -> &'static str {
    match entorno {
        JVM => "`<dependencies>` de un `pom.xml`",
        _ => "`[project].dependencies` de un `pyproject.toml`",
    }
}

/// Lo que declara un fichero de entorno, según de cuál se trate.
pub(crate) fn declaradas_en(entorno: &str, texto: &str) -> Vec<String> {
    match entorno {
        JVM => dependencias_de_pom(texto),
        _ => dependencias_de(texto),
    }
}

/// El entorno de `/entorno/<lenguaje>`: `python` o `jvm`, y nada más.
pub(crate) fn entorno_valido(e: &str) -> Result<&'static str, Respuesta> {
    match e {
        PYTHON => Ok(PYTHON),
        JVM => Ok(JVM),
        otro => Err(Respuesta::error(
            404,
            format!("`{otro}` no declara entorno: `python` (`pyproject.toml`) o `jvm` (`pom.xml`)"),
        )),
    }
}

/// La declaración de un alcance: la unión de los `dependencies` de los
/// `pyproject.toml` que le tocan, ordenada y sin repetidos.
///
/// Sin alcance, la de la celda (la raíz y todos los paquetes). Con alcance
/// —`packages/<p>/<carpeta>`—, la raíz, **su** paquete y **su** repositorio:
/// lo común sigue siendo común, y lo de al lado deja de pesar.
pub(crate) fn declaracion_en(raiz: &Path, alcance: Option<&str>, entorno: &str) -> Vec<String> {
    let fichero = fichero_de(entorno);
    let mut ficheros = vec![raiz.join(fichero)];
    match alcance.map(str::trim).filter(|s| !s.is_empty()) {
        None => {
            if let Ok(d) = std::fs::read_dir(raiz.join("packages")) {
                for e in d.flatten() {
                    ficheros.push(e.path().join(fichero));
                }
            }
        }
        Some(a) => {
            let partes: Vec<&str> = a.trim_matches('/').split('/').collect();
            // `packages/<p>/<carpeta…>`: el paquete, y después cada nivel hasta
            // el repositorio — una carpeta honda hereda de la de encima.
            if partes.len() >= 2 && partes[0] == "packages" {
                let mut acc = raiz.join("packages").join(partes[1]);
                ficheros.push(acc.join(fichero));
                for p in &partes[2..] {
                    acc = acc.join(p);
                    ficheros.push(acc.join(fichero));
                }
            }
        }
    }
    let mut deps: Vec<String> = ficheros
        .iter()
        .filter_map(|f| std::fs::read_to_string(f).ok())
        .flat_map(|t| declaradas_en(entorno, &t))
        .collect();
    deps.sort();
    deps.dedup();
    deps
}

/// `[project].dependencies` de un `pyproject.toml`, y nada más. Un analizador
/// mínimo: la tabla `[project]`, la clave `dependencies`, y las cadenas de su
/// lista (una o varias líneas, con comentarios). Lo que no encaje, se ignora.
pub(crate) fn dependencias_de(texto: &str) -> Vec<String> {
    let mut en_project = false;
    let mut en_lista = false;
    let mut acumulado = String::new();
    for linea in texto.lines() {
        let l = sin_comentario(linea).trim();
        if l.starts_with('[') {
            en_project = l == "[project]";
            en_lista = false;
            continue;
        }
        if !en_project {
            continue;
        }
        if !en_lista {
            let Some(resto) = l.strip_prefix("dependencies") else {
                continue;
            };
            let Some(resto) = resto.trim_start().strip_prefix('=') else {
                continue;
            };
            let resto = resto.trim_start();
            let Some(resto) = resto.strip_prefix('[') else {
                continue;
            };
            en_lista = true;
            acumulado.push_str(resto);
        } else {
            acumulado.push_str(l);
        }
        if en_lista && acumulado.contains(']') {
            break;
        }
        acumulado.push(' ');
    }
    let cuerpo = acumulado.split(']').next().unwrap_or("");
    cadenas_de(cuerpo)
}

fn sin_comentario(l: &str) -> &str {
    // `#` fuera de comillas empieza un comentario.
    let mut dentro = false;
    for (i, c) in l.char_indices() {
        match c {
            '"' | '\'' => dentro = !dentro,
            '#' if !dentro => return &l[..i],
            _ => {}
        }
    }
    l
}

fn cadenas_de(cuerpo: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut actual = String::new();
    let mut comilla: Option<char> = None;
    for c in cuerpo.chars() {
        match comilla {
            None if c == '"' || c == '\'' => comilla = Some(c),
            None => {}
            Some(q) if c == q => {
                let s = actual.trim().to_string();
                if !s.is_empty() {
                    out.push(s);
                }
                actual.clear();
                comilla = None;
            }
            Some(_) => actual.push(c),
        }
    }
    out
}

/// `<dependencies>` de un `pom.xml`, como `groupId:artifactId:version`: la
/// misma lista de cadenas que en Python, en el idioma de Maven.
///
/// Se leen **sólo** las de `project/dependencies`. Quedan fuera, y a
/// propósito:
///
/// - `<dependencyManagement>`, `<build>`, `<plugins>`, `<profiles>` y
///   cualquier otra cosa que el fichero traiga: **no se honran y no se
///   finge** que sí;
/// - las de ámbito `test`, `provided` o `system`: la capa es lo que la sesión
///   necesita para **correr**;
/// - las que no traen `<version>`: sin `<dependencyManagement>` no hay quien
///   la fije, y bajar «la última» haría que la misma capa significara dos
///   cosas distintas en dos días distintos;
/// - las que dejan una propiedad sin resolver (`${arrow.version}`): tampoco
///   copiamos `<properties>`, así que aquí no hay quien la resuelva.
pub(crate) fn dependencias_de_pom(texto: &str) -> Vec<String> {
    let t = sin_comentarios_xml(texto);
    let mut fuera: Vec<String> = Vec::new();
    let mut camino: Vec<String> = Vec::new();
    let mut campos: Vec<(String, String)> = Vec::new();
    let mut suelto = String::new();
    let mut resto = t.as_str();
    while let Some(i) = resto.find('<') {
        suelto.push_str(&resto[..i]);
        let Some(j) = resto[i + 1..].find('>') else {
            break;
        };
        let etiqueta = resto[i + 1..i + 1 + j].trim();
        resto = &resto[i + 1 + j + 1..];
        // `<?xml …?>`, `<!DOCTYPE …>`: ni abren ni cierran nada.
        if etiqueta.starts_with('?') || etiqueta.starts_with('!') {
            suelto.clear();
            continue;
        }
        if let Some(cierre) = etiqueta.strip_prefix('/') {
            let n = nombre_xml(cierre);
            if camino.last().map(String::as_str) == Some(n) {
                if en_una_dependencia(&camino) {
                    // Un campo de la dependencia; más hondo —`<exclusions>`—
                    // no se honra, y cerrar la dependencia la entrega.
                    if camino.len() == 4 {
                        campos.push((n.to_string(), suelto.trim().to_string()));
                    } else if camino.len() == 3 {
                        if let Some(gav) = gav_de(&campos) {
                            fuera.push(gav);
                        }
                        campos.clear();
                    }
                }
                camino.pop();
            }
            suelto.clear();
            continue;
        }
        let n = nombre_xml(etiqueta);
        // `<scope/>` y demás vacíos: se abren y se cierran en el mismo sitio.
        if !etiqueta.ends_with('/') {
            camino.push(n.to_string());
        }
        suelto.clear();
    }
    fuera
}

/// El nombre de una etiqueta: sin atributos, sin `/` final y sin prefijo de
/// espacio de nombres (`mvn:project` es `project`).
fn nombre_xml(etiqueta: &str) -> &str {
    let n = etiqueta
        .split(|c: char| c.is_whitespace())
        .next()
        .unwrap_or("")
        .trim_end_matches('/');
    match n.rsplit_once(':') {
        Some((_, n)) => n,
        None => n,
    }
}

/// ¿Vamos por dentro de `project/dependencies/dependency`? Es lo que deja
/// fuera a `<dependencyManagement>` y a las de los `<profiles>` sin tener que
/// nombrarlas una a una.
fn en_una_dependencia(camino: &[String]) -> bool {
    camino.len() >= 3
        && camino[0] == "project"
        && camino[1] == "dependencies"
        && camino[2] == "dependency"
}

/// `groupId:artifactId:version` de una dependencia, si de verdad lo es.
fn gav_de(campos: &[(String, String)]) -> Option<String> {
    let v = |k: &str| {
        campos
            .iter()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.as_str())
            .unwrap_or("")
    };
    let (g, a, version) = (v("groupId"), v("artifactId"), v("version"));
    if g.is_empty() || a.is_empty() || version.is_empty() {
        return None;
    }
    if !matches!(v("scope"), "" | "compile" | "runtime") {
        return None;
    }
    // ⭐ El alfabeto de una coordenada, estricto A PROPÓSITO: de esto sale un
    //   `pom.xml` generado en el Job que resuelve, y lo que entra lo escribe el
    //   cliente en su árbol. Deja fuera de paso lo que no se puede resolver
    //   —`${arrow.version}`, que no copiamos `<properties>`—, y es el MISMO
    //   alfabeto que comprueba `Capa.java` para que los dos digests coincidan.
    let letra = |c: char| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-';
    if !g.chars().all(letra) || !a.chars().all(letra) {
        return None;
    }
    if !version.chars().all(|c| letra(c) || c == '+') {
        return None;
    }
    Some(format!("{g}:{a}:{version}"))
}

fn sin_comentarios_xml(t: &str) -> String {
    let mut fuera = String::with_capacity(t.len());
    let mut resto = t;
    while let Some(i) = resto.find("<!--") {
        fuera.push_str(&resto[..i]);
        match resto[i..].find("-->") {
            Some(j) => resto = &resto[i + j + "-->".len()..],
            None => return fuera,
        }
    }
    fuera.push_str(resto);
    fuera
}

/// `capa-<12 hex>` de la declaración; vacío si no hay dependencias.
///
/// ⭐ El entorno entra en el digest —salvo en Python, para que las capas ya
///   resueltas sigan llamándose igual—: dos lenguajes no comparten capa ni
///   informe aunque llegaran a declarar la misma lista.
pub(crate) fn digest_de(deps: &[String], entorno: &str) -> String {
    if deps.is_empty() {
        return String::new();
    }
    let sembrado = match entorno {
        PYTHON | "" => deps.join("\n"),
        e => format!("{e}\n{}", deps.join("\n")),
    };
    let d = ore_core::digest::de_bytes(sembrado.as_bytes());
    format!("capa-{}", &d["sha256:".len().."sha256:".len() + 12])
}

/// El informe de una capa: el suyo (`entorno/<digest>.json`) o, si no está, el
/// de antes de 0036 **sólo si habla de este mismo digest**.
pub(crate) fn informe_de(raiz: &Path, digest: &str) -> Option<Json> {
    let leer = |ruta: std::path::PathBuf| -> Option<Json> {
        let t = std::fs::read_to_string(ruta).ok()?;
        let n = ore_core::parse::parse(&t).ok()?;
        Some(crate::rutas::de_node(&n))
    };
    if !digest.is_empty()
        && let Some(j) = leer(raiz.join(informe_ruta(digest)))
    {
        return Some(j);
    }
    let viejo = leer(raiz.join(INFORME))?;
    let suyo = match &viejo {
        Json::Obj(m) => matches!(m.get("digest"), Some(Json::Str(d)) if d == digest),
        _ => false,
    };
    suyo.then_some(viejo)
}

pub(crate) struct Entorno {
    pub declarado: Vec<String>,
    pub digest: String,
    pub informe: Option<Json>,
    /// `sin-dependencias` · `pendiente` · `lista` · `error`.
    pub estado: &'static str,
}

/// El entorno de un alcance (0036 ③): su declaración, su digest y su informe.
pub(crate) fn entorno_de_en(raiz: &Path, alcance: Option<&str>, entorno: &str) -> Entorno {
    let declarado = declaracion_en(raiz, alcance, entorno);
    let digest = digest_de(&declarado, entorno);
    let informe = informe_de(raiz, &digest);
    let campo = |k: &str| match &informe {
        Some(Json::Obj(m)) => match m.get(k) {
            Some(Json::Str(s)) => s.clone(),
            _ => String::new(),
        },
        _ => String::new(),
    };
    let estado = if declarado.is_empty() {
        "sin-dependencias"
    } else if campo("digest") != digest {
        "pendiente"
    } else if campo("estado") == "lista" {
        "lista"
    } else if campo("estado") == "error" {
        "error"
    } else {
        "pendiente"
    };
    Entorno {
        declarado,
        digest,
        informe,
        estado,
    }
}

fn ficha(e: &Entorno) -> Json {
    Json::obj([
        (
            "declarado",
            Json::Arr(e.declarado.iter().map(Json::s).collect()),
        ),
        ("digest", Json::s(&e.digest)),
        ("estado", Json::s(e.estado)),
        (
            "informe",
            e.informe.clone().unwrap_or_else(|| Json::obj([])),
        ),
    ])
}

/// El alcance de `X-Ore-Raiz`, comprobado: `packages/<p>/<carpeta…>`, sin
/// salirse y sin `..`. Vacío o ausente es la celda entera, como siempre.
pub(crate) fn alcance_valido(a: Option<&str>) -> Result<Option<String>, Respuesta> {
    let Some(a) = a.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let limpio = a.trim_matches('/');
    let partes: Vec<&str> = limpio.split('/').collect();
    let bien = partes.len() >= 3
        && partes[0] == "packages"
        && partes.iter().all(|p| {
            !p.is_empty()
                && *p != ".."
                && p.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        });
    if !bien {
        return Err(Respuesta::error(
            422,
            format!(
                "`{a}` no es un alcance: `X-Ore-Raiz` es `packages/<paquete>/<carpeta>` (la carpeta de un repositorio)"
            ),
        ));
    }
    Ok(Some(limpio.to_string()))
}

impl Servidor {
    /// `GET /entorno`, en la rama de `X-Ore-Rama` y en el alcance de
    /// `X-Ore-Raiz` (0036 ③): sin alcance, el de la celda.
    pub(crate) fn entorno(
        &self,
        rama: Option<&str>,
        alcance: Option<&str>,
        entorno: &str,
    ) -> Respuesta {
        let alcance = match alcance_valido(alcance) {
            Ok(a) => a,
            Err(r) => return r,
        };
        self.leyendo_en(rama, |raiz| {
            if let Some(a) = &alcance
                && !raiz.join(a).is_dir()
            {
                return Respuesta::error(404, format!("no hay `{a}` en el árbol"));
            }
            let mut r = Respuesta::ok(ficha(&entorno_de_en(raiz, alcance.as_deref(), entorno)));
            if let Json::Obj(m) = &mut r.cuerpo {
                m.insert("entorno".into(), Json::s(entorno));
                if let Some(a) = &alcance {
                    m.insert("alcance".into(), Json::s(a));
                }
            }
            r
        })
    }

    /// `POST /entorno`: encolar la capa. 200 si ya está lista, 202 encolada,
    /// 422 sin dependencias.
    pub(crate) fn resolver_entorno(
        &self,
        sujeto: &Identidad,
        rama: Option<&str>,
        alcance: Option<&str>,
        entorno: &str,
    ) -> Respuesta {
        let alcance = match alcance_valido(alcance) {
            Ok(a) => a,
            Err(r) => return r,
        };
        let e = match self.leyendo_en(rama, |raiz| {
            Respuesta::ok(ficha(&entorno_de_en(raiz, alcance.as_deref(), entorno)))
        }) {
            r if r.codigo != 200 => return r,
            r => r.cuerpo,
        };
        let campo = |k: &str| match &e {
            Json::Obj(m) => match m.get(k) {
                Some(Json::Str(s)) => s.clone(),
                _ => String::new(),
            },
            _ => String::new(),
        };
        match campo("estado").as_str() {
            "sin-dependencias" => Respuesta::error(
                422,
                match &alcance {
                    Some(a) => format!(
                        "`{a}` no declara dependencias, ni su paquete ni la raíz: nada que resolver ({})",
                        donde_de(entorno)
                    ),
                    None => format!(
                        "el árbol no declara dependencias: nada que resolver ({})",
                        donde_de(entorno)
                    ),
                },
            ),
            "lista" => Respuesta::ok(e),
            estado => match self.encolar_capa(
                &campo("digest"),
                rama.unwrap_or(""),
                alcance.as_deref().unwrap_or(""),
                sujeto,
                &if estado == "error" {
                    format!(
                        "r{}",
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0)
                    )
                } else {
                    "1".to_string()
                },
                entorno,
            ) {
                Ok((job, dicho)) => {
                    let mut e = e;
                    if let Json::Obj(m) = &mut e {
                        m.insert("job".into(), Json::s(job));
                        m.insert("cola".into(), Json::s(dicho));
                    }
                    Respuesta {
                        codigo: 202,
                        cuerpo: e,
                    }
                }
                Err(r) => r,
            },
        }
    }

    /// El Job de la capa a la cola: `(nombre del Job, qué pasó)`.
    pub(crate) fn encolar_capa(
        &self,
        digest: &str,
        rama: &str,
        alcance: &str,
        sujeto: &Identidad,
        intento: &str,
        entorno: &str,
    ) -> Result<(String, String), Respuesta> {
        let Some(forja) = &self.cola else {
            return Err(Respuesta::error(
                503,
                "este servidor no sabe de ninguna cola (`--cola`): no hay quien resuelva la capa",
            ));
        };
        let prestado = forja
            .clonar()
            .map_err(|e| Respuesta::error(502, e.to_string()))?;
        let dir = prestado.ruta();
        let nombre = cola::plantilla_capa_de(entorno);
        let plantilla = std::fs::read_to_string(dir.join(nombre)).map_err(|_| {
            Respuesta::error(
                503,
                format!("la cola no trae `{nombre}`: hay que converger este inquilino"),
            )
        })?;
        let (fichero, texto, job) =
            cola::rendir_capa(&plantilla, entorno, digest, rama, alcance, intento)
                .map_err(|e| Respuesta::error(500, e))?;
        std::fs::write(dir.join(&fichero), &texto)
            .map_err(|e| Respuesta::error(500, format!("no se pudo escribir `{fichero}`: {e}")))?;
        if !forja.hay_cambios(dir) {
            return Ok((job, format!("ya encolada como `{fichero}`")));
        }
        match forja.publicar(dir, sujeto, &format!("Resolver la capa {digest}")) {
            Ok(c) => Ok((job, format!("encolada como `{fichero}` · commit {c}"))),
            Err(e) => Err(Respuesta::error(502, format!("NO encolada: {e}"))),
        }
    }
}

#[cfg(test)]
mod prueba {
    use super::*;

    #[test]
    fn solo_dependencies_de_project_en_una_o_varias_lineas() {
        let t = r#"
[build-system]
requires = ["setuptools"]

[project]
name = "hr"
dependencies = [
  "polars>=1.40",  # rapido
  'scikit-learn',
]
optional-dependencies = { dev = ["pytest"] }

[tool.otro]
dependencies = ["no-esta"]
"#;
        assert_eq!(dependencias_de(t), vec!["polars>=1.40", "scikit-learn"]);
        assert_eq!(
            dependencias_de("[project]\ndependencies = [\"a\", \"b\"]\n"),
            vec!["a", "b"]
        );
        assert!(dependencias_de("[project]\nname = 'x'\n").is_empty());
        assert!(dependencias_de("dependencies = ['fuera-de-project']").is_empty());
    }

    #[test]
    fn la_union_se_ordena_y_el_digest_nombra_la_capa() {
        let d = tempfile_dir();
        std::fs::write(
            d.join("pyproject.toml"),
            "[project]\ndependencies = ['polars', 'pandas']\n",
        )
        .unwrap();
        std::fs::create_dir_all(d.join("packages/hr")).unwrap();
        std::fs::write(
            d.join("packages/hr/pyproject.toml"),
            "[project]\ndependencies = ['polars', 'duckdb']\n",
        )
        .unwrap();
        let deps = declaracion_en(&d, None, PYTHON);
        assert_eq!(deps, vec!["duckdb", "pandas", "polars"]);
        let h = digest_de(&deps, PYTHON);
        assert!(h.starts_with("capa-") && h.len() == 17);
        assert_eq!(digest_de(&[], PYTHON), "");
        assert_eq!(entorno_de_en(&d, None, PYTHON).estado, "pendiente");
        std::fs::create_dir_all(d.join("entorno")).unwrap();
        std::fs::write(
            d.join(INFORME),
            format!("{{\"digest\":\"{h}\",\"estado\":\"lista\"}}"),
        )
        .unwrap();
        assert_eq!(entorno_de_en(&d, None, PYTHON).estado, "lista");
        std::fs::write(
            d.join(INFORME),
            "{\"digest\":\"capa-otra\",\"estado\":\"lista\"}",
        )
        .unwrap();
        assert_eq!(entorno_de_en(&d, None, PYTHON).estado, "pendiente");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// ⭐ EL FICHERO SEMBRADO Y EL QUE LO LEE, DE ACUERDO (0037 ③c · e).
    ///
    /// La semilla de `transforms-java` trae su `pom.xml` como la de Python trae
    /// su `pyproject.toml`. Vacío no declara nada —y por tanto no construye
    /// ninguna capa—, y el ejemplo que lleva comentado **declara de verdad**:
    /// sembrar un ejemplo que este lector no entendiera sería sembrar una
    /// promesa.
    #[test]
    fn el_pom_que_sembramos_es_el_que_sabemos_leer() {
        let pom = ore_core::clases::de("transforms-java")
            .and_then(|c| {
                c.semilla
                    .iter()
                    .find(|(r, _)| *r == "pom.xml")
                    .map(|(_, t)| *t)
            })
            .expect("la semilla de Java trae su `pom.xml`");
        assert!(
            dependencias_de_pom(pom).is_empty(),
            "nace vacío: una capa para nada no se construye"
        );
        let descomentado = pom
            .replace("<!-- <dependency>", "<dependency>")
            .replace("</dependency> -->", "</dependency>");
        assert_eq!(
            dependencias_de_pom(&descomentado),
            vec!["org.apache.commons:commons-lang3:3.17.0"],
            "el ejemplo comentado declara de verdad"
        );
    }

    /// 0037 ③c: de un `pom.xml` se lee `<dependencies>` **y nada más**.
    ///
    /// Un pom de verdad trae mucho que no honramos —gestión de versiones,
    /// plugins, perfiles— y la tentación sería leerlo «por si acaso». Lo que
    /// entra es lo que la sesión necesita para correr, y lo que no se puede
    /// resolver sin honrar el resto (una versión ausente, una propiedad) se
    /// queda fuera antes de convertirse en una capa que significa dos cosas.
    #[test]
    fn un_pom_declara_sus_dependencias_y_nada_mas() {
        let pom = r#"<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://maven.apache.org/POM/4.0.0">
  <modelVersion>4.0.0</modelVersion>
  <!-- <dependency> en un comentario no es una dependencia -->
  <dependencyManagement>
    <dependencies>
      <dependency>
        <groupId>no.entra</groupId><artifactId>gestionada</artifactId><version>1.0</version>
      </dependency>
    </dependencies>
  </dependencyManagement>
  <dependencies>
    <dependency>
      <groupId>org.apache.arrow</groupId>
      <artifactId>arrow-vector</artifactId>
      <version>19.0.0</version>
    </dependency>
    <dependency>
      <groupId>org.apache.commons</groupId><artifactId>commons-lang3</artifactId>
      <version>3.17.0</version><scope>runtime</scope>
      <exclusions><exclusion><groupId>x</groupId><artifactId>y</artifactId></exclusion></exclusions>
    </dependency>
    <dependency>
      <groupId>org.junit.jupiter</groupId><artifactId>junit-jupiter</artifactId>
      <version>5.11.0</version><scope>test</scope>
    </dependency>
    <dependency>
      <groupId>sin.version</groupId><artifactId>quien-sabe</artifactId>
    </dependency>
    <dependency>
      <groupId>sin.resolver</groupId><artifactId>propiedad</artifactId>
      <version>${arrow.version}</version>
    </dependency>
  </dependencies>
  <build><plugins><plugin>
    <groupId>no.entra</groupId><artifactId>un-plugin</artifactId><version>1.0</version>
  </plugin></plugins></build>
  <profiles><profile><id>otro</id><dependencies><dependency>
    <groupId>no.entra</groupId><artifactId>de-un-perfil</artifactId><version>1.0</version>
  </dependency></dependencies></profile></profiles>
</project>
"#;
        assert_eq!(
            dependencias_de_pom(pom),
            vec![
                "org.apache.arrow:arrow-vector:19.0.0",
                "org.apache.commons:commons-lang3:3.17.0"
            ]
        );
        // Un fichero que no es un pom —o que está a medias— no declara nada, y
        // no se lleva por delante al que lo lee.
        for roto in [
            "",
            "esto no es xml",
            "<project><dependencies><dependency><groupId>a",
            "<project><dependencies></dependencies></project>",
        ] {
            assert!(dependencias_de_pom(roto).is_empty(), "{roto}");
        }
    }

    /// El alcance y el digest son los mismos; lo que cambia es el fichero.
    #[test]
    fn la_capa_de_la_jvm_tiene_el_mismo_alcance_y_su_propio_digest() {
        let d = tempfile_dir2("jvm");
        let dep = |g: &str| {
            format!(
                "<project><dependencies><dependency><groupId>{g}</groupId>\
                 <artifactId>a</artifactId><version>1.0</version></dependency>\
                 </dependencies></project>"
            )
        };
        std::fs::write(d.join("pom.xml"), dep("raiz")).unwrap();
        std::fs::create_dir_all(d.join("packages/hr/modelos")).unwrap();
        std::fs::create_dir_all(d.join("packages/hr/analisis")).unwrap();
        std::fs::write(d.join("packages/hr/pom.xml"), dep("paquete")).unwrap();
        std::fs::write(d.join("packages/hr/modelos/pom.xml"), dep("suyo")).unwrap();
        assert_eq!(
            declaracion_en(&d, Some("packages/hr/modelos"), JVM),
            vec!["paquete:a:1.0", "raiz:a:1.0", "suyo:a:1.0"]
        );
        let al_lado = declaracion_en(&d, Some("packages/hr/analisis"), JVM);
        assert_eq!(al_lado, vec!["paquete:a:1.0", "raiz:a:1.0"]);
        // Y el `pyproject.toml` de al lado no se cuela en la capa de la JVM.
        std::fs::write(
            d.join("packages/hr/analisis/pyproject.toml"),
            "[project]\ndependencies = ['polars']\n",
        )
        .unwrap();
        assert_eq!(
            declaracion_en(&d, Some("packages/hr/analisis"), JVM),
            al_lado
        );
        assert_eq!(
            declaracion_en(&d, Some("packages/hr/analisis"), PYTHON),
            vec!["polars"]
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    /// ⭐ Dos entornos no comparten capa aunque declararan lo mismo: el informe
    ///   de uno pasando por el del otro sería un puesto arrancando con las
    ///   ruedas de otro lenguaje.
    #[test]
    fn dos_entornos_con_la_misma_lista_no_comparten_capa() {
        let deps = vec!["a:b:1.0".to_string()];
        let py = digest_de(&deps, PYTHON);
        let jvm = digest_de(&deps, JVM);
        assert_ne!(py, jvm);
        assert!(jvm.starts_with("capa-") && jvm.len() == 17);
        assert_ne!(informe_ruta(&py), informe_ruta(&jvm));
        // Python sigue nombrándose como antes de ③c: las capas ya resueltas no
        // se renombran por esto.
        assert_eq!(py, digest_de(&deps, ""));
        assert_eq!(digest_de(&[], JVM), "");
    }

    #[test]
    fn solo_python_y_la_jvm_declaran_entorno() {
        assert_eq!(entorno_valido("python").ok(), Some(PYTHON));
        assert_eq!(entorno_valido("jvm").ok(), Some(JVM));
        assert_eq!(fichero_de(JVM), "pom.xml");
        assert_eq!(fichero_de(PYTHON), "pyproject.toml");
        let r = entorno_valido("node").unwrap_err();
        assert_eq!(r.codigo, 404);
    }

    /// 0036 ③: la capa es del repositorio, no de la celda.
    ///
    /// Tres repositorios en dos paquetes: lo de la raíz y lo del paquete llegan
    /// a los suyos, y **lo de al lado no llega a nadie**. Es el acoplamiento que
    /// este paso rompe: sin esto, el `torch` de uno lo bajarían todos.
    #[test]
    fn la_capa_de_un_repositorio_no_carga_con_la_del_de_al_lado() {
        let d = tempfile_dir2("alcance");
        std::fs::write(
            d.join("pyproject.toml"),
            "[project]\ndependencies = ['polars']\n",
        )
        .unwrap();
        for (ruta, deps) in [
            ("packages/hr", "['duckdb']"),
            ("packages/hr/modelos", "['torch']"),
            ("packages/hr/analisis", "[]"),
            ("packages/ventas", "['pyarrow']"),
        ] {
            std::fs::create_dir_all(d.join(ruta)).unwrap();
            std::fs::write(
                d.join(ruta).join("pyproject.toml"),
                format!("[project]\ndependencies = {deps}\n"),
            )
            .unwrap();
        }
        // La celda: todo junto, como hasta 0036.
        assert_eq!(
            declaracion_en(&d, None, PYTHON),
            vec!["duckdb", "polars", "pyarrow"],
            "sin alcance, la unión de la raíz y los paquetes"
        );
        // Un repositorio: la raíz, su paquete y él. Lo de al lado, no.
        assert_eq!(
            declaracion_en(&d, Some("packages/hr/modelos"), PYTHON),
            vec!["duckdb", "polars", "torch"]
        );
        let al_lado = declaracion_en(&d, Some("packages/hr/analisis"), PYTHON);
        assert_eq!(al_lado, vec!["duckdb", "polars"]);
        assert!(
            !al_lado.contains(&"torch".to_string()),
            "el `torch` del de al lado NO lastra a este"
        );
        assert!(
            !al_lado.contains(&"pyarrow".to_string()),
            "ni lo de otro paquete"
        );
        // Dos alcances, dos digests: por eso el informe es uno por digest.
        let a = digest_de(
            &declaracion_en(&d, Some("packages/hr/modelos"), PYTHON),
            PYTHON,
        );
        let b = digest_de(&al_lado, PYTHON);
        assert_ne!(a, b);
        std::fs::create_dir_all(d.join("entorno")).unwrap();
        std::fs::write(
            d.join(informe_ruta(&a)),
            format!("{{\"digest\":\"{a}\",\"estado\":\"lista\"}}"),
        )
        .unwrap();
        assert_eq!(
            entorno_de_en(&d, Some("packages/hr/modelos"), PYTHON).estado,
            "lista"
        );
        assert_eq!(
            entorno_de_en(&d, Some("packages/hr/analisis"), PYTHON).estado,
            "pendiente",
            "la capa del vecino no vale por la suya"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn un_alcance_es_una_carpeta_de_un_paquete_y_nada_mas() {
        let bien = |a: Option<&str>| match alcance_valido(a) {
            Ok(v) => v,
            Err(_) => panic!("`{a:?}` tenía que valer"),
        };
        assert_eq!(bien(None), None);
        assert_eq!(bien(Some("  ")), None);
        assert_eq!(
            bien(Some("packages/hr/raw")).as_deref(),
            Some("packages/hr/raw")
        );
        assert!(
            alcance_valido(Some("packages/hr")).is_err(),
            "el paquete no"
        );
        assert!(alcance_valido(Some("../etc")).is_err());
        assert!(alcance_valido(Some("packages/hr/../../etc")).is_err());
        assert!(alcance_valido(Some("otra/cosa/aqui")).is_err());
    }

    fn tempfile_dir2(caso: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("ore-entorno-{}-{caso}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn tempfile_dir() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("ore-entorno-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }
}
