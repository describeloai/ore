//! **Un `@Transform` de Java** (ORE 0055 T1·7, OOS v1alpha25 `01` §5.5): lo
//! que un método de la clase del fichero declara leer y escribir, leído **sin
//! compilar**, como `python` lee un `@transform`.
//!
//! No hace falta un compilador: basta un lexer que sepa qué es un comentario,
//! una cadena, un *text block* y un carácter —lo que hay dentro no es código—, y
//! la forma de una clase: sus miembros (campos, métodos, tipos anidados,
//! bloques) por sus llaves y paréntesis. Sobre eso, la regla:
//!
//! - la anotación **es** `ore.Transform`: `@ore.Transform`, o `@Transform` con
//!   `import ore.Transform;` o `import ore.*;` sin otra `Transform` importada ni
//!   declarada en el fichero (una importación de un tipo gana a la de un
//!   paquete entero, como en Java);
//! - sobre un método `public static` sin parámetros de la clase del fichero —la
//!   de nivel superior con el nombre del `.java`—, con su nombre una sola vez;
//! - `inputs = {…}` (o un valor sin llaves) y `output = …`, cada uno una cadena
//!   literal o un campo `static final String` de la clase con una cadena
//!   literal, por su nombre o por el de la clase. Lo demás —también lo que
//!   Java pliega, una concatenación— se calcula, y no se deriva.
//!
//! Lo que no es esto no se adivina: [`Clase::sintaxis`] dice por qué el fichero
//! no se lee (un comentario o una cadena sin cerrar, llaves que no casan), y
//! entonces sus documentos se quedan como estaban.

use crate::firma::{Fallo, Rango};
use crate::transform::{Produccion, Sitios, Transform, corto};
use std::collections::BTreeMap;

/// Lo que un `.java` dice de sus transforms.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Clase {
    /// El nombre de la clase del fichero, si la tiene.
    pub nombre: Option<String>,
    /// Los métodos de esa clase, cada uno con el sitio de su nombre (un nombre
    /// sobrecargado sale una vez por declaración).
    pub metodos: Vec<(String, Rango)>,
    /// Cada método con `@ore.Transform`, en el orden del fichero.
    pub transforms: Vec<Transform>,
    /// Un `@ore.Transform` fuera de un método de la clase del fichero: en otra
    /// clase, en una anidada, sobre un campo o dentro de un cuerpo.
    pub avisos: Vec<Fallo>,
    /// Lo que impide leer el fichero. Si hay algo, lo demás no cuenta.
    pub sintaxis: Vec<Fallo>,
}

impl Clase {
    /// Si un método de la clase del fichero se llama `nombre`.
    pub fn tiene_metodo(&self, nombre: &str) -> bool {
        self.metodos.iter().any(|(n, _)| n == nombre)
    }
    /// Si el fichero dice tener transforms: uno que se derivó (o no), o uno fuera de sitio.
    pub fn declara_transforms(&self) -> bool {
        !self.transforms.is_empty() || !self.avisos.is_empty()
    }
}

/// Si merece la pena leer un `.java` buscando transforms: sin el texto
/// `Transform` no puede haber ninguno (un filtro, no una respuesta).
pub fn puede_tener_transforms(fuente: &str) -> bool {
    fuente.contains("Transform")
}

/// `<ruta>.java:<método>` → `(ruta, método)`, o `None` si la forma no vale
/// (`01` §4): la ruta, relativa con `/`, sin `..` ni `/` inicial, que termina
/// en `.java`; el método, un identificador de Java.
pub fn entrypoint(s: &str) -> Option<(&str, &str)> {
    let (ruta, m) = s.rsplit_once(':')?;
    let ruta_ok = !ruta.starts_with('/')
        && !ruta.contains('\\')
        && !ruta.contains(':')
        && ruta.ends_with(".java")
        && ruta.len() > ".java".len()
        && ruta.split('/').all(|t| !t.is_empty() && t != "..");
    let mut cs = m.chars();
    let m_ok = cs.next().is_some_and(es_inicio) && cs.all(es_resto);
    (ruta_ok && m_ok).then_some((ruta, m))
}

/// Cómo se arregla un valor que no se lee.
const DE_TRANSFORM: &str = "it is read without compiling the file: a string literal \
                            (`\"sales.orders\"`) or a `static final String` field of the class \
                            initialized with one (`static final String ORDERS = \"sales.orders\";`), \
                            by its name or the class's (`Summary.ORDERS`)";

// ── el lexer ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tk {
    Id(String),
    /// Una cadena literal, ya sin escapes.
    Cad(String),
    /// Un *text block* (`"""…"""`): un literal, pero no de los que se leen.
    Bloque,
    /// Un carácter o un número: no importan, pero son un valor.
    Valor,
    /// Un comentario `/** … */`, con su texto.
    Doc(String),
    /// Cualquier otro signo, uno a uno.
    P(char),
}

#[derive(Debug, Clone)]
struct Token {
    tk: Tk,
    r: Rango,
}

fn es_inicio(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == '$'
}

