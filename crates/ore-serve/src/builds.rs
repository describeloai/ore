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
        return Err(Respuesta::error(
            404,
            match que {
                Que::Output(o) => format!(
                    "no `Transform` in this branch writes `{o}`: commit the code that writes it (its document is derived at commit), or say which `rama` it is in"
                ),
                Que::Fichero(f) => format!(
                    "`{f}` has no `Transform` in this branch: no `@transform` def or writing statement of it has a committed document"
                ),
            },
        ));
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
        .map(|d| {
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
        })
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
            return Err(Respuesta::error(
                422,
                format!("`{codigo}` is not in the tree (OOS2042)"),
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
                    Respuesta::error(422, format!("`{codigo}` does not parse (OOS2043)"))
                })?;
                let Some(t) = n.checked_sub(1).and_then(|i| trozos.get(i)) else {
                    return Err(Respuesta::error(
                        422,
                        format!("`{codigo}` has no statement {n} (OOS2042)"),
                    ));
                };
                let (celda, _) = crate::puestos::celda_de_sentencia(&codigo, t, &pkg);
                format!("{}{celda}", preludio_sql(&build))
            }
            otro => {
                return Err(Respuesta::error(
                    422,
                    format!(
                        "`{documento}` is `runtime: {otro}`: only python and sql transforms build yet"
                    ),
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
        r#"# El arnés de un build (ORE 0055 B1): {documento}
import json as _json
import os as _os
import traceback as _tb

{guarda}_FICHERO = {fichero}
_DEF = {def_}
_COMMIT = (_os.environ.get("ORE_CODIGO") or "").rpartition("@")[2]
_os.environ["ORE_BUILD"] = _json.dumps(dict(_json.loads({build}), id=_ore.session.id, commit=_COMMIT))


def _donde(e):
    """Dónde se rompió, en el fichero del transform."""
    for fr in reversed(_tb.extract_tb(e.__traceback__)):
        if fr.filename == _FICHERO:
            return " (%s, line %d)" % (_FICHERO, fr.lineno)
    return ""


if not hasattr(_ore, "_modulo_en_carga"):
    raise RuntimeError("this job runs an older ORE SDK, which cannot build: rebuild the image")
try:
    _codigo = compile({fuente}, _FICHERO, "exec")
except SyntaxError as e:
    raise RuntimeError("SyntaxError: %s (%s, line %s)" % (e.msg, _FICHERO, e.lineno)) from None
_modulo = {{"__name__": "ore_build", "__file__": _FICHERO}}
# D15: mientras el módulo carga, un `@transform` llamado no corre: falla.
_ore._modulo_en_carga(_FICHERO)
try:
    exec(_codigo, _modulo)
except _ore.TransformCalledWhileLoading as e:
    raise RuntimeError("%s: %s" % (_FICHERO, e)) from None
except Exception as e:  # noqa: BLE001
    raise RuntimeError("%s while loading %s: %s%s" % (type(e).__name__, _FICHERO, e, _donde(e))) from None
finally:
    _ore._modulo_en_carga(None)
_f = _modulo.get(_DEF)
if not callable(_f) or getattr(_f, "output", None) is None:
    raise RuntimeError("`%s` is not a `@transform` def of %s at this commit" % (_DEF, _FICHERO))
# El build lo llama él: una vez, y con lo que el documento deja.
try:
    _hecho = _f()
except Exception as e:  # noqa: BLE001
    raise RuntimeError("%s: %s%s" % (type(e).__name__, e, _donde(e))) from None
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
        return Err(Respuesta::error(
            409,
            "`main` is protected: a build writes, so it runs in a branch. Build from your branch and propose",
        ));
    }
    Ok(())
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
        if crate::puestos::es_agente(sujeto) {
            return Respuesta::error(403, "an agent does not launch builds: a person does");
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
                return Respuesta::error(
                    422,
                    format!(
                        "`{}` is a `{}` repository, and that class does not write data: its transforms do not build",
                        repositorio.unwrap_or_default(),
                        c.id
                    ),
                );
            }
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
            if let Json::Obj(m) = &mut r.cuerpo {
                let id = match m.get("id") {
                    Some(Json::Str(s)) => s.clone(),
                    _ => String::new(),
                };
                m.insert("build".into(), Json::s(id));
                m.insert("output".into(), Json::s(&h.output));
                m.insert("transform".into(), Json::s(&h.documento));
                m.insert("entrypoint".into(), Json::s(&h.entrypoint));
                m.insert("commit".into(), Json::s(&commit));
                m.insert("estado".into(), Json::s("queued"));
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
        // La línea del fichero del usuario en los errores.
        assert!(
            t.contains(r#"" (%s, line %d)" % (_FICHERO, fr.lineno)"#),
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
}
