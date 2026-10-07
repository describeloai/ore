//! 0055 B1 · **El build**: `POST /builds {output | fichero, rama?}`.
//!
//! Un build construye la salida de un `Transform` del árbol **con el código del
//! commit** —no el del editor (D13)— en un Job propio: el de un trabajo
//! (`lanzar_trabajo`), con la capa y la clase **del repositorio** del código y
//! el documento como **techo** de lo que lee y escribe. Lo que el
//! `@transform` declare dentro ha de caber en él (`cabe_en_el_techo`): el
//! código no ensancha lo que el documento dice.
//!
//! - **`output`**: el `Transform` cuyo `spec.output` es ésa (forma corta o de
//!   tres partes), en la cabeza de la rama. Uno, y su respuesta es su ficha.
//! - **`fichero`**: cada `Transform` cuyo `entrypoint` está en ese fichero
//!   (D14: Build construye todos los del fichero), un build por cada uno;
//!   la respuesta, `{fichero, builds: [<ficha>…]}`.
//!
//! **La rama** (D16): la que se pide, o la de la persona en el repositorio del
//! código (`<persona>/<repo>`, como su sesión). `main` es una rama más, salvo
//! protegida (`.arbol/ramas.yaml`): entonces 409.
//!
//! **El arnés**: en Python carga el módulo y llama él al `def` —un
//! `@transform` llamado al cargar falla (D15), con su línea—; en SQL corre la
//! sentencia `n` como la corre un trabajo (`celda_de_sentencia`). Todo lo de
//! fuera entra como literal de cadena JSON: nada se interpola como código.
use crate::puestos::{Construccion, Donde, Transform};
use crate::rutas::Servidor;
use ore_core::document::Kind;
use ore_core::json::Json;
use ore_core::parse::Node;
use ore_entrada::http::Respuesta;
use ore_entrada::identidad::Identidad;
use std::path::{Path, PathBuf};

/// Qué se construye.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Que {
    /// La salida, en forma corta.
    Output(String),
    /// Una ruta del árbol (`packages/<p>/<repo>/x.py`).
    Fichero(String),
}

/// Un `Transform` del árbol, listo para lanzar su build.
#[derive(Debug, Clone)]
pub(crate) struct Hallado {
    /// El documento, desde la raíz.
    pub documento: String,
    pub nombre: String,
    pub entrypoint: String,
    pub output: String,
    pub inputs: Vec<String>,
    /// El fichero del código, desde la raíz.
    pub codigo: String,
    /// La celda que corre: el arnés (Python) o la sentencia (SQL).
    pub celda: String,
}

/// Los códigos que dicen que el documento no es el de su código, o que su
/// salida no tiene dónde nacer: con cualquiera de ellos, no se construye.
const NO_SE_CONSTRUYE: &[&str] = &["OOS2042", "OOS2043", "OOS2013", "OOS2018", "OOS2037"];

