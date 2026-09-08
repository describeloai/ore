//! Las rutas, y la puerta que las monta o no.
//!
//! # Qué se sirve, y por qué justo esto
//!
//! El primer flujo de una consola: **dar de alta un origen y ver lo que trajo**.
//! Y ese flujo cae partido exactamente por donde el sustrato ya estaba partido:
//!
//! ```text
//!   dar de alta la fuente     `ore source add`   AQUÍ    no abre un socket
//!   ver qué contiene          `ore source catalog`  un JOB   habla con el origen
//!   inducir la ontología      `ore discover`     AQUÍ    lee un fichero
//!   cerrar las decisiones     `ore review`       AQUÍ    las respuestas vienen de fuera
//! ```
//!
//! ⇒ **El botón «añadir origen» lo sirve el plano de control.** El de «ver qué
//! tiene» no, y no por una regla que hayamos escrito: porque el binario que
//! corre aquí **no lleva cliente TLS**.
//!
//! # Las dos cosas que este módulo se niega a hacer
//!
//! - **No acepta una URL con credencial dentro.** Podría: `ore source add` sabe
//!   separarla y mandarla a `.env.local`. Pero un fichero en el disco de un pod
//!   no es un secreto guardado, y aceptarlo hoy sería crear el agujero y
//!   apuntarlo para luego. Se niega con el motivo escrito, y el día que haya un
//!   sitio de verdad donde ponerla, esta negativa es lo que hay que quitar.
//! - **No deja que un nombre de la URL toque el sistema de ficheros.** Un
//!   segmento se valida contra un alfabeto cerrado antes de convertirse en un
//!   camino; `..` no es un caso especial que haya que recordar, es algo que el
//!   alfabeto ya no admite.

use crate::http::{Peticion, Respuesta};
use crate::identidad::{Identidad, Proveedor, SinIdentidad};
use crate::mando;
use ore_core::json::Json;
use ore_core::parse::{self, Node, Style};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub struct Servidor {
    /// El binario `ore`. Es una ruta y no un nombre para que un despliegue no
    /// dependa del `PATH` del contenedor.
    pub binario: PathBuf,
    /// La raíz del repositorio ontológico.
    pub raiz: PathBuf,
    /// El puerto de identidad. `None` ⇒ **las rutas de datos no se montan**.
    pub identidad: Option<Proveedor>,
}

impl Servidor {
    pub fn atender(&self, p: &Peticion) -> Respuesta {
        let seg = p.segmentos();
        match (p.metodo.as_str(), seg.as_slice()) {
            // ── Sin identidad: sólo lo que no dice nada del árbol ────────────
            ("GET", ["salud"]) => Respuesta::ok(Json::obj([("ok", Json::Bool(true))])),
            ("GET", ["version"]) => self.version(),

            // ── Con identidad ────────────────────────────────────────────────
            _ => match self.quien(p) {
                Err(r) => r,
                Ok(sujeto) => self.con_sujeto(p, &sujeto, &seg),
            },
        }
    }

    /// Quién pregunta, o la respuesta que hay que darle.
    ///
    /// Sin proveedor configurado esto contesta **404 y no 401**: la ruta no está
    /// porque no se montó, y decir «no autorizado» insinuaría que existe y que
    /// con la credencial correcta contestaría.
    fn quien(&self, p: &Peticion) -> Result<Identidad, Respuesta> {
        let Some(proveedor) = self.identidad.as_ref() else {
            return Err(Respuesta::error(
                404,
                "sin proveedor de identidad configurado, las rutas de datos no se montan",
            ));
        };
        proveedor(&p.cabeceras).map_err(|e| match e {
            SinIdentidad::Ausente => Respuesta::error(401, "esta ruta necesita un sujeto"),
            SinIdentidad::Invalida(m) => Respuesta::error(401, m),
        })
    }

