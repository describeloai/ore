//! **Las filas de un bucket** (0046 E6): `leer` de una `Table` con `format`,
//! en Arrow.
//!
//! Lo que se lee y cómo lo dice el árbol, no este programa: la petición trae
//! el `format` de la tabla y sus tipos congelados (`ore_driver::Fichero`), y
//! aquí no se vuelve a deducir nada. Cinco decisiones, investigadas y medidas
//! contra el bucket de F1 (ADR 0046, «Lo que E6 decidió»):
//!
//! 1. **El formato viene del árbol.** Como Glue, Trino o un `FILE FORMAT` de
//!    Snowflake: la tabla lo guarda y el lector lo obedece.
//! 2. **Lo que no encaja se rescata o para**, según la tabla declare
//!    `_rescued_data` (v1alpha16 `03` §1.1). Rescatado: nulo en su columna, y
//!    su texto con el fichero en la rescatada. Sin declararla: la lectura se
//!    para con la columna, el fichero, la fila y el valor. Nunca un nulo
//!    callado.
//! 3. **Lo vacío, la regla de `COPY` de PostgreSQL** (`03` §1.2): un campo
//!    vacío sin comillas es nulo y `""` es la cadena vacía. Por eso el CSV se
//!    lee con un analizador propio: el de `arrow-csv` no ve las comillas.
//! 4. **Cada fichero, fijado a lo que el listado dijo** (`If-Match`). S3 es
//!    consistente por clave y no entre claves; si uno cambia mientras se lee,
//!    la lectura para y no mezcla dos versiones.
//! 5. **Un Parquet grande, por rangos**: hasta [`UMBRAL`], un GET; por encima,
//!    el pie por su sufijo y, grupo de filas a grupo de filas, sólo los trozos
//!    de las columnas pedidas, juntando los cercanos. La memoria queda en un
//!    grupo de filas.
//!
//! El tipo de cada valor lo analiza [`Fisico::analizar`], la misma forma
//! canónica con la que `ore-store` estrecha: lo que aquí encaja, allí también.

use arrow_array::builder::{
    BooleanBuilder, Date32Builder, Decimal128Builder, Float64Builder, Int64Builder, StringBuilder,
    Time64MicrosecondBuilder, TimestampMicrosecondBuilder,
};
use arrow_array::{ArrayRef, BooleanArray, RecordBatch, Scalar, StringArray};
use arrow_schema::{DataType, Field, Schema, SchemaRef, TimeUnit};
use ore_core::json::Json;
use ore_core::tipos::{Fisico, Valor};
use ore_driver::{Fichero, Peticion};
use ore_objetos::Objeto;
use ore_objetos::Origen;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, Read as _, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Hasta aquí, un Parquet se baja de una vez: con ficheros pequeños lo que
/// cuesta es el número de peticiones, no los bytes (DuckDB lo midió: pedir
/// troceado multiplicó por 51 las peticiones).
pub const UMBRAL: u64 = 16 * 1024 * 1024;
/// Dos trozos de un Parquet a menos de esto se piden juntos.
const HUECO: u64 = 1024 * 1024;
/// **Lo que este lector sabe poner** sobre las filas de un bucket (ADR 0053
/// F2·4). Los diez operadores, sobre las filas y —los que pueden— antes: una
/// partición que no cumple no se baja, y un grupo de filas de un Parquet cuyas
/// estadísticas no pueden cumplir, tampoco. `limit` deja de listar y de leer.
/// `orderBy` no: un listado no ordena, y ordenar aquí sería juntar la tabla
/// entera en memoria; con `orderBy`, el motor ordena y el `limit` no se empuja.
pub const CAPACIDADES: ore_driver::capacidades::Capacidades =
    ore_driver::capacidades::Capacidades {
        // El de cada driver lo pone `driver::main` (el `NOMBRE` de su proveedor).
        conector: "ore-read-objetos",
        version: env!("CARGO_PKG_VERSION"),
        operadores: ore_driver::OPERADORES,
        limit: true,
        order_by: false,
        estimar: true,
        servir: true,
    };

/// El principio del mensaje de una lectura que agotó su `timeoutMs`: `main`
/// lo lleva al código `tiempo`.
pub const AGOTADO: &str = "se agotó el tiempo";
/// Y el de una que se canceló.
pub const CANCELADA: &str = "cancelada";

/// Filas por lote de Arrow: las de `arrow-csv` y las del lector de Parquet.
const LOTE: usize = 8192;
const RESCATADA: &str = ore_core::document::COLUMNA_RESCATADA;
/// La zona de un instante como la escribe el almacén (`ore-store`, `UTC`).
const UTC: &str = "+00:00";

/// Lo que se leyó, para el aviso de `stderr`, y el freno de la lectura.
#[derive(Debug, Default)]
pub struct Leido {
    pub filas: u64,
    pub ficheros: usize,
    /// Columna → cuántos valores se rescataron.
    pub rescatados: BTreeMap<String, u64>,
    /// Ficheros que una partición descartó sin bajarlos.
    pub descartados: usize,
    /// Grupos de filas de Parquet que sus estadísticas descartaron.
    pub grupos_descartados: usize,
    /// `limit`: cuántas filas como mucho.
    limite: Option<u64>,
    /// `timeoutMs`: hasta cuándo, y cuántos ms eran.
    hasta: Option<(Instant, u64)>,
    /// La cancelación de `servir`.
    cancelada: Option<Arc<AtomicBool>>,
}

impl Leido {
    /// Si ya salieron las filas que `limit` pedía.
    fn basta(&self) -> bool {
        self.limite.is_some_and(|l| self.filas >= l)
    }

    /// Si hay que parar: el tiempo se agotó o alguien canceló.
    fn freno(&self) -> Result<(), String> {
        if self
            .cancelada
            .as_ref()
            .is_some_and(|c| c.load(Ordering::SeqCst))
        {
            return Err(format!("{CANCELADA} con {} filas leídas", self.filas));
        }
        match self.hasta {
            Some((h, ms)) if Instant::now() > h => Err(format!(
                "{AGOTADO}: {ms} ms, con {} filas leídas",
                self.filas
            )),
            _ => Ok(()),
        }
    }
}

/// Una condición de la petición, ya en los tipos de su columna.
struct Condicion {
    col: usize,
    op: String,
    /// En su tipo: uno; varios en `in`; ninguno en `isNull`, `isNotNull` y
    /// `like`.
    valores: Vec<Valor>,
    /// Los mismos, como escalares de Arrow.
    escalares: Vec<ArrayRef>,
    /// El patrón de `like`.
    patron: Option<String>,
}

/// Dos valores del mismo físico, comparados (`None`: no comparables).
fn comparar(a: &Valor, b: &Valor) -> Option<std::cmp::Ordering> {
    match (a, b) {
        (Valor::Texto(x), Valor::Texto(y)) => Some(x.cmp(y)),
        (Valor::Entero(x), Valor::Entero(y)) => Some(x.cmp(y)),
        (Valor::Real(x), Valor::Real(y)) => x.partial_cmp(y),
        (Valor::Logico(x), Valor::Logico(y)) => Some(x.cmp(y)),
        (Valor::Decimal(x), Valor::Decimal(y)) => Some(x.cmp(y)),
        (Valor::Fecha(x), Valor::Fecha(y)) => Some(x.cmp(y)),
        (Valor::Hora(x), Valor::Hora(y)) => Some(x.cmp(y)),
        (Valor::FechaHora(x), Valor::FechaHora(y)) => Some(x.cmp(y)),
        (Valor::Instante(x), Valor::Instante(y)) => Some(x.cmp(y)),
        _ => None,
    }
}

/// `LIKE` de SQL, sin carácter de escape: `%` cualquier secuencia, `_` un
/// carácter, lo demás literal.
pub fn como(texto: &str, patron: &str) -> bool {
    fn ir(t: &[char], p: &[char]) -> bool {
        match p.split_first() {
            None => t.is_empty(),
            Some(('%', resto)) => (0..=t.len()).any(|i| ir(&t[i..], resto)),
            Some(('_', resto)) => !t.is_empty() && ir(&t[1..], resto),
            Some((c, resto)) => t.first() == Some(c) && ir(&t[1..], resto),
        }
    }
    let t: Vec<char> = texto.chars().collect();
    let p: Vec<char> = patron.chars().collect();
    ir(&t, &p)
}

impl Condicion {
    /// **Si un valor la cumple**, con la semántica de SQL: un nulo no cumple
    /// nada salvo `isNull`. Es lo que descarta una partición.
    fn cumple(&self, v: Option<&Valor>) -> bool {
        use std::cmp::Ordering as O;
        match (self.op.as_str(), v) {
            ("isNull", v) => v.is_none(),
            ("isNotNull", v) => v.is_some(),
            (_, None) => false,
            ("like", Some(Valor::Texto(t))) => self.patron.as_deref().is_some_and(|p| como(t, p)),
            ("in", Some(v)) => self
                .valores
                .iter()
                .any(|x| comparar(v, x) == Some(O::Equal)),
            (op, Some(v)) => {
                let Some(o) = self.valores.first().and_then(|x| comparar(v, x)) else {
                    return false;
                };
                match op {
                    "eq" => o == O::Equal,
                    "neq" => o != O::Equal,
                    "lt" => o == O::Less,
                    "le" => o != O::Greater,
                    "gt" => o == O::Greater,
                    "ge" => o != O::Less,
                    _ => true,
                }
            }
        }
    }

