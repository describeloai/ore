//! **El catálogo de un bucket**: qué hay, agrupado como lo nombrará el árbol.
//!
//! Dos cosas salen de un bucket (spec v1alpha16 `00-scope` §2):
//!
//! - **tablas**, de los ficheros que son filas (Parquet, CSV, JSONL): lo que
//!   `ore source induce` escribirá como una `Table` con `format`;
//! - **conjuntos de objetos**, de lo que no lo es (documentos, imágenes…): lo
//!   que escribirá como un `ObjectTable`, por carpeta y tipo de medio.
//!
//! # La regla de una tabla, medida en F1
//!
//! «Un prefijo es una tabla» no aguanta un bucket real: en `Nueva carpeta/`
//! había diez CSV sueltos con esquemas distintos. Así que:
//!
//! - una carpeta con particiones Hive (`fecha=…`) es **una** tabla, con las
//!   claves del camino como columnas;
//! - una carpeta con varios ficheros del mismo formato es una tabla si
//!   **comparten esquema** (en JSONL, siempre: sus claves se unen); si no,
//!   **cada fichero es la suya**;
//! - un fichero solo es su tabla.
//!
//! # Lo que se lee
//!
//! Un listado, y por grupo los primeros bytes de un par de objetos para
//! confirmar el tipo; el pie de los Parquet; 64 KB de los CSV y JSONL. Medido
//! en F1: 668 KB leídos para 171 MB. Nunca un fichero entero.
//!
//! # El nombre
//!
//! `<schema>.<nombre>`, como en las demás familias: el schema es la primera
//! carpeta (o `default` en la raíz), el nombre la carpeta o el fichero, en
//! identificador. El objeto de verdad —con sus espacios y sus mayúsculas— va
//! aparte, en `object` o en `prefix`. Dos que se llamarían igual se
//! distinguen con `_2`, aquí: el nombre es la clave del catálogo. Un nombre
//! mide como mucho [`LARGO`] (lo que admite el resto de ORE, 64, con sitio para
//! el `_2`). El fichero que escribe Spark (`part-00066-tid-…-c000.snappy.parquet`)
//! da su carpeta como nombre, o `part_00066` en la raíz: su nombre entero no
//! dice nada.
//!
//! # Las filas (ORE 0062)
//!
//! Cada tabla dice cuántas filas tiene si se **contaron** (`rows`: el pie de
//! un Parquet, un CSV que cabe en la muestra) o cuántas **se estiman**
//! (`rowsEstimate`: un CSV grande, por los bytes de su muestra). Una tabla de
//! varios ficheros suma lo de los que se leyeron, y si no se leyeron todos
//! extrapola por bytes: entonces es estimada aunque cada pie fuera exacto.
//!
//! # Una carpeta no es un objeto
//!
//! Un objeto vacío cuyo nombre es la carpeta de otros (`processedManifest`,
//! junto a `processedManifest/manifest.json`) es el marcador que dejan algunas
//! herramientas al crear una carpeta (medido en el contenedor `mnist` de Azure
//! Open Datasets). No es dato: se salta, como el que acaba en `/`.
//!
//! # Lo oculto no es dato
//!
//! Lo que empieza por `_` o por `.` —el fichero o una de sus carpetas— es lo
//! que Hadoop, Spark y Delta dejan al escribir (`_SUCCESS`, `_committed_…`,
//! `_started_…`, `_temporary/`, `.crc`) y lo que sus lectores saltan. Aquí
//! también: ni tabla ni objeto, y un aviso dice cuántos.

use crate::medio::{self, Clase, Formato};
use crate::tabular::{self, Esquema};
use ore_base::document::COLUMNA_RESCATADA as RESCATADA;
use ore_base::json::Json;
use ore_driver::catalogo::{Catalogo, Columna, Objetos, Tabla};
use ore_objetos::Objeto;
use ore_objetos::Origen;
use std::collections::{BTreeMap, BTreeSet};

/// Los bytes que se leen para confirmar el tipo: [`medio::CABEZA`].
fn rango_de_la_cabeza() -> String {
    format!("0-{}", medio::CABEZA - 1)
}

/// Cuántos objetos de un grupo se abren para confirmar su tipo.
const CONFIRMAR: usize = 2;
/// Cuántos sin extensión se abren, como mucho, para saber qué son.
const SIN_EXTENSION: usize = 100;
/// Cuántos ficheros de una carpeta se comparan para decidir si son una tabla.
const COMPARAR: usize = 20;
/// Lo que mide un identificador, como mucho: 64 menos el sitio de un `_NN`.
pub const LARGO: usize = 60;

/// Un identificador de OOS: minúsculas, `_` por lo demás, sin empezar por
/// dígito. `Nueva carpeta` → `nueva_carpeta`; `Foto Portada 2026` →
/// `foto_portada_2026`.
pub fn identificador(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        let c = match c {
            'á' | 'à' | 'ä' | 'â' | 'Á' => 'a',
            'é' | 'è' | 'ë' | 'ê' | 'É' => 'e',
            'í' | 'ì' | 'ï' | 'î' | 'Í' => 'i',
            'ó' | 'ò' | 'ö' | 'ô' | 'Ó' => 'o',
            'ú' | 'ù' | 'ü' | 'û' | 'Ú' => 'u',
            'ñ' | 'Ñ' => 'n',
            c => c,
        };
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    let out = out.trim_matches('_').to_string();
    let out = match out.chars().next() {
        None => "x".into(),
        Some(c) if c.is_ascii_digit() => format!("x_{out}"),
        Some(_) => out,
    };
    // Sólo ASCII a estas alturas: cortar por bytes es cortar por caracteres.
    out[..out.len().min(LARGO)]
        .trim_end_matches('_')
        .to_string()
}

