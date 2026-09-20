//! **La carga: Parquet.**
//!
//! El [ADR 0015](../../../docs/decisions/0015-el-protocolo-del-almacen.md) la
//! eligió por dos cosas que un formato propio no da: **cualquier motor la lee**,
//! y **algún día la escribe el origen** — Snowflake y Databricks escriben
//! Parquet a un destino compatible con S3, así que el día que lo hagan cambia
//! quién produce la carga y no cambia el sobre.
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
//! El tipo de OOS sigue viajando en la cabecera del sobre, que es donde el
//! esquema es normativo; el Parquet lleva el físico, que es lo que un motor
//! lee sin abrir la cabecera.

use arrow_array::builder::{
    BooleanBuilder, Date32Builder, Decimal128Builder, Float64Builder, Int64Builder, StringBuilder,
    Time64MicrosecondBuilder, TimestampMicrosecondBuilder,
};
use arrow_array::{ArrayRef, RecordBatch};
use arrow_schema::{DataType, Field, Schema, TimeUnit};
use ore_core::tipos::{Fisico, Valor};
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;
use std::collections::BTreeMap;
use std::sync::Arc;

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
        Fisico::Instante => DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
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

/// Las filas ya leídas: cada una es columna → valor **en texto**, que es como
/// las entrega el protocolo del driver. Una columna ausente en una fila es un
/// hueco, y aquí se escribe como nulo.
pub type Fila = BTreeMap<String, String>;

/// Lo que sale de escribir: los bytes, y lo que no se pudo estrechar.
pub struct Carga {
    pub bytes: Vec<u8>,
    /// Columna → por qué se quedó como texto. Vacío es la copia que el
    /// contrato promete; lo que haya aquí va al informe tal cual.
    pub sin_estrechar: BTreeMap<String, String>,
}

/// **Escribe el Parquet.** Determinista sobre la misma entrada: mismo esquema,
/// mismas filas, mismo orden ⟹ mismos bytes. Sin eso el sobre no se podría
/// nombrar por su digest.
pub fn escribir(esquema: &BTreeMap<String, String>, filas: &[Fila]) -> Result<Carga, String> {
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

    let lote = RecordBatch::try_new(schema.clone(), columnas)
        .map_err(|e| format!("las columnas no cuadran con el esquema: {e}"))?;

    // SNAPPY y no zstd: el nivel de zstd es un parámetro más que tendría que
    // fijarse para que dos escrituras dieran los mismos bytes, y la compresión
    // de una copia no es donde se gana. SNAPPY además es Rust puro aquí.
    let props = WriterProperties::builder()
        .set_compression(Compression::SNAPPY)
        .build();
    let mut out = Vec::new();
    let mut w = ArrowWriter::try_new(&mut out, schema, Some(props))
        .map_err(|e| format!("no se pudo abrir el escritor de Parquet: {e}"))?;
    w.write(&lote)
        .map_err(|e| format!("no se pudo escribir el lote: {e}"))?;
    w.close()
        .map_err(|e| format!("no se pudo cerrar el Parquet: {e}"))?;
    Ok(Carga {
        bytes: out,
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
            TimestampMicrosecondBuilder::new().with_timezone("UTC"),
            Valor::Instante
        ),
    }
}

/// **Volver a leer el Parquet.** La mitad que faltaba del formato.
///
/// Se escribía desde M0 y no lo leía nadie, y eso estaba bien mientras una copia
/// solo se poblara. En cuanto se **refresca**, hace falta: fundir un incremento
/// con lo que ya había exige abrir lo que ya había.
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

/// **La fusión: lo que había, más el incremento, por clave.**
///
/// Es la operación que convierte *«leer menos»* en *«leer menos y seguir estando
/// entera»*, y las dos hacen falta: un refresco que lee 10 filas y sella una
/// copia de 10 no es más rápido, es **incorrecto**.
///
/// # Por qué esto no es el circuito Δ
///
/// Se miró si `ore-maintain` servía, y no: [ADR 0013](../../../docs/decisions/0013-el-protocolo-del-mantenedor.md)
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

    fn tipos(parquet: &[u8]) -> BTreeMap<String, String> {
        use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
        let b = bytes::Bytes::copy_from_slice(parquet);
        let r = ParquetRecordBatchReaderBuilder::try_new(b).expect("parquet");
        r.schema()
            .fields()
            .iter()
            .map(|f| (f.name().clone(), f.data_type().to_string()))
            .collect()
    }

    /// **Lo que el nombre exige.** Si dos escrituras de las mismas filas dieran
    /// bytes distintos, cada re-materialización sería otra copia y el almacén
    /// crecería sin que nada cambiase.
    #[test]
    fn dos_escrituras_de_las_mismas_filas_dan_los_mismos_bytes() {
        let a = escribir(&esquema(), &filas()).expect("escribe");
        let b = escribir(&esquema(), &filas()).expect("escribe");
        assert_eq!(a.bytes, b.bytes, "el Parquet no es determinista");
        assert_eq!(&a.bytes[..4], b"PAR1", "y es Parquet de verdad");
        assert!(a.sin_estrechar.is_empty(), "{:?}", a.sin_estrechar);
    }

    /// La tabla de 0032 §1, en el Parquet que sale: cada escalar en su físico.
    #[test]
    fn la_copia_lleva_el_tipo_de_la_tabla() {
        let c = escribir(&esquema(), &filas()).expect("escribe");
        let t = tipos(&c.bytes);
        assert_eq!(t["activo"], "Boolean");
        assert_eq!(t["id"], "Int64");
        assert_eq!(t["pais"], "Utf8");
        assert_eq!(t["total"], "Decimal128(38, 18)");
        assert_eq!(t["peso"], "Float64");
        assert_eq!(t["dia"], "Date32");
        assert_eq!(t["hora"], "Time64(Microsecond)");
        assert_eq!(t["cuando"], "Timestamp(Microsecond, None)");
        assert_eq!(t["visto"], "Timestamp(Microsecond, Some(\"UTC\"))");
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
        let t = tipos(&c.bytes);
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
}