/// El cuerpo de `POST /builds`.
pub(crate) fn pedido(cuerpo: &str) -> Result<(Que, Option<String>), Respuesta> {
    let n = match ore_core::parse::parse(cuerpo) {
        Ok(n) if !cuerpo.trim().is_empty() => n,
        _ => return Err(Respuesta::error(400, "the body is not JSON")),
    };
    let campo = |k: &str| {
        n.get(k)
            .and_then(|(_, v)| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let que = match (campo("output"), campo("fichero")) {
        (Some(o), None) => {
            if !matches!(o.split('.').count(), 2 | 3) || o.split('.').any(str::is_empty) {
                return Err(Respuesta::error(
                    422,
                    format!(
                        "`{o}` is not an output: `<database>.<schema>.<name>` (or `<database>.<name>`, in `default`)"
                    ),
                ));
            }
            Que::Output(ore_core::normalize::a_corto(&o).into_owned())
        }
        (None, Some(f)) => {
            if f.starts_with('/')
                || f.contains('\\')
                || f.split('/').any(|s| s == ".." || s.is_empty())
                || !(f.ends_with(".py") || f.ends_with(".sql"))
            {
                return Err(Respuesta::error(
                    422,
                    format!("`{f}` is not a `.py` or `.sql` path of the tree"),
                ));
            }
            Que::Fichero(f)
        }
        _ => {
            return Err(Respuesta::error(
                422,
                "a build names what it builds: `output` (a dataset) or `fichero` (every transform of a file), one of the two",
            ));
        }
    };
    let rama = campo("rama");
    if let Some(r) = &rama
        && let Err(m) = crate::propuestas::nombre_de_rama_valido(r)
    {
        return Err(Respuesta::error(422, m));
    }
    Ok((que, rama))
}

/// La carpeta del paquete de un documento: la primera hacia arriba con
/// `package.yaml`, o la raíz.
fn carpeta_del_paquete(doc: &Path, raiz: &Path) -> PathBuf {
    let mut d = doc.parent();
    while let Some(c) = d {
        if c.join("package.yaml").is_file() || c == raiz {
            return c.to_path_buf();
        }
        d = c.parent();
    }
    raiz.to_path_buf()
}

fn rel(raiz: &Path, p: &Path) -> String {
    p.strip_prefix(raiz)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Los `Transform` de `que` en el árbol de `raiz`, cada uno con su celda. 404
/// si no hay ninguno; 422, con los diagnósticos, si alguno no es el de su
/// código (o su salida no tiene base o schema); 422 en `runtime: java`.
pub(crate) fn hallar(raiz: &Path, que: &Que) -> Result<Vec<Hallado>, Respuesta> {
    let (pkg, _) = ore_core::validate::cargar_paquete(raiz);
    struct Candidato<'a> {
        t: &'a ore_core::link::Loaded,
        runtime: String,
        clave: String,
        fichero: PathBuf,
        output: String,
    }
    let mut candidatos = Vec::new();
    for t in pkg.docs.iter().filter(|d| d.kind == Kind::Transform) {
        let runtime = t
            .section("runtime")
            .and_then(Node::as_str)
            .unwrap_or("")
            .to_string();
        let ep = t.section("entrypoint").and_then(Node::as_str).unwrap_or("");
        let leido = match runtime.as_str() {
            "python" => ore_core::promover::entrypoint(ep).map(|(r, d)| (r, d.to_string())),
            "sql" => ore_core::transformar::entrypoint_sql(ep).map(|(r, n)| (r, n.to_string())),
            _ => ep.rsplit_once(':').map(|(r, c)| (r, c.to_string())),
        };
        let Some((ruta, clave)) = leido else { continue };
        let fichero = carpeta_del_paquete(&t.path, &pkg.root).join(ruta);
        let Some(output) = t
            .section("output")
            .and_then(Node::as_str)
            .map(|o| ore_core::link::cualificar(o, t))
        else {
            continue;
        };
        let output = ore_core::normalize::a_corto(&output).into_owned();
        let es = match que {
            Que::Output(o) => *o == output,
            Que::Fichero(f) => rel(raiz, &fichero) == *f,
        };
        if es {
            candidatos.push(Candidato {
                t,
                runtime,
                clave,
                fichero,
                output,
            });
        }
    }
    if candidatos.is_empty() {
        return Err(match que {
            Que::Output(o) => con_motivo(
                Respuesta::error(
                    404,
                    format!(
                        "no `Transform` in this branch writes `{o}`: commit the code that writes it (its document is derived at commit), or say which `rama` it is in"
                    ),
                ),
                "not_found",
            ),
            Que::Fichero(f) => por_que_no_hay(raiz, &pkg, f),
        });
    }

    // Que cada uno sea el documento de su código, y que su salida pueda nacer.
    let mut diags = Vec::new();
    ore_core::transformar::comprobar(&pkg, &mut diags);
    let suyos: Vec<Json> = diags
        .iter()
        .filter(|d| NO_SE_CONSTRUYE.contains(&d.code.as_str()))
        .filter(|d| {
            candidatos
                .iter()
                .any(|c| d.file == c.t.path || d.file == c.fichero)
        })
        .map(|d| diagnostico_json(raiz, d))
        .collect();
    if !suyos.is_empty() {
        return Err(Respuesta {
            codigo: 422,
            cuerpo: Json::obj([
                (
                    "error",
                    Json::s(
                        "the `Transform` is not the one its code gives at this commit: nothing was built",
                    ),
                ),
                ("diagnostics", Json::Arr(suyos)),
                ("motivo", Json::s("diagnostics")),
            ]),
        });
    }

    let mut hallados = Vec::new();
    for c in candidatos {
        let documento = rel(raiz, &c.t.path);
        let codigo = rel(raiz, &c.fichero);
        let entrypoint =
            c.t.section("entrypoint")
                .and_then(Node::as_str)
                .unwrap_or_default()
                .to_string();
        let Ok(fuente) = std::fs::read_to_string(&c.fichero) else {
            return Err(diagnostico_suelto(
                "OOS2042",
                format!("`{codigo}` is not in the tree"),
                &codigo,
            ));
        };
        let inputs: Vec<String> = c
            .t
            .section("inputs")
            .map(Node::items)
            .unwrap_or(&[])
            .iter()
            .filter_map(Node::as_str)
            .map(|i| ore_core::normalize::a_corto(&ore_core::link::cualificar(i, c.t)).into_owned())
            .collect();
        let build = Json::obj([
            ("transform", Json::s(&documento)),
            ("entrypoint", Json::s(&entrypoint)),
            ("output", Json::s(&c.output)),
        ]);
        let celda = match c.runtime.as_str() {
            "python" => arnes_python(&ArnesDeBuild {
                documento: &documento,
                fichero: &codigo,
                fuente: &fuente,
                def: &c.clave,
                build: &build,
            }),
            "sql" => {
                let n: usize = c.clave.parse().unwrap_or(0);
                let trozos = ore_core::sql_del_arbol::guion::guion(&fuente).map_err(|_| {
                    diagnostico_suelto("OOS2043", format!("`{codigo}` does not parse"), &codigo)
                })?;
                let Some(t) = n.checked_sub(1).and_then(|i| trozos.get(i)) else {
                    return Err(diagnostico_suelto(
                        "OOS2042",
                        format!("`{codigo}` has no statement {n}"),
                        &codigo,
                    ));
                };
                // 0057 B4·6: en un build, lo que escribe la sentencia lo escribe
                // el `Transform` —lee el origen en vivo y `write()`—, también
                // si lee de un origen. La copia mantenida (0053 F7·3) es lo de
                // una sesión: aquí el build ya es el trabajo que copia.
                let (celda, _) = match &t.sentencia {
                    ore_core::sql_del_arbol::guion::Sentencia::Unidad(u) => {
                        crate::puestos::celda_de_unidad(
                            &codigo,
                            u,
                            ore_core::sql_del_arbol::anchored_to(&pkg, u).as_deref(),
                        )
                    }
                    _ => crate::puestos::celda_de_sentencia(&codigo, t, &pkg),
                };
                format!("{}{celda}", preludio_sql(&build))
            }
            otro => {
                return Err(con_motivo(
                    Respuesta::error(
                        422,
                        format!(
                            "`{documento}` is `runtime: {otro}`: only python and sql transforms build yet"
                        ),
                    ),
                    "not_buildable",
                ));
            }
        };
        hallados.push(Hallado {
            documento,
            nombre: c
                .t
                .meta("name")
                .and_then(Node::as_str)
                .unwrap_or_default()
                .to_string(),
            entrypoint,
            output: c.output,
            inputs,
            codigo,
            celda,
        });
    }
    Ok(hallados)
}

/// Un diagnóstico, como lo lee la consola.
fn diagnostico_json(raiz: &Path, d: &ore_core::diag::Diagnostic) -> Json {
    let mut m = vec![
        ("code", Json::s(d.code.as_str())),
        ("message", Json::s(&d.message)),
        ("file", Json::s(rel(raiz, &d.file))),
    ];
    if let Some(p) = d.pos {
        m.push(("line", Json::Int(p.line as i64)));
        m.push(("column", Json::Int(p.col as i64)));
    }
    if let Some(h) = &d.help {
        m.push(("help", Json::s(h)));
    }
    Json::obj(m)
}

/// 0055 · **Por qué un fichero no tiene nada que construir**: no está en la
/// rama; no se lee (422 con su `OOS2043`, el fichero y la línea); no tiene
/// ningún `@transform` ni sentencia que escriba; o los tiene y sus documentos
/// no están en esta rama (no se ha hecho commit).
fn por_que_no_hay(raiz: &Path, pkg: &ore_core::link::Package, f: &str) -> Respuesta {
    let ruta = raiz.join(f);
    let Ok(fuente) = std::fs::read_to_string(&ruta) else {
        return con_motivo(
            Respuesta::error(404, format!("`{f}` is not in this branch")),
            "not_found",
        );
    };
    let mut diags = Vec::new();
    ore_core::transformar::comprobar(pkg, &mut diags);
    let rotos: Vec<Json> = diags
        .iter()
        .filter(|d| d.code.as_str() == "OOS2043" && d.file == ruta)
        .map(|d| diagnostico_json(raiz, d))
        .collect();
    if !rotos.is_empty() {
        let primero = diags
            .iter()
            .find(|d| d.code.as_str() == "OOS2043" && d.file == ruta);
        let donde = primero
            .and_then(|d| d.pos)
            .map(|p| format!("line {}: ", p.line))
            .unwrap_or_default();
        return Respuesta {
            codigo: 422,
            cuerpo: Json::obj([
                (
                    "error",
                    Json::s(format!(
                        "`{f}` has nothing to build because it does not parse: {donde}{}",
                        primero.map(|d| d.message.as_str()).unwrap_or_default()
                    )),
                ),
                ("diagnostics", Json::Arr(rotos)),
                ("motivo", Json::s("diagnostics")),
            ]),
        };
    }
    let base = f.rsplit('/').next().unwrap_or(f);
    let tiene: Vec<String> = if f.ends_with(".sql") {
        ore_core::transformar::derivar_sql(&fuente, base)
            .map(|g| {
                g.transforms
                    .iter()
                    .map(|(n, p)| format!("statement {n} → `{}`", p.output))
                    .collect()
            })
            .unwrap_or_default()
    } else {
        ore_code::python::derivar(&fuente, base)
            .transforms
            .iter()
            .map(|x| format!("`{}`", x.nombre))
            .collect()
    };
    let mensaje = if tiene.is_empty() {
        if f.ends_with(".sql") {
            format!(
                "`{f}` has nothing to build: no statement of it writes data (`create or replace dataset … as select`, `insert into … select`)"
            )
        } else {
            format!(
                "`{f}` has nothing to build: no top-level def of it has `@transform` (`from ore import transform`)"
            )
        }
    } else {
        format!(
            "`{f}` has transforms ({}), but their documents are not committed in this branch: commit the file (its documents are derived at commit) and build again",
            tiene.join(", ")
        )
    };
    con_motivo(Respuesta::error(404, mensaje), "not_found")
}

pub(crate) struct ArnesDeBuild<'a> {
    pub documento: &'a str,
    pub fichero: &'a str,
    pub fuente: &'a str,
    pub def: &'a str,
    /// `{transform, entrypoint, output}`: la procedencia, sin el id (que es el
    /// del puesto) ni el commit (que es el del trabajo, `ORE_CODIGO`).
    pub build: &'a Json,
}

/// La celda del build de un transform de Python. Como el arnés de una función
/// (`funciones::arnes`): un literal de cadena JSON es un literal de cadena de
/// Python, así que el código, la ruta y el `def` entran como cadenas y el
/// módulo se compila con `compile`. Carga el módulo —con D15 armado: un
/// `@transform` llamado al cargar falla con su línea— y llama al `def`, que
/// se declara al servidor dentro del techo que el build ya puso.
pub(crate) fn arnes_python(a: &ArnesDeBuild<'_>) -> String {
    let cad = |s: &str| Json::s(s).jcs();
    format!(
        r#"# El arnés de un build (ORE 0055 B1, B2): {documento}
import json as _json
import os as _os
import traceback as _tb

{guarda}_FICHERO = {fichero}
_DEF = {def_}
_COMMIT = (_os.environ.get("ORE_CODIGO") or "").rpartition("@")[2]
_os.environ["ORE_BUILD"] = _json.dumps(dict(_json.loads({build}), id=_ore.session.id, commit=_COMMIT))


def _linea(e):
    """La línea donde se rompió, en el fichero del transform (o `None`)."""
    for fr in reversed(_tb.extract_tb(e.__traceback__)):
        if fr.filename == _FICHERO:
            return fr.lineno
    return None


def _falla(tipo, mensaje, linea):
    """El error del build: al informe con su tipo, su fichero y su línea (B2), y
    como excepción con `(fichero, line N)` en el texto."""
    _ore._para_el_informe({{"error": {{"tipo": tipo, "fichero": _FICHERO, "linea": linea}}}})
    if linea:
        mensaje = "%s (%s, line %d)" % (mensaje, _FICHERO, linea)
    return RuntimeError(mensaje)


if not hasattr(_ore, "_para_el_informe"):
    raise RuntimeError("this job runs an older ORE SDK, which cannot build: rebuild the image")
try:
    _codigo = compile({fuente}, _FICHERO, "exec")
except SyntaxError as e:
    raise _falla("syntax", "SyntaxError: %s" % e.msg, e.lineno) from None
_modulo = {{"__name__": "ore_build", "__file__": _FICHERO}}
# D15: mientras el módulo carga, un `@transform` llamado no corre: falla.
_ore._modulo_en_carga(_FICHERO)
try:
    exec(_codigo, _modulo)
except _ore.TransformCalledWhileLoading as e:
    _ore._para_el_informe({{"error": {{"tipo": "called-while-loading", "fichero": _FICHERO, "linea": e.linea}}}})
    raise RuntimeError("%s: %s" % (_FICHERO, e)) from None
except Exception as e:  # noqa: BLE001
    raise _falla("load", "%s while loading the module: %s" % (type(e).__name__, e), _linea(e)) from None
finally:
    _ore._modulo_en_carga(None)
_f = _modulo.get(_DEF)
if not callable(_f) or getattr(_f, "output", None) is None:
    raise _falla("not-a-transform", "`%s` is not a `@transform` def of %s at this commit" % (_DEF, _FICHERO), None)
# El build lo llama él: una vez, y con lo que el documento deja.
try:
    _hecho = _f()
except Exception as e:  # noqa: BLE001
    raise _falla("runtime", "%s: %s" % (type(e).__name__, e), _linea(e)) from None
if isinstance(_hecho, dict) and "rows" in _hecho:
    print("%s · built from %s:%s · %d rows%s" % (_f.output, _FICHERO, _DEF, _hecho["rows"], " · the same write: nothing new" if _hecho.get("repeated") else ""))
    _ore._resultado_de_escritura(_hecho)
else:
    print("%s · built from %s:%s · the def did not return what `write()` returns" % (_f.output, _FICHERO, _DEF))
"#,
        documento = a.documento,
        guarda = ore_core::sdk::guarda_python(),
        fichero = cad(a.fichero),
        def_ = cad(a.def),
        build = cad(&a.build.jcs()),
        fuente = cad(a.fuente),
    )
}

/// Lo que va delante de la sentencia de un build de SQL: la procedencia del
/// build, en el entorno, para el `write()` que la celda hace.
fn preludio_sql(build: &Json) -> String {
    format!(
        "# El build de una sentencia (ORE 0055 B1).\n\
         import json as _json\n\
         import os as _os\n\
         import ore as _ore_b\n\
         _os.environ[\"ORE_BUILD\"] = _json.dumps(dict(_json.loads({}), id=_ore_b.session.id, \
         commit=(_os.environ.get(\"ORE_CODIGO\") or \"\").rpartition(\"@\")[2]))\n\n",
        Json::s(build.jcs()).jcs()
    )
}

/// D16 · `main` es una rama más, salvo protegida (`.arbol/ramas.yaml` en el
/// árbol de `main`): un build escribe, y en una `main` protegida no se escribe
/// sin propuesta.
pub(crate) fn se_construye_en(raiz: &Path, en_main: bool) -> Result<(), Respuesta> {
    if en_main && crate::politica::Politica::de_raiz(raiz).protegida {
        return Err(con_motivo(
            Respuesta::error(
                409,
                "`main` is protected: a build writes, so it runs in a branch. Build from your branch and propose",
            ),
            "protected",
        ));
    }
    Ok(())
}

/// B2 · **El porqué de un no, para una máquina**: cada respuesta de `POST
/// /builds` que no es 2xx lleva `motivo` junto a `error` —`protected`,
/// `layer` (con `retry: true`), `not_found`, `diagnostics`, `forbidden`,
/// `not_buildable`; y `invalid` (un cuerpo que no vale) o `error` (lo demás:
/// la forja, la cola)—. El texto de `error` no cambia.
pub(crate) fn con_motivo(mut r: Respuesta, motivo: &str) -> Respuesta {
    if let Json::Obj(m) = &mut r.cuerpo {
        m.insert("motivo".into(), Json::s(motivo));
    }
    r
}

/// El `motivo` de lo que llega sin él: la capa (409 con `capa`, de
/// `capa_para`), un cuerpo que no vale (400/422 de `pedido`), u otro error.
pub(crate) fn con_motivo_por_defecto(mut r: Respuesta) -> Respuesta {
    if (200..300).contains(&r.codigo) {
        return r;
    }
    let Json::Obj(m) = &mut r.cuerpo else {
        return r;
    };
    if m.contains_key("motivo") {
        return r;
    }
    if r.codigo == 409 && m.contains_key("capa") {
        m.insert("motivo".into(), Json::s("layer"));
        m.insert("retry".into(), Json::Bool(true));
    } else if matches!(r.codigo, 400 | 422) {
        m.insert("motivo".into(), Json::s("invalid"));
    } else {
        m.insert("motivo".into(), Json::s("error"));
    }
    r
}

/// Un 422 de un solo diagnóstico, con la forma de los de `comprobar`.
fn diagnostico_suelto(code: &str, mensaje: String, fichero: &str) -> Respuesta {
    Respuesta {
        codigo: 422,
        cuerpo: Json::obj([
            (
                "error",
                Json::s(format!("{mensaje} ({code}): nothing was built")),
            ),
            (
                "diagnostics",
                Json::Arr(vec![Json::obj([
                    ("code", Json::s(code)),
                    ("message", Json::s(&mensaje)),
                    ("file", Json::s(fichero)),
                ])]),
            ),
            ("motivo", Json::s("diagnostics")),
        ]),
    }
}

/// El repositorio registrado (en `main`) que contiene `codigo`: el de ruta más
/// larga que lo tiene dentro. `(ruta, plantilla)`.
fn repositorio_de<'a>(
    registrados: &'a [(String, String)],
    codigo: &str,
) -> Option<&'a (String, String)> {
    registrados
        .iter()
        .filter(|(r, _)| codigo.starts_with(&format!("{}/", r.trim_end_matches('/'))))
        .max_by_key(|(r, _)| r.len())
}

