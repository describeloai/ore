//! **La carga: las filas, tipadas, como un lote de Arrow.**
//!
//! El [ADR 0015](../../../docs/decisions/0015-el-protocolo-del-almacen.md)
//! eligió Parquet por dos cosas que un formato propio no da: **cualquier motor
//! lo lee**, y **algún día lo escribe el origen**. Desde W3.6a (0031 §10) la
//! copia es una **tabla Iceberg** y los ficheros Parquet los escribe `iceberg`
//! (`lago.rs`); lo que este módulo produce es el `RecordBatch` con el físico de
//! cada columna, y lo que sigue leyendo es Parquet — el de los ficheros de datos
//! de la tabla, para fundir y para `leer`, y el de la carga de un sobre
//! `ORECOPY1` heredado.
//!
//! # El mapa de tipos (0032, que revisa 0015)
//!
//! `0015` estrechaba solo `Integer` y `Boolean`: *«un `Decimal` metido en un
//! `double` pierde precisión en silencio, y una fecha reinterpretada a un huso
//! ajeno es peor que una cadena honesta»*. Sigue siendo verdad, y por eso la
//! tabla de [`ore_core::tipos`] no tiene ni un `double` para `Decimal` ni un
//! huso ajeno: `Decimal` → `decimal128` exacto, `DateTimeTz` → un instante en
//! UTC. Con eso la objeción se satisface y el estrechamiento se completa:
//! **todo escalar de OOS se estrecha según la tabla**.
//!
//! # La regla de lo que no analiza
//!
//! El texto que el driver entrega se analiza al sellar con la forma canónica de
//! cada escalar. **Un valor que no analiza no se inventa**: la columna entera
//! queda como `string` y el informe lo dice (`sin_estrechar`). Ni un nulo
//! silencioso, ni una copia rota por un valor: una columna de texto honesta, y
//! el aviso donde se lee. Determinista —mismos bytes para la misma entrada—,
//! que es lo que el digest exige.
//!
//! El tipo de OOS sigue viajando en la cabecera (hoy, una propiedad del
//! snapshot de la tabla), que es donde el esquema es normativo; el Parquet
//! lleva el físico, que es lo que un motor lee sin abrir la cabecera.

use arrow_array::builder::{
    BinaryBuilder, BooleanBuilder, Date32Builder, Decimal128Builder, Float64Builder, Int64Builder, StringBuilder,
    Time64MicrosecondBuilder, TimestampMicrosecondBuilder,
};
use arrow_array::{ArrayRef, RecordBatch};
use arrow_schema::{DataType, Field, Schema, TimeUnit};
use ore_base::tipos::{Fisico, Forma, Valor};
use std::collections::BTreeMap;
use std::sync::Arc;

/// La zona de un instante, tal como Iceberg la nombra en Arrow (`timestamptz`
/// → `+00:00`, y no `UTC`: la misma física, otro nombre). Se escribe así desde
/// el lote para que el esquema que sale de la tabla y el del lote casen byte a
/// byte, que es lo que el escritor exige.
pub const UTC: &str = "+00:00";

/// El físico de una columna, desde el tipo de OOS que la cabecera declara. Un
/// tipo que no analiza (no debería llegar: la cabecera la escribe `ore`) va
/// como texto, que es lo que era.
fn fisico_de(oos: &str) -> Fisico {
    ore_base::types::parse_type(oos)
        .map(|t| Fisico::de(&t))
        .unwrap_or(Fisico::Texto)
}

/// Del físico del contrato al `DataType` de Arrow. Las diez líneas que el
/// núcleo no quiso tener.
fn arrow_de(f: &Fisico) -> DataType {
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
        Fisico::Binario => DataType::Binary,
    }
}

/// El `DataType` de Arrow de un tipo de OOS tal como la cabecera lo declara:
/// lo que `lote` le da a cada columna, para que `copiar` (en Arrow, sin pasar
/// por el texto) escriba exactamente el mismo esquema que `sellar`.
///
/// v1alpha17: lo compuesto con su forma entera (0049 B1): un struct, una lista
/// y un vector, que es una lista de `float32` en el lago.
/// ORE 0067 G3·1 · El campo de Arrow de una columna del contrato, **con la marca
/// `geoarrow.wkb` y su CRS si es geo** (lo que no hace [`arrow_del_oos`], que da
/// solo el tipo): es lo que dice a [`al_contrato`] que esa columna pasa por el
/// códec, y al lago que la escriba el escritor geo.
pub fn campo_del_oos(nombre: &str, oos: &str) -> Field {
    match ore_base::types::parse_type(oos) {
        Ok(t) => campo(nombre, &Forma::de(&t)),
        Err(_) => Field::new(nombre, arrow_de(&Fisico::Texto), true),
    }
}

/// ORE 0067 G3·1 · Una columna geo del flujo de un driver, por el códec: texto
/// (hex WKB/EWKB, WKT, GeoJSON) o binario, a WKB ISO validado contra su tipo.
/// En Arrow un valor que no es del tipo no deja la columna como texto (como en
/// la copia por texto): para el lote entero, con la fila y el porqué.
pub(crate) fn geo_al_contrato(col: &ArrayRef, campo: &Field) -> Result<ArrayRef, String> {
    use arrow_array::Array;
    use arrow_array::cast::AsArray;
    let tipo = campo.metadata().get(META_TIPO).cloned().unwrap_or_default();
    let Ok(t) = ore_base::types::parse_type(&tipo) else {
        return Err(format!("`{}`: `{tipo}` no es un tipo geo", campo.name()));
    };
    let Forma::Geo {
        subtipo,
        crs,
        esferica,
    } = Forma::de(&t)
    else {
        return Err(format!("`{}`: `{tipo}` no es un tipo geo", campo.name()));
    };
    let valor = |i: usize| -> Option<ore_base::geo::Valor<'_>> {
        use ore_base::geo::Valor as V;
        if col.is_null(i) {
            return None;
        }
        Some(match col.data_type() {
            DataType::Utf8 => V::Texto(col.as_string::<i32>().value(i)),
            DataType::LargeUtf8 => V::Texto(col.as_string::<i64>().value(i)),
            DataType::Utf8View => V::Texto(col.as_string_view().value(i)),
            DataType::Binary => V::Bytes(col.as_binary::<i32>().value(i)),
            DataType::LargeBinary => V::Bytes(col.as_binary::<i64>().value(i)),
            DataType::BinaryView => V::Bytes(col.as_binary_view().value(i)),
            _ => return None,
        })
    };
    if !matches!(
        col.data_type(),
        DataType::Utf8
            | DataType::LargeUtf8
            | DataType::Utf8View
            | DataType::Binary
            | DataType::LargeBinary
            | DataType::BinaryView
    ) {
        return Err(format!(
            "la columna `{}` es `{tipo}` y llega como `{}`: una geometría llega como texto o como binario",
            campo.name(),
            col.data_type()
        ));
    }
    let mut b = arrow_array::builder::BinaryBuilder::new();
    for i in 0..col.len() {
        match valor(i) {
            None => b.append_null(),
            Some(v) => match ore_base::geo::a_wkb(v, &subtipo, &crs, esferica) {
                Ok(w) => b.append_value(w),
                Err(porque) => {
                    return Err(format!(
                        "la columna `{}` es `{tipo}` y el valor de la fila {i} no lo es: {porque}",
                        campo.name()
                    ));
                }
            },
        }
    }
    Ok(Arc::new(b.finish()) as ArrayRef)
}

pub fn arrow_del_oos(oos: &str) -> DataType {
    match ore_base::types::parse_type(oos) {
        Ok(t) => arrow_de_forma(&Forma::de(&t)),
        Err(_) => arrow_de(&Fisico::Texto),
    }
}

/// De la forma al `DataType`. Los hijos no llevan id: los pone el esquema de la
/// tabla ([`crate::lago::esquema_deseado`]), que es quien los conoce.
pub fn arrow_de_forma(f: &Forma) -> DataType {
    match f {
        Forma::Escalar(e) => arrow_de(e),
        Forma::Struct(cs) => DataType::Struct(
            cs.iter()
                .map(|(n, f)| Field::new(n, arrow_de_forma(f), true))
                .collect(),
        ),
        Forma::Lista(f) => DataType::List(Arc::new(Field::new(ELEMENTO, arrow_de_forma(f), true))),
        Forma::Vector(_) => DataType::List(Arc::new(Field::new(ELEMENTO, DataType::Float32, true))),
        // v1alpha32: WKB. La extensión `geoarrow.wkb` y su CRS van en los
        // metadatos del campo (`Forma::geoarrow_metadatos`), y el tipo lógico
        // de Parquet y la clave GeoParquet los pone el escritor del lago
        // (ORE 0067 G2).
        Forma::Geo { .. } => DataType::Binary,
    }
}

/// v1alpha32 (ORE 0067 G1·2) · **El campo de Arrow de una columna**: su tipo
/// y, si es una geometría, la extensión `geoarrow.wkb` con su CRS en los
/// metadatos (`01-la-geometria` §4). La marca va en el campo, no en el tipo:
/// es lo que GeoArrow lee, y lo que pyarrow y DuckDB entregan como geometría.
pub fn campo(nombre: &str, f: &Forma) -> Field {
    let c = Field::new(nombre, arrow_de_forma(f), true);
    match (f.geoarrow_metadatos(), f) {
        (
            Some(m),
            Forma::Geo {
                subtipo,
                crs,
                esferica,
            },
        ) => {
            // ORE 0067 G2·1: y el tipo de OOS, para que el lago lo guarde en
            // `ore.geo.<columna>` sin volver a analizar nada.
            let tipo = if *esferica {
                format!("Geography<{subtipo}>")
            } else {
                format!("Geometry<{subtipo}, {crs}>")
            };
            let tipo = ore_base::types::parse_type(&tipo)
                .map(|t| t.to_string())
                .unwrap_or(tipo);
            c.with_metadata(std::collections::HashMap::from([
                ("ARROW:extension:name".to_string(), GEOARROW_WKB.to_string()),
                ("ARROW:extension:metadata".to_string(), m),
                (META_TIPO.to_string(), tipo),
            ]))
        }
        _ => c,
    }
}

/// ORE 0067 G2·1 · La clave de los metadatos de un campo que lleva su tipo de
/// OOS (solo en las columnas geo, que Arrow no sabe nombrar enteras).
pub const META_TIPO: &str = "ore:type";

/// El nombre de la extensión de GeoArrow para WKB.
pub const GEOARROW_WKB: &str = "geoarrow.wkb";

/// El nombre del elemento de una lista, el de Iceberg y Parquet.
pub const ELEMENTO: &str = "element";