    /// **Si algún valor entre `min` y `max` podría cumplirla**: lo que
    /// descarta un grupo de filas por sus estadísticas. Ante la duda, sí.
    fn puede(&self, min: &Valor, max: &Valor) -> bool {
        use std::cmp::Ordering as O;
        let dentro = |v: &Valor| {
            comparar(min, v).is_none_or(|o| o != O::Greater)
                && comparar(max, v).is_none_or(|o| o != O::Less)
        };
        let Some(v) = self.valores.first() else {
            return true;
        };
        match self.op.as_str() {
            "eq" => dentro(v),
            "in" => self.valores.iter().any(dentro),
            "lt" => comparar(min, v).is_none_or(|o| o == O::Less),
            "le" => comparar(min, v).is_none_or(|o| o != O::Greater),
            "gt" => comparar(max, v).is_none_or(|o| o == O::Greater),
            "ge" => comparar(max, v).is_none_or(|o| o != O::Less),
            _ => true,
        }
    }

    /// La máscara de la condición sobre una columna: nulo donde SQL dice
    /// «desconocido», que el filtro trata como falso.
    fn mascara(&self, a: &ArrayRef) -> Result<BooleanArray, String> {
        use arrow_ord::cmp;
        let e = |e: arrow_schema::ArrowError| format!("el filtro no se pudo aplicar: {e}");
        let uno = || Scalar::new(self.escalares[0].clone());
        Ok(match self.op.as_str() {
            "eq" => cmp::eq(a, &uno()).map_err(e)?,
            "neq" => cmp::neq(a, &uno()).map_err(e)?,
            "lt" => cmp::lt(a, &uno()).map_err(e)?,
            "le" => cmp::lt_eq(a, &uno()).map_err(e)?,
            "gt" => cmp::gt(a, &uno()).map_err(e)?,
            "ge" => cmp::gt_eq(a, &uno()).map_err(e)?,
            "isNull" => arrow_arith::boolean::is_null(a).map_err(e)?,
            "isNotNull" => arrow_arith::boolean::is_not_null(a).map_err(e)?,
            "in" => {
                let mut m = BooleanArray::from(vec![false; a.len()]);
                for x in &self.escalares {
                    let igual = cmp::eq(a, &Scalar::new(x.clone())).map_err(e)?;
                    m = arrow_arith::boolean::or(&m, &igual).map_err(e)?;
                }
                m
            }
            "like" => {
                use arrow_array::cast::AsArray;
                let p = self.patron.as_deref().unwrap_or("");
                a.as_string::<i32>()
                    .iter()
                    .map(|v| v.map(|t| como(t, p)))
                    .collect()
            }
            otro => return Err(format!("`{otro}` no es un operador de este lector")),
        })
    }
}

/// **Leer**: las filas de la tabla, en un flujo Arrow IPC por `salida`, con su
/// marca de fin. Un error a mitad deja el flujo sin marca, y el almacén no
/// sella una copia corta.
pub fn leer(
    o: &dyn Origen,
    p: &Peticion,
    salida: &mut dyn Write,
    umbral: u64,
) -> Result<Leido, String> {
    leer_con(o, p, salida, umbral, None)
}

/// [`leer`] con la cancelación de `servir`.
pub fn leer_con(
    o: &dyn Origen,
    p: &Peticion,
    salida: &mut dyn Write,
    umbral: u64,
    cancelada: Option<Arc<AtomicBool>>,
) -> Result<Leido, String> {
    let inicio = Instant::now();
    if p.formato.as_deref() != Some("arrow") {
        return Err("este lector contesta en Arrow, y la petición no lo pide".into());
    }
    // 0053 F9·1: el listado de un `ObjectTable`, como filas.
    if let Some(patron) = &p.listado {
        return leer_listado(o, p, patron, salida, inicio, cancelada);
    }
    let f = p.fichero.as_ref().ok_or(
        "la petición no dice cómo se leen los ficheros: `leer` es de una `Table` con `format` \
         (un `ObjectTable` no son filas)",
    )?;
    let plan = Plan::de(p, f)?;
    let ficheros = listado(o, &p.objeto, f.patron.as_deref())?;
    let mut escritor = arrow_ipc::writer::StreamWriter::try_new(salida, &plan.salida)
        .map_err(|e| format!("no se pudo empezar el flujo: {e}"))?;
    let mut leido = Leido {
        ficheros: ficheros.len(),
        limite: p.limit,
        hasta: p
            .timeout_ms
            .map(|ms| (inicio + Duration::from_millis(ms), ms)),
        cancelada,
        ..Default::default()
    };
    for obj in &ficheros {
        leido.freno()?;
        if leido.basta() {
            break;
        }
        let rel = relativa(&p.objeto, &obj.clave);
        let particiones = particiones(rel);
        if !plan.particion_cumple(&particiones) {
            leido.descartados += 1;
            continue;
        }
        match f.tipo.as_str() {
            "csv" => csv(o, obj, f, &plan, &particiones, &mut escritor, &mut leido)?,
            "jsonl" => jsonl(o, obj, &plan, &particiones, &mut escritor, &mut leido)?,
            "parquet" => parquet(
                o,
                obj,
                &plan,
                &particiones,
                umbral,
                &mut escritor,
                &mut leido,
            )?,
            otro => return Err(format!("`format.type: {otro}` no es un formato de filas")),
        }
    }
    escritor
        .finish()
        .map_err(|e| format!("no se pudo cerrar el flujo: {e}"))?;
    Ok(leido)
}

// ── Qué se lee ──────────────────────────────────────────────────────────────

/// Los ficheros de la tabla, **en orden de clave**: la misma lectura da las
/// mismas filas en el mismo orden.
/// **Estimar** (ADR 0053 F2·4): lo que leer costaría, sin leer nada más que
/// el listado: los ficheros que quedan tras descartar particiones y sus bytes.
/// Las filas no: saberlas pediría el pie de cada Parquet.
pub fn estimar(o: &dyn Origen, p: &Peticion) -> Result<String, String> {
    let f = p
        .fichero
        .as_ref()
        .ok_or("la petición no dice cómo se leen los ficheros")?;
    let plan = Plan::de(p, f)?;
    let ficheros = listado(o, &p.objeto, f.patron.as_deref())?;
    let quedan: Vec<&Objeto> = ficheros
        .iter()
        .filter(|x| plan.particion_cumple(&particiones(relativa(&p.objeto, &x.clave))))
        .collect();
    let bytes: u64 = quedan.iter().map(|x| x.tamano).sum();
    Ok(Json::obj([
        ("bytes", Json::Int(bytes as i64)),
        (
            "descartados",
            Json::Int((ficheros.len() - quedan.len()) as i64),
        ),
        ("ficheros", Json::Int(quedan.len() as i64)),
        ("fuente", Json::s("listado")),
    ])
    .jcs())
}

fn listado(o: &dyn Origen, objeto: &str, patron: Option<&str>) -> Result<Vec<Objeto>, String> {
    let mut v: Vec<Objeto> = o
        .listar(objeto)?
        .into_iter()
        .filter(|x| !x.clave.ends_with('/'))
        .filter(|x| objeto.ends_with('/') || x.clave == objeto)
        .filter(|x| patron.is_none_or(|pt| casa(pt, relativa(objeto, &x.clave))))
        .collect();
    v.sort_by(|a, b| a.clave.cmp(&b.clave));
    Ok(v)
}

/// **El listado de un `ObjectTable`** (0053 F9·1): una fila por objeto bajo
/// el prefijo que casa con su `match`, con las columnas fijas que se pidan
/// (`01-object-table` §1) y una por partición. Son metadatos: ningún objeto se
/// abre. Lo que el listado no da (`contentType`, `checksum`, `version`) sale
/// nulo. Los filtros no se empujan (el listado no los declara): `limit` sí.
fn leer_listado(
    o: &dyn Origen,
    p: &Peticion,
    patron: &str,
    salida: &mut dyn Write,
    inicio: Instant,
    cancelada: Option<Arc<AtomicBool>>,
) -> Result<Leido, String> {
    if !p.filtros.is_empty() {
        return Err("el listado de un `ObjectTable` no filtra en el origen: los filtros los evalúa el motor".into());
    }
    let prefijo = if p.objeto.ends_with('/') || p.objeto.is_empty() {
        p.objeto.clone()
    } else {
        format!("{}/", p.objeto)
    };
    let patron = (!patron.is_empty()).then_some(patron);
    let objetos = listado(o, &prefijo, patron)?;
    let fisico = |c: &str| match c {
        "size" => Fisico::Entero,
        "modified" => Fisico::Instante,
        _ => Fisico::Texto,
    };
    let campos: Vec<(String, String)> = p.proyeccion.clone();
    let esquema: SchemaRef = Arc::new(Schema::new(
        campos
            .iter()
            .map(|(prop, col)| Field::new(prop, tipo_arrow(&fisico(col)), true))
            .collect::<Vec<_>>(),
    ));
    let mut escritor = arrow_ipc::writer::StreamWriter::try_new(salida, &esquema)
        .map_err(|e| format!("no se pudo empezar el flujo: {e}"))?;
    let mut leido = Leido {
        ficheros: objetos.len(),
        limite: p.limit,
        hasta: p
            .timeout_ms
            .map(|ms| (inicio + Duration::from_millis(ms), ms)),
        cancelada,
        ..Default::default()
    };
    for trozo in objetos.chunks(8192) {
        leido.freno()?;
        if leido.basta() {
            break;
        }
        let queda = leido.limite.map_or(trozo.len(), |l| {
            (l - leido.filas).min(trozo.len() as u64) as usize
        });
        let trozo = &trozo[..queda];
        let mut cols: Vec<Col> = campos.iter().map(|(_, c)| Col::nuevo(&fisico(c))).collect();
        for obj in trozo {
            let parts = particiones(relativa(&prefijo, &obj.clave));
            for ((_, c), col) in campos.iter().zip(cols.iter_mut()) {
                let texto: Option<String> = match c.as_str() {
                    "key" => Some(obj.clave.clone()),
                    "size" => Some(obj.tamano.to_string()),
                    "modified" => Some(obj.modificado.clone()),
                    "contentType" | "checksum" | "version" => None,
                    otra => parts.get(otra).cloned(),
                };
                match texto.and_then(|t| fisico(c).analizar(&t)) {
                    Some(v) => col.valor(v),
                    None => col.nulo(),
                }
            }
        }
        let lote = RecordBatch::try_new(esquema.clone(), cols.iter_mut().map(Col::fin).collect())
            .map_err(|e| format!("no se pudo hacer el lote del listado: {e}"))?;
        escritor
            .write(&lote)
            .map_err(|e| format!("no se pudo escribir el lote: {e}"))?;
        leido.filas += trozo.len() as u64;
    }
    escritor
        .finish()
        .map_err(|e| format!("no se pudo cerrar el flujo: {e}"))?;
    Ok(leido)
}

