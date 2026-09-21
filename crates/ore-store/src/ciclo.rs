//! **El ciclo del almacén delegado**, fuera del compilador — lo que `ore-store-r2`
//! y `ore-store-gcs` tienen en común: todo menos el transporte.
//!
//! Normativo: [ADR 0015](../../../docs/decisions/0015-el-protocolo-del-almacen.md),
//! revisado por [0031 §10](../../../docs/decisions/0031-el-puesto.md) (W3.6a,
//! 2026-09-20: **la copia es un dataset**). Es la **tercera** vez que este
//! árbol delega, y por la misma razón que las dos anteriores: `ore` no puede
//! abrir un socket, y no por promesa — `ore-cli/tests/dependencias.rs` lee el
//! `Cargo.lock` y falla si aparece una crate de red, de TLS o de FFI en su
//! cierre.
//!
//! | | qué delega | ADR |
//! |---|---|---|
//! | `ore-read-<tipo>` | leer filas de un origen | 0008 |
//! | `ore-maintain` | correr el circuito Δ | 0013 |
//! | **`ore-store-<tipo>`** | **escribir el dataset y devolver su puntero** | **0015 · 0031 §10** |
//!
//! # El protocolo
//!
//! Hereda la línea de 0008 —*«la petición es un fragmento del plan, no SQL»*—
//! llevada a su sitio:
//!
//! > **Lo que viaja no son llamadas al almacén: son las filas y el puntero.**
//!
//! - **stdin**: la petición en JSON canónico, **una línea**, y después las filas,
//!   **una por línea**, como objetos JSON de cadenas. Por stdin y no por `argv`
//!   por lo mismo de siempre: `argv` lo lee cualquier proceso, y una fila es un
//!   dato;
//! - **stdout**: una línea JSON con el `metadata_location` nuevo, el snapshot y
//!   las cuentas;
//! - **stderr**: lo que haya que contar.
//!
//! Este programa **no sabe qué es una entidad, ni un conducto, ni una vista.**
//! Recibe una cabecera y un flujo de filas, y escribe una tabla Iceberg.
//!
//! # Lo que cambió con el dataset (W3.6a)
//!
//! Hasta aquí el almacén guardaba **el estado**: un recibo por cabecera decía
//! qué artefacto era el vigente, y `buscar`/`anterior` lo consultaban. Desde
//! W3.6a el estado vive **en el árbol** —`copias/<p>_<v>.json` apunta al
//! `metadata.json` de la tabla— y el árbol es el catálogo: el commit del Job
//! es el *swap* y la forja el *compare-and-set*. Así que:
//!
//! - **`buscar`** ya no busca un recibo: comprueba que el puntero que el árbol
//!   trae sigue existiendo en el bucket;
//! - **`anterior`** desaparece: sobre qué fundir y desde dónde leer lo sabe el
//!   puntero, y lo decide `ore`;
//! - **`sellar`** recibe `dataset` y, si la tabla ya existe, `base` (su
//!   `metadata_location`) y `fundir`; devuelve el `metadata_location` nuevo.
//!   Rehacer es sobrescribir, y la historia queda en los snapshots;
//! - **`recoger`** expira snapshots y retira lo que ningún snapshot nombra;
//!   **`recoger-huerfanas`** retira los datasets que ningún puntero reclama y
//!   los sobres heredados que ningún puntero sigue nombrando;
//! - **`leer`** abre la tabla por `metadata_location` (o el sobre por `clave`,
//!   mientras quede alguno) y devuelve la cabecera y las filas, como siempre.
//!
//! # Lo que cambió con el verbo escribir (W3.6c, 0031 §11)
//!
//! Un puesto escribe con `write()` y un motor de fuera (PyIceberg, DuckDB,
//! Spark) escribe por el catálogo REST que `ore-serve` habla; los dos hacen
//! **lo mismo en dos mitades**: el escritor deja los ficheros de datos, los
//! manifiestos y la lista en el bucket, y el catálogo aplica `requirements` +
//! `updates` y escribe el `metadata.json`. Aquí son dos verbos:
//!
//! - **`escribir`**: la petición en la primera línea (`dataset`, `modo`
//!   `anexar`|`sobrescribir`, `base` si la tabla existe, `operacion` —la clave
//!   de idempotencia—, `propiedades` para una tabla que nace) y después **la
//!   tabla Arrow por IPC**, tal como el SDK la mandó; se lleva al físico de
//!   0032 (`carga::normalizar`), se escribe, y se devuelven los `requirements`
//!   y `updates` que un catálogo aplica, con las cuentas. La clave de operación
//!   la trae la petición (`operacion`) o **sale del contenido**
//!   (`operacion: "contenido"`, con `semilla`: la huella de los valores, no de
//!   los bytes del IPC, que cambian entre dos lecturas de lo mismo);
//! - **`aplicar`**: `{metadata_location?, dataset, requirements, updates}` —de
//!   `escribir` o de un cliente de fuera— → se validan, se aplican y se
//!   escribe el `metadata.json` siguiente (desde cero si `assert-create`).
//!   Devuelve el `metadata_location` nuevo, el snapshot, las columnas como
//!   Iceberg y como OOS, la clave de operación del snapshot y la retención.
//!
//! Y `leer` deja de exigir la cabecera de la copia: lo que otro escribió
//! también se lee, con una cabecera hecha del esquema de la tabla.

use crate::almacen::Almacen;
use crate::lago::{self, Lago, Operacion};
use crate::{carga, sobre};
use ore_core::json::Json;
use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, Read};
use std::sync::Arc;

