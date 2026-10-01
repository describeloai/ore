//! v1alpha18 — **promover**: el código de un repositorio pasa a ser una
//! función cuando un documento lo nombra (ORE 0050, sobre 0031 W3.8).
//!
//! El documento es el contrato y el `def` lo cumple. Aquí se comprueba lo que
//! se puede comprobar **leyendo**, sin ejecutar nada:
//!
//! - la forma: `runtime: python` es de v1alpha18, lleva `entrypoint`
//!   `<ruta>.py:<def>` dentro del paquete, y `models` es solo suyo (`OOS1004`);
//! - que el fichero esté y defina el `def` en su nivel superior (`OOS2042`);
//! - que la cabecera del `def` sea la firma: la fila si hay `over`, y
//!   exactamente las claves de `input`, con lo obligatorio sin valor por
//!   defecto y lo opcional con él (`OOS2043`);
//! - que cada modelo de `models` resuelva (`OOS2005`, como `model`).
//!
//! # Por qué un lector de cabeceras y no un analizador de Python
//!
//! La firma es **sintaxis**: los nombres de los parámetros, si tienen valor
//! por defecto y si hay `*args`/`**kwargs`. Para eso basta con encontrar el
//! `def` en la columna cero y leer su lista de parámetros respetando
//! paréntesis, corchetes, llaves, cadenas y comentarios. Las anotaciones y el
//! cuerpo no son firma (v1alpha18 01 §4.1): los tipos son los del documento.

use crate::code::Code;
use crate::diag::Diagnostic;
use crate::document::{ApiVersion, Kind};
use crate::link::{Loaded, Package};
use crate::parse::Node;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub fn comprobar(pkg: &Package, out: &mut Vec<Diagnostic>) {
    for f in pkg.of(Kind::Function) {
        let version = f.version();
        let runtime = f.section("runtime").and_then(|n| n.as_str()).unwrap_or("");
        let de_18 = version.is_some_and(|v| v >= ApiVersion::V1Alpha18);

        // ── OOS1004 · el runtime de la versión ───────────────────────────
        if runtime == "python" && !de_18 {
            out.push(forma(
                f,
                f.section("runtime"),
                "`runtime: python` es de v1alpha18".to_string(),
                "una función de código se declara con `apiVersion: oos.dev/v1alpha18`; hasta \
                 v1alpha17 el runtime era `wasm` o `model`",
            ));
            continue;
        }
        if de_18 && !matches!(runtime, "wasm" | "model" | "python") {
            out.push(forma(
                f,
                f.section("runtime"),
                format!("`runtime: {runtime}` no es un runtime de OOS"),
                "`wasm` (un módulo sin red), `model` (un modelo del árbol) o `python` (un `def` \
                 del paquete)",
            ));
            continue;
        }

        // ── OOS1004 · `models`, solo de una función de código ────────────
        if let Some(m) = f.section("models")
            && runtime != "python"
        {
            out.push(forma(
                f,
                Some(m),
                format!("`models` con `runtime: {runtime}`"),
                "`models` dice qué modelos puede llamar el código de una función `python`. Un \
                 `wasm` no tiene red, y con `runtime: model` el modelo es lo que se ejecuta: \
                 `model`",
            ));
        }
        modelos_usados(pkg, f, out);

        if runtime != "python" {
            continue;
        }

        // ── OOS1004 · el `entrypoint` ────────────────────────────────────
        let Some(nodo) = f.section("entrypoint") else {
            out.push(forma(
                f,
                None,
                format!(
                    "`{}` es `runtime: python` y no dice `entrypoint`",
                    f.qname().unwrap_or_default()
                ),
                "el `entrypoint` es lo que el documento promueve: `<ruta>.py:<def>`, relativo a \
                 la carpeta del paquete",
            ));
            continue;
        };
        let texto = nodo.as_str().unwrap_or("");
        let Some((ruta, nombre)) = entrypoint(texto) else {
            out.push(forma(
                f,
                Some(nodo),
                format!("`entrypoint: {texto}` no es `<ruta>.py:<def>` dentro del paquete"),
                "la ruta es relativa a la carpeta del paquete, con `/`, sin `..` ni `/` inicial, \
                 y termina en `.py`; detrás de `:`, el nombre del `def`. Una función no nombra \
                 código de otro paquete",
            ));
            continue;
        };

        // ── OOS2042 · el fichero y el `def` están ────────────────────────
        let carpeta = carpeta_del_paquete(&f.path, &pkg.root);
        let fichero = carpeta.join(ruta);
        let Ok(codigo) = std::fs::read_to_string(&fichero) else {
            out.push(
                Diagnostic::new(
                    Code::Oos2042,
                    &f.path,
                    format!("`entrypoint: {texto}`: `{ruta}` no está en el paquete"),
                )
                .at(nodo.pos())
                .help(
                    "el documento promueve código que no hay. Escribe el fichero, o corrige la \
                     ruta: es relativa a la carpeta del paquete (la de su `package.yaml`)",
                ),
            );
            continue;
        };
        let cabecera = match cabecera_del_def(&codigo, nombre) {
            Ok(c) => c,
            Err(falta) => {
                let (msg, ayuda) = match falta {
                    Falta::NoEsta => (
                        format!("`{ruta}` no define `def {nombre}(…)` en su nivel superior"),
                        "el `entrypoint` nombra una función del módulo: no un método de una \
                         clase, ni una función dentro de otra",
                    ),
                    Falta::Asincrona => (
                        format!("`{nombre}` en `{ruta}` es un `async def`"),
                        "el runtime llama al `def` y espera su valor; una corrutina es otra \
                         cosa. Escríbelo como `def`",
                    ),
                };
                out.push(
                    Diagnostic::new(Code::Oos2042, &f.path, msg)
                        .at(nodo.pos())
                        .help(ayuda),
                );
                continue;
            }
        };

        // ── OOS2043 · la cabecera es la firma ────────────────────────────
        if let Some(m) = firma(f, &cabecera) {
            out.push(
                Diagnostic::new(Code::Oos2043, &f.path, format!("`{ruta}:{nombre}`: {m}"))
                    .at(nodo.pos())
                    .help(
                        "con `over`, el primer parámetro es la fila; los demás, exactamente \
                         las claves de `input`. Lo obligatorio sin valor por defecto, lo \
                         opcional con él, y sin `*args` ni `**kwargs`: un consumidor solo \
                         conoce el documento",
                    ),
            );
        }
    }
}