fn es_resto(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

fn rango(a: usize, z: usize) -> Rango {
    Rango {
        inicio: a as u32,
        fin: z as u32,
    }
}

fn lexer(f: &str) -> Result<Vec<Token>, Fallo> {
    let b = f.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let siguiente = |i: usize| f[i..].chars().next();
    while i < b.len() {
        let c = siguiente(i).unwrap_or('\0');
        let a = i;
        if c.is_whitespace() {
            i += c.len_utf8();
        } else if f[i..].starts_with("//") {
            i = f[i..].find('\n').map_or(b.len(), |n| i + n);
        } else if f[i..].starts_with("/*") {
            let Some(n) = f[i + 2..].find("*/") else {
                return Err(Fallo::new(
                    rango(a, a + 2),
                    "a `/*` comment that is never closed",
                ));
            };
            let z = i + 2 + n + 2;
            if f[i..].starts_with("/**") && z - i > 4 {
                out.push(Token {
                    tk: Tk::Doc(f[i + 3..z - 2].to_string()),
                    r: rango(a, z),
                });
            }
            i = z;
        } else if f[i..].starts_with("\"\"\"") {
            // Un text block: hasta el `"""` que no va escapado.
            let mut j = i + 3;
            loop {
                match f[j..].find("\"\"\"") {
                    None => {
                        return Err(Fallo::new(
                            rango(a, a + 3),
                            "a text block that is never closed",
                        ));
                    }
                    Some(n) => {
                        let k = j + n;
                        let barras = f[..k].chars().rev().take_while(|&x| x == '\\').count();
                        if barras % 2 == 0 {
                            i = k + 3;
                            break;
                        }
                        j = k + 1;
                    }
                }
            }
            out.push(Token {
                tk: Tk::Bloque,
                r: rango(a, i),
            });
        } else if c == '"' || c == '\'' {
            let (texto, z) = literal(f, i, c)?;
            out.push(Token {
                tk: if c == '"' { Tk::Cad(texto) } else { Tk::Valor },
                r: rango(a, z),
            });
            i = z;
        } else if es_inicio(c) {
            let mut j = i;
            while let Some(x) = siguiente(j).filter(|&x| es_resto(x)) {
                j += x.len_utf8();
            }
            out.push(Token {
                tk: Tk::Id(f[i..j].to_string()),
                r: rango(a, j),
            });
            i = j;
        } else if c.is_ascii_digit()
            || (c == '.' && f[i + 1..].starts_with(|x: char| x.is_ascii_digit()))
        {
            let mut j = i;
            while let Some(x) = siguiente(j) {
                let exponente = matches!(x, '+' | '-')
                    && j > i
                    && matches!(b[j - 1], b'e' | b'E' | b'p' | b'P')
                    && !f[i..j].starts_with("0x");
                if x.is_ascii_alphanumeric() || x == '_' || x == '.' || exponente {
                    j += 1;
                } else {
                    break;
                }
            }
            out.push(Token {
                tk: Tk::Valor,
                r: rango(a, j),
            });
            i = j;
        } else {
            out.push(Token {
                tk: Tk::P(c),
                r: rango(a, a + c.len_utf8()),
            });
            i += c.len_utf8();
        }
    }
    Ok(out)
}

/// Una cadena (`"`) o un carácter (`'`) desde `i`: su texto sin escapes y
/// dónde acaba.
fn literal(f: &str, i: usize, q: char) -> Result<(String, usize), Fallo> {
    let mut texto = String::new();
    let mut it = f[i + 1..].char_indices();
    let sin_cerrar = || {
        Fallo::new(
            rango(i, i + 1),
            if q == '"' {
                "a string that is never closed"
            } else {
                "a character literal that is never closed"
            },
        )
    };
    while let Some((n, c)) = it.next() {
        match c {
            '\n' | '\r' => return Err(sin_cerrar()),
            x if x == q => return Ok((texto, i + 1 + n + 1)),
            '\\' => match it.next().map(|(_, e)| e) {
                Some('n') => texto.push('\n'),
                Some('t') => texto.push('\t'),
                Some('r') => texto.push('\r'),
                Some('b') => texto.push('\u{8}'),
                Some('f') => texto.push('\u{c}'),
                Some('s') => texto.push(' '),
                Some('u') => {
                    let mut h = String::new();
                    for _ in 0..4 {
                        if let Some((_, x)) = it.next() {
                            h.push(x);
                        }
                    }
                    if let Some(x) = u32::from_str_radix(h.trim_start_matches('u'), 16)
                        .ok()
                        .and_then(char::from_u32)
                    {
                        texto.push(x);
                    }
                }
                Some(d @ '0'..='7') => {
                    let mut v = d.to_digit(8).unwrap_or(0);
                    for _ in 0..2 {
                        let resto = it.clone().next().map(|(_, x)| x);
                        match resto.and_then(|x| x.to_digit(8)) {
                            Some(o) if v * 8 + o <= 0o377 => {
                                v = v * 8 + o;
                                it.next();
                            }
                            _ => break,
                        }
                    }
                    texto.push(char::from_u32(v).unwrap_or('\0'));
                }
                Some(x) => texto.push(x),
                None => return Err(sin_cerrar()),
            },
            x => texto.push(x),
        }
    }
    Err(sin_cerrar())
}

// ── la forma de la clase ────────────────────────────────────────────────────

/// Lo que un campo de la clase da para nombrarlo en una anotación.
#[derive(Debug, Clone)]
enum Campo {
    Cadena(String),
    /// Por qué no vale.
    No(&'static str),
}

/// Una anotación: si es la de ORE, dónde está y lo que va entre paréntesis.
struct Anotacion {
    es_ore: bool,
    r: Rango,
    /// Los tokens entre `(` y `)`, sin ellos; `None` sin paréntesis.
    args: Option<Vec<Token>>,
}

/// Un método de la clase del fichero con un `@ore.Transform`, por resolver
/// cuando se sepan todos los campos.
struct Pendiente {
    nombre: String,
    r: Rango,
    anotacion: Anotacion,
    publico_y_estatico: bool,
    con_parametros: bool,
    doc: Option<String>,
}

struct Lector<'a> {
    t: &'a [Token],
    /// `Transform` sin cualificar es la de ORE.
    simple_es_ore: bool,
}

impl Lector<'_> {
    fn id(&self, i: usize) -> Option<&str> {
        match self.t.get(i).map(|x| &x.tk) {
            Some(Tk::Id(s)) => Some(s),
            _ => None,
        }
    }
    fn p(&self, i: usize, c: char) -> bool {
        matches!(self.t.get(i).map(|x| &x.tk), Some(Tk::P(x)) if *x == c)
    }
    /// El índice del que cierra el `abre` de `i` (en `i` está el que abre).
    fn cierre(&self, i: usize, abre: char, cierra: char) -> Option<usize> {
        let mut d = 0usize;
        for (k, x) in self.t.iter().enumerate().skip(i) {
            match x.tk {
                Tk::P(c) if c == abre => d += 1,
                Tk::P(c) if c == cierra => {
                    d -= 1;
                    if d == 0 {
                        return Some(k);
                    }
                }
                _ => {}
            }
        }
        None
    }
    /// La anotación que empieza en `i` (un `@`): la anotación y dónde sigue.
    fn anotacion(&self, i: usize) -> (Anotacion, usize) {
        let mut j = i + 1;
        let mut nombre = Vec::new();
        while let Some(n) = self.id(j) {
            nombre.push(n.to_string());
            j += 1;
            if self.p(j, '.') && self.id(j + 1).is_some() {
                j += 1;
            } else {
                break;
            }
        }
        let es_ore = match nombre
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .as_slice()
        {
            ["ore", "Transform"] => true,
            ["Transform"] => self.simple_es_ore,
            _ => false,
        };
        let mut fin = self.t.get(j.saturating_sub(1)).map_or(0, |x| x.r.fin);
        let mut args = None;
        if self.p(j, '(')
            && let Some(k) = self.cierre(j, '(', ')')
        {
            args = Some(self.t[j + 1..k].to_vec());
            fin = self.t[k].r.fin;
            j = k + 1;
        }
        (
            Anotacion {
                es_ore,
                r: Rango {
                    inicio: self.t[i].r.inicio,
                    fin,
                },
                args,
            },
            j,
        )
    }
    /// Cada `@ore.Transform` entre `a` y `z`: un aviso por cada uno (están
    /// donde no cuentan).
    fn avisos_en(&self, a: usize, z: usize, donde: &str, avisos: &mut Vec<Fallo>) {
        let mut i = a;
        while i < z {
            if self.p(i, '@') && self.id(i + 1) != Some("interface") {
                let (an, j) = self.anotacion(i);
                if an.es_ore {
                    avisos.push(
                        Fallo::new(an.r, format!("`@Transform` {donde}"))
                            .ayuda("a transform is a `public static` method of the file's class (the top-level one named like the `.java`)"),
                    );
                }
                i = j.max(i + 1);
            } else {
                i += 1;
            }
        }
    }
}

