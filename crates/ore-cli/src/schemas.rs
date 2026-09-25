//! **`ore package schema`** (0038 P6): crear un schema de un paquete y
//! renombrarlo. Lo que la consola pedía con un modal y un doble clic, y hasta
//! hoy se quedaba en el estado del navegador.
//!
//! # Crear
//!
//! Un schema existe porque un `kind: Schema` lo declara, en su carpeta (v1alpha13
//! `01-el-schema` §2): una carpeta vacía no viaja en git. Crear es escribir
//! `<paquete>/<nombre>/schema.yaml`, y nada más.
//!
//! # Renombrar
//!
//! Medido antes (`pruebas-de-fuego/medida-renombrar-schema.py`, un árbol
//! descubierto con lo que lo nombra desde fuera). Renombrar son cinco cosas, y
//! con las cinco el árbol compila exactamente igual que antes:
//!
//! 1. la carpeta, entera —lo de dentro se nombra entre sí en UNA parte
//!    (`backedBy: clientes`), así que viaja sin tocarlo—;
//! 2. `metadata.name` del `Schema` y `metadata.schema` de lo que declara;
//! 3. lo que lo nombra en TRES partes desde fuera (`ventas.viejo.x`), en los
//!    `.yaml` y en los `.sql` del árbol; el código (`.py`, `.java`…) no se
//!    toca: se dice, porque reescribir el programa de alguien no es renombrar;
//! 4. los punteros (`datasets/<paquete>/<schema>/`): los bytes del lago no se
//!    mueven —el puntero dice dónde están, y eso sigue siendo cierto—;
//! 5. el `moved` en el manifiesto, uno por nombre: sin él `ore diff` ve veinte
//!    `OOS5007` (cada `Entity` y cada `View` «borradas»); con él, un `minor`.
//!
//! Y una sexta que no es del árbol compilado: el **alcance**. Lo descubierto
//! vive en la carpeta del schema del ORIGEN, y la siguiente inducción —`review`,
//! `model`, `copy`— lo volvería a emitir allí: dos carpetas con las mismas
//! tablas. El alcance guarda el nombre nuevo (`schemas: {origen: nuevo}`) y el
//! inductor lo aplica (`Regla::schemas`).
//!
//! # La puerta
//!
//! Los dos verbos pasan por la de siempre: **el árbol no empeora**. Si la
//! compilación de después tiene un diagnóstico que la de antes no tenía, se
//! deshace todo y sale 65 con los nuevos. Para el renombrado, lo de antes se
//! compara YA TRADUCIDO: un `OOS2010` sobre `ventas.viejo.X` que ya estaba es
//! el mismo defecto que sobre `ventas.nuevo.X`, no uno nuevo.
//!
//! Salidas: 0 bien · 65 no vale (nombre, `default`, empeora) · 66 no hay
//! (paquete o schema) · 73 choca (ya existe, o un catálogo sin alcance).

use ore_core::json::Json;
use ore_core::normalize::{SCHEMA_POR_DEFECTO, corto};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const NO_VALE: u8 = 65;
const NO_HAY: u8 = 66;
const CHOCA: u8 = 73;

/// Las carpetas de kind (`tables/`, `views/`…): un schema que se llamara así
/// sería indistinguible de ellas en la ruta de un documento.
const CARPETAS_DE_KIND: &[&str] = &[
    "tables",
    "views",
    "datasets",
    "entities",
    "functions",
    "actions",
    "models",
    "interfaces",
    "concepts",
];

/// Lo que se lee como código y no se reescribe: se dice.
const CODIGO: &[&str] = &[
    "py", "ipynb", "java", "scala", "kt", "mjs", "js", "ts", "r", "sc",
];

fn falla(codigo: u8, mensaje: impl std::fmt::Display, ayuda: &[&str]) -> ExitCode {
    eprintln!("error: {mensaje}");
    for a in ayuda {
        eprintln!("  {a}");
    }
    ExitCode::from(codigo)
}

