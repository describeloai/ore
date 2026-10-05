//! ⭐ ORE 0050 P3 · ACTUALIZAR UNA DECLARACIÓN SIN PISARLA.
//!
//! «Upgrade» (`POST /repositorios/{ruta}/actualizar`) propone en una rama los
//! ficheros de la semilla de hoy. Para el código está bien —lo que cambió sale
//! en el diff—, pero el fichero donde un repositorio declara sus paquetes es
//! SUYO: sustituirlo por el de la semilla borraba sus librerías, y el diff lo
//! enseñaba como si fuera parte de la actualización.
//!
//! ⇒ Aquí se FUSIONA: se **añade** lo que la semilla declara y el repositorio
//!   no tiene —por nombre—, y **no se toca** lo que ya está, con su versión,
//!   sus comentarios y su orden. Una edición de texto, no un volcado: el diff
//!   de la propuesta es exactamente lo añadido.
//!
//! | fichero | lo que se mira | lo que se añade |
//! |---|---|---|
//! | `pyproject.toml` | `[project].dependencies` y `[dependency-groups].dev` | cada requisito cuyo nombre (PEP 503) no está en ninguna de las dos |
//! | `package.json` | `dependencies`, `devDependencies` (y `peer`/`optional`, para no duplicar) | cada paquete que no está en ninguna |
//! | `.gitignore` | sus líneas | los patrones que faltan, al final |
//!
//! ⛔ Lo que no se entiende no se toca: un `package.json` que no es un objeto,
//!   un `pyproject.toml` sin `[project]`. La propuesta lo dice, y el fichero se
//!   queda como estaba.

use crate::parse::Node;

/// Lo que sale de fusionar.
#[derive(Debug, PartialEq)]
pub enum Fusion {
    /// Ya tiene todo lo que la semilla declara: no se escribe.
    Igual,
    /// El texto nuevo y, para el mensaje, lo añadido (`ore==1.0.0 (dependencies)`).
    Nueva { texto: String, anadido: Vec<String> },
    /// No se entiende: se deja como está, con el motivo.
    NoSeEntiende(String),
}

/// Si `rel` es un fichero que se fusiona en vez de sustituirse.
pub fn es_declaracion(rel: &str) -> bool {
    matches!(
        rel.rsplit('/').next(),
        Some("pyproject.toml" | "package.json" | ".gitignore")
    )
}

/// Fusiona en `actual` lo que `semilla` declara (los dos, ficheros `rel`).
pub fn fusionar(rel: &str, actual: &str, semilla: &str) -> Fusion {
    match rel.rsplit('/').next() {
        Some("pyproject.toml") => pyproject(actual, semilla),
        Some("package.json") => package(actual, semilla),
        Some(".gitignore") => gitignore(actual, semilla),
        _ => Fusion::NoSeEntiende(format!("`{rel}` no es una declaración")),
    }
}

// ── pyproject.toml ─────────────────────────────────────────────────────────

const LISTAS_PY: [(&str, &str, &str); 2] = [
    ("[project]", "dependencies", "dependencies"),
    ("[dependency-groups]", "dev", "dev"),
];

/// PEP 503: `Google_Cloud.Storage` y `google-cloud-storage` son el mismo.
fn pep503(n: &str) -> String {
    let mut o = String::new();
    for c in n.trim().to_lowercase().chars() {
        let c = if matches!(c, '_' | '.') { '-' } else { c };
        if !(c == '-' && o.ends_with('-')) {
            o.push(c);
        }
    }
    o
}

/// El nombre de un requisito de PEP 508 (`polars[pyarrow]>=1` → `polars`).
fn nombre_de(req: &str) -> &str {
    let r = req.trim();
    let fin = r
        .find(|c: char| "<>=!~;[ (@".contains(c))
        .unwrap_or(r.len());
    &r[..fin]
}

/// Una lista de TOML: dónde abre y cierra, y sus cadenas con su sitio.
struct Lista {
    abre: usize,
    cierra: usize,
    /// (valor, desde la comilla de apertura, hasta pasada la de cierre)
    items: Vec<(String, usize, usize)>,
}