/// Lo que Hadoop, Spark y Delta dejan al escribir y sus lectores saltan: un
/// segmento del camino que empieza por `_` o por `.`.
fn oculto(rel: &str) -> bool {
    rel.split('/')
        .any(|p| p.starts_with('_') || p.starts_with('.'))
}

/// `part-00066-tid-…-c000.snappy.parquet` → `Some("00066")`: un fichero de Spark.
fn parte_de_spark(base: &str) -> Option<&str> {
    let resto = base.strip_prefix("part-")?;
    let n = resto.split(['-', '.']).next()?;
    (!n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())).then_some(n)
}

/// Un objeto, ya situado y clasificado.
struct Item<'a> {
    obj: &'a Objeto,
    /// La clave relativa al prefijo de la fuente.
    rel: String,
    /// Su carpeta relativa, con barra final; `""` en la raíz.
    dir: String,
    ext: String,
    clase: Clase,
}

/// `(raíz de la tabla, claves)` si la carpeta tiene particiones Hive.
fn hive(dir: &str) -> Option<(String, Vec<String>)> {
    let partes: Vec<&str> = dir.trim_end_matches('/').split('/').collect();
    let es_k_v = |p: &str| {
        p.split_once('=').is_some_and(|(k, v)| {
            !k.is_empty()
                && !v.is_empty()
                && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
    };
    let i = partes.iter().position(|p| es_k_v(p))?;
    let claves: Vec<String> = partes[i..]
        .iter()
        .take_while(|p| es_k_v(p))
        .map(|p| {
            p.split_once('=')
                .map(|(k, _)| k.to_string())
                .unwrap_or_default()
        })
        .collect();
    let raiz = if i == 0 {
        String::new()
    } else {
        format!("{}/", partes[..i].join("/"))
    };
    Some((raiz, claves))
}

/// El schema de algo que vive en `dir` (relativa): su primera carpeta, o
/// `default` en la raíz.
fn schema_de(dir: &str) -> String {
    match dir.split('/').find(|p| !p.is_empty()) {
        Some(p) => identificador(p),
        None => "default".into(),
    }
}

fn ultima(dir: &str) -> Option<&str> {
    dir.trim_end_matches('/')
        .rsplit('/')
        .find(|p| !p.is_empty())
}

fn tallo(rel: &str) -> String {
    let base = rel.rsplit('/').next().unwrap_or(rel);
    // Un fichero de Spark se llama como su carpeta; en la raíz, `part_<n>`.
    if let Some(n) = parte_de_spark(base) {
        return match ultima(&rel[..rel.len() - base.len()]) {
            Some(d) => identificador(d),
            None => format!("part_{n}"),
        };
    }
    let t = base.rsplit_once('.').map(|(a, _)| a).unwrap_or(base);
    identificador(t)
}

/// Nombres únicos por schema, sin distinguir mayúsculas.
#[derive(Default)]
struct Nombres(BTreeSet<(String, String)>);

impl Nombres {
    fn dar(&mut self, schema: &str, base: &str) -> String {
        let mut n = base.to_string();
        let mut i = 2;
        while !self.0.insert((schema.to_string(), n.to_lowercase())) {
            n = format!("{base}_{i}");
            i += 1;
        }
        format!("{schema}.{n}")
    }
}

fn cara_d() -> Json {
    Json::obj([
        ("mode", Json::s("retract")),
        ("witness", Json::s("listing")),
    ])
}

fn cara_i(bytes: u64) -> Json {
    // Leer un conjunto entero cuesta lo que pesa: por encima de 256 MB el
    // planificador lo evita salvo necesidad.
    // Y lo que se empuja (0053 F5·3): el conector v2 filtra al leer el
    // fichero, con los diez operadores; se declara lo de siempre, sin `like`.
    Json::obj([
        (
            "fullScan",
            Json::s(if bytes > 256 * 1024 * 1024 {
                "expensive"
            } else {
                "cheap"
            }),
        ),
        (
            "predicatePushdown",
            Json::Arr(
                ore_driver::EMPUJE_INDUCIDO
                    .iter()
                    .map(|o| Json::s(*o))
                    .collect(),
            ),
        ),
    ])
}

fn esquema_de(o: &dyn Origen, f: Formato, obj: &Objeto, ext: &str) -> Result<Esquema, String> {
    match f {
        Formato::Parquet => tabular::parquet(o, &obj.clave),
        Formato::Csv => tabular::csv(o, &obj.clave, obj.tamano, ext == "tsv"),
        Formato::Jsonl => tabular::jsonl(o, &obj.clave, obj.tamano),
    }
}

fn firma(cols: &[Columna]) -> Vec<(String, Option<String>)> {
    cols.iter()
        .map(|c| (c.nombre.clone(), c.tipo.clone()))
        .collect()
}

/// La unión de las columnas de varios JSONL, en el orden en que aparecen. Una
/// que cambia de tipo entre ficheros se queda sin tipo y se cita.
fn unir(esquemas: &[Esquema]) -> Vec<Columna> {
    let mut out: Vec<Columna> = Vec::new();
    for e in esquemas {
        for c in &e.columnas {
            match out.iter_mut().find(|x| x.nombre == c.nombre) {
                None => out.push(c.clone()),
                Some(x) if x.tipo == c.tipo => {}
                Some(x) if x.tipo.is_none() => {}
                Some(x) => {
                    x.origen = Some(format!(
                        "{} en un fichero, {} en otro",
                        x.tipo.as_deref().unwrap_or("?"),
                        c.tipo.as_deref().unwrap_or("?")
                    ));
                    x.tipo = None;
                }
            }
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn tabla(
    nombre: String,
    objeto: String,
    formato: Formato,
    patron: Option<String>,
    particiones: &[String],
    mut esquema: Esquema,
    bytes: u64,
    avisos: &mut Vec<String>,
) -> Tabla {
    for p in particiones {
        if !esquema.columnas.iter().any(|c| &c.nombre == p) {
            esquema.columnas.push(Columna {
                nombre: p.clone(),
                tipo: Some("String".into()),
                ..Default::default()
            });
        }
    }
    // v1alpha16 `03` §1.1 (0046 E6): lo que se deduce de una muestra —un CSV,
    // un JSONL— trae su columna rescatada, y lo que no encaje con el tipo
    // deducido va ahí en vez de parar la copia. Un Parquet no: su tipo es el
    // del fichero.
    if matches!(formato, Formato::Csv | Formato::Jsonl)
        && !esquema.columnas.iter().any(|c| c.nombre == RESCATADA)
    {
        esquema.columnas.push(Columna {
            nombre: RESCATADA.into(),
            tipo: Some("String".into()),
            ..Default::default()
        });
    }
    avisos.append(&mut esquema.avisos);
    let mut f: Vec<(&str, Json)> = vec![("type", Json::s(formato.nombre()))];
    if let Some(p) = patron {
        f.push(("match", Json::s(p)));
    }
    if !particiones.is_empty() {
        f.push((
            "partitions",
            Json::Arr(particiones.iter().map(Json::s).collect()),
        ));
    }
    if let Some(s) = esquema.separador {
        f.push(("delimiter", Json::s(s.to_string())));
    }
    Tabla {
        nombre,
        columnas: esquema.columnas,
        filas: esquema.filas,
        filas_estimadas: esquema.filas_estimadas,
        clase: "table".into(),
        lee: Some(cara_i(bytes)),
        cambia: Some(cara_d()),
        objeto: Some(objeto),
        formato: Some(Json::obj(f)),
        ..Default::default()
    }
}

/// ⭐ 0062 · **Las filas de una tabla de varios ficheros**, de lo que se leyó
/// de ellos: `(tamaño, contadas, estimadas)` por fichero leído, de `todos`, que
/// pesan `bytes`. Contadas si se contaron TODOS; si no, estimadas: lo leído
/// (contado o estimado) extrapolado por bytes. Si algún leído no dice nada,
/// nada: una suma con un hueco no es una estimación.
fn filas_de_varios(
    leidos: &[(u64, Option<u64>, Option<u64>)],
    todos: usize,
    bytes: u64,
) -> (Option<u64>, Option<u64>) {
    if leidos.len() == todos && leidos.iter().all(|l| l.1.is_some()) {
        return (Some(leidos.iter().filter_map(|l| l.1).sum()), None);
    }
    let Some(filas) = leidos.iter().map(|l| l.1.or(l.2)).sum::<Option<u64>>() else {
        return (None, None);
    };
    let leido: u64 = leidos.iter().map(|l| l.0).sum();
    if leido == 0 {
        return (None, None);
    }
    let est = (filas as f64 * bytes as f64 / leido as f64).round() as u64;
    (None, Some(est))
}

/// **El catálogo.** `prefijo` es el de la fuente (vacío o con barra final).
pub fn leer(
    o: &dyn Origen,
    fuente: &str,
    prefijo: &str,
    avisos: &mut Vec<String>,
) -> Result<Catalogo, String> {
    let lista = o.listar(prefijo)?;
    // Una «carpeta» creada desde la consola es un objeto vacío acabado en `/`;
    // o uno vacío que se llama como la carpeta de otros (0062, `mnist`).
    let carpetas: BTreeSet<&str> = lista
        .iter()
        .flat_map(|x| x.clave.match_indices('/').map(|(i, _)| &x.clave[..i]))
        .collect();
    let lista: Vec<&Objeto> = lista
        .iter()
        .filter(|x| !x.clave.ends_with('/'))
        .filter(|x| !(x.tamano == 0 && carpetas.contains(x.clave.as_str())))
        .collect();
    let antes = lista.len();
    let lista: Vec<&Objeto> = lista
        .into_iter()
        .filter(|x| !oculto(&x.clave[prefijo.len().min(x.clave.len())..]))
        .collect();
    if lista.len() < antes {
        avisos.push(format!(
            "{} objeto(s) ocultos saltados (empiezan por `_` o `.`, como `_SUCCESS`: lo que dejan Spark y Hadoop al escribir)",
            antes - lista.len()
        ));
    }
    if lista.is_empty() && antes > 0 {
        return Err(format!(
            "bajo `{prefijo}` sólo hay objetos ocultos (empiezan por `_` o `.`): no hay datos que catalogar"
        ));
    }
    if lista.is_empty() {
        return Err(format!(
            "no hay ningún objeto bajo `{prefijo}`: o está vacío, o esta credencial no lo ve"
        ));
    }

    // ── 1 · qué es cada uno: la extensión propone, los bytes deciden ─────────
    let mut items: Vec<Item> = lista
        .iter()
        .map(|x| {
            let rel = x.clave[prefijo.len()..].to_string();
            let dir = match rel.rfind('/') {
                Some(i) => rel[..=i].to_string(),
                None => String::new(),
            };
            let ext = medio::extension(&rel);
            let clase = medio::por_extension(&ext).unwrap_or(Clase::Medio("binary"));
            Item {
                obj: x,
                rel,
                dir,
                ext,
                clase,
            }
        })
        .collect();
    let mut grupos: BTreeMap<(String, String), Vec<usize>> = BTreeMap::new();
    for (i, it) in items.iter().enumerate() {
        grupos
            .entry((it.dir.clone(), it.ext.clone()))
            .or_default()
            .push(i);
    }
    let mut sin_extension = 0usize;
    // ⭐ ADR 0061 O4·4 · Un fichero que no se deja leer (sin permiso, medido en
    //   un SFTP) es un aviso, no el fin del catálogo: su grupo se decide con
    //   los demás, o por su extensión. Si no se deja leer NINGUNO, sí se falla:
    //   eso es que falta el permiso, y `check` lo dice.
    let (mut intentos, mut primer_fallo): (usize, Option<String>) = (0, None);
    let mut fallos = 0usize;
    for ((_, ext), idx) in &grupos {
        if ext.is_empty() {
            for &i in idx {
                if sin_extension >= SIN_EXTENSION {
                    break;
                }
                sin_extension += 1;
                let cab = o
                    .rango(&items[i].obj.clave, &rango_de_la_cabeza())
                    .unwrap_or_default();
                items[i].clase = medio::confirmar("", &cab).0;
            }
            continue;
        }
        let mut decidida: Option<Clase> = None;
        for &i in idx.iter().take(CONFIRMAR) {
            if items[i].obj.tamano == 0 {
                continue;
            }
            intentos += 1;
            let cab = match o.rango(&items[i].obj.clave, &rango_de_la_cabeza()) {
                Ok(c) => c,
                Err(e) => {
                    fallos += 1;
                    avisos.push(format!(
                        "`{}` no se deja leer, y su tipo no se confirma: {e}",
                        items[i].obj.clave
                    ));
                    primer_fallo.get_or_insert(e);
                    continue;
                }
            };
            let (c, aviso) = medio::confirmar(ext, &cab);
            if let Some(a) = aviso {
                avisos.push(format!("`{}`: {a}", items[i].obj.clave));
            }
            decidida = Some(c);
        }
        if let Some(c) = decidida {
            for &i in idx {
                items[i].clase = c;
            }
        }
    }
    if intentos > 0
        && fallos == intentos
        && let Some(e) = primer_fallo
    {
        return Err(e);
    }

    let mut nombres = Nombres::default();
    let mut tablas: Vec<Tabla> = Vec::new();
    let mut objetos: Vec<Objetos> = Vec::new();

    // ── 2 · lo que es filas: tablas ──────────────────────────────────────────
    // Por (raíz, formato): la raíz de una tabla Hive es la carpeta de encima
    // de sus particiones; la de lo demás, su carpeta.
    let mut de_filas: BTreeMap<(String, Formato), (Vec<usize>, Vec<String>)> = BTreeMap::new();
    for (i, it) in items.iter().enumerate() {
        let Clase::Filas(f) = it.clase else { continue };
        let (raiz, claves) = match hive(&it.dir) {
            Some((r, k)) => (r, k),
            None => (it.dir.clone(), Vec::new()),
        };
        let e = de_filas.entry((raiz, f)).or_default();
        e.0.push(i);
        if e.1.is_empty() {
            e.1 = claves;
        }
    }
    for ((raiz, f), (idx, particiones)) in de_filas {
        let bytes: u64 = idx.iter().map(|&i| items[i].obj.tamano).sum();
        let ext = items[idx[0]].ext.clone();
        let schema = schema_de(&raiz);
        let base_carpeta = ultima(&raiz).map(identificador);
        if !particiones.is_empty() {
            // Una tabla Hive: el esquema del primero, las particiones como
            // columnas, y las filas de los que se leyeron (0062).
            let esquema = esquema_de(o, f, items[idx[0]].obj, &ext)?;
            let mut leidos = vec![(
                items[idx[0]].obj.tamano,
                esquema.filas,
                esquema.filas_estimadas,
            )];
            for &i in idx.iter().skip(1).take(COMPARAR - 1) {
                let e = esquema_de(o, f, items[i].obj, &items[i].ext).ok();
                leidos.push((
                    items[i].obj.tamano,
                    e.as_ref().and_then(|e| e.filas),
                    e.as_ref().and_then(|e| e.filas_estimadas),
                ));
            }
            let (filas, filas_estimadas) = filas_de_varios(&leidos, idx.len(), bytes);
            let nombre = nombres.dar(&schema, &base_carpeta.unwrap_or_else(|| "raiz".into()));
            tablas.push(tabla(
                nombre,
                format!("{prefijo}{raiz}"),
                f,
                Some(format!("**/*.{ext}")),
                &particiones,
                Esquema {
                    filas,
                    filas_estimadas,
                    ..esquema
                },
                bytes,
                avisos,
            ));
            continue;
        }
        // Sin particiones: ¿comparten esquema?
        let mut esquemas = Vec::new();
        for &i in idx.iter().take(COMPARAR) {
            esquemas.push(esquema_de(o, f, items[i].obj, &items[i].ext)?);
        }
        let comparten = idx.len() > 1
            && (f == Formato::Jsonl
                || esquemas
                    .windows(2)
                    .all(|w| firma(&w[0].columnas) == firma(&w[1].columnas)));
        if comparten {
            if idx.len() > COMPARAR {
                avisos.push(format!(
                    "`{prefijo}{raiz}`: {} ficheros; se compararon {COMPARAR}",
                    idx.len()
                ));
            }
            let leidos: Vec<_> = idx
                .iter()
                .zip(&esquemas)
                .map(|(&i, e)| (items[i].obj.tamano, e.filas, e.filas_estimadas))
                .collect();
            let (filas, filas_estimadas) = filas_de_varios(&leidos, idx.len(), bytes);
            let mut unido = if f == Formato::Jsonl {
                let cols = unir(&esquemas);
                Esquema {
                    columnas: cols,
                    filas: None,
                    filas_estimadas: None,
                    separador: None,
                    avisos: esquemas
                        .iter_mut()
                        .flat_map(|e| e.avisos.drain(..))
                        .collect(),
                }
            } else {
                esquemas.swap_remove(0)
            };
            unido.filas = filas;
            unido.filas_estimadas = filas_estimadas;
            let nombre = nombres.dar(&schema, &base_carpeta.unwrap_or_else(|| "raiz".into()));
            // `*`, no `**`: los de las subcarpetas son de su propia tabla.
            tablas.push(tabla(
                nombre,
                format!("{prefijo}{raiz}"),
                f,
                Some(format!("*.{ext}")),
                &[],
                unido,
                bytes,
                avisos,
            ));
            continue;
        }
        // Cada fichero, la suya.
        for (k, &i) in idx.iter().enumerate() {
            let esquema = match esquemas.get_mut(k) {
                Some(e) => std::mem::replace(
                    e,
                    Esquema {
                        columnas: Vec::new(),
                        filas: None,
                        filas_estimadas: None,
                        separador: None,
                        avisos: Vec::new(),
                    },
                ),
                None => esquema_de(o, f, items[i].obj, &items[i].ext)?,
            };
            let nombre = nombres.dar(&schema, &tallo(&items[i].rel));
            tablas.push(tabla(
                nombre,
                items[i].obj.clave.clone(),
                f,
                None,
                &[],
                esquema,
                items[i].obj.tamano,
                avisos,
            ));
        }
    }

    // ── 3 · lo que no es filas: conjuntos de objetos ─────────────────────────
    // Por carpeta. Si en la carpeta no hay más que un medio, un conjunto; si
    // hay más cosas —otro medio, o ficheros que son tablas—, uno por medio y
    // extensión, con su patrón. Si hay objetos más abajo, el patrón no cruza
    // `/` y los de abajo son de su carpeta.
    let mut por_carpeta: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, it) in items.iter().enumerate() {
        por_carpeta.entry(it.dir.clone()).or_default().push(i);
    }
    for (dir, idx) in &por_carpeta {
        let medios: BTreeSet<&str> = idx
            .iter()
            .filter_map(|&i| match items[i].clase {
                Clase::Medio(m) => Some(m),
                Clase::Filas(_) => None,
            })
            .collect();
        if medios.is_empty() {
            continue;
        }
        let hay_filas = idx
            .iter()
            .any(|&i| matches!(items[i].clase, Clase::Filas(_)));
        let hay_debajo = items
            .iter()
            .any(|it| it.dir != *dir && it.dir.starts_with(dir.as_str()));
        let mezclada = hay_filas || medios.len() > 1;
        // (medio, extensión tal cual si la carpeta está mezclada) → objetos
        let mut conjuntos: BTreeMap<(&str, String), Vec<usize>> = BTreeMap::new();
        for &i in idx {
            let Clase::Medio(m) = items[i].clase else {
                continue;
            };
            let tal_cual = items[i]
                .rel
                .rsplit_once('.')
                .map(|(_, e)| e.to_string())
                .unwrap_or_default();
            let clave = if mezclada { tal_cual } else { String::new() };
            conjuntos.entry((m, clave)).or_default().push(i);
        }
        let schema = schema_de(dir);
        let base = ultima(dir)
            .map(identificador)
            .unwrap_or_else(|| "raiz".into());
        for ((m, ext), js) in conjuntos {
            let patron = if !ext.is_empty() {
                Some(format!("*.{ext}"))
            } else if hay_debajo {
                Some("*".to_string())
            } else {
                None
            };
            let nombre = if mezclada {
                let sufijo = if ext.is_empty() {
                    m.to_string()
                } else {
                    identificador(&ext)
                };
                nombres.dar(&schema, &format!("{base}_{sufijo}"))
            } else {
                nombres.dar(&schema, &base)
            };
            let bytes: u64 = js.iter().map(|&i| items[i].obj.tamano).sum();
            let extensiones: BTreeSet<String> = js
                .iter()
                .map(|&i| items[i].ext.clone())
                .filter(|e| !e.is_empty())
                .collect();
            objetos.push(Objetos {
                nombre,
                prefijo: format!("{prefijo}{dir}"),
                patron,
                medio: m.to_string(),
                particiones: Vec::new(),
                cuantos: js.len() as u64,
                bytes,
                extensiones: extensiones.into_iter().collect(),
                lee: Some(cara_i(bytes)),
                cambia: Some(cara_d()),
            });
        }
    }

    Ok(Catalogo {
        fuente: fuente.to_string(),
        tablas,
        objetos,
    })
}

/// **El testigo de un conjunto: su listado.** La huella de los pares (clave,
/// ETag, tamaño) ordenados: lo que cambió entre dos es la diferencia de dos
/// listados, y un objeto que desaparece la cambia (spec `01` §5). El ETag no
/// es la huella del contenido, pero cambia cuando el contenido cambia, y basta
/// para saber SI cambió sin abrir cada objeto.
pub fn testigo(o: &dyn Origen, objeto: &str) -> Result<String, String> {
    let lista = o.listar(objeto)?;
    let mut filas: Vec<String> = lista
        .iter()
        .filter(|x| !x.clave.ends_with('/'))
        .filter(|x| objeto.ends_with('/') || x.clave == objeto)
        .map(|x| format!("{}\t{}\t{}", x.clave, x.etag, x.tamano))
        .collect();
    filas.sort();
    let texto = filas.join("\n");
    Ok(ore_driver::testigo(
        "listing",
        Some(&format!(
            "sha256:{}",
            ore_objetos::hex(&ore_objetos::sha256(texto.as_bytes()))
        )),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ore_objetos::memoria::EnMemoria;

    /// Un Parquet de verdad, pequeño, escrito aquí: `id` texto, `total`
    /// decimal(12, 2).
    fn un_parquet() -> Vec<u8> {
        use parquet::basic::{ConvertedType, LogicalType, Repetition, Type as Fisico};
        use parquet::file::properties::WriterProperties;
        use parquet::file::writer::SerializedFileWriter;
        use parquet::schema::types::Type;
        let id = Type::primitive_type_builder("id", Fisico::BYTE_ARRAY)
            .with_repetition(Repetition::OPTIONAL)
            .with_logical_type(Some(LogicalType::String))
            .build()
            .unwrap();
        let total = Type::primitive_type_builder("total", Fisico::INT64)
            .with_repetition(Repetition::OPTIONAL)
            .with_converted_type(ConvertedType::DECIMAL)
            .with_precision(12)
            .with_scale(2)
            .build()
            .unwrap();
        let esquema = Type::group_type_builder("schema")
            .with_fields(vec![std::sync::Arc::new(id), std::sync::Arc::new(total)])
            .build()
            .unwrap();
        let mut buf = Vec::new();
        let w = SerializedFileWriter::new(
            &mut buf,
            std::sync::Arc::new(esquema),
            std::sync::Arc::new(WriterProperties::builder().build()),
        )
        .unwrap();
        w.close().unwrap();
        buf
    }

    /// El bucket de F1, en pequeño y con sus nombres de verdad.
    fn bucket() -> EnMemoria {
        let pq = un_parquet();
        let csv_a = b"order_id,customer_id,price\n1,a,10.5\n2,b,3\n".to_vec();
        let csv_b = b"customer_id,customer_zip_code_prefix\na,01037\nb,14409\n".to_vec();
        EnMemoria::con(&[
            ("Nueva carpeta/archive.zip", b"PK\x03\x04zipzip".to_vec()),
            (
                "Nueva carpeta/contratos/contrato-001.pdf",
                b"%PDF-1.4 uno".to_vec(),
            ),
            (
                "Nueva carpeta/contratos/contrato-002.pdf",
                b"%PDF-1.4 dos".to_vec(),
            ),
            (
                "Nueva carpeta/fotos/Foto Portada 2026.JPG",
                b"\xff\xd8\xff\xe0jpg".to_vec(),
            ),
            (
                "Nueva carpeta/fotos/producto-001.png",
                b"\x89PNG\r\n".to_vec(),
            ),
            (
                "Nueva carpeta/logs/app-2026-09-27.jsonl",
                b"{\"ts\":\"2026-09-27T10:00:00Z\",\"n\":1}\n".to_vec(),
            ),
            (
                "Nueva carpeta/logs/app-2026-09-28.jsonl",
                b"{\"ts\":\"2026-09-28T10:00:00Z\",\"n\":2,\"usuario\":\"x\"}\n".to_vec(),
            ),
            ("Nueva carpeta/olist_orders_dataset.csv", csv_a),
            ("Nueva carpeta/olist_customers_dataset.csv", csv_b),
            (
                "Nueva carpeta/ventas/pedidos/fecha=2026-09-01/part-0000.parquet",
                pq.clone(),
            ),
            (
                "Nueva carpeta/ventas/pedidos/fecha=2026-09-02/part-0000.parquet",
                pq,
            ),
            ("Nueva carpeta/vacia/", Vec::new()),
            ("bloc anucios.txt", b"hola".to_vec()),
            ("kit-s3.zip", b"PK\x03\x04kit".to_vec()),
        ])
    }

    fn por_nombre<'a>(c: &'a Catalogo, n: &str) -> &'a Tabla {
        c.tablas.iter().find(|t| t.nombre == n).unwrap_or_else(|| {
            panic!(
                "no está `{n}`: {:?}",
                c.tablas.iter().map(|t| &t.nombre).collect::<Vec<_>>()
            )
        })
    }

    fn conjunto<'a>(c: &'a Catalogo, n: &str) -> &'a Objetos {
        c.objetos.iter().find(|t| t.nombre == n).unwrap_or_else(|| {
            panic!(
                "no está `{n}`: {:?}",
                c.objetos.iter().map(|t| &t.nombre).collect::<Vec<_>>()
            )
        })
    }

    #[test]
    fn el_bucket_de_f1_se_cataloga_como_lo_nombrara_el_arbol() {
        let o = bucket();
        let mut avisos = Vec::new();
        let c = leer(&o, "s3_ventas", "", &mut avisos).unwrap();

        // Hive: una tabla, la partición como columna, las filas de los pies.
        let p = por_nombre(&c, "nueva_carpeta.pedidos");
        assert_eq!(p.objeto.as_deref(), Some("Nueva carpeta/ventas/pedidos/"));
        let f = ore_base::json::Json::jcs(p.formato.as_ref().unwrap());
        assert!(
            f.contains("\"type\":\"parquet\"")
                && f.contains("\"partitions\":[\"fecha\"]")
                && f.contains("**/*.parquet"),
            "{f}"
        );
        let cols: Vec<(&str, Option<&str>)> = p
            .columnas
            .iter()
            .map(|c| (c.nombre.as_str(), c.tipo.as_deref()))
            .collect();
        assert_eq!(
            cols,
            [
                ("id", Some("String")),
                ("total", Some("Decimal<12, 2>")),
                ("fecha", Some("String"))
            ]
        );
        assert_eq!(p.filas, Some(0));

        // Dos CSV sueltos con esquemas distintos: dos tablas, cada una su clave.
        let a = por_nombre(&c, "nueva_carpeta.olist_orders_dataset");
        assert_eq!(
            a.objeto.as_deref(),
            Some("Nueva carpeta/olist_orders_dataset.csv")
        );
        let b = por_nombre(&c, "nueva_carpeta.olist_customers_dataset");
        assert_eq!(
            b.columnas[1].tipo.as_deref(),
            Some("String"),
            "el código postal se queda en texto"
        );

        // JSONL de una carpeta: una tabla, la unión de las claves.
        let l = por_nombre(&c, "nueva_carpeta.logs");
        assert_eq!(l.objeto.as_deref(), Some("Nueva carpeta/logs/"));
        assert!(l.columnas.iter().any(|c| c.nombre == "usuario"));

        // Los medios: una carpeta de un medio es un conjunto sin patrón; una
        // mezclada, uno por medio y extensión con el suyo.
        let fotos = conjunto(&c, "nueva_carpeta.fotos");
        assert_eq!(fotos.medio, "image");
        assert_eq!(fotos.prefijo, "Nueva carpeta/fotos/");
        assert_eq!(fotos.patron, None);
        assert_eq!(fotos.extensiones, ["jpg", "png"]);
        assert_eq!(fotos.cuantos, 2);
        let pdf = conjunto(&c, "nueva_carpeta.contratos");
        assert_eq!(pdf.medio, "document");
        let zip = conjunto(&c, "nueva_carpeta.nueva_carpeta_zip");
        assert_eq!(
            (zip.medio.as_str(), zip.patron.as_deref()),
            ("archive", Some("*.zip"))
        );
        let txt = conjunto(&c, "default.raiz_txt");
        assert_eq!(
            (txt.prefijo.as_str(), txt.patron.as_deref()),
            ("", Some("*.txt"))
        );
        conjunto(&c, "default.raiz_zip");

        // Lo que no es de nadie: la carpeta vacía no es un objeto.
        assert!(c.objetos.iter().all(|x| !x.prefijo.contains("vacia")));
        // Y se leyó poco: nunca un fichero entero.
        assert!(
            o.bytes_leidos.get() < 4096,
            "{} bytes",
            o.bytes_leidos.get()
        );
    }

    #[test]
    fn csv_con_el_mismo_esquema_son_una_tabla() {
        let o = EnMemoria::con(&[
            ("ventas/2026-01.csv", b"id,n\n1,2\n".to_vec()),
            ("ventas/2026-02.csv", b"id,n\n3,4\n".to_vec()),
        ]);
        let c = leer(&o, "f", "", &mut Vec::new()).unwrap();
        assert_eq!(c.tablas.len(), 1);
        assert_eq!(c.tablas[0].nombre, "ventas.ventas");
        assert_eq!(c.tablas[0].objeto.as_deref(), Some("ventas/"));
    }

    /// Un origen en el que unos ficheros no se dejan leer (sin permiso, como
    /// en un SFTP): lo demás, como el de memoria.
    struct ConCerrados(EnMemoria, &'static [&'static str]);

    impl Origen for ConCerrados {
        fn listar(&self, p: &str) -> Result<Vec<ore_objetos::Objeto>, String> {
            self.0.listar(p)
        }
        fn rango(&self, clave: &str, r: &str) -> Result<Vec<u8>, String> {
            if self.1.contains(&clave) {
                return Err(format!("`{clave}`: sin permiso de lectura (SFTP 3)"));
            }
            self.0.rango(clave, r)
        }
        fn abrir(&self, c: &str, e: &str) -> Result<Box<dyn std::io::Read + '_>, String> {
            self.0.abrir(c, e)
        }
        fn rango_de(&self, c: &str, r: &str, e: &str) -> Result<Vec<u8>, String> {
            self.0.rango_de(c, r, e)
        }
        fn listar_versiones(&self, p: &str) -> Result<Vec<ore_objetos::Version>, String> {
            self.0.listar_versiones(p)
        }
        fn huella_de(&self, c: &str, v: &str) -> Result<Option<String>, String> {
            self.0.huella_de(c, v)
        }
        fn abrir_version(
            &self,
            c: &str,
            v: &str,
        ) -> Result<(Box<dyn std::io::Read + '_>, ore_objetos::Abierto), String> {
            self.0.abrir_version(c, v)
        }
    }

    /// ⭐ ADR 0061 O4·4 · Un fichero que no se deja leer es un aviso, no el fin
    /// del catálogo; si no se deja leer ninguno, sí se falla (falta permiso).
    #[test]
    fn un_fichero_que_no_se_deja_leer_no_tumba_el_catalogo() {
        let pares = [
            ("docs/a.pdf", b"%PDF-1.4\n% a\n".to_vec()),
            ("docs/cerrado.pdf", b"%PDF-1.4\n% b\n".to_vec()),
            ("ventas/v.csv", b"id,n\n1,2\n".to_vec()),
        ];
        let o = ConCerrados(EnMemoria::con(&pares), &["docs/cerrado.pdf"]);
        let mut avisos = Vec::new();
        let c = leer(&o, "f", "", &mut avisos).expect("se cataloga igual");
        assert_eq!((c.tablas.len(), c.objetos.len()), (1, 1), "{c:?}");
        assert!(
            avisos
                .iter()
                .any(|a| a.contains("docs/cerrado.pdf") && a.contains("SFTP 3")),
            "{avisos:?}"
        );
        let todos = ConCerrados(
            EnMemoria::con(&pares),
            &["docs/a.pdf", "docs/cerrado.pdf", "ventas/v.csv"],
        );
        assert!(leer(&todos, "f", "", &mut Vec::new()).is_err());
    }

    #[test]
    fn los_nombres_que_chocan_se_distinguen() {
        let o = EnMemoria::con(&[
            ("a/Informe.csv", b"x\n1\n".to_vec()),
            ("a/informe.CSV", b"y\n2\n".to_vec()),
        ]);
        let c = leer(&o, "f", "", &mut Vec::new()).unwrap();
        let mut n: Vec<&str> = c.tablas.iter().map(|t| t.nombre.as_str()).collect();
        n.sort();
        assert_eq!(n, ["a.informe", "a.informe_2"]);
    }

    #[test]
    fn el_testigo_cambia_si_algo_desaparece() {
        let mut o = bucket();
        let a = testigo(&o, "Nueva carpeta/contratos/").unwrap();
        o.objetos.remove("Nueva carpeta/contratos/contrato-002.pdf");
        let b = testigo(&o, "Nueva carpeta/contratos/").unwrap();
        assert_ne!(a, b);
        assert!(a.contains("\"modo\":\"listing\""), "{a}");
    }

    #[test]
    fn un_identificador_de_oos() {
        assert_eq!(identificador("Nueva carpeta"), "nueva_carpeta");
        assert_eq!(identificador("Foto Portada 2026"), "foto_portada_2026");
        assert_eq!(identificador("2026-01"), "x_2026_01");
        assert_eq!(identificador("año"), "ano");
        // Nunca más de LARGO, y sin `_` al final del corte.
        let largo = identificador(&"a_".repeat(80));
        assert!(largo.len() <= LARGO && !largo.ends_with('_'), "{largo}");
    }

    /// Lo que deja Spark: los ocultos fuera, y el nombre de la parte, su carpeta.
    #[test]
    fn lo_de_spark_se_lee_como_spark() {
        for o in [
            "_SUCCESS",
            "green/_committed_4753095944193949832",
            "green/_started_1",
            "x/_temporary/0/part-0.parquet",
            ".hidden.csv",
            "a/b.parquet.crc/.x",
        ] {
            assert!(oculto(o), "{o}");
        }
        for o in ["green/part-0.parquet", "ventas_2026.csv", "a/b_c.parquet"] {
            assert!(!oculto(o), "{o}");
        }
        let parte = "part-00066-tid-4753095944193949832-fee7e113-666d-4114-9fcb-bcd3046479f3-2636-2.c000.snappy.parquet";
        assert_eq!(tallo(parte), "part_00066");
        assert_eq!(
            tallo(&format!("puYear=2019/puMonth=1/{parte}")),
            "pumonth_1"
        );
        assert_eq!(tallo("Nueva carpeta/ventas 2026.csv"), "ventas_2026");
        assert_eq!(parte_de_spark("part-.parquet"), None);
    }

    /// 0062 · El contenedor `mnist` de Azure Open Datasets, en pequeño: un CSV
    /// grande se estima, uno pequeño se cuenta, una tabla Hive suma, un Parquet
    /// cuenta, y la carpeta vacía que se llama como otras no es un objeto.
    #[test]
    fn las_filas_se_cuentan_o_se_estiman_y_una_carpeta_no_es_un_objeto() {
        let fila = "5,0,0,128,255\n"; // 14 bytes
        let grande = fila.repeat(50_000).into_bytes();
        let pequeno = fila.repeat(3).into_bytes();
        let o = EnMemoria::con(&[
            (
                "train.csv",
                [b"label,a,b,c,d\n".to_vec(), grande.clone()].concat(),
            ),
            // Otras columnas: no se une con `train.csv` (como en `mnist`).
            ("labels.csv", b"label,x\n5,0\n7,1\n9,2\n".to_vec()),
            (
                "processed/dataset_filter=train/train.csv",
                [b"label,a,b,c,d\n".to_vec(), grande].concat(),
            ),
            (
                "processed/dataset_filter=test/t10k.csv",
                [b"label,a,b,c,d\n".to_vec(), pequeno].concat(),
            ),
            ("green/part-0.parquet", un_parquet()),
            ("processedManifest", Vec::new()),
            ("processedManifest/manifest.json", b"{}".to_vec()),
        ]);
        let mut avisos = Vec::new();
        let c = leer(&o, "mnist", "", &mut avisos).unwrap();

        let t = por_nombre(&c, "default.train");
        assert_eq!((t.filas, t.filas_estimadas), (None, Some(50_000)));
        let t = por_nombre(&c, "default.labels");
        assert_eq!((t.filas, t.filas_estimadas), (Some(3), None));
        // Hive: uno contado y otro estimado dan una estimación de la suma.
        let t = por_nombre(&c, "processed.processed");
        assert_eq!((t.filas, t.filas_estimadas), (None, Some(50_003)));
        let t = por_nombre(&c, "green.green");
        assert_eq!((t.filas, t.filas_estimadas), (Some(0), None));

        assert!(
            c.objetos
                .iter()
                .all(|x| !x.prefijo.is_empty() || x.cuantos != 1 || x.bytes != 0),
            "el marcador `processedManifest` no es un objeto: {:?}",
            c.objetos.iter().map(|x| &x.nombre).collect::<Vec<_>>()
        );
        let m = conjunto(&c, "processedmanifest.processedmanifest");
        assert_eq!(m.cuantos, 1);

        // Y lo estimado viaja aparte en el catálogo.
        let j = ore_driver::catalogo::escribir(&c);
        assert!(j.contains("\"rowsEstimate\": 50000"), "{j}");
        let de_vuelta = Catalogo::leer(&j).unwrap();
        assert_eq!(
            por_nombre(&de_vuelta, "default.train").filas_estimadas,
            Some(50_000)
        );
    }
}
