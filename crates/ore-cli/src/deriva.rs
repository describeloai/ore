//! `ore drift` — **qué se movió en el origen desde que se declaró**.
//!
//! # Las dos preguntas que no son la misma
//!
//! `ore diff` contesta **¿quién se rompe?** —una relación entre dos versiones— y
//! esto contesta **¿qué se movió?** —una relación entre el mundo y lo dicho—. No
//! es un matiz: el `Shape` de `diff` no guarda las tablas, guarda el sustrato
//! *«por su efecto sobre cada vista»*, así que **una columna nueva que ninguna
//! vista proyecta es invisible para él**. Y está bien que lo sea; es justo la
//! mitad que la deriva existe para contar.
//!
//! # Qué compara, y qué no puede comparar
//!
//! El catálogo del origen contra las **`kind: Table`** del paquete, que son la
//! declaración del **plano físico** — `01-table` dice que una tabla *es* el
//! hecho. Objetos, columnas, el tipo físico citado y las dos caras.
//!
//! Lo que **no** compara, y no es un descuido: el catálogo trae `primaryKey`,
//! `uniqueKeys` y `foreignKeys`, y el inductor los reparte a la **`Entity`** —a
//! su clave y a sus relaciones—. Ahí ya no son un hecho del origen: son lo que
//! `review` aceptó, y alguien pudo decidir otra cosa. Compararlos contra el
//! catálogo llamaría deriva a una decisión.
//!
//! Su deriva sí se puede ver, y por otro camino: **el catálogo capturado**. Dos
//! capturas se comparan como dos ficheros, y para eso existe `--from`.
//!
//! # La frontera que decide si esto sirve
//!
//! Un paquete gobernado tiene cosas que el catálogo **no puede saber**: quién
//! responde, la madurez, la frescura, si se materializa, las etiquetas y las
//! descripciones que escribió una persona. Nada de eso se mira. Sin esa frontera
//! saldría deriva en todos los paquetes gobernados, siempre, y el detector sería
//! inservible el primer día — la fatiga de alertas que todo el sector reporta,
//! pero por construcción.
//!
//! Y hay una segunda frontera, más tonta y más fácil de olvidar: **solo se miran
//! las tablas de esta fuente**. Un paquete con dos orígenes daría todas las del
//! otro como desaparecidas.
//!
//! # El código de salida
//!
//! **`0` sin deriva · `2` con deriva.** No es un `sysexit`, y es a propósito: es
//! la convención de `terraform plan -detailed-exitcode`, y existe para que esto
//! entre en un pipeline sin que nadie parsee su salida. Los errores —no se
//! encuentra la fuente, el catálogo no analiza— sí usan los `sysexits` del resto
//! del árbol, para que «no pude preguntar» y «el origen cambió» **no se
//! confundan nunca**.
//!
//! # Y no corrige nada
//!
//! Enseña y para. Escribir la corrección es otro acto —y decidir qué se corrige
//! solo y qué se pregunta es una conversación que se tiene con el detector ya
//! delante, no antes.

use std::path::Path;
use std::process::ExitCode;

use ore_core::document::Kind;
use ore_core::link::{Loaded, Package};
use ore_core::parse::Node;
use ore_driver::catalogo::Catalogo;

/// Hacia dónde se movió, que es la primera de las tres preguntas.
///
/// Sale de la promoción de tipos de Iceberg: un cambio no es una clase, son dos
/// y opuestas. Y la tercera —`Incomparable`— no es pereza: dos conjuntos que se
/// cruzan pierden y ganan a la vez, que es lo que `diff` ya hace con el recorte
/// de una vista emitiendo **los dos** códigos en vez de inventar un tercero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direccion {
    /// El origen admite o trae **más** que antes. No rompe lo declarado.
    Ensancha,
    /// El origen admite o trae **menos**. Es lo que duele.
    Estrecha,
    Incomparable,
}

impl Direccion {
    fn marca(self) -> &'static str {
        match self {
            Direccion::Ensancha => "+",
            Direccion::Estrecha => "-",
            Direccion::Incomparable => "~",
        }
    }
}

/// Una cosa que se movió.
#[derive(Debug, Clone)]
pub struct Deriva {
    /// El objeto o la columna, con el nombre que usa el origen.
    pub sujeto: String,
    pub que: String,
    pub de: String,
    pub a: String,
    pub direccion: Direccion,
    /// Las vistas del paquete que lo proyectan. Es el *blast radius*, y aquí
    /// **no se aprende**: la vista que usa esa columna está escrita.
    pub duele_a: Vec<String>,
}