/// Las cabeceras de tabla, con dónde empieza y acaba su cuerpo.
fn tablas(t: &str) -> Vec<(String, usize, usize)> {
    let mut v: Vec<(String, usize, usize)> = Vec::new();
    let mut off = 0;
    for linea in t.split_inclusive('\n') {
        let l = sin_comentario(linea).trim();
        if l.starts_with('[') && l.ends_with(']') {
            if let Some(u) = v.last_mut() {
                u.2 = off;
            }
            v.push((l.to_string(), off + linea.len(), t.len()));
        }
        off += linea.len();
    }
    v
}

fn sin_comentario(l: &str) -> &str {
    let mut dentro: Option<char> = None;
    for (i, c) in l.char_indices() {
        match (dentro, c) {
            (None, '"' | '\'') => dentro = Some(c),
            (Some(q), _) if c == q => dentro = None,
            (None, '#') => return &l[..i],
            _ => {}
        }
    }
    l
}

/// La lista `clave = [ … ]` del cuerpo `[desde, hasta)`.
fn lista(t: &str, desde: usize, hasta: usize, clave: &str) -> Option<Lista> {
    let mut off = desde;
    for linea in t[desde..hasta].split_inclusive('\n') {
        let l = sin_comentario(linea);
        let sangria = l.len() - l.trim_start().len();
        let resto = l.trim_start();
        if let Some(r) = resto.strip_prefix(clave)
            && let Some(r2) = r.trim_start().strip_prefix('=')
            && r2.trim_start().starts_with('[')
        {
            let abre = off
                + sangria
                + clave.len()
                + (r.len() - r2.len())
                + (r2.len() - r2.trim_start().len());
            return escanear(t, abre);
        }
        off += linea.len();
    }
    None
}

/// Desde el `[` de `abre`: hasta su `]`, con las cadenas de primer nivel.
fn escanear(t: &str, abre: usize) -> Option<Lista> {
    let b = t.as_bytes();
    let mut items = Vec::new();
    let (mut i, mut nivel) = (abre + 1, 0usize);
    while i < b.len() {
        match b[i] {
            b'"' | b'\'' => {
                let q = b[i];
                let ini = i;
                i += 1;
                while i < b.len() && b[i] != q {
                    if b[i] == b'\\' && q == b'"' {
                        i += 1;
                    }
                    i += 1;
                }
                if nivel == 0 {
                    items.push((t[ini + 1..i.min(b.len())].to_string(), ini, i + 1));
                }
            }
            b'#' => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'[' | b'{' => nivel += 1,
            b'}' => nivel = nivel.saturating_sub(1),
            b']' if nivel == 0 => {
                return Some(Lista {
                    abre,
                    cierra: i,
                    items,
                });
            }
            b']' => nivel -= 1,
            _ => {}
        }
        i += 1;
    }
    None
}

/// `req` en la lista `clave` de `tabla`; la crea si no está.
fn anadir_toml(t: &str, tabla: &str, clave: &str, req: &str) -> Result<String, String> {
    let item = format!("\"{}\"", req.replace('\\', "\\\\").replace('"', "\\\""));
    let Some((_, desde, hasta)) = tablas(t).into_iter().find(|(n, _, _)| n == tabla) else {
        if tabla == "[project]" {
            return Err("no tiene `[project]`".into());
        }
        let mut s = t.to_string();
        if !s.is_empty() && !s.ends_with('\n') {
            s.push('\n');
        }
        s.push_str(&format!("\n{tabla}\n{clave} = [\n  {item},\n]\n"));
        return Ok(s);
    };
    let Some(l) = lista(t, desde, hasta, clave) else {
        // La clave, al final de su tabla (tras su última línea con algo).
        let cuerpo = &t[desde..hasta];
        let fin = desde + cuerpo.trim_end().len();
        let fin = t[fin..].find('\n').map_or(t.len(), |n| fin + n + 1);
        let mut s = t.to_string();
        let salto = if fin == t.len() && !t.ends_with('\n') {
            "\n"
        } else {
            ""
        };
        s.insert_str(fin, &format!("{salto}{clave} = [\n  {item},\n]\n"));
        return Ok(s);
    };
    let multilinea = t[l.abre..l.cierra].contains('\n');
    let mut s = t.to_string();
    match l.items.last() {
        None if multilinea => s.insert_str(l.abre + 1, &format!("\n  {item},")),
        None => s.replace_range(l.abre + 1..l.cierra, &item),
        Some(&(_, ini, fin)) => {
            let tras = &t[fin..l.cierra];
            let coma = tras.trim_start_matches([' ', '\t']).starts_with(',');
            if !multilinea {
                if coma {
                    let c = fin + tras.find(',').unwrap() + 1;
                    s.insert_str(c, &format!(" {item},"));
                } else {
                    s.insert_str(fin, &format!(", {item}"));
                }
            } else {
                let inicio_linea = t[..ini].rfind('\n').map_or(0, |n| n + 1);
                let sangria: String = t[inicio_linea..ini]
                    .chars()
                    .take_while(|c| *c == ' ' || *c == '\t')
                    .collect();
                match tras.find('\n') {
                    Some(n) => {
                        // Al final de la línea del último (tras su comentario),
                        // y con su coma si no la tenía.
                        s.insert_str(fin + n, &format!("\n{sangria}{item},"));
                        if !coma {
                            s.insert(fin, ',');
                        }
                    }
                    None => {
                        let extra = if coma { "" } else { "," };
                        s.insert_str(l.cierra, &format!("\n{sangria}{item}{extra}\n"));
                        if !coma {
                            s.insert(fin, ',');
                        }
                    }
                }
            }
        }
    }
    Ok(s)
}