    fn con_sujeto(&self, p: &Peticion, _sujeto: &Identidad, seg: &[&str]) -> Respuesta {
        match (p.metodo.as_str(), seg) {
            ("GET", ["fuentes"]) => self.fuentes(),
            ("POST", ["fuentes"]) => self.alta_de_fuente(p),
            ("GET", ["paquetes"]) => self.paquetes(),
            ("GET", ["paquetes", n, "decisiones"]) => self.decisiones(n),
            ("POST", ["paquetes", n, "decisiones"]) => self.responder(p, n),
            ("GET", _) | ("POST", _) => Respuesta::error(404, "no hay nada en ese camino"),
            _ => Respuesta::error(405, "método no admitido"),
        }
    }

    // ── Lo que no toca el árbol ─────────────────────────────────────────────

    fn version(&self) -> Respuesta {
        match mando::correr(&self.binario, &self.raiz, &[String::from("--version")]) {
            Ok(s) if s.bien() => Respuesta::ok(Json::obj([
                ("motor", Json::s(s.stdout.trim())),
                ("plano", Json::s("control")),
            ])),
            Ok(s) => Respuesta::error(500, format!("`ore --version` falló: {}", s.stderr.trim())),
            Err(e) => Respuesta::error(500, e.to_string()),
        }
    }

    // ── Las fuentes ─────────────────────────────────────────────────────────

    /// Lo que el manifiesto declara. **Nunca un secreto**: el manifiesto no
    /// tiene ninguno —`connectionEnv` dice dónde buscarlo, no qué es— y esto no
    /// mira el entorno para completarlo.
    fn fuentes(&self) -> Respuesta {
        let manifiesto = self.raiz.join("ontology.config.yaml");
        let texto = match std::fs::read_to_string(&manifiesto) {
            Ok(t) => t,
            Err(_) => {
                return Respuesta::error(404, "este directorio no es un repositorio ontológico");
            }
        };
        let arbol = match parse::parse(&texto) {
            Ok(n) => n,
            Err(e) => return Respuesta::error(500, format!("el manifiesto no analiza: {e:?}")),
        };
        let lista = match arbol.get("datasources") {
            Some((_, n)) => n
                .items()
                .iter()
                .map(|f| {
                    let campo =
                        |k: &str| f.get(k).and_then(|(_, v)| v.as_str()).unwrap_or_default();
                    Json::obj([
                        ("name", Json::s(campo("name"))),
                        ("type", Json::s(campo("type"))),
                        ("connectionEnv", Json::s(campo("connectionEnv"))),
                        ("description", Json::s(campo("description"))),
                    ])
                })
                .collect(),
            None => Vec::new(),
        };
        Respuesta::ok(Json::obj([("datasources", Json::Arr(lista))]))
    }

    fn alta_de_fuente(&self, p: &Peticion) -> Respuesta {
        let cuerpo = match analizar(&p.cuerpo) {
            Ok(n) => n,
            Err(r) => return r,
        };
        let campo = |k: &str| {
            cuerpo
                .get(k)
                .and_then(|(_, v)| v.as_str())
                .map(str::to_string)
        };
        let Some(nombre) = campo("name") else {
            return Respuesta::error(422, "falta `name`");
        };
        let Some(url) = campo("url") else {
            return Respuesta::error(422, "falta `url`");
        };
        if let Err(m) = token(&nombre) {
            return Respuesta::error(422, format!("`name`: {m}"));
        }
        if let Err(m) = sin_credencial(&url) {
            return Respuesta::error(422, m);
        }

        let mut args = vec![
            "source".into(),
            "add".into(),
            "--name".into(),
            nombre.clone(),
            url,
        ];
        if let Some(t) = campo("type") {
            args.push("--type".into());
            args.push(t);
        }
        if let Some(d) = campo("description") {
            args.push("--description".into());
            args.push(d);
        }

        match mando::correr(&self.binario, &self.raiz, &args) {
            Err(e) => Respuesta::error(500, e.to_string()),
            Ok(s) if !s.bien() => Respuesta::error(
                409,
                format!(
                    "`ore source add` devolvió {}: {}",
                    s.codigo,
                    primera_linea(&s.stdout, &s.stderr)
                ),
            ),
            Ok(s) => Respuesta::creado(Json::obj([
                ("name", Json::s(nombre)),
                ("informe", Json::s(s.stdout.trim())),
                // Lo que sigue, dicho en la respuesta y no en la documentación:
                // leer el origen NO se hace aquí, y quien pintó el botón tiene
                // que saberlo sin ir a buscarlo.
                (
                    "siguiente",
                    Json::s(
                        "leer el catálogo del origen no corre en el plano de control: \
                         necesita un Job con la imagen de drivers",
                    ),
                ),
            ])),
        }
    }

