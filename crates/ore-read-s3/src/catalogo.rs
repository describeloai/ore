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
//! distinguen con `_2`, aquí: el nombre es la clave del catálogo.

use crate::medio::{self, Clase, Formato};
use crate::origen::Origen;
use crate::tabular::{self, Esquema};
use ore_core::document::COLUMNA_RESCATADA as RESCATADA;
use ore_core::json::Json;
use ore_driver::catalogo::{Catalogo, Columna, Objetos, Tabla};
use ore_s3::Objeto;
use std::collections::{BTreeMap, BTreeSet};

/// Cuántos objetos de un grupo se abren para confirmar su tipo.
const CONFIRMAR: usize = 2;
/// Cuántos sin extensión se abren, como mucho, para saber qué son.
const SIN_EXTENSION: usize = 100;
/// Cuántos ficheros de una carpeta se comparan para decidir si son una tabla.
const COMPARAR: usize = 20;

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
    match out.chars().next() {
        None => "x".into(),
        Some(c) if c.is_ascii_digit() => format!("x_{out}"),
        Some(_) => out,
    }
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
        clase: "table".into(),
        lee: Some(cara_i(bytes)),
        cambia: Some(cara_d()),
        objeto: Some(objeto),
        formato: Some(Json::obj(f)),
        ..Default::default()
    }
}

/// **El catálogo.** `prefijo` es el de la fuente (vacío o con barra final).
pub fn leer(
    o: &dyn Origen,
    fuente: &str,
    prefijo: &str,
    avisos: &mut Vec<String>,
) -> Result<Catalogo, String> {
    let lista = o.listar(prefijo)?;
    // Una «carpeta» creada desde la consola es un objeto vacío acabado en `/`.
    let lista: Vec<&Objeto> = lista.iter().filter(|x| !x.clave.ends_with('/')).collect();
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
    for ((_, ext), idx) in &grupos {
        if ext.is_empty() {
            for &i in idx {
                if sin_extension >= SIN_EXTENSION {
                    break;
                }
                sin_extension += 1;
                let cab = o.rango(&items[i].obj.clave, "0-15").unwrap_or_default();
                items[i].clase = medio::confirmar("", &cab).0;
            }
            continue;
        }
        let mut decidida: Option<Clase> = None;
        for &i in idx.iter().take(CONFIRMAR) {
            if items[i].obj.tamano == 0 {
                continue;
            }
            let cab = o.rango(&items[i].obj.clave, "0-15")?;
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
            // columnas, y las filas si todos los pies se leyeron.
            let esquema = esquema_de(o, f, items[idx[0]].obj, &ext)?;
            let mut filas = esquema.filas;
            if f == Formato::Parquet && idx.len() > 1 {
                filas = idx
                    .iter()
                    .take(COMPARAR)
                    .map(|&i| tabular::parquet(o, &items[i].obj.clave).ok()?.filas)
                    .sum::<Option<u64>>()
                    .filter(|_| idx.len() <= COMPARAR);
            }
            let nombre = nombres.dar(&schema, &base_carpeta.unwrap_or_else(|| "raiz".into()));
            tablas.push(tabla(
                nombre,
                format!("{prefijo}{raiz}"),
                f,
                Some(format!("**/*.{ext}")),
                &particiones,
                Esquema { filas, ..esquema },
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
            let mut unido = if f == Formato::Jsonl {
                let cols = unir(&esquemas);
                Esquema {
                    columnas: cols,
                    filas: None,
                    separador: None,
                    avisos: esquemas
                        .iter_mut()
                        .flat_map(|e| e.avisos.drain(..))
                        .collect(),
                }
            } else {
                let mut primero = esquemas.swap_remove(0);
                primero.filas = None;
                primero
            };
            if f == Formato::Parquet {
                unido.filas = None;
            }
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
            ore_s3::hex(&ore_s3::sha256(texto.as_bytes()))
        )),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::origen::EnMemoria;

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
        let f = ore_core::json::Json::jcs(p.formato.as_ref().unwrap());
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
    }
}
