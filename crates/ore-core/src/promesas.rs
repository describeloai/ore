//! ⭐ **LAS PROMESAS DE UN ACTIVO** (OOS v1alpha31 `01-las-promesas`, ADR 0063 S7).
//!
//! Lo que el dueño de un `Dataset` o de una `MediaCollection` promete de él, en
//! su `spec.promises`, comprobado en cada refresco. Aquí vive **el catálogo**
//! —la biblioteca por familia: qué métricas hay, cuáles miden una columna, qué
//! argumentos y qué unidades admiten— y **lo que el compilador dice** de una
//! promesa mal escrita:
//!
//! | Código | Qué |
//! |---|---|
//! | `OOS1004` | la forma: métrica que no existe o de la otra familia, columna que falta o sobra, argumentos que la métrica no admite, cero o dos comparadores, un rango al revés, `notEmptied` con algo más que su severidad |
//! | `OOS2052` | la columna no es del activo: ni de su `fields` (una copia) ni de su `columns` (uno escrito) |
//! | `OOS2053` | dos promesas con el mismo `id` |
//!
//! Que `promises` no exista antes de v1alpha31, ni en lo que no se refresca
//! (`View`, `Table`, `ObjectTable`), es `OOS1005` por las claves de cada versión
//! (`document.rs`): aquí no hace falta decirlo otra vez.
//!
//! Medir y juzgar es de S7b en adelante: este catálogo es el que el juez lee.

use crate::code::Code;
use crate::diag::Diagnostic;
use crate::document::{ApiVersion, Kind};
use crate::json::Json;
use crate::parse::{Node, Style};
use std::collections::BTreeSet;
use std::path::Path;

/// La familia de un activo: de qué biblioteca son sus métricas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Familia {
    /// Un `Dataset`: filas y columnas.
    Tabular,
    /// Una `MediaCollection`: ítems.
    Objetos,
}

impl Familia {
    pub fn de(kind: Kind) -> Option<Familia> {
        match kind {
            Kind::Dataset => Some(Familia::Tabular),
            Kind::MediaCollection => Some(Familia::Objetos),
            _ => None,
        }
    }

    fn nombre(self) -> &'static str {
        match self {
            Familia::Tabular => "tabular (un `Dataset`)",
            Familia::Objetos => "de objetos (una `MediaCollection`)",
        }
    }

    /// La unidad por defecto: lo que se cuenta.
    pub fn unidad(self) -> &'static str {
        match self {
            Familia::Tabular => "rows",
            Familia::Objetos => "items",
        }
    }
}

/// Una métrica de la biblioteca.
#[derive(Debug)]
pub struct Metrica {
    pub nombre: &'static str,
    pub familia: Familia,
    /// Mide una columna: `column` es obligatoria. Si no, está prohibida.
    pub columna: bool,
    /// Los `arguments` que admite.
    pub argumentos: &'static [&'static str],
    /// Si los lleva, exactamente uno de estos (vacío: ninguno obligatorio).
    pub uno_de: &'static [&'static str],
    /// Es un cambio respecto de la versión anterior: siempre en `percent`.
    pub cambio: bool,
}

const fn m(
    nombre: &'static str,
    familia: Familia,
    columna: bool,
    argumentos: &'static [&'static str],
    uno_de: &'static [&'static str],
    cambio: bool,
) -> Metrica {
    Metrica {
        nombre,
        familia,
        columna,
        argumentos,
        uno_de,
        cambio,
    }
}

/// **La biblioteca** (`01` §3). Añadir una métrica es añadir su fila aquí, su
/// esquema en la spec y su medidor.
pub const BIBLIOTECA: &[Metrica] = &[
    m("rowCount", Familia::Tabular, false, &[], &[], false),
    m("rowCountChange", Familia::Tabular, false, &[], &[], true),
    m("nullValues", Familia::Tabular, true, &[], &[], false),
    m(
        "missingValues",
        Familia::Tabular,
        true,
        &["missingValues"],
        &[],
        false,
    ),
    m(
        "invalidValues",
        Familia::Tabular,
        true,
        &["validValues", "between"],
        &["validValues", "between"],
        false,
    ),
    m("duplicateValues", Familia::Tabular, true, &[], &[], false),
    m("itemCount", Familia::Objetos, false, &[], &[], false),
    m("itemCountChange", Familia::Objetos, false, &[], &[], true),
    m("emptyItems", Familia::Objetos, false, &[], &[], false),
    m(
        "invalidFormats",
        Familia::Objetos,
        false,
        &["formats", "families"],
        &["formats", "families"],
        false,
    ),
    m(
        "invalidSizes",
        Familia::Objetos,
        false,
        &["between"],
        &["between"],
        false,
    ),
    m("duplicateItems", Familia::Objetos, false, &[], &[], false),
    m("lostItems", Familia::Objetos, false, &[], &[], false),
];

pub fn metrica(nombre: &str) -> Option<&'static Metrica> {
    BIBLIOTECA.iter().find(|x| x.nombre == nombre)
}

/// `notEmptied`, la promesa de serie (`01` §5): se ablanda, no se quita.
pub const DE_SERIE: &str = "notEmptied";

/// Los comparadores de ODCS, con su nombre y su sentido.
pub const COMPARADORES: &[&str] = &[
    "mustBe",
    "mustNotBe",
    "mustBeGreaterThan",
    "mustBeGreaterOrEqualTo",
    "mustBeLessThan",
    "mustBeLessOrEqualTo",
    "mustBeBetween",
    "mustNotBeBetween",
];

const RANGOS: &[&str] = &["mustBeBetween", "mustNotBeBetween"];

/// Las familias que `arguments.families` admite: las de un `ObjectTable` de
/// v1alpha29 (un ítem es de una, o `binary`).
fn familias_de_medios() -> Vec<&'static str> {
    crate::document::medios_de_objetos(ApiVersion::V1Alpha29)
}

/// **Lo que el compilador dice de `spec.promises`** (`01` §2, §3, §5). Solo en
/// un `Dataset` o una `MediaCollection` de v1alpha31 o posterior: en lo demás
/// la clave ya es `OOS1005`.
pub fn validar(
    file: &Path,
    root: &Node,
    kind: Kind,
    version: ApiVersion,
    out: &mut Vec<Diagnostic>,
) {
    let Some(familia) = Familia::de(kind) else {
        return;
    };
    if version < ApiVersion::V1Alpha31 {
        return;
    }
    let Some((_, spec)) = root.get("spec") else {
        return;
    };
    let Some((clave, lista)) = spec.get("promises") else {
        return;
    };
    let forma = |n: &Node, msg: String, ayuda: &str| {
        let d = Diagnostic::new(Code::Oos1004, file, msg).at(n.pos());
        if ayuda.is_empty() { d } else { d.help(ayuda) }
    };
    let Node::Sequence { items, .. } = lista else {
        out.push(forma(
            clave,
            "`promises` es una lista de promesas".into(),
            "cada una con `- id: …`, su `metric` y un comparador (v1alpha31 `01` §2)",
        ));
        return;
    };
    if items.is_empty() {
        out.push(forma(
            clave,
            "`promises` está vacía".into(),
            "sin promesas, quita la clave: `notEmptied` se aplica igual (v1alpha31 `01` §5)",
        ));
        return;
    }
    let columnas = columnas_declaradas(spec);
    let mut vistos: BTreeSet<String> = BTreeSet::new();
    for p in items {
        let Node::Mapping { .. } = p else {
            out.push(forma(p, "una promesa es un mapa".into(), ""));
            continue;
        };
        let Some((kid, vid)) = p.get("id") else {
            out.push(forma(
                p,
                "una promesa sin `id`".into(),
                "el `id` es lo que nombran una retención, un aviso y la consola",
            ));
            continue;
        };
        let id = vid.as_str().unwrap_or_default();
        if !es_identificador(vid) {
            out.push(forma(
                vid,
                format!("`id: {id}` no es un identificador"),
                "letras, dígitos y `_`, empezando por una letra",
            ));
            continue;
        }
        // ── OOS2053 · cada promesa, su nombre ───────────────────────────────
        if !vistos.insert(id.to_string()) {
            out.push(
                Diagnostic::new(
                    Code::Oos2053,
                    file,
                    format!("dos promesas `{id}` en el mismo activo"),
                )
                .at(kid.pos())
                .help(
                    "una retención o un aviso nombran la promesa por su `id`: con dos, no se \
                     sabría cuál. Si son dos promesas, dales dos nombres",
                ),
            );
        }
        if id == DE_SERIE {
            de_serie(p, &forma, out);
            continue;
        }
        una(p, id, familia, columnas.as_ref(), file, &forma, out);
    }
}

