//! **El entorno** (ADR 0031 §3, W3.2): lo que el árbol declara que una sesión
//! Python necesita, y la capa que lo resuelve.
//!
//! # La declaración
//!
//! `pyproject.toml` en la raíz del árbol y/o en `packages/<p>/pyproject.toml`,
//! con lo de siempre:
//!
//! ```text
//! [project]
//! dependencies = ["polars>=1.40", "scikit-learn"]
//! ```
//!
//! Se lee **sólo** `[project].dependencies` (una lista de cadenas); lo demás
//! del fichero se ignora. La unión de todos, ordenada y sin repetidos, es la
//! declaración del árbol, y su digest nombra la capa: `capa-<12 hex>`.
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
//! `POST /puestos` la usa: con la capa lista, el puesto nace con ella; con la
//! capa pendiente, la encola y contesta 409 para que la consola espere.

use crate::cola;
use crate::rutas::Servidor;
use ore_core::json::Json;
use ore_entrada::http::Respuesta;
use ore_entrada::identidad::Identidad;
use std::path::Path;

pub(crate) const INFORME: &str = "entorno/python.json";

/// La declaración del árbol: la unión de los `dependencies` de cada
/// `pyproject.toml`, ordenada y sin repetidos.
pub(crate) fn declaracion(raiz: &Path) -> Vec<String> {
    let mut ficheros = vec![raiz.join("pyproject.toml")];
    if let Ok(d) = std::fs::read_dir(raiz.join("packages")) {
        for e in d.flatten() {
            ficheros.push(e.path().join("pyproject.toml"));
        }
    }
    let mut deps: Vec<String> = ficheros
        .iter()
        .filter_map(|f| std::fs::read_to_string(f).ok())
        .flat_map(|t| dependencias_de(&t))
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

/// `capa-<12 hex>` de la declaración; vacío si no hay dependencias.
pub(crate) fn digest_de(deps: &[String]) -> String {
    if deps.is_empty() {
        return String::new();
    }
    let d = ore_core::digest::de_bytes(deps.join("\n").as_bytes());
    format!("capa-{}", &d["sha256:".len().."sha256:".len() + 12])
}

/// El informe del árbol, si lo hay.
pub(crate) fn informe_de(raiz: &Path) -> Option<Json> {
    let t = std::fs::read_to_string(raiz.join(INFORME)).ok()?;
    let n = ore_core::parse::parse(&t).ok()?;
    Some(crate::rutas::de_node(&n))
}

pub(crate) struct Entorno {
    pub declarado: Vec<String>,
    pub digest: String,
    pub informe: Option<Json>,
    /// `sin-dependencias` · `pendiente` · `lista` · `error`.
    pub estado: &'static str,
}

pub(crate) fn entorno_de(raiz: &Path) -> Entorno {
    let declarado = declaracion(raiz);
    let digest = digest_de(&declarado);
    let informe = informe_de(raiz);
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

impl Servidor {
    /// `GET /entorno`, en la rama de `X-Ore-Rama`.
    pub(crate) fn entorno(&self, rama: Option<&str>) -> Respuesta {
        self.leyendo_en(rama, |raiz| Respuesta::ok(ficha(&entorno_de(raiz))))
    }

    /// `POST /entorno`: encolar la capa. 200 si ya está lista, 202 encolada,
    /// 422 sin dependencias.
    pub(crate) fn resolver_entorno(&self, sujeto: &Identidad, rama: Option<&str>) -> Respuesta {
        let e = match self.leyendo_en(rama, |raiz| Respuesta::ok(ficha(&entorno_de(raiz)))) {
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
                "el árbol no declara dependencias: nada que resolver (`[project].dependencies` de un `pyproject.toml`)",
            ),
            "lista" => Respuesta::ok(e),
            estado => match self.encolar_capa(
                &campo("digest"),
                rama.unwrap_or(""),
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
        sujeto: &Identidad,
        intento: &str,
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
        let plantilla = std::fs::read_to_string(dir.join(cola::PLANTILLA_CAPA)).map_err(|_| {
            Respuesta::error(
                503,
                format!(
                    "la cola no trae `{}`: hay que converger este inquilino",
                    cola::PLANTILLA_CAPA
                ),
            )
        })?;
        let (fichero, texto, job) = cola::rendir_capa(&plantilla, digest, rama, intento)
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
        let deps = declaracion(&d);
        assert_eq!(deps, vec!["duckdb", "pandas", "polars"]);
        let h = digest_de(&deps);
        assert!(h.starts_with("capa-") && h.len() == 17);
        assert_eq!(digest_de(&[]), "");
        assert_eq!(entorno_de(&d).estado, "pendiente");
        std::fs::create_dir_all(d.join("entorno")).unwrap();
        std::fs::write(
            d.join(INFORME),
            format!("{{\"digest\":\"{h}\",\"estado\":\"lista\"}}"),
        )
        .unwrap();
        assert_eq!(entorno_de(&d).estado, "lista");
        std::fs::write(
            d.join(INFORME),
            "{\"digest\":\"capa-otra\",\"estado\":\"lista\"}",
        )
        .unwrap();
        assert_eq!(entorno_de(&d).estado, "pendiente");
        let _ = std::fs::remove_dir_all(&d);
    }

    fn tempfile_dir() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("ore-entorno-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }
}