impl Servidor {
    /// `POST /builds {output | fichero, rama?}` (0055 B1).
    pub(crate) fn abrir_build(&self, sujeto: &Identidad, cuerpo: &str) -> Respuesta {
        con_motivo_por_defecto(self.abrir_build_sin_motivo(sujeto, cuerpo))
    }

    fn abrir_build_sin_motivo(&self, sujeto: &Identidad, cuerpo: &str) -> Respuesta {
        if crate::puestos::es_agente(sujeto) {
            return con_motivo(
                Respuesta::error(403, "an agent does not launch builds: a person does"),
                "forbidden",
            );
        }
        let (que, rama) = match pedido(cuerpo) {
            Ok(x) => x,
            Err(r) => return r,
        };

        // Los repositorios registrados, de `main` (su identidad y su clase son
        // un registro, 0036 ⑤); y, sin rama dicha, en qué repositorio está el
        // código, para tomar la rama de la persona en él.
        let mut registrados: Vec<(String, String)> = Vec::new();
        let mut repo_en_main: Option<String> = None;
        let r = self.leyendo(|raiz| {
            registrados = ore_core::repositorios::leer(raiz)
                .into_iter()
                .map(|r| (r.ruta, r.plantilla.unwrap_or_default()))
                .collect();
            repo_en_main = match &que {
                Que::Fichero(f) => repositorio_de(&registrados, f).map(|(r, _)| r.clone()),
                Que::Output(o) => {
                    ore_core::generar::productor_de(raiz, o).and_then(|p| p.repositorio)
                }
            };
            Respuesta::ok(Json::obj([]))
        });
        if r.codigo != 200 {
            return r;
        }
        let rama = match self.rama_del_puesto(sujeto, rama, repo_en_main.as_deref()) {
            Ok(r) => r,
            Err(r) => return r,
        };
        // D16: `main` es una rama más, salvo protegida.
        let en_main = rama
            .as_deref()
            .is_none_or(|r| self.es_la_rama_por_defecto(r));

        let mut hallados: Vec<Hallado> = Vec::new();
        let mut commit = String::from("local");
        let r = self.leyendo_en(rama.as_deref(), |raiz| {
            if let Err(r) = se_construye_en(raiz, en_main) {
                return r;
            }
            match hallar(raiz, &que) {
                Ok(h) => hallados = h,
                Err(r) => return r,
            }
            commit = std::process::Command::new("git")
                .args(["rev-parse", "--short", "HEAD"])
                .current_dir(raiz)
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "local".into());
            Respuesta::ok(Json::obj([]))
        });
        if r.codigo != 200 {
            return r;
        }