/// **La comparación.** Función pura de (catálogo, paquete): ni red ni ficheros,
/// que es lo que permite que tenga pruebas.
pub fn comparar(cat: &Catalogo, pkg: &Package) -> Vec<Deriva> {
    let mut out = Vec::new();

    // Solo las de ESTA fuente. Ver la cabecera.
    let tablas: Vec<&Loaded> = pkg
        .docs
        .iter()
        .filter(|d| d.kind == Kind::Table)
        .filter(|d| cadena(d, "datasource").as_deref() == Some(cat.fuente()))
        .collect();

    for t in &tablas {
        let Some(objeto) = cadena(t, "object") else {
            continue;
        };
        let Some(c) = cat.tablas.iter().find(|c| c.nombre == objeto) else {
            out.push(Deriva {
                sujeto: objeto.clone(),
                que: "el objeto".into(),
                de: "declarado".into(),
                a: "no está en el origen".into(),
                direccion: Direccion::Estrecha,
                duele_a: vistas_de(pkg, t, None),
            });
            continue;
        };

        columnas(pkg, t, c, &objeto, &mut out);
        cara(
            &objeto,
            "reads",
            t.section("reads"),
            c.lee.as_ref(),
            &mut out,
        );
        cara(
            &objeto,
            "changes",
            t.section("changes"),
            c.cambia.as_ref(),
            &mut out,
        );
    }

    // Y lo que el origen tiene y nadie declaró. Es la deriva más común, no rompe
    // a nadie, y hoy no la ve ningún `diff` — por eso sale como informe.
    let declarados: Vec<String> = tablas.iter().filter_map(|t| cadena(t, "object")).collect();
    for c in &cat.tablas {
        if !declarados.contains(&c.nombre) {
            out.push(Deriva {
                sujeto: c.nombre.clone(),
                que: "el objeto".into(),
                de: "no declarado".into(),
                a: "está en el origen".into(),
                direccion: Direccion::Ensancha,
                duele_a: Vec::new(),
            });
        }
    }
    out
}