/// Un nombre de schema: un identificador de OOS (`^[a-zA-Z][a-zA-Z0-9_]*$`,
/// hasta 128), que no es `default` ni `information_schema` ni una carpeta de
/// kind.
fn nombre_valido(n: &str) -> Result<(), String> {
    let mut cs = n.chars();
    let forma = n.len() <= 128
        && cs.next().is_some_and(|c| c.is_ascii_alphabetic())
        && cs.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !forma {
        return Err(format!(
            "`{n}` no puede ser un schema: una letra y luego letras, cifras o `_` (hasta 128)"
        ));
    }
    let bajo = n.to_ascii_lowercase();
    if ore_core::schema::RESERVADOS.contains(&bajo.as_str()) {
        return Err(format!(
            "`{n}` está reservado: `default` existe sin declararse e `information_schema` es de SQL"
        ));
    }
    if CARPETAS_DE_KIND.contains(&bajo.as_str()) {
        return Err(format!(
            "`{n}` es una carpeta de kind: `<paquete>/{n}/` ya significa otra cosa"
        ));
    }
    Ok(())
}

/// El paquete: su carpeta y su manifiesto.
fn paquete_de(raiz: &Path, paquete: &str) -> Result<(PathBuf, PathBuf), ExitCode> {
    let dir = raiz.join("packages").join(paquete);
    let manifiesto = dir.join("package.yaml");
    if paquete.is_empty() || paquete.contains(['/', '\\', '.']) || !manifiesto.is_file() {
        return Err(falla(
            NO_HAY,
            format!("no hay paquete `{paquete}`"),
            &["ore package new <nombre> --owner <handle>"],
        ));
    }
    Ok((dir, manifiesto))
}

/// Lo que hay en la carpeta del paquete con ese nombre, sin mirar mayúsculas:
/// dos schemas que sólo difieren en ellas son el mismo para SQL.
fn ocupado(dir: &Path, n: &str) -> Option<String> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .find(|f| f.eq_ignore_ascii_case(n))
}