        let mut fichas = Vec::new();
        for h in hallados {
            let (repositorio, clase) = match repositorio_de(&registrados, &h.codigo) {
                Some((r, plantilla)) => (Some(r.clone()), ore_core::clases::de(plantilla)),
                None => (None, None),
            };
            if let Some(c) = clase
                && !c.escribe
            {
                return con_motivo(
                    Respuesta::error(
                        422,
                        format!(
                            "`{}` is a `{}` repository, and that class does not write data: its transforms do not build",
                            repositorio.unwrap_or_default(),
                            c.id
                        ),
                    ),
                    "not_buildable",
                );
            }
            let repositorio_del_build = repositorio.clone();
            // 0049 B4·2: lo que lee y es una colección, fijado al lanzar.
            let fijadas = self.fijar_colecciones(rama.as_deref(), &h.inputs);
            let techo = Transform {
                nombre: h.nombre.clone(),
                inputs: h.inputs.clone(),
                output: h.output.clone(),
                fijadas,
                techo: true,
            };
            let mut r = self.lanzar_trabajo(
                sujeto,
                rama.clone(),
                "python",
                "python",
                h.codigo.clone(),
                commit.clone(),
                h.celda,
                Vec::new(),
                Some(techo),
                None,
                Donde {
                    repositorio,
                    clase,
                    build: Some(Construccion {
                        documento: h.documento.clone(),
                        entrypoint: h.entrypoint.clone(),
                        output: h.output.clone(),
                        repositorio: repositorio_del_build,
                        creado_s: ahora_s(),
                    }),
                },
            );
            if r.codigo != 202 {
                if !fichas.is_empty()
                    && let Json::Obj(m) = &mut r.cuerpo
                {
                    m.insert("launched".into(), Json::Arr(fichas));
                }
                return r;
            }
            // B2: la ficha del trabajo, con el `Build` del historial encima
            // (`estado: queued`, `creado`…): la misma forma que `GET /builds`.
            if let Json::Obj(m) = &mut r.cuerpo {
                let id = match m.get("id") {
                    Some(Json::Str(s)) => s.clone(),
                    _ => String::new(),
                };
                let visto = self.con_los_puestos(|l| {
                    l.get(&id).and_then(|p| visto_de_puesto(&id, p, ahora_s()))
                });
                if let Some(Json::Obj(b)) = visto.map(|v| v.a_json(false)) {
                    m.extend(b);
                }
            }
            fichas.push(r.cuerpo);
        }
        match que {
            Que::Output(_) if fichas.len() == 1 => Respuesta {
                codigo: 202,
                cuerpo: fichas.pop().unwrap_or_else(|| Json::obj([])),
            },
            Que::Output(_) | Que::Fichero(_) => Respuesta {
                codigo: 202,
                cuerpo: Json::obj([
                    (
                        "fichero",
                        Json::s(match &que {
                            Que::Fichero(f) => f.as_str(),
                            Que::Output(o) => o.as_str(),
                        }),
                    ),
                    ("builds", Json::Arr(fichas)),
                ]),
            },
        }
    }
}

// ── B2 · el historial ───────────────────────────────────────────────────────

fn ahora_s() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Cuánto `log` se enseña: lo último, que es donde está lo que pasó.
const LOG_MAXIMO: usize = 64 * 1024;

/// El error de un build, como lo lee la consola: `tipo` es `syntax`, `load`,
/// `called-while-loading` (D15), `not-a-transform`, `runtime` o `lost`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ErrorDeBuild {
    pub tipo: String,
    pub mensaje: String,
    pub fichero: Option<String>,
    pub linea: Option<i64>,
}

/// **Un build como lo dice el historial** (B2): de lo vivo —un puesto en la
/// memoria— o de su informe (`trabajos/<id>.json`), que sobrevive a un
/// reinicio.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Visto {
    pub build: String,
    pub output: String,
    pub transform: String,
    pub entrypoint: String,
    pub commit: String,
    pub rama: String,
    pub repositorio: Option<String>,
    pub quien: String,
    /// `queued | starting | running | succeeded | failed`.
    pub estado: &'static str,
    pub creado_s: u64,
    pub inicio_s: Option<u64>,
    pub fin_s: Option<u64>,
    pub ms: Option<i64>,
    pub filas: Option<i64>,
    pub snapshot: Option<String>,
    pub error: Option<ErrorDeBuild>,
    pub log: Option<String>,
}

impl Visto {
    /// El `Build` del contrato; `log` sólo en el detalle.
    pub(crate) fn a_json(&self, detalle: bool) -> Json {
        let mut m = vec![
            ("build", Json::s(&self.build)),
            ("output", Json::s(&self.output)),
            ("transform", Json::s(&self.transform)),
            ("entrypoint", Json::s(&self.entrypoint)),
            ("commit", Json::s(&self.commit)),
            ("rama", Json::s(&self.rama)),
            (
                "repositorio",
                self.repositorio
                    .as_deref()
                    .map(Json::s)
                    .unwrap_or(Json::Crudo("null".into())),
            ),
            ("quien", Json::s(&self.quien)),
            ("estado", Json::s(self.estado)),
            ("creado", Json::s(crate::assets::rfc3339(self.creado_s))),
        ];
        if let Some(i) = self.inicio_s {
            m.push(("inicio", Json::s(crate::assets::rfc3339(i))));
        }
        if let Some(f) = self.fin_s {
            m.push(("fin", Json::s(crate::assets::rfc3339(f))));
        }
        if let Some(ms) = self.ms {
            m.push(("ms", Json::Int(ms)));
        }
        if let Some(f) = self.filas {
            m.push(("filas", Json::Int(f)));
        }
        if let Some(s) = &self.snapshot {
            m.push(("snapshot", Json::s(s)));
        }
        if let Some(e) = &self.error {
            let mut x = vec![("tipo", Json::s(&e.tipo)), ("mensaje", Json::s(&e.mensaje))];
            if let Some(f) = &e.fichero {
                x.push(("fichero", Json::s(f)));
            }
            if let Some(l) = e.linea {
                x.push(("linea", Json::Int(l)));
            }
            m.push(("error", Json::obj(x)));
        }
        if detalle && let Some(l) = &self.log {
            m.push(("log", Json::s(l)));
        }
        Json::obj(m)
    }
}

/// Un campo de texto (sin `null`).
fn texto(n: &Node, k: &str) -> Option<String> {
    n.get(k)
        .and_then(|(_, v)| v.as_str())
        .filter(|s| !matches!(*s, "null" | "~" | ""))
        .map(str::to_string)
}

fn entero(n: &Node, k: &str) -> Option<i64> {
    texto(n, k).and_then(|s| s.parse().ok())
}

/// Lo que la salida de la celda dice del build: si salió bien, sus filas y
/// su snapshot, o su error con el fichero y la línea que el arnés puso en el
/// informe de la celda (no se lee del texto); y lo impreso, para el `log`.
struct DeLaSalida {
    fallo: bool,
    ms: Option<i64>,
    filas: Option<i64>,
    snapshot: Option<String>,
    error: Option<ErrorDeBuild>,
    log: Option<String>,
}

fn de_la_salida(salida: &Node, codigo: &str) -> DeLaSalida {
    let fallo = texto(salida, "tipo").as_deref() == Some("error");
    let informe = salida.get("informe").map(|(_, v)| v);
    let error_informado = informe.and_then(|i| i.get("error")).map(|(_, v)| v);
    let error = fallo.then(|| ErrorDeBuild {
        tipo: error_informado
            .and_then(|e| texto(e, "tipo"))
            .unwrap_or_else(|| "runtime".into()),
        mensaje: texto(salida, "mensaje")
            .or_else(|| texto(salida, "nombre"))
            .unwrap_or_else(|| "the build failed".into()),
        fichero: error_informado
            .and_then(|e| texto(e, "fichero"))
            .or_else(|| Some(codigo.to_string())),
        linea: error_informado.and_then(|e| entero(e, "linea")),
    });
    let log = texto(salida, "texto").map(|t| {
        if t.len() <= LOG_MAXIMO {
            return t;
        }
        let mut corte = t.len() - LOG_MAXIMO;
        while !t.is_char_boundary(corte) {
            corte += 1;
        }
        format!("… (the first {corte} bytes are not shown)\n{}", &t[corte..])
    });
    DeLaSalida {
        fallo,
        ms: entero(salida, "ms"),
        filas: informe.and_then(|i| entero(i, "filas")),
        snapshot: informe.and_then(|i| texto(i, "snapshot")),
        error,
        log,
    }
}