fn columnas(
    pkg: &Package,
    t: &Loaded,
    c: &ore_driver::catalogo::Tabla,
    objeto: &str,
    out: &mut Vec<Deriva>,
) {
    let declaradas: Vec<String> = t
        .section("columns")
        .map(|n| {
            n.entries()
                .iter()
                .filter_map(|(k, _)| k.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    for d in &declaradas {
        match c.columnas.iter().find(|x| &x.nombre == d) {
            None => out.push(Deriva {
                sujeto: format!("{objeto}.{d}"),
                que: "la columna".into(),
                de: "declarada".into(),
                a: "no está en el origen".into(),
                direccion: Direccion::Estrecha,
                duele_a: vistas_de(pkg, t, Some(d)),
            }),
            // El tipo FÍSICO, y solo cuando el documento lo cita. Lo cita
            // exactamente cuando el lector no supo traducirlo —`physicalType`—;
            // si lo tradujo, el tipo vive en la entidad y ahí ya es gobierno.
            Some(col) => {
                let antes = t
                    .section("columns")
                    .and_then(|n| n.get(d))
                    .and_then(|(_, v)| v.get("physicalType"))
                    .and_then(|(_, v)| v.as_str())
                    .map(String::from);
                if let (Some(a), Some(b)) = (antes, col.origen.clone())
                    && a != b
                {
                    out.push(Deriva {
                        sujeto: format!("{objeto}.{d}"),
                        que: "el tipo físico".into(),
                        de: a,
                        a: b,
                        direccion: Direccion::Incomparable,
                        duele_a: vistas_de(pkg, t, Some(d)),
                    });
                }
            }
        }
    }

    for col in &c.columnas {
        if !declaradas.contains(&col.nombre) {
            out.push(Deriva {
                sujeto: format!("{objeto}.{}", col.nombre),
                que: "la columna".into(),
                de: "no declarada".into(),
                a: "está en el origen".into(),
                direccion: Direccion::Ensancha,
                duele_a: Vec::new(),
            });
        }
    }
}

/// Una de las dos caras, clave a clave.
///
/// La dirección no se inventa: sale de lo que el propio esquema publicado dice
/// de cada clave.
///
/// | clave | qué es estrechar |
/// |---|---|
/// | `predicatePushdown` · `aggregatePushdown` | perder operadores: se empuja menos |
/// | `requiredFilters` | **ganar** columnas: el origen exige más para contestar |
/// | `fullScan` | bajar la escalera `cheap` → `expensive` → `forbidden` |
/// | `projectionPushdown` | pasar a `false`: la máscara deja de ser estructural (`OOS2029`) |
/// | `witness` | dejar de ser `log`, que es el único que ordena — y sin orden no hay rango por posición |
/// | `mode` | pasar a `none`: el origen deja de emitir cambios |
fn cara(
    objeto: &str,
    nombre: &str,
    antes: Option<&Node>,
    ahora: Option<&Json>,
    out: &mut Vec<Deriva>,
) {
    let Some(ahora) = ahora else { return };
    let Some(antes) = antes else { return };
    for (clave, b) in pares(ahora) {
        // **Se comparan valores, no serializaciones.** La primera version
        // comparaba los dos lados como texto y daba doce derivas falsas contra
        // el catalogo del que salio el paquete: el documento escribe `[eq]` y
        // el JSON `["eq"]`, que es la misma lista con dos comillas de mas. Es
        // exactamente lo que el aserto de cero deriva existe para cazar, y lo
        // cazo en la primera ejecucion.
        let a = antes.get(&clave).map(|(_, v)| valor_de_nodo(v));
        if a.as_ref() == Some(&b) {
            continue;
        }
        let (a, b) = (a.map(|x| x.texto()).unwrap_or_default(), b.texto());
        out.push(Deriva {
            sujeto: format!("{objeto}.{nombre}.{clave}"),
            que: "la cara".into(),
            de: if a.is_empty() {
                "sin declarar".into()
            } else {
                a.clone()
            },
            a: b.clone(),
            direccion: direccion_de(&clave, &a, &b),
            duele_a: Vec::new(),
        });
    }
}

fn direccion_de(clave: &str, a: &str, b: &str) -> Direccion {
    let conjunto = |s: &str| -> Vec<String> {
        s.trim_matches(['[', ']'])
            .split(',')
            .map(|x| x.trim().trim_matches('"').to_string())
            .filter(|x| !x.is_empty())
            .collect()
    };
    match clave {
        "predicatePushdown" | "aggregatePushdown" => {
            let (x, y) = (conjunto(a), conjunto(b));
            comparar_conjuntos(&x, &y)
        }
        // Al revés: exigir MÁS filtros es admitir menos.
        "requiredFilters" => {
            let (x, y) = (conjunto(a), conjunto(b));
            comparar_conjuntos(&y, &x)
        }
        "fullScan" => {
            let nivel = |s: &str| match s {
                "cheap" => 2,
                "expensive" => 1,
                _ => 0,
            };
            match nivel(b).cmp(&nivel(a)) {
                std::cmp::Ordering::Greater => Direccion::Ensancha,
                std::cmp::Ordering::Less => Direccion::Estrecha,
                std::cmp::Ordering::Equal => Direccion::Incomparable,
            }
        }
        "projectionPushdown" => match (a, b) {
            (_, "false") => Direccion::Estrecha,
            (_, "true") => Direccion::Ensancha,
            _ => Direccion::Incomparable,
        },
        // `log` es el único testigo que ORDENA, y el rango por posición depende
        // de eso: `materializar.rs` lo decide con `testigo.0 == "log"`.
        "witness" => match (a == "log", b == "log") {
            (true, false) => Direccion::Estrecha,
            (false, true) => Direccion::Ensancha,
            _ => Direccion::Incomparable,
        },
        "mode" => match (a == "none", b == "none") {
            (false, true) => Direccion::Estrecha,
            (true, false) => Direccion::Ensancha,
            _ => Direccion::Incomparable,
        },
        _ => Direccion::Incomparable,
    }
}

fn comparar_conjuntos(a: &[String], b: &[String]) -> Direccion {
    let pierde = a.iter().any(|x| !b.contains(x));
    let gana = b.iter().any(|x| !a.contains(x));
    match (pierde, gana) {
        (true, false) => Direccion::Estrecha,
        (false, true) => Direccion::Ensancha,
        _ => Direccion::Incomparable,
    }
}

/// Las vistas del paquete que salen de esta tabla, y opcionalmente las que
/// proyectan una columna concreta.
///
/// **Esto es el *blast radius*, y aquí no se aprende.** Las plataformas de
/// observabilidad puntúan el impacto deduciendo qué tablas se usan más; aquí la
/// vista que proyecta esa columna está escrita en un fichero.
fn vistas_de(pkg: &Package, tabla: &Loaded, columna: Option<&str>) -> Vec<String> {
    let corto = tabla
        .root
        .get("metadata")
        .and_then(|(_, m)| m.get("name"))
        .and_then(|(_, n)| n.as_str())
        .unwrap_or_default()
        .to_string();
    let mut out = Vec::new();
    for v in pkg.docs.iter().filter(|d| d.kind == Kind::View) {
        let de = v
            .section("from")
            .and_then(|n| n.get("table"))
            .and_then(|(_, x)| x.as_str())
            .unwrap_or_default();
        if de != corto && !de.ends_with(&format!(".{corto}")) {
            continue;
        }
        if let Some(c) = columna {
            let usa = v
                .section("fields")
                .is_some_and(|n| n.entries().iter().any(|(_, val)| val.as_str() == Some(c)));
            if !usa {
                continue;
            }
        }
        if let Some(q) = v.qname() {
            out.push(q);
        }
    }
    out.sort();
    out
}

// ── Lo que se enseña ────────────────────────────────────────────────────────

/// De dónde sale el catálogo. **Las dos, y `--from` es la que decide**: un
/// aserto que exigiera un servidor no se ejecutaría nunca en la suite, así que
/// `--from` es lo que hace esto probable. Y separarlas evita lo que importa —«no
/// pude preguntarle al origen» y «el origen cambió» son dos respuestas.
pub enum Origen<'a> {
    Fuente(&'a str),
    Fichero(&'a Path),
}

pub fn detectar(raiz: &Path, origen: Origen<'_>) -> ExitCode {
    let texto = match origen {
        Origen::Fichero(p) => match std::fs::read_to_string(p) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("error: no se pudo leer `{}`: {e}", p.display());
                return ExitCode::from(66); // EX_NOINPUT
            }
        },
        Origen::Fuente(f) => match crate::lector::catalogo(raiz, f) {
            Ok(t) => t,
            Err(fallo) => return crate::lector::imprimir(fallo),
        },
    };
    let cat = match Catalogo::leer(&texto) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(65); // EX_DATAERR
        }
    };
    // **Sin exigir validez**: la deriva se pregunta sobre lo que hay, y un
    // paquete con la cola de decisiones abierta es el caso típico.
    let pkg = ore_core::validate::cargar_paquete(raiz).0;
    let derivas = comparar(&cat, &pkg);

    if derivas.is_empty() {
        println!(
            "sin deriva · `{}` dice lo que el paquete declara",
            cat.fuente()
        );
        return ExitCode::SUCCESS;
    }

    // Ordenado por a quién le duele, no por qué cambió: es lo que el sector
    // aprendió por las malas con la fatiga de alertas.
    let (mut duelen, resto): (Vec<&Deriva>, Vec<&Deriva>) = derivas
        .iter()
        .partition(|d| d.direccion == Direccion::Estrecha || !d.duele_a.is_empty());
    duelen.sort_by_key(|d| std::cmp::Reverse(d.duele_a.len()));

    println!("{} deriva(s) · fuente `{}`", derivas.len(), cat.fuente());
    for grupo in [duelen, resto] {
        for d in grupo {
            println!();
            println!("  {} {} · {}", d.direccion.marca(), d.sujeto, d.que);
            println!("      {} → {}", d.de, d.a);
            if !d.duele_a.is_empty() {
                println!("      lo proyecta: {}", d.duele_a.join(", "));
            }
        }
    }
    println!();
    println!("  · nada se ha corregido: este mando enseña y para.");
    // La convención de `terraform plan -detailed-exitcode`. Ver la cabecera.
    ExitCode::from(2)
}