const MODIFICADORES: &[&str] = &[
    "public",
    "private",
    "protected",
    "static",
    "final",
    "abstract",
    "synchronized",
    "native",
    "default",
    "strictfp",
    "transient",
    "volatile",
    "sealed",
    "non",
];
const TIPOS: &[&str] = &["class", "interface", "enum", "record"];

/// Un `.java` → lo que dice de sus transforms (OOS v1alpha25 `01` §5.5).
pub fn derivar(fuente: &str, ruta: &str) -> Clase {
    let mut c = Clase::default();
    let tokens = match lexer(fuente) {
        Ok(t) => t,
        Err(f) => {
            c.sintaxis.push(f);
            return c;
        }
    };
    let todo: Vec<Token> = tokens
        .iter()
        .filter(|x| !matches!(x.tk, Tk::Doc(_)))
        .cloned()
        .collect();
    let base = ruta
        .rsplit('/')
        .next()
        .unwrap_or(ruta)
        .trim_end_matches(".java")
        .to_string();

    // ── las llaves casan ──────────────────────────────────────────────────
    let mut pila: Vec<(char, Rango)> = Vec::new();
    for x in &todo {
        if let Tk::P(p) = x.tk {
            match p {
                '{' | '(' | '[' => pila.push((p, x.r)),
                '}' | ')' | ']' => {
                    let abre = match p {
                        '}' => '{',
                        ')' => '(',
                        _ => '[',
                    };
                    if pila.pop().map(|(a, _)| a) != Some(abre) {
                        c.sintaxis
                            .push(Fallo::new(x.r, format!("a `{p}` that closes nothing")));
                        return c;
                    }
                }
                _ => {}
            }
        }
    }
    if let Some((p, r)) = pila.pop() {
        c.sintaxis
            .push(Fallo::new(r, format!("a `{p}` that is never closed")));
        return c;
    }

    // ── las importaciones y los tipos que el fichero declara ────────────────
    let mut importa_ore = false;
    let mut importa_paquete_ore = false;
    let mut otra_importada = false;
    let mut declarada = false;
    let mut i = 0;
    while i < todo.len() {
        let l = Lector {
            t: &todo,
            simple_es_ore: false,
        };
        if l.id(i) == Some("import") {
            let mut j = i + 1;
            let estatica = l.id(j) == Some("static");
            if estatica {
                j += 1;
            }
            let mut partes = Vec::new();
            while j < todo.len() && !l.p(j, ';') {
                match &todo[j].tk {
                    Tk::Id(s) => partes.push(s.clone()),
                    Tk::P('*') => partes.push("*".into()),
                    _ => {}
                }
                j += 1;
            }
            if !estatica {
                match partes
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .as_slice()
                {
                    ["ore", "Transform"] => importa_ore = true,
                    ["ore", "*"] => importa_paquete_ore = true,
                    [.., "Transform"] => otra_importada = true,
                    _ => {}
                }
            }
            i = j + 1;
            continue;
        }
        // `class Transform`, `@interface Transform`…: tapa a la de ORE (no `X.class`).
        if l.id(i)
            .is_some_and(|k| TIPOS.contains(&k) || k == "interface")
            && (i == 0 || !l.p(i - 1, '.'))
            && l.id(i + 1) == Some("Transform")
        {
            declarada = true;
        }
        i += 1;
    }
    // Como en Java: la importación de un tipo gana a la de un paquete entero, y
    // dos `Transform` (dos importadas, o una declarada) no son la de ORE.
    let simple_es_ore = !declarada && !otra_importada && (importa_ore || importa_paquete_ore);
    let l = Lector {
        t: &todo,
        simple_es_ore,
    };

    // ── los tipos de nivel superior ───────────────────────────────────────
    // Los Doc, por posición: el de un miembro es el último antes de él.
    let docs: Vec<(u32, String)> = tokens
        .iter()
        .filter_map(|x| match &x.tk {
            Tk::Doc(s) => Some((x.r.fin, s.clone())),
            _ => None,
        })
        .collect();
    let mut i = 0;
    while i < todo.len() {
        if l.id(i) == Some("package") || l.id(i) == Some("import") {
            while i < todo.len() && !l.p(i, ';') {
                i += 1;
            }
            i += 1;
            continue;
        }
        if l.p(i, ';') {
            i += 1;
            continue;
        }
        // Un tipo de nivel superior: sus anotaciones y modificadores, la palabra y su nombre.
        let mut j = i;
        let mut anotaciones = Vec::new();
        let mut palabra = None;
        while j < todo.len() {
            if l.p(j, '@') && l.id(j + 1) == Some("interface") {
                palabra = Some("@interface");
                j += 2;
                break;
            }
            if l.p(j, '@') {
                let (an, k) = l.anotacion(j);
                anotaciones.push(an);
                j = k;
                continue;
            }
            match l.id(j) {
                Some(k) if TIPOS.contains(&k) => {
                    palabra = Some(k);
                    j += 1;
                    break;
                }
                Some(_) => j += 1,
                None => {
                    if l.p(j, '-') {
                        j += 1; // `non-sealed`
                    } else {
                        break;
                    }
                }
            }
        }
        let nombre = l.id(j).map(str::to_string);
        // Hasta su cuerpo.
        let Some(abre) = (j..todo.len()).find(|&k| l.p(k, '{')) else {
            break;
        };
        let Some(cierra) = l.cierre(abre, '{', '}') else {
            break;
        };
        for an in anotaciones.iter().filter(|a| a.es_ore) {
            c.avisos.push(
                Fallo::new(an.r, "`@Transform` on a type")
                    .ayuda("a transform is a `public static` method of the file's class"),
            );
        }
        if palabra == Some("class")
            && nombre.as_deref() == Some(base.as_str())
            && c.nombre.is_none()
        {
            c.nombre = nombre.clone();
            miembros(&l, fuente, abre + 1, cierra, &base, ruta, &docs, &mut c);
        } else {
            let donde = match &nombre {
                Some(n) if palabra == Some("class") => {
                    format!("in `{n}`, which is not the file's class (`{base}`)")
                }
                Some(n) => format!("in `{n}`, which is not a class"),
                None => "outside the file's class".to_string(),
            };
            l.avisos_en(abre + 1, cierra, &donde, &mut c.avisos);
        }
        i = cierra + 1;
    }
    c
}