/// Un build vivo (en la memoria), o `None` si el puesto no es un build.
pub(crate) fn visto_de_puesto(id: &str, p: &crate::puestos::Puesto, ahora: u64) -> Option<Visto> {
    use crate::puestos::Estado;
    let t = p.trabajo.as_ref()?;
    let b = t.build.as_ref()?;
    let celda = p.celdas.get(&1);
    let inicio_s = celda
        .and_then(|c| c.empezada)
        .map(|e| ahora.saturating_sub(e.elapsed().as_secs()));
    let mut v = Visto {
        build: id.to_string(),
        output: b.output.clone(),
        transform: b.documento.clone(),
        entrypoint: b.entrypoint.clone(),
        commit: t.commit.clone(),
        rama: p.rama.clone().unwrap_or_default(),
        repositorio: b.repositorio.clone(),
        quien: p.persona.clone(),
        estado: "queued",
        creado_s: b.creado_s,
        inicio_s,
        fin_s: None,
        ms: None,
        filas: None,
        snapshot: None,
        error: None,
        log: None,
    };
    let salida = celda
        .and_then(|c| c.salida.as_ref())
        .and_then(|s| ore_core::parse::parse(&s.jcs()).ok());
    if let Some(s) = salida {
        let d = de_la_salida(&s, &t.codigo);
        v.estado = if d.fallo { "failed" } else { "succeeded" };
        v.fin_s = t
            .informe
            .as_ref()
            .and_then(|i| ore_core::parse::parse(&i.jcs()).ok())
            .and_then(|i| entero(&i, "terminado_s"))
            .map(|x| x as u64)
            .or(Some(ahora));
        (v.ms, v.filas, v.snapshot, v.error, v.log) = (d.ms, d.filas, d.snapshot, d.error, d.log);
    } else if p.estado == Estado::Cerrado || crate::puestos::perdido(p) {
        v.estado = "failed";
        v.error = Some(ErrorDeBuild {
            tipo: "lost".into(),
            mensaje: "the build's job ended without reporting a result".into(),
            fichero: None,
            linea: None,
        });
    } else if inicio_s.is_some() {
        v.estado = "running";
    } else if p.estado == Estado::Vivo {
        v.estado = "starting";
    }
    Some(v)
}

/// Un build terminado, de su informe (`trabajos/<id>.json`); `None` si el
/// informe no es el de un build.
pub(crate) fn visto_de_informe(n: &Node) -> Option<Visto> {
    let b = n.get("build").map(|(_, v)| v)?;
    let build = texto(b, "id")?;
    let codigo = texto(n, "codigo").unwrap_or_default();
    let salida = n.get("salida").map(|(_, v)| v);
    let d = salida.map(|s| de_la_salida(s, &codigo));
    let fallo =
        d.as_ref().is_none_or(|d| d.fallo) || texto(n, "estado").as_deref() == Some("error");
    let d = d.unwrap_or(DeLaSalida {
        fallo: true,
        ms: None,
        filas: None,
        snapshot: None,
        error: None,
        log: None,
    });
    Some(Visto {
        build,
        output: texto(b, "output").unwrap_or_default(),
        transform: texto(b, "transform").unwrap_or_default(),
        entrypoint: texto(b, "entrypoint").unwrap_or_default(),
        commit: texto(b, "commit")
            .or_else(|| texto(n, "commit"))
            .unwrap_or_default(),
        rama: texto(b, "rama").unwrap_or_default(),
        repositorio: texto(b, "repositorio"),
        quien: texto(n, "persona").unwrap_or_default(),
        estado: if fallo { "failed" } else { "succeeded" },
        creado_s: entero(b, "creado_s").unwrap_or(0) as u64,
        inicio_s: entero(b, "inicio_s").map(|x| x as u64),
        fin_s: entero(n, "terminado_s").map(|x| x as u64),
        ms: entero(n, "ms").or(d.ms),
        filas: d.filas,
        snapshot: d.snapshot,
        error: if fallo {
            d.error.or(Some(ErrorDeBuild {
                tipo: "runtime".into(),
                mensaje: "the build failed".into(),
                fichero: Some(codigo),
                linea: None,
            }))
        } else {
            None
        },
        log: d.log,
    })
}

/// Lo que se pide en `GET /builds`.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Filtro {
    pub output: Option<String>,
    pub repositorio: Option<String>,
    pub rama: Option<String>,
    pub limit: usize,
}

impl Filtro {
    pub(crate) fn de(
        consulta: &std::collections::BTreeMap<String, String>,
    ) -> Result<Filtro, Respuesta> {
        let c = |k: &str| {
            consulta
                .get(k)
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let limit = match c("limit") {
            None => 20,
            Some(l) => match l.parse::<usize>() {
                Ok(n) if (1..=500).contains(&n) => n,
                _ => {
                    return Err(Respuesta::error(
                        422,
                        format!("`limit={l}` is not a number from 1 to 500"),
                    ));
                }
            },
        };
        Ok(Filtro {
            output: c("output").map(|o| ore_core::normalize::a_corto(&o).into_owned()),
            repositorio: c("repositorio").map(|r| r.trim_matches('/').to_string()),
            rama: c("rama"),
            limit,
        })
    }

    fn deja(&self, v: &Visto) -> bool {
        self.output.as_ref().is_none_or(|o| *o == v.output)
            && self
                .repositorio
                .as_ref()
                .is_none_or(|r| v.repositorio.as_deref() == Some(r.as_str()))
            && self.rama.as_ref().is_none_or(|r| *r == v.rama)
    }
}

/// Lo vivo y lo informado juntos, sin repetir (lo vivo manda: es lo último),
/// de quien pregunta, filtrado, del más nuevo al más viejo.
pub(crate) fn historial(
    vivos: Vec<Visto>,
    informados: Vec<Visto>,
    quien: &str,
    f: &Filtro,
) -> Vec<Visto> {
    let mut por_id: std::collections::BTreeMap<String, Visto> = Default::default();
    for v in informados.into_iter().chain(vivos) {
        por_id.insert(v.build.clone(), v);
    }
    let mut todos: Vec<Visto> = por_id
        .into_values()
        .filter(|v| v.quien == quien && f.deja(v))
        .collect();
    todos.sort_by(|a, b| b.creado_s.cmp(&a.creado_s).then(b.build.cmp(&a.build)));
    todos.truncate(f.limit);
    todos
}

/// Los informes de build de unos `trabajos/<id>.json`.
fn de_los_textos<'a>(textos: impl Iterator<Item = &'a str>) -> Vec<Visto> {
    textos
        .filter(|t| t.contains("\"build\""))
        .filter_map(|t| ore_core::parse::parse(t).ok())
        .filter_map(|n| visto_de_informe(&n))
        .collect()
}

impl Servidor {
    /// Los builds de lo vivo, de quien sea (el filtro de quién, en `historial`).
    fn builds_vivos(&self) -> Vec<Visto> {
        let ahora = ahora_s();
        self.con_los_puestos(|l| {
            l.iter()
                .filter_map(|(id, p)| visto_de_puesto(id, p, ahora))
                .collect()
        })
    }

    /// Los builds terminados, de sus informes: en todas las ramas de la forja
    /// (un build confirma el suyo en su rama), o en el directorio del árbol.
    fn builds_informados(&self) -> Vec<Visto> {
        if let crate::rutas::Arbol::Forja(forja) = &self.arbol
            && let Ok(fs) = forja.en_las_ramas("trabajos", "trabajo-")
        {
            return de_los_textos(fs.iter().map(|(_, _, t)| t.as_str()));
        }
        let mut textos = Vec::new();
        let _ = self.leyendo(|raiz| {
            if let Ok(es) = std::fs::read_dir(raiz.join("trabajos")) {
                textos = es
                    .flatten()
                    .filter(|e| e.file_name().to_string_lossy().starts_with("trabajo-"))
                    .filter_map(|e| std::fs::read_to_string(e.path()).ok())
                    .collect();
            }
            Respuesta::ok(Json::obj([]))
        });
        de_los_textos(textos.iter().map(String::as_str))
    }