fn forma(f: &Loaded, nodo: Option<&Node>, msg: String, ayuda: &str) -> Diagnostic {
    Diagnostic::new(Code::Oos1004, &f.path, msg)
        .at(nodo.map(Node::pos).unwrap_or_else(|| f.root.pos()))
        .help(ayuda.to_string())
}

// ── OOS2005 · cada modelo de `models` resuelve ──────────────────────────────

fn modelos_usados(pkg: &Package, f: &Loaded, out: &mut Vec<Diagnostic>) {
    for nodo in f.section("models").map(|n| n.items()).unwrap_or(&[]) {
        let Some(referencia) = nodo.as_str() else {
            continue;
        };
        if pkg.resolve_model(referencia, f).is_none() {
            out.push(
                crate::link::referencia_rota(&f.path, nodo, referencia, "models").help(format!(
                    "`{referencia}` no resuelve a ningún `kind: Model` (v1alpha15: una parte es \
                     el paquete y el schema de la función; dos, `<paquete>.<nombre>` en \
                     `default`; tres, completo). El código solo puede llamar a lo que el \
                     árbol tiene"
                )),
            );
        }
    }
}

// ── el `entrypoint` ─────────────────────────────────────────────────────────

/// `<ruta>.py:<def>` → `(ruta, def)`, o `None` si la forma no vale: ruta
/// relativa con `/`, sin `..` ni `/` inicial, que termina en `.py`; el `def`,
/// un identificador.
pub fn entrypoint(s: &str) -> Option<(&str, &str)> {
    let (ruta, nombre) = s.rsplit_once(':')?;
    let ident = |x: &str| {
        let mut c = x.chars();
        c.next()
            .is_some_and(|p| p.is_ascii_alphabetic() || p == '_')
            && c.all(|p| p.is_ascii_alphanumeric() || p == '_')
    };
    let ruta_ok = !ruta.is_empty()
        && !ruta.starts_with('/')
        && !ruta.contains('\\')
        && !ruta.contains(':')
        && ruta.ends_with(".py")
        && ruta.split('/').all(|t| !t.is_empty() && t != "..");
    (ruta_ok && ident(nombre)).then_some((ruta, nombre))
}

/// La carpeta del paquete donde vive el documento: la más cercana, subiendo,
/// que tiene `package.yaml`. Sin ninguna dentro del árbol, la raíz (un
/// paquete suelto).
fn carpeta_del_paquete(doc: &Path, raiz: &Path) -> PathBuf {
    let mut d = doc.parent();
    while let Some(c) = d {
        if c.join("package.yaml").is_file() {
            return c.to_path_buf();
        }
        if c == raiz {
            break;
        }
        d = c.parent();
    }
    raiz.to_path_buf()
}