fn pyproject(actual: &str, semilla: &str) -> Fusion {
    let presentes = |t: &str| -> Vec<String> {
        let mut v = Vec::new();
        for (tabla, clave, _) in LISTAS_PY {
            if let Some((_, d, h)) = tablas(t).into_iter().find(|(n, _, _)| n == tabla)
                && let Some(l) = lista(t, d, h, clave)
            {
                v.extend(l.items.iter().map(|(x, _, _)| pep503(nombre_de(x))));
            }
        }
        v
    };
    if !tablas(actual).iter().any(|(n, _, _)| n == "[project]") {
        return Fusion::NoSeEntiende("no tiene `[project]`: sus dependencias no se leen".into());
    }
    let mut texto = actual.to_string();
    let mut anadido = Vec::new();
    for (tabla, clave, dicho) in LISTAS_PY {
        let Some((_, d, h)) = tablas(semilla).into_iter().find(|(n, _, _)| n == tabla) else {
            continue;
        };
        let Some(l) = lista(semilla, d, h, clave) else {
            continue;
        };
        for (req, _, _) in l.items {
            if presentes(&texto).contains(&pep503(nombre_de(&req))) {
                continue;
            }
            match anadir_toml(&texto, tabla, clave, &req) {
                Ok(t) => texto = t,
                Err(e) => return Fusion::NoSeEntiende(e),
            }
            anadido.push(format!("{req} ({dicho})"));
        }
    }
    if anadido.is_empty() {
        Fusion::Igual
    } else {
        Fusion::Nueva { texto, anadido }
    }
}

// ── package.json ───────────────────────────────────────────────────────────

const SECCIONES_JS: [&str; 4] = [
    "dependencies",
    "devDependencies",
    "peerDependencies",
    "optionalDependencies",
];