    // ── Los paquetes y su cola ──────────────────────────────────────────────

    fn paquetes(&self) -> Respuesta {
        let dir = self.raiz.join("packages");
        let Ok(entradas) = std::fs::read_dir(&dir) else {
            return Respuesta::ok(Json::obj([("packages", Json::Arr(Vec::new()))]));
        };
        let mut lista: Vec<(String, Json)> = Vec::new();
        for e in entradas.flatten() {
            let manifiesto = e.path().join("package.yaml");
            let Ok(texto) = std::fs::read_to_string(&manifiesto) else {
                continue;
            };
            let Ok(arbol) = parse::parse(&texto) else {
                continue;
            };
            let campo = |k: &str| {
                arbol
                    .get("metadata")
                    .and_then(|(_, m)| m.get(k))
                    .and_then(|(_, v)| v.as_str())
                    .unwrap_or_default()
                    .to_string()
            };
            let nombre = e.file_name().to_string_lossy().into_owned();
            let cola = e.path().join(COLA).is_file();
            lista.push((
                nombre.clone(),
                Json::obj([
                    ("name", Json::s(nombre)),
                    ("version", Json::s(campo("version"))),
                    ("decisionesPendientes", Json::Bool(cola)),
                ]),
            ));
        }
        lista.sort_by(|a, b| a.0.cmp(&b.0));
        Respuesta::ok(Json::obj([(
            "packages",
            Json::Arr(lista.into_iter().map(|(_, j)| j).collect()),
        )]))
    }

    /// La cola tal como el inductor la dejó: cada decisión con su `id`, su
    /// `because` y sus `options`. **Es un formulario servido en JSON**, y por eso
    /// esta ruta no la reescribe — reordenar u omitir aquí sería una segunda
    /// opinión sobre lo que hay que preguntar.
    fn decisiones(&self, paquete: &str) -> Respuesta {
        let dir = match self.paquete(paquete) {
            Ok(d) => d,
            Err(r) => return r,
        };
        match std::fs::read_to_string(dir.join(COLA)) {
            Err(_) => Respuesta::ok(Json::obj([
                ("pending", Json::Arr(Vec::new())),
                (
                    "nota",
                    Json::s("no hay cola: o no se indujo, o ya se cerró entera"),
                ),
            ])),
            Ok(t) => match parse::parse(&t) {
                Ok(n) => Respuesta::ok(de_node(&n)),
                Err(e) => Respuesta::error(500, format!("la cola no analiza: {e:?}")),
            },
        }
    }