// ── la cabecera del `def` ───────────────────────────────────────────────────

#[derive(Debug, PartialEq)]
pub struct Parametro {
    pub nombre: String,
    pub por_defecto: bool,
    /// Detrás de `*` o de `*args`: solo por nombre.
    pub solo_nombre: bool,
}

#[derive(Debug, PartialEq, Default)]
pub struct Cabecera {
    pub parametros: Vec<Parametro>,
    pub args: bool,
    pub kwargs: bool,
}

#[derive(Debug, PartialEq)]
pub enum Falta {
    NoEsta,
    Asincrona,
}

/// La lista de parámetros del `def <nombre>` del nivel superior de `codigo`.
pub fn cabecera_del_def(codigo: &str, nombre: &str) -> Result<Cabecera, Falta> {
    let mut asincrona = false;
    let mut inicio = None;
    let mut desplazamiento = 0usize;
    for linea in codigo.split_inclusive('\n') {
        for (prefijo, es_async) in [("def ", false), ("async def ", true)] {
            if let Some(resto) = linea.strip_prefix(prefijo)
                && let Some(tras) = resto.strip_prefix(nombre)
                && tras.trim_start().starts_with('(')
            {
                if es_async {
                    asincrona = true;
                } else if inicio.is_none() {
                    let parentesis = linea.len() - tras.trim_start().len();
                    inicio = Some(desplazamiento + parentesis + 1);
                }
            }
        }
        desplazamiento += linea.len();
    }
    let Some(i) = inicio else {
        return Err(if asincrona {
            Falta::Asincrona
        } else {
            Falta::NoEsta
        });
    };
    Ok(leer_parametros(&codigo[i..]))
}

/// Desde justo después de `(` hasta el `)` que la cierra: los trozos de nivel
/// superior separados por comas, sin cadenas ni comentarios que confundan.
fn leer_parametros(s: &str) -> Cabecera {
    let mut trozos: Vec<String> = vec![String::new()];
    let mut nivel = 0i32;
    let mut cadena: Option<char> = None;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if let Some(q) = cadena {
            trozos.last_mut().unwrap().push(c);
            if c == '\\' {
                if let Some(n) = chars.next() {
                    trozos.last_mut().unwrap().push(n);
                }
            } else if c == q {
                cadena = None;
            }
            continue;
        }
        match c {
            '#' => {
                for n in chars.by_ref() {
                    if n == '\n' {
                        break;
                    }
                }
            }
            '\'' | '"' => {
                cadena = Some(c);
                trozos.last_mut().unwrap().push(c);
            }
            '(' | '[' | '{' => {
                nivel += 1;
                trozos.last_mut().unwrap().push(c);
            }
            ')' | ']' | '}' => {
                if nivel == 0 {
                    break;
                }
                nivel -= 1;
                trozos.last_mut().unwrap().push(c);
            }
            ',' if nivel == 0 => trozos.push(String::new()),
            _ => trozos.last_mut().unwrap().push(c),
        }
    }

    let mut cab = Cabecera::default();
    let mut solo_nombre = false;
    for t in trozos.iter().map(|t| t.trim()).filter(|t| !t.is_empty()) {
        if t == "/" {
            continue;
        }
        if t == "*" {
            solo_nombre = true;
            continue;
        }
        if t.starts_with("**") {
            cab.kwargs = true;
            continue;
        }
        if t.starts_with('*') {
            cab.args = true;
            solo_nombre = true;
            continue;
        }
        let nombre: String = t
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        cab.parametros.push(Parametro {
            nombre,
            por_defecto: igual_de_nivel_superior(t),
            solo_nombre,
        });
    }
    cab
}

/// Si el trozo de un parámetro lleva `=` fuera de corchetes y cadenas: su
/// valor por defecto (`x: dict[str, int] = {}`).
fn igual_de_nivel_superior(t: &str) -> bool {
    let mut nivel = 0i32;
    let mut cadena: Option<char> = None;
    for c in t.chars() {
        match (cadena, c) {
            (Some(q), _) if c == q => cadena = None,
            (Some(_), _) => {}
            (None, '\'' | '"') => cadena = Some(c),
            (None, '(' | '[' | '{') => nivel += 1,
            (None, ')' | ']' | '}') => nivel -= 1,
            (None, '=') if nivel == 0 => return true,
            _ => {}
        }
    }
    false
}