/// Los miembros de la clase del fichero, entre `a` y `z` (sin sus llaves).
#[allow(clippy::too_many_arguments)]
fn miembros(
    l: &Lector<'_>,
    fuente: &str,
    a: usize,
    z: usize,
    clase: &str,
    ruta: &str,
    docs: &[(u32, String)],
    c: &mut Clase,
) {
    let mut campos: BTreeMap<String, Campo> = BTreeMap::new();
    let mut pendientes: Vec<Pendiente> = Vec::new();
    let mut i = a;
    let mut fin_anterior = l.t.get(a.saturating_sub(1)).map_or(0, |x| x.r.fin);
    while i < z {
        if l.p(i, ';') {
            fin_anterior = l.t[i].r.fin;
            i += 1;
            continue;
        }
        let inicio_miembro = l.t[i].r.inicio;
        // Lo que va antes del cuerpo o del `;`: anotaciones, modificadores, tipo, nombre…
        let mut anotaciones: Vec<Anotacion> = Vec::new();
        let mut cabeza: Vec<usize> = Vec::new(); // índices de los tokens que no son anotación
        let mut j = i;
        let mut igual = false;
        let mut cuerpo: Option<(usize, usize)> = None;
        let mut fin = z;
        while j < z {
            if l.p(j, '@') && l.id(j + 1) != Some("interface") && !igual {
                let (an, k) = l.anotacion(j);
                anotaciones.push(an);
                j = k;
                continue;
            }
            if l.p(j, '(') {
                let k = l.cierre(j, '(', ')').unwrap_or(z);
                cabeza.extend(j..=k);
                j = k + 1;
                continue;
            }
            if l.p(j, '{') {
                let k = l.cierre(j, '{', '}').unwrap_or(z);
                if igual {
                    cabeza.extend(j..=k);
                    j = k + 1;
                    continue;
                }
                cuerpo = Some((j, k));
                fin = k + 1;
                break;
            }
            if l.p(j, ';') {
                cabeza.push(j);
                fin = j + 1;
                break;
            }
            if l.p(j, '=') {
                igual = true;
            }
            cabeza.push(j);
            j += 1;
        }
        let doc = docs
            .iter()
            .rev()
            .find(|(f, _)| *f <= inicio_miembro && *f >= fin_anterior)
            .map(|(_, d)| d.clone());
        let palabras: Vec<&str> = cabeza.iter().filter_map(|&k| l.id(k)).collect();
        let es_tipo = cabeza.iter().enumerate().any(|(n, &k)| {
            l.id(k).is_some_and(|w| TIPOS.contains(&w)) && (n == 0 || !l.p(cabeza[n - 1], '.'))
        }) || (l.p(i, '@') && l.id(i + 1) == Some("interface"));
        // Un método si su `(` va antes de cualquier `=` (`x = f()` es un campo).
        let igual_en = cabeza.iter().position(|&k| l.p(k, '='));
        let paren = cabeza
            .iter()
            .position(|&k| l.p(k, '('))
            .filter(|&pn| igual_en.is_none_or(|q| pn < q));
        if es_tipo {
            for an in anotaciones.iter().filter(|a| a.es_ore) {
                c.avisos.push(
                    Fallo::new(an.r, "`@Transform` on a nested type")
                        .ayuda("a transform is a `public static` method of the file's class"),
                );
            }
            if let Some((p, q)) = cuerpo {
                l.avisos_en(
                    p + 1,
                    q,
                    "in a nested class, not in the file's class",
                    &mut c.avisos,
                );
            }
        } else if let Some(pn) = paren {
            // Un método (o un constructor): el nombre es lo que va justo antes del `(`.
            let k = cabeza[pn];
            let nombre = pn
                .checked_sub(1)
                .and_then(|n| l.id(cabeza[n]))
                .map(str::to_string);
            let cierre = l.cierre(k, '(', ')').unwrap_or(k);
            let con_parametros = cierre > k + 1;
            if let Some(n) = nombre.filter(|n| n != clase) {
                let r = l.t[cabeza[pn - 1]].r;
                c.metodos.push((n.clone(), r));
                let mods: Vec<&str> = palabras
                    .iter()
                    .copied()
                    .filter(|w| MODIFICADORES.contains(w))
                    .collect();
                let ores: Vec<Anotacion> = anotaciones.into_iter().filter(|a| a.es_ore).collect();
                let mut ores = ores.into_iter();
                if let Some(an) = ores.next() {
                    for otra in ores {
                        c.avisos
                            .push(Fallo::new(otra.r, format!("`@Transform` twice on `{n}`")));
                    }
                    pendientes.push(Pendiente {
                        nombre: n,
                        r,
                        anotacion: an,
                        publico_y_estatico: mods.contains(&"public") && mods.contains(&"static"),
                        con_parametros,
                        doc,
                    });
                }
            } else {
                for an in anotaciones.iter().filter(|a| a.es_ore) {
                    c.avisos
                        .push(Fallo::new(an.r, "`@Transform` on a constructor"));
                }
            }
            if let Some((p, q)) = cuerpo {
                l.avisos_en(p + 1, q, "inside a method body", &mut c.avisos);
            }
        } else if cuerpo.is_some() {
            // Un bloque (`static { … }`, `{ … }`).
            for an in anotaciones.iter().filter(|a| a.es_ore) {
                c.avisos.push(Fallo::new(an.r, "`@Transform` on a block"));
            }
            if let Some((p, q)) = cuerpo {
                l.avisos_en(p + 1, q, "inside a block", &mut c.avisos);
            }
        } else {
            // Un campo: `[mods] Tipo a [= v] (, b [= v])* ;`.
            for an in anotaciones.iter().filter(|a| a.es_ore) {
                c.avisos.push(Fallo::new(an.r, "`@Transform` on a field"));
            }
            campos_de(l, &cabeza, &mut campos);
        }
        fin_anterior =
            l.t.get(fin.saturating_sub(1))
                .map_or(fin_anterior, |x| x.r.fin);
        i = fin;
    }

    // Los nombres repetidos, y cada transform con todos los campos ya sabidos.
    for p in pendientes {
        let veces = c.metodos.iter().filter(|(n, _)| *n == p.nombre).count();
        c.transforms
            .push(transform(p, veces, clase, ruta, &campos, fuente));
    }
}

