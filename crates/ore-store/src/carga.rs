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
    BooleanBuilder, Date32Builder, Decimal128Builder, Float64Builder, Int64Builder, StringBuilder,
    Time64MicrosecondBuilder, TimestampMicrosecondBuilder,
};
use arrow_array::{ArrayRef, RecordBatch};
use arrow_schema::{DataType, Field, Schema, TimeUnit};
use ore_core::tipos::{Fisico, Valor};
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
    ore_core::types::parse_type(oos)
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
    }
}

/// El `DataType` de Arrow de un tipo de OOS tal como la cabecera lo declara:
/// lo que `lote` le da a cada columna, para que `copiar` (en Arrow, sin pasar
/// por el texto) escriba exactamente el mismo esquema que `sellar`.
pub fn arrow_del_oos(oos: &str) -> DataType {
    arrow_de(&fisico_de(oos))
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
        let destino = match campo.data_type() {
            DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => DataType::Utf8,
            DataType::Int8
            | DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::UInt8
            | DataType::UInt16
            | DataType::UInt32 => DataType::Int64,
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
                     texto, entero, real, lógico, decimal, fecha, hora, fecha-hora o instante"
                ));
            }
        };
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
    use arrow_array::cast::AsArray;
    use arrow_array::types::{
        Date32Type, Decimal128Type, Float64Type, Int64Type, Time64MicrosecondType,
        TimestampMicrosecondType,
    };
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
        let fisicos: Vec<Fisico> = esquema
            .fields()
            .iter()
            .map(|c| {
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
                let col = lote.column(c);
                // Un nulo **no se convierte en cadena vacía**: la fila que no
                // traía la columna y la que la traía vacía no son la misma, y
                // confundirlas aquí las fundiría mal en el paso siguiente.
                if col.is_null(i) {
                    continue;
                }
                let fisico = &fisicos[c];
                let v = match fisico {
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
        campos.push(Field::new(f.name(), destino.clone(), true));
        columnas.push(col);
    }
    for (f, c) in lote.schema().fields().iter().zip(lote.columns()) {
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
                format!("el contrato declara `{}` y el flujo no la trae", campo.name())
            })?;
            if col.data_type() == campo.data_type() {
                return Ok(col.clone());
            }
            let convertir = |c: &ArrayRef, a: &DataType| {
                arrow_cast::cast_with_options(c, a, &estricto).map_err(|e| {
                    format!(
                        "la columna `{}` llega como `{}` y no convierte a `{}` (lo que su contrato                          declara): {e}",
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
                        "la columna `{}` llega como `{}` y un valor no cabe en `{}` sin perder                          cifras: el contrato es más estrecho que el origen",
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
}
