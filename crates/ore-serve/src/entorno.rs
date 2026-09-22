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

/// La declaración de un alcance: la unión de los `dependencies` de los
/// `pyproject.toml` que le tocan, ordenada y sin repetidos.
///
/// Sin alcance, la de la celda (la raíz y todos los paquetes). Con alcance
/// —`packages/<p>/<carpeta>`—, la raíz, **su** paquete y **su** repositorio:
/// lo común sigue siendo común, y lo de al lado deja de pesar.
pub(crate) fn declaracion_en(raiz: &Path, alcance: Option<&str>) -> Vec<String> {
    let mut ficheros = vec![raiz.join("pyproject.toml")];
    match alcance.map(str::trim).filter(|s| !s.is_empty()) {
        None => {
            if let Ok(d) = std::fs::read_dir(raiz.join("packages")) {
                for e in d.flatten() {
                    ficheros.push(e.path().join("pyproject.toml"));
                }
            }
        }
        Some(a) => {
            let partes: Vec<&str> = a.trim_matches('/').split('/').collect();
            // `packages/<p>/<carpeta…>`: el paquete, y después cada nivel hasta
            // el repositorio — una carpeta honda hereda de la de encima.
            if partes.len() >= 2 && partes[0] == "packages" {
                let mut acc = raiz.join("packages").join(partes[1]);
                ficheros.push(acc.join("pyproject.toml"));
                for p in &partes[2..] {
                    acc = acc.join(p);
                    ficheros.push(acc.join("pyproject.toml"));
                }
            }
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
pub(crate) fn entorno_de_en(raiz: &Path, alcance: Option<&str>) -> Entorno {
    let declarado = declaracion_en(raiz, alcance);
    let digest = digest_de(&declarado);
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
    pub(crate) fn entorno(&self, rama: Option<&str>, alcance: Option<&str>) -> Respuesta {
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
            let mut r = Respuesta::ok(ficha(&entorno_de_en(raiz, alcance.as_deref())));
            if let (Json::Obj(m), Some(a)) = (&mut r.cuerpo, &alcance) {
                m.insert("alcance".into(), Json::s(a));
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
    ) -> Respuesta {
        let alcance = match alcance_valido(alcance) {
            Ok(a) => a,
            Err(r) => return r,
        };
        let e = match self.leyendo_en(rama, |raiz| {
            Respuesta::ok(ficha(&entorno_de_en(raiz, alcance.as_deref())))
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
                        "`{a}` no declara dependencias, ni su paquete ni la raíz: nada que resolver (`[project].dependencies` de un `pyproject.toml`)"
                    ),
                    None => "el árbol no declara dependencias: nada que resolver (`[project].dependencies` de un `pyproject.toml`)".to_string(),
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
        let (fichero, texto, job) = cola::rendir_capa(&plantilla, digest, rama, alcance, intento)
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
        let deps = declaracion_en(&d, None);
        assert_eq!(deps, vec!["duckdb", "pandas", "polars"]);
        let h = digest_de(&deps);
        assert!(h.starts_with("capa-") && h.len() == 17);
        assert_eq!(digest_de(&[]), "");
        assert_eq!(entorno_de_en(&d, None).estado, "pendiente");
        std::fs::create_dir_all(d.join("entorno")).unwrap();
        std::fs::write(
            d.join(INFORME),
            format!("{{\"digest\":\"{h}\",\"estado\":\"lista\"}}"),
        )
        .unwrap();
        assert_eq!(entorno_de_en(&d, None).estado, "lista");
        std::fs::write(
            d.join(INFORME),
            "{\"digest\":\"capa-otra\",\"estado\":\"lista\"}",
        )
        .unwrap();
        assert_eq!(entorno_de_en(&d, None).estado, "pendiente");
        let _ = std::fs::remove_dir_all(&d);
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
            declaracion_en(&d, None),
            vec!["duckdb", "polars", "pyarrow"],
            "sin alcance, la unión de la raíz y los paquetes"
        );
        // Un repositorio: la raíz, su paquete y él. Lo de al lado, no.
        assert_eq!(
            declaracion_en(&d, Some("packages/hr/modelos")),
            vec!["duckdb", "polars", "torch"]
        );
        let al_lado = declaracion_en(&d, Some("packages/hr/analisis"));
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
        let a = digest_de(&declaracion_en(&d, Some("packages/hr/modelos")));
        let b = digest_de(&al_lado);
        assert_ne!(a, b);
        std::fs::create_dir_all(d.join("entorno")).unwrap();
        std::fs::write(
            d.join(informe_ruta(&a)),
            format!("{{\"digest\":\"{a}\",\"estado\":\"lista\"}}"),
        )
        .unwrap();
        assert_eq!(
            entorno_de_en(&d, Some("packages/hr/modelos")).estado,
            "lista"
        );
        assert_eq!(
            entorno_de_en(&d, Some("packages/hr/analisis")).estado,
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