/// **El destino de una columna que llega** (0032, y v1alpha17 para lo
/// compuesto): cada escalar a su físico; un struct, campo a campo; una lista,
/// su elemento. Dentro de una lista un real **no** se ensancha: `float32` es
/// lo que un vector es (0049 B1), y `float16` sube a `float32`. Una lista de
/// tamaño fijo —un vector de numpy o de pyarrow— es una lista.
fn destino_de(nombre: &str, dt: &DataType, en_lista: bool) -> Result<DataType, String> {
    Ok(match dt {
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => DataType::Utf8,
        DataType::Int8
        | DataType::Int16
        | DataType::Int32
        | DataType::Int64
        | DataType::UInt8
        | DataType::UInt16
        | DataType::UInt32 => DataType::Int64,
        DataType::Float16 | DataType::Float32 if en_lista => DataType::Float32,
        DataType::Float16 | DataType::Float32 | DataType::Float64 => DataType::Float64,
        DataType::Boolean => DataType::Boolean,
        DataType::Decimal128(p, e) if *e >= 0 => DataType::Decimal128(*p, *e),
        DataType::Date32 | DataType::Date64 => DataType::Date32,
        DataType::Time32(_) | DataType::Time64(_) => DataType::Time64(TimeUnit::Microsecond),
        DataType::Timestamp(_, None) => DataType::Timestamp(TimeUnit::Microsecond, None),
        DataType::Timestamp(_, Some(_)) => {
            DataType::Timestamp(TimeUnit::Microsecond, Some(UTC.into()))
        }
        DataType::Dictionary(_, v) if matches!(**v, DataType::Utf8 | DataType::LargeUtf8) => {
            DataType::Utf8
        }
        DataType::Struct(cs) => {
            if cs.is_empty() {
                return Err(format!("la columna `{nombre}` es un struct sin campos"));
            }
            DataType::Struct(
                cs.iter()
                    .map(|c| {
                        destino_de(&format!("{nombre}.{}", c.name()), c.data_type(), false)
                            .map(|d| Field::new(c.name(), d, true))
                    })
                    .collect::<Result<Vec<_>, _>>()?
                    .into(),
            )
        }
        DataType::List(e) | DataType::LargeList(e) | DataType::FixedSizeList(e, _) => {
            if matches!(
                e.data_type(),
                DataType::List(_) | DataType::LargeList(_) | DataType::FixedSizeList(..)
            ) {
                return Err(format!(
                    "la columna `{nombre}` es una lista de listas: escríbela como lista de structs \
                     (OOS v1alpha17 `01` §4)"
                ));
            }
            DataType::List(Arc::new(Field::new(
                ELEMENTO,
                destino_de(&format!("{nombre}[]"), e.data_type(), true)?,
                true,
            )))
        }
        DataType::UInt64 => {
            return Err(format!(
                "la columna `{nombre}` es `uint64`: no cabe en `int64` sin mentir (0032); \
                 conviértela antes de escribir"
            ));
        }
        DataType::Null => {
            return Err(format!(
                "la columna `{nombre}` no tiene tipo (`null`): dale uno antes de escribir (0032)"
            ));
        }
        otro => {
            return Err(format!(
                "la columna `{nombre}` es `{otro}`, que el contrato de tipos (0032) no tiene: \
                 texto, entero, real, lógico, decimal, fecha, hora, fecha-hora o instante, y \
                 (v1alpha17) struct, lista y vector"
            ));
        }
    })
}

/// ORE 0067 G2·3·3 · **Una geometría que llega por IPC**: la columna binaria
/// con la marca `geoarrow.wkb`. Su CRS sale de los metadatos de la extensión,
/// como `AUTORIDAD:CÓDIGO` (el de ORE) o como PROJJSON con su `id` (el de
/// GeoPandas y DuckDB); sin CRS se niega, porque uno que no se sabe no es
/// `OGC:CRS84` (`01-la-geometria` §1). Cada valor se lee y valida
/// (`geo::comprobar`) y se guarda en WKB ISO. El subtipo es `Any`: lo que
/// declara el documento, si lo declara, manda en la tabla.
fn geo_que_llega(campo: &Field, col: &ArrayRef) -> Result<Option<(Forma, ArrayRef)>, String> {
    use arrow_array::Array;
    use arrow_array::cast::AsArray;
    if campo
        .metadata()
        .get("ARROW:extension:name")
        .map(String::as_str)
        != Some(GEOARROW_WKB)
    {
        return Ok(None);
    }
    let nombre = campo.name();
    let meta = campo
        .metadata()
        .get("ARROW:extension:metadata")
        .cloned()
        .unwrap_or_default();
    let (crs, esferica) =
        crs_de_geoarrow(&meta).map_err(|e| format!("la columna `{nombre}` · {e}"))?;
    let valor = |i: usize| -> Option<&[u8]> {
        if col.is_null(i) {
            return None;
        }
        col.as_binary_opt::<i32>()
            .map(|a| a.value(i))
            .or_else(|| col.as_binary_opt::<i64>().map(|a| a.value(i)))
            .or_else(|| col.as_binary_view_opt().map(|a| a.value(i)))
    };
    if !matches!(
        col.data_type(),
        DataType::Binary | DataType::LargeBinary | DataType::BinaryView
    ) {
        return Err(format!(
            "la columna `{nombre}` lleva la marca `geoarrow.wkb` y es `{}`: WKB es binario",
            col.data_type()
        ));
    }
    let mut b = arrow_array::builder::BinaryBuilder::new();
    for i in 0..col.len() {
        match valor(i) {
            None => b.append_null(),
            Some(v) => {
                let w = ore_base::geo::leer(v)
                    .and_then(|w| ore_base::geo::comprobar(&w, "Any", &crs, esferica).map(|_| w))
                    .map_err(|e| format!("la columna `{nombre}`, fila {i}: {e}"))?;
                b.append_value(&w.iso);
            }
        }
    }
    let forma = Forma::Geo {
        subtipo: ore_base::types::GEO_SUBTIPO_DEFECTO.to_string(),
        crs,
        esferica,
    };
    Ok(Some((forma, Arc::new(b.finish()) as ArrayRef)))
}

/// El CRS (`AUTORIDAD:CÓDIGO`) y si las aristas son esféricas, desde los
/// metadatos de la extensión `geoarrow.wkb`: `{"crs": "EPSG:25830"}`,
/// `{"crs": {…PROJJSON con "id": {"authority": "EPSG", "code": 25830}}}` o
/// `{"crs_type": "projjson", "crs": {…}}`, y `"edges": "spherical"`.
pub fn crs_de_geoarrow(meta: &str) -> Result<(String, bool), String> {
    let j: serde_json::Value = if meta.trim().is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_str(meta)
            .map_err(|e| format!("los metadatos de `geoarrow.wkb` no son JSON: {e}"))?
    };
    let esferica = j["edges"].as_str() == Some("spherical");
    let crs = match &j["crs"] {
        serde_json::Value::String(s) => {
            let s = s.trim();
            // Un PROJJSON en una cadena (algunas versiones lo serializan así).
            if s.starts_with('{') {
                let o: serde_json::Value =
                    serde_json::from_str(s).map_err(|e| format!("el CRS no es PROJJSON: {e}"))?;
                codigo_de_projjson(&o)?
            } else {
                s.to_string()
            }
        }
        o @ serde_json::Value::Object(_) => codigo_de_projjson(o)?,
        _ => {
            return Err(
                "llega sin CRS: una geometría sin CRS no se escribe (uno que no se sabe no es \
                        `OGC:CRS84`). Dale uno antes de escribir (`gdf.set_crs(\"EPSG:4326\")`)"
                    .into(),
            );
        }
    };
    // Que sea un CRS de OOS (`AUTORIDAD:CÓDIGO`): lo dice el analizador del tipo.
    ore_base::types::parse_type(&format!("Geometry<Any, {crs}>"))
        .map_err(|_| format!("`{crs}` no es un CRS que ORE sepa nombrar (`AUTORIDAD:CÓDIGO`)"))?;
    Ok((crs, esferica))
}

/// El `AUTORIDAD:CÓDIGO` de un PROJJSON, por su `id` (o el primero de `ids`).
fn codigo_de_projjson(o: &serde_json::Value) -> Result<String, String> {
    let id = if o["id"].is_object() {
        &o["id"]
    } else {
        &o["ids"][0]
    };
    match (id["authority"].as_str(), &id["code"]) {
        (Some(a), serde_json::Value::Number(n)) => Ok(format!("{a}:{n}")),
        (Some(a), serde_json::Value::String(c)) => Ok(format!("{a}:{c}")),
        _ => Err(format!(
            "el CRS `{}` no trae su código de autoridad (`id`): ORE guarda el CRS por su código",
            o["name"].as_str().unwrap_or("sin nombre")
        )),
    }
}

/// **El mismo tipo, sin los ids**: dos `DataType` que solo difieren en los
/// metadatos de sus hijos (el `PARQUET:field_id` que pone el esquema de la
/// tabla) son la misma forma.
pub fn sin_ids(dt: &DataType) -> DataType {
    match dt {
        DataType::Struct(cs) => DataType::Struct(
            cs.iter()
                .map(|c| Field::new(c.name(), sin_ids(c.data_type()), true))
                .collect(),
        ),
        DataType::List(e) => {
            DataType::List(Arc::new(Field::new(e.name(), sin_ids(e.data_type()), true)))
        }
        otro => otro.clone(),
    }
}

/// **Una columna, con el tipo de la tabla**: los mismos valores, reescritos con
/// los hijos que `destino` declara (sus nombres y sus ids). Es lo que el
/// escritor de Iceberg exige —el tipo del lote, byte a byte el del esquema— y
/// lo que un `cast` no hace sobre lo anidado. Solo vale entre formas iguales
/// ([`sin_ids`]); lo demás lo dice quien llama.
pub fn con_tipo(col: &ArrayRef, destino: &DataType) -> Result<ArrayRef, String> {
    use arrow_array::cast::AsArray;
    use arrow_array::{Array, ListArray, StructArray};
    if col.data_type() == destino {
        return Ok(col.clone());
    }
    match destino {
        DataType::Struct(cs) => {
            let s = col.as_struct_opt().ok_or("no es un struct")?;
            let hijos = cs
                .iter()
                .zip(s.columns())
                .map(|(c, h)| con_tipo(h, c.data_type()))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Arc::new(
                StructArray::try_new(cs.clone(), hijos, s.nulls().cloned())
                    .map_err(|e| e.to_string())?,
            ))
        }
        DataType::List(e) => {
            let l = col.as_list_opt::<i32>().ok_or("no es una lista")?;
            let valores = con_tipo(l.values(), e.data_type())?;
            Ok(Arc::new(
                ListArray::try_new(e.clone(), l.offsets().clone(), valores, l.nulls().cloned())
                    .map_err(|e| e.to_string())?,
            ))
        }
        _ => arrow_cast::cast(col, destino).map_err(|e| e.to_string()),
    }
}

/// La vuelta: el físico que un `DataType` leído de la carga representa. Lo que
/// esta crate no escribe (un Parquet de otro) se lee como texto si es texto, y
/// se niega si no.
fn fisico_del_arrow(d: &DataType) -> Option<Fisico> {
    Some(match d {
        DataType::Utf8 | DataType::LargeUtf8 => Fisico::Texto,
        DataType::Int64 => Fisico::Entero,
        DataType::Float64 => Fisico::Real,
        DataType::Boolean => Fisico::Logico,
        DataType::Decimal128(p, s) if *s >= 0 => Fisico::Decimal {
            precision: *p,
            escala: *s as u8,
        },
        DataType::Date32 => Fisico::Fecha,
        DataType::Time64(TimeUnit::Microsecond) => Fisico::Hora,
        DataType::Timestamp(TimeUnit::Microsecond, None) => Fisico::FechaHora,
        DataType::Timestamp(TimeUnit::Microsecond, Some(_)) => Fisico::Instante,
        _ => return None,
    })
}