/// La clave relativa a la tabla: bajo el prefijo, o el nombre del fichero.
fn relativa<'a>(objeto: &str, clave: &'a str) -> &'a str {
    if objeto.ends_with('/') {
        clave.strip_prefix(objeto).unwrap_or(clave)
    } else {
        clave.rsplit('/').next().unwrap_or(clave)
    }
}

/// Las particiones Hive del camino: `fecha=2026-09-01/…` → `fecha`.
fn particiones(rel: &str) -> BTreeMap<String, String> {
    let mut segs: Vec<&str> = rel.split('/').collect();
    segs.pop();
    segs.iter()
        .filter_map(|s| s.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// **El *glob* de `match`** (v1alpha16, como en `ObjectTable`): `*` no cruza
/// `/`, `**` sí —y `**/` vale también por ninguna carpeta—, `?` es un carácter.
pub fn casa(patron: &str, texto: &str) -> bool {
    fn m(p: &[u8], t: &[u8]) -> bool {
        match p {
            [] => t.is_empty(),
            [b'*', b'*', b'/', resto @ ..] => {
                (0..=t.len()).any(|i| (i == 0 || t[i - 1] == b'/') && m(resto, &t[i..]))
            }
            [b'*', b'*', resto @ ..] => (0..=t.len()).any(|i| m(resto, &t[i..])),
            [b'*', resto @ ..] => {
                for i in 0..=t.len() {
                    if m(resto, &t[i..]) {
                        return true;
                    }
                    if i < t.len() && t[i] == b'/' {
                        break;
                    }
                }
                false
            }
            [b'?', resto @ ..] => !t.is_empty() && t[0] != b'/' && m(resto, &t[1..]),
            [c, resto @ ..] => !t.is_empty() && t[0] == *c && m(resto, &t[1..]),
        }
    }
    m(patron.as_bytes(), texto.as_bytes())
}

/// Una columna física que hace falta leer: por la proyección o por un filtro.
struct Columna {
    nombre: String,
    fisico: Fisico,
    particion: bool,
}

struct Plan {
    cols: Vec<Columna>,
    indice: BTreeMap<String, usize>,
    /// Todas las columnas que la tabla declara: lo que no está aquí es «una
    /// columna que la tabla no declara» (`03` §1.1).
    declaradas: BTreeSet<String>,
    /// El orden de la tabla, para un CSV sin cabecera.
    orden: Vec<String>,
    /// Si la rescatada se pide. Declarada y no pedida no rescata: lo que no
    /// encaja pararía en silencio en una columna que nadie ve, y eso es un nulo
    /// callado.
    rescata: bool,
    salida: SchemaRef,
    proyeccion: Vec<(String, String)>,
    /// Las condiciones de la petición, cada una en el tipo de su columna.
    filtros: Vec<Condicion>,
}

impl Plan {
    /// **Si un fichero puede tener filas que cumplan**, por los valores de
    /// sus particiones (`fecha=2026-09-01/…`): uno que no, no se baja. La
    /// partición nula de Hive es `__HIVE_DEFAULT_PARTITION__`.
    fn particion_cumple(&self, particiones: &BTreeMap<String, String>) -> bool {
        self.filtros.iter().all(|c| {
            let col = &self.cols[c.col];
            if !col.particion {
                return true;
            }
            let v = particiones
                .get(&col.nombre)
                .filter(|v| v.as_str() != "__HIVE_DEFAULT_PARTITION__")
                .and_then(|t| col.fisico.analizar(t));
            c.cumple(v.as_ref())
        })
    }

    /// **Si un grupo de filas de un Parquet puede tener filas que cumplan**,
    /// por el mínimo y el máximo que guarda de cada columna. Sólo donde el
    /// valor de la estadística es el de la columna sin conversión (enteros,
    /// reales, fechas, texto); lo demás, por si acaso, se lee.
    fn grupo_puede(&self, g: &parquet::file::metadata::RowGroupMetaData) -> bool {
        use parquet::file::statistics::Statistics as E;
        self.filtros.iter().all(|c| {
            let col = &self.cols[c.col];
            if col.particion {
                return true;
            }
            let Some(st) = g
                .columns()
                .iter()
                .find(|m| m.column_path().parts() == [col.nombre.clone()])
                .and_then(|m| m.statistics())
            else {
                return true;
            };
            let par = match (st, &col.fisico) {
                (E::Int64(s), Fisico::Entero) => s
                    .min_opt()
                    .zip(s.max_opt())
                    .map(|(a, b)| (Valor::Entero(*a), Valor::Entero(*b))),
                (E::Int32(s), Fisico::Entero) => s
                    .min_opt()
                    .zip(s.max_opt())
                    .map(|(a, b)| (Valor::Entero((*a).into()), Valor::Entero((*b).into()))),
                (E::Int32(s), Fisico::Fecha) => s
                    .min_opt()
                    .zip(s.max_opt())
                    .map(|(a, b)| (Valor::Fecha(*a), Valor::Fecha(*b))),
                (E::Double(s), Fisico::Real) => s
                    .min_opt()
                    .zip(s.max_opt())
                    .map(|(a, b)| (Valor::Real(*a), Valor::Real(*b))),
                (E::ByteArray(s), Fisico::Texto) => {
                    s.min_opt().zip(s.max_opt()).and_then(|(a, b)| {
                        Some((
                            Valor::Texto(a.as_utf8().ok()?.to_string()),
                            Valor::Texto(b.as_utf8().ok()?.to_string()),
                        ))
                    })
                }
                _ => None,
            };
            par.is_none_or(|(min, max)| c.puede(&min, &max))
        })
    }
}

fn fisico_de(tipo: Option<&str>) -> Fisico {
    tipo.and_then(|t| ore_core::types::parse_type(t).ok())
        .map(|t| Fisico::de(&t))
        .unwrap_or(Fisico::Texto)
}

/// El `DataType` de Arrow del físico: el mismo que escribe `ore-store`, para
/// que el lote llegue ya como el contrato lo quiere.
fn tipo_arrow(f: &Fisico) -> DataType {
    match f {
        Fisico::Texto => DataType::Utf8,
        Fisico::Entero => DataType::Int64,
        Fisico::Real => DataType::Float64,
        Fisico::Logico => DataType::Boolean,
        Fisico::Decimal { precision, escala } => DataType::Decimal128(*precision, *escala as i8),
        Fisico::Fecha => DataType::Date32,
        Fisico::Hora => DataType::Time64(TimeUnit::Microsecond),
        Fisico::FechaHora => DataType::Timestamp(TimeUnit::Microsecond, None),
        Fisico::Instante => DataType::Timestamp(TimeUnit::Microsecond, Some(UTC.into())),
    }
}

impl Plan {
    fn de(p: &Peticion, f: &Fichero) -> Result<Plan, String> {
        let tipos: BTreeMap<&str, &str> = f
            .tipos
            .iter()
            .map(|(c, t)| (c.as_str(), t.as_str()))
            .collect();
        let mut plan = Plan {
            cols: Vec::new(),
            indice: BTreeMap::new(),
            declaradas: f.tipos.iter().map(|(c, _)| c.clone()).collect(),
            orden: f.tipos.iter().map(|(c, _)| c.clone()).collect(),
            rescata: false,
            salida: Arc::new(Schema::empty()),
            proyeccion: p.proyeccion.clone(),
            filtros: Vec::new(),
        };
        let anadir = |plan: &mut Plan, col: &str| -> usize {
            if let Some(i) = plan.indice.get(col) {
                return *i;
            }
            plan.cols.push(Columna {
                nombre: col.to_string(),
                fisico: fisico_de(tipos.get(col).copied()),
                particion: f.particiones.iter().any(|x| x == col),
            });
            plan.indice.insert(col.to_string(), plan.cols.len() - 1);
            plan.cols.len() - 1
        };
        let mut campos = Vec::new();
        for (prop, col) in &p.proyeccion {
            if col == RESCATADA {
                if f.tipo == "parquet" {
                    return Err(format!(
                        "`{RESCATADA}` en un Parquet: el tipo lo trae el fichero (03 §1.1)"
                    ));
                }
                plan.rescata = true;
                campos.push(Field::new(prop, DataType::Utf8, true));
                continue;
            }
            let i = anadir(&mut plan, col);
            campos.push(Field::new(prop, tipo_arrow(&plan.cols[i].fisico), true));
        }
        // Lo que no declara (`orderBy`) se niega en vez de servir de más.
        CAPACIDADES.admite(p).map_err(|f| f.mensaje)?;
        for f in &p.filtros {
            use ore_driver::Valor as Derecha;
            let col = &f.columna;
            let i = anadir(&mut plan, col);
            let fisico = plan.cols[i].fisico;
            let analizar = |t: &str| {
                fisico.analizar(t).ok_or_else(|| {
                    format!(
                        "el filtro `{col} {} {t}` no es un {} y la columna lo es",
                        f.operador,
                        fisico.nombre()
                    )
                })
            };
            let (valores, patron) = match (&f.valor, f.operador.as_str()) {
                (Derecha::Uno(t), "like") => {
                    if fisico != Fisico::Texto {
                        return Err(format!("`like` sobre `{col}`, que no es texto"));
                    }
                    (Vec::new(), Some(t.clone()))
                }
                (Derecha::Uno(t), _) => (vec![analizar(t)?], None),
                (Derecha::Lista(l), _) => (
                    l.iter().map(|t| analizar(t)).collect::<Result<_, _>>()?,
                    None,
                ),
                (Derecha::Ninguno, _) => (Vec::new(), None),
            };
            let escalares = valores
                .iter()
                .map(|v| {
                    let mut c = Col::nuevo(&fisico);
                    c.valor(v.clone());
                    c.fin()
                })
                .collect();
            plan.filtros.push(Condicion {
                col: i,
                op: f.operador.clone(),
                valores,
                escalares,
                patron,
            });
        }
        plan.salida = Arc::new(Schema::new(campos));
        Ok(plan)
    }
}

// ── El lote ─────────────────────────────────────────────────────────────────

/// Un constructor por físico.
enum Col {
    Texto(StringBuilder),
    Entero(Int64Builder),
    Real(Float64Builder),
    Logico(BooleanBuilder),
    Decimal(Decimal128Builder),
    Fecha(Date32Builder),
    Hora(Time64MicrosecondBuilder),
    Marca(TimestampMicrosecondBuilder),
}

impl Col {
    fn nuevo(f: &Fisico) -> Col {
        match f {
            Fisico::Texto => Col::Texto(StringBuilder::new()),
            Fisico::Entero => Col::Entero(Int64Builder::new()),
            Fisico::Real => Col::Real(Float64Builder::new()),
            Fisico::Logico => Col::Logico(BooleanBuilder::new()),
            Fisico::Decimal { precision, escala } => Col::Decimal(
                Decimal128Builder::new()
                    .with_precision_and_scale(*precision, *escala as i8)
                    .expect("la precisión de 0032 cabe en decimal128"),
            ),
            Fisico::Fecha => Col::Fecha(Date32Builder::new()),
            Fisico::Hora => Col::Hora(Time64MicrosecondBuilder::new()),
            Fisico::FechaHora => Col::Marca(TimestampMicrosecondBuilder::new()),
            Fisico::Instante => Col::Marca(TimestampMicrosecondBuilder::new().with_timezone(UTC)),
        }
    }

    fn nulo(&mut self) {
        match self {
            Col::Texto(b) => b.append_null(),
            Col::Entero(b) => b.append_null(),
            Col::Real(b) => b.append_null(),
            Col::Logico(b) => b.append_null(),
            Col::Decimal(b) => b.append_null(),
            Col::Fecha(b) => b.append_null(),
            Col::Hora(b) => b.append_null(),
            Col::Marca(b) => b.append_null(),
        }
    }

    fn valor(&mut self, v: Valor) {
        match (self, v) {
            (Col::Texto(b), Valor::Texto(s)) => b.append_value(s),
            (Col::Entero(b), Valor::Entero(n)) => b.append_value(n),
            (Col::Real(b), Valor::Real(x)) => b.append_value(x),
            (Col::Logico(b), Valor::Logico(x)) => b.append_value(x),
            (Col::Decimal(b), Valor::Decimal(n)) => b.append_value(n),
            (Col::Fecha(b), Valor::Fecha(d)) => b.append_value(d),
            (Col::Hora(b), Valor::Hora(us)) => b.append_value(us),
            (Col::Marca(b), Valor::FechaHora(us) | Valor::Instante(us)) => b.append_value(us),
            // `Fisico::analizar` da el valor de su físico, y el constructor
            // es de ese físico: no hay otra pareja.
            (c, _) => c.nulo(),
        }
    }

    fn fin(&mut self) -> ArrayRef {
        match self {
            Col::Texto(b) => Arc::new(b.finish()),
            Col::Entero(b) => Arc::new(b.finish()),
            Col::Real(b) => Arc::new(b.finish()),
            Col::Logico(b) => Arc::new(b.finish()),
            Col::Decimal(b) => Arc::new(b.finish()),
            Col::Fecha(b) => Arc::new(b.finish()),
            Col::Hora(b) => Arc::new(b.finish()),
            Col::Marca(b) => Arc::new(b.finish()),
        }
    }
}

/// El lote a medio hacer de un formato de texto: una celda por columna, cada
/// una analizada con su tipo al llegar.
struct Lote<'p> {
    plan: &'p Plan,
    cols: Vec<Col>,
    rescate: StringBuilder,
    n: usize,
}

impl<'p> Lote<'p> {
    fn nuevo(plan: &'p Plan) -> Lote<'p> {
        Lote {
            plan,
            cols: plan.cols.iter().map(|c| Col::nuevo(&c.fisico)).collect(),
            rescate: StringBuilder::new(),
            n: 0,
        }
    }

    /// Una fila: `celdas[i]` es el texto de la columna `i` del plan (`None`,
    /// nulo), y `extras`, lo que la tabla no declara.
    fn fila(
        &mut self,
        celdas: Vec<Option<String>>,
        extras: Vec<(String, String)>,
        clave: &str,
        numero: u64,
        leido: &mut Leido,
    ) -> Result<(), String> {
        let mut rescatado: BTreeMap<String, Json> = BTreeMap::new();
        for (i, celda) in celdas.into_iter().enumerate() {
            let col = &self.plan.cols[i];
            match celda {
                None => self.cols[i].nulo(),
                Some(t) => match col.fisico.analizar(&t) {
                    Some(v) => self.cols[i].valor(v),
                    None if self.plan.rescata => {
                        self.cols[i].nulo();
                        *leido.rescatados.entry(col.nombre.clone()).or_default() += 1;
                        rescatado.insert(col.nombre.clone(), Json::s(t));
                    }
                    None => {
                        let muestra: String = t.chars().take(40).collect();
                        return Err(format!(
                            "`{clave}`, fila {numero}: `{}` = `{muestra}` no es {}. La tabla no \
                             declara `{RESCATADA}`, así que la lectura se para en vez de dejar \
                             un nulo callado (v1alpha16 03 §1.1)",
                            col.nombre,
                            col.fisico.nombre()
                        ));
                    }
                },
            }
        }
        if self.plan.rescata {
            for (k, v) in extras {
                *leido.rescatados.entry(k.clone()).or_default() += 1;
                rescatado.insert(k, Json::s(v));
            }
            if rescatado.is_empty() {
                self.rescate.append_null();
            } else {
                rescatado.insert("_file".into(), Json::s(clave));
                self.rescate.append_value(Json::Obj(rescatado).jcs());
            }
        }
        self.n += 1;
        Ok(())
    }

    fn vaciar(
        &mut self,
        escritor: &mut arrow_ipc::writer::StreamWriter<&mut dyn Write>,
        leido: &mut Leido,
    ) -> Result<(), String> {
        if self.n == 0 {
            return Ok(());
        }
        let internas: Vec<ArrayRef> = self.cols.iter_mut().map(Col::fin).collect();
        let rescate: ArrayRef = Arc::new(self.rescate.finish());
        self.n = 0;
        emitir(self.plan, &internas, Some(rescate), escritor, leido)
    }
}

/// Del lote interno —una columna por columna física— al de salida: los
/// filtros, y la proyección con el nombre de cada propiedad.
fn emitir(
    plan: &Plan,
    internas: &[ArrayRef],
    rescate: Option<ArrayRef>,
    escritor: &mut arrow_ipc::writer::StreamWriter<&mut dyn Write>,
    leido: &mut Leido,
) -> Result<(), String> {
    leido.freno()?;
    if leido.basta() {
        return Ok(());
    }
    let n = internas.first().map(|c| c.len()).unwrap_or(0);
    let columnas: Vec<ArrayRef> = plan
        .proyeccion
        .iter()
        .map(|(_, col)| {
            if col == RESCATADA {
                rescate
                    .clone()
                    .unwrap_or_else(|| arrow_array::new_null_array(&DataType::Utf8, n))
            } else {
                internas[plan.indice[col]].clone()
            }
        })
        .collect();
    let mut lote = RecordBatch::try_new_with_options(
        plan.salida.clone(),
        columnas,
        &arrow_array::RecordBatchOptions::new().with_row_count(Some(n)),
    )
    .map_err(|e| format!("el lote no casa con su esquema: {e}"))?;
    if !plan.filtros.is_empty() {
        let mut quedan: Option<BooleanArray> = None;
        for c in &plan.filtros {
            let m = c.mascara(&internas[c.col])?;
            quedan = Some(match quedan {
                None => m,
                Some(q) => arrow_arith::boolean::and(&q, &m)
                    .map_err(|e| format!("el filtro no se pudo aplicar: {e}"))?,
            });
        }
        if let Some(q) = quedan {
            lote = arrow_select::filter::filter_record_batch(&lote, &q)
                .map_err(|e| format!("el filtro no se pudo aplicar: {e}"))?;
        }
    }
    // `limit`: lo que falta, y nada más.
    if let Some(l) = leido.limite {
        let faltan = l.saturating_sub(leido.filas) as usize;
        if lote.num_rows() > faltan {
            lote = lote.slice(0, faltan);
        }
    }
    if lote.num_rows() == 0 {
        return Ok(());
    }
    leido.filas += lote.num_rows() as u64;
    escritor
        .write(&lote)
        .map_err(|e| format!("no se pudo escribir el flujo: {e}"))
}

// ── CSV ─────────────────────────────────────────────────────────────────────

/// Un campo de un CSV, y si venía entre comillas: lo que separa el nulo de la
/// cadena vacía (`03` §1.2).
struct Campo {
    texto: String,
    comillas: bool,
}

/// **El analizador de CSV**, en flujo y byte a byte: RFC 4180 (comillas, `""`
/// dentro de ellas, saltos de línea dentro de un valor —medido: 5.495 en las
/// reseñas de Olist—), `\r\n`, y el BOM del principio.
struct Csv<R: BufRead> {
    r: R,
    sep: u8,
    latin1: bool,
    principio: bool,
}

#[derive(PartialEq, Clone, Copy)]
enum Estado {
    Inicio,
    Libre,
    Dentro,
    TrasComilla,
}

fn texto_de(bytes: Vec<u8>, latin1: bool) -> Result<String, String> {
    if latin1 {
        return Ok(bytes.into_iter().map(char::from).collect());
    }
    String::from_utf8(bytes).map_err(|_| {
        "un campo no es UTF-8: si el fichero es latin-1, la tabla lo dice con `encoding`"
            .to_string()
    })
}

impl<R: BufRead> Csv<R> {
    fn registro(&mut self) -> Result<Option<Vec<Campo>>, String> {
        let mut campos: Vec<Campo> = Vec::new();
        let mut campo: Vec<u8> = Vec::new();
        let mut comillas = false;
        let mut e = Estado::Inicio;
        let mut algo = false;
        loop {
            let buf = self
                .r
                .fill_buf()
                .map_err(|e| format!("el fichero dejó de llegar: {e}"))?;
            if buf.is_empty() {
                if !algo {
                    return Ok(None);
                }
                if e == Estado::Dentro {
                    return Err("el fichero acaba dentro de unas comillas sin cerrar".into());
                }
                campos.push(Campo {
                    texto: texto_de(campo, self.latin1)?,
                    comillas,
                });
                return Ok(Some(campos));
            }
            let mut i = 0;
            if self.principio {
                self.principio = false;
                if buf.starts_with(&[0xEF, 0xBB, 0xBF]) {
                    i = 3;
                }
            }
            let (sep, latin1) = (self.sep, self.latin1);
            let mut fin = false;
            while i < buf.len() {
                let b = buf[i];
                i += 1;
                algo = true;
                let cerrar = |campo: &mut Vec<u8>, comillas: &mut bool, campos: &mut Vec<Campo>| {
                    texto_de(std::mem::take(campo), latin1).map(|texto| {
                        campos.push(Campo {
                            texto,
                            comillas: *comillas,
                        });
                        *comillas = false;
                    })
                };
                match e {
                    Estado::Dentro => {
                        if b == b'"' {
                            e = Estado::TrasComilla;
                        } else {
                            campo.push(b);
                        }
                    }
                    Estado::TrasComilla if b == b'"' => {
                        campo.push(b'"');
                        e = Estado::Dentro;
                    }
                    Estado::Inicio if b == b'"' => {
                        comillas = true;
                        e = Estado::Dentro;
                    }
                    _ if b == sep => {
                        cerrar(&mut campo, &mut comillas, &mut campos)?;
                        e = Estado::Inicio;
                    }
                    _ if b == b'\n' => {
                        cerrar(&mut campo, &mut comillas, &mut campos)?;
                        fin = true;
                        break;
                    }
                    _ if b == b'\r' => {}
                    _ => {
                        campo.push(b);
                        e = Estado::Libre;
                    }
                }
            }
            self.r.consume(i);
            if fin {
                return Ok(Some(campos));
            }
        }
    }
}

fn es_latin1(codificacion: Option<&str>) -> Result<bool, String> {
    match codificacion.map(|c| c.to_ascii_lowercase()) {
        None => Ok(false),
        Some(c) if c == "utf-8" || c == "utf8" => Ok(false),
        Some(c) if c == "latin1" || c == "latin-1" || c == "iso-8859-1" => Ok(true),
        Some(c) => Err(format!(
            "`encoding: {c}`: este lector lee utf-8 y latin-1 (iso-8859-1)"
        )),
    }
}

#[allow(clippy::too_many_arguments)]
fn csv(
    o: &dyn Origen,
    obj: &Objeto,
    f: &Fichero,
    plan: &Plan,
    particiones: &BTreeMap<String, String>,
    escritor: &mut arrow_ipc::writer::StreamWriter<&mut dyn Write>,
    leido: &mut Leido,
) -> Result<(), String> {
    if !f.separador.is_ascii() {
        return Err(format!(
            "el separador `{}` no es un carácter ASCII",
            f.separador
        ));
    }
    let lector = o.abrir(&obj.clave, &obj.etag)?;
    let mut csv = Csv {
        r: std::io::BufReader::with_capacity(256 * 1024, lector),
        sep: f.separador as u8,
        latin1: es_latin1(f.codificacion.as_deref())?,
        principio: true,
    };
    let clave = &obj.clave;
    let con_contexto = |e: String, fila: u64| format!("`{clave}`, fila {fila}: {e}");
    // Dónde está cada columna: por la cabecera, o por el orden de la tabla.
    let (posicion, nombres): (Vec<Option<usize>>, Vec<String>) = if f.cabecera {
        let Some(cab) = csv.registro().map_err(|e| con_contexto(e, 1))? else {
            return Ok(());
        };
        let nombres: Vec<String> = cab.into_iter().map(|c| c.texto).collect();
        (
            plan.cols
                .iter()
                .map(|c| nombres.iter().position(|n| *n == c.nombre))
                .collect(),
            nombres,
        )
    } else {
        (
            plan.cols
                .iter()
                .map(|c| plan.orden.iter().position(|n| *n == c.nombre))
                .collect(),
            plan.orden.clone(),
        )
    };
    let mut lote = Lote::nuevo(plan);
    let mut numero = if f.cabecera { 1 } else { 0 };
    while let Some(campos) = csv.registro().map_err(|e| con_contexto(e, numero + 1))? {
        numero += 1;
        // Una línea en blanco no es una fila.
        if campos.len() == 1 && campos[0].texto.is_empty() && !campos[0].comillas {
            continue;
        }
        let celdas: Vec<Option<String>> = plan
            .cols
            .iter()
            .zip(&posicion)
            .map(|(c, pos)| {
                if c.particion {
                    return particiones.get(&c.nombre).cloned();
                }
                let campo = campos.get((*pos)?)?;
                (campo.comillas || !campo.texto.is_empty()).then(|| campo.texto.clone())
            })
            .collect();
        let extras: Vec<(String, String)> = if plan.rescata {
            campos
                .iter()
                .enumerate()
                .filter(|(i, c)| {
                    (c.comillas || !c.texto.is_empty())
                        && nombres.get(*i).is_none_or(|n| !plan.declaradas.contains(n))
                })
                .map(|(i, c)| {
                    (
                        nombres.get(i).cloned().unwrap_or_else(|| format!("_c{i}")),
                        c.texto.clone(),
                    )
                })
                .collect()
        } else {
            Vec::new()
        };
        lote.fila(celdas, extras, clave, numero, leido)?;
        if lote.n >= LOTE {
            lote.vaciar(escritor, leido)?;
            if leido.basta() {
                return Ok(());
            }
        }
    }
    lote.vaciar(escritor, leido)
}

// ── JSONL ───────────────────────────────────────────────────────────────────

/// El texto de un valor JSON para analizarlo con el tipo de su columna: una
/// cadena es su contenido; un número, un lógico o algo anidado, su JSON.
fn texto_json(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::Null => None,
        serde_json::Value::String(s) => Some(s.clone()),
        otro => Some(otro.to_string()),
    }
}

fn jsonl(
    o: &dyn Origen,
    obj: &Objeto,
    plan: &Plan,
    particiones: &BTreeMap<String, String>,
    escritor: &mut arrow_ipc::writer::StreamWriter<&mut dyn Write>,
    leido: &mut Leido,
) -> Result<(), String> {
    let lector = std::io::BufReader::with_capacity(256 * 1024, o.abrir(&obj.clave, &obj.etag)?);
    let clave = &obj.clave;
    let mut lote = Lote::nuevo(plan);
    for (i, linea) in lector.lines().enumerate() {
        let numero = i as u64 + 1;
        let linea = linea.map_err(|e| format!("`{clave}`, línea {numero}: {e}"))?;
        let linea = linea.trim();
        if linea.is_empty() {
            continue;
        }
        let v: serde_json::Value = serde_json::from_str(linea)
            .map_err(|e| format!("`{clave}`, línea {numero}: no es JSON ({e})"))?;
        let serde_json::Value::Object(m) = v else {
            return Err(format!("`{clave}`, línea {numero}: no es un objeto JSON"));
        };
        let celdas = plan
            .cols
            .iter()
            .map(|c| {
                if c.particion && !m.contains_key(&c.nombre) {
                    return particiones.get(&c.nombre).cloned();
                }
                m.get(&c.nombre).and_then(texto_json)
            })
            .collect();
        let extras = if plan.rescata {
            m.iter()
                .filter(|(k, v)| !plan.declaradas.contains(*k) && !v.is_null())
                .filter_map(|(k, v)| Some((k.clone(), texto_json(v)?)))
                .collect()
        } else {
            Vec::new()
        };
        lote.fila(celdas, extras, clave, numero, leido)?;
        if lote.n >= LOTE {
            lote.vaciar(escritor, leido)?;
            if leido.basta() {
                return Ok(());
            }
        }
    }
    lote.vaciar(escritor, leido)
}

// ── Parquet ─────────────────────────────────────────────────────────────────

/// **Un Parquet en trozos**: lo pedido por rangos de un grupo de filas, y
/// nada más. El lector de `parquet` pide cada trozo de columna; aquí se sirve
/// de lo ya bajado, y lo que no se bajó es un error (no una petición oculta).
struct Disperso {
    tamano: u64,
    trozos: Vec<(u64, bytes::Bytes)>,
}

impl Disperso {
    fn trozo(&self, desde: u64, largo: u64) -> parquet::errors::Result<bytes::Bytes> {
        for (inicio, b) in &self.trozos {
            let fin = inicio + b.len() as u64;
            if desde >= *inicio && desde + largo <= fin {
                let a = (desde - inicio) as usize;
                return Ok(b.slice(a..a + largo as usize));
            }
        }
        Err(parquet::errors::ParquetError::General(format!(
            "el lector pidió {desde}+{largo}, que no se bajó"
        )))
    }
}

impl parquet::file::reader::Length for Disperso {
    fn len(&self) -> u64 {
        self.tamano
    }
}

impl parquet::file::reader::ChunkReader for Disperso {
    type T = std::io::Cursor<bytes::Bytes>;

    fn get_read(&self, start: u64) -> parquet::errors::Result<Self::T> {
        let (inicio, b) = self
            .trozos
            .iter()
            .find(|(i, b)| start >= *i && start < i + b.len() as u64)
            .ok_or_else(|| {
                parquet::errors::ParquetError::General(format!(
                    "el lector pidió desde {start}, que no se bajó"
                ))
            })?;
        Ok(std::io::Cursor::new(b.slice((start - inicio) as usize..)))
    }

    fn get_bytes(&self, start: u64, length: usize) -> parquet::errors::Result<bytes::Bytes> {
        self.trozo(start, length as u64)
    }
}

/// Los rangos `(desde, largo)` juntando los que están a menos de [`HUECO`].
fn juntar(mut rangos: Vec<(u64, u64)>) -> Vec<(u64, u64)> {
    rangos.sort();
    let mut out: Vec<(u64, u64)> = Vec::new();
    for (d, l) in rangos {
        match out.last_mut() {
            Some((d0, l0)) if d <= *d0 + *l0 + HUECO => *l0 = (*l0).max(d + l - *d0),
            _ => out.push((d, l)),
        }
    }
    out
}

fn parquet(
    o: &dyn Origen,
    obj: &Objeto,
    plan: &Plan,
    particiones: &BTreeMap<String, String>,
    umbral: u64,
    escritor: &mut arrow_ipc::writer::StreamWriter<&mut dyn Write>,
    leido: &mut Leido,
) -> Result<(), String> {
    use parquet::arrow::ProjectionMask;
    use parquet::arrow::arrow_reader::{ArrowReaderMetadata, ParquetRecordBatchReaderBuilder};
    let clave = &obj.clave;
    let error = |e: parquet::errors::ParquetError| format!("`{clave}` no se lee como Parquet: {e}");
    let del_fichero: BTreeSet<&str> = plan
        .cols
        .iter()
        .filter(|c| !c.particion)
        .map(|c| c.nombre.as_str())
        .collect();
    let mascara = |d: &parquet::schema::types::SchemaDescriptor| {
        let hojas: Vec<usize> = d
            .columns()
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                c.path()
                    .parts()
                    .first()
                    .is_some_and(|p| del_fichero.contains(p.as_str()))
            })
            .map(|(i, _)| i)
            .collect();
        (ProjectionMask::leaves(d, hojas.iter().copied()), hojas)
    };
    let convertir = |lote: RecordBatch,
                     leido: &mut Leido,
                     escritor: &mut arrow_ipc::writer::StreamWriter<&mut dyn Write>|
     -> Result<(), String> {
        let n = lote.num_rows();
        let internas = plan
            .cols
            .iter()
            .map(|c| {
                let tipo = tipo_arrow(&c.fisico);
                if c.particion {
                    let v = particiones.get(&c.nombre).cloned();
                    let a: ArrayRef = Arc::new(StringArray::from(vec![v; n]));
                    return arrow_cast::cast(&a, &tipo).map_err(|e| {
                        format!(
                            "la partición `{}` de `{clave}` no es {}: {e}",
                            c.nombre,
                            c.fisico.nombre()
                        )
                    });
                }
                match lote.column_by_name(&c.nombre) {
                    None => Ok(arrow_array::new_null_array(&tipo, n)),
                    Some(a) if *a.data_type() == tipo => Ok(a.clone()),
                    Some(a) => arrow_cast::cast_with_options(
                        a,
                        &tipo,
                        &arrow_cast::CastOptions {
                            safe: false,
                            ..Default::default()
                        },
                    )
                    .map_err(|e| {
                        format!(
                            "`{}` de `{clave}` es `{}` y la tabla dice {}: {e}",
                            c.nombre,
                            a.data_type(),
                            c.fisico.nombre()
                        )
                    }),
                }
            })
            .collect::<Result<Vec<_>, String>>()?;
        emitir(plan, &internas, None, escritor, leido)
    };

    if obj.tamano <= umbral {
        let mut bytes = Vec::with_capacity(obj.tamano as usize);
        o.abrir(clave, &obj.etag)?
            .read_to_end(&mut bytes)
            .map_err(|e| format!("`{clave}` dejó de llegar: {e}"))?;
        let b =
            ParquetRecordBatchReaderBuilder::try_new(bytes::Bytes::from(bytes)).map_err(error)?;
        let (m, _) = mascara(b.parquet_schema());
        for lote in b
            .with_projection(m)
            .with_batch_size(LOTE)
            .build()
            .map_err(error)?
        {
            convertir(
                lote.map_err(|e| format!("`{clave}`: {e}"))?,
                leido,
                escritor,
            )?;
            if leido.basta() {
                return Ok(());
            }
        }
        return Ok(());
    }

    // Grande: el pie por su sufijo, y cada grupo de filas por sus trozos.
    let cola = o.rango_de(clave, "-8", &obj.etag)?;
    if cola.len() < 8 || &cola[4..8] != b"PAR1" {
        return Err(format!("`{clave}` no termina como un Parquet (`PAR1`)"));
    }
    let n = u32::from_le_bytes([cola[0], cola[1], cola[2], cola[3]]) as u64;
    let pie = o.rango_de(clave, &format!("-{}", n + 8), &obj.etag)?;
    let meta = parquet::file::metadata::ParquetMetaDataReader::decode_metadata(&pie[..n as usize])
        .map_err(error)?;
    let meta = ArrowReaderMetadata::try_new(Arc::new(meta), Default::default()).map_err(error)?;
    let (m, hojas) = mascara(meta.metadata().file_metadata().schema_descr());
    for g in 0..meta.metadata().num_row_groups() {
        let grupo = meta.metadata().row_group(g);
        // Lo que sus estadísticas dicen que no puede cumplir, no se baja.
        if !plan.grupo_puede(grupo) {
            leido.grupos_descartados += 1;
            continue;
        }
        let rangos = juntar(
            hojas
                .iter()
                .map(|h| grupo.column(*h).byte_range())
                .collect(),
        );
        let mut trozos = Vec::with_capacity(rangos.len());
        for (d, l) in rangos {
            let b = o.rango_de(clave, &format!("{d}-{}", d + l - 1), &obj.etag)?;
            trozos.push((d, bytes::Bytes::from(b)));
        }
        let disperso = Disperso {
            tamano: obj.tamano,
            trozos,
        };
        let lector = ParquetRecordBatchReaderBuilder::new_with_metadata(disperso, meta.clone())
            .with_row_groups(vec![g])
            .with_projection(m.clone())
            .with_batch_size(LOTE)
            .build()
            .map_err(error)?;
        for lote in lector {
            convertir(
                lote.map_err(|e| format!("`{clave}`: {e}"))?,
                leido,
                escritor,
            )?;
            if leido.basta() {
                return Ok(());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow_array::Array as _;
    use arrow_array::cast::AsArray;
    use ore_objetos::memoria::EnMemoria;

    fn peticion(tipo: &str, objeto: &str, proy: &[&str], tipos: &[(&str, &str)]) -> Peticion {
        Peticion {
            url: "s3://b".into(),
            objeto: objeto.into(),
            proyeccion: proy
                .iter()
                .map(|c| (c.to_string(), c.to_string()))
                .collect(),
            formato: Some("arrow".into()),
            fichero: Some(Fichero {
                tipo: tipo.into(),
                cabecera: true,
                separador: ',',
                tipos: tipos
                    .iter()
                    .map(|(c, t)| (c.to_string(), t.to_string()))
                    .collect(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    /// Lee y devuelve los lotes del flujo, ya juntos.
    fn filas(o: &EnMemoria, p: &Peticion, umbral: u64) -> Result<(RecordBatch, Leido), String> {
        let mut buf: Vec<u8> = Vec::new();
        let l = leer(o, p, &mut buf, umbral)?;
        let r = arrow_ipc::reader::StreamReader::try_new(std::io::Cursor::new(buf), None)
            .expect("un flujo Arrow con su marca de fin");
        let esquema = r.schema();
        let lotes: Vec<RecordBatch> = r.map(|l| l.unwrap()).collect();
        Ok((
            arrow_select::concat::concat_batches(&esquema, &lotes).unwrap(),
            l,
        ))
    }

    fn texto(b: &RecordBatch, col: &str) -> Vec<Option<String>> {
        b.column_by_name(col)
            .unwrap()
            .as_string::<i32>()
            .iter()
            .map(|v| v.map(String::from))
            .collect()
    }

    /// 0053 F9·1 · **El listado de un `ObjectTable`, como filas**: lo que
    /// casa con su `match` bajo el prefijo, con las columnas pedidas (las que
    /// el listado no da, nulas), una partición y el `limit`.
    #[test]
    fn el_listado_de_un_object_table_son_filas() {
        let o = EnMemoria::con(&[
            ("docs/anio=2026/a.pdf", b"aaaa".to_vec()),
            ("docs/anio=2025/b.pdf", b"bb".to_vec()),
            ("docs/anio=2026/c.txt", b"c".to_vec()),
            ("otros/d.pdf", b"d".to_vec()),
        ]);
        let mut p = Peticion {
            url: "s3://b".into(),
            objeto: "docs/".into(),
            proyeccion: ["key", "size", "modified", "contentType", "anio"]
                .iter()
                .map(|c| (c.to_string(), c.to_string()))
                .collect(),
            formato: Some("arrow".into()),
            listado: Some("**/*.pdf".into()),
            ..Default::default()
        };
        let (b, l) = filas(&o, &p, UMBRAL).unwrap();
        assert_eq!(l.filas, 2);
        assert_eq!(
            texto(&b, "key"),
            [
                Some("docs/anio=2025/b.pdf".into()),
                Some("docs/anio=2026/a.pdf".into())
            ]
        );
        assert_eq!(
            texto(&b, "anio"),
            [Some("2025".into()), Some("2026".into())]
        );
        assert_eq!(texto(&b, "contentType"), [None, None]);
        let tam = b
            .column_by_name("size")
            .unwrap()
            .as_primitive::<arrow_array::types::Int64Type>();
        assert_eq!(tam.values().to_vec(), [2, 4]);
        assert!(!b.column_by_name("modified").unwrap().is_null(0));
        p.limit = Some(1);
        assert_eq!(filas(&o, &p, UMBRAL).unwrap().1.filas, 1);
        p.limit = None;
        p.filtros = vec![ore_driver::Filtro::uno("key", "eq", "x")];
        assert!(
            filas(&o, &p, UMBRAL).is_err(),
            "los filtros no se empujan al listado"
        );
    }

    const TIPOS: &[(&str, &str)] = &[
        ("id", "String"),
        ("n", "Integer"),
        ("cuando", "DateTime"),
        ("nota", "String"),
    ];

    /// **Lo vacío y las comillas** (`03` §1.2): vacío sin comillas es nulo,
    /// `""` es la cadena vacía; un salto de línea y unas comillas dentro de
    /// un valor son del valor; el BOM no es del nombre; una línea en blanco no
    /// es una fila. Y cada valor, en su tipo.
    #[test]
    fn un_csv_se_lee_con_la_regla_de_postgres() {
        let csv = "\u{feff}id,n,cuando,nota\r\n\
                   \"01\",7,2017-10-02 10:56:33,\"dijo \"\"hola\"\"\ny se fue\"\r\n\
                   02,,,\"\"\r\n\
                   \r\n\
                   03,-2,2018-01-18 00:00:00,\r\n";
        let o = EnMemoria::con(&[("t/a.csv", csv.as_bytes().to_vec())]);
        let p = peticion("csv", "t/a.csv", &["id", "n", "cuando", "nota"], TIPOS);
        let (b, l) = filas(&o, &p, UMBRAL).expect("lee");
        assert_eq!(b.num_rows(), 3);
        assert_eq!(l.filas, 3);
        assert_eq!(
            texto(&b, "id"),
            [Some("01".into()), Some("02".into()), Some("03".into())],
            "los ceros a la izquierda de un código se quedan"
        );
        assert_eq!(
            texto(&b, "nota"),
            [
                Some("dijo \"hola\"\ny se fue".into()),
                Some("".into()),
                None
            ],
            "`\"\"` es la cadena vacía y el vacío sin comillas, nulo"
        );
        let n = b
            .column_by_name("n")
            .unwrap()
            .as_primitive::<arrow_array::types::Int64Type>();
        assert_eq!(n.iter().collect::<Vec<_>>(), [Some(7), None, Some(-2)]);
        assert_eq!(
            b.column_by_name("cuando").unwrap().data_type(),
            &DataType::Timestamp(TimeUnit::Microsecond, None)
        );
        assert_eq!(b.column_by_name("cuando").unwrap().null_count(), 1);
    }

    /// **Sin `_rescued_data`, lo que no encaja para la lectura** —con el
    /// fichero, la fila, la columna y el valor—, y nunca es un nulo callado.
    #[test]
    fn sin_rescatada_un_valor_que_no_encaja_para() {
        let o = EnMemoria::con(&[("t/a.csv", b"id,n\n1,7\n2,siete\n".to_vec())]);
        let p = peticion("csv", "t/a.csv", &["id", "n"], TIPOS);
        let e = filas(&o, &p, UMBRAL).expect_err("para");
        assert!(e.contains("`t/a.csv`, fila 3"), "{e}");
        assert!(e.contains("`n` = `siete` no es Integer"), "{e}");
        assert!(e.contains("_rescued_data"), "{e}");
    }

    /// **Con `_rescued_data`, se rescata y se sigue** (`03` §1.1): el valor,
    /// nulo en su columna y su texto en la rescatada con el fichero; una
    /// columna que la tabla no declara, también; la fila sin nada, nula.
    #[test]
    fn con_rescatada_lo_que_no_encaja_va_a_ella_con_su_fichero() {
        let o = EnMemoria::con(&[("t/a.csv", b"id,n,sobra\n1,7,\n2,siete,x\n".to_vec())]);
        let mut tipos = TIPOS.to_vec();
        tipos.push((RESCATADA, "String"));
        let p = peticion("csv", "t/a.csv", &["id", "n", RESCATADA], &tipos);
        let (b, l) = filas(&o, &p, UMBRAL).expect("lee");
        assert_eq!(b.num_rows(), 2);
        assert_eq!(b.column_by_name("n").unwrap().null_count(), 1);
        assert_eq!(
            texto(&b, RESCATADA),
            [
                None,
                Some(r#"{"_file":"t/a.csv","n":"siete","sobra":"x"}"#.into())
            ]
        );
        assert_eq!(l.rescatados["n"], 1);
        assert_eq!(l.rescatados["sobra"], 1);
    }

    /// Un CSV **sin cabecera** se lee por el orden de la tabla, con su
    /// separador.
    #[test]
    fn sin_cabecera_manda_el_orden_de_la_tabla() {
        let o = EnMemoria::con(&[("t/a.csv", b"x;5\ny;6\n".to_vec())]);
        let mut p = peticion(
            "csv",
            "t/a.csv",
            &["n", "id"],
            &[("id", "String"), ("n", "Integer")],
        );
        let f = p.fichero.as_mut().unwrap();
        f.cabecera = false;
        f.separador = ';';
        let (b, _) = filas(&o, &p, UMBRAL).expect("lee");
        assert_eq!(texto(&b, "id"), [Some("x".into()), Some("y".into())]);
        assert_eq!(
            b.schema().field(0).name(),
            "n",
            "sale en el orden de la proyección"
        );
    }

    /// **JSONL**: la clave ausente y `null` son nulo; `""` es la cadena vacía;
    /// un número va a una columna de texto como su JSON; una cadena que no es
    /// un entero se rescata, y una clave que la tabla no declara, también.
    #[test]
    fn un_jsonl_se_lee_con_sus_tipos() {
        let j = "{\"id\":\"a\",\"n\":1,\"nota\":\"\"}\n\
                 \n\
                 {\"id\":5,\"n\":\"dos\",\"nuevo\":true}\n\
                 {\"id\":\"c\",\"n\":null}\n";
        let o = EnMemoria::con(&[("logs/d1.jsonl", j.as_bytes().to_vec())]);
        let mut tipos = TIPOS.to_vec();
        tipos.push((RESCATADA, "String"));
        let p = peticion("jsonl", "logs/", &["id", "n", "nota", RESCATADA], &tipos);
        let (b, _) = filas(&o, &p, UMBRAL).expect("lee");
        assert_eq!(b.num_rows(), 3);
        assert_eq!(
            texto(&b, "id"),
            [Some("a".into()), Some("5".into()), Some("c".into())]
        );
        assert_eq!(texto(&b, "nota"), [Some("".into()), None, None]);
        assert_eq!(b.column_by_name("n").unwrap().null_count(), 2);
        assert_eq!(
            texto(&b, RESCATADA)[1].as_deref(),
            Some(r#"{"_file":"logs/d1.jsonl","n":"dos","nuevo":"true"}"#)
        );
    }

    fn un_parquet(filas_por_grupo: usize, ids: &[&str], unidades: &[i32]) -> Vec<u8> {
        use arrow_array::{Int32Array, StringArray};
        let esquema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, true),
            Field::new("unidades", DataType::Int32, true),
            Field::new("nota", DataType::Utf8, true),
        ]));
        let lote = RecordBatch::try_new(
            esquema.clone(),
            vec![
                Arc::new(StringArray::from(ids.to_vec())),
                Arc::new(Int32Array::from(unidades.to_vec())),
                Arc::new(StringArray::from(vec!["x".repeat(2000); ids.len()])),
            ],
        )
        .unwrap();
        let mut buf = Vec::new();
        let props = parquet::file::properties::WriterProperties::builder()
            .set_max_row_group_row_count(Some(filas_por_grupo))
            .set_dictionary_enabled(false)
            .build();
        let mut w = parquet::arrow::ArrowWriter::try_new(&mut buf, esquema, Some(props)).unwrap();
        w.write(&lote).unwrap();
        w.close().unwrap();
        buf
    }

    /// **Parquet con partición Hive y `match`**: `fecha` sale del camino,
    /// `unidades` (int32) llega como `Integer`, y lo que el patrón no nombra
    /// no se lee.
    #[test]
    fn un_parquet_particionado_se_lee_con_su_particion() {
        let o = EnMemoria::con(&[
            (
                "v/p/fecha=2026-09-01/a.parquet",
                un_parquet(10, &["a", "b"], &[1, 2]),
            ),
            (
                "v/p/fecha=2026-09-02/b.parquet",
                un_parquet(10, &["c"], &[3]),
            ),
            ("v/p/_SUCCESS", Vec::new()),
        ]);
        let mut p = peticion(
            "parquet",
            "v/p/",
            &["id", "unidades", "fecha"],
            &[
                ("id", "String"),
                ("unidades", "Integer"),
                ("fecha", "String"),
            ],
        );
        let f = p.fichero.as_mut().unwrap();
        f.patron = Some("**/*.parquet".into());
        f.particiones = vec!["fecha".into()];
        let (b, l) = filas(&o, &p, UMBRAL).expect("lee");
        assert_eq!(l.ficheros, 2, "`_SUCCESS` no es del patrón");
        assert_eq!(
            texto(&b, "fecha"),
            [
                Some("2026-09-01".into()),
                Some("2026-09-01".into()),
                Some("2026-09-02".into())
            ]
        );
        assert_eq!(
            b.column_by_name("unidades").unwrap().data_type(),
            &DataType::Int64
        );
    }

    /// **Un Parquet grande, por rangos**: por encima del umbral se leen el pie
    /// y, grupo a grupo, sólo los trozos de las columnas pedidas. Las mismas
    /// filas que bajándolo entero, y muchos menos bytes: `nota`, que no se
    /// pide, es casi todo el fichero.
    #[test]
    fn un_parquet_grande_se_lee_por_rangos_y_solo_lo_pedido() {
        let ids: Vec<String> = (0..300).map(|i| format!("id{i}")).collect();
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        let unidades: Vec<i32> = (0..300).collect();
        let fichero = un_parquet(100, &ids, &unidades);
        let tamano = fichero.len();
        let p = peticion(
            "parquet",
            "g/a.parquet",
            &["id", "unidades"],
            &[
                ("id", "String"),
                ("unidades", "Integer"),
                ("nota", "String"),
            ],
        );
        let o = EnMemoria::con(&[("g/a.parquet", fichero.clone())]);
        let (entero, _) = filas(&o, &p, UMBRAL).expect("entero");
        let o = EnMemoria::con(&[("g/a.parquet", fichero)]);
        let (troceado, l) = filas(&o, &p, 1024).expect("por rangos");
        assert_eq!(l.filas, 300);
        assert_eq!(entero, troceado, "las mismas filas por los dos caminos");
        assert_eq!(
            o.lecturas.get(),
            2 + 3,
            "el pie (2) y un trozo por grupo de filas (3)"
        );
        assert!(
            o.bytes_leidos.get() * 10 < tamano,
            "{} de {tamano} bytes: se bajó lo que no se pidió",
            o.bytes_leidos.get()
        );
    }

    /// **Fijado a lo que el listado dijo**: un fichero reescrito entre el
    /// listado y la lectura para la lectura con `412`, en vez de mezclar dos
    /// versiones.
    #[test]
    fn un_fichero_que_cambia_mientras_se_lee_para() {
        let mut o = EnMemoria::con(&[("t/a.csv", b"id\n1\n".to_vec())]);
        o.cambiadas.insert("t/a.csv".into());
        let p = peticion("csv", "t/a.csv", &["id"], TIPOS);
        let e = filas(&o, &p, UMBRAL).expect_err("para");
        assert!(e.contains("cambió mientras se leía (412)"), "{e}");
    }

    /// **El filtro de igualdad**, con el valor en el tipo de su columna, y
    /// sobre una columna que no se proyecta.
    #[test]
    fn el_filtro_recorta_por_igualdad_en_su_tipo() {
        let o = EnMemoria::con(&[("t/a.csv", b"id,n\na,1\nb,2\nc,1\n".to_vec())]);
        let mut p = peticion("csv", "t/a.csv", &["id"], TIPOS);
        p.filtros = vec![ore_driver::Filtro::uno("n", "eq", "1")];
        let (b, l) = filas(&o, &p, UMBRAL).expect("lee");
        assert_eq!(texto(&b, "id"), [Some("a".into()), Some("c".into())]);
        assert_eq!(l.filas, 2);
        p.filtros = vec![ore_driver::Filtro::uno("n", "eq", "uno")];
        let e = filas(&o, &p, UMBRAL).expect_err("un literal que no es del tipo");
        assert!(e.contains("no es un Integer"), "{e}");
    }

    /// **Los diez operadores sobre las filas** (0053 F2·4), con la semántica
    /// de SQL: un nulo no cumple nada salvo `isNull`.
    #[test]
    fn los_operadores_v2_sobre_las_filas() {
        use ore_driver::{Filtro, Valor as D};
        let o = EnMemoria::con(&[("t/a.csv", b"id,n\na,1\nb,2\nc,\nd,5\n_x,3\n".to_vec())]);
        let base = peticion("csv", "t/a.csv", &["id"], TIPOS);
        let ids = |f: Filtro| -> Vec<String> {
            let mut p = base.clone();
            p.filtros = vec![f];
            let (b, _) = filas(&o, &p, UMBRAL).expect("lee");
            texto(&b, "id").into_iter().flatten().collect()
        };
        let lista = |op: &str, v: &[&str]| Filtro {
            columna: "n".into(),
            operador: op.into(),
            valor: D::Lista(v.iter().map(|x| x.to_string()).collect()),
        };
        assert_eq!(ids(Filtro::uno("n", "neq", "2")), ["a", "d", "_x"]);
        assert_eq!(ids(Filtro::uno("n", "ge", "3")), ["d", "_x"]);
        assert_eq!(ids(Filtro::uno("n", "lt", "2")), ["a"]);
        assert_eq!(ids(Filtro::uno("n", "le", "2")), ["a", "b"]);
        assert_eq!(ids(Filtro::uno("n", "gt", "4")), ["d"]);
        assert_eq!(ids(lista("in", &["1", "5"])), ["a", "d"]);
        assert!(ids(lista("in", &[])).is_empty());
        assert_eq!(
            ids(Filtro {
                columna: "n".into(),
                operador: "isNull".into(),
                valor: D::Ninguno
            }),
            ["c"]
        );
        assert_eq!(
            ids(Filtro {
                columna: "n".into(),
                operador: "isNotNull".into(),
                valor: D::Ninguno
            })
            .len(),
            4
        );
        assert_eq!(ids(Filtro::uno("id", "like", "_")), ["a", "b", "c", "d"]);
        let mut p = base.clone();
        p.filtros = vec![Filtro::uno("n", "like", "1")];
        let e = filas(&o, &p, UMBRAL).expect_err("like sobre un entero");
        assert!(e.contains("no es texto"), "{e}");
        p.filtros.clear();
        p.orden = vec![ore_driver::Orden {
            columna: "n".into(),
            descendente: false,
        }];
        assert!(filas(&o, &p, UMBRAL).is_err(), "`orderBy` no se declara");
    }

    /// **Una partición que no cumple no se baja**, y `estimar` lo dice sin
    /// leer nada más que el listado.
    #[test]
    fn una_particion_que_no_cumple_no_se_baja() {
        let o = EnMemoria::con(&[
            (
                "v/p/fecha=2026-09-01/a.parquet",
                un_parquet(10, &["a", "b"], &[1, 2]),
            ),
            (
                "v/p/fecha=2026-09-02/b.parquet",
                un_parquet(10, &["c"], &[3]),
            ),
            (
                "v/p/fecha=2026-09-03/c.parquet",
                un_parquet(10, &["d"], &[4]),
            ),
        ]);
        let mut p = peticion(
            "parquet",
            "v/p/",
            &["id"],
            &[("id", "String"), ("unidades", "Integer"), ("fecha", "Date")],
        );
        let f = p.fichero.as_mut().unwrap();
        f.patron = Some("**/*.parquet".into());
        f.particiones = vec!["fecha".into()];
        p.filtros = vec![ore_driver::Filtro::uno("fecha", "ge", "2026-09-02")];
        let (b, l) = filas(&o, &p, UMBRAL).expect("lee");
        assert_eq!(texto(&b, "id"), [Some("c".into()), Some("d".into())]);
        assert_eq!(l.descartados, 1);
        assert_eq!(o.lecturas.get(), 2, "la partición que no cumple no se baja");
        let e = estimar(&o, &p).expect("estima");
        assert!(
            e.contains("\"ficheros\":2") && e.contains("\"descartados\":1"),
            "{e}"
        );
        assert_eq!(o.lecturas.get(), 2, "estimar no lee");
    }

    /// **Un grupo de filas que no puede cumplir no se baja**: sus
    /// estadísticas (mínimo y máximo) lo dicen desde el pie.
    #[test]
    fn un_grupo_de_filas_que_no_puede_cumplir_no_se_baja() {
        let ids: Vec<String> = (0..300).map(|i| format!("id{i}")).collect();
        let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
        let unidades: Vec<i32> = (0..300).collect();
        let o = EnMemoria::con(&[("g/a.parquet", un_parquet(100, &ids, &unidades))]);
        let mut p = peticion(
            "parquet",
            "g/a.parquet",
            &["id"],
            &[
                ("id", "String"),
                ("unidades", "Integer"),
                ("nota", "String"),
            ],
        );
        p.filtros = vec![ore_driver::Filtro::uno("unidades", "ge", "250")];
        let (_, l) = filas(&o, &p, 1024).expect("por rangos");
        assert_eq!(l.filas, 50);
        assert_eq!(l.grupos_descartados, 2);
        assert_eq!(o.lecturas.get(), 2 + 1, "el pie (2) y sólo el tercer grupo");
    }

    /// **`limit` deja de leer**: el tercer fichero no se baja.
    #[test]
    fn limit_deja_de_listar_y_de_leer() {
        let o = EnMemoria::con(&[
            ("q/a.parquet", un_parquet(10, &["a", "b"], &[1, 2])),
            ("q/b.parquet", un_parquet(10, &["c", "d"], &[3, 4])),
            ("q/c.parquet", un_parquet(10, &["e", "f"], &[5, 6])),
        ]);
        let mut p = peticion(
            "parquet",
            "q/",
            &["id"],
            &[("id", "String"), ("unidades", "Integer")],
        );
        p.limit = Some(3);
        let (b, l) = filas(&o, &p, UMBRAL).expect("lee");
        assert_eq!(b.num_rows(), 3);
        assert_eq!(l.filas, 3);
        assert_eq!(o.lecturas.get(), 2, "el tercero no se baja");
        p.limit = None;
        p.timeout_ms = Some(0);
        let e = filas(&o, &p, UMBRAL).expect_err("sin tiempo");
        assert!(e.starts_with(AGOTADO), "{e}");
    }

    #[test]
    fn el_glob_de_match() {
        assert!(casa("*.jsonl", "d1.jsonl"));
        assert!(!casa("*.jsonl", "x/d1.jsonl"), "`*` no cruza `/`");
        assert!(casa("**/*.parquet", "fecha=1/a.parquet"));
        assert!(
            casa("**/*.parquet", "a.parquet"),
            "`**/` vale por ninguna carpeta"
        );
        assert!(casa("**/*.parquet", "a/b/c.parquet"));
        assert!(!casa("**/*.parquet", "a/b/_SUCCESS"));
        assert!(casa("d?.csv", "d1.csv"));
    }
}