/// El objeto valor de la clave `clave` del objeto que abre en `abre`:
/// `(abre, cierra)` de sus llaves. Sin `clave`, el propio objeto raíz.
fn objeto_json(t: &str, clave: Option<&str>) -> Option<(usize, usize)> {
    let b = t.as_bytes();
    let raiz = t.find('{')?;
    let (mut i, mut nivel) = (raiz, 0usize);
    let mut ultima: Option<String> = None;
    let mut valor_de: Option<usize> = None;
    let mut pila: Vec<usize> = Vec::new();
    while i < b.len() {
        match b[i] {
            b'"' => {
                let ini = i;
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                if nivel == 1 {
                    ultima = Some(t[ini + 1..i].to_string());
                }
            }
            b':' if nivel == 1 => {
                if clave.is_some() && ultima.as_deref() == clave {
                    valor_de = Some(i);
                }
            }
            b'{' | b'[' => {
                nivel += 1;
                pila.push(i);
            }
            b'}' | b']' => {
                let ab = pila.pop()?;
                nivel -= 1;
                if clave.is_none() && nivel == 0 {
                    return Some((ab, i));
                }
                if let Some(v) = valor_de
                    && nivel == 1
                    && ab > v
                    && b[ab] == b'{'
                    && t[v + 1..ab].trim().is_empty()
                {
                    return Some((ab, i));
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// La sangría de la línea donde está `pos`.
fn sangria_en(t: &str, pos: usize) -> String {
    let ini = t[..pos].rfind('\n').map_or(0, |n| n + 1);
    t[ini..]
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect()
}

/// `"nombre": "valor"` dentro del objeto `(abre, cierra)`.
fn anadir_en_objeto(t: &str, (abre, cierra): (usize, usize), entrada: &str) -> String {
    let dentro = &t[abre + 1..cierra];
    let mut s = t.to_string();
    let base = sangria_en(t, abre);
    if dentro.trim().is_empty() {
        // La entrada un nivel más dentro que su clave; la llave, a la altura de ésta.
        s.replace_range(abre + 1..cierra, &format!("\n{base}  {entrada}\n{base}"));
        return s;
    }
    let ultimo = abre + 1 + dentro.trim_end().len();
    if dentro.contains('\n') {
        let primera = abre + 1 + (dentro.len() - dentro.trim_start().len());
        let sangria = sangria_en(t, primera);
        s.insert_str(ultimo, &format!(",\n{sangria}{entrada}"));
    } else {
        s.insert_str(ultimo, &format!(", {entrada}"));
    }
    s
}

fn package(actual: &str, semilla: &str) -> Fusion {
    let Ok(n) = crate::parse::parse(actual) else {
        return Fusion::NoSeEntiende("no es JSON".into());
    };
    if !matches!(n, Node::Mapping { .. }) {
        return Fusion::NoSeEntiende("no es un objeto JSON".into());
    }
    let Ok(sem) = crate::parse::parse(semilla) else {
        return Fusion::NoSeEntiende("la semilla no es JSON".into());
    };
    let nombres = |n: &Node| -> Vec<String> {
        SECCIONES_JS
            .iter()
            .filter_map(|s| n.get(s).map(|(_, v)| v))
            .flat_map(|v| {
                v.entries()
                    .iter()
                    .filter_map(|(k, _)| k.as_str().map(String::from))
            })
            .collect()
    };
    let tiene = nombres(&n);
    let mut texto = actual.to_string();
    let mut anadido = Vec::new();
    for seccion in ["dependencies", "devDependencies"] {
        let Some((_, deps)) = sem.get(seccion) else {
            continue;
        };
        for (k, v) in deps.entries() {
            let (Some(k), Some(v)) = (k.as_str(), v.as_str()) else {
                continue;
            };
            if tiene.iter().any(|x| x == k) {
                continue;
            }
            let entrada = format!("\"{k}\": \"{v}\"");
            texto = match objeto_json(&texto, Some(seccion)) {
                Some(o) => anadir_en_objeto(&texto, o, &entrada),
                None => {
                    let Some(raiz) = objeto_json(&texto, None) else {
                        return Fusion::NoSeEntiende("no es un objeto JSON".into());
                    };
                    anadir_en_objeto(
                        &texto,
                        raiz,
                        &format!("\"{seccion}\": {{\n    {entrada}\n  }}"),
                    )
                }
            };
            anadido.push(format!("{k}@{v} ({seccion})"));
        }
    }
    if anadido.is_empty() {
        Fusion::Igual
    } else {
        Fusion::Nueva { texto, anadido }
    }
}

// ── .gitignore ─────────────────────────────────────────────────────────────

fn gitignore(actual: &str, semilla: &str) -> Fusion {
    let hay: Vec<&str> = actual.lines().map(str::trim).collect();
    let faltan: Vec<&str> = semilla
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#') && !hay.contains(l))
        .collect();
    if faltan.is_empty() {
        return Fusion::Igual;
    }
    let mut texto = actual.to_string();
    if !texto.is_empty() && !texto.ends_with('\n') {
        texto.push('\n');
    }
    if !texto.is_empty() {
        texto.push('\n');
    }
    texto.push_str("# From the repository template.\n");
    for l in &faltan {
        texto.push_str(l);
        texto.push('\n');
    }
    Fusion::Nueva {
        texto,
        anadido: faltan.iter().map(|l| format!("{l} (.gitignore)")).collect(),
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    const SEMILLA_PY: &str = "# c\n[project]\nname = \"x\"\ndependencies = [\n  \"ore==1.0.0\",\n]\n\n[dependency-groups]\ndev = [\n  \"pytest==9.1.1\",\n]\n";

    fn nueva(f: Fusion) -> (String, Vec<String>) {
        match f {
            Fusion::Nueva { texto, anadido } => (texto, anadido),
            otra => panic!("{otra:?}"),
        }
    }

    /// El de la semilla v10, con su comentario: se añade y lo demás se queda.
    #[test]
    fn el_pyproject_de_antes_recibe_el_sdk_y_pytest() {
        let viejo = "# Nace vacío.\n\n[project]\nname = \"repositorio\"\nversion = \"0.1.0\"\ndependencies = []\n# dependencies = [\"polars\"]\n";
        let (t, a) = nueva(fusionar("pyproject.toml", viejo, SEMILLA_PY));
        assert_eq!(
            t,
            "# Nace vacío.\n\n[project]\nname = \"repositorio\"\nversion = \"0.1.0\"\ndependencies = [\"ore==1.0.0\"]\n# dependencies = [\"polars\"]\n\n[dependency-groups]\ndev = [\n  \"pytest==9.1.1\",\n]\n"
        );
        assert_eq!(a, ["ore==1.0.0 (dependencies)", "pytest==9.1.1 (dev)"]);
        // Y otra vez: ya está todo.
        assert_eq!(fusionar("pyproject.toml", &t, SEMILLA_PY), Fusion::Igual);
    }

    /// El de pyfunctionsv1 (P2·5): polars y hypothesis se quedan como estaban.
    #[test]
    fn lo_declarado_se_queda_con_su_version() {
        let suyo = "[project]\nname = \"repository\"\nversion = \"0.1.0\"\ndependencies = [\"polars>=1.30\"]\n\n[dependency-groups]\ndev = [\"hypothesis\", \"Pytest>=8\"]\n";
        let (t, a) = nueva(fusionar("pyproject.toml", suyo, SEMILLA_PY));
        assert_eq!(
            t,
            "[project]\nname = \"repository\"\nversion = \"0.1.0\"\ndependencies = [\"polars>=1.30\", \"ore==1.0.0\"]\n\n[dependency-groups]\ndev = [\"hypothesis\", \"Pytest>=8\"]\n"
        );
        // pytest ya estaba (otro rango, otra mayúscula): no se toca.
        assert_eq!(a, ["ore==1.0.0 (dependencies)"]);
    }

    /// En varias líneas: con su sangría, su coma y tras el comentario.
    #[test]
    fn en_varias_lineas_con_comentarios() {
        let suyo = "[project]\nname = \"r\"\ndependencies = [\n    \"polars[pyarrow]>=1.30\"  # rápido\n]\n[tool.x]\ndev = [\"no\"]\n";
        let (t, _) = nueva(fusionar("pyproject.toml", suyo, SEMILLA_PY));
        assert_eq!(
            t,
            "[project]\nname = \"r\"\ndependencies = [\n    \"polars[pyarrow]>=1.30\",  # rápido\n    \"ore==1.0.0\",\n]\n[tool.x]\ndev = [\"no\"]\n\n[dependency-groups]\ndev = [\n  \"pytest==9.1.1\",\n]\n"
        );
        // El grupo existe sin `dev`: la clave, al final de su tabla.
        let g = "[project]\ndependencies = [\"ore==1.0.0\"]\n\n[dependency-groups]\nlint = [\"ruff\"]\n\n[tool.y]\nz = 1\n";
        let (t, _) = nueva(fusionar("pyproject.toml", g, SEMILLA_PY));
        assert_eq!(
            t,
            "[project]\ndependencies = [\"ore==1.0.0\"]\n\n[dependency-groups]\nlint = [\"ruff\"]\ndev = [\n  \"pytest==9.1.1\",\n]\n\n[tool.y]\nz = 1\n"
        );
    }

    /// Sin `[project]`, o sin `dependencies` en él.
    #[test]
    fn lo_que_no_se_entiende_no_se_toca() {
        let fuera = "dependencies = [\"polars\"]\n[dependency-groups]\ndev = [\"hypothesis\"]\n";
        assert!(matches!(
            fusionar("pyproject.toml", fuera, SEMILLA_PY),
            Fusion::NoSeEntiende(_)
        ));
        let sin = "[project]\nname = \"r\"\n\n[tool.x]\na = 1\n";
        let (t, _) = nueva(fusionar("pyproject.toml", sin, SEMILLA_PY));
        assert!(
            t.starts_with(
                "[project]\nname = \"r\"\ndependencies = [\n  \"ore==1.0.0\",\n]\n\n[tool.x]"
            ),
            "{t}"
        );
        assert!(matches!(
            fusionar("package.json", "no es json {", "{}"),
            Fusion::NoSeEntiende(_)
        ));
    }

    const SEMILLA_JS: &str = "{\n  \"dependencies\": {\n    \"ore\": \"1.0.0\"\n  },\n  \"devDependencies\": {\n    \"@types/node\": \"24.19.1\",\n    \"typescript\": \"5.9.3\"\n  }\n}\n";

    /// La deuda de L6: un package.json con lo suyo recibe lo de la semilla.
    #[test]
    fn el_package_json_conserva_lo_suyo() {
        let suyo = "{\n  \"private\": true,\n  \"dependencies\": {\n    \"apache-arrow\": \"^21.0.0\",\n    \"ore\": \"1.0.0\"\n  },\n  \"devDependencies\": {\n    \"typescript\": \"5.4.5\"\n  }\n}\n";
        let (t, a) = nueva(fusionar("package.json", suyo, SEMILLA_JS));
        assert_eq!(
            t,
            "{\n  \"private\": true,\n  \"dependencies\": {\n    \"apache-arrow\": \"^21.0.0\",\n    \"ore\": \"1.0.0\"\n  },\n  \"devDependencies\": {\n    \"typescript\": \"5.4.5\",\n    \"@types/node\": \"24.19.1\"\n  }\n}\n"
        );
        assert_eq!(a, ["@types/node@24.19.1 (devDependencies)"]);
        // Sin la sección, se crea; y otra vez, nada.
        let (t, _) = nueva(fusionar(
            "package.json",
            "{\n  \"private\": true\n}\n",
            SEMILLA_JS,
        ));
        assert!(crate::parse::parse(&t).is_ok(), "{t}");
        assert!(
            t.contains("\"ore\": \"1.0.0\"") && t.contains("\"typescript\": \"5.9.3\""),
            "{t}"
        );
        assert_eq!(fusionar("package.json", &t, SEMILLA_JS), Fusion::Igual);
    }

    #[test]
    fn un_objeto_vacio_se_rellena_con_su_sangria() {
        let (t, _) = nueva(fusionar(
            "package.json",
            "{\n  \"dependencies\": {},\n  \"devDependencies\": {\"typescript\": \"5.9.3\", \"@types/node\": \"24.19.1\"}\n}\n",
            SEMILLA_JS,
        ));
        assert_eq!(
            t,
            "{\n  \"dependencies\": {\n    \"ore\": \"1.0.0\"\n  },\n  \"devDependencies\": {\"typescript\": \"5.9.3\", \"@types/node\": \"24.19.1\"}\n}\n"
        );
    }

    #[test]
    fn el_gitignore_suma_lo_que_falta() {
        let (t, a) = nueva(fusionar(
            ".gitignore",
            "mio/\n__pycache__/\n",
            "# x\n__pycache__/\n.venv/\n",
        ));
        assert_eq!(
            t,
            "mio/\n__pycache__/\n\n# From the repository template.\n.venv/\n"
        );
        assert_eq!(a, [".venv/ (.gitignore)"]);
        assert_eq!(
            fusionar(".gitignore", &t, "__pycache__/\n.venv/\n"),
            Fusion::Igual
        );
        assert!(es_declaracion("packages/p/r/pyproject.toml") && !es_declaracion("functions/a.py"));
    }
}