/// **0032 al escribir: el lote que llega por IPC, llevado al físico del
/// contrato.** Lo que un SDK manda tiene el físico de su lenguaje —pandas
/// escribe `timestamp[ns]`, Node `int32` o `float32` cuando le viene bien,
/// pyarrow `large_utf8` o `utf8_view`— y la tabla del lago tiene **los diez
/// físicos de 0032** y ninguno más. Cada columna se convierte a su físico
/// (`cast`, sin inventar nada: los enteros cortos ensanchan a `int64`, los
/// reales a `float64`, el texto grande a texto, `ns` → `µs`, una zona → UTC
/// —el instante no cambia—, `date64` → `date32`, `time32` → `time64[µs]`), y
/// lo que el contrato no tiene **se niega con el nombre de la columna**:
/// `uint64` (no cabe en `int64` sin mentir), `null` (una columna sin tipo),
/// `decimal256`, binario, anidados.
pub fn normalizar(lote: &RecordBatch) -> Result<RecordBatch, String> {
    let mut campos = Vec::with_capacity(lote.num_columns());
    let mut columnas = Vec::with_capacity(lote.num_columns());
    for (campo, col) in lote.schema().fields().iter().zip(lote.columns()) {
        let nombre = campo.name();
        // ORE 0067 G2·3·3: una columna binaria con la marca `geoarrow.wkb` es una
        // geometría (la que da GeoPandas, DuckDB o un `write()` de otro SDK). Sin
        // la marca, un binario se sigue negando abajo.
        if let Some((forma, valores)) = geo_que_llega(campo, col)? {
            campos.push(self::campo(nombre, &forma));
            columnas.push(valores);
            continue;
        }
        let destino = destino_de(nombre, campo.data_type(), false)?;
        let valores = if col.data_type() == &destino {
            col.clone()
        } else {
            arrow_cast::cast(col, &destino).map_err(|e| {
                format!(
                    "la columna `{nombre}` no se pudo llevar de `{}` a `{destino}`: {e}",
                    col.data_type()
                )
            })?
        };
        // v1alpha17: el `cast` de lo anidado deja los hijos con los nombres de
        // Arrow (`item`); se reescriben con los del destino (`element`).
        let valores = if valores.data_type() == &destino {
            valores
        } else {
            con_tipo(&valores, &destino)
                .map_err(|e| format!("la columna `{nombre}` no toma su forma: {e}"))?
        };
        campos.push(Field::new(nombre, destino, true));
        columnas.push(valores);
    }
    RecordBatch::try_new(Arc::new(Schema::new(campos)), columnas)
        .map_err(|e| format!("el lote no cuadra tras normalizar: {e}"))
}

/// **Un lote normalizado, a los tipos que la tabla ya tiene donde no se pierde
/// nada**: un `decimal(p, s)` a un `decimal(P, s)` con `P ≥ p`; un entero a un
/// decimal (exacto); un real a un decimal (a la escala de la columna: es lo que
/// quien escribe `4` o `4.5` en una columna `decimal(10, 2)` está pidiendo, y un
/// lenguaje sin decimales —JS— no puede pedir otra cosa). Lo demás ya lo fijó
/// [`normalizar`] (un solo físico por escalar); lo que sí cambia de tipo
/// (`string` → `long`) es otra columna, con otro id, y eso lo decide el esquema.
pub fn ensanchar(lote: &RecordBatch, tabla: &iceberg::spec::Schema) -> Result<RecordBatch, String> {
    let mut campos = Vec::with_capacity(lote.num_columns());
    let mut columnas = Vec::with_capacity(lote.num_columns());
    let mut cambio = false;
    for (campo, col) in lote.schema().fields().iter().zip(lote.columns()) {
        let destino = match (
            campo.data_type(),
            tabla
                .field_by_name(campo.name())
                .map(|f| f.field_type.as_ref()),
        ) {
            (
                DataType::Decimal128(p, e),
                Some(iceberg::spec::Type::Primitive(iceberg::spec::PrimitiveType::Decimal {
                    precision,
                    scale,
                })),
            ) if *scale as i8 == *e && *precision as u8 > *p => {
                Some(DataType::Decimal128(*precision as u8, *e))
            }
            (
                DataType::Int64 | DataType::Float64,
                Some(iceberg::spec::Type::Primitive(iceberg::spec::PrimitiveType::Decimal {
                    precision,
                    scale,
                })),
            ) => Some(DataType::Decimal128(*precision as u8, *scale as i8)),
            _ => None,
        };
        match destino {
            Some(d) => {
                columnas.push(arrow_cast::cast(col, &d).map_err(|e| {
                    format!(
                        "la columna `{}` no se pudo ensanchar a `{d}`: {e}",
                        campo.name()
                    )
                })?);
                campos.push(Field::new(campo.name(), d, true));
                cambio = true;
            }
            None => {
                columnas.push(col.clone());
                campos.push(campo.as_ref().clone());
            }
        }
    }
    if !cambio {
        return Ok(lote.clone());
    }
    RecordBatch::try_new(Arc::new(Schema::new(campos)), columnas)
        .map_err(|e| format!("el lote no cuadra tras ensanchar: {e}"))
}