/// `notEmptied`: su severidad y su descripción, nada más (`01` §5).
fn de_serie(
    p: &Node,
    forma: &dyn Fn(&Node, String, &str) -> Diagnostic,
    out: &mut Vec<Diagnostic>,
) {
    for (k, v) in p.entries() {
        match k.as_str().unwrap_or_default() {
            "id" => {}
            "severity" => severidad(v, forma, out),
            "description" => {}
            otra => out.push(forma(
                k,
                format!("`{otra}` en `{DE_SERIE}`"),
                "es la promesa de serie: solo se ablanda (`severity: warning`) o se describe. \
                 Lo que mide no se cambia (v1alpha31 `01` §5)",
            )),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn una(
    p: &Node,
    id: &str,
    familia: Familia,
    columnas: Option<&BTreeSet<String>>,
    file: &Path,
    forma: &dyn Fn(&Node, String, &str) -> Diagnostic,
    out: &mut Vec<Diagnostic>,
) {
    // Las claves de una promesa.
    for (k, _) in p.entries() {
        let c = k.as_str().unwrap_or_default();
        if !matches!(
            c,
            "id" | "metric" | "column" | "arguments" | "unit" | "severity" | "description"
        ) && !COMPARADORES.contains(&c)
        {
            out.push(forma(
                k,
                format!("`{c}` no es una clave de una promesa"),
                "una promesa lleva `id`, `metric`, `column`, `arguments`, `unit`, un comparador, \
                 `severity` y `description` (v1alpha31 `01` §2)",
            ));
        }
    }
    // La métrica, de la biblioteca de su familia.
    let Some((km, vm)) = p.get("metric") else {
        out.push(forma(
            p,
            format!("la promesa `{id}` no dice qué mide: le falta `metric`"),
            &format!("la biblioteca {}: {}", familia.nombre(), nombres(familia)),
        ));
        return;
    };
    let nombre = vm.as_str().unwrap_or_default();
    let Some(met) = metrica(nombre).filter(|x| x.familia == familia) else {
        let por_que = match metrica(nombre) {
            Some(x) => format!(
                "`{nombre}` es de la biblioteca {}, y este activo es {}",
                x.familia.nombre(),
                familia.nombre()
            ),
            None => format!("`{nombre}` no es una métrica de la biblioteca"),
        };
        out.push(
            forma(vm, format!("`metric: {nombre}` en la promesa `{id}`"), "").help(format!(
                "{por_que}. La biblioteca {}: {} (v1alpha31 `01` §3)",
                familia.nombre(),
                nombres(familia)
            )),
        );
        let _ = km;
        return;
    };
    // La columna: obligatoria en las de columna, prohibida en las demás.
    match (met.columna, p.get("column")) {
        (true, None) => out.push(forma(
            p,
            format!("la promesa `{id}` ({nombre}) no dice qué columna mide"),
            "añade `column: <columna>`",
        )),
        (false, Some((k, _))) => out.push(forma(
            k,
            format!("`{nombre}` no mide una columna, y la promesa `{id}` dice `column`"),
            "quita `column`",
        )),
        (true, Some((_, v))) => {
            let c = v.as_str().unwrap_or_default();
            // ── OOS2052 · la columna, del activo ──────────────────────────────
            if let Some(cs) = columnas
                && !cs.contains(c)
            {
                out.push(
                    Diagnostic::new(
                        Code::Oos2052,
                        file,
                        format!("la promesa `{id}` mide `{c}`, que no es una columna del activo"),
                    )
                    .at(v.pos())
                    .help(format!(
                        "sus columnas son {}",
                        cs.iter()
                            .map(|x| format!("`{x}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )),
                );
            }
        }
        (false, None) => {}
    }
    // Los argumentos.
    match p.get("arguments") {
        None if !met.uno_de.is_empty() => out.push(forma(
            p,
            format!("`{nombre}` necesita `arguments` en la promesa `{id}`"),
            &format!("uno de {}", lista(met.uno_de)),
        )),
        None => {}
        Some((k, a)) => {
            if met.argumentos.is_empty() {
                out.push(forma(
                    k,
                    format!("`{nombre}` no lleva `arguments`"),
                    "quita `arguments`",
                ));
            } else {
                argumentos(a, met, id, forma, out);
            }
        }
    }
    // La unidad.
    if let Some((_, u)) = p.get("unit") {
        let u = u.as_str().unwrap_or_default();
        let admitidas: &[&str] = if met.cambio {
            &["percent"]
        } else {
            &[familia.unidad(), "percent"]
        };
        if !admitidas.contains(&u) {
            out.push(forma(
                p.get("unit").map(|(_, v)| v).unwrap_or(p),
                format!("`unit: {u}` en la promesa `{id}`"),
                &format!(
                    "{nombre} admite {}{}",
                    lista(admitidas),
                    if met.cambio {
                        ": un cambio se mide siempre en %"
                    } else {
                        ""
                    }
                ),
            ));
        }
    }
    // Exactamente un comparador, y su umbral.
    let comparadores: Vec<(&Node, &Node)> = p
        .entries()
        .iter()
        .filter(|(k, _)| COMPARADORES.contains(&k.as_str().unwrap_or_default()))
        .map(|(k, v)| (k, v))
        .collect();
    match comparadores.as_slice() {
        [] => out.push(forma(
            p,
            format!("la promesa `{id}` no compara con nada"),
            &format!("añade un comparador: {}", lista(COMPARADORES)),
        )),
        [(k, v)] => {
            let c = k.as_str().unwrap_or_default();
            if RANGOS.contains(&c) {
                rango(v, &format!("`{c}` de la promesa `{id}`"), forma, out);
            } else if !es_umbral(v) {
                out.push(forma(
                    v,
                    format!("`{c}` de la promesa `{id}` no es un número"),
                    "un entero, o un decimal entre comillas (`\"0.5\"`)",
                ));
            }
        }
        [_, (k, _), ..] => out.push(forma(
            k,
            format!(
                "la promesa `{id}` tiene {} comparadores",
                comparadores.len()
            ),
            "una promesa compara una vez. Si son dos condiciones, son dos promesas, cada una \
             con su `id`; un intervalo es `mustBeBetween: [mínimo, máximo]`",
        )),
    }
    if let Some((_, s)) = p.get("severity") {
        severidad(s, forma, out);
    }
}

/// Los `arguments` de una métrica: los que admite, y uno solo de los
/// excluyentes.
fn argumentos(
    a: &Node,
    met: &Metrica,
    id: &str,
    forma: &dyn Fn(&Node, String, &str) -> Diagnostic,
    out: &mut Vec<Diagnostic>,
) {
    let Node::Mapping { entries, .. } = a else {
        out.push(forma(a, "`arguments` es un mapa".into(), ""));
        return;
    };
    let mut excluyentes = 0;
    for (k, v) in entries {
        let c = k.as_str().unwrap_or_default();
        if !met.argumentos.contains(&c) {
            out.push(forma(
                k,
                format!("`{}` no admite `arguments.{c}`", met.nombre),
                &format!("admite {}", lista(met.argumentos)),
            ));
            continue;
        }
        if met.uno_de.contains(&c) {
            excluyentes += 1;
        }
        match c {
            "between" => rango(v, &format!("`arguments.between` de `{id}`"), forma, out),
            "families" => {
                let admitidas = familias_de_medios();
                for x in no_vacia(v, c, forma, out) {
                    let f = x.as_str().unwrap_or_default();
                    if !admitidas.contains(&f) {
                        out.push(forma(
                            x,
                            format!("`{f}` no es una familia de medios"),
                            &format!("las familias: {}", admitidas.join(", ")),
                        ));
                    }
                }
            }
            _ => {
                no_vacia(v, c, forma, out);
            }
        }
    }
    if !met.uno_de.is_empty() && excluyentes != 1 {
        out.push(forma(
            a,
            format!(
                "`{}` lleva exactamente uno de {} en la promesa `{id}`",
                met.nombre,
                lista(met.uno_de)
            ),
            "",
        ));
    }
}

fn no_vacia<'a>(
    v: &'a Node,
    c: &str,
    forma: &dyn Fn(&Node, String, &str) -> Diagnostic,
    out: &mut Vec<Diagnostic>,
) -> &'a [Node] {
    match v {
        Node::Sequence { items, .. } if !items.is_empty() => items,
        _ => {
            out.push(forma(v, format!("`{c}` es una lista con algo"), ""));
            &[]
        }
    }
}

/// `[mínimo, máximo]`, dos umbrales, y el mínimo no por encima del máximo.
fn rango(
    v: &Node,
    que: &str,
    forma: &dyn Fn(&Node, String, &str) -> Diagnostic,
    out: &mut Vec<Diagnostic>,
) {
    let ayuda = "`[mínimo, máximo]`, cerrado: dos enteros, o decimales entre comillas";
    match v.items() {
        [a, b] if es_umbral(a) && es_umbral(b) => {
            if let (Some(x), Some(y)) = (numero(a), numero(b))
                && x > y
            {
                out.push(forma(v, format!("{que}: el mínimo pasa del máximo"), ayuda));
            }
        }
        _ => out.push(forma(v, format!("{que} no es un rango"), ayuda)),
    }
}

fn severidad(
    s: &Node,
    forma: &dyn Fn(&Node, String, &str) -> Diagnostic,
    out: &mut Vec<Diagnostic>,
) {
    let v = s.as_str().unwrap_or_default();
    if !matches!(v, "error" | "warning") {
        out.push(forma(
            s,
            format!("`severity: {v}`"),
            "`error` (Blocking: si falla, el refresco no se publica) o `warning` (se publica y \
             se anota)",
        ));
    }
}

/// **Lo que la re-inducción no se puede llevar** (S7a·3). `ore discover`
/// regenera entero lo que lleva su marca, y un `Dataset` inducido al que su
/// dueño añadió `promises` las perdería en la pasada siguiente. Si `viejo`
/// —lo que había en el fichero— tiene un bloque `promises` en su `spec` y
/// `nuevo` no, el bloque pasa tal cual al final del `spec` de `nuevo`, y su
/// `apiVersion` sube a v1alpha31 si era anterior (la puerta es del documento;
/// para estos `kind` subir es neutro). Lo demás de `nuevo`, intacto.
pub fn conservar(viejo: &str, nuevo: &str) -> String {
    let Some(bloque) = bloque_de_promesas(viejo) else {
        return nuevo.to_string();
    };
    if bloque_de_promesas(nuevo).is_some() {
        return nuevo.to_string();
    }
    let lineas: Vec<&str> = nuevo.lines().collect();
    let Some(i) = lineas.iter().position(|l| *l == "spec:") else {
        return nuevo.to_string();
    };
    // El final del `spec`: la siguiente clave de primer nivel, o el final.
    let mut fin = lineas.len();
    for (j, l) in lineas.iter().enumerate().skip(i + 1) {
        if !l.is_empty() && !l.starts_with(' ') && !l.starts_with('#') {
            fin = j;
            break;
        }
    }
    while fin > i + 1 && lineas[fin - 1].trim().is_empty() {
        fin -= 1;
    }
    let mut out: Vec<String> = Vec::with_capacity(lineas.len() + bloque.len());
    for (j, l) in lineas.iter().enumerate() {
        if j == fin {
            out.extend(bloque.iter().cloned());
        }
        out.push(subir_version(l));
    }
    if fin == lineas.len() {
        out.extend(bloque.iter().cloned());
    }
    let mut texto = out.join("\n");
    if nuevo.ends_with('\n') {
        texto.push('\n');
    }
    texto
}

/// El bloque `  promises:` del `spec` de un documento, con sus líneas.
fn bloque_de_promesas(texto: &str) -> Option<Vec<String>> {
    let lineas: Vec<&str> = texto.lines().collect();
    let i = lineas.iter().position(|l| *l == "spec:")?;
    let mut bloque = Vec::new();
    let mut dentro = false;
    for l in &lineas[i + 1..] {
        if !l.is_empty() && !l.starts_with(' ') && !l.starts_with('#') {
            break;
        }
        if dentro {
            // Sigue mientras la línea sea suya: más sangrada, un `  -` de la
            // lista a dos espacios, o en blanco.
            if l.trim().is_empty() || l.starts_with("   ") || l.starts_with("  -") {
                bloque.push(l.to_string());
                continue;
            }
            break;
        }
        if l.starts_with("  promises:") {
            dentro = true;
            bloque.push(l.to_string());
        }
    }
    while bloque.last().is_some_and(|l| l.trim().is_empty()) {
        bloque.pop();
    }
    (!bloque.is_empty()).then_some(bloque)
}

/// **Escribir las promesas de un documento** (0063 S7f): el bloque `promises`
/// pasa a ser `promesas` —la lista entera, cada una un objeto como el YAML—, y
/// el `apiVersion` sube a v1alpha31. Con la lista vacía, el bloque se va (y la
/// versión se queda). Lo demás del texto, intacto: comentarios y orden.
pub fn reescribir(texto: &str, promesas: &[Json]) -> String {
    let sin = quitar_bloque(texto);
    if promesas.is_empty() {
        return sin;
    }
    let mut bloque = String::from("spec:\n  promises:\n");
    for p in promesas {
        let Json::Obj(m) = p else { continue };
        let mut primera = true;
        let mut claves: Vec<&String> = m.keys().collect();
        claves.sort_by_key(|k| orden_de_clave(k));
        for k in claves {
            let v = &m[k];
            bloque.push_str(if primera { "    - " } else { "      " });
            primera = false;
            bloque.push_str(&format!("{k}: {}\n", yaml_de(v)));
        }
    }
    conservar(&bloque, &sin)
}

/// El orden en que se escribe una promesa: lo que se lee primero, primero.
fn orden_de_clave(k: &str) -> (usize, String) {
    let i = ["id", "description", "metric", "column", "arguments", "unit"]
        .iter()
        .position(|x| *x == k)
        .or_else(|| COMPARADORES.iter().position(|x| *x == k).map(|i| 10 + i))
        .unwrap_or(if k == "severity" { 50 } else { 60 });
    (i, k.to_string())
}

/// Un valor en YAML de una línea: un identificador o una palabra va tal cual;
/// un decimal, entre comillas (`OOS6003`); lo demás, como cadena JSON, que es
/// YAML válido.
fn yaml_de(v: &Json) -> String {
    match v {
        Json::Int(i) => i.to_string(),
        Json::Bool(b) => b.to_string(),
        Json::Crudo(c) if c.parse::<f64>().is_ok() => format!("\"{c}\""),
        Json::Crudo(c) => c.clone(),
        Json::Str(s) => {
            let llano = !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/'))
                && !s.starts_with(['-', '.'])
                && s.parse::<f64>().is_err()
                && !matches!(
                    s.as_str(),
                    "true" | "false" | "null" | "yes" | "no" | "on" | "off" | "~"
                );
            if llano { s.clone() } else { Json::s(s).jcs() }
        }
        Json::Arr(xs) => format!(
            "[{}]",
            xs.iter().map(yaml_de).collect::<Vec<_>>().join(", ")
        ),
        Json::Obj(m) => format!(
            "{{ {} }}",
            m.iter()
                .map(|(k, v)| format!("{k}: {}", yaml_de(v)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// El texto sin su bloque `promises` (si lo tenía).
fn quitar_bloque(texto: &str) -> String {
    let Some(bloque) = bloque_de_promesas(texto) else {
        return texto.to_string();
    };
    let lineas: Vec<&str> = texto.lines().collect();
    let Some(i) = lineas.iter().position(|l| l.starts_with("  promises:")) else {
        return texto.to_string();
    };
    let mut out: Vec<&str> = Vec::with_capacity(lineas.len());
    out.extend_from_slice(&lineas[..i]);
    out.extend_from_slice(&lineas[(i + bloque.len()).min(lineas.len())..]);
    let mut t = out.join("\n");
    if texto.ends_with('\n') {
        t.push('\n');
    }
    t
}

/// `apiVersion: oos.dev/v1alphaN` con N < 31 pasa a v1alpha31.
fn subir_version(l: &str) -> String {
    if let Some(v) = l.strip_prefix("apiVersion: ")
        && let Some(v) = ApiVersion::parse(v.trim())
        && v < ApiVersion::V1Alpha31
    {
        return format!("apiVersion: {}", ApiVersion::V1Alpha31.as_str());
    }
    l.to_string()
}

/// Las columnas que el activo declara: las claves de `fields` (una copia) o
/// de `columns` (uno escrito). `None` si no declara ninguna: entonces la
/// columna se sabrá al medir (`unknown`, `01` §3).
pub fn columnas_declaradas(spec: &Node) -> Option<BTreeSet<String>> {
    ["fields", "columns"].iter().find_map(|k| {
        let (_, m) = spec.get(k)?;
        let Node::Mapping { entries, .. } = m else {
            return None;
        };
        Some(
            entries
                .iter()
                .filter_map(|(c, _)| c.as_str().map(String::from))
                .collect(),
        )
    })
}

/// Un umbral: un entero sin comillas, o un decimal como cadena. Un decimal sin
/// comillas ya es `OOS6003` (la forma canónica), y no se dice dos veces.
fn es_umbral(n: &Node) -> bool {
    match n {
        Node::Scalar {
            raw,
            style: Style::Plain,
            ..
        } => raw.parse::<i64>().is_ok() || raw.parse::<f64>().is_ok_and(f64::is_finite),
        Node::Scalar { raw, .. } => es_decimal(raw),
        _ => false,
    }
}

fn es_decimal(s: &str) -> bool {
    let s = s.strip_prefix('-').unwrap_or(s);
    let (e, d) = s.split_once('.').unwrap_or((s, "0"));
    !e.is_empty()
        && !d.is_empty()
        && e.bytes().all(|b| b.is_ascii_digit())
        && d.bytes().all(|b| b.is_ascii_digit())
}

/// El valor de un umbral, para comparar los dos extremos de un rango.
pub fn numero(n: &Node) -> Option<f64> {
    n.as_str()?.parse::<f64>().ok().filter(|x| x.is_finite())
}

fn es_identificador(n: &Node) -> bool {
    let Some(s) = n.as_str() else { return false };
    let mut c = s.chars();
    c.next().is_some_and(|x| x.is_ascii_alphabetic())
        && c.all(|x| x.is_ascii_alphanumeric() || x == '_')
        && s.len() <= 128
}

fn nombres(familia: Familia) -> String {
    BIBLIOTECA
        .iter()
        .filter(|x| x.familia == familia)
        .map(|x| format!("`{}`", x.nombre))
        .collect::<Vec<_>>()
        .join(", ")
}

fn lista(xs: &[&str]) -> String {
    xs.iter()
        .map(|x| format!("`{x}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

// ══ S7b · EL JUEZ ═══════════════════════════════════════════════════════════
//
// Tres pasos, y solo el del medio toca datos (S7c y S7d, en `ore-store` y en
// la transacción de una colección):
//
// 1. `de_documento`: las promesas de un activo, ya tipadas, con `notEmptied`
//    de serie. Lo mal escrito no llega aquí: el compilador lo paró (S7a).
// 2. `peticion`: lo que la pasada tiene que contar, una entrada por promesa
//    que cuenta algo. Sin promesas que cuenten, vacía: no se mide nada más.
// 3. `juzgar`: con lo contado, `pass`, `fail` o `unknown` para cada una, con
//    lo medido y lo esperado. `retenida` dice si alguna `error` falló.

/// Lo que una promesa compara con lo medido (los nombres de ODCS).
#[derive(Debug, Clone, PartialEq)]
pub enum Comparador {
    Igual(f64),
    Distinto(f64),
    Mayor(f64),
    MayorOIgual(f64),
    Menor(f64),
    MenorOIgual(f64),
    Entre(f64, f64),
    FueraDe(f64, f64),
}

impl Comparador {
    fn de(p: &Node) -> Option<Comparador> {
        let uno = |k: &str| p.get(k).and_then(|(_, v)| numero(v));
        let dos = |k: &str| match p.get(k).map(|(_, v)| v.items()) {
            Some([a, b]) => Some((numero(a)?, numero(b)?)),
            _ => None,
        };
        COMPARADORES.iter().find_map(|c| match *c {
            "mustBe" => uno(c).map(Comparador::Igual),
            "mustNotBe" => uno(c).map(Comparador::Distinto),
            "mustBeGreaterThan" => uno(c).map(Comparador::Mayor),
            "mustBeGreaterOrEqualTo" => uno(c).map(Comparador::MayorOIgual),
            "mustBeLessThan" => uno(c).map(Comparador::Menor),
            "mustBeLessOrEqualTo" => uno(c).map(Comparador::MenorOIgual),
            "mustBeBetween" => dos(c).map(|(a, b)| Comparador::Entre(a, b)),
            "mustNotBeBetween" => dos(c).map(|(a, b)| Comparador::FueraDe(a, b)),
            _ => None,
        })
    }

    pub fn cumple(&self, x: f64) -> bool {
        match *self {
            Comparador::Igual(v) => x == v,
            Comparador::Distinto(v) => x != v,
            Comparador::Mayor(v) => x > v,
            Comparador::MayorOIgual(v) => x >= v,
            Comparador::Menor(v) => x < v,
            Comparador::MenorOIgual(v) => x <= v,
            Comparador::Entre(a, b) => a <= x && x <= b,
            Comparador::FueraDe(a, b) => x < a || b < x,
        }
    }

    /// Lo esperado, para quien lo lee (la consola, el puntero): en inglés.
    pub fn texto(&self, pct: bool) -> String {
        let n = |v: f64| format!("{}{}", cifra(v), if pct { " %" } else { "" });
        match *self {
            Comparador::Igual(v) => format!("= {}", n(v)),
            Comparador::Distinto(v) => format!("≠ {}", n(v)),
            Comparador::Mayor(v) => format!("> {}", n(v)),
            Comparador::MayorOIgual(v) => format!("≥ {}", n(v)),
            Comparador::Menor(v) => format!("< {}", n(v)),
            Comparador::MenorOIgual(v) => format!("≤ {}", n(v)),
            Comparador::Entre(a, b) => format!("between {} and {}", cifra(a), n(b)),
            Comparador::FueraDe(a, b) => format!("not between {} and {}", cifra(a), n(b)),
        }
    }
}

/// Lo que algunas métricas necesitan para contar (`arguments`).
#[derive(Debug, Clone, PartialEq)]
pub enum Argumentos {
    Ninguno,
    /// `missingValues`: lo que además de nulo cuenta como ausente, en crudo.
    Ausentes(Vec<String>),
    /// `invalidValues` con `validValues`, en crudo: quien cuenta compara con
    /// el texto del valor.
    Validos(Vec<String>),
    /// `between`: `[mínimo, máximo]`, cerrado.
    Entre(f64, f64),
    Formatos(Vec<String>),
    Familias(Vec<String>),
}

/// Una promesa, tipada.
#[derive(Debug, Clone, PartialEq)]
pub struct Promesa {
    pub id: String,
    /// `None` en `notEmptied`: no se escribe con métrica, es de serie.
    pub metrica: Option<&'static str>,
    pub columna: Option<String>,
    pub argumentos: Argumentos,
    pub porcentaje: bool,
    pub comparador: Option<Comparador>,
    /// `error` (Blocking) o no (`warning`).
    pub bloquea: bool,
    pub de_serie: bool,
}

/// **Las promesas de un activo** (`spec` de un `Dataset` o una
/// `MediaCollection`), con `notEmptied` siempre: la declarada (que solo
/// cambia su severidad) o la de serie, `error`. Lo que no se entiende se
/// salta: el compilador ya lo dijo.
pub fn de_documento(spec: Option<&Node>, familia: Familia) -> Vec<Promesa> {
    let mut out = Vec::new();
    let mut de_serie = Promesa {
        id: DE_SERIE.into(),
        metrica: None,
        columna: None,
        argumentos: Argumentos::Ninguno,
        porcentaje: false,
        comparador: None,
        bloquea: true,
        de_serie: true,
    };
    let lista = spec
        .and_then(|s| s.get("promises"))
        .map(|(_, l)| l.items())
        .unwrap_or(&[]);
    for p in lista {
        let Some(id) = p.get("id").and_then(|(_, v)| v.as_str()) else {
            continue;
        };
        let bloquea = p.get("severity").and_then(|(_, v)| v.as_str()) != Some("warning");
        if id == DE_SERIE {
            de_serie.bloquea = bloquea;
            continue;
        }
        let Some(m) = p
            .get("metric")
            .and_then(|(_, v)| v.as_str())
            .and_then(metrica)
            .filter(|m| m.familia == familia)
        else {
            continue;
        };
        let Some(comparador) = Comparador::de(p) else {
            continue;
        };
        let args = p.get("arguments").map(|(_, a)| a);
        let crudos = |k: &str| -> Option<Vec<String>> {
            Some(
                args?
                    .get(k)?
                    .1
                    .items()
                    .iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect(),
            )
        };
        let entre = args.and_then(|a| match a.get("between").map(|(_, v)| v.items()) {
            Some([x, y]) => Some(Argumentos::Entre(numero(x)?, numero(y)?)),
            _ => None,
        });
        let argumentos = match m.nombre {
            "missingValues" => Argumentos::Ausentes(crudos("missingValues").unwrap_or_default()),
            "invalidValues" | "invalidSizes" => crudos("validValues")
                .map(Argumentos::Validos)
                .or(entre)
                .unwrap_or(Argumentos::Ninguno),
            "invalidFormats" => crudos("formats")
                .map(Argumentos::Formatos)
                .or_else(|| crudos("families").map(Argumentos::Familias))
                .unwrap_or(Argumentos::Ninguno),
            _ => Argumentos::Ninguno,
        };
        out.push(Promesa {
            id: id.into(),
            metrica: Some(m.nombre),
            columna: p
                .get("column")
                .and_then(|(_, v)| v.as_str())
                .map(String::from),
            argumentos,
            porcentaje: m.cambio || p.get("unit").and_then(|(_, v)| v.as_str()) == Some("percent"),
            comparador: Some(comparador),
            bloquea,
            de_serie: false,
        });
    }
    out.push(de_serie);
    out
}

/// ¿Esta métrica sale del total (filas o ítems), o hay que contarla?
fn del_total(metrica: &str) -> bool {
    matches!(
        metrica,
        "rowCount" | "rowCountChange" | "itemCount" | "itemCountChange"
    )
}

/// **Lo que la pasada tiene que contar** (va a `ore-store`, o a la transacción
/// de la colección): una entrada por promesa que cuenta algo, con su `id`, su
/// `metric`, su `column` y sus `arguments` tal como se escribieron. El total y
/// los nulos ya se cuentan siempre: lo que solo necesita eso no pide nada.
/// Sin nada que contar, una lista vacía, y la pasada es la de siempre.
pub fn peticion(promesas: &[Promesa]) -> Json {
    Json::Arr(
        promesas
            .iter()
            .filter_map(|p| {
                let m = p.metrica?;
                if del_total(m) {
                    return None;
                }
                let mut o: Vec<(&'static str, Json)> =
                    vec![("id", Json::s(&p.id)), ("metric", Json::s(m))];
                if let Some(c) = &p.columna {
                    o.push(("column", Json::s(c)));
                }
                let lista = |xs: &[String]| Json::Arr(xs.iter().map(Json::s).collect());
                match &p.argumentos {
                    Argumentos::Ninguno => {}
                    Argumentos::Ausentes(v) => o.push(("missingValues", lista(v))),
                    Argumentos::Validos(v) => o.push(("validValues", lista(v))),
                    Argumentos::Entre(a, b) => o.push((
                        "between",
                        Json::Arr(vec![Json::s(cifra(*a)), Json::s(cifra(*b))]),
                    )),
                    Argumentos::Formatos(v) => o.push(("formats", lista(v))),
                    Argumentos::Familias(v) => o.push(("families", lista(v))),
                }
                Some(Json::obj(o))
            })
            .collect(),
    )
}

/// **Lo que la pasada contó.**
#[derive(Debug, Clone, Default)]
pub struct Medido {
    /// Las filas o los ítems de la versión que se va a publicar. `None` si no
    /// se sabe (entonces todo lo que depende de él es `unknown`).
    pub total: Option<u64>,
    /// Los de la versión publicada anterior; `None` si no la hay.
    pub anterior: Option<u64>,
    /// Lo contado para cada promesa que lo pidió, por su `id`; o por qué no se
    /// pudo (en inglés: lo lee el dueño). Lo que falta aquí es `unknown`.
    pub cuentas: std::collections::BTreeMap<String, Result<u64, String>>,
    /// Sobre cuántas filas se contaron las `cuentas` (0063 S7e): en un
    /// `write()` que anexa, sólo las que llegan; `None` es `total`. Es el
    /// denominador de un porcentaje.
    pub contadas: Option<u64>,
}

/// El resultado de una promesa (`01` §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resultado {
    Pass,
    Fail,
    Unknown,
}

impl Resultado {
    pub fn as_str(self) -> &'static str {
        match self {
            Resultado::Pass => "pass",
            Resultado::Fail => "fail",
            Resultado::Unknown => "unknown",
        }
    }
}

/// Lo que el juez dice de una promesa.
#[derive(Debug, Clone, PartialEq)]
pub struct Veredicto {
    pub id: String,
    pub resultado: Resultado,
    /// Lo medido, en la unidad de la promesa (% si `percent`).
    pub medido: Option<f64>,
    pub porcentaje: bool,
    /// Lo esperado, en inglés («≥ 900», «= 0 %»).
    pub esperado: String,
    pub bloquea: bool,
    pub de_serie: bool,
    /// Por qué `unknown`, en inglés.
    pub motivo: Option<String>,
}

impl Veredicto {
    /// Para el puntero y el informe (`promesas: [...]`).
    pub fn json(&self) -> Json {
        let mut o: Vec<(&'static str, Json)> = vec![
            ("id", Json::s(&self.id)),
            ("resultado", Json::s(self.resultado.as_str())),
            ("esperado", Json::s(&self.esperado)),
            (
                "severidad",
                Json::s(if self.bloquea { "error" } else { "warning" }),
            ),
        ];
        if let Some(x) = self.medido {
            let v = if x.fract() == 0.0 && x.abs() < 9.0e15 {
                Json::Int(x as i64)
            } else {
                Json::s(cifra(x))
            };
            o.push(("medido", v));
        }
        if self.porcentaje {
            o.push(("unidad", Json::s("percent")));
        }
        if self.de_serie {
            o.push(("de_serie", Json::Bool(true)));
        }
        if let Some(m) = &self.motivo {
            o.push(("motivo", Json::s(m)));
        }
        Json::obj(o)
    }
}

/// **El juez** (`01` §6): cada promesa contra lo medido.
pub fn juzgar(promesas: &[Promesa], medido: &Medido) -> Vec<Veredicto> {
    promesas.iter().map(|p| uno_de(p, medido)).collect()
}

fn uno_de(p: &Promesa, medido: &Medido) -> Veredicto {
    let mut v = Veredicto {
        id: p.id.clone(),
        resultado: Resultado::Unknown,
        medido: None,
        porcentaje: p.porcentaje,
        esperado: String::new(),
        bloquea: p.bloquea,
        de_serie: p.de_serie,
        motivo: None,
    };
    let no_se = |mut v: Veredicto, m: &str| {
        v.motivo = Some(m.to_string());
        v
    };
    // `notEmptied`: una versión vacía tras una que no lo estaba.
    let (Some(m), Some(c)) = (p.metrica, &p.comparador) else {
        v.esperado = "not empty when the previous version had data".into();
        let Some(t) = medido.total else {
            return no_se(v, "the size of this version is not known");
        };
        v.medido = Some(t as f64);
        v.resultado = if t == 0 && medido.anterior.is_some_and(|a| a > 0) {
            Resultado::Fail
        } else {
            Resultado::Pass
        };
        return v;
    };
    v.esperado = c.texto(p.porcentaje);
    let Some(total) = medido.total else {
        return no_se(v, "the size of this version is not known");
    };
    let valor = match m {
        "rowCount" | "itemCount" => total as f64,
        "rowCountChange" | "itemCountChange" => match medido.anterior {
            None => return no_se(v, "there is no previous version to compare with"),
            Some(0) => return no_se(v, "the previous version was empty"),
            Some(a) => (total as f64 - a as f64) / a as f64 * 100.0,
        },
        _ => match medido.cuentas.get(&p.id) {
            None => return no_se(v, "this refresh did not measure it"),
            Some(Err(m)) => return no_se(v, m),
            Some(Ok(n)) if p.porcentaje => {
                let de = medido.contadas.unwrap_or(total);
                if de == 0 {
                    0.0
                } else {
                    *n as f64 / de as f64 * 100.0
                }
            }
            Some(Ok(n)) => *n as f64,
        },
    };
    let valor = (valor * 100.0).round() / 100.0;
    v.medido = Some(valor);
    v.resultado = if c.cumple(valor) {
        Resultado::Pass
    } else {
        Resultado::Fail
    };
    v
}

/// La primera promesa `error` que falló: la que retiene la versión.
pub fn retenida(veredictos: &[Veredicto]) -> Option<&Veredicto> {
    veredictos
        .iter()
        .find(|v| v.bloquea && v.resultado == Resultado::Fail)
}

/// Las `warning` que fallaron: se publica, y se anotan.
pub fn avisos(veredictos: &[Veredicto]) -> Vec<&Veredicto> {
    veredictos
        .iter()
        .filter(|v| !v.bloquea && v.resultado == Resultado::Fail)
        .collect()
}

/// El motivo de una retención o un aviso, en inglés: «promise `volumen`:
/// measured 500, expected ≥ 900».
pub fn por_que(v: &Veredicto) -> String {
    let medido = v
        .medido
        .map(|x| format!("{}{}", cifra(x), if v.porcentaje { " %" } else { "" }))
        .unwrap_or_else(|| "nothing".into());
    format!(
        "promise `{}`: measured {medido}, expected {}",
        v.id, v.esperado
    )
}

/// Un número como se lee: sin `.0`, y con dos decimales como mucho.
fn cifra(x: f64) -> String {
    let r = (x * 100.0).round() / 100.0;
    if r.fract() == 0.0 {
        format!("{}", r as i64)
    } else {
        let s = format!("{r:.2}");
        s.trim_end_matches('0').to_string()
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn diags(doc: &str) -> Vec<(Code, String)> {
        crate::validate::validate_document(Path::new("x.yaml"), doc)
            .into_iter()
            .map(|d| (d.code, d.message))
            .collect()
    }

    const COPIA: &str = "apiVersion: oos.dev/v1alpha31\nkind: Dataset\nmetadata: { name: pedidos, namespace: ventas }\nspec:\n  owner: team:ventas\n  from: { table: ventas.orders }\n  fields: { id: id, cliente: customer, total: total }\n";

    fn con(promesas: &str) -> String {
        format!("{COPIA}  promises:\n{promesas}")
    }

    #[test]
    fn una_promesa_bien_escrita_compila() {
        let d = diags(&con(
            "    - id: volumen\n      metric: rowCount\n      mustBeGreaterOrEqualTo: 900\n    - id: total_en_rango\n      column: total\n      metric: invalidValues\n      arguments: { between: [0, 10000] }\n      unit: percent\n      mustBeLessThan: \"0.5\"\n      severity: warning\n    - id: notEmptied\n      severity: warning\n",
        ));
        assert!(d.is_empty(), "{d:?}");
    }

    #[test]
    fn la_biblioteca_es_de_su_familia() {
        let d = diags(&con(
            "    - id: f\n      metric: itemCount\n      mustBe: 0\n",
        ));
        assert_eq!(d.len(), 1, "{d:?}");
        assert_eq!(d[0].0, Code::Oos1004);
        let d = diags(&con(
            "    - id: f\n      metric: rowCont\n      mustBe: 0\n",
        ));
        assert_eq!(d[0].0, Code::Oos1004);
    }

    #[test]
    fn un_comparador_ni_mas_ni_menos() {
        let d = diags(&con("    - id: v\n      metric: rowCount\n"));
        assert_eq!(d.len(), 1, "{d:?}");
        assert!(d[0].1.contains("no compara"), "{d:?}");
        let d = diags(&con(
            "    - id: v\n      metric: rowCount\n      mustBeGreaterThan: 1\n      mustBeLessThan: 9\n",
        ));
        assert!(d[0].1.contains("2 comparadores"), "{d:?}");
        let d = diags(&con(
            "    - id: v\n      metric: rowCount\n      mustBeBetween: [9, 1]\n",
        ));
        assert!(d[0].1.contains("el mínimo pasa del máximo"), "{d:?}");
    }

    #[test]
    fn la_columna_es_del_activo_y_solo_donde_toca() {
        let d = diags(&con(
            "    - id: c\n      column: comprador\n      metric: nullValues\n      mustBe: 0\n",
        ));
        assert_eq!(d.len(), 1, "{d:?}");
        assert_eq!(d[0].0, Code::Oos2052);
        let d = diags(&con(
            "    - id: c\n      metric: nullValues\n      mustBe: 0\n",
        ));
        assert_eq!(d[0].0, Code::Oos1004);
        let d = diags(&con(
            "    - id: c\n      column: id\n      metric: rowCount\n      mustBe: 0\n",
        ));
        assert_eq!(d[0].0, Code::Oos1004);
    }

    #[test]
    fn los_argumentos_de_cada_metrica() {
        // invalidValues: uno de validValues o between, no los dos ni ninguno.
        let d = diags(&con(
            "    - id: e\n      column: id\n      metric: invalidValues\n      mustBe: 0\n",
        ));
        assert_eq!(d[0].0, Code::Oos1004, "{d:?}");
        let d = diags(&con(
            "    - id: e\n      column: id\n      metric: invalidValues\n      arguments: { validValues: [1], between: [0, 1] }\n      mustBe: 0\n",
        ));
        assert!(d[0].1.contains("exactamente uno"), "{d:?}");
        let d = diags(&con(
            "    - id: e\n      column: id\n      metric: nullValues\n      arguments: { validValues: [1] }\n      mustBe: 0\n",
        ));
        assert!(d[0].1.contains("no lleva `arguments`"), "{d:?}");
        // Un cambio es en %.
        let d = diags(&con(
            "    - id: e\n      metric: rowCountChange\n      unit: rows\n      mustBeGreaterThan: -50\n",
        ));
        assert!(d[0].1.contains("unit: rows"), "{d:?}");
    }

    #[test]
    fn cada_promesa_su_nombre_y_not_emptied_solo_se_ablanda() {
        let d = diags(&con(
            "    - id: v\n      metric: rowCount\n      mustBe: 1\n    - id: v\n      metric: rowCount\n      mustBe: 2\n",
        ));
        assert_eq!(d.len(), 1, "{d:?}");
        assert_eq!(d[0].0, Code::Oos2053);
        let d = diags(&con(
            "    - id: notEmptied\n      metric: rowCount\n      mustBe: 0\n",
        ));
        assert_eq!(d.len(), 2, "{d:?}");
        assert!(d.iter().all(|x| x.0 == Code::Oos1004), "{d:?}");
    }

    #[test]
    fn la_biblioteca_de_objetos() {
        let doc = "apiVersion: oos.dev/v1alpha31\nkind: MediaCollection\nmetadata: { name: imagenes, namespace: lago }\nspec:\n  owner: team:data\n  media: image\n  formats: [jpeg]\n  promises:\n    - id: solo\n      metric: invalidFormats\n      arguments: { families: [image] }\n      mustBe: 0\n    - id: sizes\n      metric: invalidSizes\n      arguments: { between: [1, 1000] }\n      unit: items\n      mustBe: 0\n";
        assert!(diags(doc).is_empty(), "{:?}", diags(doc));
        let mal = doc.replace("families: [image]", "families: [imagen]");
        let d = diags(&mal);
        assert!(d[0].1.contains("no es una familia"), "{d:?}");
        let mal = doc.replace("      unit: items\n", "      column: x\n");
        let d = diags(&mal);
        assert!(d[0].1.contains("no mide una columna"), "{d:?}");
    }

    #[test]
    fn la_re_induccion_conserva_las_promesas() {
        let viejo = "# ore discover: marca\napiVersion: oos.dev/v1alpha31\nkind: Dataset\nmetadata:\n  name: pedidos\n  namespace: ventas\nspec:\n  owner: \"user:victor\"\n  from: { table: pg.public.pedidos }\n  fields:\n    id: id\n  promises:\n    - id: volumen\n      metric: rowCount\n\n      mustBeGreaterOrEqualTo: 900\n  - id: id_unico\n    column: id\n    metric: duplicateValues\n    mustBe: 0\n";
        let nuevo = "# ore discover: marca\napiVersion: oos.dev/v1alpha12\nkind: Dataset\nmetadata:\n  name: pedidos\n  namespace: ventas\nspec:\n  owner: \"user:victor\"\n  from: { table: pg.public.pedidos }\n  fields:\n    id: id\n    total: total\n";
        let c = conservar(viejo, nuevo);
        assert_eq!(
            c,
            "# ore discover: marca\napiVersion: oos.dev/v1alpha31\nkind: Dataset\nmetadata:\n  name: pedidos\n  namespace: ventas\nspec:\n  owner: \"user:victor\"\n  from: { table: pg.public.pedidos }\n  fields:\n    id: id\n    total: total\n  promises:\n    - id: volumen\n      metric: rowCount\n\n      mustBeGreaterOrEqualTo: 900\n  - id: id_unico\n    column: id\n    metric: duplicateValues\n    mustBe: 0\n"
        );
        // Sin promesas en lo viejo, lo nuevo tal cual (y su versión también).
        assert_eq!(conservar(nuevo, nuevo), nuevo);
        // Una clave de primer nivel después del `spec` no se come el bloque.
        let con_cola = format!("{nuevo}status: x\n");
        assert!(conservar(viejo, &con_cola).ends_with("    mustBe: 0\nstatus: x\n"));
        // Y lo conservado compila (sin la marca, que no es del documento).
        let limpio = "apiVersion: oos.dev/v1alpha12\nkind: Dataset\nmetadata: { name: pedidos, namespace: ventas }\nspec:\n  owner: team:ventas\n  from: { table: ventas.orders }\n  fields: { id: id }\n";
        let c = conservar(
            &con("    - id: v\n      metric: rowCount\n      mustBe: 1\n"),
            limpio,
        );
        assert!(c.starts_with("apiVersion: oos.dev/v1alpha31\n"), "{c}");
        assert!(diags(&c).is_empty(), "{:?}", diags(&c));
    }

    #[test]
    fn reescribir_pone_quita_y_sustituye() {
        let doc = "# ore discover: marca\napiVersion: oos.dev/v1alpha13\nkind: Dataset\nmetadata:\n  name: pedidos\n  namespace: ventas\nspec:\n  owner: \"user:victor\"\n  from: { table: ventas.orders }\n  fields:\n    id: id\n    cliente: customer\n    total: total\n";
        let ps = |j: &str| -> Vec<Json> {
            match Json::de_node_fiel(&crate::parse::parse(j).unwrap()) {
                Json::Arr(a) => a,
                _ => vec![],
            }
        };
        let dos = ps(
            r#"[{"severity":"warning","id":"cliente_presente","metric":"nullValues","column":"cliente","unit":"percent","mustBeLessOrEqualTo":0.5,"description":"llega del CRM: a veces sin cliente"},{"id":"estado","metric":"invalidValues","column":"total","arguments":{"between":[0,2000]},"mustBe":0},{"id":"notEmptied","severity":"warning"}]"#,
        );
        let c = reescribir(doc, &dos);
        assert!(
            c.starts_with("# ore discover: marca\napiVersion: oos.dev/v1alpha31\n"),
            "{c}"
        );
        assert!(
            c.ends_with("    total: total\n  promises:\n    - id: cliente_presente\n      description: \"llega del CRM: a veces sin cliente\"\n      metric: nullValues\n      column: cliente\n      unit: percent\n      mustBeLessOrEqualTo: \"0.5\"\n      severity: warning\n    - id: estado\n      metric: invalidValues\n      column: total\n      arguments: { between: [0, 2000] }\n      mustBe: 0\n    - id: notEmptied\n      severity: warning\n"),
            "{c}"
        );
        let sin_marca = c.lines().skip(1).collect::<Vec<_>>().join("\n");
        assert!(diags(&sin_marca).is_empty(), "{:?}", diags(&sin_marca));
        // Sustituir: la lista nueva, no la suma.
        let una = ps(r#"[{"id":"volumen","metric":"rowCount","mustBeGreaterOrEqualTo":900}]"#);
        let c2 = reescribir(&c, &una);
        assert!(
            !c2.contains("cliente_presente") && c2.contains("- id: volumen"),
            "{c2}"
        );
        assert_eq!(c2.matches("promises:").count(), 1);
        // Vacía: el bloque se va; lo demás, como estaba.
        let c3 = reescribir(&c2, &[]);
        assert_eq!(c3, doc.replace("v1alpha13", "v1alpha31"));
    }

    #[test]
    fn subir_la_version_es_neutro() {
        // Un `Dataset` de v1alpha12 y una colección de v1alpha19, sin promesas:
        // a v1alpha31 compilan igual.
        let dataset = COPIA.replace("v1alpha31", "v1alpha12");
        assert!(diags(&dataset).is_empty(), "{:?}", diags(&dataset));
        assert!(diags(COPIA).is_empty(), "{:?}", diags(COPIA));
        let coleccion = "apiVersion: oos.dev/v1alpha19\nkind: MediaCollection\nmetadata: { name: imagenes, namespace: lago }\nspec:\n  owner: team:data\n  media: image\n  formats: [jpeg]\n";
        assert_eq!(
            diags(coleccion).len(),
            diags(&coleccion.replace("v1alpha19", "v1alpha31")).len()
        );
    }

    fn promesas(doc: &str) -> Vec<Promesa> {
        let n = crate::parse::parse(doc).unwrap();
        let familia = match n.get("kind").and_then(|(_, k)| k.as_str()) {
            Some("MediaCollection") => Familia::Objetos,
            _ => Familia::Tabular,
        };
        de_documento(n.get("spec").map(|(_, s)| s), familia)
    }

    #[test]
    fn de_documento_tipa_y_pone_not_emptied() {
        let ps = promesas(&con(
            "    - id: volumen\n      metric: rowCount\n      mustBeGreaterOrEqualTo: 900\n    - id: rango\n      column: total\n      metric: invalidValues\n      arguments: { between: [0, \"10000.5\"] }\n      unit: percent\n      mustBeLessThan: \"0.5\"\n      severity: warning\n    - id: estado\n      column: cliente\n      metric: invalidValues\n      arguments: { validValues: [a, 1, true] }\n      mustBe: 0\n",
        ));
        assert_eq!(ps.len(), 4);
        assert_eq!(ps[0].comparador, Some(Comparador::MayorOIgual(900.0)));
        assert!(ps[0].bloquea && !ps[0].porcentaje);
        assert_eq!(ps[1].argumentos, Argumentos::Entre(0.0, 10000.5));
        assert_eq!(ps[1].comparador, Some(Comparador::Menor(0.5)));
        assert!(!ps[1].bloquea && ps[1].porcentaje);
        assert_eq!(
            ps[2].argumentos,
            Argumentos::Validos(vec!["a".into(), "1".into(), "true".into()])
        );
        assert!(ps[3].de_serie && ps[3].bloquea && ps[3].id == DE_SERIE);
        // Ablandada: la misma, en `warning`, y una sola.
        let ps = promesas(&con("    - id: notEmptied\n      severity: warning\n"));
        assert_eq!(ps.len(), 1);
        assert!(ps[0].de_serie && !ps[0].bloquea);
        // Sin promesas: solo la de serie.
        assert_eq!(promesas(COPIA).len(), 1);
    }

    #[test]
    fn la_peticion_solo_pide_lo_que_hay_que_contar() {
        // Sin promesas que cuenten, nada: la pasada es la de siempre.
        assert_eq!(peticion(&promesas(COPIA)).jcs(), "[]");
        let ps = promesas(&con(
            "    - id: volumen\n      metric: rowCount\n      mustBe: 1\n    - id: cambio\n      metric: rowCountChange\n      mustBeGreaterThan: -50\n    - id: unico\n      column: id\n      metric: duplicateValues\n      mustBe: 0\n    - id: rango\n      column: total\n      metric: invalidValues\n      arguments: { between: [0, \"0.5\"] }\n      mustBe: 0\n",
        ));
        assert_eq!(
            peticion(&ps).jcs(),
            r#"[{"column":"id","id":"unico","metric":"duplicateValues"},{"between":["0","0.5"],"column":"total","id":"rango","metric":"invalidValues"}]"#
        );
    }

    #[test]
    fn el_juez() {
        let ps = promesas(&con(
            "    - id: volumen\n      metric: rowCount\n      mustBeGreaterOrEqualTo: 900\n    - id: caida\n      metric: rowCountChange\n      mustBeGreaterThan: -50\n    - id: nulos\n      column: cliente\n      metric: nullValues\n      unit: percent\n      mustBeLessOrEqualTo: 1\n      severity: warning\n    - id: unico\n      column: id\n      metric: duplicateValues\n      mustBe: 0\n",
        ));
        let mut m = Medido {
            total: Some(500),
            anterior: Some(1000),
            ..Default::default()
        };
        m.cuentas.insert("nulos".into(), Ok(50));
        m.cuentas
            .insert("unico".into(), Err("too many values to check".into()));
        let v = juzgar(&ps, &m);
        let r: Vec<(&str, Resultado, Option<f64>)> = v
            .iter()
            .map(|x| (x.id.as_str(), x.resultado, x.medido))
            .collect();
        assert_eq!(
            r,
            vec![
                ("volumen", Resultado::Fail, Some(500.0)),
                ("caida", Resultado::Fail, Some(-50.0)),
                ("nulos", Resultado::Fail, Some(10.0)),
                ("unico", Resultado::Unknown, None),
                ("notEmptied", Resultado::Pass, Some(500.0)),
            ]
        );
        assert_eq!(
            retenida(&v).map(por_que).as_deref(),
            Some("promise `volumen`: measured 500, expected ≥ 900")
        );
        assert_eq!(avisos(&v).len(), 1);
        assert_eq!(
            por_que(avisos(&v)[0]),
            "promise `nulos`: measured 10 %, expected ≤ 1 %"
        );
        assert_eq!(
            v[3].json().jcs(),
            r#"{"esperado":"= 0","id":"unico","motivo":"too many values to check","resultado":"unknown","severidad":"error"}"#
        );
        // Lo que no se midió es `unknown`, nunca `pass`; y sin anterior, el
        // cambio tampoco se sabe.
        let v = juzgar(
            &ps,
            &Medido {
                total: Some(1000),
                ..Default::default()
            },
        );
        assert_eq!(v[0].resultado, Resultado::Pass);
        assert_eq!(v[1].resultado, Resultado::Unknown);
        assert_eq!(v[2].resultado, Resultado::Unknown);
        assert!(retenida(&v).is_none());
    }

    #[test]
    fn not_emptied_y_la_coleccion() {
        let ps = promesas(COPIA);
        let vacia = |anterior| {
            juzgar(
                &ps,
                &Medido {
                    total: Some(0),
                    anterior,
                    ..Default::default()
                },
            )
        };
        assert_eq!(vacia(Some(10))[0].resultado, Resultado::Fail);
        assert_eq!(vacia(Some(0))[0].resultado, Resultado::Pass);
        assert_eq!(vacia(None)[0].resultado, Resultado::Pass);
        assert_eq!(
            juzgar(&ps, &Medido::default())[0].resultado,
            Resultado::Unknown
        );
        let doc = "apiVersion: oos.dev/v1alpha31\nkind: MediaCollection\nmetadata: { name: i, namespace: l }\nspec:\n  owner: team:data\n  media: image\n  formats: [jpeg]\n  promises:\n    - id: solo\n      metric: invalidFormats\n      arguments: { families: [image] }\n      unit: percent\n      mustBeLessThan: 5\n    - id: borrado\n      metric: itemCountChange\n      mustBeGreaterThan: -20\n";
        let ps = promesas(doc);
        assert_eq!(ps[0].argumentos, Argumentos::Familias(vec!["image".into()]));
        let mut m = Medido {
            total: Some(40),
            anterior: Some(100),
            ..Default::default()
        };
        m.cuentas.insert("solo".into(), Ok(1));
        let v = juzgar(&ps, &m);
        assert_eq!((v[0].resultado, v[0].medido), (Resultado::Pass, Some(2.5)));
        assert_eq!(
            (v[1].resultado, v[1].medido),
            (Resultado::Fail, Some(-60.0))
        );
        assert_eq!(
            v[0].json().jcs(),
            r#"{"esperado":"< 5 %","id":"solo","medido":"2.5","resultado":"pass","severidad":"error","unidad":"percent"}"#
        );
        // 0063 S7e: contado sobre lo que llega (un `write()` que anexa), el
        // porcentaje es de eso: 1 de 4 son 25 %, no 1 de 40.
        m.contadas = Some(4);
        assert_eq!(juzgar(&ps, &m)[0].medido, Some(25.0));
    }

    #[test]
    fn la_puerta_es_del_documento() {
        let d = diags(
            &con("    - id: v\n      metric: rowCount\n      mustBe: 1\n")
                .replace("v1alpha31", "v1alpha30"),
        );
        assert_eq!(d.len(), 1, "{d:?}");
        assert_eq!(d[0].0, Code::Oos1005);
    }
}