fn campos_de(l: &Lector<'_>, cabeza: &[usize], campos: &mut BTreeMap<String, Campo>) {
    let tks: Vec<&Token> = cabeza.iter().map(|&k| &l.t[k]).collect();
    let mods: Vec<&str> = tks
        .iter()
        .filter_map(|t| match &t.tk {
            Tk::Id(s) if MODIFICADORES.contains(&s.as_str()) => Some(s.as_str()),
            _ => None,
        })
        .collect();
    let estatico = mods.contains(&"static");
    let fin = mods.contains(&"final");
    // El tipo: lo que va entre los modificadores y el primer nombre.
    let resto: Vec<&Token> = tks
        .iter()
        .copied()
        .skip_while(|t| matches!(&t.tk, Tk::Id(s) if MODIFICADORES.contains(&s.as_str())))
        .collect();
    let primer_nombre = (0..resto.len()).find(|&n| {
        matches!(resto[n].tk, Tk::Id(_))
            && n > 0
            && matches!(
                resto.get(n + 1).map(|t| &t.tk),
                Some(Tk::P('=' | ',' | ';')) | None
            )
    });
    let Some(pn) = primer_nombre else { return };
    let tipo: Vec<String> = resto[..pn]
        .iter()
        .map(|t| match &t.tk {
            Tk::Id(s) => s.clone(),
            Tk::P(c) => c.to_string(),
            _ => "?".into(),
        })
        .collect();
    let es_string = tipo == ["String"] || tipo == ["java", ".", "lang", ".", "String"];
    // Cada declarador: nombre [= valor], separados por `,` de nivel 0.
    let mut n = pn;
    while n < resto.len() {
        let Tk::Id(nombre) = &resto[n].tk else { break };
        let mut m = n + 1;
        let mut valor: Vec<&Token> = Vec::new();
        let mut hay_valor = false;
        if matches!(resto.get(m).map(|t| &t.tk), Some(Tk::P('='))) {
            hay_valor = true;
            m += 1;
            let mut d = 0i32;
            while m < resto.len() {
                match resto[m].tk {
                    Tk::P('(' | '{' | '[') => d += 1,
                    Tk::P(')' | '}' | ']') => d -= 1,
                    Tk::P(',' | ';') if d == 0 => break,
                    _ => {}
                }
                valor.push(resto[m]);
                m += 1;
            }
        }
        let campo = if !es_string {
            Campo::No("is not a `String`")
        } else if !estatico {
            Campo::No("is not `static`")
        } else if !fin {
            Campo::No("is not `final`: anything could reassign it before the build")
        } else {
            match valor.as_slice() {
                [Token { tk: Tk::Cad(s), .. }] => Campo::Cadena(s.clone()),
                _ if !hay_valor => {
                    Campo::No("has no initializer here (it is assigned somewhere else)")
                }
                _ => Campo::No("is not initialized with a string literal"),
            }
        };
        campos.insert(nombre.clone(), campo);
        if !matches!(resto.get(m).map(|t| &t.tk), Some(Tk::P(','))) {
            break;
        }
        n = m + 1;
    }
}