    fn responder(&self, p: &Peticion, paquete: &str) -> Respuesta {
        let dir = match self.paquete(paquete) {
            Ok(d) => d,
            Err(r) => return r,
        };
        let cuerpo = match analizar(&p.cuerpo) {
            Ok(n) => n,
            Err(r) => return r,
        };
        let Some((_, respuestas)) = cuerpo.get("answers") else {
            return Respuesta::error(422, "falta `answers`");
        };
        if respuestas.entries().is_empty() {
            return Respuesta::error(422, "`answers` está vacío: no hay nada que cerrar");
        }

        // Se escribe como JSON y `ore review` lo lee como YAML. No es un truco:
        // JSON es un subconjunto de YAML, y es la misma economía por la que este
        // árbol no lleva analizador de JSON (ADR 0002).
        let fichero = temporal("respuestas", "json");
        let texto = Json::obj([("answers", de_node(respuestas))]).jcs();
        if std::fs::write(&fichero, texto).is_err() {
            return Respuesta::error(500, "no se pudo escribir el fichero de respuestas");
        }

        let args = vec![
            "review".into(),
            dir.to_string_lossy().into_owned(),
            "--answers".into(),
            fichero.to_string_lossy().into_owned(),
        ];
        let salida = mando::correr(&self.binario, &self.raiz, &args);
        let _ = std::fs::remove_file(&fichero);

        match salida {
            Err(e) => Respuesta::error(500, e.to_string()),
            Ok(s) if !s.bien() => Respuesta::error(
                422,
                format!(
                    "`ore review` devolvió {}: {}",
                    s.codigo,
                    primera_linea(&s.stdout, &s.stderr)
                ),
            ),
            Ok(s) => Respuesta::ok(Json::obj([
                ("informe", Json::s(s.stdout.trim())),
                ("quedan", Json::Bool(dir.join(COLA).is_file())),
            ])),
        }
    }

    /// El directorio de un paquete, con el nombre ya comprobado.
    fn paquete(&self, nombre: &str) -> Result<PathBuf, Respuesta> {
        token(nombre).map_err(|m| Respuesta::error(422, format!("nombre de paquete: {m}")))?;
        let dir = self.raiz.join("packages").join(nombre);
        if dir.is_dir() {
            Ok(dir)
        } else {
            Err(Respuesta::error(404, "no hay tal paquete"))
        }
    }
}

const COLA: &str = "discover.pending.json";

// ── Comprobaciones ──────────────────────────────────────────────────────────

/// Un nombre que puede convertirse en un camino sin sorpresas.
///
/// El alfabeto es cerrado, así que `..`, `/`, `\` y los nombres reservados de
/// Windows no son casos que haya que acordarse de excluir: no están.
pub fn token(v: &str) -> Result<(), String> {
    if v.is_empty() || v.len() > 64 {
        return Err("tiene que medir entre 1 y 64 caracteres".into());
    }
    if !v
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err("sólo letras ASCII, dígitos, `_` y `-`".into());
    }
    Ok(())
}

/// Una URL que **no** trae la credencial dentro.
///
/// Lo que se busca es la autoridad con `usuario:clave@`, y de paso las dos
/// formas de meterla en la consulta. No pretende ser exhaustivo: pretende que
/// el caso normal no pase inadvertido, y el caso raro que pase no crea un
/// secreto donde no lo había — `ore source add` lo sacaría del manifiesto
/// igual.
pub fn sin_credencial(url: &str) -> Result<(), String> {
    let motivo = "esta URL trae una credencial dentro.\n\
        El plano de control no tiene hoy dónde guardarla: `ore source add` la \
        mandaría a `.env.local`, y un fichero en el disco de un pod no es un \
        secreto guardado.\n\
        Las fuentes cuya credencial la presta la nube —BigQuery por Workload \
        Identity, por ejemplo— se dan de alta sin ella.";
    if let Some(resto) = url.split_once("://").map(|(_, r)| r) {
        let autoridad = resto.split(['/', '?', '#']).next().unwrap_or("");
        if autoridad.contains('@') {
            return Err(motivo.into());
        }
    }
    let bajo = url.to_ascii_lowercase();
    for pista in ["password=", "passwd=", "pwd=", "secret=", "token="] {
        if bajo.contains(pista) {
            return Err(motivo.into());
        }
    }
    Ok(())
}

// ── Utilidades ──────────────────────────────────────────────────────────────