/// El `main` de los dos binarios: lee la petición, elige el verbo, y contesta
/// una línea. Quién guarda los bytes lo decide el que llama.
pub fn principal(cuenta: Arc<dyn Almacen>) -> std::process::ExitCode {
    let verbo = std::env::args().nth(1).unwrap_or_else(|| "sellar".into());
    match correr(&verbo, cuenta) {
        Ok(linea) => {
            println!("{linea}");
            std::process::ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn correr(verbo: &str, cuenta: Arc<dyn Almacen>) -> Result<String, String> {
    // La primera línea es la petición; lo que sigue son filas de texto (una por
    // línea) o, para `escribir`, la tabla Arrow por IPC: bytes, no texto.
    let stdin = std::io::stdin();
    let mut lector = std::io::BufReader::with_capacity(1 << 20, stdin.lock());
    let mut primera = String::new();
    while primera.trim().is_empty() {
        primera.clear();
        if lector
            .read_line(&mut primera)
            .map_err(|e| format!("no se pudo leer la entrada: {e}"))?
            == 0
        {
            return Err(
                "la entrada está vacía: se esperaba la petición en la primera línea".into(),
            );
        }
    }
    let primera = primera.trim().to_string();
    let primera = primera.as_str();
    let n =
        ore_core::parse::parse(primera).map_err(|e| format!("la petición no analiza: {e:?}"))?;
    if verbo == "escribir" {
        return escribir(&Lago::nuevo(cuenta), primera, lector);
    }
    let mut texto = String::new();
    lector
        .read_to_string(&mut texto)
        .map_err(|e| format!("no se pudo leer la entrada: {e}"))?;
    let lineas = texto.lines().filter(|l| !l.trim().is_empty());
    let campo = |k: &str| -> Option<String> {
        n.get(k)
            .and_then(|(_, v)| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from)
    };
    let bandera = |k: &str| campo(k).is_some_and(|v| v == "true");
    let lago = Lago::nuevo(cuenta.clone());

    match verbo {
        // **El paso 4.** El puntero lo trae el árbol; aquí sólo se comprueba
        // que lo que apunta sigue en el bucket (un HEAD), para no decir «ya
        // está» de una copia que alguien vació.
        "buscar" => {
            let ml = campo("metadata_location")
                .ok_or("a `buscar` le falta `metadata_location`: el puntero del árbol")?;
            let clave = lago.clave(&ml).map_err(|e| e.to_string())?;
            Ok(Json::obj([("existe", Json::Bool(cuenta.existe(&clave)?))]).jcs())
        }
        "sellar" => {
            let cab = leer_cabecera(primera)?;
            let dataset = campo("dataset").ok_or(
                "a `sellar` le falta `dataset`: bajo qué nombre vive la tabla (`copias/<p>_<v>`)",
            )?;
            sellar(
                &lago,
                &cab,
                &dataset,
                campo("base").as_deref(),
                bandera("fundir"),
                lineas,
            )
        }
        // La copia de una vista cuya raíz es una tabla del lago (0031 «(d)»):
        // sin driver de texto. Se lee la tabla origen en Arrow, se proyecta, se
        // filtra y se sella igual que `sellar`.
        "copiar" => {
            let cab = leer_cabecera(primera)?;
            let dataset = campo("dataset").ok_or(
                "a `copiar` le falta `dataset`: bajo qué nombre vive la copia (`copias/<p>_<v>`)",
            )?;
            let origen = n
                .get("origen")
                .map(|(_, o)| o.clone())
                .ok_or("a `copiar` le falta `origen`: la tabla del lago que se copia")?;
            copiar(
                &lago,
                &cab,
                &dataset,
                campo("base").as_deref(),
                bandera("fundir"),
                &origen,
            )
        }
        "recoger" | "recoger-seco" => {
            let dataset = campo("dataset").ok_or("a `recoger` le falta `dataset`")?;
            let ml = campo("metadata_location")
                .ok_or("a `recoger` le falta `metadata_location`: el puntero vigente")?;
            let edad = campo("edad_ms").and_then(|v| v.parse::<i64>().ok());
            recoger(&lago, &dataset, &ml, edad, verbo == "recoger-seco")
        }
        "aplicar" => aplicar(&lago, primera),
        "esbozar" => esbozar(&lago, primera),
        // El `metadata.json` de un puntero, tal cual está en el bucket: lo que
        // un `loadTable` del catálogo REST devuelve como `metadata`.
        "metadatos" => {
            let ml = campo("metadata_location")
                .ok_or("a `metadatos` le falta `metadata_location`: el puntero del dataset")?;
            let clave = lago.clave(&ml).map_err(|e| e.to_string())?;
            let bytes = cuenta
                .leer_bytes(&clave)?
                .ok_or_else(|| format!("`{ml}` no está en el bucket"))?;
            String::from_utf8(bytes).map_err(|_| format!("`{ml}` no es texto"))
        }
        // La credencial prestada para escribir SÓLO bajo un dataset (0031 §11 ③).
        "prestar" => {
            let dataset = campo("dataset").ok_or("a `prestar` le falta `dataset`")?;
            let prefijo = format!("{}/{dataset}/", lago::RAIZ);
            let p = cuenta.prestar(&prefijo)?;
            Ok(Json::obj([
                ("acotada", Json::Bool(p.acotada)),
                ("caduca_ms", Json::Int(p.caduca_ms.unwrap_or(-1))),
                (
                    "config",
                    Json::Obj(p.config.into_iter().map(|(k, v)| (k, Json::s(v))).collect()),
                ),
                ("prefijo", Json::s(lago.uri(&prefijo))),
            ])
            .jcs())
        }
        "recoger-huerfanas" => recoger_huerfanas(&lago, &n),
        "leer" => leer(&lago, &n),
        "historia" => {
            let ml = campo("metadata_location")
                .ok_or("a `historia` le falta `metadata_location`: el puntero del dataset")?;
            historia(&lago, &ml, campo("dataset").as_deref().unwrap_or("dataset"))
        }
        otro => Err(format!(
            "verbo desconocido `{otro}`: hace `buscar`, `sellar`, `copiar`, `escribir`, \
             `aplicar`, `esbozar`, `metadatos`, `prestar`, `recoger`, `recoger-seco`, \
             `recoger-huerfanas`, `leer` e `historia`"
        )),
    }
}

/// **Sella el dataset**: las filas que llegan, tipadas con el contrato de 0032,
/// van a la tabla Iceberg de `dataset`. Tres caminos, y el puntero decide cuál:
///
/// - **sin `base`**: la tabla no existe (primera copia, o un puntero heredado
///   que apunta a un sobre): se crea y se escribe en UN `metadata.json`;
/// - **con `base` y `fundir`**: un refresco. Lo que había se lee de la tabla,
///   se funde con el incremento por la clave, y el resultado **sobrescribe**
///   (un snapshot que sustituye todos los ficheros: los 10 M de la medida
///   tardan 3,8 s en reescribirse, y el snapshot anterior sigue ahí);
/// - **con `base` y sin `fundir`**: rehacer, o un plan que cambió. Las filas
///   que llegan son la copia entera y sobrescriben.
///
/// Si el esquema del lote no es el de la tabla (una columna nueva, una que se
/// fue, una que cambió de tipo), la tabla lo adopta antes de escribir.
///
/// La cabecera —plan, esquema, testigo, clave, conducto— va como propiedades
/// del snapshot, y `leer` la devuelve tal cual: el dataset es autodescriptivo
/// como lo era el sobre.
fn sellar<'a>(
    lago: &Lago,
    cab: &sobre::Cabecera,
    dataset: &str,
    base: Option<&str>,
    fundir: bool,
    filas: impl Iterator<Item = &'a str>,
) -> Result<String, String> {
    let llegadas: Vec<carga::Fila> = filas
        .map(objeto_plano)
        .collect::<Result<Vec<_>, String>>()?;

    let previa = match base {
        Some(b) => Some(lago.abrir(b, dataset)?),
        None => None,
    };

    // **La fusión, y la trampa de determinismo que trae debajo.** Sin clave no
    // hay con qué fundir, así que las filas van tal cual y una copia solo se
    // puede rehacer entera — la otra cara de `OOS2023`. Con clave se funde
    // siempre que haya base: `fundir` ordena por clave, y así una copia
    // rehecha entera y una refrescada dan los mismos ficheros para el mismo
    // estado.
    let filas = if fundir {
        if cab.clave.is_empty() {
            return Err(
                "se pidió fundir sobre la copia anterior y la cabecera no declara `clave`: sin \
                 ella no se sabe qué fila sustituye a cuál"
                    .into(),
            );
        }
        let Some(t) = &previa else {
            return Err("se pidió fundir y no hay `base` sobre la que fundir".into());
        };
        carga::fundir(lago.filas(t)?, llegadas, &cab.clave)
    } else if cab.clave.is_empty() {
        llegadas
    } else {
        carga::fundir(Vec::new(), llegadas, &cab.clave)
    };

    // **Cuántas filas traen cada columna.** El informe decía «32 951 filas,
    // copiada» de una copia con 2 de 9 columnas vacías (medida W1 §B). Se
    // cuenta por columna, y lo que salga vacío se ve en el informe.
    let columnas = Json::Obj(
        cab.esquema
            .keys()
            .map(|c| {
                let n = filas.iter().filter(|f| f.contains_key(c)).count();
                (c.clone(), Json::Int(n as i64))
            })
            .collect(),
    );
    let carga::Lote {
        lote,
        sin_estrechar,
    } = carga::lote(&cab.esquema, &filas)?;
    confirmar_copia(
        lago,
        cab,
        dataset,
        base,
        fundir,
        previa,
        vec![lote],
        columnas,
        sin_estrechar,
        None,
    )
}

/// **`copiar`: sellar sin pasar por el texto** (0031 «(d)»). La raíz de la
/// vista es una tabla del lago —`datasource: lago`, lo que `write()` dejó, o
/// la copia de otra vista— y no hay `ore-read-lago`: el puntero está en el
/// árbol y no en una URL, y el protocolo de texto de 0008 es el camino lento
/// (medido: `ore-store leer` 8,3 s por millón de filas, Arrow 0,4 s). Aquí se
/// abre la tabla origen por su `metadata_location`, se leen sus lotes vivos
/// (con los position deletes aplicados), se aplican los filtros de la vista
/// (`[[columna, "eq", valor]]`, con el literal llevado al tipo de la columna),
/// se proyecta por nombre (`proyeccion: {campo: columna}`) y cada columna se
/// lleva al físico que la cabecera declara (`cast`; lo que no convierte se
/// queda como texto y va a `sin_estrechar`, como en `sellar`). De ahí en
/// adelante es exactamente `sellar`: la cabecera va como propiedades del
/// snapshot, la tabla nace o se sobrescribe, y `leer` la devuelve igual.
///
/// El orden de las filas es el del origen: no se ordena por clave como hace
/// `sellar` con el texto, porque aquí la fuente es una tabla —con ficheros y
/// orden propios— y reordenar un millón de filas para que el Parquet salga
/// igual no vale lo que cuesta. `fundir` funde por clave en Arrow
/// (`carga::fundir_lotes`).
fn copiar(
    lago: &Lago,
    cab: &sobre::Cabecera,
    dataset: &str,
    base: Option<&str>,
    fundir: bool,
    origen: &ore_core::parse::Node,
) -> Result<String, String> {
    use arrow_array::{Array, RecordBatch, StringArray};
    use arrow_schema::{Field, Schema};
    use std::sync::Arc;

    let campo = |k: &str| -> Option<String> {
        origen
            .get(k)
            .and_then(|(_, v)| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from)
    };
    let ml = campo("metadata_location")
        .ok_or("a `origen` le falta `metadata_location`: el puntero de la tabla que se copia")?;
    let nombre_origen = campo("dataset").unwrap_or_else(|| "origen".into());
    let fuente = lago.abrir(&ml, &nombre_origen)?;
    let proyeccion: Vec<(String, String)> = origen
        .get("proyeccion")
        .map(|(_, p)| {
            p.entries()
                .iter()
                .filter_map(|(k, v)| Some((k.as_str()?.to_string(), v.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default();
    let filtros: Vec<(String, String)> = origen
        .get("filtros")
        .map(|(_, f)| {
            f.items()
                .iter()
                .map(|t| {
                    let i = t.items();
                    match (
                        i.first().and_then(|x| x.as_str()),
                        i.get(1).and_then(|x| x.as_str()),
                        i.get(2).and_then(|x| x.as_str()),
                    ) {
                        (Some(c), Some("eq"), Some(v)) => Ok((c.to_string(), v.to_string())),
                        (Some(c), Some(op), _) => Err(format!(
                            "el filtro sobre `{c}` es `{op}` y `copiar` sólo sabe `eq`"
                        )),
                        _ => Err("un filtro no tiene la forma `[columna, \"eq\", valor]`".into()),
                    }
                })
                .collect::<Result<Vec<_>, String>>()
        })
        .transpose()?
        .unwrap_or_default();
    // Cada campo de la cabecera nombra una columna del origen; lo que no esté
    // en la proyección no se copia, y una columna que el origen no tiene se
    // dice con su nombre (la vista se validó contra la Table, pero la tabla
    // pudo evolucionar desde entonces).
    for (campo, col) in &proyeccion {
        if fuente
            .metadata()
            .current_schema()
            .field_by_name(col)
            .is_none()
        {
            return Err(format!(
                "la vista proyecta `{campo}` desde `{col}`, y la tabla `{nombre_origen}` no tiene esa columna"
            ));
        }
    }
    for (col, _) in &filtros {
        if fuente
            .metadata()
            .current_schema()
            .field_by_name(col)
            .is_none()
        {
            return Err(format!(
                "la vista filtra por `{col}`, y la tabla `{nombre_origen}` no tiene esa columna"
            ));
        }
    }

    // El esquema de la copia: el de la cabecera (en su orden), cada columna en
    // el físico de 0032 que declara — o como texto si el origen no convierte.
    let previa = match base {
        Some(b) => Some(lago.abrir(b, dataset)?),
        None => None,
    };
    let leidos = lago.lotes(&fuente)?;
    let leidas: usize = leidos.iter().map(|l| l.num_rows()).sum();
    let mut sin_estrechar: BTreeMap<String, String> = BTreeMap::new();
    let mut lotes: Vec<RecordBatch> = Vec::with_capacity(leidos.len());
    let mut destino: Option<Arc<Schema>> = None;
    for lote in &leidos {
        // ① los filtros: `eq` sobre la columna del origen, con el literal
        // llevado al tipo de la columna (un `10.5` contra un decimal es
        // `10.50`; un instante en texto, el instante). Un nulo no es igual a
        // nada, y no pasa.
        let mut lote = lote.clone();
        for (col, valor) in &filtros {
            let c = lote
                .column_by_name(col)
                .ok_or_else(|| format!("el lote no trae `{col}`"))?;
            let literal = arrow_cast::cast(&StringArray::from(vec![valor.as_str()]), c.data_type())
                .map_err(|e| {
                    format!(
                        "el filtro `{col} = {valor}` no es un `{}`: {e}",
                        c.data_type()
                    )
                })?;
            if literal.is_null(0) {
                return Err(format!(
                    "el filtro `{col} = {valor}` no es un `{}`",
                    c.data_type()
                ));
            }
            let igual = arrow_ord::cmp::eq(c, &arrow_array::Scalar::new(literal))
                .map_err(|e| format!("no se pudo comparar `{col}`: {e}"))?;
            lote = arrow_select::filter::filter_record_batch(&lote, &igual)
                .map_err(|e| format!("no se pudo filtrar por `{col}`: {e}"))?;
        }
        // ② la proyección, al esquema de la cabecera. El físico se decide con
        // el primer lote y vale para todos: si una columna no convierte en uno,
        // se queda texto en todos (los lotes de una tabla llevan el mismo tipo).
        let esquema = match &destino {
            Some(d) => d.clone(),
            None => {
                let mut campos = Vec::with_capacity(cab.esquema.len());
                for (nombre, tipo) in &cab.esquema {
                    let col = proyeccion.iter().find(|(c, _)| c == nombre).map(|(_, o)| o.as_str())
                        .ok_or_else(|| format!("la cabecera declara `{nombre}` y la proyección no dice de qué columna sale"))?;
                    let c = lote
                        .column_by_name(col)
                        .ok_or_else(|| format!("el lote no trae `{col}`"))?;
                    let pedido = carga::arrow_del_oos(tipo);
                    let fisico = if c.data_type() == &pedido
                        || arrow_cast::can_cast_types(c.data_type(), &pedido)
                    {
                        pedido
                    } else {
                        sin_estrechar.insert(
                            nombre.clone(),
                            format!(
                                "la columna `{col}` del origen es `{}` y no convierte a `{tipo}`",
                                c.data_type()
                            ),
                        );
                        arrow_schema::DataType::Utf8
                    };
                    campos.push(Field::new(nombre, fisico, true));
                }
                let d = Arc::new(Schema::new(campos));
                destino = Some(d.clone());
                d
            }
        };
        let mut columnas = Vec::with_capacity(esquema.fields().len());
        for f in esquema.fields() {
            let col = proyeccion
                .iter()
                .find(|(c, _)| c == f.name())
                .map(|(_, o)| o.as_str())
                .unwrap_or(f.name());
            let c = lote
                .column_by_name(col)
                .ok_or_else(|| format!("el lote no trae `{col}`"))?;
            columnas.push(if c.data_type() == f.data_type() {
                c.clone()
            } else {
                arrow_cast::cast(c, f.data_type()).map_err(|e| {
                    format!("la columna `{col}` no convierte a `{}`: {e}", f.data_type())
                })?
            });
        }
        let l = RecordBatch::try_new(esquema, columnas)
            .map_err(|e| format!("el lote proyectado no construye: {e}"))?;
        if l.num_rows() > 0 {
            lotes.push(l);
        }
    }
    let esquema = match destino {
        Some(d) => d,
        // Un origen sin filas: la copia nace vacía, con el esquema de la cabecera.
        None => Arc::new(Schema::new(
            cab.esquema
                .iter()
                .map(|(n, t)| Field::new(n, carga::arrow_del_oos(t), true))
                .collect::<Vec<_>>(),
        )),
    };
    if lotes.is_empty() {
        lotes.push(RecordBatch::new_empty(esquema.clone()));
    }
    // Las mismas cuentas que `sellar`: cuántas filas traen cada columna.
    let columnas = Json::Obj(
        esquema
            .fields()
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let n: usize = lotes
                    .iter()
                    .map(|l| l.num_rows() - l.column(i).null_count())
                    .sum();
                (f.name().clone(), Json::Int(n as i64))
            })
            .collect(),
    );
    confirmar_copia(
        lago,
        cab,
        dataset,
        base,
        fundir,
        previa,
        lotes,
        columnas,
        sin_estrechar,
        Some(leidas),
    )
}

/// **La cola común de `sellar` y `copiar`**: los lotes ya tipados van a la
/// tabla de `dataset` —fundidos por clave sobre lo que había si `fundir`—, la
/// cabecera como propiedades del snapshot, y el puntero nuevo con sus cuentas.
#[allow(clippy::too_many_arguments)]
fn confirmar_copia(
    lago: &Lago,
    cab: &sobre::Cabecera,
    dataset: &str,
    base: Option<&str>,
    fundir: bool,
    previa: Option<iceberg::table::Table>,
    lotes: Vec<arrow_array::RecordBatch>,
    columnas: Json,
    sin_estrechar: BTreeMap<String, String>,
    leidas: Option<usize>,
) -> Result<String, String> {
    // `sellar` funde en texto antes de tipar; `copiar` funde aquí, en Arrow,
    // sobre los lotes vivos de la copia anterior llevados al esquema del lote.
    let lotes = if fundir && leidas.is_some() {
        if cab.clave.is_empty() {
            return Err(
                "se pidió fundir sobre la copia anterior y la cabecera no declara `clave`: sin \
                 ella no se sabe qué fila sustituye a cuál"
                    .into(),
            );
        }
        let Some(t) = &previa else {
            return Err("se pidió fundir y no hay `base` sobre la que fundir".into());
        };
        let esquema = lotes[0].schema();
        let viejos = lago
            .lotes(t)?
            .iter()
            .map(|l| carga::al_esquema(l, &esquema))
            .collect::<Result<Vec<_>, _>>()?;
        carga::fundir_lotes(viejos, &lotes, &cab.clave)?
    } else {
        lotes
    };

    // El esquema que el lote pide, con los ids de la tabla si la hay.
    let deseado = lago::esquema_deseado(
        &lago::columnas_de(&lotes[0]),
        previa
            .as_ref()
            .map(|t| t.metadata().current_schema().as_ref()),
    )?;
    let mut propiedades_snapshot = HashMap::from([
        (lago::PROP_CABECERA.to_string(), cab.jcs()),
        (lago::PROP_PLAN.to_string(), cab.plan.clone()),
        (
            lago::PROP_TESTIGO_MODO.to_string(),
            cab.testigo.modo.clone(),
        ),
    ]);
    if let Some(v) = &cab.testigo.valor {
        propiedades_snapshot.insert(lago::PROP_TESTIGO_VALOR.to_string(), v.clone());
    }

    let (tabla, operacion, esquema_cambiado) = match previa {
        Some(t) => {
            let (t, cambiado) = lago.esquema(&t, deseado)?;
            (t, Operacion::Sobrescribir, cambiado)
        }
        None => {
            let t = lago.crear(
                dataset,
                deseado,
                HashMap::from([
                    (lago::PROP_CABECERA.to_string(), cab.jcs()),
                    ("ore.conducto".to_string(), cab.conducto.clone()),
                ]),
            )?;
            (t, Operacion::Anexar, false)
        }
    };
    let escrito = lago.instantanea(&tabla, lotes, operacion, propiedades_snapshot)?;
    let t = &escrito.tabla;

    Ok(Json::obj([
        ("bytes", Json::Int(escrito.bytes as i64)),
        ("columnas", columnas),
        ("esquema_cambiado", Json::Bool(esquema_cambiado)),
        ("ficheros", Json::Int(escrito.ficheros as i64)),
        ("filas", Json::Int(escrito.filas as i64)),
        // Cuántas filas se leyeron del origen (`copiar`): con filtros, más de
        // las que van a la copia. Es la unidad de 0014, y la dice el que lee.
        (
            "leidas",
            Json::Int(leidas.unwrap_or(escrito.filas as usize) as i64),
        ),
        (
            "metadata_location",
            Json::s(t.metadata_location().unwrap_or_default()),
        ),
        (
            "operacion",
            Json::s(match (operacion, base) {
                (Operacion::Anexar, _) => "creada",
                (Operacion::Sobrescribir, _) if fundir => "refrescada",
                (Operacion::Sobrescribir, _) => "sobrescrita",
            }),
        ),
        ("retirados", Json::Int(escrito.retirados as i64)),
        // Las columnas que la tabla de 0032 quería estrechar y se quedaron como
        // texto porque un valor no analizó, con el porqué. Vacío es lo que el
        // contrato promete; lo que haya va al informe tal cual.
        (
            "sin_estrechar",
            Json::Obj(
                sin_estrechar
                    .iter()
                    .map(|(c, p)| (c.clone(), Json::s(p)))
                    .collect(),
            ),
        ),
        (
            "snapshot",
            Json::s(
                t.metadata()
                    .current_snapshot_id()
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
            ),
        ),
        ("ubicacion", Json::s(t.metadata().location())),
    ])
    .jcs())
}

/// **`escribir`: la primera mitad del verbo escribir** (0031 §11 ②). La tabla
/// Arrow llega por IPC tal como el SDK la mandó; cada lote se lleva al físico
/// de 0032 ([`carga::normalizar`]: lo que no cabe se niega con el nombre de la
/// columna), el esquema de la tabla pasa a ser el del lote (por id, como en la
/// copia), los ficheros, los manifiestos y la lista se escriben, y **no se
/// confirma nada**: se devuelven los `requirements` y `updates` que un
/// catálogo aplica —`ore-store aplicar` detrás de `ore datasets --commit`, o
/// cualquier catálogo REST— y las cuentas. `operacion` es la clave de
/// idempotencia: va al resumen del snapshot y el catálogo la coteja.
fn escribir(lago: &Lago, peticion: &str, lector: impl std::io::Read) -> Result<String, String> {
    // Con serde: el `esbozo` es un metadata.json entero, con `null` dentro.
    let n: serde_json::Value = serde_json::from_str(peticion)
        .map_err(|e| format!("la petición de `escribir` no es JSON: {e}"))?;
    let campo = |k: &str| {
        n.get(k)
            .and_then(|v| v.as_str())
            .filter(|c| !c.is_empty())
            .map(String::from)
    };
    let dataset = campo("dataset").ok_or(
        "a `escribir` le falta `dataset`: bajo qué nombre vive la tabla (`datasets/<p>_<t>`)",
    )?;
    let modo = campo("modo").unwrap_or_else(|| "sobrescribir".into());
    let operacion = match modo.as_str() {
        "sobrescribir" | "upsert" => Operacion::Sobrescribir,
        "anexar" => Operacion::Anexar,
        otro => {
            return Err(format!(
                "`modo` es `sobrescribir`, `anexar` o `upsert`, no `{otro}`"
            ));
        }
    };
    // La clave del upsert: la de la petición, o la que la tabla ya declara.
    let clave_upsert: Vec<String> = n
        .get("clave")
        .and_then(|c| c.as_array())
        .map(|c| {
            c.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let semilla = campo("semilla").unwrap_or_default();
    // La procedencia va al resumen del snapshot tal cual llegó (JCS): es del
    // que escribe, y el catálogo la lleva al puntero sin interpretarla.
    let procedencia = n
        .get("procedencia")
        .filter(|p| p.is_object())
        .and_then(|p| ore_core::parse::parse(&p.to_string()).ok())
        .map(|p| Json::de_node(&p).jcs());
    let clave_pedida = campo("operacion");
    let propiedades: HashMap<String, String> = n
        .get("propiedades")
        .and_then(|p| p.as_object())
        .map(|p| {
            p.iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default();

    // La tabla Arrow, lote a lote, al físico del contrato: por IPC (lo que
    // pyarrow y Arrow Java escriben) o como Parquet (`formato: parquet`: lo que
    // DuckDB escribe desde Node, que no lleva Arrow).
    let mut lotes = Vec::new();
    match campo("formato").as_deref() {
        None | Some("ipc") => {
            let flujo = arrow_ipc::reader::StreamReader::try_new(lector, None)
                .map_err(|e| format!("lo que sigue a la petición no es un flujo Arrow IPC: {e}"))?;
            for lote in flujo {
                let lote =
                    lote.map_err(|e| format!("un lote del flujo IPC no se pudo leer: {e}"))?;
                if lote.num_rows() > 0 {
                    lotes.push(carga::normalizar(&lote)?);
                }
            }
        }
        Some("parquet") => {
            let mut bytes = Vec::new();
            let mut lector = lector;
            lector
                .read_to_end(&mut bytes)
                .map_err(|e| format!("no se pudo leer el Parquet: {e}"))?;
            let b = bytes::Bytes::from(bytes);
            let flujo = parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder::try_new(b)
                .map_err(|e| format!("lo que sigue a la petición no es un Parquet legible: {e}"))?
                .with_batch_size(1 << 16)
                .build()
                .map_err(|e| format!("el Parquet no se pudo abrir: {e}"))?;
            for lote in flujo {
                let lote = lote.map_err(|e| format!("un lote del Parquet no se pudo leer: {e}"))?;
                if lote.num_rows() > 0 {
                    lotes.push(carga::normalizar(&lote)?);
                }
            }
        }
        Some(otro) => return Err(format!("`formato` es `ipc` o `parquet`, no `{otro}`")),
    }
    let Some(primero) = lotes.first() else {
        return Err("el flujo IPC no trae ninguna fila: nada que escribir".into());
    };
    let columnas = lago::columnas_de(primero);
    if columnas.is_empty() {
        return Err("la tabla no tiene columnas: nada que escribir".into());
    }
    // La clave de operación: la de la petición, o la del contenido.
    let clave = match clave_pedida.as_deref() {
        Some("contenido") => {
            let huella = carga::huella(&lotes);
            Some(format!("{:.32}", {
                use sha2::{Digest, Sha256};
                let mut h = Sha256::new();
                h.update(semilla.as_bytes());
                h.update(b"|");
                h.update(huella.as_bytes());
                h.finalize()
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
            }))
        }
        otra => otra.map(String::from),
    };
    for (i, l) in lotes.iter().enumerate().skip(1) {
        if lago::columnas_de(l) != columnas {
            return Err(format!(
                "el lote {i} del flujo no tiene las columnas del primero"
            ));
        }
    }

    let previa = match campo("base") {
        Some(b) => Some(lago.abrir(&b, &dataset)?),
        None => None,
    };
    // Una tabla que nace puede venir ya esbozada por el catálogo
    // (`stage-create`: uuid, ubicación, esquema con sus ids): se escribe sobre
    // ESE esbozo, y el commit con `assert-create` la hace nacer tal cual.
    let esbozo = match n.get("esbozo") {
        Some(e) if e.is_object() => {
            let meta: iceberg::spec::TableMetadata = serde_json::from_value(e.clone())
                .map_err(|e| format!("`esbozo` no es un metadata.json de Iceberg: {e}"))?;
            Some(lago.esbozada(meta, &dataset)?)
        }
        _ => None,
    };
    let base_esquema = previa.as_ref().or(esbozo.as_ref());
    // Un decimal más estrecho que el de la tabla (pandas infiere `decimal(3, 2)`
    // de `4.00` donde la tabla tiene `decimal(10, 2)`) se ensancha a la misma
    // escala: no es otro tipo —no pierde nada— y no merece otro id de columna.
    let lotes = match base_esquema.map(|t| t.metadata().current_schema().clone()) {
        Some(esquema) => lotes
            .into_iter()
            .map(|l| carga::ensanchar(&l, &esquema))
            .collect::<Result<Vec<_>, _>>()?,
        None => lotes,
    };
    // `upsert` (0031 §11 ⑤): copy-on-write en Arrow. Lo que había —con sus
    // position deletes aplicados— menos las claves que llegan, más lo que
    // llega, y todo al esquema unión (la tabla más las columnas nuevas del
    // lote: un upsert no tira columnas); se escribe entero como `overwrite`.
    // La clave: la de la petición, o la que la tabla declara (`ore.clave`).
    let mut clave_upsert = clave_upsert;
    if modo == "upsert" {
        if clave_upsert.is_empty() {
            clave_upsert = previa
                .as_ref()
                .and_then(|t| t.metadata().properties().get(lago::PROP_CLAVE).cloned())
                .map(|c| c.split(',').map(String::from).collect())
                .unwrap_or_default();
        }
        if clave_upsert.is_empty() {
            return Err(
                "`upsert` quiere `clave` (las columnas que identifican una fila): la tabla no la declara todavía"
                    .into(),
            );
        }
    }
    let lotes = match (&previa, modo.as_str()) {
        (Some(t), "upsert") => {
            let de_la_tabla = iceberg::arrow::schema_to_arrow_schema(t.metadata().current_schema())
                .map_err(|e| format!("el esquema de la tabla no pasa a Arrow: {e}"))?;
            let mut campos: Vec<arrow_schema::Field> = lotes[0]
                .schema()
                .fields()
                .iter()
                .map(|f| f.as_ref().clone())
                .collect();
            for f in de_la_tabla.fields() {
                if !campos.iter().any(|c| c.name() == f.name()) {
                    campos.push(f.as_ref().clone().with_nullable(true));
                }
            }
            let union = std::sync::Arc::new(arrow_schema::Schema::new(campos));
            for c in &clave_upsert {
                if union.field_with_name(c).is_err() {
                    return Err(format!(
                        "la clave nombra `{c}`, que no es una columna de la tabla ni del lote"
                    ));
                }
            }
            let nuevos = lotes
                .iter()
                .map(|l| carga::al_esquema(l, &union))
                .collect::<Result<Vec<_>, _>>()?;
            let viejos = lago
                .lotes(t)?
                .iter()
                .map(|l| carga::al_esquema(l, &union))
                .collect::<Result<Vec<_>, _>>()?;
            carga::fundir_lotes(viejos, &nuevos, &clave_upsert)?
        }
        _ => lotes,
    };
    let columnas = lago::columnas_de(&lotes[0]);
    let deseado = lago::esquema_deseado(
        &columnas,
        base_esquema.map(|t| t.metadata().current_schema().as_ref()),
    )?;
    let tabla = match (&previa, esbozo) {
        (Some(t), _) => t.clone(),
        (None, Some(t)) => t,
        (None, None) => {
            let mut props = propiedades;
            props
                .entry(lago::PROP_DATASET.into())
                .or_insert_with(|| dataset.clone());
            lago.crear(&dataset, deseado.clone(), props)?
        }
    };
    let mut resumen = HashMap::new();
    if let Some(c) = &clave {
        resumen.insert(lago::PROP_OPERACION.to_string(), c.clone());
    }
    if modo == "upsert" {
        resumen.insert(lago::PROP_MODO.to_string(), modo.clone());
        resumen.insert(lago::PROP_CLAVE.to_string(), clave_upsert.join(","));
    }
    if let Some(p) = &procedencia {
        resumen.insert(lago::PROP_PROCEDENCIA.to_string(), p.clone());
    }
    let mut p = lago.preparar(&tabla, deseado, lotes, operacion, resumen)?;
    // La clave queda declarada en la tabla, para la siguiente escritura.
    if modo == "upsert"
        && tabla.metadata().properties().get(lago::PROP_CLAVE) != Some(&clave_upsert.join(","))
    {
        p.cambios.push(iceberg::TableUpdate::SetProperties {
            updates: HashMap::from([(lago::PROP_CLAVE.to_string(), clave_upsert.join(","))]),
        });
    }
    let columnas_json = |t: &iceberg::table::Table| -> Json {
        Json::Obj(
            lago::columnas_iceberg(t)
                .into_iter()
                .map(|(k, v)| (k, Json::s(v)))
                .collect(),
        )
    };
    Ok(Json::obj([
        ("bytes", Json::Int(p.bytes as i64)),
        ("columnas", columnas_json(&tabla)),
        ("esquema_cambiado", Json::Bool(p.esquema_cambiado)),
        ("ficheros", Json::Int(p.ficheros as i64)),
        ("filas", Json::Int(p.filas as i64)),
        ("modo", Json::s(&modo)),
        ("nueva", Json::Bool(previa.is_none())),
        ("operacion", Json::s(clave.unwrap_or_default())),
        ("requirements", json_de(&p.requisitos)?),
        ("retirados", Json::Int(p.retirados as i64)),
        ("snapshot", Json::s(p.snapshot_id.to_string())),
        ("ubicacion", Json::s(tabla.metadata().location())),
        ("updates", json_de(&p.cambios)?),
    ])
    .jcs())
}

/// Lo que `iceberg` serializa con serde (el JSON de la spec REST), como `Json`
/// del núcleo para que salga en la misma línea que lo demás.
fn json_de<T: serde::Serialize>(v: &T) -> Result<Json, String> {
    let texto = serde_json::to_string(v).map_err(|e| format!("no se pudo serializar: {e}"))?;
    let n =
        ore_core::parse::parse(&texto).map_err(|e| format!("lo serializado no analiza: {e:?}"))?;
    Ok(Json::de_node(&n))
}

/// **`aplicar`: la segunda mitad** (0031 §11 ①): el cuerpo de un `updateTable`
/// —de `escribir`, de PyIceberg, de DuckDB— contra la tabla del puntero, o
/// contra ninguna si el requisito es `assert-create`. La petición es una línea:
/// `{"dataset": …, "metadata_location": …?, "requirements": […], "updates": […]}`.
/// Se validan los requisitos (un `assert-ref-snapshot-id` que no cuadra es un
/// conflicto, y se dice como tal), se aplican los cambios y se escribe el
/// siguiente `metadata.json`. Nadie lo apunta todavía.
///
/// La petición puede traer los `requirements` y `updates` arriba, o **el cuerpo
/// del cliente tal cual** en `peticion` (con `cambio: i` si es un
/// `commitTransaction` con `table-changes`): `ore` lo pasa sin reanalizar,
/// porque su JSON no modela `null` y un `assert-ref-snapshot-id` de una tabla
/// recién nacida lo lleva. Con `crear: true`, `peticion` es un
/// `CreateTableRequest` (`schema`, `partition-spec`, `write-order`,
/// `properties`) y los cambios se construyen de él. `retencion_defecto` son
/// las propiedades de retención que una tabla que nace recibe si no las trae.
fn aplicar(lago: &Lago, peticion: &str) -> Result<String, String> {
    let j: serde_json::Value = serde_json::from_str(peticion)
        .map_err(|e| format!("la petición de `aplicar` no es JSON: {e}"))?;
    let dataset = j
        .get("dataset")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or("a `aplicar` le falta `dataset`")?
        .to_string();
    let ml = j
        .get("metadata_location")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);
    let (requisitos, mut cambios) = if j.get("crear").and_then(|v| v.as_bool()) == Some(true) {
        let cuerpo = j
            .get("peticion")
            .ok_or("a `aplicar` con `crear` le falta `peticion`")?;
        cambios_de_creacion(lago, cuerpo, &dataset)?
    } else {
        let cuerpo = match j.get("peticion") {
            Some(p) => match j.get("cambio").and_then(|v| v.as_u64()) {
                Some(i) => p
                    .get("table-changes")
                    .and_then(|t| t.get(i as usize))
                    .ok_or_else(|| format!("la petición no tiene `table-changes[{i}]`"))?,
                None => p,
            },
            None => &j,
        };
        let requisitos: Vec<iceberg::TableRequirement> = cuerpo
            .get("requirements")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|e| format!("`requirements` no se entiende: {e}"))?
            .unwrap_or_default();
        let cambios: Vec<iceberg::TableUpdate> = cuerpo
            .get("updates")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|e| format!("`updates` no se entiende: {e}"))?
            .unwrap_or_default();
        (requisitos, cambios)
    };
    if cambios.is_empty() {
        return Err("`updates` está vacío: nada que aplicar".into());
    }
    let base = match &ml {
        Some(m) => Some(lago.abrir(m, &dataset)?),
        None => None,
    };
    // La retención de una tabla que nace: lo que traiga, o el defecto.
    if base.is_none()
        && let Some(defecto) = j.get("retencion_defecto").and_then(|v| v.as_object())
    {
        let ya = cambios.iter().any(|c| {
            matches!(c, iceberg::TableUpdate::SetProperties { updates } if updates.contains_key(lago::PROP_RETENCION_EDAD))
        });
        if !ya {
            let updates: HashMap<String, String> = defecto
                .iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                .collect();
            if !updates.is_empty() {
                cambios.push(iceberg::TableUpdate::SetProperties { updates });
            }
        }
    }
    let t = lago
        .aplicar(base.as_ref(), &dataset, requisitos, cambios)
        .map_err(|e| {
            if e.contains("Conflict") || e.contains("conflict") || e.contains("does not match") {
                format!("conflicto: {e}")
            } else {
                e
            }
        })?;
    let (edad, minimo) = Lago::retencion(&t, None);
    Ok(Json::obj([
        (
            "columnas",
            Json::Obj(
                lago::columnas_iceberg(&t)
                    .into_iter()
                    .map(|(k, v)| (k, Json::s(v)))
                    .collect(),
            ),
        ),
        (
            "columnas_oos",
            Json::Obj(
                lago::columnas_oos(&t)
                    .into_iter()
                    .map(|(k, v)| (k, Json::s(v)))
                    .collect(),
            ),
        ),
        ("filas", Json::Int(Lago::filas_del_snapshot(&t) as i64)),
        (
            "metadata_location",
            Json::s(t.metadata_location().unwrap_or_default()),
        ),
        ("nueva", Json::Bool(ml.is_none())),
        (
            "operacion",
            Json::s(
                t.metadata()
                    .current_snapshot()
                    .and_then(|s| {
                        s.summary()
                            .additional_properties
                            .get(lago::PROP_OPERACION)
                            .cloned()
                    })
                    .unwrap_or_default(),
            ),
        ),
        (
            "retencion",
            Json::obj([
                ("edad_ms", edad.map(Json::Int).unwrap_or(Json::Int(-1))),
                ("minimo", Json::Int(minimo as i64)),
            ]),
        ),
        (
            "snapshot",
            Json::s(
                t.metadata()
                    .current_snapshot_id()
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
            ),
        ),
        (
            "snapshots",
            Json::Int(t.metadata().snapshots().count() as i64),
        ),
        ("ubicacion", Json::s(t.metadata().location())),
        ("uuid", Json::s(t.metadata().uuid().to_string())),
    ])
    .jcs())
}

/// Los cambios con los que una tabla nace de un `CreateTableRequest` (la spec
/// REST: `schema`, `partition-spec`, `write-order`, `properties`, y
/// `format-version` entre las propiedades), con `assert-create` de requisito.
fn cambios_de_creacion(
    lago: &Lago,
    cuerpo: &serde_json::Value,
    dataset: &str,
) -> Result<(Vec<iceberg::TableRequirement>, Vec<iceberg::TableUpdate>), String> {
    use iceberg::TableUpdate;
    let esquema: iceberg::spec::Schema = cuerpo
        .get("schema")
        .cloned()
        .ok_or_else(|| "a la creación le falta `schema`".to_string())
        .and_then(|v| {
            serde_json::from_value(v).map_err(|e| format!("`schema` no se entiende: {e}"))
        })?;
    let mut cambios = vec![
        TableUpdate::AddSchema { schema: esquema },
        TableUpdate::SetCurrentSchema { schema_id: -1 },
    ];
    if let Some(v) = cuerpo.get("partition-spec").filter(|v| !v.is_null()) {
        let spec: iceberg::spec::UnboundPartitionSpec = serde_json::from_value(v.clone())
            .map_err(|e| format!("`partition-spec` no se entiende: {e}"))?;
        cambios.push(TableUpdate::AddSpec { spec });
        cambios.push(TableUpdate::SetDefaultSpec { spec_id: -1 });
    }
    if let Some(v) = cuerpo.get("write-order").filter(|v| !v.is_null()) {
        let orden: iceberg::spec::SortOrder = serde_json::from_value(v.clone())
            .map_err(|e| format!("`write-order` no se entiende: {e}"))?;
        cambios.push(TableUpdate::AddSortOrder { sort_order: orden });
        cambios.push(TableUpdate::SetDefaultSortOrder { sort_order_id: -1 });
    }
    let ubicacion = cuerpo
        .get("location")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
        .unwrap_or_else(|| lago.uri(&format!("{}/{dataset}", lago::RAIZ)));
    cambios.push(TableUpdate::SetLocation {
        location: ubicacion,
    });
    let mut props: HashMap<String, String> = cuerpo
        .get("properties")
        .and_then(|v| v.as_object())
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default();
    if let Some(v) = props.remove("format-version") {
        let version = match v.as_str() {
            "1" => iceberg::spec::FormatVersion::V1,
            "2" => iceberg::spec::FormatVersion::V2,
            "3" => iceberg::spec::FormatVersion::V3,
            otro => return Err(format!("`format-version: {otro}` no es 1, 2 ni 3")),
        };
        cambios.push(TableUpdate::UpgradeFormatVersion {
            format_version: version,
        });
    }
    props
        .entry(lago::PROP_DATASET.into())
        .or_insert_with(|| dataset.into());
    cambios.push(TableUpdate::SetProperties { updates: props });
    Ok((vec![iceberg::TableRequirement::NotExist], cambios))
}

/// **`esbozar`: la tabla que nacería, sin escribir nada** (`stage-create` de
/// la spec REST): el cliente recibe los metadatos —uuid, esquema con sus ids,
/// ubicación— para escribir sus ficheros, y la tabla nace en el commit que
/// siga con `assert-create`. Petición: `{dataset, peticion: <CreateTableRequest>}`.
fn esbozar(lago: &Lago, peticion: &str) -> Result<String, String> {
    let j: serde_json::Value = serde_json::from_str(peticion)
        .map_err(|e| format!("la petición de `esbozar` no es JSON: {e}"))?;
    let dataset = j
        .get("dataset")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or("a `esbozar` le falta `dataset`")?;
    let cuerpo = j.get("peticion").ok_or("a `esbozar` le falta `peticion`")?;
    let (_, cambios) = cambios_de_creacion(lago, cuerpo, dataset)?;
    let t = lago.esbozar(dataset, cambios)?;
    let meta = serde_json::to_string(t.metadata())
        .map_err(|e| format!("los metadatos no se pudieron serializar: {e}"))?;
    Ok(format!(
        "{{\"metadata\":{meta},\"ubicacion\":{},\"uuid\":\"{}\"}}",
        serde_json::to_string(t.metadata().location()).unwrap_or_default(),
        t.metadata().uuid()
    ))
}

/// **`leer`: la copia, de vuelta, fila a fila** (0029 ③ «traer», F4a·I1).
///
/// La entrada es **el puntero**: `{"metadata_location": "…"}` para un dataset,
/// `{"clave": "ore/v1/<sha256>"}` para un sobre heredado mientras quede alguno.
/// La salida es la cabecera en una línea —la misma que `ore` selló, que el
/// dataset guarda como propiedad de su snapshot— y después **las filas, una
/// por línea**, como objetos JSON de cadenas: el protocolo de 0008 en sentido
/// contrario. Quien lee sabe así **qué copia** leyó y puede dejarlo escrito en
/// lo que produzca.
fn leer(lago: &Lago, n: &ore_core::parse::Node) -> Result<String, String> {
    let campo = |k: &str| {
        n.get(k)
            .and_then(|(_, v)| v.as_str())
            .filter(|c| !c.is_empty())
            .map(String::from)
    };
    let (cabecera, filas) = if let Some(ml) = campo("metadata_location") {
        let dataset = campo("dataset").unwrap_or_else(|| "dataset".into());
        let t = lago.abrir(&ml, &dataset)?;
        // La cabecera de la copia si la hay; si no —lo escribió `write()` o un
        // motor de fuera— una hecha del esquema de la tabla, que es lo que
        // quien lee necesita saber: qué columnas y de qué tipo.
        let cab = Lago::propiedad(&t, lago::PROP_CABECERA).unwrap_or_else(|| {
            let meta = t.metadata();
            sobre::Cabecera {
                plan: meta
                    .properties()
                    .get("ore.plan")
                    .cloned()
                    .unwrap_or_default(),
                esquema: lago::columnas_oos(&t),
                testigo: sobre::Testigo {
                    modo: "snapshot".into(),
                    valor: meta.current_snapshot_id().map(|s| s.to_string()),
                },
                clave: Vec::new(),
                conducto: meta
                    .properties()
                    .get("ore.conducto")
                    .cloned()
                    .unwrap_or_default(),
            }
            .jcs()
        });
        (cab, lago.filas(&t)?)
    } else if let Some(clave) = campo("clave") {
        let cuenta = lago_cuenta(lago)?;
        let bytes = cuenta
            .leer_bytes(&clave)?
            .ok_or_else(|| format!("`{clave}` no está en el almacén"))?;
        let (cabecera, payload) = sobre::abrir(&bytes)?;
        (cabecera, carga::leer(payload)?)
    } else {
        return Err(
            "a la petición le falta `metadata_location` (el puntero del dataset) o `clave` (un sobre heredado)"
                .into(),
        );
    };
    let mut out = String::with_capacity(cabecera.len() + filas.len() * 64);
    out.push_str(&cabecera);
    for f in &filas {
        out.push('\n');
        out.push_str(&Json::Obj(f.iter().map(|(k, v)| (k.clone(), Json::s(v))).collect()).jcs());
    }
    Ok(out)
}

/// **`historia`: la ficha del dataset** (W3.6b). Lo que el puntero no dice y
/// la tabla sí: sus snapshots —cuándo, qué operación, cuántas filas, con qué
/// testigo y qué plan— y su esquema de Iceberg. Es lo que la consola enseña
/// como la ficha del dataset, y lo que `git log` del puntero no puede contar
/// solo (un snapshot que el Job no llegó a apuntar no tiene commit).
fn historia(lago: &Lago, metadata_location: &str, dataset: &str) -> Result<String, String> {
    let t = lago.abrir(metadata_location, dataset)?;
    let meta = t.metadata();
    let mut snapshots: Vec<(i64, Json)> = meta
        .snapshots()
        .map(|s| {
            let p = &s.summary().additional_properties;
            let prop = |k: &str| Json::s(p.get(k).cloned().unwrap_or_default());
            let n = |k: &str| Json::Int(p.get(k).and_then(|v| v.parse().ok()).unwrap_or(0));
            (
                s.timestamp_ms(),
                Json::obj([
                    ("id", Json::s(s.snapshot_id().to_string())),
                    ("cuando_ms", Json::Int(s.timestamp_ms())),
                    (
                        "operacion",
                        Json::s(format!("{:?}", s.summary().operation).to_lowercase()),
                    ),
                    ("filas", Json::Int(Lago::filas_de(&t, s) as i64)),
                    ("ficheros", n("total-data-files")),
                    ("bytes", n("total-files-size")),
                    ("anadidas", n("added-records")),
                    ("retiradas", n("deleted-records")),
                    ("plan", prop(lago::PROP_PLAN)),
                    ("idempotencia", prop(lago::PROP_OPERACION)),
                    // De qué salió, si quien escribió lo dijo (W3.7 ③).
                    (
                        "procedencia",
                        p.get(lago::PROP_PROCEDENCIA)
                            .and_then(|s| ore_core::parse::parse(s).ok())
                            .map(|n| Json::de_node(&n))
                            .unwrap_or_else(|| Json::obj([])),
                    ),
                    (
                        "testigo",
                        Json::obj([
                            ("modo", prop(lago::PROP_TESTIGO_MODO)),
                            ("valor", prop(lago::PROP_TESTIGO_VALOR)),
                        ]),
                    ),
                    (
                        "vigente",
                        Json::Bool(Some(s.snapshot_id()) == meta.current_snapshot_id()),
                    ),
                ]),
            )
        })
        .collect();
    // Del más reciente al más viejo: lo que una ficha enseña.
    snapshots.sort_by_key(|(t, _)| -*t);
    let (edad, minimo) = Lago::retencion(&t, None);
    Ok(Json::obj([
        (
            "retencion",
            Json::obj([
                ("edad_ms", edad.map(Json::Int).unwrap_or(Json::Int(-1))),
                ("minimo", Json::Int(minimo as i64)),
            ]),
        ),
        (
            "esquema",
            Json::Obj(
                lago::columnas_iceberg(&t)
                    .into_iter()
                    .map(|(k, v)| (k, Json::s(v)))
                    .collect(),
            ),
        ),
        ("metadata_location", Json::s(metadata_location)),
        ("metadata_log", Json::Int(meta.metadata_log().len() as i64)),
        (
            "snapshot",
            Json::s(
                meta.current_snapshot_id()
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
            ),
        ),
        (
            "snapshots",
            Json::Arr(snapshots.into_iter().map(|(_, j)| j).collect()),
        ),
        ("ubicacion", Json::s(meta.location())),
        ("uuid", Json::s(meta.uuid().to_string())),
    ])
    .jcs())
}

fn lago_cuenta(lago: &Lago) -> Result<Arc<dyn Almacen>, String> {
    lago.cuenta_publica()
}

/// **La recogida de basura de un dataset**, y por qué es explícita.
///
/// Un snapshot superado **sigue siendo cierto hasta su marca**, y alguien puede
/// estar leyéndolo por su id (`iceberg_scan(…, snapshot_from_id)`). Así que se
/// expira cuando alguien lo pide, y `recoger-seco` dice antes qué se iría.
///
/// Dos pasos: expirar los snapshots que no son el vigente y son más viejos que
/// la edad que rige —**la de la tabla** (`history.expire.max-snapshot-age-ms`,
/// 0031 §11 ⑥) o, si la tabla no la declara, `edad_ms`; sin ninguna de las
/// dos **no se expira nada**, y `history.expire.min-snapshots-to-keep` se
/// respeta—, y retirar del bucket lo que ningún snapshot que quede nombra
/// —ficheros de los expirados, y lo que dejó una pasada que no llegó a
/// apuntarse en el árbol—. Si expiró alguno hay un `metadata.json` nuevo, y se
/// devuelve: **el puntero tiene que moverse a él**.
fn recoger(
    lago: &Lago,
    dataset: &str,
    metadata_location: &str,
    edad_ms: Option<i64>,
    seco: bool,
) -> Result<String, String> {
    let tabla = lago.abrir(metadata_location, dataset)?;
    let antes = tabla.metadata().snapshots().count();
    // `edad_ms: -1` en la respuesta es «sin retención»: no se expira nada.
    let (edad, minimo) = Lago::retencion(&tabla, edad_ms);
    let (tabla, expirados) = match edad {
        None => (tabla, Vec::new()),
        Some(e) if seco => {
            let ids = Lago::expirables(&tabla, e, minimo);
            (tabla, ids)
        }
        Some(e) => lago.expirar(&tabla, e, minimo)?,
    };
    let ficheros = lago.huerfanos(&tabla, seco)?;
    Ok(Json::obj([
        ("edad_ms", edad.map(Json::Int).unwrap_or(Json::Int(-1))),
        ("expirados", Json::Int(expirados.len() as i64)),
        ("ficheros", Json::Int(ficheros as i64)),
        (
            "metadata_location",
            Json::s(tabla.metadata_location().unwrap_or(metadata_location)),
        ),
        ("minimo", Json::Int(minimo as i64)),
        ("seco", Json::Bool(seco)),
        ("snapshots", Json::Int(antes as i64)),
    ])
    .jcs())
}

/// **Lo que ningún puntero reclama.** `recoger` limpia DENTRO de un dataset
/// vigente; esto retira los datasets **cuyo puntero ya no está en el árbol**
/// —la base se retiró, la vista dejó de declarar copia— y los sobres heredados
/// (`ore/v1/`) que ningún puntero sigue nombrando. La entrada es una línea JSON
/// con lo que el árbol reclama —TODOS los datasets con puntero, no sólo los de
/// esta pasada— y `seco` para decir qué se iría sin tocar nada:
///
/// ```text
/// {"datasets": ["copias/ventas_pedidos", …], "claves": ["ore/v1/…", …], "seco": false}
/// ```
fn recoger_huerfanas(lago: &Lago, n: &ore_core::parse::Node) -> Result<String, String> {
    let cuenta = lago_cuenta(lago)?;
    let seco = n
        .get("seco")
        .and_then(|(_, v)| v.as_str())
        .is_some_and(|v| v == "true");
    let lista = |k: &str| -> std::collections::BTreeSet<String> {
        n.get(k)
            .map(|(_, v)| v.items())
            .unwrap_or(&[])
            .iter()
            .filter_map(|p| p.as_str())
            .map(String::from)
            .collect()
    };
    let datasets = lista("datasets");
    let claves = lista("claves");

    // Los datasets del bucket: `ore/v2/<clase>/<nombre>/…` → `<clase>/<nombre>`.
    let raiz = format!("{}/", lago::RAIZ);
    let mut huerfanos: std::collections::BTreeSet<String> = Default::default();
    let mut objetos = 0usize;
    for k in cuenta.listar(&raiz)? {
        let resto = &k[raiz.len()..];
        let mut partes = resto.splitn(3, '/');
        let (Some(clase), Some(nombre)) = (partes.next(), partes.next()) else {
            continue;
        };
        let dataset = format!("{clase}/{nombre}");
        if datasets.contains(&dataset) {
            continue;
        }
        huerfanos.insert(dataset);
        if !seco {
            cuenta.borrar(&k)?;
        }
        objetos += 1;
    }
    // Y lo heredado: sobres y recibos de `ore/v1/`. Los recibos no los lee ya
    // nadie; un sobre sólo se queda si un puntero todavía lo nombra.
    let mut heredados = 0usize;
    for k in cuenta.listar("ore/v1/")? {
        if claves.contains(&k) {
            continue;
        }
        if !seco {
            cuenta.borrar(&k)?;
        }
        heredados += 1;
    }
    Ok(Json::obj([
        ("datasets", Json::Int(datasets.len() as i64)),
        ("heredados", Json::Int(heredados as i64)),
        ("huerfanos", Json::Int(huerfanos.len() as i64)),
        ("objetos", Json::Int(objetos as i64)),
        ("seco", Json::Bool(seco)),
    ])
    .jcs())
}

/// La cabecera, leída con el analizador del núcleo — el mismo que lee YAML, que
/// es un superconjunto de JSON. No entra un analizador más para esto. Los
/// campos de la petición que no son de la cabecera (`dataset`, `base`,
/// `fundir`) simplemente no se leen: la cabecera dice QUÉ CONTIENE la copia, y
/// no cómo se construyó.
fn leer_cabecera(linea: &str) -> Result<sobre::Cabecera, String> {
    let n = ore_core::parse::parse(linea).map_err(|e| format!("la cabecera no analiza: {e:?}"))?;
    let s = |k: &str| -> Result<String, String> {
        n.get(k)
            .and_then(|(_, v)| v.as_str())
            .map(String::from)
            .ok_or_else(|| format!("a la cabecera le falta `{k}`"))
    };
    let esquema = n
        .get("esquema")
        .map(|(_, e)| {
            e.entries()
                .iter()
                .filter_map(|(k, v)| Some((k.as_str()?.to_string(), v.as_str()?.to_string())))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    if esquema.is_empty() {
        return Err("la cabecera no declara `esquema`: sin él no hay tabla que escribir".into());
    }
    let testigo = n.get("testigo").map(|(_, t)| sobre::Testigo {
        modo: t
            .get("modo")
            .and_then(|(_, v)| v.as_str())
            .unwrap_or("none")
            .to_string(),
        valor: t
            .get("valor")
            .and_then(|(_, v)| v.as_str())
            .map(String::from),
    });
    Ok(sobre::Cabecera {
        plan: s("plan")?,
        esquema,
        testigo: testigo.unwrap_or_default(),
        clave: n
            .get("clave")
            .map(|(_, v)| {
                v.items()
                    .iter()
                    .filter_map(|i| i.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        conducto: s("conducto")?,
    })
}

/// Una fila: un objeto de cadenas y nada más. Un valor que no sea escalar es un
/// defecto de quien la produjo — y se dice, en vez de aplanarlo.
fn objeto_plano(linea: &str) -> Result<carga::Fila, String> {
    let n = ore_core::parse::parse(linea).map_err(|e| format!("una fila no analiza: {e:?}"))?;
    let mut out = BTreeMap::new();
    for (k, v) in n.entries() {
        let Some(nombre) = k.as_str() else { continue };
        match v.as_str() {
            Some(x) => {
                out.insert(nombre.to_string(), x.to_string());
            }
            None => {
                return Err(format!(
                    "`{nombre}` no es un escalar: una fila es un objeto plano, y aplanarlo aquí \
                     inventaría una codificación que nadie declaró"
                ));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Un almacén en memoria: lo justo para que el ciclo se pruebe sin red.
    #[derive(Default)]
    pub struct Memoria(pub Mutex<BTreeMap<String, Vec<u8>>>);

    impl Almacen for Memoria {
        fn base(&self) -> String {
            "memory://pruebas".into()
        }
        fn leer(&self, clave: &str) -> Result<Option<String>, String> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .get(clave)
                .map(|b| String::from_utf8_lossy(b).into_owned()))
        }
        fn existe(&self, clave: &str) -> Result<bool, String> {
            Ok(self.0.lock().unwrap().contains_key(clave))
        }
        // Como los de verdad: si estaba, no se toca (`If-None-Match: *`).
        fn subir(&self, clave: &str, cuerpo: &[u8]) -> Result<bool, String> {
            let mut m = self.0.lock().unwrap();
            if m.contains_key(clave) {
                return Ok(false);
            }
            m.insert(clave.to_string(), cuerpo.to_vec());
            Ok(true)
        }
        fn listar(&self, prefijo: &str) -> Result<Vec<String>, String> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .keys()
                .filter(|k| k.starts_with(prefijo))
                .cloned()
                .collect())
        }
        fn borrar(&self, clave: &str) -> Result<(), String> {
            self.0.lock().unwrap().remove(clave);
            Ok(())
        }
        fn leer_bytes(&self, clave: &str) -> Result<Option<Vec<u8>>, String> {
            Ok(self.0.lock().unwrap().get(clave).cloned())
        }
    }

    fn cabecera(testigo: &str) -> sobre::Cabecera {
        sobre::Cabecera {
            plan: "sha256:plan".into(),
            esquema: [
                ("id".to_string(), "Integer".to_string()),
                ("nombre".to_string(), "String".to_string()),
                ("total".to_string(), "Decimal".to_string()),
            ]
            .into(),
            testigo: sobre::Testigo {
                modo: "log".into(),
                valor: Some(testigo.into()),
            },
            clave: vec!["id".into()],
            conducto: "materialization.payload".into(),
        }
    }

    fn campo(linea: &str, k: &str) -> String {
        ore_core::parse::parse(linea)
            .unwrap()
            .get(k)
            .unwrap_or_else(|| panic!("falta `{k}` en {linea}"))
            .1
            .as_str()
            .unwrap()
            .to_string()
    }

    fn filas_de(lago: &Lago, ml: &str) -> Vec<carga::Fila> {
        lago.filas(&lago.abrir(ml, "copias/p_v").unwrap()).unwrap()
    }

    /// **El ciclo entero sobre un almacén en memoria**: crear, «ya está»,
    /// refrescar fundiendo, rehacer sobrescribiendo, leer, recoger. Es lo que
    /// `refresco.sh` mide contra un bucket de verdad, sin el bucket.
    #[test]
    fn crear_refrescar_sobrescribir_leer_y_recoger() {
        let cuenta: Arc<Memoria> = Arc::new(Memoria::default());
        let lago = Lago::nuevo(cuenta.clone());

        // ① crear: UN metadata.json, un fichero de datos, un manifiesto y su lista
        let s1 = sellar(
            &lago,
            &cabecera("7"),
            "copias/p_v",
            None,
            false,
            [
                "{\"id\":\"2\",\"nombre\":\"Bea\",\"total\":\"10.50\"}",
                "{\"id\":\"1\"}",
            ]
            .into_iter(),
        )
        .expect("sella");
        assert_eq!(campo(&s1, "operacion"), "creada");
        assert_eq!(campo(&s1, "filas"), "2");
        let ml1 = campo(&s1, "metadata_location");
        assert!(
            ml1.starts_with("memory://pruebas/ore/v2/copias/p_v/metadata/00000-"),
            "{ml1}"
        );
        assert_eq!(
            cuenta.0.lock().unwrap().len(),
            4,
            "{:?}",
            cuenta.0.lock().unwrap().keys()
        );
        let t1 = lago.abrir(&ml1, "copias/p_v").unwrap();
        assert_eq!(t1.metadata().snapshots().count(), 1);
        assert_eq!(
            Lago::propiedad(&t1, lago::PROP_TESTIGO_VALOR).as_deref(),
            Some("7")
        );
        let f = filas_de(&lago, &ml1);
        assert_eq!(f.len(), 2);
        assert_eq!(f[0]["id"], "1", "ordenadas por la clave");
        assert_eq!(f[1]["total"], "10.5", "el decimal vuelve canónico");

        // ② refrescar: 1 fila nueva y 1 que cambia, fundidas sobre la base
        let s2 = sellar(
            &lago,
            &cabecera("9"),
            "copias/p_v",
            Some(&ml1),
            true,
            [
                "{\"id\":\"3\",\"nombre\":\"Cai\"}",
                "{\"id\":\"2\",\"nombre\":\"Bea2\",\"total\":\"11\"}",
            ]
            .into_iter(),
        )
        .expect("refresca");
        assert_eq!(campo(&s2, "operacion"), "refrescada");
        assert_eq!(
            campo(&s2, "filas"),
            "3",
            "la copia entera, no el incremento"
        );
        assert_eq!(
            campo(&s2, "retirados"),
            "1",
            "el fichero anterior queda retirado"
        );
        let ml2 = campo(&s2, "metadata_location");
        assert!(ml2.contains("/metadata/00001-"), "{ml2}");
        let f = filas_de(&lago, &ml2);
        assert_eq!(f.len(), 3);
        assert_eq!(f[1]["nombre"], "Bea2");
        assert_eq!(f[1]["total"], "11");
        assert_eq!(f[2]["nombre"], "Cai");
        let t2 = lago.abrir(&ml2, "copias/p_v").unwrap();
        assert_eq!(t2.metadata().snapshots().count(), 2, "la historia se queda");
        assert_eq!(
            Lago::propiedad(&t2, lago::PROP_TESTIGO_VALOR).as_deref(),
            Some("9")
        );
        // y el snapshot anterior sigue siendo legible tal cual era
        let f1 = filas_de(&lago, &ml1);
        assert_eq!(f1.len(), 2);

        // ③ rehacer: las filas que llegan son la copia entera
        let s3 = sellar(
            &lago,
            &cabecera("9"),
            "copias/p_v",
            Some(&ml2),
            false,
            ["{\"id\":\"5\",\"nombre\":\"Eva\"}"].into_iter(),
        )
        .expect("rehace");
        assert_eq!(campo(&s3, "operacion"), "sobrescrita");
        assert_eq!(campo(&s3, "filas"), "1");
        let ml3 = campo(&s3, "metadata_location");
        assert_eq!(filas_de(&lago, &ml3).len(), 1);

        // ④ leer: la cabecera que se selló y las filas, una por línea
        let leido = leer(
            &lago,
            &ore_core::parse::parse(&format!("{{\"metadata_location\":\"{ml3}\"}}")).unwrap(),
        )
        .expect("lee");
        let lineas: Vec<&str> = leido.lines().collect();
        assert_eq!(lineas.len(), 2, "{leido}");
        assert_eq!(lineas[0], cabecera("9").jcs());
        assert_eq!(
            lineas[1], "{\"id\":\"5\",\"nombre\":\"Eva\"}",
            "el nulo no viaja"
        );

        // ⑤ recoger: expiran los dos snapshots superados y se van sus ficheros
        let antes = cuenta.0.lock().unwrap().len();
        // sin edad —ni en la tabla ni en la petición— no se expira nada (§11 ⑥)
        let nada = recoger(&lago, "copias/p_v", &ml3, None, true).expect("nada");
        assert_eq!(campo(&nada, "expirados"), "0");
        let seco = recoger(&lago, "copias/p_v", &ml3, Some(0), true).expect("seco");
        assert_eq!(campo(&seco, "expirados"), "2");
        assert_eq!(
            cuenta.0.lock().unwrap().len(),
            antes,
            "en seco no se toca nada"
        );
        let r = recoger(&lago, "copias/p_v", &ml3, Some(0), false).expect("recoge");
        assert_eq!(campo(&r, "expirados"), "2");
        let ml4 = campo(&r, "metadata_location");
        assert_ne!(ml4, ml3, "expirar deja un metadata.json nuevo");
        let t4 = lago.abrir(&ml4, "copias/p_v").unwrap();
        assert_eq!(t4.metadata().snapshots().count(), 1);
        assert_eq!(filas_de(&lago, &ml4).len(), 1, "la vigente sigue entera");
        assert!(
            campo(&r, "ficheros").parse::<usize>().unwrap() >= 5,
            "los ficheros de los expirados se fueron: {r}"
        );
        // y recoger otra vez no mueve el puntero
        let r2 = recoger(&lago, "copias/p_v", &ml4, Some(0), false).expect("recoge");
        assert_eq!(campo(&r2, "metadata_location"), ml4);
        assert_eq!(campo(&r2, "ficheros"), "0");

        // ⑥ la ficha: los snapshots que había antes de recoger, del más nuevo
        // al más viejo, con su operación y su testigo; y tras recoger, uno.
        let h = historia(&lago, &ml3, "copias/p_v").expect("historia");
        let n = ore_core::parse::parse(&h).unwrap();
        let ss = n.get("snapshots").unwrap().1.items().to_vec();
        assert_eq!(ss.len(), 3, "{h}");
        assert_eq!(ss[0].get("vigente").unwrap().1.as_str(), Some("true"));
        assert_eq!(
            ss[0].get("operacion").unwrap().1.as_str(),
            Some("overwrite")
        );
        assert_eq!(ss[0].get("filas").unwrap().1.as_str(), Some("1"));
        assert_eq!(ss[2].get("operacion").unwrap().1.as_str(), Some("append"));
        assert_eq!(
            ss[2]
                .get("testigo")
                .unwrap()
                .1
                .get("valor")
                .unwrap()
                .1
                .as_str(),
            Some("7")
        );
        assert_eq!(
            n.get("esquema").unwrap().1.get("total").unwrap().1.as_str(),
            Some("decimal(38, 18)")
        );
        let h4 = historia(&lago, &ml4, "copias/p_v").expect("historia");
        let n4 = ore_core::parse::parse(&h4).unwrap();
        assert_eq!(n4.get("snapshots").unwrap().1.items().len(), 1);
    }

    /// Un cambio de esquema —una columna nueva y otra que cambia de tipo— lo
    /// adopta la tabla antes de escribir, y la vieja sigue legible.
    #[test]
    fn el_esquema_evoluciona_con_el_lote() {
        let cuenta: Arc<Memoria> = Arc::new(Memoria::default());
        let lago = Lago::nuevo(cuenta);
        let s1 = sellar(
            &lago,
            &cabecera("1"),
            "copias/p_v",
            None,
            false,
            ["{\"id\":\"1\",\"nombre\":\"a\",\"total\":\"1\"}"].into_iter(),
        )
        .unwrap();
        let ml1 = campo(&s1, "metadata_location");
        let mut cab = cabecera("2");
        cab.esquema.insert("pais".into(), "String".into());
        cab.esquema.insert("total".into(), "Integer".into());
        let s2 = sellar(
            &lago,
            &cab,
            "copias/p_v",
            Some(&ml1),
            false,
            ["{\"id\":\"1\",\"nombre\":\"a\",\"total\":\"2\",\"pais\":\"ES\"}"].into_iter(),
        )
        .unwrap();
        assert_eq!(campo(&s2, "esquema_cambiado"), "true");
        let ml2 = campo(&s2, "metadata_location");
        let t2 = lago.abrir(&ml2, "copias/p_v").unwrap();
        let cols = lago::columnas_iceberg(&t2);
        assert_eq!(cols["pais"], "string");
        assert_eq!(cols["total"], "long");
        let esq = t2.metadata().current_schema();
        assert_eq!(esq.field_by_name("id").unwrap().id, 1, "conserva su id");
        assert!(
            esq.field_by_name("total").unwrap().id > 3,
            "otro tipo, otro id"
        );
        assert_eq!(t2.metadata().schemas_iter().count(), 2);
        let f = filas_de(&lago, &ml2);
        assert_eq!(f[0]["pais"], "ES");
        assert_eq!(f[0]["total"], "2");
    }

    /// Lo huérfano: un dataset sin puntero se va entero; el que se reclama, no.
    #[test]
    fn lo_que_ningun_puntero_reclama_se_va() {
        let cuenta: Arc<Memoria> = Arc::new(Memoria::default());
        let lago = Lago::nuevo(cuenta.clone());
        for d in ["copias/p_a", "copias/p_b"] {
            sellar(
                &lago,
                &cabecera("1"),
                d,
                None,
                false,
                ["{\"id\":\"1\"}"].into_iter(),
            )
            .unwrap();
        }
        cuenta.subir("ore/v1/viejo", b"ORECOPY1...").unwrap();
        cuenta.subir("ore/v1/plan/x/y", b"ore/v1/viejo").unwrap();
        let n = ore_core::parse::parse(
            "{\"datasets\":[\"copias/p_a\"],\"claves\":[\"ore/v1/viejo\"],\"seco\":false}",
        )
        .unwrap();
        let r = recoger_huerfanas(&lago, &n).unwrap();
        assert_eq!(campo(&r, "huerfanos"), "1");
        assert_eq!(
            campo(&r, "heredados"),
            "1",
            "el recibo se va, el sobre nombrado se queda"
        );
        let claves: Vec<String> = cuenta.0.lock().unwrap().keys().cloned().collect();
        assert!(
            claves.iter().all(|k| !k.contains("copias/p_b/")),
            "{claves:?}"
        );
        assert!(claves.iter().any(|k| k.contains("copias/p_a/")));
        assert!(claves.contains(&"ore/v1/viejo".to_string()));
        assert!(!claves.contains(&"ore/v1/plan/x/y".to_string()));
    }

    /// La tabla Arrow que un SDK mandaría, con lo que cada lenguaje tiene de
    /// suyo: `int32`, `timestamp[ns]`, `large_utf8`, una zona que no es UTC.
    /// 0032 al escribir la lleva a los físicos del contrato.
    fn tabla_ipc(desde: i64, n: i64, con_canal: bool) -> Vec<u8> {
        use arrow_array::{
            Decimal128Array, Int32Array, LargeStringArray, StringArray, TimestampNanosecondArray,
        };
        use arrow_schema::{Field, Schema};
        let mut campos = vec![
            Field::new("id", arrow_schema::DataType::Int32, true),
            Field::new("nombre", arrow_schema::DataType::LargeUtf8, true),
            Field::new("total", arrow_schema::DataType::Decimal128(18, 2), true),
            Field::new(
                "cuando",
                arrow_schema::DataType::Timestamp(
                    arrow_schema::TimeUnit::Nanosecond,
                    Some("Europe/Madrid".into()),
                ),
                true,
            ),
        ];
        let mut columnas: Vec<arrow_array::ArrayRef> = vec![
            Arc::new(Int32Array::from_iter_values(
                (desde..desde + n).map(|i| i as i32),
            )),
            Arc::new(LargeStringArray::from_iter_values(
                (desde..desde + n).map(|i| format!("n{i}")),
            )),
            Arc::new(
                Decimal128Array::from_iter_values(
                    (desde..desde + n).map(|i| (i * 100 + 50) as i128),
                )
                .with_precision_and_scale(18, 2)
                .unwrap(),
            ),
            Arc::new(
                TimestampNanosecondArray::from_iter_values(
                    (desde..desde + n).map(|i| 1_700_000_000_000_000_000 + i * 1_000),
                )
                .with_timezone("Europe/Madrid"),
            ),
        ];
        if con_canal {
            campos.push(Field::new("canal", arrow_schema::DataType::Utf8, true));
            columnas.push(Arc::new(StringArray::from_iter_values(
                (desde..desde + n).map(|_| "web"),
            )));
        }
        let esquema = Arc::new(Schema::new(campos));
        let lote = arrow_array::RecordBatch::try_new(esquema.clone(), columnas).unwrap();
        let mut bytes = Vec::new();
        {
            let mut w = arrow_ipc::writer::StreamWriter::try_new(&mut bytes, &esquema).unwrap();
            w.write(&lote).unwrap();
            w.finish().unwrap();
        }
        bytes
    }

    fn nodo(linea: &str) -> ore_core::parse::Node {
        ore_core::parse::parse(linea).unwrap()
    }

    fn aplicar_lo_escrito(lago: &Lago, escrito: &str, dataset: &str, base: Option<&str>) -> String {
        let e: serde_json::Value = serde_json::from_str(escrito).unwrap();
        let mut pet = serde_json::json!({
            "dataset": dataset,
            "requirements": e["requirements"],
            "updates": e["updates"],
        });
        if let Some(b) = base {
            pet["metadata_location"] = serde_json::Value::String(b.into());
        }
        aplicar(lago, &pet.to_string()).expect("aplica")
    }

    /// **El verbo escribir en sus dos mitades, sobre un almacén en memoria.**
    /// La tabla llega por IPC con los físicos de su lenguaje; `escribir` la
    /// lleva a 0032, deja los ficheros y devuelve `requirements` + `updates`;
    /// `aplicar` los convierte en el `metadata.json`. Nace, anexa, sobrescribe
    /// con una columna nueva, y `leer` lo lee sin cabecera de copia.
    #[test]
    fn escribir_por_ipc_y_aplicar() {
        let cuenta: Arc<Memoria> = Arc::new(Memoria::default());
        let lago = Lago::nuevo(cuenta.clone());
        let ds = "datasets/ventas_salida";

        // ① nace: sin base, la petición trae la clave de operación y propiedades
        let e1 = escribir(
            &lago,
            &format!(
                "{{\"dataset\":\"{ds}\",\"modo\":\"sobrescribir\",\"operacion\":\"op-1\",\"propiedades\":{{\"history.expire.max-snapshot-age-ms\":\"0\"}}}}"
            ),
            &tabla_ipc(0, 3, false)[..],
        )
        .expect("escribe");
        assert_eq!(campo(&e1, "nueva"), "true");
        assert_eq!(campo(&e1, "filas"), "3");
        assert_eq!(campo(&e1, "operacion"), "op-1");
        let j1: serde_json::Value = serde_json::from_str(&e1).unwrap();
        assert_eq!(j1["requirements"][0]["type"], "assert-create");
        let acciones: Vec<&str> = j1["updates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|u| u["action"].as_str().unwrap())
            .collect();
        assert_eq!(
            acciones,
            [
                "assign-uuid",
                "upgrade-format-version",
                "add-schema",
                "set-current-schema",
                "add-spec",
                "set-default-spec",
                "add-sort-order",
                "set-default-sort-order",
                "set-location",
                "set-properties",
                "add-snapshot",
                "set-snapshot-ref"
            ],
            "{e1}"
        );
        // los físicos de 0032, no los del lenguaje
        assert_eq!(j1["columnas"]["id"], "long");
        assert_eq!(j1["columnas"]["nombre"], "string");
        assert_eq!(j1["columnas"]["total"], "decimal(18, 2)");
        assert_eq!(j1["columnas"]["cuando"], "timestamptz");
        assert_eq!(
            j1["updates"][10]["snapshot"]["summary"]["ore.operacion"],
            "op-1"
        );
        // nada confirmado todavía: ficheros de datos + manifiesto + lista, sin metadata.json
        assert!(
            !cuenta
                .0
                .lock()
                .unwrap()
                .keys()
                .any(|k| k.ends_with(".metadata.json")),
            "{:?}",
            cuenta.0.lock().unwrap().keys()
        );
        let a1 = aplicar_lo_escrito(&lago, &e1, ds, None);
        let ml1 = campo(&a1, "metadata_location");
        assert!(
            ml1.starts_with("memory://pruebas/ore/v2/datasets/ventas_salida/metadata/00000-"),
            "{ml1}"
        );
        assert_eq!(campo(&a1, "nueva"), "true");
        assert_eq!(campo(&a1, "filas"), "3");
        assert_eq!(campo(&a1, "operacion"), "op-1");
        let ja1: serde_json::Value = serde_json::from_str(&a1).unwrap();
        assert_eq!(ja1["columnas_oos"]["cuando"], "DateTimeTz");
        assert_eq!(ja1["columnas_oos"]["total"], "Decimal");
        assert_eq!(ja1["retencion"]["edad_ms"], 0);

        // leer, sin cabecera de copia: una hecha del esquema, y las filas canónicas
        let l = leer(
            &lago,
            &nodo(&format!(
                "{{\"metadata_location\":\"{ml1}\",\"dataset\":\"{ds}\"}}"
            )),
        )
        .expect("lee");
        let lineas: Vec<&str> = l.lines().collect();
        assert_eq!(lineas.len(), 4, "{l}");
        let cab = nodo(lineas[0]);
        assert_eq!(
            cab.get("esquema").unwrap().1.get("id").unwrap().1.as_str(),
            Some("Integer")
        );
        assert_eq!(
            cab.get("testigo")
                .unwrap()
                .1
                .get("modo")
                .unwrap()
                .1
                .as_str(),
            Some("snapshot")
        );
        let f0 = nodo(lineas[1]);
        assert_eq!(f0.get("total").unwrap().1.as_str(), Some("0.5"));
        assert_eq!(
            f0.get("cuando").unwrap().1.as_str(),
            Some("2023-11-14 22:13:20+00"),
            "ns → µs, Madrid → UTC: el instante no cambia"
        );

        // ② anexar sobre la base, con otra clave
        let e2 = escribir(
            &lago,
            &format!("{{\"dataset\":\"{ds}\",\"modo\":\"anexar\",\"base\":\"{ml1}\",\"operacion\":\"op-2\"}}"),
            &tabla_ipc(3, 2, false)[..],
        )
        .expect("anexa");
        assert_eq!(campo(&e2, "nueva"), "false");
        assert_eq!(campo(&e2, "filas"), "5", "el total tras anexar");
        let j2: serde_json::Value = serde_json::from_str(&e2).unwrap();
        assert_eq!(j2["requirements"][0]["type"], "assert-table-uuid");
        assert_eq!(j2["requirements"][1]["type"], "assert-ref-snapshot-id");
        assert_eq!(j2["updates"][0]["action"], "add-snapshot");
        let a2 = aplicar_lo_escrito(&lago, &e2, ds, Some(&ml1));
        let ml2 = campo(&a2, "metadata_location");
        assert_ne!(ml2, ml1);
        assert_eq!(campo(&a2, "filas"), "5");
        assert_eq!(campo(&a2, "snapshots"), "2");
        // aplicar lo mismo otra vez contra la tabla que ya avanzó es un
        // conflicto: `main` ya no apunta a donde el requisito dice (contra
        // qué puntero se comprueba lo decide `ore`, que es el catálogo)
        let otra = aplicar_lo_escrito_err(&lago, &e2, ds, Some(&ml2));
        assert!(otra.contains("conflicto"), "{otra}");

        // ③ sobrescribir con una columna más: el esquema cambia dentro del commit
        let e3 = escribir(
            &lago,
            &format!("{{\"dataset\":\"{ds}\",\"base\":\"{ml2}\",\"operacion\":\"op-3\"}}"),
            &tabla_ipc(10, 4, true)[..],
        )
        .expect("sobrescribe");
        assert_eq!(campo(&e3, "esquema_cambiado"), "true");
        let j3: serde_json::Value = serde_json::from_str(&e3).unwrap();
        assert_eq!(j3["updates"][0]["action"], "add-schema");
        assert_eq!(j3["updates"][1]["action"], "set-current-schema");
        assert_eq!(j3["updates"][2]["action"], "add-snapshot");
        assert_eq!(
            j3["updates"][2]["snapshot"]["summary"]["operation"],
            "overwrite"
        );
        assert_eq!(
            j3["updates"][2]["snapshot"]["schema-id"], 1,
            "el id que el constructor le dará"
        );
        let a3 = aplicar_lo_escrito(&lago, &e3, ds, Some(&ml2));
        let ml3 = campo(&a3, "metadata_location");
        let ja3: serde_json::Value = serde_json::from_str(&a3).unwrap();
        assert_eq!(ja3["columnas"]["canal"], "string");
        assert_eq!(campo(&a3, "filas"), "4");
        let t3 = lago.abrir(&ml3, ds).unwrap();
        assert_eq!(t3.metadata().current_schema_id(), 1);
        assert_eq!(t3.metadata().snapshots().count(), 3);
        let f = lago.filas(&t3).unwrap();
        assert_eq!(f.len(), 4);
        assert_eq!(f[0]["canal"], "web");

        // ④ la ficha: cada snapshot con su clave de idempotencia; la retención de la tabla
        let h = historia(&lago, &ml3, ds).expect("historia");
        let hn: serde_json::Value = serde_json::from_str(&h).unwrap();
        assert_eq!(hn["snapshots"][0]["idempotencia"], "op-3");
        assert_eq!(hn["snapshots"][2]["idempotencia"], "op-1");
        assert_eq!(hn["retencion"]["edad_ms"], 0);
        // y recoger obedece a la tabla (0 ms) aunque nadie mande edad
        let r = recoger(&lago, ds, &ml3, None, true).expect("seco");
        assert_eq!(campo(&r, "expirados"), "2", "{r}");
        assert_eq!(campo(&r, "edad_ms"), "0");

        // ⑤ esbozada por el catálogo (`stage-create`) y escrita como Parquet:
        // nace con el uuid del esbozo y `assert-create`
        let st = esbozar(&lago, &serde_json::json!({"dataset": "datasets/ventas_pq", "peticion": {"name": "pq", "schema": {"type": "struct", "schema-id": 0, "fields": [{"id": 1, "name": "id", "type": "long", "required": false}, {"id": 2, "name": "nombre", "type": "string", "required": false}]}}}).to_string()).unwrap();
        let stj: serde_json::Value = serde_json::from_str(&st).unwrap();
        let mut pq = Vec::new();
        {
            use arrow_array::{Int32Array, StringArray};
            let esquema = Arc::new(arrow_schema::Schema::new(vec![
                arrow_schema::Field::new("id", arrow_schema::DataType::Int32, true),
                arrow_schema::Field::new("nombre", arrow_schema::DataType::Utf8, true),
            ]));
            let lote = arrow_array::RecordBatch::try_new(
                esquema.clone(),
                vec![
                    Arc::new(Int32Array::from_iter_values([1, 2])),
                    Arc::new(StringArray::from_iter_values(["a", "b"])),
                ],
            )
            .unwrap();
            let mut w = parquet::arrow::ArrowWriter::try_new(&mut pq, esquema, None).unwrap();
            w.write(&lote).unwrap();
            w.close().unwrap();
        }
        let e5 = escribir(&lago, &serde_json::json!({"dataset": "datasets/ventas_pq", "formato": "parquet", "operacion": "op-pq", "esbozo": stj["metadata"]}).to_string(), &pq[..]).expect("escribe parquet sobre el esbozo");
        let j5: serde_json::Value = serde_json::from_str(&e5).unwrap();
        assert_eq!(j5["requirements"][0]["type"], "assert-create");
        assert_eq!(j5["updates"][0]["uuid"], stj["uuid"], "el uuid del esbozo");
        assert_eq!(j5["columnas"]["id"], "long", "int32 del Parquet → long");
        let a5 = aplicar_lo_escrito(&lago, &e5, "datasets/ventas_pq", None);
        assert_eq!(campo(&a5, "uuid"), stj["uuid"].as_str().unwrap());
        assert_eq!(campo(&a5, "filas"), "2");

        // ⑥ lo que 0032 no tiene se niega con el nombre de la columna
        let mut bytes = Vec::new();
        {
            use arrow_array::UInt64Array;
            let esquema = Arc::new(arrow_schema::Schema::new(vec![arrow_schema::Field::new(
                "grande",
                arrow_schema::DataType::UInt64,
                true,
            )]));
            let lote = arrow_array::RecordBatch::try_new(
                esquema.clone(),
                vec![Arc::new(UInt64Array::from_iter_values([1u64]))],
            )
            .unwrap();
            let mut w = arrow_ipc::writer::StreamWriter::try_new(&mut bytes, &esquema).unwrap();
            w.write(&lote).unwrap();
            w.finish().unwrap();
        }
        let e = escribir(&lago, &format!("{{\"dataset\":\"{ds}\"}}"), &bytes[..]).unwrap_err();
        assert!(e.contains("`grande`") && e.contains("uint64"), "{e}");
    }

    /// `modo: upsert` (0031 §11 ⑤): copy-on-write en Arrow. Lo que había menos
    /// las claves que llegan, más lo que llega; la clave queda en la tabla y la
    /// siguiente escritura no la repite; una columna nueva no tira las de antes.
    #[test]
    fn upsert_funde_por_clave_y_declara_la_clave() {
        let cuenta: Arc<Memoria> = Arc::new(Memoria::default());
        let lago = Lago::nuevo(cuenta);
        let ds = "datasets/ventas_ups";
        let e1 = escribir(
            &lago,
            &format!("{{\"dataset\":\"{ds}\",\"modo\":\"sobrescribir\",\"operacion\":\"u-1\"}}"),
            &tabla_ipc(0, 5, false)[..],
        )
        .expect("nace");
        let ml1 = campo(
            &aplicar_lo_escrito(&lago, &e1, ds, None),
            "metadata_location",
        );
        // sin clave y sin declararla: se dice
        let sin = escribir(
            &lago,
            &format!("{{\"dataset\":\"{ds}\",\"modo\":\"upsert\",\"base\":\"{ml1}\",\"operacion\":\"u-2\"}}"),
            &tabla_ipc(3, 3, false)[..],
        )
        .unwrap_err();
        assert!(sin.contains("`upsert` quiere `clave`"), "{sin}");
        // ids 3 y 4 cambian (n3, n4 con otro total: 3..6 trae 3,4,5), 5 es nuevo → 6 filas
        let e2 = escribir(
            &lago,
            &format!("{{\"dataset\":\"{ds}\",\"modo\":\"upsert\",\"clave\":[\"id\"],\"base\":\"{ml1}\",\"operacion\":\"u-2\"}}"),
            &tabla_ipc(3, 3, false)[..],
        )
        .expect("upsert");
        assert_eq!(campo(&e2, "modo"), "upsert");
        assert_eq!(campo(&e2, "filas"), "6", "{e2}");
        assert_eq!(
            campo(&e2, "retirados"),
            "1",
            "copy-on-write: el fichero de antes se retira"
        );
        let j2: serde_json::Value = serde_json::from_str(&e2).unwrap();
        let acciones: Vec<&str> = j2["updates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|u| u["action"].as_str().unwrap())
            .collect();
        assert_eq!(
            acciones,
            ["add-snapshot", "set-snapshot-ref", "set-properties"],
            "{e2}"
        );
        assert_eq!(j2["updates"][2]["updates"]["ore.clave"], "id");
        assert_eq!(
            j2["updates"][0]["snapshot"]["summary"]["operation"],
            "overwrite"
        );
        assert_eq!(
            j2["updates"][0]["snapshot"]["summary"]["ore.modo"],
            "upsert"
        );
        assert_eq!(j2["updates"][0]["snapshot"]["summary"]["ore.clave"], "id");
        let a2 = aplicar_lo_escrito(&lago, &e2, ds, Some(&ml1));
        let ml2 = campo(&a2, "metadata_location");
        assert_eq!(campo(&a2, "filas"), "6");
        let l = leer(
            &lago,
            &nodo(&format!(
                "{{\"metadata_location\":\"{ml2}\",\"dataset\":\"{ds}\"}}"
            )),
        )
        .expect("lee");
        let filas: Vec<serde_json::Value> = l
            .lines()
            .skip(1)
            .map(|x| serde_json::from_str(x).unwrap())
            .collect();
        assert_eq!(filas.len(), 6, "{l}");
        let mut ids: Vec<&str> = filas.iter().map(|f| f["id"].as_str().unwrap()).collect();
        ids.sort();
        assert_eq!(ids, ["0", "1", "2", "3", "4", "5"]);
        // la misma clave, sin repetirla (la tabla la declara), y una columna nueva: la unión
        let e3 = escribir(
            &lago,
            &format!("{{\"dataset\":\"{ds}\",\"modo\":\"upsert\",\"base\":\"{ml2}\",\"operacion\":\"u-3\"}}"),
            &tabla_ipc(5, 2, true)[..],
        )
        .expect("upsert con la clave de la tabla");
        assert_eq!(campo(&e3, "filas"), "7");
        assert_eq!(campo(&e3, "esquema_cambiado"), "true");
        let j3: serde_json::Value = serde_json::from_str(&e3).unwrap();
        assert!(
            !j3["updates"]
                .as_array()
                .unwrap()
                .iter()
                .any(|u| u["action"] == "set-properties"),
            "la clave ya estaba: {e3}"
        );
        let a3 = aplicar_lo_escrito(&lago, &e3, ds, Some(&ml2));
        let ml3 = campo(&a3, "metadata_location");
        let l = leer(
            &lago,
            &nodo(&format!(
                "{{\"metadata_location\":\"{ml3}\",\"dataset\":\"{ds}\"}}"
            )),
        )
        .expect("lee");
        let filas: Vec<serde_json::Value> = l
            .lines()
            .skip(1)
            .map(|x| serde_json::from_str(x).unwrap())
            .collect();
        assert_eq!(filas.len(), 7, "{l}");
        let con_canal = filas.iter().filter(|f| f.get("canal").is_some()).count();
        assert_eq!(
            con_canal, 2,
            "sólo las dos nuevas traen `canal`; las de antes lo tienen nulo: {l}"
        );
        assert!(
            filas.iter().all(|f| f.get("nombre").is_some()),
            "ninguna columna de antes se tira"
        );
        // una clave que no es columna: se dice
        let mal = escribir(
            &lago,
            &format!("{{\"dataset\":\"{ds}\",\"modo\":\"upsert\",\"clave\":[\"nadie\"],\"base\":\"{ml3}\",\"operacion\":\"u-4\"}}"),
            &tabla_ipc(0, 1, false)[..],
        )
        .unwrap_err();
        assert!(mal.contains("`nadie`"), "{mal}");
    }

    /// `copiar` (0031 «(d)»): la copia de una vista cuya raíz es una tabla del
    /// lago, en Arrow y sin texto: proyecta por nombre, filtra con el literal
    /// al tipo de la columna, lleva cada columna al físico de la cabecera, y
    /// sella igual que `sellar` (la cabecera en el snapshot, `leer` la da).
    #[test]
    fn copiar_proyecta_filtra_y_sella_sin_texto() {
        let cuenta: Arc<Memoria> = Arc::new(Memoria::default());
        let lago = Lago::nuevo(cuenta);
        let ds = "datasets/ventas_origen";
        // la tabla origen: ids 0..5, n0..n4, totales 0.50..4.50
        let e1 = escribir(
            &lago,
            &format!("{{\"dataset\":\"{ds}\",\"operacion\":\"c-1\"}}"),
            &tabla_ipc(0, 5, false)[..],
        )
        .expect("nace");
        let ml = campo(
            &aplicar_lo_escrito(&lago, &e1, ds, None),
            "metadata_location",
        );
        let peticion = |filtros: &str, base: &str| {
            format!(
                "{{\"dataset\":\"copias/ventas_v\",\"fundir\":false{base},\"origen\":{{\"dataset\":\"{ds}\",\"metadata_location\":\"{ml}\",\"proyeccion\":{{\"clave\":\"id\",\"importe\":\"total\",\"quien\":\"nombre\"}},\"filtros\":{filtros}}}}}"
            )
        };
        let cab = sobre::Cabecera {
            plan: "sha256:plan".into(),
            esquema: [
                ("clave".to_string(), "Integer".to_string()),
                ("importe".to_string(), "Decimal".to_string()),
                ("quien".to_string(), "String".to_string()),
            ]
            .into(),
            testigo: sobre::Testigo {
                modo: "snapshot".into(),
                valor: Some("1".into()),
            },
            clave: Vec::new(),
            conducto: "materialization.payload".into(),
        };
        // con un filtro: el literal `2.50` contra el decimal(18,2) del origen
        let n = nodo(&peticion("[[\"total\",\"eq\",\"2.5\"]]", ""));
        let origen = n.get("origen").unwrap().1.clone();
        let c = copiar(&lago, &cab, "copias/ventas_v", None, false, &origen).expect("copia");
        assert_eq!(campo(&c, "operacion"), "creada");
        assert_eq!(campo(&c, "filas"), "1", "{c}");
        assert_eq!(
            campo(&c, "leidas"),
            "5",
            "las cinco del origen se leyeron: {c}"
        );
        let l = leer(
            &lago,
            &nodo(&format!(
                "{{\"metadata_location\":\"{}\",\"dataset\":\"copias/ventas_v\"}}",
                campo(&c, "metadata_location")
            )),
        )
        .expect("lee");
        let mut lineas = l.lines();
        let cabecera: serde_json::Value = serde_json::from_str(lineas.next().unwrap()).unwrap();
        assert_eq!(cabecera["conducto"], "materialization.payload");
        assert_eq!(cabecera["testigo"]["valor"], "1");
        let filas: Vec<serde_json::Value> =
            lineas.map(|x| serde_json::from_str(x).unwrap()).collect();
        assert_eq!(filas.len(), 1, "{l}");
        assert_eq!(filas[0]["clave"], "2");
        assert_eq!(filas[0]["importe"], "2.5");
        assert_eq!(filas[0]["quien"], "n2");
        // sin filtro, sobre la copia anterior: se sobrescribe entera
        let ml_v = campo(&c, "metadata_location");
        let n = nodo(&peticion("[]", &format!(",\"base\":\"{ml_v}\"")));
        let origen = n.get("origen").unwrap().1.clone();
        let c2 = copiar(&lago, &cab, "copias/ventas_v", Some(&ml_v), false, &origen)
            .expect("copia entera");
        assert_eq!(campo(&c2, "operacion"), "sobrescrita");
        assert_eq!(campo(&c2, "filas"), "5");
        let j: serde_json::Value = serde_json::from_str(&c2).unwrap();
        assert_eq!(j["columnas"]["importe"], 5);
        // una columna que el origen no tiene, y un filtro que no es del tipo: se dicen
        let n = nodo(&peticion("[]", "").replace("\"quien\":\"nombre\"", "\"quien\":\"apellido\""));
        let origen = n.get("origen").unwrap().1.clone();
        let e = copiar(&lago, &cab, "copias/ventas_v", None, false, &origen).unwrap_err();
        assert!(e.contains("`apellido`"), "{e}");
        let n = nodo(&peticion("[[\"id\",\"eq\",\"tres\"]]", ""));
        let origen = n.get("origen").unwrap().1.clone();
        let e = copiar(&lago, &cab, "copias/ventas_v", None, false, &origen).unwrap_err();
        assert!(e.contains("`id = tres`"), "{e}");
    }

    fn aplicar_lo_escrito_err(
        lago: &Lago,
        escrito: &str,
        dataset: &str,
        base: Option<&str>,
    ) -> String {
        let e: serde_json::Value = serde_json::from_str(escrito).unwrap();
        let pet = serde_json::json!({
            "dataset": dataset,
            "metadata_location": base,
            "requirements": e["requirements"],
            "updates": e["updates"],
        });
        aplicar(lago, &pet.to_string()).unwrap_err()
    }

    /// **Lo que PyIceberg y DuckDB mandan de verdad** (`tests/cuerpos/`, los
    /// cuerpos capturados en `medida-w3-escribir.py`), aplicado tal cual: la
    /// tabla nace de un `assert-create` con once cambios (DuckDB), dos
    /// `append` encadenados y un `overwrite` de dos snapshots (PyIceberg), y
    /// la carrera: el segundo de dos con la misma base es un conflicto.
    #[test]
    fn los_cuerpos_de_pyiceberg_y_duckdb_se_aplican() {
        let cuenta: Arc<Memoria> = Arc::new(Memoria::default());
        let lago = Lago::nuevo(cuenta.clone());
        // los cuerpos nombran `s3://copia`; aquí el almacén es `memory://pruebas`.
        // Y sus snapshots tienen la hora de la medida: se traen a ahora, porque
        // un snapshot más de un minuto anterior al último cambio de la tabla se
        // niega (la misma tolerancia que Iceberg-Java).
        let desfase = lago::ahora_ms() - 1_789_919_000_000;
        let cuerpo = |txt: &str| {
            let mut j: serde_json::Value =
                serde_json::from_str(&txt.replace("s3://copia", "memory://pruebas")).unwrap();
            fn mover(v: &mut serde_json::Value, d: i64) {
                match v {
                    serde_json::Value::Object(m) => {
                        if let Some(t) = m.get_mut("timestamp-ms").and_then(|t| t.as_i64()) {
                            m.insert("timestamp-ms".into(), serde_json::json!(t + d));
                        }
                        for x in m.values_mut() {
                            mover(x, d);
                        }
                    }
                    serde_json::Value::Array(a) => a.iter_mut().for_each(|x| mover(x, d)),
                    _ => {}
                }
            }
            mover(&mut j, desfase);
            j.to_string()
        };

        // DuckDB: `stage-create` + commit con `assert-create` y sus once cambios
        let d = cuerpo(include_str!("../tests/cuerpos/duckdb-assert-create.json"));
        let dj: serde_json::Value = serde_json::from_str(&d).unwrap();
        let pet = serde_json::json!({"dataset": "datasets/ventas_pato", "requirements": dj["requirements"], "updates": dj["updates"]});
        let a = aplicar(&lago, &pet.to_string()).expect("la tabla de DuckDB nace");
        assert_eq!(
            campo(&a, "uuid"),
            "b52eee68-6f5d-4010-8aff-48d6846d7df0",
            "el uuid que el cliente asignó"
        );
        assert_eq!(campo(&a, "snapshot"), "1216494158087733543");
        assert_eq!(campo(&a, "filas"), "10");
        let aj: serde_json::Value = serde_json::from_str(&a).unwrap();
        assert_eq!(aj["columnas_oos"]["id"], "Integer");
        assert_eq!(aj["columnas_oos"]["pais"], "String");
        assert!(
            campo(&a, "metadata_location")
                .starts_with("memory://pruebas/ore/v2/datasets/ventas_pato/metadata/00000-"),
            "{a}"
        );
        // y aplicarlo otra vez sobre lo que nació es un conflicto (`assert-create`)
        let pet2 = serde_json::json!({"dataset": "datasets/ventas_pato", "metadata_location": campo(&a, "metadata_location"), "requirements": dj["requirements"], "updates": dj["updates"]});
        let e = aplicar(&lago, &pet2.to_string()).unwrap_err();
        assert!(e.contains("conflicto"), "{e}");

        // DuckDB antes pidió `stage-create`: `esbozar` devuelve los metadatos
        // sin escribir nada (uuid, esquema con ids, ubicación)
        let st: serde_json::Value =
            serde_json::from_str(include_str!("../tests/cuerpos/duckdb-stage-create.json"))
                .unwrap();
        let objetos_antes = cuenta.0.lock().unwrap().len();
        let e = esbozar(
            &lago,
            &serde_json::json!({"dataset": "datasets/ventas_pato", "peticion": st}).to_string(),
        )
        .expect("esboza");
        let ej: serde_json::Value = serde_json::from_str(&e).unwrap();
        assert_eq!(ej["metadata"]["format-version"], 2);
        assert_eq!(ej["metadata"]["schemas"][0]["fields"][1]["name"], "pais");
        assert_eq!(
            ej["ubicacion"],
            "memory://pruebas/ore/v2/datasets/ventas_pato"
        );
        assert_eq!(
            cuenta.0.lock().unwrap().len(),
            objetos_antes,
            "esbozar no escribe"
        );

        // PyIceberg: la tabla nace de un `createTable` sin stage (su cuerpo,
        // con `crear: true`: los cambios salen de él), y después los commits
        let c: serde_json::Value = serde_json::from_str(&cuerpo(include_str!(
            "../tests/cuerpos/pyiceberg-create-table.json"
        )))
        .unwrap();
        let pet = serde_json::json!({"dataset": "datasets/ventas_escrita", "crear": true, "peticion": c,
            "retencion_defecto": {"history.expire.max-snapshot-age-ms": "604800000", "history.expire.min-snapshots-to-keep": "1"}});
        let a0 = aplicar(&lago, &pet.to_string()).expect("nace");
        assert_eq!(campo(&a0, "snapshot"), "", "sin snapshot todavía");
        let a0j: serde_json::Value = serde_json::from_str(&a0).unwrap();
        assert_eq!(
            a0j["retencion"]["edad_ms"], 1,
            "la retención que PyIceberg declaró como propiedad manda sobre el defecto"
        );
        let uuid_py = campo(&a0, "uuid");
        assert_eq!(a0j["columnas_oos"]["cuando"], "DateTimeTz");
        let mut ml = campo(&a0, "metadata_location");
        let mut filas = Vec::new();
        for (f, txt) in [
            (
                "append-1",
                include_str!("../tests/cuerpos/pyiceberg-append-1.json"),
            ),
            (
                "append-2",
                include_str!("../tests/cuerpos/pyiceberg-append-2.json"),
            ),
            (
                "overwrite",
                include_str!("../tests/cuerpos/pyiceberg-overwrite.json"),
            ),
        ] {
            let j: serde_json::Value = serde_json::from_str(
                &cuerpo(txt).replace("2a035a11-5a84-4677-99e9-10e8ceb8476c", &uuid_py),
            )
            .unwrap();
            // tal cual llegó (`peticion`), que es como `ore` lo pasa
            let pet = serde_json::json!({"dataset": "datasets/ventas_escrita", "metadata_location": ml, "peticion": j});
            let a = aplicar(&lago, &pet.to_string()).unwrap_or_else(|e| panic!("{f}: {e}"));
            ml = campo(&a, "metadata_location");
            filas.push(campo(&a, "filas"));
        }
        assert_eq!(
            filas,
            ["100000", "200000", "100000"],
            "append, append, overwrite"
        );
        let t = lago.abrir(&ml, "datasets/ventas_escrita").unwrap();
        assert_eq!(
            t.metadata().snapshots().count(),
            4,
            "el overwrite de PyIceberg son dos snapshots"
        );
        assert_eq!(t.metadata().current_snapshot_id(), Some(885001052737991215));

        // la carrera: el cuerpo de la mano que perdió (base vieja) es un conflicto
        let perdedor = r#"{"requirements": [{"type": "assert-ref-snapshot-id", "ref": "main", "snapshot-id": 3058442587801331811}, {"type": "assert-table-uuid", "uuid": "2a035a11-5a84-4677-99e9-10e8ceb8476c"}], "updates": []}"#;
        let j: serde_json::Value = serde_json::from_str(
            &perdedor.replace("2a035a11-5a84-4677-99e9-10e8ceb8476c", &uuid_py),
        )
        .unwrap();
        let pet = serde_json::json!({"dataset": "datasets/ventas_escrita", "metadata_location": ml, "requirements": j["requirements"], "updates": [{"action": "set-properties", "updates": {"x": "y"}}]});
        let e = aplicar(&lago, &pet.to_string()).unwrap_err();
        assert!(e.contains("conflicto"), "{e}");

        // DuckDB anexa sobre la tabla de PyIceberg: su cuerpo, con el uuid y
        // la base de la tabla bajo prueba (los suyos eran de otra corrida)
        let dk = cuerpo(include_str!(
            "../tests/cuerpos/duckdb-transactions-commit.json"
        ));
        let mut dj: serde_json::Value =
            serde_json::from_str(&dk.replace("2a035a11-5a84-4677-99e9-10e8ceb8476c", &uuid_py))
                .unwrap();
        let cambio = &mut dj["table-changes"][0];
        cambio["requirements"][1]["snapshot-id"] = serde_json::json!(885001052737991215i64);
        cambio["updates"][0]["snapshot"]["parent-snapshot-id"] =
            serde_json::json!(885001052737991215i64);
        let pet = serde_json::json!({"dataset": "datasets/ventas_escrita", "metadata_location": ml, "requirements": cambio["requirements"], "updates": cambio["updates"]});
        let a = aplicar(&lago, &pet.to_string()).expect("DuckDB anexa");
        assert_eq!(campo(&a, "snapshot"), "9023652766361145222");
        assert_eq!(campo(&a, "filas"), "200040");
        // la historia lo cuenta todo, y ninguno de estos trae clave de operación
        let h = historia(
            &lago,
            &campo(&a, "metadata_location"),
            "datasets/ventas_escrita",
        )
        .unwrap();
        let hj: serde_json::Value = serde_json::from_str(&h).unwrap();
        assert_eq!(hj["snapshots"].as_array().unwrap().len(), 5);
        assert_eq!(hj["snapshots"][0]["idempotencia"], "");
    }

    #[test]
    fn leer_lo_que_no_esta_lo_dice() {
        let lago = Lago::nuevo(Arc::new(Memoria::default()));
        let n = ore_core::parse::parse("{\"clave\":\"ore/v1/nadie\"}").unwrap();
        let e = leer(&lago, &n).unwrap_err();
        assert!(e.contains("no está en el almacén"), "{e}");
        let n = ore_core::parse::parse("{\"plan\":\"x\"}").unwrap();
        let e = leer(&lago, &n).unwrap_err();
        assert!(e.contains("le falta `metadata_location`"), "{e}");
    }
}