/// Un `@ore.Transform` sobre un método → su transform: lo que declara, o
/// todo lo que impide leerlo.
fn transform(
    p: Pendiente,
    veces: usize,
    clase: &str,
    ruta: &str,
    campos: &BTreeMap<String, Campo>,
    fuente: &str,
) -> Transform {
    let mut fallos: Vec<Fallo> = Vec::new();
    let mut sitios = Sitios {
        decorador: p.anotacion.r,
        ..Sitios::default()
    };
    if !p.publico_y_estatico {
        fallos.push(
            Fallo::new(
                p.r,
                format!(
                    "`@Transform` is on `{}`, which is not `public static`",
                    p.nombre
                ),
            )
            .ayuda("the build calls it as it is: `public static Object name() throws Exception`"),
        );
    }
    if p.con_parametros {
        fallos.push(
            Fallo::new(
                p.r,
                format!(
                    "`{}` has parameters, and the build calls it with none",
                    p.nombre
                ),
            )
            .ayuda("a transform reads what it declares in `inputs`: take no parameters"),
        );
    }
    if veces > 1 {
        fallos.push(
            Fallo::new(p.r, format!("`{}` is declared {veces} times in `{clase}`", p.nombre))
                .ayuda("the `entrypoint` names the method by its name: give the transform a name of its own"),
        );
    }
    let valor = |ts: &[Token]| -> Result<(String, Rango), Fallo> {
        let r = Rango {
            inicio: ts.first().map_or(p.anotacion.r.inicio, |t| t.r.inicio),
            fin: ts.last().map_or(p.anotacion.r.fin, |t| t.r.fin),
        };
        let por_nombre = |n: &str| match campos.get(n) {
            Some(Campo::Cadena(s)) => Ok((s.clone(), r)),
            Some(Campo::No(por)) => Err(Fallo::new(r, format!("`{n}` {por}")).ayuda(DE_TRANSFORM)),
            None => {
                Err(Fallo::new(r, format!("`{n}` is not a field of `{clase}`")).ayuda(DE_TRANSFORM))
            }
        };
        match ts.iter().map(|t| &t.tk).collect::<Vec<_>>().as_slice() {
            [Tk::Cad(s)] => Ok((s.clone(), r)),
            [Tk::Bloque] => Err(Fallo::new(r, "a text block is not read").ayuda(DE_TRANSFORM)),
            [Tk::Id(n)] => por_nombre(n),
            [Tk::Id(c), Tk::P('.'), Tk::Id(n)] if c == clase => por_nombre(n),
            [] => Err(Fallo::new(r, "a value is missing").ayuda(DE_TRANSFORM)),
            _ => Err(Fallo::new(
                r,
                format!(
                    "`{}` is computed: it is not a string literal or a `static final String` of the class",
                    fuente.get(r.inicio as usize..r.fin as usize).unwrap_or("…")
                ),
            )
            .ayuda(DE_TRANSFORM)),
        }
    };
    // Los argumentos, por nombre: `nombre = valor`, separados por `,` de nivel 0.
    let mut inputs: Option<Vec<String>> = None;
    let mut output: Option<String> = None;
    match &p.anotacion.args {
        None => fallos.push(
            Fallo::new(p.anotacion.r, "`@Transform` without `inputs` or `output`")
                .ayuda("a transform says what it reads and what it writes: `@Transform(inputs = {\"sales.orders\"}, output = \"sales.summary\")`"),
        ),
        Some(args) => {
            for arg in partir(args) {
                let (nombre, resto) = match arg.as_slice() {
                    [Token { tk: Tk::Id(n), .. }, Token { tk: Tk::P('='), .. }, resto @ ..] => (n.clone(), resto),
                    _ => {
                        let r = Rango {
                            inicio: arg.first().map_or(p.anotacion.r.inicio, |t| t.r.inicio),
                            fin: arg.last().map_or(p.anotacion.r.fin, |t| t.r.fin),
                        };
                        fallos.push(
                            Fallo::new(r, "`@Transform` with a value without its name")
                                .ayuda("they go by name: `inputs = {…}` and `output = …`"),
                        );
                        continue;
                    }
                };
                match nombre.as_str() {
                    "inputs" => {
                        let items: Vec<Vec<Token>> = match resto {
                            [Token { tk: Tk::P('{'), .. }, medio @ .., Token { tk: Tk::P('}'), .. }] => partir(medio),
                            uno => vec![uno.to_vec()],
                        };
                        let mut leidos: Vec<String> = Vec::new();
                        for it in items {
                            match valor(&it) {
                                // Sin repetir (§4): la primera vez cuenta.
                                Ok((n, r)) => {
                                    let n = corto(&n);
                                    if !leidos.contains(&n) {
                                        sitios.inputs.push((n.clone(), r));
                                        leidos.push(n);
                                    }
                                }
                                Err(f) => fallos.push(f),
                            }
                        }
                        inputs = Some(leidos);
                    }
                    "output" => match valor(resto) {
                        Ok((n, r)) => {
                            sitios.output = Some(r);
                            output = Some(corto(&n));
                        }
                        Err(f) => fallos.push(f),
                    },
                    otro => fallos.push(
                        Fallo::new(arg[0].r, format!("`@Transform` has no argument `{otro}`"))
                            .ayuda("its arguments are `inputs` and `output`"),
                    ),
                }
            }
            for (k, falta) in [("inputs", inputs.is_none()), ("output", output.is_none())] {
                if falta && !fallos.iter().any(|f| f.mensaje.contains(&format!("`{k}`"))) {
                    fallos.push(
                        Fallo::new(p.anotacion.r, format!("`@Transform` without `{k}`"))
                            .ayuda("a transform says what it reads (`inputs = {…}`) and what it writes (`output = …`)"),
                    );
                }
            }
        }
    }
    fallos.sort_by_key(|f| f.rango);
    let resultado = match (inputs, output) {
        (Some(inputs), Some(output)) if fallos.is_empty() => Ok(Produccion {
            runtime: "java",
            entrypoint: format!("{ruta}:{}", p.nombre),
            descripcion: descripcion(p.doc.as_deref()),
            inputs,
            output,
        }),
        _ => Err(fallos),
    };
    Transform {
        nombre: p.nombre,
        rango: p.r,
        resultado,
        sitios,
    }
}