// ── OOS2043 · la cabecera contra el documento ───────────────────────────────

/// Lo que no casa, dicho en una frase; `None` si la cabecera es la firma.
fn firma(f: &Loaded, cab: &Cabecera) -> Option<String> {
    if cab.args || cab.kwargs {
        return Some("la superficie no es cerrada: `*args` o `**kwargs`".into());
    }
    let mut parametros: &[Parametro] = &cab.parametros;
    if f.section("over").is_some() {
        let Some((fila, resto)) = parametros.split_first() else {
            return Some("con `over`, el primer parámetro es la fila, y no hay ninguno".into());
        };
        if fila.por_defecto || fila.solo_nombre {
            return Some(format!(
                "con `over`, el primer parámetro es la fila, posicional y sin valor por \
                 defecto; `{}` no lo es",
                fila.nombre
            ));
        }
        parametros = resto;
    }

    let input: Vec<(String, bool)> = f
        .section("input")
        .map(|n| n.entries())
        .unwrap_or(&[])
        .iter()
        .filter_map(|(k, v)| {
            let obligatorio = v
                .get("required")
                .and_then(|(_, r)| r.as_str())
                .is_some_and(|r| r == "true");
            k.as_str().map(|k| (k.to_string(), obligatorio))
        })
        .collect();
    let declarados: BTreeSet<&str> = input.iter().map(|(k, _)| k.as_str()).collect();
    let del_def: BTreeSet<&str> = parametros.iter().map(|p| p.nombre.as_str()).collect();

    if let Some(sobra) = del_def.difference(&declarados).next() {
        let que = if f.section("over").is_none() && declarados.is_empty() && del_def.len() == 1 {
            " (sin `over` no hay fila)"
        } else {
            ""
        };
        return Some(format!(
            "el `def` pide `{sobra}`, que `input` no declara{que}"
        ));
    }
    if let Some(falta) = declarados.difference(&del_def).next() {
        return Some(format!("`input` declara `{falta}` y el `def` no lo recibe"));
    }
    for (k, obligatorio) in &input {
        let p = parametros.iter().find(|p| &p.nombre == k)?;
        if *obligatorio && p.por_defecto {
            return Some(format!(
                "`{k}` es obligatorio en el documento y el `def` le da un valor por defecto"
            ));
        }
        if !*obligatorio && !p.por_defecto {
            return Some(format!(
                "`{k}` es opcional en el documento (`required` ausente es `false`) y el `def` \
                 no le da valor por defecto"
            ));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_entrypoint_es_una_ruta_del_paquete_y_un_def() {
        assert_eq!(
            entrypoint("funciones/r.py:riesgo"),
            Some(("funciones/r.py", "riesgo"))
        );
        assert_eq!(entrypoint("a/b/c.py:_f1"), Some(("a/b/c.py", "_f1")));
        for malo in [
            "/a.py:f",
            "../a.py:f",
            "a/../b.py:f",
            "a.py",
            "a.ts:f",
            "a.py:1f",
            "a.py:",
            "a//b.py:f",
        ] {
            assert_eq!(entrypoint(malo), None, "{malo}");
        }
    }

    #[test]
    fn la_cabecera_se_lee_sin_ejecutar() {
        let c = cabecera_del_def(
            "import x\n\n@dec\ndef f(\n    fila,  # la fila\n    umbral: dict[str, int] = {'a': 1},\n    *, moneda: str = \"(,)\",\n) -> dict:\n    pass\n",
            "f",
        )
        .unwrap();
        let n: Vec<_> = c
            .parametros
            .iter()
            .map(|p| (p.nombre.as_str(), p.por_defecto, p.solo_nombre))
            .collect();
        assert_eq!(
            n,
            vec![
                ("fila", false, false),
                ("umbral", true, false),
                ("moneda", true, true)
            ]
        );
        assert!(!c.args && !c.kwargs);
    }

    #[test]
    fn lo_que_no_esta_en_el_nivel_superior_no_cuenta() {
        assert_eq!(
            cabecera_del_def("class A:\n    def f(self):\n        pass\n", "f"),
            Err(Falta::NoEsta)
        );
        assert_eq!(
            cabecera_del_def("async def f(x):\n    pass\n", "f"),
            Err(Falta::Asincrona)
        );
        assert_eq!(
            cabecera_del_def("def fx(a):\n    pass\n", "f"),
            Err(Falta::NoEsta)
        );
        let c = cabecera_del_def("def f(a, *rest, **kw):\n    pass\n", "f").unwrap();
        assert!(c.args && c.kwargs);
    }
}