/// Una cadena como escalar entre comillas dobles de YAML.
fn comillas(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '"' => o.push_str("\\\""),
            '\n' => o.push_str("\\n"),
            '\r' => {}
            '\t' => o.push_str("\\t"),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// El documento de un schema (v1alpha13 01 §2).
fn documento(
    nombre: &str,
    paquete: &str,
    descripcion: Option<&str>,
    owner: Option<&str>,
) -> String {
    let mut t = format!(
        "apiVersion: oos.dev/v1alpha13\nkind: Schema\nmetadata:\n  name: {nombre}\n  namespace: {paquete}\n"
    );
    if let Some(d) = descripcion.map(str::trim).filter(|d| !d.is_empty()) {
        t.push_str(&format!("  description: {}\n", comillas(d)));
    }
    t.push_str(&format!(
        "# Lo que vive en esta carpeta se llama `{paquete}.{nombre}.<nombre>`.\n"
    ));
    if let Some(o) = owner {
        t.push_str(&format!("spec:\n  owner: {}\n", comillas(o)));
    }
    t
}

// ── La puerta ───────────────────────────────────────────────────────────────

/// Los diagnósticos del árbol como (código, mensaje): dos son el mismo defecto
/// si coinciden en eso, esté donde esté (lo mismo que `ore-serve`).
fn diagnosticos(raiz: &Path) -> Vec<(String, String, String)> {
    ore_core::validate_package(raiz)
        .into_iter()
        .map(|d| {
            let donde = d
                .render(raiz)
                .lines()
                .nth(1)
                .unwrap_or_default()
                .trim()
                .to_string();
            (d.code.to_string(), d.message, donde)
        })
        .collect()
}

/// Los de `despues` que no estaban en `antes` (traducidos con `traducir`).
fn nuevos(
    antes: &[(String, String, String)],
    despues: Vec<(String, String, String)>,
    traducir: impl Fn(&str) -> String,
) -> Vec<(String, String, String)> {
    let habia: BTreeSet<(String, String)> = antes
        .iter()
        .map(|(c, m, _)| (c.clone(), traducir(m)))
        .collect();
    despues
        .into_iter()
        .filter(|(c, m, _)| !habia.contains(&(c.clone(), m.clone())))
        .collect()
}

fn decir_nuevos(que: &str, nuevos: &[(String, String, String)]) -> ExitCode {
    eprintln!("error: el árbol empeora con {que}: nada se ha escrito");
    for (c, m, donde) in nuevos.iter().take(8) {
        eprintln!("  {c}: {m}");
        if !donde.is_empty() {
            eprintln!("    {donde}");
        }
    }
    if nuevos.len() > 8 {
        eprintln!("  … y {} más", nuevos.len() - 8);
    }
    ExitCode::from(NO_VALE)
}

// ── Crear ───────────────────────────────────────────────────────────────────

/// **`ore package schema new <paquete> <nombre>`**.
pub fn nuevo(
    raiz: &Path,
    paquete: &str,
    nombre: &str,
    descripcion: Option<&str>,
    owner: Option<&str>,
    json: bool,
) -> ExitCode {
    let (dir, _) = match paquete_de(raiz, paquete) {
        Ok(d) => d,
        Err(c) => return c,
    };
    if let Err(m) = nombre_valido(nombre) {
        return falla(NO_VALE, m, &[]);
    }
    if let Some(o) = owner
        && !ore_core::pertenencia::es_handle(o)
    {
        return falla(
            NO_VALE,
            format!("`{o}` no es un handle"),
            &["`team:<handle>` o `user:<handle>`; sin `--owner` responde el dueño del paquete"],
        );
    }
    // Ya declarado: en su carpeta, o en cualquier otra (mal puesto, pero suyo).
    let pkg = ore_core::validate::cargar_paquete(raiz).0;
    let ya = pkg
        .docs
        .iter()
        .filter(|d| d.kind == ore_core::document::Kind::Schema)
        .any(|s| {
            s.meta("namespace").and_then(|n| n.as_str()) == Some(paquete)
                && s.meta("name")
                    .and_then(|n| n.as_str())
                    .is_some_and(|n| n.eq_ignore_ascii_case(nombre))
        });
    let carpeta = match ocupado(&dir, nombre) {
        Some(f) if ya || dir.join(&f).join("schema.yaml").is_file() => {
            return falla(
                CHOCA,
                format!("ya hay un schema `{f}` en `{paquete}`"),
                &["dos schemas que sólo difieren en mayúsculas son el mismo para SQL"],
            );
        }
        // Una carpeta que ya está (0034 ④, o lo de un schema sin declarar):
        // se adopta, con el nombre que ya tiene en disco.
        Some(f) if f == nombre => dir.join(&f),
        Some(f) => {
            return falla(
                CHOCA,
                format!("ya hay `{f}` en `{paquete}`, y sólo difiere en mayúsculas de `{nombre}`"),
                &[],
            );
        }
        None if ya => {
            return falla(
                CHOCA,
                format!("`{paquete}` ya declara un schema `{nombre}` (fuera de su carpeta)"),
                &["`ore validate` lo señala con OOS2036"],
            );
        }
        None => dir.join(nombre),
    };
    let fichero = carpeta.join("schema.yaml");
    let creada = !carpeta.exists();
    let antes = diagnosticos(raiz);
    if let Err(e) = std::fs::create_dir_all(&carpeta)
        .and_then(|_| std::fs::write(&fichero, documento(nombre, paquete, descripcion, owner)))
    {
        return falla(
            70,
            format!("no se pudo escribir `{}`: {e}", fichero.display()),
            &[],
        );
    }
    let malos = nuevos(&antes, diagnosticos(raiz), str::to_string);
    if !malos.is_empty() {
        let _ = std::fs::remove_file(&fichero);
        if creada {
            let _ = std::fs::remove_dir_all(&carpeta);
        }
        return decir_nuevos(&format!("el schema `{paquete}.{nombre}`"), &malos);
    }
    let rel = relativo(raiz, &fichero);
    if json {
        println!(
            "{}",
            Json::obj([
                ("package", Json::s(paquete)),
                ("schema", Json::s(nombre)),
                ("fichero", Json::s(&rel)),
                ("adoptada", Json::Bool(!creada)),
            ])
            .pretty()
        );
    } else {
        println!("  schema `{paquete}.{nombre}` · {rel}");
        if !creada {
            println!(
                "  (la carpeta ya estaba: lo de dentro en v1alpha13 ya puede declarar `schema: {nombre}`)"
            );
        }
    }
    ExitCode::SUCCESS
}

// ── Renombrar ───────────────────────────────────────────────────────────────

fn relativo(raiz: &Path, p: &Path) -> String {
    p.strip_prefix(raiz)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

fn es_de_nombre(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// **Reapunta `<paquete>.<viejo>.x` a `<paquete>.<nuevo>.x`** en un texto: las
/// referencias de tres partes, y sólo ellas. Es texto y no posición a
/// propósito: una de tres partes no se confunde con nada —ni la forma corta ni
/// un nombre suelto la contienen— y así llega igual a un `.sql`, a un
/// `exports` o a un `writes: p.s.Entidad.propiedad`. En un manifiesto, lo que
/// sigue a `from:` es la historia (`moved`) y no se toca.
fn reapuntar(
    texto: &str,
    paquete: &str,
    viejo: &str,
    nuevo: &str,
    manifiesto: bool,
) -> (String, usize) {
    let aguja = format!("{paquete}.{viejo}.");
    let mut out = String::with_capacity(texto.len());
    let mut n = 0;
    let mut resto = texto;
    let mut previo: Option<char> = None;
    while let Some(i) = resto.find(&aguja) {
        let antes = &resto[..i];
        let delante = antes.chars().next_back().or(previo);
        let detras = resto[i + aguja.len()..].chars().next();
        let suelto = !delante.is_some_and(|c| es_de_nombre(c) || c == '.')
            && detras.is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
        let historia = manifiesto && {
            let hasta: String = out.clone() + antes;
            hasta.trim_end().ends_with("from:")
        };
        out.push_str(antes);
        if suelto && !historia {
            out.push_str(&format!("{paquete}.{nuevo}."));
            n += 1;
        } else {
            out.push_str(&aguja);
        }
        previo = aguja.chars().next_back();
        resto = &resto[i + aguja.len()..];
    }
    out.push_str(resto);
    (out, n)
}

/// `metadata.schema` (y, en el `Schema`, `metadata.name`) de `viejo` a
/// `nuevo`, por POSICIÓN: `schema:` o `name:` pueden aparecer en otros sitios.
fn metadata_a(texto: &str, viejo: &str, nuevo: &str) -> String {
    let Ok(raiz) = ore_core::parse::parse(texto) else {
        return texto.to_string();
    };
    let es_schema = raiz.get("kind").and_then(|(_, k)| k.as_str()) == Some("Schema");
    let Some((_, m)) = raiz.get("metadata") else {
        return texto.to_string();
    };
    let mut lineas: Vec<String> = texto.lines().map(String::from).collect();
    let claves: &[&str] = if es_schema { &["name"] } else { &["schema"] };
    // De atrás adelante: una sustitución no mueve la posición de la anterior.
    let mut sitios: Vec<ore_core::diag::Pos> = claves
        .iter()
        .filter_map(|k| m.get(k))
        .filter(|(_, v)| v.as_str() == Some(viejo))
        .map(|(_, v)| v.pos())
        .collect();
    sitios.sort_by_key(|p| std::cmp::Reverse((p.line, p.col)));
    for p in sitios {
        if let Some((i, l)) = crate::paquete::sustituir_en(&lineas, p, nuevo) {
            lineas[i] = l;
        }
    }
    let mut s = lineas.join("\n");
    if texto.ends_with('\n') {
        s.push('\n');
    }
    s
}

/// Los ficheros de un directorio, recursivo, sin `.git` ni lo que se construye.
fn ficheros(dir: &Path, fuera: &[PathBuf], out: &mut Vec<PathBuf>) {
    let Ok(es) = std::fs::read_dir(dir) else {
        return;
    };
    for e in es.flatten() {
        let p = e.path();
        let nombre = e.file_name().to_string_lossy().into_owned();
        if fuera.iter().any(|f| f == &p) {
            continue;
        }
        if p.is_dir() {
            if matches!(nombre.as_str(), ".git" | "node_modules" | "target") {
                continue;
            }
            ficheros(&p, fuera, out);
        } else {
            out.push(p);
        }
    }
}

fn extension(p: &Path) -> String {
    p.extension()
        .and_then(|x| x.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// Un cambio del renombrado: lo que había y lo que habrá (`None` = no está).
type Cambios = BTreeMap<PathBuf, (Option<Vec<u8>>, Option<Vec<u8>>)>;

fn aplicar(cambios: &Cambios) -> Result<(), String> {
    for (p, (_, despues)) in cambios {
        if let Some(b) = despues {
            if let Some(d) = p.parent() {
                std::fs::create_dir_all(d).map_err(|e| format!("`{}`: {e}", d.display()))?;
            }
            std::fs::write(p, b).map_err(|e| format!("`{}`: {e}", p.display()))?;
        }
    }
    for (p, (_, despues)) in cambios {
        if despues.is_none() {
            std::fs::remove_file(p).map_err(|e| format!("`{}`: {e}", p.display()))?;
        }
    }
    Ok(())
}

fn deshacer(cambios: &Cambios) {
    for (p, (antes, _)) in cambios {
        match antes {
            Some(b) => {
                if let Some(d) = p.parent() {
                    let _ = std::fs::create_dir_all(d);
                }
                let _ = std::fs::write(p, b);
            }
            None => {
                let _ = std::fs::remove_file(p);
            }
        }
    }
}

/// Retira los directorios que quedaron vacíos bajo `dir` (y `dir`).
fn podar(dir: &Path) {
    if let Ok(es) = std::fs::read_dir(dir) {
        for e in es.flatten() {
            if e.path().is_dir() {
                podar(&e.path());
            }
        }
    }
    let _ = std::fs::remove_dir(dir);
}

/// **`ore package schema rename <paquete> <viejo> <nuevo>`**.
pub fn renombrar(
    raiz: &Path,
    paquete: &str,
    viejo: &str,
    nuevo: &str,
    since: Option<&str>,
    json: bool,
) -> ExitCode {
    let (dir, manifiesto) = match paquete_de(raiz, paquete) {
        Ok(d) => d,
        Err(c) => return c,
    };
    if viejo == SCHEMA_POR_DEFECTO {
        return falla(
            NO_VALE,
            "`default` no se renombra: es donde está lo que no dice otro schema",
            &["crea el schema nuevo y mueve a él lo que quieras de `default`"],
        );
    }
    if let Err(m) = nombre_valido(nuevo) {
        return falla(NO_VALE, m, &[]);
    }
    if viejo.eq_ignore_ascii_case(nuevo) {
        return falla(
            NO_VALE,
            format!("`{viejo}` y `{nuevo}` son el mismo schema para SQL (sólo cambian mayúsculas)"),
            &[],
        );
    }
    let carpeta = dir.join(viejo);
    let declarado = std::fs::read_to_string(carpeta.join("schema.yaml"))
        .ok()
        .and_then(|t| ore_core::parse::parse(&t).ok())
        .and_then(|n| {
            n.get("metadata")
                .and_then(|(_, m)| m.get("name"))
                .and_then(|(_, v)| v.as_str().map(String::from))
        });
    if declarado.as_deref() != Some(viejo) {
        return falla(
            NO_HAY,
            format!("`{paquete}` no declara el schema `{viejo}`"),
            &[&format!(
                "un schema es su `packages/{paquete}/{viejo}/schema.yaml`"
            )],
        );
    }
    if let Some(f) = ocupado(&dir, nuevo) {
        return falla(CHOCA, format!("ya hay `{f}` en `{paquete}`"), &[]);
    }
    let punteros_viejos = raiz.join("datasets").join(paquete).join(viejo);
    let punteros_nuevos = raiz.join("datasets").join(paquete).join(nuevo);
    if punteros_nuevos.exists() {
        return falla(
            CHOCA,
            format!("ya hay punteros en `{}`", relativo(raiz, &punteros_nuevos)),
            &[],
        );
    }
    // El alcance: lo descubierto volvería a su carpeta del origen en la
    // siguiente inducción si la regla no lo dijera.
    let alcance = match crate::alcance::del_paquete(&dir) {
        Ok(a) => a,
        Err(m) => return falla(NO_VALE, m, &[]),
    };
    if alcance.is_none() && dir.join("discover.catalog.json").is_file() {
        return falla(
            CHOCA,
            format!("`{paquete}` es el catálogo entero de una fuente, sin alcance"),
            &[
                "lo re-induce su Job de catálogo tal como el origen lo nombra, y el nombre nuevo",
                "no sobreviviría a la siguiente pasada. Renombra en una base (`POST /paquetes`).",
            ],
        );
    }

    // ── el plan ─────────────────────────────────────────────────────────────
    let mut cambios: Cambios = BTreeMap::new();
    let mut movidos = 0usize;
    let mut punteros = 0usize;
    let mut reapuntados: BTreeMap<String, usize> = BTreeMap::new();
    let mut a_mano: Vec<String> = Vec::new();
    let leer = |p: &Path| std::fs::read(p).ok();

    // ① y ② lo de dentro, a su carpeta nueva: la metadata y lo que nombre en
    //    tres partes (lo suyo en una parte viaja sin tocarlo)
    let traducir = |f: &Path,
                    bytes: Vec<u8>,
                    reapuntados: &mut BTreeMap<String, usize>,
                    a_mano: &mut Vec<String>,
                    dentro: bool|
     -> Vec<u8> {
        let ext = extension(f);
        let Ok(t) = String::from_utf8(bytes.clone()) else {
            return bytes;
        };
        match ext.as_str() {
            "yaml" | "yml" | "sql" => {
                let t = if dentro && ext != "sql" {
                    metadata_a(&t, viejo, nuevo)
                } else {
                    t
                };
                let (t, n) = reapuntar(&t, paquete, viejo, nuevo, f.ends_with("package.yaml"));
                if n > 0 {
                    reapuntados.insert(relativo(raiz, f), n);
                }
                t.into_bytes()
            }
            e if CODIGO.contains(&e) && t.contains(&format!("{paquete}.{viejo}.")) => {
                a_mano.push(relativo(raiz, f));
                bytes
            }
            _ => bytes,
        }
    };
    let mut dentro = Vec::new();
    ficheros(&carpeta, &[], &mut dentro);
    for f in dentro {
        let Some(bytes) = leer(&f) else { continue };
        let destino = dir.join(nuevo).join(f.strip_prefix(&carpeta).unwrap_or(&f));
        let otros = traducir(&destino, bytes.clone(), &mut reapuntados, &mut a_mano, true);
        cambios.insert(f, (Some(bytes), None));
        cambios.insert(destino, (None, Some(otros)));
        movidos += 1;
    }

    // ④ los punteros, a su sitio nuevo (lo que nombran en tres partes también)
    let mut suyos = Vec::new();
    ficheros(&punteros_viejos, &[], &mut suyos);
    for f in suyos {
        let Some(bytes) = leer(&f) else { continue };
        let destino = punteros_nuevos.join(f.strip_prefix(&punteros_viejos).unwrap_or(&f));
        let otros = match String::from_utf8(bytes.clone()) {
            Ok(t) => reapuntar(&t, paquete, viejo, nuevo, false).0.into_bytes(),
            Err(_) => bytes.clone(),
        };
        cambios.insert(f, (Some(bytes), None));
        cambios.insert(destino, (None, Some(otros)));
        punteros += 1;
    }

    // ③ lo que lo nombra desde fuera: el resto del árbol
    let mut resto = Vec::new();
    ficheros(
        raiz,
        &[carpeta.clone(), punteros_viejos.clone()],
        &mut resto,
    );
    for f in resto {
        let ext = extension(&f);
        if !matches!(ext.as_str(), "yaml" | "yml" | "sql") && !CODIGO.contains(&ext.as_str()) {
            continue;
        }
        let Some(bytes) = leer(&f) else { continue };
        let otros = traducir(&f, bytes.clone(), &mut reapuntados, &mut a_mano, false);
        if otros != bytes {
            cambios.insert(f, (Some(bytes), Some(otros)));
        }
    }

    // ⑤ el anuncio: un `moved` por cada nombre del schema, en el manifiesto
    let pkg = ore_core::validate::cargar_paquete(raiz).0;
    let nombres: BTreeSet<String> = pkg
        .docs
        .iter()
        .filter(|d| d.path.starts_with(&carpeta))
        .filter(|d| d.kind.con_schema())
        .filter(|d| {
            d.version()
                .is_some_and(|v| v >= ore_core::document::ApiVersion::V1Alpha13)
        })
        .filter(|d| d.schema() == Some(viejo))
        .filter_map(|d| d.meta("name").and_then(|n| n.as_str()).map(String::from))
        .collect();
    let version = pkg
        .docs
        .iter()
        .find(|d| d.kind == ore_core::document::Kind::Package && d.path == manifiesto)
        .and_then(|d| d.meta("version").and_then(|v| v.as_str()).map(String::from))
        .unwrap_or_else(|| "0.1.0".into());
    let desde = since.map(String::from).unwrap_or(version);
    let original = match leer(&manifiesto) {
        Some(b) => b,
        None => {
            return falla(
                70,
                format!("no se pudo leer `{}`", manifiesto.display()),
                &[],
            );
        }
    };
    let mut texto = match cambios.get(&manifiesto) {
        Some((_, Some(b))) => String::from_utf8_lossy(b).into_owned(),
        _ => String::from_utf8_lossy(&original).into_owned(),
    };
    for n in &nombres {
        texto = match crate::paquete::anunciar(
            &texto,
            &corto(paquete, viejo, n),
            &corto(paquete, nuevo, n),
            &desde,
        ) {
            Ok(t) => t,
            Err(m) => return falla(NO_VALE, format!("el manifiesto: {m}"), &[]),
        };
    }
    if texto.as_bytes() != original.as_slice() {
        cambios.insert(
            manifiesto.clone(),
            (Some(original), Some(texto.into_bytes())),
        );
    }

    // ⑥ la regla del alcance
    if let Some(mut a) = alcance {
        a.renombrar_schema(viejo, nuevo);
        let r = crate::alcance::ruta(&dir);
        cambios.insert(r.clone(), (leer(&r), Some(a.escribir().into_bytes())));
    }

    // ── la puerta ───────────────────────────────────────────────────────────
    let antes = diagnosticos(raiz);
    if let Err(m) = aplicar(&cambios) {
        deshacer(&cambios);
        return falla(70, format!("no se pudo escribir: {m}"), &[]);
    }
    let (de_a, de_b) = (format!("{paquete}.{viejo}."), format!("{paquete}.{nuevo}."));
    let (sch_a, sch_b) = (format!("`{viejo}`"), format!("`{nuevo}`"));
    let malos = nuevos(&antes, diagnosticos(raiz), |m| {
        m.replace(&de_a, &de_b).replace(&sch_a, &sch_b)
    });
    if !malos.is_empty() {
        deshacer(&cambios);
        podar(&dir.join(nuevo));
        podar(&punteros_nuevos);
        return decir_nuevos(
            &format!("renombrar `{paquete}.{viejo}` a `{paquete}.{nuevo}`"),
            &malos,
        );
    }
    podar(&carpeta);
    podar(&punteros_viejos);

    a_mano.sort();
    a_mano.dedup();
    if json {
        println!(
            "{}",
            Json::obj([
                ("package", Json::s(paquete)),
                ("from", Json::s(viejo)),
                ("to", Json::s(nuevo)),
                ("movidos", Json::Int(movidos as i64)),
                ("punteros", Json::Int(punteros as i64)),
                (
                    "reapuntados",
                    Json::Arr(reapuntados.keys().map(Json::s).collect()),
                ),
                ("anunciados", Json::Int(nombres.len() as i64)),
                ("aMano", Json::Arr(a_mano.iter().map(Json::s).collect())),
            ])
            .pretty()
        );
        return ExitCode::SUCCESS;
    }
    println!("  `{paquete}.{viejo}` → `{paquete}.{nuevo}`");
    println!("  {movidos} fichero(s) a `packages/{paquete}/{nuevo}/`");
    if punteros > 0 {
        println!(
            "  {punteros} puntero(s) a `datasets/{paquete}/{nuevo}/` (los bytes del lago no se mueven)"
        );
    }
    if !reapuntados.is_empty() {
        println!(
            "  {} referencia(s) reapuntada(s) en {} fichero(s):",
            reapuntados.values().sum::<usize>(),
            reapuntados.len()
        );
        for (f, n) in &reapuntados {
            println!("    {f}  ·  {n}");
        }
    }
    if !nombres.is_empty() {
        println!(
            "  {} nombre(s) anunciado(s) en `moved` (desde {desde}): `ore diff` lo ve como un renombrado",
            nombres.len()
        );
    }
    if !a_mano.is_empty() {
        println!("  ⚠ código que lo nombra y NO se ha tocado:");
        for f in &a_mano {
            println!("    {f}");
        }
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn se_reapunta_lo_de_tres_partes_y_nada_mas() {
        let t = "from: { table: ventas.viejo.clientes }\n\
                 otro: xventas.viejo.a, ventas.viejo2.b, ventas.viejo., ventas.viejo.c\n\
                 sql: select * from ventas.viejo.pedidos join ventas.nuevo.x\n";
        let (s, n) = reapuntar(t, "ventas", "viejo", "nuevo", false);
        assert_eq!(n, 3, "{s}");
        assert!(s.contains("table: ventas.nuevo.clientes"), "{s}");
        assert!(s.contains("xventas.viejo.a"), "{s}");
        assert!(s.contains("ventas.viejo2.b"), "{s}");
        assert!(s.contains("ventas.viejo., ventas.nuevo.c"), "{s}");
        assert!(s.contains("from ventas.nuevo.pedidos"), "{s}");
    }

    #[test]
    fn en_el_manifiesto_la_historia_no_se_toca() {
        let t = "spec: { owner: \"team:v\", exports: [ventas.viejo.x], moved: [{ from: ventas.viejo.y, to: ventas.viejo.z, since: 0.1.0 }] }\n";
        let (s, n) = reapuntar(t, "ventas", "viejo", "nuevo", true);
        assert_eq!(n, 2, "{s}");
        assert!(s.contains("exports: [ventas.nuevo.x]"), "{s}");
        assert!(s.contains("from: ventas.viejo.y"), "{s}");
        assert!(s.contains("to: ventas.nuevo.z"), "{s}");
    }

    #[test]
    fn la_metadata_por_posicion() {
        let v = "apiVersion: oos.dev/v1alpha13\nkind: View\nmetadata: { name: viejo, namespace: ventas, schema: viejo }\nspec:\n  schema: viejo\n";
        let s = metadata_a(v, "viejo", "nuevo");
        assert!(
            s.contains("{ name: viejo, namespace: ventas, schema: nuevo }"),
            "{s}"
        );
        assert!(s.contains("  schema: viejo\n"), "{s}");
        let d = "apiVersion: oos.dev/v1alpha13\nkind: Schema\nmetadata:\n  name: viejo\n  namespace: ventas\n";
        let s = metadata_a(d, "viejo", "nuevo");
        assert!(s.contains("  name: nuevo\n"), "{s}");
    }

    #[test]
    fn los_nombres_que_no_pueden_ser_un_schema() {
        assert!(nombre_valido("espana").is_ok());
        assert!(nombre_valido("Ventas_2").is_ok());
        for n in [
            "",
            "1x",
            "a-b",
            "a.b",
            "default",
            "DEFAULT",
            "information_schema",
            "tables",
            "views",
        ] {
            assert!(nombre_valido(n).is_err(), "{n}");
        }
    }

    #[test]
    fn el_documento_compila_con_y_sin_descripcion() {
        let d = documento(
            "espana",
            "ventas",
            Some("Lo de \"España\"\ny más"),
            Some("team:v"),
        );
        let n = ore_core::parse::parse(&d).expect("analiza");
        let (_, m) = n.get("metadata").unwrap();
        assert_eq!(
            m.get("description").and_then(|(_, v)| v.as_str()),
            Some("Lo de \"España\"\ny más")
        );
        assert!(documento("espana", "ventas", None, None).ends_with(".<nombre>`.\n"));
    }
}