/// Los tokens partidos por las `,` de nivel 0 (un `,` final no da un vacío).
fn partir(ts: &[Token]) -> Vec<Vec<Token>> {
    let mut out: Vec<Vec<Token>> = vec![Vec::new()];
    let mut d = 0i32;
    for t in ts {
        match t.tk {
            Tk::P('(' | '{' | '[') => d += 1,
            Tk::P(')' | '}' | ']') => d -= 1,
            Tk::P(',') if d == 0 => {
                out.push(Vec::new());
                continue;
            }
            _ => {}
        }
        if let Some(u) = out.last_mut() {
            u.push(t.clone());
        }
    }
    if out.last().is_some_and(Vec::is_empty) {
        out.pop();
    }
    out
}

/// La `description`: la primera línea no vacía del Javadoc que no es una
/// etiqueta, sin el `*` del margen (§5.5).
fn descripcion(doc: Option<&str>) -> Option<String> {
    doc?.lines()
        .map(|l| {
            let l = l.trim();
            l.strip_prefix('*').map_or(l, str::trim)
        })
        .take_while(|l| !l.starts_with('@'))
        .find(|l| !l.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RESUMEN: &str = r#"import static ore.Ore.*;

import ore.Transform;

public class Resumen {
    static final String PEDIDOS = "ventas.pedidos";

    /**
     * El total por país.
     *
     * @return lo que escribe write()
     */
    @Transform(inputs = {PEDIDOS, "ventas.clientes"}, output = "ventas.resumen")
    public static Object resumen() throws Exception {
        return write("ventas.resumen", over(PEDIDOS));
    }
}
"#;

    fn uno(fuente: &str) -> Result<Produccion, Vec<String>> {
        let c = derivar(fuente, "etl/transforms/Resumen.java");
        assert!(c.sintaxis.is_empty(), "{:?}", c.sintaxis);
        assert_eq!(c.transforms.len(), 1, "{c:?}");
        c.transforms[0]
            .resultado
            .clone()
            .map_err(|fs| fs.into_iter().map(|f| f.mensaje).collect())
    }

    #[test]
    fn el_de_la_spec_se_deriva_como_su_caso() {
        let p = uno(RESUMEN).unwrap();
        assert_eq!(
            p,
            Produccion {
                runtime: "java",
                entrypoint: "etl/transforms/Resumen.java:resumen".into(),
                descripcion: Some("El total por país.".into()),
                inputs: vec!["ventas.pedidos".into(), "ventas.clientes".into()],
                output: "ventas.resumen".into(),
            }
        );
        let c = derivar(RESUMEN, "etl/transforms/Resumen.java");
        assert_eq!(c.nombre.as_deref(), Some("Resumen"));
        assert!(c.tiene_metodo("resumen"));
        assert!(c.avisos.is_empty());
        let s = &c.transforms[0].sitios;
        assert_eq!(
            &RESUMEN[s.decorador.inicio as usize..s.decorador.inicio as usize + 10],
            "@Transform"
        );
        assert_eq!(
            &RESUMEN[s.inputs[0].1.inicio as usize..s.inputs[0].1.fin as usize],
            "PEDIDOS"
        );
        assert_eq!(
            s.output
                .map(|r| &RESUMEN[r.inicio as usize..r.fin as usize]),
            Some("\"ventas.resumen\"")
        );
    }

    #[test]
    fn dos_en_una_clase_con_comentarios_y_cadenas_que_no_cuentan() {
        let f = r#"import static ore.Ore.*;
import ore.*;
// @Transform(inputs = {}, output = "ventas.nada")
/* @Transform(inputs = {}, output = "ventas.nada2") */
public final class Pedidos {
    static final String CLIENTES = "ventas.clientes", OTRA = "ventas.default.otra";
    private static final java.lang.String TEXTO = """
        @Transform(inputs = {}, output = "x.y")
        """;

    @Transform(inputs = "ventas.pedidos", output = "ventas.por_pais")
    public static Object porPais() throws Exception { return write("ventas.por_pais", sql("select '}' as n")); }

    @ore.Transform(inputs = {Pedidos.CLIENTES, OTRA,}, output = "ventas.activos")
    public static Object activos() throws Exception {
        String texto = "@Transform(inputs = {}, output = \"ventas.otra\")";
        char c = '}';
        Runnable r = () -> { int[] a = {1, 2}; };
        return write("ventas.activos", over(CLIENTES));
    }

    static String ayuda() { return "no"; }
    class Dentro { void m() {} }
}
"#;
        let c = derivar(f, "etl/transforms/Pedidos.java");
        assert!(c.sintaxis.is_empty() && c.avisos.is_empty(), "{c:?}");
        let ps: Vec<Produccion> = c
            .transforms
            .iter()
            .map(|t| t.resultado.clone().unwrap())
            .collect();
        assert_eq!(ps.len(), 2);
        assert_eq!(
            (ps[0].inputs.clone(), ps[0].output.as_str()),
            (vec!["ventas.pedidos".to_string()], "ventas.por_pais")
        );
        assert_eq!(
            ps[1].inputs,
            vec!["ventas.clientes".to_string(), "ventas.otra".to_string()]
        );
        assert_eq!(ps[1].descripcion, None);
        assert!(c.tiene_metodo("ayuda") && !c.tiene_metodo("m"));
    }

    #[test]
    fn lo_que_se_calcula_o_no_es_de_aqui_no_se_deriva() {
        let casos: &[(&str, &str, &str)] = &[
            (
                "{PEDIDOS, \"ventas.clientes\"}",
                "{\"ventas.\" + \"pedidos\"}",
                "is computed",
            ),
            (
                "static final String PEDIDOS",
                "static String PEDIDOS",
                "is not `final`",
            ),
            (
                "static final String PEDIDOS",
                "final String PEDIDOS",
                "is not `static`",
            ),
            ("resumen()", "resumen(String pais)", "has parameters"),
            (
                "public static Object resumen()",
                "static Object resumen()",
                "not `public static`",
            ),
            (
                "{PEDIDOS, \"ventas.clientes\"}",
                "{Otra.PEDIDOS}",
                "is computed",
            ),
            (
                "{PEDIDOS, \"ventas.clientes\"}",
                "{NADA}",
                "is not a field of `Resumen`",
            ),
            (
                "output = \"ventas.resumen\"",
                "output = \"\"\"\n x\"\"\"",
                "a text block",
            ),
            (
                "output = \"ventas.resumen\"",
                "salida = \"ventas.resumen\"",
                "no argument `salida`",
            ),
            (
                "inputs = {PEDIDOS, \"ventas.clientes\"}, ",
                "",
                "without `inputs`",
            ),
            (
                "(inputs = {PEDIDOS, \"ventas.clientes\"}, output = \"ventas.resumen\")",
                "",
                "without `inputs` or `output`",
            ),
            (
                "(inputs = {PEDIDOS, \"ventas.clientes\"}, output = \"ventas.resumen\")",
                "(\"ventas.resumen\")",
                "without its name",
            ),
        ];
        for (a, b, espera) in casos {
            assert!(RESUMEN.contains(a), "{a}");
            let e = uno(&RESUMEN.replacen(a, b, 1)).unwrap_err();
            assert!(e.iter().any(|m| m.contains(espera)), "{b}: {e:?}");
        }
        // Un nombre repetido.
        let f = RESUMEN.replace(
            "    static final String PEDIDOS",
            "    public static void resumen(int x) {}\n    static final String PEDIDOS",
        );
        assert!(
            uno(&f)
                .unwrap_err()
                .iter()
                .any(|m| m.contains("declared 2 times"))
        );
    }

    #[test]
    fn que_transform_es_la_de_ore() {
        let con = |import: &str| {
            derivar(
                &RESUMEN.replace("import ore.Transform;", import),
                "etl/transforms/Resumen.java",
            )
        };
        assert_eq!(con("import ore.*;").transforms.len(), 1);
        assert_eq!(con("import com.acme.Transform;").transforms.len(), 0);
        assert_eq!(
            con("import ore.*;\nimport com.acme.Transform;")
                .transforms
                .len(),
            0
        );
        assert_eq!(con("").transforms.len(), 0);
        assert_eq!(con("@interface Transform {}").transforms.len(), 0);
        // Cualificada, siempre; y su método sigue siendo un método.
        let c = derivar(
            &RESUMEN
                .replace("import ore.Transform;", "")
                .replace("@Transform(", "@ore.Transform("),
            "etl/transforms/Resumen.java",
        );
        assert_eq!(c.transforms.len(), 1);
        let c = con("import com.acme.Transform;");
        assert!(c.tiene_metodo("resumen"));
    }

    #[test]
    fn fuera_de_sitio_es_un_aviso() {
        let f = "import ore.Transform;\npublic class A {\n  static class B { @Transform(inputs = {}, output = \"a.b\") public static void x() {} }\n}\nclass C { @Transform(inputs = {}, output = \"a.c\") public static void y() {} }\n";
        let c = derivar(f, "A.java");
        assert!(c.transforms.is_empty());
        assert_eq!(c.avisos.len(), 2, "{:?}", c.avisos);
        assert!(c.avisos[0].mensaje.contains("nested"), "{:?}", c.avisos);
        assert!(
            c.avisos[1].mensaje.contains("not the file's class"),
            "{:?}",
            c.avisos
        );
        // La clase con otro nombre que el fichero no es la del fichero.
        let c = derivar(RESUMEN, "etl/transforms/Otro.java");
        assert!(c.transforms.is_empty() && c.nombre.is_none() && c.avisos.len() == 1);
    }

    #[test]
    fn lo_que_no_se_lee_lo_dice() {
        for (f, espera) in [
            ("public class A { /* sin cerrar", "never closed"),
            (
                "public class A { String s = \"sin cerrar\n; }",
                "never closed",
            ),
            ("public class A { void m() { }", "never closed"),
            ("public class A { } }", "closes nothing"),
        ] {
            let c = derivar(f, "A.java");
            assert!(
                c.sintaxis.iter().any(|x| x.mensaje.contains(espera)),
                "{f}: {c:?}"
            );
        }
    }

    #[test]
    fn el_entrypoint_de_java() {
        assert_eq!(
            entrypoint("etl/transforms/Resumen.java:resumen"),
            Some(("etl/transforms/Resumen.java", "resumen"))
        );
        assert_eq!(entrypoint("A.java:$x_1"), Some(("A.java", "$x_1")));
        for malo in [
            "A.java",
            "A.java:",
            "/A.java:m",
            "../A.java:m",
            "A.py:m",
            "A.java:1m",
            "a\\A.java:m",
            ".java:m",
        ] {
            assert_eq!(entrypoint(malo), None, "{malo}");
        }
    }

    #[test]
    fn la_descripcion_es_la_primera_linea_del_javadoc() {
        assert_eq!(
            descripcion(Some("*\n * Uno.\n * Dos.\n")),
            Some("Uno.".into())
        );
        assert_eq!(descripcion(Some(" @return x\n")), None);
        assert_eq!(descripcion(Some(" Solo. ")), Some("Solo.".into()));
        assert_eq!(descripcion(None), None);
    }
}