/// **La huella del contenido de unos lotes ya normalizados**: los valores, en
/// orden, columna a columna, por sus bytes de verdad —no los del IPC, que llevan
/// relleno y bits sin especificar y cambian entre dos lecturas de lo mismo—.
/// Es lo que hace a la clave de operación (0031 §11 ④) **de la tabla** y no de
/// cómo llegó: la misma tabla dos veces es la misma escritura.
pub fn huella(lotes: &[RecordBatch]) -> String {
    use arrow_array::cast::AsArray;
    use arrow_array::types::{
        Date32Type, Decimal128Type, Float64Type, Int64Type, Time64MicrosecondType,
        TimestampMicrosecondType,
    };
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    if let Some(primero) = lotes.first() {
        for f in primero.schema().fields() {
            h.update(f.name().as_bytes());
            h.update(b"\0");
            h.update(f.data_type().to_string().as_bytes());
            h.update(b"\0");
        }
    }
    for lote in lotes {
        for col in lote.columns() {
            for i in 0..col.len() {
                if col.is_null(i) {
                    h.update(b"\x01");
                    continue;
                }
                h.update(b"\x02");
                match col.data_type() {
                    DataType::Utf8 => h.update(col.as_string::<i32>().value(i).as_bytes()),
                    DataType::Int64 => {
                        h.update(col.as_primitive::<Int64Type>().value(i).to_le_bytes())
                    }
                    DataType::Float64 => h.update(
                        col.as_primitive::<Float64Type>()
                            .value(i)
                            .to_bits()
                            .to_le_bytes(),
                    ),
                    DataType::Boolean => h.update([col.as_boolean().value(i) as u8]),
                    DataType::Decimal128(_, _) => {
                        h.update(col.as_primitive::<Decimal128Type>().value(i).to_le_bytes())
                    }
                    DataType::Date32 => {
                        h.update(col.as_primitive::<Date32Type>().value(i).to_le_bytes())
                    }
                    DataType::Time64(TimeUnit::Microsecond) => h.update(
                        col.as_primitive::<Time64MicrosecondType>()
                            .value(i)
                            .to_le_bytes(),
                    ),
                    DataType::Timestamp(TimeUnit::Microsecond, _) => h.update(
                        col.as_primitive::<TimestampMicrosecondType>()
                            .value(i)
                            .to_le_bytes(),
                    ),
                    // tras `normalizar` no queda otro tipo; si quedara, su texto
                    otro => h.update(format!("{otro}").as_bytes()),
                }
                h.update(b"\0");
            }
        }
    }
    let d = h.finalize();
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// Las filas ya leídas: cada una es columna → valor **en texto**, que es como
/// las entrega el protocolo del driver. Una columna ausente en una fila es un
/// hueco, y aquí se escribe como nulo.
pub type Fila = BTreeMap<String, String>;

/// Lo que sale de tipar: el lote, y lo que no se pudo estrechar.
pub struct Lote {
    pub lote: RecordBatch,
    /// Columna → por qué se quedó como texto. Vacío es la copia que el
    /// contrato promete; lo que haya aquí va al informe tal cual.
    pub sin_estrechar: BTreeMap<String, String>,
}

/// **El lote tipado.** Las columnas van en el orden del esquema (por nombre,
/// que es el orden de un `BTreeMap`), cada una en el físico de la tabla de
/// 0032 o como texto si un valor no analizó.
pub fn lote(esquema: &BTreeMap<String, String>, filas: &[Fila]) -> Result<Lote, String> {
    let mut campos: Vec<Field> = Vec::with_capacity(esquema.len());
    let mut columnas: Vec<ArrayRef> = Vec::with_capacity(esquema.len());
    let mut sin_estrechar = BTreeMap::new();

    for (nombre, tipo) in esquema {
        // v1alpha32 (ORE 0067 G1·3): una geometría no es un escalar. Su texto
        // es el WKB en hexadecimal, que se lee, se valida contra el tipo y se
        // guarda como `Binary` con la marca `geoarrow.wkb`. La regla de lo que
        // no analiza es la misma: un valor que no es una geometría de la
        // columna deja la columna entera como texto, y el informe lo dice.
        if let Ok(t) = ore_base::types::parse_type(tipo)
            && let forma @ Forma::Geo { .. } = Forma::de(&t)
        {
            match columna_geo(nombre, tipo, &forma, filas) {
                Ok(col) => {
                    campos.push(campo(nombre, &forma));
                    columnas.push(col);
                }
                Err(porque) => {
                    sin_estrechar.insert(nombre.clone(), porque);
                    campos.push(Field::new(nombre, arrow_de(&Fisico::Texto), true));
                    columnas.push(construir(&Fisico::Texto, nombre, filas, Vec::new()));
                }
            }
            continue;
        }
        let pedido = fisico_de(tipo);
        // Primero se analiza todo; solo si TODO analiza se estrecha. Un valor
        // que no cabe no tira la copia ni se vuelve nulo: la columna se queda
        // texto, y se dice cuántos y cuál.
        let (fisico, valores) = match analizar_columna(&pedido, nombre, filas) {
            Ok(v) => (pedido, v),
            Err(porque) => {
                sin_estrechar.insert(nombre.clone(), porque);
                (Fisico::Texto, Vec::new())
            }
        };
        campos.push(Field::new(nombre, arrow_de(&fisico), true));
        columnas.push(construir(&fisico, nombre, filas, valores));
    }
    let schema = Arc::new(Schema::new(campos));
    let lote = RecordBatch::try_new(schema, columnas)
        .map_err(|e| format!("las columnas no cuadran con el esquema: {e}"))?;
    Ok(Lote {
        lote,
        sin_estrechar,
    })
}

/// v1alpha32 (ORE 0067 G1·3) · **Una columna geo desde el texto**: cada valor,
/// por [`ore_core::geo::valor_de_texto`] (hexadecimal → WKB validado contra
/// subtipo, dimensiones, SRID y rango → WKB ISO). `Err` dice cuántos no son
/// del tipo, enseña el primero y por qué, como [`analizar_columna`].
fn columna_geo(
    nombre: &str,
    tipo: &str,
    forma: &Forma,
    filas: &[Fila],
) -> Result<ArrayRef, String> {
    let Forma::Geo {
        subtipo,
        crs,
        esferica,
    } = forma
    else {
        unreachable!("solo se llama con una geometría")
    };
    let mut b = arrow_array::builder::BinaryBuilder::new();
    let mut malos = 0usize;
    let mut primero: Option<(&str, String)> = None;
    for f in filas {
        match f.get(nombre) {
            None => b.append_null(),
            Some(t) => match ore_base::geo::valor_de_texto(t, subtipo, crs, *esferica) {
                Ok(wkb) => b.append_value(wkb),
                Err(porque) => {
                    malos += 1;
                    b.append_null();
                    primero.get_or_insert((t, porque));
                }
            },
        }
    }
    match primero {
        None => Ok(Arc::new(b.finish()) as ArrayRef),
        Some((p, porque)) => {
            let muestra: String = p.chars().take(40).collect();
            Err(format!(
                "{malos} de {} valores no son `{tipo}`: el primero es `{muestra}` ({porque}); la                  columna se queda como texto",
                filas.len()
            ))
        }
    }
}

/// Analiza una columna entera con su físico. `Err` dice cuántos valores no son
/// del tipo y enseña el primero, que es lo que hace falta para ir al origen.
fn analizar_columna(
    fisico: &Fisico,
    nombre: &str,
    filas: &[Fila],
) -> Result<Vec<Option<Valor>>, String> {
    if *fisico == Fisico::Texto {
        return Ok(Vec::new());
    }
    let mut valores = Vec::with_capacity(filas.len());
    let mut malos = 0usize;
    let mut primero: Option<&str> = None;
    for f in filas {
        match f.get(nombre) {
            None => valores.push(None),
            Some(t) => match fisico.analizar(t) {
                Some(v) => valores.push(Some(v)),
                None => {
                    malos += 1;
                    primero.get_or_insert(t);
                }
            },
        }
    }
    match primero {
        None => Ok(valores),
        Some(p) => {
            let muestra: String = p.chars().take(40).collect();
            Err(format!(
                "{malos} de {} valores no son {}: el primero es `{muestra}`; la columna se \
                 queda como texto",
                filas.len(),
                fisico.nombre()
            ))
        }
    }
}

/// Construye la columna de Arrow. `valores` viene de [`analizar_columna`] y
/// va en el mismo orden que `filas`; para texto viene vacío y se lee de las
/// filas directamente.
fn construir(
    fisico: &Fisico,
    nombre: &str,
    filas: &[Fila],
    valores: Vec<Option<Valor>>,
) -> ArrayRef {
    macro_rules! primitiva {
        ($b:expr, $variante:path) => {{
            let mut b = $b;
            for v in &valores {
                match v {
                    Some($variante(x)) => b.append_value(*x),
                    _ => b.append_null(),
                }
            }
            Arc::new(b.finish()) as ArrayRef
        }};
    }
    match fisico {
        Fisico::Texto => {
            let mut b = StringBuilder::new();
            for f in filas {
                match f.get(nombre) {
                    Some(v) => b.append_value(v),
                    None => b.append_null(),
                }
            }
            Arc::new(b.finish()) as ArrayRef
        }
        Fisico::Entero => primitiva!(Int64Builder::new(), Valor::Entero),
        Fisico::Real => primitiva!(Float64Builder::new(), Valor::Real),
        Fisico::Logico => primitiva!(BooleanBuilder::new(), Valor::Logico),
        Fisico::Decimal { .. } => primitiva!(
            Decimal128Builder::new().with_data_type(arrow_de(fisico)),
            Valor::Decimal
        ),
        Fisico::Fecha => primitiva!(Date32Builder::new(), Valor::Fecha),
        Fisico::Hora => primitiva!(Time64MicrosecondBuilder::new(), Valor::Hora),
        Fisico::FechaHora => primitiva!(TimestampMicrosecondBuilder::new(), Valor::FechaHora),
        Fisico::Instante => primitiva!(
            TimestampMicrosecondBuilder::new().with_timezone(UTC),
            Valor::Instante
        ),
        Fisico::Binario => {
            let mut b = BinaryBuilder::new();
            for v in &valores {
                match v {
                    Some(Valor::Bytes(x)) => b.append_value(x),
                    _ => b.append_null(),
                }
            }
            Arc::new(b.finish()) as ArrayRef
        }
    }
}

/// **Volver a leer el Parquet.** Un fichero de datos de la tabla Iceberg (los
/// escribe `iceberg` con el esquema que sale de [`lote`]), o la carga de un sobre
/// heredado. Fundir un incremento con lo que ya había exige abrir lo que ya
/// había, y `leer` devuelve la copia entera por aquí.
///
/// Todo vuelve a texto, que es como entró: cada columna estrechada se escribe
/// en la forma canónica de su escalar ([`Valor::texto`]), que es la misma que
/// se analizó al sellar. Así lo que se lee de una copia y lo que llega del
/// origen son el mismo texto, y fundirlos y volver a sellar da los mismos
/// bytes.
pub fn leer(parquet: &[u8]) -> Result<Vec<Fila>, String> {
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

    let b = bytes::Bytes::copy_from_slice(parquet);
    let lector = ParquetRecordBatchReaderBuilder::try_new(b)
        .map_err(|e| format!("la carga no es un Parquet legible: {e}"))?
        .build()
        .map_err(|e| format!("no se pudo abrir la carga: {e}"))?;

    let mut out: Vec<Fila> = Vec::new();
    for lote in lector {
        let lote = lote.map_err(|e| format!("un lote de la carga no se lee: {e}"))?;
        let esquema = lote.schema();
        // v1alpha32 (ORE 0067 G1·3): una columna binaria es una geometría
        // (la única que esta carga escribe así), y vuelve como su WKB en
        // hexadecimal: el texto que se analizó al sellar, así que fundir y
        // volver a sellar da los mismos bytes. Se escribe `Binary` y la tabla
        // Iceberg la devuelve `LargeBinary` (así traduce `iceberg` su
        // `binary`): las dos son lo mismo aquí.
        let es_binaria = |d: &DataType| matches!(d, DataType::Binary | DataType::LargeBinary);
        let binaria: Vec<bool> = esquema
            .fields()
            .iter()
            .map(|c| es_binaria(c.data_type()))
            .collect();
        // ORE 0072 F1·1: la cobertura (`<geo>_bbox`) es derivada de su
        // geometría: no es texto que haya llegado, y al volver a escribir se
        // calcula otra vez. Se omite.
        let cobertura: Vec<bool> = esquema
            .fields()
            .iter()
            .map(|c| {
                matches!(c.data_type(), DataType::Struct(_))
                    && c.name()
                        .strip_suffix(crate::lago_geo::SUFIJO_COBERTURA)
                        .and_then(|g| esquema.field_with_name(g).ok())
                        .is_some_and(|g| es_binaria(g.data_type()))
            })
            .collect();
        let fisicos: Vec<Fisico> = esquema
            .fields()
            .iter()
            .enumerate()
            .map(|(k, c)| {
                if es_binaria(c.data_type()) || cobertura[k] {
                    return Ok(Fisico::Texto);
                }
                fisico_del_arrow(c.data_type()).ok_or_else(|| {
                    format!(
                        "la columna `{}` es `{}` y esta carga no lo escribe",
                        c.name(),
                        c.data_type()
                    )
                })
            })
            .collect::<Result<_, _>>()?;
        for i in 0..lote.num_rows() {
            let mut f = Fila::new();
            for (c, campo) in esquema.fields().iter().enumerate() {
                if cobertura[c] {
                    continue;
                }
                let col = lote.column(c);
                // Un nulo **no se convierte en cadena vacía**: la fila que no
                // traía la columna y la que la traía vacía no son la misma, y
                // confundirlas aquí las fundiría mal en el paso siguiente.
                if col.is_null(i) {
                    continue;
                }
                let v = if binaria[c] {
                    use arrow_array::cast::AsArray;
                    col.as_binary_opt::<i32>()
                        .map(|a| ore_base::geo::a_hex(a.value(i)))
                        .or_else(|| {
                            col.as_binary_opt::<i64>()
                                .map(|a| ore_base::geo::a_hex(a.value(i)))
                        })
                } else {
                    texto_en(col, &fisicos[c], i)
                };
                if let Some(v) = v {
                    f.insert(campo.name().clone(), v);
                }
            }
            out.push(f);
        }
    }
    Ok(out)
}

/// El texto canónico de la celda `i` de una columna de físico `fisico`
/// ([`Valor::texto`]): lo que se analizó al sellar. `None` si la columna no es
/// del tipo Arrow que su físico dice.
fn texto_en(col: &ArrayRef, fisico: &Fisico, i: usize) -> Option<String> {
    use arrow_array::cast::AsArray;
    use arrow_array::types::{
        Date32Type, Decimal128Type, Float64Type, Int64Type, Time64MicrosecondType,
        TimestampMicrosecondType,
    };
    match fisico {
        Fisico::Texto => col.as_string_opt::<i32>().map(|a| a.value(i).to_string()),
        Fisico::Entero => col
            .as_primitive_opt::<Int64Type>()
            .map(|a| Valor::Entero(a.value(i)).texto(fisico)),
        Fisico::Real => col
            .as_primitive_opt::<Float64Type>()
            .map(|a| Valor::Real(a.value(i)).texto(fisico)),
        Fisico::Logico => col
            .as_boolean_opt()
            .map(|a| Valor::Logico(a.value(i)).texto(fisico)),
        Fisico::Decimal { .. } => col
            .as_primitive_opt::<Decimal128Type>()
            .map(|a| Valor::Decimal(a.value(i)).texto(fisico)),
        Fisico::Fecha => col
            .as_primitive_opt::<Date32Type>()
            .map(|a| Valor::Fecha(a.value(i)).texto(fisico)),
        Fisico::Hora => col
            .as_primitive_opt::<Time64MicrosecondType>()
            .map(|a| Valor::Hora(a.value(i)).texto(fisico)),
        Fisico::FechaHora => col
            .as_primitive_opt::<TimestampMicrosecondType>()
            .map(|a| Valor::FechaHora(a.value(i)).texto(fisico)),
        Fisico::Instante => col
            .as_primitive_opt::<TimestampMicrosecondType>()
            .map(|a| Valor::Instante(a.value(i)).texto(fisico)),
        Fisico::Binario => col
            .as_binary_opt::<i32>()
            .map(|a| Valor::Bytes(a.value(i).to_vec()).texto(fisico)),
    }
}

/// **Las filas de un lote para enseñarlas** (la muestra de un activo): como
/// [`leer`] —el texto canónico de cada escalar que ORE escribe, y un nulo es
/// la columna que falta—, pero **sin negarse** a un tipo que la carga no
/// escribe: lo anidado de v1alpha17 (`Struct`, `List`) y lo que trae un
/// origen por la pasarela (`Int32`, `Float32`, `Timestamp` en otra unidad…)
/// salen como los escribe Arrow. Enseñar no es fundir: aquí no hay vuelta.
pub fn filas_para_ver(lote: &RecordBatch) -> Vec<Fila> {
    use arrow_cast::display::{ArrayFormatter, FormatOptions};
    let esquema = lote.schema();
    let opciones = FormatOptions::default();
    let columnas: Vec<(Option<Fisico>, Option<ArrayFormatter<'_>>)> = esquema
        .fields()
        .iter()
        .enumerate()
        .map(|(c, campo)| {
            let fisico = fisico_del_arrow(campo.data_type());
            let formato = match fisico {
                Some(_) => None,
                None => ArrayFormatter::try_new(lote.column(c).as_ref(), &opciones).ok(),
            };
            (fisico, formato)
        })
        .collect();
    (0..lote.num_rows())
        .map(|i| {
            let mut f = Fila::new();
            for (c, campo) in esquema.fields().iter().enumerate() {
                let col = lote.column(c);
                if col.is_null(i) {
                    continue;
                }
                let v = match &columnas[c] {
                    (Some(fisico), _) => texto_en(col, fisico, i),
                    (None, Some(formato)) => Some(formato.value(i).to_string()),
                    (None, None) => None,
                };
                if let Some(v) = v {
                    f.insert(campo.name().clone(), v);
                }
            }
            f
        })
        .collect()
}

/// **Del tipo de Arrow al escalar de OOS** (la muestra de una `Table` leída
/// por la pasarela): la misma vuelta que `lago::oos_de_iceberg`, y lo que el
/// contrato no tiene, `String`.
pub fn oos_de_arrow(d: &DataType) -> String {
    match d {
        DataType::Int8
        | DataType::Int16
        | DataType::Int32
        | DataType::Int64
        | DataType::UInt8
        | DataType::UInt16
        | DataType::UInt32
        | DataType::UInt64 => "Integer".into(),
        DataType::Float16 | DataType::Float32 | DataType::Float64 => "Float".into(),
        DataType::Boolean => "Boolean".into(),
        DataType::Decimal128(p, s) | DataType::Decimal256(p, s) if *s >= 0 => {
            if (*p, *s as u8) == ore_base::tipos::DECIMAL_POR_DEFECTO {
                "Decimal".into()
            } else {
                format!("Decimal<{p}, {s}>")
            }
        }
        DataType::Date32 | DataType::Date64 => "Date".into(),
        DataType::Time32(_) | DataType::Time64(_) => "Time".into(),
        DataType::Timestamp(_, None) => "DateTime".into(),
        DataType::Timestamp(_, Some(_)) => "DateTimeTz".into(),
        _ => "String".into(),
    }
}

/// **Los lotes de un Parquet, tal como están** (los tipos del fichero): lo que
/// `modo: upsert` lee de la tabla para fundir en Arrow, sin pasar por texto.
pub fn lotes_de_parquet(parquet: &[u8]) -> Result<Vec<RecordBatch>, String> {
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
    let b = bytes::Bytes::copy_from_slice(parquet);
    let lector = ParquetRecordBatchReaderBuilder::try_new(b)
        .map_err(|e| format!("la carga no es un Parquet legible: {e}"))?
        .with_batch_size(65_536)
        .build()
        .map_err(|e| format!("no se pudo abrir la carga: {e}"))?;
    lector
        .map(|l| l.map_err(|e| format!("un lote de la carga no se lee: {e}")))
        .collect()
}

/// **Un lote que se SUMA a lo que hay** (`anexar`, `upsert`), al esquema de la
/// tabla: las columnas de la tabla con SUS tipos y en su orden, y detrás las
/// nuevas del lote. Una columna que falta va nula; una de otro tipo se
/// convierte sólo si no se pierde nada (un decimal que cabe —misma escala o
/// menor, los mismos dígitos enteros o menos—, un entero o un real a decimal,
/// un entero a real), y con la conversión ESTRICTA: lo que no quepa es error,
/// no un nulo. Cualquier otro cambio de tipo se niega con el nombre de la
/// columna.
///
/// ⛔ Medido (`medida-el-sql-que-escribe.sh`): sin esto, anexar `0.5`
///   (`decimal(2, 1)`) a una columna `decimal(38, 2)` hacía otra columna con
///   otro id —la regla de [`crate::lago::esquema_deseado`], que vale al
///   sobrescribir porque se reescribe todo— y los ficheros que ya había, que
///   siguen vivos al anexar, se leían NULL: se perdía lo escrito.
pub fn conformar(lote: &RecordBatch, tabla: &iceberg::spec::Schema) -> Result<RecordBatch, String> {
    let de_la_tabla = iceberg::arrow::schema_to_arrow_schema(tabla)
        .map_err(|e| format!("el esquema de la tabla no pasa a Arrow: {e}"))?;
    let estricta = arrow_cast::CastOptions {
        safe: false,
        ..Default::default()
    };
    let n = lote.num_rows();
    let mut campos = Vec::with_capacity(de_la_tabla.fields().len());
    let mut columnas: Vec<ArrayRef> = Vec::with_capacity(de_la_tabla.fields().len());
    for f in de_la_tabla.fields() {
        let destino = f.data_type();
        let col = match lote.column_by_name(f.name()) {
            None => arrow_array::new_null_array(destino, n),
            Some(c) if c.data_type() == destino => c.clone(),
            // v1alpha17: la misma forma, sin los ids de la tabla.
            Some(c) if sin_ids(c.data_type()) == sin_ids(destino) => con_tipo(c, destino)?,
            Some(c) if sin_perdida(c.data_type(), destino) => {
                arrow_cast::cast_with_options(c, destino, &estricta).map_err(|e| {
                    format!(
                        "la columna `{}` (`{}`) no cabe en la de la tabla (`{destino}`): {e}",
                        f.name(),
                        c.data_type()
                    )
                })?
            }
            Some(c) => {
                return Err(format!(
                    "la columna `{}` es `{destino}` en la tabla y llega `{}`: al añadir filas \
                     no se cambia el tipo de una columna (las que ya hay la perderían); \
                     conviértela a `{destino}`, o sobrescribe la tabla",
                    f.name(),
                    c.data_type()
                ));
            }
        };
        // ORE 0072 F1·1: la marca geo del lote sigue con la columna; sin ella
        // un `append` escribía la geometría como un binario sin tipo, sin
        // orden Hilbert y sin cajas (y sin cobertura).
        let marca: std::collections::HashMap<String, String> = lote
            .schema()
            .field_with_name(f.name())
            .map(|l| {
                l.metadata()
                    .iter()
                    .filter(|(k, _)| {
                        k.as_str() == "ARROW:extension:name"
                            || k.as_str() == "ARROW:extension:metadata"
                            || k.as_str() == META_TIPO
                    })
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect()
            })
            .unwrap_or_default();
        campos.push(Field::new(f.name(), destino.clone(), true).with_metadata(marca));
        columnas.push(col);
    }
    // ORE 0072 F1·1: una tabla tiene `<col>_bbox` sólo si TODOS sus ficheros la
    // llevan (quien pode por ella descartaría las filas sin cobertura). Al
    // anexar a una que no la tiene no se añade: llega con la próxima
    // sobrescritura, que reescribe todos los ficheros.
    let coberturas: Vec<String> = crate::lago_geo::columnas_geo(&lote.schema())
        .iter()
        .map(|g| crate::lago_geo::nombre_cobertura(&g.nombre))
        .collect();
    for (f, c) in lote.schema().fields().iter().zip(lote.columns()) {
        if coberturas.contains(f.name()) && de_la_tabla.field_with_name(f.name()).is_err() {
            continue;
        }
        if de_la_tabla.field_with_name(f.name()).is_err() {
            campos.push(f.as_ref().clone().with_nullable(true));
            columnas.push(c.clone());
        }
    }
    RecordBatch::try_new(Arc::new(Schema::new(campos)), columnas)
        .map_err(|e| format!("el lote no cuadra con la tabla: {e}"))
}

/// ¿Pasa un valor de `de` a `a` sin perder nada (o, de real a decimal, a la
/// escala de la columna, que es lo que quien lo escribe pide)?
fn sin_perdida(de: &DataType, a: &DataType) -> bool {
    match (de, a) {
        (DataType::Decimal128(p, s), DataType::Decimal128(pp, ss)) => {
            *s >= 0 && s <= ss && (*p as i16 - *s as i16) <= (*pp as i16 - *ss as i16)
        }
        (DataType::Int64 | DataType::Float64, DataType::Decimal128(..)) => true,
        (DataType::Int64, DataType::Float64) => true,
        // ORE 0072 F1·0: DuckDB entrega la geometría (y cualquier binario) como
        // `Binary` y la tabla lo guarda como `LargeBinary`: los mismos bytes con
        // desplazamientos de 64 bits. Sin esto, un `append` geo fallaba siempre.
        (DataType::Binary, DataType::LargeBinary) => true,
        _ => false,
    }
}

/// Si convertir `de` en `a` es seguro para un decimal: entre dos decimales,
/// solo si ensancha ([`sin_perdida`]); cualquier otro par no es asunto de esta
/// guarda y sigue decidiéndolo el cast.
fn decimal_ensancha(de: &DataType, a: &DataType) -> bool {
    match (de, a) {
        (DataType::Decimal128(..), DataType::Decimal128(..)) => sin_perdida(de, a),
        _ => true,
    }
}

/// **Un lote al esquema `destino`, por nombre**: las columnas que faltan van
/// nulas (un fichero de antes de que existieran), una de otro tipo se
/// convierte si Arrow sabe (`cast`) y si no se dice con su nombre, y una que
/// `destino` no tiene se queda fuera: la dejó fuera una escritura anterior
/// (el esquema de la tabla sigue al lote, y un fichero viejo puede llevar
/// columnas que la tabla ya no declara). Lo que llega nunca pierde nada:
/// `destino` es la unión de la tabla y del lote.
pub fn al_esquema(lote: &RecordBatch, destino: &Arc<Schema>) -> Result<RecordBatch, String> {
    let n = lote.num_rows();
    let columnas = destino
        .fields()
        .iter()
        .map(|campo| -> Result<ArrayRef, String> {
            match lote.column_by_name(campo.name()) {
                None => Ok(arrow_array::new_null_array(campo.data_type(), n)),
                Some(col) if col.data_type() == campo.data_type() => Ok(col.clone()),
                // v1alpha17: la misma forma con otros ids (el lote frente a la
                // tabla) no es otro tipo: se reescribe con los del destino.
                Some(col) if sin_ids(col.data_type()) == sin_ids(campo.data_type()) => {
                    con_tipo(col, campo.data_type())
                }
                // **Un decimal solo se convierte si ensancha** (02-entity §3.4).
                // `arrow_cast` de `decimal(38, 18)` a `decimal(10, 2)` devuelve
                // `Ok` redondeando `0.005` a `0.01` y dejando en NULL lo que no
                // cabe; con `safe: false` detecta el desbordamiento pero sigue
                // redondeando (medido el 2026-09-26). Así que la pregunta no se
                // le hace al cast: se niega antes, y la copia se rehace entera.
                Some(col) if !decimal_ensancha(col.data_type(), campo.data_type()) => Err(format!(
                    "la columna `{}` es `{}` en un fichero y `{}` en la tabla: convertirla \
                     perdería cifras —el cast redondea sin decirlo—, así que no se funde. \
                     Rehaz la copia entera (`ore materialize --rehacer`)",
                    campo.name(),
                    col.data_type(),
                    campo.data_type()
                )),
                Some(col) => arrow_cast::cast(col, campo.data_type()).map_err(|e| {
                    format!(
                        "la columna `{}` es `{}` en un fichero y `{}` en la tabla, y no se convierte: {e}",
                        campo.name(),
                        col.data_type(),
                        campo.data_type()
                    )
                }),
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    RecordBatch::try_new(destino.clone(), columnas)
        .map_err(|e| format!("el lote no construye: {e}"))
}

/// ORE 0051 P6 · **Una columna que nunca es nula no trae un nulo.** Se mira
/// antes de escribir, lote a lote, y la primera fila que lo traiga niega la
/// copia entera con su columna y su número (`desde` es cuántas filas iban
/// antes de este lote): no hay snapshot, y la copia de antes sigue servida.
///
/// El escritor de Iceberg también lo negaría —la columna es `required`—, pero
/// con un error de Arrow que no dice ni qué fila ni qué hacer. Y lo que hay
/// que hacer casi siempre es lo mismo: el origen dejó de garantizarlo, y
/// catalogarlo otra vez lo afloja (P3, P5).
pub fn sin_nulos(
    lote: &RecordBatch,
    obligatorias: &std::collections::BTreeSet<String>,
    desde: usize,
    dataset: &str,
) -> Result<(), String> {
    use arrow_array::Array;
    for c in obligatorias {
        let Some(col) = lote.column_by_name(c) else {
            continue;
        };
        if col.null_count() == 0 {
            continue;
        }
        let fila = (0..col.len()).find(|&i| col.is_null(i)).unwrap_or(0);
        return Err(format!(
            "la columna `{c}` de `{dataset}` nunca es nula —lo garantiza su origen— y la fila {} \
             trae un nulo: la copia no se escribe. Si el origen dejó de garantizarlo, vuelve a \
             catalogar la fuente y la columna se afloja",
            desde + fila + 1
        ));
    }
    Ok(())
}

/// **Un lote que llega de un driver, al contrato de la cabecera** (ADR 0043).
///
/// Más estricto que [`al_esquema`], porque aquí nada es de un fichero viejo: el
/// driver entrega exactamente la proyección.
///
/// - una columna que falta, o que sobra, se dice con su nombre;
/// - la conversión es la de Arrow **sin su modo seguro**: un valor que no
///   convierte es un error, no un nulo;
/// - un decimal que estrecha —la Storage Read da todo NUMERIC como
///   `decimal(38, 9)`, y el contrato dice `Decimal<10, 2>`— se convierte y se
///   **vuelve a convertir**: si no sale idéntico, se perdían cifras y se niega.
///   El cast redondea sin decirlo (medido el 2026-09-26); la vuelta lo dice.
pub fn al_contrato(lote: &RecordBatch, destino: &Arc<Schema>) -> Result<RecordBatch, String> {
    if let Some(sobra) = lote
        .schema()
        .fields()
        .iter()
        .find(|f| destino.field_with_name(f.name()).is_err())
    {
        return Err(format!(
            "el flujo trae `{}`, y el contrato no la declara",
            sobra.name()
        ));
    }
    let estricto = arrow_cast::CastOptions {
        safe: false,
        ..Default::default()
    };
    let columnas = destino
        .fields()
        .iter()
        .map(|campo| -> Result<ArrayRef, String> {
            let col = lote.column_by_name(campo.name()).ok_or_else(|| {
                format!(
                    "el contrato declara `{}` y el flujo no la trae",
                    campo.name()
                )
            })?;
            // ORE 0067 G3·1: una geometría pasa por el códec, nunca por un `cast`
            // (medido en G3·0: `Utf8` → `Binary` guardaba los bytes del texto hex).
            if campo
                .metadata()
                .get("ARROW:extension:name")
                .map(String::as_str)
                == Some(GEOARROW_WKB)
            {
                return geo_al_contrato(col, campo);
            }
            if col.data_type() == campo.data_type() {
                return Ok(col.clone());
            }
            let convertir = |c: &ArrayRef, a: &DataType| {
                arrow_cast::cast_with_options(c, a, &estricto).map_err(|e| {
                    format!(
                        "la columna `{}` llega como `{}` y no convierte a `{}` (lo que su contrato \
                        declara): {e}",
                        campo.name(),
                        col.data_type(),
                        campo.data_type()
                    )
                })
            };
            let ida = convertir(col, campo.data_type())?;
            if let (DataType::Decimal128(..), DataType::Decimal128(..)) =
                (col.data_type(), campo.data_type())
                && !sin_perdida(col.data_type(), campo.data_type())
            {
                use arrow_array::cast::AsArray;
                let vuelta = convertir(&ida, col.data_type())?;
                let (a, b) = (
                    col.as_primitive::<arrow_array::types::Decimal128Type>(),
                    vuelta.as_primitive::<arrow_array::types::Decimal128Type>(),
                );
                if a.iter().zip(b.iter()).any(|(x, y)| x != y) {
                    return Err(format!(
                        "la columna `{}` llega como `{}` y un valor no cabe en `{}` sin perder \
                        cifras: el contrato es más estrecho que el origen",
                        campo.name(),
                        col.data_type(),
                        campo.data_type()
                    ));
                }
            }
            Ok(ida)
        })
        .collect::<Result<Vec<_>, _>>()?;
    RecordBatch::try_new(destino.clone(), columnas)
        .map_err(|e| format!("el lote no construye: {e}"))
}

/// El texto de la clave de cada fila: los valores de las columnas de `clave`
/// tal como Arrow los enseña, separados por `\0` (un nulo es `\x01`, que
/// ningún valor lleva).
fn claves_de(lote: &RecordBatch, clave: &[String]) -> Result<Vec<String>, String> {
    use arrow_cast::display::{ArrayFormatter, FormatOptions};
    let opciones = FormatOptions::default();
    let columnas = clave
        .iter()
        .map(|c| {
            lote.column_by_name(c)
                .ok_or_else(|| format!("la clave nombra `{c}`, que no es una columna de la tabla"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let formatos = columnas
        .iter()
        .map(|c| ArrayFormatter::try_new(c.as_ref(), &opciones).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    let mut out = Vec::with_capacity(lote.num_rows());
    for i in 0..lote.num_rows() {
        let mut k = String::new();
        for (c, f) in columnas.iter().zip(&formatos) {
            if c.is_null(i) {
                k.push('\x01');
            } else {
                k.push_str(&f.value(i).to_string());
            }
            k.push('\0');
        }
        out.push(k);
    }
    Ok(out)
}

/// **La fusión por clave, en Arrow** (`modo: upsert`, 0031 §11 ⑤): lo que
/// había **menos las filas cuya clave trae el lote nuevo**, más el lote nuevo.
/// Copy-on-write: el resultado se escribe entero y ningún lector tiene que
/// aplicar nada. Los dos lados ya vienen al mismo esquema (`al_esquema`).
/// Dos filas con la misma clave dentro de lo nuevo se quedan las dos: el
/// escritor no decide cuál gana, y el catálogo lo enseña tal cual.
pub fn fundir_lotes(
    viejos: Vec<RecordBatch>,
    nuevos: &[RecordBatch],
    clave: &[String],
) -> Result<Vec<RecordBatch>, String> {
    use arrow_array::BooleanArray;
    use arrow_select::filter::filter_record_batch;
    use std::collections::HashSet;
    if clave.is_empty() {
        return Err("`upsert` quiere `clave`: las columnas que identifican una fila".into());
    }
    let mut llegan: HashSet<String> = HashSet::new();
    for l in nuevos {
        llegan.extend(claves_de(l, clave)?);
    }
    let mut out = Vec::with_capacity(viejos.len() + nuevos.len());
    for v in viejos {
        if v.num_rows() == 0 {
            continue;
        }
        let ks = claves_de(&v, clave)?;
        let quedan: BooleanArray = ks.iter().map(|k| Some(!llegan.contains(k))).collect();
        let f = filter_record_batch(&v, &quedan).map_err(|e| format!("no se pudo filtrar: {e}"))?;
        if f.num_rows() > 0 {
            out.push(f);
        }
    }
    out.extend(nuevos.iter().cloned());
    Ok(out)
}

/// **La fusión: lo que había, más el incremento, por clave.**
///
/// Es la operación que convierte *«leer menos»* en *«leer menos y seguir estando
/// entera»*, y las dos hacen falta: un refresco que lee 10 filas y sella una
/// copia de 10 no es más rápido, es **incorrecto**.
///
/// # Por qué esto no es el circuito Δ
///
/// Se miró si `ore-maintain` (retirado en 0040 paso 6) servía, y no: [ADR 0013](../../../docs/decisions/0013-el-protocolo-del-mantenedor.md)
/// dice de él *«la sesión ES el estado, y cerrarla es tirarlo»*. Ese estado es
/// efímero por decisión; el de una copia **sobrevive**, y vive en un objeto que
/// solo este programa puede abrir. Reusarlo habría sido forzar la pieza.
///
/// Lo que queda aquí es mecánica de datos y **ninguna semántica**: unas columnas
/// identifican una fila, y la fila nueva gana. El almacén sigue sin saber qué es
/// una entidad.
///
/// # El orden es estable, y hace falta que lo sea
///
/// El resultado se ordena por la clave. Si dependiera del orden de llegada, dos
/// refrescos que trajeran el mismo incremento en distinto orden darían Parquets
/// distintos — y el artefacto dejaría de poder nombrarse por su digest.
pub fn fundir(anteriores: Vec<Fila>, delta: Vec<Fila>, clave: &[String]) -> Vec<Fila> {
    let k = |f: &Fila| -> Vec<String> {
        clave
            .iter()
            .map(|c| f.get(c).cloned().unwrap_or_default())
            .collect()
    };
    let mut por_clave: BTreeMap<Vec<String>, Fila> =
        anteriores.into_iter().map(|f| (k(&f), f)).collect();
    for f in delta {
        por_clave.insert(k(&f), f);
    }
    por_clave.into_values().collect()
}

#[cfg(test)]
mod tests {
    /// ORE 0067 G2·3·3 · **Lo que llega por IPC con la marca `geoarrow.wkb`**:
    /// el CRS de GeoPandas (PROJJSON con `id`), de ORE (`AUTORIDAD:CÓDIGO`) o
    /// ninguno (se niega); el WKB se valida y queda ISO; sin marca, un binario
    /// se sigue negando.
    #[test]
    fn normalizar_acepta_la_geometria_marcada() {
        use arrow_array::{Array, BinaryArray, Int64Array};
        let projjson = r#"{"crs": {"$schema": "https://proj.org/schemas/v0.7/projjson.schema.json", "type": "ProjectedCRS", "name": "ETRS89 / UTM zone 30N", "id": {"authority": "EPSG", "code": 25830}}}"#;
        assert_eq!(
            crs_de_geoarrow(projjson).unwrap(),
            ("EPSG:25830".to_string(), false)
        );
        assert_eq!(
            crs_de_geoarrow(r#"{"crs":"OGC:CRS84","edges":"spherical"}"#).unwrap(),
            ("OGC:CRS84".to_string(), true)
        );
        assert_eq!(
            crs_de_geoarrow(
                r#"{"crs_type":"projjson","crs":{"ids":[{"authority":"OGC","code":"CRS84"}]}}"#
            )
            .unwrap()
            .0,
            "OGC:CRS84"
        );
        assert!(crs_de_geoarrow("{}").unwrap_err().contains("sin CRS"));
        assert!(
            crs_de_geoarrow(r#"{"crs":{"type":"GeographicCRS","name":"propio"}}"#)
                .unwrap_err()
                .contains("código de autoridad")
        );

        let ewkb =
            ore_base::geo::de_hex("0101000020E6100000406A1327F7BB0DC053E8BCC62E454440").unwrap();
        let marca = |m: &str| {
            std::collections::HashMap::from([
                ("ARROW:extension:name".to_string(), GEOARROW_WKB.to_string()),
                ("ARROW:extension:metadata".to_string(), m.to_string()),
            ])
        };
        let lote = |campo: Field, col: ArrayRef| {
            RecordBatch::try_new(
                Arc::new(Schema::new(vec![
                    Field::new("id", DataType::Int32, true),
                    campo,
                ])),
                vec![Arc::new(arrow_array::Int32Array::from(vec![1, 2])), col],
            )
            .unwrap()
        };
        let col: ArrayRef = Arc::new(arrow_array::LargeBinaryArray::from(vec![
            Some(ewkb.as_slice()),
            None,
        ]));
        let n = normalizar(&lote(
            Field::new("geom", DataType::LargeBinary, true)
                .with_metadata(marca(r#"{"crs":"EPSG:4326"}"#)),
            col.clone(),
        ))
        .unwrap();
        let f = n.schema().field_with_name("geom").unwrap().clone();
        assert_eq!(f.data_type(), &DataType::Binary);
        assert_eq!(
            f.metadata()["ARROW:extension:metadata"],
            r#"{"crs":"EPSG:4326"}"#
        );
        assert_eq!(f.metadata()[META_TIPO], "Geometry<Any, EPSG:4326>");
        let g = n
            .column_by_name("geom")
            .unwrap()
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        assert_eq!(
            ore_base::geo::a_hex(g.value(0)),
            "0101000000406A1327F7BB0DC053E8BCC62E454440"
        );
        assert!(g.is_null(1));
        assert!(
            n.column_by_name("id")
                .unwrap()
                .as_any()
                .downcast_ref::<Int64Array>()
                .is_some()
        );
        // El SRID del EWKB no casa con el CRS declarado: se niega con la fila.
        let e = normalizar(&lote(
            Field::new("geom", DataType::LargeBinary, true)
                .with_metadata(marca(r#"{"crs":"EPSG:25830"}"#)),
            col.clone(),
        ))
        .unwrap_err();
        assert!(e.contains("fila 0") && e.contains("reproyectar"), "{e}");
        // Sin marca, un binario sigue sin tener sitio en el contrato.
        assert!(normalizar(&lote(Field::new("geom", DataType::LargeBinary, true), col)).is_err());
    }

    /// ORE 0067 G2·3·4 · **El Parquet que escribe DuckDB** (lo que `write()` de
    /// Node manda, `formato: parquet`): GEOMETRY con el CRS en PROJJSON en el
    /// tipo lógico. `parquet` lo da como la marca `geoarrow.wkb`, y `normalizar`
    /// saca de él `EPSG:25830`.
    #[test]
    fn el_parquet_geo_de_duckdb_llega_con_su_crs() {
        let bytes = include_bytes!("../tests/cuerpos/geo-duckdb-25830.parquet");
        let lotes = lotes_de_parquet(bytes).unwrap();
        let n = normalizar(&lotes[0]).unwrap();
        let f = n.schema().field_with_name("geom").unwrap().clone();
        assert_eq!(
            f.metadata()["ARROW:extension:metadata"],
            r#"{"crs":"EPSG:25830"}"#,
            "{:?}",
            lotes[0].schema()
        );
        assert_eq!(f.metadata()[META_TIPO], "Geometry<Any, EPSG:25830>");
    }

    /// v1alpha32 (ORE 0067 G1·3) · **La copia de una columna geo**: el texto del
    /// driver (el EWKB de PostGIS, en hexadecimal) sale del lote como `Binary`
    /// marcado con WKB ISO; el Parquet vuelve por `leer` como el hexadecimal
    /// ISO, y volver a sellarlo da los mismos bytes. Un valor que no es del
    /// tipo deja la columna como texto y lo dice.
    #[test]
    fn la_copia_lleva_la_geometria_al_lote() {
        use arrow_array::{Array, BinaryArray};
        let pg = "0101000020E6100000406A1327F7BB0DC053E8BCC62E454440";
        let iso = "0101000000406A1327F7BB0DC053E8BCC62E454440";
        let esquema = BTreeMap::from([
            ("geom".to_string(), "Geometry<Point>".to_string()),
            ("id".to_string(), "Integer".to_string()),
        ]);
        let filas: Vec<Fila> = vec![
            BTreeMap::from([("id".into(), "1".into()), ("geom".into(), pg.into())]),
            BTreeMap::from([("id".into(), "2".into())]),
        ];
        let Lote {
            lote: l,
            sin_estrechar,
        } = lote(&esquema, &filas).unwrap();
        assert!(sin_estrechar.is_empty(), "{sin_estrechar:?}");
        let campo_geom = l.schema().field_with_name("geom").unwrap().clone();
        assert_eq!(campo_geom.data_type(), &DataType::Binary);
        assert_eq!(campo_geom.metadata()["ARROW:extension:name"], GEOARROW_WKB);
        assert_eq!(
            campo_geom.metadata()["ARROW:extension:metadata"],
            r#"{"crs":"OGC:CRS84"}"#
        );
        let col = l
            .column_by_name("geom")
            .unwrap()
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        assert_eq!(ore_base::geo::a_hex(col.value(0)), iso);
        assert!(col.is_null(1));
        // Por el Parquet y de vuelta: el hexadecimal ISO, y la nula sigue nula.
        let mut buf = Vec::new();
        let mut w = parquet::arrow::ArrowWriter::try_new(&mut buf, l.schema(), None).unwrap();
        w.write(&l).unwrap();
        w.close().unwrap();
        let vuelta = leer(&buf).unwrap();
        assert_eq!(vuelta[0]["geom"], iso);
        assert!(!vuelta[1].contains_key("geom"));
        let otra = lote(&esquema, &vuelta).unwrap().lote;
        assert_eq!(
            otra.column_by_name("geom").unwrap().to_data(),
            l.column_by_name("geom").unwrap().to_data()
        );
        // ORE 0067 G3·1: un WKT entra por el códec (antes se quedaba texto).
        let wkt = vec![BTreeMap::from([
            ("id".into(), "1".into()),
            ("geom".into(), "POINT(1 2)".into()),
        ])];
        let l = lote(&esquema, &wkt).unwrap().lote;
        assert_eq!(
            leer_geom_hex(&l),
            "0101000000000000000000F03F0000000000000040"
        );
        // Lo que no es geometría, o un polígono en una columna de puntos: la
        // columna se queda texto.
        for (malo, dice) in [
            ("hola", "WKB, WKT ni GeoJSON"),
            ("010300000000000000", "un `Polygon` en una columna `Point`"),
        ] {
            let filas = vec![BTreeMap::from([
                ("id".into(), "1".into()),
                ("geom".into(), malo.into()),
            ])];
            let Lote {
                lote: l,
                sin_estrechar,
            } = lote(&esquema, &filas).unwrap();
            let e = &sin_estrechar["geom"];
            assert!(
                e.contains("no son `Geometry<Point>`") && e.contains(dice),
                "{e}"
            );
            assert_eq!(
                l.schema().field_with_name("geom").unwrap().data_type(),
                &DataType::Utf8
            );
        }
    }

    fn leer_geom_hex(l: &RecordBatch) -> String {
        use arrow_array::cast::AsArray;
        ore_base::geo::a_hex(
            l.column_by_name("geom")
                .unwrap()
                .as_binary::<i32>()
                .value(0),
        )
    }

    /// v1alpha32 (ORE 0067 G1·2) · **Ida y vuelta del contrato entre capas**:
    /// el hexadecimal del protocolo → WKB validado → una columna de Arrow con
    /// la marca `geoarrow.wkb` y su CRS → otra vez el hexadecimal. Con nula,
    /// vacía, `Z` y un WKB roto, que no llega a la columna.
    #[test]
    fn una_geometria_va_y_vuelve_por_arrow() {
        use arrow_array::{Array, BinaryArray};
        let forma = Forma::de(&ore_base::types::parse_type("Geometry<Any, EPSG:4326>").unwrap());
        let Forma::Geo {
            subtipo,
            crs,
            esferica,
        } = &forma
        else {
            unreachable!()
        };
        let entrada = [
            Some("0101000020E6100000406A1327F7BB0DC053E8BCC62E454440"), // EWKB de PostGIS
            None,                                                       // nula
            Some("0101000000000000000000F87F000000000000F87F"),         // POINT EMPTY
            Some("01E9030000000000000000F03F00000000000000400000000000000840"), // POINT Z (1 2 3)
        ];
        let valores: Vec<Option<Vec<u8>>> = entrada
            .iter()
            .map(|v| v.map(|h| ore_base::geo::valor_de_texto(h, subtipo, crs, *esferica).unwrap()))
            .collect();
        let columna = BinaryArray::from_iter(valores.iter().map(|v| v.as_deref()));
        let c = campo("geom", &forma);
        assert_eq!(c.data_type(), &DataType::Binary);
        assert_eq!(c.metadata()["ARROW:extension:name"], GEOARROW_WKB);
        assert_eq!(
            c.metadata()["ARROW:extension:metadata"],
            r#"{"crs":"EPSG:4326"}"#
        );
        assert!(columna.is_null(1) && !columna.is_null(2));
        let vuelta: Vec<Option<String>> = (0..columna.len())
            .map(|i| (!columna.is_null(i)).then(|| ore_base::geo::a_hex(columna.value(i))))
            .collect();
        assert_eq!(
            vuelta,
            vec![
                Some("0101000000406A1327F7BB0DC053E8BCC62E454440".into()), // ISO, sin SRID
                None,
                Some("0101000000000000000000F87F000000000000F87F".into()),
                Some("01E9030000000000000000F03F00000000000000400000000000000840".into()),
            ]
        );
        // Lo roto no llega a la columna: lo dice quien escribe.
        assert!(ore_base::geo::valor_de_texto("0101000000", subtipo, crs, *esferica).is_err());
        // Una geografía lleva sus aristas en la marca.
        let g = campo(
            "ruta",
            &Forma::de(&ore_base::types::parse_type("Geography<LineString>").unwrap()),
        );
        assert_eq!(
            g.metadata()["ARROW:extension:metadata"],
            r#"{"crs":"OGC:CRS84","edges":"spherical"}"#
        );
        // Una columna sin geometría no lleva marca.
        assert!(
            campo(
                "id",
                &Forma::de(&ore_base::types::parse_type("String").unwrap())
            )
            .metadata()
            .is_empty()
        );
    }

    use super::*;

    fn esquema() -> BTreeMap<String, String> {
        [
            ("activo", "Boolean"),
            ("id", "Integer"),
            ("pais", "String"),
            ("total", "Decimal"),
            ("peso", "Float"),
            ("dia", "Date"),
            ("hora", "Time"),
            ("cuando", "DateTime"),
            ("visto", "DateTimeTz"),
            ("importe", "Money<EUR, 2>"),
        ]
        .into_iter()
        .map(|(c, t)| (c.to_string(), t.to_string()))
        .collect()
    }

    fn fila(pares: &[(&str, &str)]) -> Fila {
        pares
            .iter()
            .map(|(c, v)| (c.to_string(), v.to_string()))
            .collect()
    }

    fn filas() -> Vec<Fila> {
        vec![
            fila(&[
                ("activo", "true"),
                ("id", "1"),
                ("pais", "ES"),
                ("total", "10.50"),
                ("peso", "1.5"),
                ("dia", "2020-02-29"),
                ("hora", "10:00:00.5"),
                ("cuando", "2020-02-29 10:00:00.5"),
                ("visto", "2020-02-29 10:00:00.5+00"),
                ("importe", "99.99"),
            ]),
            fila(&[("activo", "false"), ("id", "2"), ("pais", "PT")]),
        ]
    }

    fn tipos(lote: &RecordBatch) -> BTreeMap<String, String> {
        lote.schema()
            .fields()
            .iter()
            .map(|f| (f.name().clone(), f.data_type().to_string()))
            .collect()
    }

    /// El Parquet que `iceberg` escribiría del lote, para probar la vuelta.
    fn parquet(lote: &RecordBatch) -> Vec<u8> {
        let mut out = Vec::new();
        let mut w =
            parquet::arrow::ArrowWriter::try_new(&mut out, lote.schema(), None).expect("escritor");
        w.write(lote).expect("lote");
        w.close().expect("cierra");
        out
    }

    fn escribir(esquema: &BTreeMap<String, String>, filas: &[Fila]) -> Result<Carga, String> {
        let l = lote(esquema, filas)?;
        Ok(Carga {
            bytes: parquet(&l.lote),
            tipos: tipos(&l.lote),
            sin_estrechar: l.sin_estrechar,
        })
    }

    struct Carga {
        bytes: Vec<u8>,
        tipos: BTreeMap<String, String>,
        sin_estrechar: BTreeMap<String, String>,
    }

    /// El lote es el mismo para la misma entrada, y su Parquet también: lo que
    /// entra decide lo que sale, sin nada del reloj ni del orden de llegada.
    #[test]
    fn dos_lotes_de_las_mismas_filas_dan_los_mismos_bytes() {
        let a = escribir(&esquema(), &filas()).expect("escribe");
        let b = escribir(&esquema(), &filas()).expect("escribe");
        assert_eq!(a.bytes, b.bytes, "el Parquet no es determinista");
        assert_eq!(&a.bytes[..4], b"PAR1", "y es Parquet de verdad");
        assert!(a.sin_estrechar.is_empty(), "{:?}", a.sin_estrechar);
    }

    /// La tabla de 0032 §1, en el lote que sale: cada escalar en su físico.
    #[test]
    fn la_copia_lleva_el_tipo_de_la_tabla() {
        let c = escribir(&esquema(), &filas()).expect("escribe");
        let t = c.tipos;
        assert_eq!(t["activo"], "Boolean");
        assert_eq!(t["id"], "Int64");
        assert_eq!(t["pais"], "Utf8");
        assert_eq!(t["total"], "Decimal128(38, 18)");
        assert_eq!(t["peso"], "Float64");
        assert_eq!(t["dia"], "Date32");
        assert_eq!(t["hora"], "Time64(µs)");
        assert_eq!(t["cuando"], "Timestamp(µs)");
        assert_eq!(t["visto"], "Timestamp(µs, \"+00:00\")");
        assert_eq!(t["importe"], "Decimal128(38, 2)");
    }

    /// El hueco es un nulo, no una cadena vacía: la fila que no trae la columna
    /// y la que la trae vacía no son la misma fila.
    #[test]
    fn una_columna_ausente_es_nula_y_no_la_cadena_vacia() {
        let sin = escribir(&esquema(), &filas()).expect("escribe");
        let mut con = filas();
        con[1].insert("pais".to_string(), String::new());
        assert_ne!(
            sin.bytes,
            escribir(&esquema(), &con).expect("escribe").bytes
        );
    }

    /// **No se inventa una conversión, y no se rompe la copia.** Un `Integer`
    /// que llega como texto es un defecto de quien lo produjo: la columna se
    /// queda como texto, entera, y el informe dice cuántos valores y cuál.
    #[test]
    fn un_valor_que_no_es_del_tipo_deja_la_columna_como_texto_y_lo_dice() {
        let mut malas = filas();
        malas[0].insert("id".to_string(), "uno".to_string());
        malas[1].insert("visto".to_string(), "1.7E9".to_string());
        let c = escribir(&esquema(), &malas).expect("escribe igualmente");
        let t = &c.tipos;
        assert_eq!(t["id"], "Utf8", "la columna entera, no la fila");
        assert_eq!(t["visto"], "Utf8");
        assert_eq!(t["total"], "Decimal128(38, 18)", "las demás no se tocan");
        assert_eq!(c.sin_estrechar.len(), 2);
        let e = &c.sin_estrechar["id"];
        assert!(e.starts_with("1 de 2 valores no son Integer"), "{e}");
        assert!(e.contains("`uno`"), "{e}");
        assert!(c.sin_estrechar["visto"].contains("DateTimeTz"));
        // Y el texto se conserva tal cual, para que el origen se pueda mirar.
        let vuelta = leer(&c.bytes).expect("lee");
        assert_eq!(vuelta[0]["id"], "uno");
        assert_eq!(vuelta[1]["visto"], "1.7E9");
    }

    /// Lo que se lee de una copia es el mismo texto que entró (en su forma
    /// canónica): fundirlo con un incremento y volver a sellar da los mismos
    /// bytes que sellar todo de golpe. Es la propiedad de la que cuelga el
    /// refresco.
    #[test]
    fn leer_devuelve_la_forma_canonica_y_resellar_da_los_mismos_bytes() {
        let c = escribir(&esquema(), &filas()).expect("escribe");
        let vuelta = leer(&c.bytes).expect("lee");
        assert_eq!(vuelta[0]["total"], "10.5", "sin los ceros de la escala");
        assert_eq!(vuelta[0]["importe"], "99.99");
        assert_eq!(vuelta[0]["dia"], "2020-02-29");
        assert_eq!(vuelta[0]["hora"], "10:00:00.5");
        assert_eq!(vuelta[0]["cuando"], "2020-02-29 10:00:00.5");
        assert_eq!(vuelta[0]["visto"], "2020-02-29 10:00:00.5+00");
        assert_eq!(vuelta[0]["peso"], "1.5");
        assert_eq!(vuelta[0]["activo"], "true");
        assert!(
            !vuelta[1].contains_key("total"),
            "el nulo sigue siendo hueco"
        );

        let otra = escribir(&esquema(), &vuelta).expect("escribe");
        assert_eq!(c.bytes, otra.bytes, "ida y vuelta cambia los bytes");

        // Y con una fusión por medio: la base leída + un incremento con el
        // mismo texto del origen = lo mismo que todo de golpe.
        let clave = vec!["id".to_string()];
        let de_golpe = escribir(&esquema(), &fundir(Vec::new(), filas(), &clave)).expect("escribe");
        let mut delta = filas();
        delta.truncate(1);
        let fundida = fundir(leer(&c.bytes).expect("lee"), delta, &clave);
        assert_eq!(
            escribir(&esquema(), &fundida).expect("escribe").bytes,
            de_golpe.bytes
        );
    }

    /// Un instante se guarda en UTC: dos textos del mismo instante con distinto
    /// desfase son el mismo valor y los mismos bytes.
    #[test]
    fn dos_desfases_del_mismo_instante_son_la_misma_copia() {
        let esq: BTreeMap<String, String> =
            [("visto".to_string(), "DateTimeTz".to_string())].into();
        let a = escribir(&esq, &[fila(&[("visto", "2024-06-01 12:00:00+00")])]).expect("escribe");
        let b =
            escribir(&esq, &[fila(&[("visto", "2024-06-01 14:00:00+02:00")])]).expect("escribe");
        assert_eq!(a.bytes, b.bytes);
        assert_eq!(
            leer(&a.bytes).expect("lee")[0]["visto"],
            "2024-06-01 12:00:00+00"
        );
    }

    /// La guarda de 0032 T4, con los valores que se midieron: `0.005` escrito
    /// en `decimal(38, 18)` y fundido con una tabla `decimal(10, 2)` salía
    /// `0.01` —y `123456789.12`, NULL— con un `Ok`. Ahora no se funde.
    #[test]
    fn un_decimal_que_pierde_cifras_no_se_funde() {
        use arrow_array::Decimal128Array;
        let viejo = Decimal128Array::from(vec![5_000_000_000_000_000i128])
            .with_precision_and_scale(38, 18)
            .unwrap();
        let lote = RecordBatch::try_new(
            Arc::new(Schema::new(vec![Field::new(
                "total",
                DataType::Decimal128(38, 18),
                true,
            )])),
            vec![Arc::new(viejo)],
        )
        .unwrap();
        let estrecho = Arc::new(Schema::new(vec![Field::new(
            "total",
            DataType::Decimal128(10, 2),
            true,
        )]));
        let e = al_esquema(&lote, &estrecho).unwrap_err();
        assert!(e.contains("rehacer"), "{e}");
        // Y ensanchar sí se funde, exacto: `19.99` de decimal(10, 2) en una tabla
        // decimal(12, 4) —dos enteras y dos decimales más— es `19.9900`.
        let estrecho_viejo = Decimal128Array::from(vec![1999i128])
            .with_precision_and_scale(10, 2)
            .unwrap();
        let lote = RecordBatch::try_new(
            Arc::new(Schema::new(vec![Field::new(
                "total",
                DataType::Decimal128(10, 2),
                true,
            )])),
            vec![Arc::new(estrecho_viejo)],
        )
        .unwrap();
        let ancho = Arc::new(Schema::new(vec![Field::new(
            "total",
            DataType::Decimal128(12, 4),
            true,
        )]));
        let r = al_esquema(&lote, &ancho).unwrap();
        let d = r
            .column(0)
            .as_any()
            .downcast_ref::<Decimal128Array>()
            .unwrap();
        assert_eq!(d.value_as_string(0), "19.9900");
    }

    /// ORE 0072 F1·0/F1·1 · Anexar geometría: llega `Binary` (DuckDB) a una tabla
    /// que la guarda `LargeBinary`, y pasa; la marca geo sigue con la columna
    /// (si no, se escribía como un binario sin tipo); y la cobertura no entra
    /// en una tabla que no la tenía (sus ficheros viejos no la llevan).
    #[test]
    fn anexar_geometria_binary_a_large_binary() {
        use arrow_array::{BinaryArray, Int64Array};
        use iceberg::spec::{NestedField, PrimitiveType, Schema as IcebergSchema, Type};
        let tabla = IcebergSchema::builder()
            .with_fields(vec![
                Arc::new(NestedField::optional(
                    1,
                    "id",
                    Type::Primitive(PrimitiveType::Long),
                )),
                Arc::new(NestedField::optional(
                    2,
                    "geom",
                    Type::Primitive(PrimitiveType::Binary),
                )),
            ])
            .build()
            .unwrap();
        let mut w = vec![1u8, 1, 0, 0, 0];
        w.extend_from_slice(&1.0f64.to_le_bytes());
        w.extend_from_slice(&2.0f64.to_le_bytes());
        let marca = std::collections::HashMap::from([
            ("ARROW:extension:name".to_string(), GEOARROW_WKB.to_string()),
            (
                "ARROW:extension:metadata".to_string(),
                r#"{"crs":"OGC:CRS84"}"#.to_string(),
            ),
        ]);
        let lote = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("id", DataType::Int64, true),
                Field::new("geom", DataType::Binary, true).with_metadata(marca),
            ])),
            vec![
                Arc::new(Int64Array::from(vec![1])),
                Arc::new(BinaryArray::from(vec![Some(w.as_slice())])),
            ],
        )
        .unwrap();
        let lote = crate::lago_geo::con_cobertura(lote).unwrap();
        assert!(lote.column_by_name("geom_bbox").is_some());
        let c = conformar(&lote, &tabla).unwrap();
        let g = c.schema().field_with_name("geom").unwrap().clone();
        assert_eq!(g.data_type(), &DataType::LargeBinary);
        assert_eq!(
            g.metadata().get("ARROW:extension:name").map(String::as_str),
            Some(GEOARROW_WKB)
        );
        assert!(
            c.column_by_name("geom_bbox").is_none(),
            "la cobertura no entra al anexar a una tabla sin ella"
        );
    }
}