    /// `GET /builds?output=&repositorio=&rama=&limit=` (B2): los de quien
    /// pregunta —como `GET /trabajos`—, del más nuevo al más viejo.
    pub(crate) fn builds_de(
        &self,
        sujeto: &Identidad,
        consulta: &std::collections::BTreeMap<String, String>,
    ) -> Respuesta {
        let f = match Filtro::de(consulta) {
            Ok(f) => f,
            Err(r) => return r,
        };
        let h = historial(
            self.builds_vivos(),
            self.builds_informados(),
            &sujeto.persona,
            &f,
        );
        Respuesta::ok(Json::obj([(
            "builds",
            Json::Arr(h.iter().map(|v| v.a_json(false)).collect()),
        )]))
    }

    /// `GET /builds/{id}` (B2): el build, con su `log`.
    pub(crate) fn build(&self, sujeto: &Identidad, id: &str) -> Respuesta {
        let vivo = self.builds_vivos().into_iter().find(|v| v.build == id);
        let v = match vivo {
            Some(v) => Some(v),
            None => self.builds_informados().into_iter().find(|v| v.build == id),
        };
        match v {
            None => Respuesta::error(404, format!("there is no build `{id}`")),
            Some(v) if v.quien != sujeto.persona && !crate::puestos::es_agente(sujeto) => {
                Respuesta::error(403, "that build is someone else's")
            }
            Some(v) => Respuesta::ok(v.a_json(true)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bien<T>(r: Result<T, Respuesta>) -> T {
        match r {
            Ok(x) => x,
            Err(r) => panic!("{} {}", r.codigo, r.cuerpo.jcs()),
        }
    }

    fn mal<T>(r: Result<T, Respuesta>) -> Respuesta {
        match r {
            Ok(_) => panic!("debía fallar"),
            Err(r) => r,
        }
    }

    fn arbol(nombre: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("builds-{nombre}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let p = d.join("packages/ventas");
        std::fs::create_dir_all(p.join("etl")).unwrap();
        std::fs::write(
            p.join("package.yaml"),
            "apiVersion: oos.dev/v1alpha1\nkind: Package\n\
             metadata: { name: ventas, version: 1.0.0, status: active, domain: v }\n\
             spec: { owner: team:v }\n",
        )
        .unwrap();
        std::fs::create_dir_all(p.join("datasets")).unwrap();
        std::fs::write(
            p.join("datasets/clientes.yaml"),
            "apiVersion: oos.dev/v1alpha12\nkind: Dataset\n\
             metadata: { name: clientes, namespace: ventas }\n\
             spec:\n  owner: team:v\n  changes: { mode: append }\n  columns:\n    id: { type: Integer }\n",
        )
        .unwrap();
        std::fs::write(
            p.join("etl/limpios.py"),
            "from ore import transform, over, write\n\n\n\
             @transform(inputs=[\"ventas.clientes\"], output=\"ventas.limpios\")\n\
             def limpios():\n    return write(\"ventas.limpios\", over(\"ventas.clientes\"))\n\n\n\
             @transform(inputs=[\"ventas.clientes\"], output=\"ventas.otros\")\n\
             def otros():\n    return write(\"ventas.otros\", over(\"ventas.clientes\"))\n",
        )
        .unwrap();
        std::fs::write(
            p.join("etl/carga.sql"),
            "select 1;\ncreate or replace dataset ventas.resumen as select * from ventas.clientes;\n",
        )
        .unwrap();
        // Los documentos, como los deriva el commit.
        let (pkg, _) = ore_core::validate::cargar_paquete(&d);
        let plan = ore_core::generar::plan_de_transforms(&pkg, None, Some("user:ana"));
        ore_core::generar::aplicar(&plan).unwrap();
        d
    }

    #[test]
    fn el_pedido_es_una_salida_o_un_fichero() {
        assert_eq!(
            bien(pedido(r#"{"output": "ventas.default.limpios"}"#)),
            (Que::Output("ventas.limpios".into()), None)
        );
        assert_eq!(
            bien(pedido(
                r#"{"fichero": "packages/ventas/etl/limpios.py", "rama": "ana/etl"}"#
            )),
            (
                Que::Fichero("packages/ventas/etl/limpios.py".into()),
                Some("ana/etl".into())
            )
        );
        for malo in [
            r#"{}"#,
            r#"{"output": "limpios"}"#,
            r#"{"output": "a.b", "fichero": "x.py"}"#,
            r#"{"fichero": "../x.py"}"#,
            r#"{"fichero": "packages/x.txt"}"#,
        ] {
            assert_eq!(mal(pedido(malo)).codigo, 422, "{malo}");
        }
        assert_eq!(mal(pedido("")).codigo, 400);
    }

    #[test]
    fn se_halla_por_salida_o_por_fichero_y_404_sin_transform() {
        let d = arbol("hallar");
        let h = bien(hallar(&d, &Que::Output("ventas.limpios".into())));
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].entrypoint, "etl/limpios.py:limpios");
        assert_eq!(h[0].codigo, "packages/ventas/etl/limpios.py");
        assert_eq!(h[0].inputs, vec!["ventas.clientes".to_string()]);
        assert!(
            h[0].documento.ends_with("pipeline/ventas.limpios.yaml"),
            "{}",
            h[0].documento
        );
        assert!(h[0].celda.contains(r#"_DEF = "limpios""#), "{}", h[0].celda);
        // D14: todos los del fichero.
        let h = bien(hallar(
            &d,
            &Que::Fichero("packages/ventas/etl/limpios.py".into()),
        ));
        let mut salidas: Vec<&str> = h.iter().map(|x| x.output.as_str()).collect();
        salidas.sort();
        assert_eq!(salidas, ["ventas.limpios", "ventas.otros"]);
        // SQL: la sentencia 2, con la procedencia del build delante.
        let h = bien(hallar(&d, &Que::Output("ventas.resumen".into())));
        assert_eq!(h[0].entrypoint, "etl/carga.sql:2");
        assert!(h[0].celda.contains("ORE_BUILD"), "{}", h[0].celda);
        assert!(h[0].celda.contains("@transform("), "{}", h[0].celda);
        assert!(!h[0].celda.contains("select 1"), "{}", h[0].celda);
        // Nadie la escribe: 404.
        let r = mal(hallar(&d, &Que::Output("ventas.nadie".into())));
        assert_eq!(r.codigo, 404);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn un_documento_que_no_es_el_de_su_codigo_es_422_con_sus_diagnosticos() {
        let d = arbol("incoherente");
        // El código cambia después del commit del documento: lee otra cosa.
        let f = d.join("packages/ventas/etl/limpios.py");
        let t = std::fs::read_to_string(&f).unwrap().replacen(
            "inputs=[\"ventas.clientes\"], output=\"ventas.limpios\"",
            "inputs=[\"ventas.pedidos\"], output=\"ventas.limpios\"",
            1,
        );
        std::fs::write(&f, t).unwrap();
        let r = mal(hallar(&d, &Que::Output("ventas.limpios".into())));
        assert_eq!(r.codigo, 422);
        let j = r.cuerpo.jcs();
        assert!(j.contains("OOS2013"), "{j}");
        assert!(j.contains(r#""motivo":"diagnostics""#), "{j}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn una_salida_sin_su_schema_no_se_construye() {
        let d = arbol("schema");
        let f = d.join("packages/ventas/etl/limpios.py");
        let t = std::fs::read_to_string(&f)
            .unwrap()
            .replace("ventas.limpios", "ventas.crudo.limpios");
        std::fs::write(&f, t).unwrap();
        let (pkg, _) = ore_core::validate::cargar_paquete(&d);
        let plan = ore_core::generar::plan_de_transforms(&pkg, None, Some("user:ana"));
        ore_core::generar::aplicar(&plan).unwrap();
        let r = mal(hallar(&d, &Que::Output("ventas.crudo.limpios".into())));
        assert_eq!(r.codigo, 422);
        assert!(r.cuerpo.jcs().contains("OOS2037"), "{}", r.cuerpo.jcs());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn el_arnes_del_build_no_interpola_codigo() {
        let build = Json::obj([
            (
                "transform",
                Json::s("packages/ventas/etl/pipeline/ventas.limpios.yaml"),
            ),
            ("entrypoint", Json::s("etl/limpios.py:limpios")),
            ("output", Json::s("ventas.limpios")),
        ]);
        let t = arnes_python(&ArnesDeBuild {
            documento: "packages/ventas/etl/pipeline/ventas.limpios.yaml",
            fichero: "packages/ventas/etl/limpios.py",
            fuente: "\"\"\"); import os #\ndef limpios():\n    pass\n",
            def: "limpios",
            build: &build,
        });
        // El código va en un literal de cadena JSON, escapado, y se compila.
        assert!(
            t.contains(
                r#"compile("\"\"\"); import os #\ndef limpios():\n    pass\n", _FICHERO, "exec")"#
            ),
            "{t}"
        );
        assert!(
            t.contains(r#"_FICHERO = "packages/ventas/etl/limpios.py""#),
            "{t}"
        );
        assert!(t.contains(r#"_DEF = "limpios""#), "{t}");
        // D15 armado mientras carga, y desarmado después.
        assert!(t.contains("_ore._modulo_en_carga(_FICHERO)"), "{t}");
        assert!(t.contains("_ore._modulo_en_carga(None)"), "{t}");
        // B2: el error va al informe con su tipo, su fichero y su línea.
        for tipo in [
            "syntax",
            "load",
            "not-a-transform",
            "runtime",
            "called-while-loading",
        ] {
            assert!(t.contains(&format!("\"{tipo}\"")), "{tipo}: {t}");
        }
        // La línea del fichero del usuario en los errores.
        assert!(
            t.contains(r#""%s (%s, line %d)" % (mensaje, _FICHERO, linea)"#),
            "{t}"
        );
        assert!(t.contains(&ore_core::sdk::guarda_python()), "{t}");
        let antes = ore_core::sdk::nombres_de_antes_en(&t);
        assert!(antes.is_empty(), "{antes:?}");
        if let Ok(dir) = std::env::var("ORE_CELDAS_GENERADAS") {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(std::path::Path::new(&dir).join("91-arnes-de-build.py"), &t).unwrap();
        }
    }

    #[test]
    fn en_main_protegida_no_se_construye_y_en_una_rama_si() {
        let d = arbol("protegida");
        assert!(se_construye_en(&d, true).is_ok());
        std::fs::create_dir_all(d.join(".arbol")).unwrap();
        std::fs::write(
            d.join(crate::politica::RUTA),
            "main:
  protegida: true
",
        )
        .unwrap();
        assert_eq!(mal(se_construye_en(&d, true)).codigo, 409);
        assert!(se_construye_en(&d, false).is_ok());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn cada_no_lleva_su_motivo() {
        let motivo = |r: &Respuesta| match &r.cuerpo {
            Json::Obj(m) => match m.get("motivo") {
                Some(Json::Str(s)) => s.clone(),
                _ => String::new(),
            },
            _ => String::new(),
        };
        let d = arbol("motivos");
        assert_eq!(
            motivo(&mal(hallar(&d, &Que::Output("ventas.nadie".into())))),
            "not_found"
        );
        std::fs::create_dir_all(d.join(".arbol")).unwrap();
        std::fs::write(d.join(crate::politica::RUTA), "main:\n  protegida: true\n").unwrap();
        assert_eq!(motivo(&mal(se_construye_en(&d, true))), "protected");
        let _ = std::fs::remove_dir_all(&d);
        // La capa que no está lista (409 con `capa`): se reintenta.
        let capa = con_motivo_por_defecto(Respuesta {
            codigo: 409,
            cuerpo: Json::obj([("error", Json::s("x")), ("capa", Json::s("capa-1"))]),
        });
        assert_eq!(motivo(&capa), "layer");
        assert!(capa.cuerpo.jcs().contains(r#""retry":true"#));
        assert_eq!(
            motivo(&con_motivo_por_defecto(mal(pedido("{}")))),
            "invalid"
        );
        assert_eq!(
            motivo(&con_motivo_por_defecto(Respuesta::error(502, "forja"))),
            "error"
        );
        // Lo que ya lo trae no se toca, y un 202 no lo lleva.
        let ya = con_motivo_por_defecto(con_motivo(Respuesta::error(422, "x"), "not_buildable"));
        assert_eq!(motivo(&ya), "not_buildable");
        let ok = con_motivo_por_defecto(Respuesta {
            codigo: 202,
            cuerpo: Json::obj([]),
        });
        assert_eq!(motivo(&ok), "");
    }

    /// 0055 · «Nothing to build» dice por qué.
    #[test]
    fn nada_que_construir_dice_por_que() {
        let d = arbol("por-que");
        let error = |r: &Respuesta| match &r.cuerpo {
            Json::Obj(m) => match m.get("error") {
                Some(Json::Str(s)) => s.clone(),
                _ => String::new(),
            },
            _ => String::new(),
        };
        // Sin `@transform`.
        std::fs::write(
            d.join("packages/ventas/etl/nada.py"),
            "def ayuda():\n    return 1\n",
        )
        .unwrap();
        let r = mal(hallar(
            &d,
            &Que::Fichero("packages/ventas/etl/nada.py".into()),
        ));
        assert_eq!(r.codigo, 404);
        assert!(
            error(&r).contains("no top-level def of it has `@transform`"),
            "{}",
            error(&r)
        );
        // Con transforms sin commitear (sin documento).
        std::fs::write(
            d.join("packages/ventas/etl/nuevo.py"),
            "from ore import transform\n\n\n@transform(inputs=[], output=\"ventas.nuevo\")\ndef nuevo():\n    pass\n",
        )
        .unwrap();
        let r = mal(hallar(
            &d,
            &Que::Fichero("packages/ventas/etl/nuevo.py".into()),
        ));
        assert_eq!(r.codigo, 404);
        assert!(
            error(&r).contains("has transforms (`nuevo`), but their documents are not committed"),
            "{}",
            error(&r)
        );
        // Que no se lee: 422 con su OOS2043 y su línea.
        std::fs::write(
            d.join("packages/ventas/etl/roto.py"),
            "from ore import transform\n\n\ndef ayuda():\n    return 1\n    @transform(inputs=[], output=\"ventas.roto\")\ndef roto():\n    pass\n",
        )
        .unwrap();
        let r = mal(hallar(
            &d,
            &Que::Fichero("packages/ventas/etl/roto.py".into()),
        ));
        assert_eq!(r.codigo, 422);
        let j = r.cuerpo.jcs();
        assert!(
            j.contains(r#""motivo":"diagnostics""#) && j.contains("OOS2043"),
            "{j}"
        );
        assert!(error(&r).contains("does not parse: line "), "{}", error(&r));
        // Que no está.
        let r = mal(hallar(
            &d,
            &Que::Fichero("packages/ventas/etl/no-esta.py".into()),
        ));
        assert_eq!(r.codigo, 404);
        assert!(error(&r).contains("is not in this branch"), "{}", error(&r));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn el_repositorio_es_el_registrado_mas_hondo_que_lo_contiene() {
        let r = vec![
            (
                "packages/ventas/etl".to_string(),
                "transforms-python".to_string(),
            ),
            (
                "packages/ventas/etl/v2".to_string(),
                "transforms-sql".to_string(),
            ),
        ];
        assert_eq!(
            repositorio_de(&r, "packages/ventas/etl/x.py").map(|x| x.0.as_str()),
            Some("packages/ventas/etl")
        );
        assert_eq!(
            repositorio_de(&r, "packages/ventas/etl/v2/x.sql").map(|x| x.0.as_str()),
            Some("packages/ventas/etl/v2")
        );
        assert_eq!(repositorio_de(&r, "packages/ventas/etlx/x.py"), None);
    }

    // ── B2 · el historial ───────────────────────────────────────────────

    fn un_build(_id: &str, creado_s: u64) -> crate::puestos::Puesto {
        let mut p = crate::puestos::prueba::un_puesto("agente:ana");
        p.estado = crate::puestos::Estado::Encolado;
        p.agente = None;
        p.rama = Some("ana/etl".into());
        p.trabajo = Some(crate::puestos::Trabajo {
            codigo: "packages/ventas/etl/limpios.py".into(),
            commit: "abc1234".into(),
            informe: None,
            funcion: None,
            build: Some(Construccion {
                documento: "packages/ventas/etl/pipeline/ventas.limpios.yaml".into(),
                entrypoint: "etl/limpios.py:limpios".into(),
                output: "ventas.limpios".into(),
                repositorio: Some("packages/ventas/etl".into()),
                creado_s,
            }),
        });
        p.celdas.insert(
            1,
            crate::puestos::Celda {
                texto: String::new(),
                lenguaje: "python".into(),
                corre: None,
                avisos: Vec::new(),
                enviada: std::time::Instant::now(),
                empezada: None,
                salida: None,
                lote: None,
            },
        );
        p
    }

    #[test]
    fn el_estado_de_un_build_vivo_sigue_a_su_puesto_y_a_su_celda() {
        let ahora = 1_791_360_000; // 2026-10-07T08:00:00Z
        let mut p = un_build("trabajo-ana-1", ahora - 10);
        let v = |p: &crate::puestos::Puesto| visto_de_puesto("trabajo-ana-1", p, ahora).unwrap();
        assert_eq!(v(&p).estado, "queued");
        let j = v(&p).a_json(false).jcs();
        assert!(j.contains(r#""creado":"2026-10-07T07:59:50Z""#), "{j}");
        assert!(j.contains(r#""repositorio":"packages/ventas/etl""#), "{j}");
        assert!(!j.contains("inicio"), "{j}");
        p.estado = crate::puestos::Estado::Vivo;
        p.agente = Some("agente:ana".into());
        p.latido = Some(std::time::Instant::now());
        assert_eq!(v(&p).estado, "starting");
        p.celdas.get_mut(&1).unwrap().empezada = Some(std::time::Instant::now());
        assert_eq!(v(&p).estado, "running");
        assert_eq!(v(&p).inicio_s, Some(ahora));
        // Bien: filas y snapshot del informe de la celda, y lo impreso.
        p.celdas.get_mut(&1).unwrap().salida = Some(Json::Crudo(
            r#"{"tipo":"texto","texto":"ventas.limpios · 3 rows\n","ms":120,"informe":{"filas":3,"snapshot":"8812"}}"#.into(),
        ));
        let x = v(&p);
        assert_eq!(
            (x.estado, x.ms, x.filas, x.snapshot.as_deref()),
            ("succeeded", Some(120), Some(3), Some("8812"))
        );
        assert!(!x.a_json(false).jcs().contains("log"));
        assert!(
            x.a_json(true)
                .jcs()
                .contains(r#""log":"ventas.limpios · 3 rows\n""#)
        );
        // Mal: el error con el fichero y la línea que el arnés informó.
        p.celdas.get_mut(&1).unwrap().salida = Some(Json::Crudo(
            r#"{"tipo":"error","nombre":"RuntimeError","mensaje":"KeyError: 'x' (packages/ventas/etl/limpios.py, line 7)","ms":5,"informe":{"error":{"tipo":"runtime","fichero":"packages/ventas/etl/limpios.py","linea":7}}}"#.into(),
        ));
        let j = v(&p).a_json(false).jcs();
        assert!(j.contains(r#""estado":"failed""#), "{j}");
        assert!(
            j.contains(r#""error":{"fichero":"packages/ventas/etl/limpios.py","linea":7,"mensaje":"KeyError: 'x' (packages/ventas/etl/limpios.py, line 7)","tipo":"runtime"}"#),
            "{j}"
        );
        // Cerrado sin decir nada: perdido.
        p.celdas.get_mut(&1).unwrap().salida = None;
        p.estado = crate::puestos::Estado::Cerrado;
        assert_eq!(v(&p).error.unwrap().tipo, "lost");
        // Un trabajo que no es un build no está en el historial.
        let mut t = crate::puestos::prueba::un_puesto("agente:ana");
        t.trabajo = un_build("x", 0).trabajo.map(|mut x| {
            x.build = None;
            x
        });
        assert!(visto_de_puesto("x", &t, ahora).is_none());
    }

    /// Tras un reinicio el puesto ya no está: el build sale de su informe.
    #[test]
    fn tras_un_reinicio_el_build_sale_de_su_informe() {
        let informe = r#"{
  "id": "trabajo-ana-1", "codigo": "packages/ventas/etl/limpios.py", "commit": "abc1234",
  "persona": "persona:ana", "rama": "ana/etl", "estado": "error", "ms": 900,
  "terminado_s": 1791360100,
  "salida": {"tipo": "error", "nombre": "RuntimeError", "texto": "cargando\n",
             "mensaje": "packages/ventas/etl/limpios.py: line 9 calls `limpios()` while the module loads; a Build calls it itself — remove the call",
             "informe": {"error": {"tipo": "called-while-loading", "fichero": "packages/ventas/etl/limpios.py", "linea": 9}}},
  "build": {"id": "trabajo-ana-1", "output": "ventas.limpios",
            "transform": "packages/ventas/etl/pipeline/ventas.limpios.yaml",
            "entrypoint": "etl/limpios.py:limpios", "commit": "abc1234", "rama": "ana/etl",
            "repositorio": "packages/ventas/etl", "creado_s": 1791360000, "inicio_s": 1791360008}
}"#;
        let v = visto_de_informe(&ore_core::parse::parse(informe).unwrap()).unwrap();
        let j = v.a_json(true).jcs();
        for esperado in [
            r#""build":"trabajo-ana-1""#,
            r#""estado":"failed""#,
            r#""creado":"2026-10-07T08:00:00Z""#,
            r#""inicio":"2026-10-07T08:00:08Z""#,
            r#""fin":"2026-10-07T08:01:40Z""#,
            r#""ms":900"#,
            r#""quien":"persona:ana""#,
            r#""tipo":"called-while-loading""#,
            r#""linea":9"#,
            r#""log":"cargando\n""#,
        ] {
            assert!(j.contains(esperado), "{esperado}: {j}");
        }
        // Un informe de un trabajo que no es un build no cuenta.
        let otro =
            r#"{"id": "trabajo-ana-2", "persona": "persona:ana", "salida": {"tipo": "vacia"}}"#;
        assert!(de_los_textos([informe, otro].into_iter()).len() == 1);
    }

    #[test]
    fn el_historial_filtra_ordena_y_no_repite() {
        let ahora = 1_791_360_000;
        let v = |id: &str, creado: u64, output: &str, rama: &str, quien: &str| {
            let mut p = un_build(id, creado);
            p.persona = quien.into();
            p.rama = Some(rama.into());
            if let Some(b) = p.trabajo.as_mut().and_then(|t| t.build.as_mut()) {
                b.output = output.into();
            }
            visto_de_puesto(id, &p, ahora).unwrap()
        };
        let vivos = vec![
            v("b1", 10, "ventas.limpios", "ana/etl", "persona:ana"),
            v("b3", 30, "ventas.otros", "ana/etl", "persona:ana"),
            v("b4", 40, "ventas.limpios", "ana/etl", "persona:bob"),
        ];
        // b1 también en un informe (más viejo): manda lo vivo.
        let mut informado = v("b1", 10, "ventas.limpios", "ana/etl", "persona:ana");
        informado.estado = "failed";
        let informados = vec![
            informado,
            v("b2", 20, "ventas.limpios", "main", "persona:ana"),
        ];
        let ids = |f: &Filtro| {
            historial(vivos.clone(), informados.clone(), "persona:ana", f)
                .iter()
                .map(|x| (x.build.clone(), x.estado))
                .collect::<Vec<_>>()
        };
        let todo = Filtro::de(&Default::default()).ok().unwrap();
        assert_eq!(todo.limit, 20);
        assert_eq!(
            ids(&todo),
            [
                ("b3".to_string(), "queued"),
                ("b2".to_string(), "queued"),
                ("b1".to_string(), "queued")
            ]
        );
        let consulta = |kv: &[(&str, &str)]| {
            Filtro::de(
                &kv.iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            )
            .ok()
            .unwrap()
        };
        let solo = |f: &Filtro| ids(f).into_iter().map(|x| x.0).collect::<Vec<_>>();
        assert_eq!(
            solo(&consulta(&[("output", "ventas.default.limpios")])),
            ["b2", "b1"]
        );
        assert_eq!(
            solo(&consulta(&[("output", "ventas.limpios"), ("rama", "main")])),
            ["b2"]
        );
        assert_eq!(
            solo(&consulta(&[("repositorio", "packages/ventas/etl/")])),
            ["b3", "b2", "b1"]
        );
        assert!(solo(&consulta(&[("repositorio", "packages/ventas/otro")])).is_empty());
        assert_eq!(solo(&consulta(&[("limit", "1")])), ["b3"]);
        let mal = |l: &str| {
            Filtro::de(&[("limit".to_string(), l.to_string())].into_iter().collect())
                .err()
                .map(|r| r.codigo)
        };
        assert_eq!(mal("0"), Some(422));
        assert_eq!(mal("x"), Some(422));
    }
}
