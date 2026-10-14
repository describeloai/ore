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
    fn la_puerta_es_del_documento() {
        let d = diags(
            &con("    - id: v\n      metric: rowCount\n      mustBe: 1\n")
                .replace("v1alpha31", "v1alpha30"),
        );
        assert_eq!(d.len(), 1, "{d:?}");
        assert_eq!(d[0].0, Code::Oos1005);
    }
}