// ── Ayudantes ───────────────────────────────────────────────────────────────

use ore_core::json::Json;

fn cadena(d: &Loaded, clave: &str) -> Option<String> {
    d.section(clave)
        .and_then(|n| n.as_str())
        .map(String::from)
        .or_else(|| {
            d.root
                .get("spec")
                .and_then(|(_, s)| s.get(clave))
                .and_then(|(_, v)| v.as_str())
                .map(String::from)
        })
}

/// **El valor de una clave de la cara, normalizado.**
///
/// Existe porque los dos lados llegan en formas distintas —un `Node` del
/// documento y un `Json` del catalogo— y la unica forma de compararlos sin
/// mentir es traerlos a la misma. Una lista es sus elementos; todo lo demas es
/// su texto.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Valor {
    Escalar(String),
    Lista(Vec<String>),
}

impl Valor {
    fn texto(&self) -> String {
        match self {
            Valor::Escalar(s) => s.clone(),
            Valor::Lista(v) => format!("[{}]", v.join(", ")),
        }
    }
}

/// Los pares de un objeto JSON del catalogo.
fn pares(j: &Json) -> Vec<(String, Valor)> {
    match j {
        Json::Obj(m) => m
            .iter()
            .map(|(k, v)| (k.clone(), valor_de_json(v)))
            .collect(),
        _ => Vec::new(),
    }
}

fn valor_de_json(j: &Json) -> Valor {
    match j {
        Json::Str(s) => Valor::Escalar(s.clone()),
        Json::Int(n) => Valor::Escalar(n.to_string()),
        Json::Bool(b) => Valor::Escalar(b.to_string()),
        Json::Arr(v) => Valor::Lista(v.iter().map(|x| valor_de_json(x).texto()).collect()),
        Json::Obj(_) => Valor::Escalar(j.jcs()),
    }
}

fn valor_de_nodo(n: &Node) -> Valor {
    match n {
        Node::Sequence { items, .. } => {
            Valor::Lista(items.iter().map(|i| valor_de_nodo(i).texto()).collect())
        }
        _ => Valor::Escalar(n.as_str().unwrap_or_default().to_string()),
    }
}