fn analizar(cuerpo: &str) -> Result<Node, Respuesta> {
    if cuerpo.trim().is_empty() {
        return Err(Respuesta::error(400, "el cuerpo está vacío"));
    }
    parse::parse(cuerpo).map_err(|e| Respuesta::error(400, format!("el cuerpo no analiza: {e:?}")))
}

/// De lo analizado a la forma canónica.
///
/// El estilo del escalar decide el tipo, igual que en `ore dev`: un `1` sin
/// comillas vuelve como número y un `"1"` como cadena. Es lo que hace que la
/// cola que sale por aquí sea **el mismo JSON** que el inductor escribió.
fn de_node(n: &Node) -> Json {
    match n {
        Node::Mapping { entries, .. } => Json::Obj(
            entries
                .iter()
                .filter_map(|(k, v)| k.as_str().map(|k| (k.to_string(), de_node(v))))
                .collect(),
        ),
        Node::Sequence { items, .. } => Json::Arr(items.iter().map(de_node).collect()),
        Node::Scalar {
            raw,
            style: Style::Plain,
            ..
        } => match raw.as_str() {
            "true" => Json::Bool(true),
            "false" => Json::Bool(false),
            _ => raw.parse::<i64>().map(Json::Int).unwrap_or(Json::s(raw)),
        },
        Node::Scalar { raw, .. } => Json::s(raw),
    }
}

fn primera_linea(stdout: &str, stderr: &str) -> String {
    let fuente = if stderr.trim().is_empty() {
        stdout
    } else {
        stderr
    };
    fuente
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim()
        .to_string()
}

static CUENTA: AtomicU64 = AtomicU64::new(0);

fn temporal(nombre: &str, extension: &str) -> PathBuf {
    let n = CUENTA.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "ore-serve-{}-{}-{}.{}",
        std::process::id(),
        n,
        nombre,
        extension
    ))
}

/// Lo que hay montado, para poder decirlo al arrancar.
pub fn mapa(con_identidad: bool) -> Vec<(&'static str, &'static str, bool)> {
    vec![
        ("GET", "/salud", true),
        ("GET", "/version", true),
        ("GET", "/fuentes", con_identidad),
        ("POST", "/fuentes", con_identidad),
        ("GET", "/paquetes", con_identidad),
        ("GET", "/paquetes/{nombre}/decisiones", con_identidad),
        ("POST", "/paquetes/{nombre}/decisiones", con_identidad),
    ]
}

pub fn ruta_de(p: &Path) -> String {
    p.display().to_string()
}

#[cfg(test)]
mod pruebas {
    use super::*;

    #[test]
    fn un_nombre_no_puede_salir_de_su_directorio() {
        assert!(token("..").is_err());
        assert!(token("../etc").is_err());
        assert!(token("a/b").is_err());
        assert!(token("a\\b").is_err());
        assert!(token("").is_err());
        assert!(token("ventas").is_ok());
        assert!(token("ventas-2_b").is_ok());
    }

    #[test]
    fn una_url_con_credencial_se_niega() {
        assert!(sin_credencial("postgres://ana:clave@host/db").is_err());
        assert!(sin_credencial("https://host/x?password=abc").is_err());
        assert!(sin_credencial("https://host/x?token=abc").is_err());
    }

    /// El caso que SÍ pasa, y es el que importa: la credencial la presta la nube.
    #[test]
    fn una_url_sin_credencial_pasa() {
        assert!(sin_credencial("bigquery://mi-proyecto/ventas").is_ok());
        assert!(sin_credencial("postgres://host:5432/db").is_ok());
        assert!(sin_credencial("jsonl:///datos/x.jsonl").is_ok());
    }

    #[test]
    fn el_tipo_del_escalar_sobrevive_al_viaje() {
        let n = parse::parse(r#"{"a": 1, "b": "1", "c": true, "d": [1, "x"]}"#).unwrap();
        assert_eq!(de_node(&n).jcs(), r#"{"a":1,"b":"1","c":true,"d":[1,"x"]}"#);
    }
}
