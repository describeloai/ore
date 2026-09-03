//! **La carga: Parquet.**
//!
//! El [ADR 0015](../../../docs/decisions/0015-el-protocolo-del-almacen.md) la
//! eligió por dos cosas que un formato propio no da: **cualquier motor la lee**,
//! y **algún día la escribe el origen** — Snowflake y Databricks escriben
//! Parquet a un destino compatible con S3, así que el día que lo hagan cambia
//! quién produce la carga y no cambia el sobre.
//!
//! # El mapa de tipos, y por qué es conservador a propósito
//!
//! Solo `Integer` y `Boolean` se estrechan. Todo lo demás viaja como cadena, tal
//! y como lo dio el origen.
//!
//! No es pereza: **estrechar un tipo es una decisión que se toma una vez y se
//! paga siempre.** Un `Decimal` metido en un `double` pierde precisión en
//! silencio, y una fecha reinterpretada a un huso ajeno es peor que una cadena
//! honesta. El tipo de OOS sí viaja —va en la cabecera del sobre, que es donde
//! el esquema es normativo— así que quien lea la copia sabe qué es cada columna
//! aunque Parquet la guarde como texto.
//!
//! Los dos que sí se estrechan son los dos donde no hay ambigüedad ninguna y el
//! ahorro es real.

use arrow_array::builder::{BooleanBuilder, Int64Builder, StringBuilder};
use arrow_array::{ArrayRef, RecordBatch};
use arrow_schema::{DataType, Field, Schema};
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Del vocabulario de escalares de OOS al de Arrow. Ver el mapa de arriba.
fn arrow_de(oos: &str) -> DataType {
    match oos {
        "Integer" => DataType::Int64,
        "Boolean" => DataType::Boolean,
        _ => DataType::Utf8,
    }
}

/// Las filas ya leídas: cada una es columna → valor **en texto**, que es como
/// las entrega el protocolo del driver. Una columna ausente en una fila es un
/// hueco, y aquí se escribe como nulo.
pub type Fila = BTreeMap<String, String>;

/// **Escribe el Parquet.** Determinista sobre la misma entrada: mismo esquema,
/// mismas filas, mismo orden ⟹ mismos bytes. Sin eso el sobre no se podría
/// nombrar por su digest.
pub fn escribir(esquema: &BTreeMap<String, String>, filas: &[Fila]) -> Result<Vec<u8>, String> {
    let campos: Vec<Field> = esquema
        .iter()
        .map(|(c, t)| Field::new(c, arrow_de(t), true))
        .collect();
    let schema = Arc::new(Schema::new(campos));

    let mut columnas: Vec<ArrayRef> = Vec::with_capacity(esquema.len());
    for (nombre, tipo) in esquema {
        columnas.push(match arrow_de(tipo) {
            DataType::Int64 => {
                let mut b = Int64Builder::new();
                for f in filas {
                    match f.get(nombre) {
                        Some(v) => match v.parse::<i64>() {
                            Ok(n) => b.append_value(n),
                            Err(_) => {
                                return Err(format!(
                                    "`{nombre}` se declaró `Integer` y llegó `{v}`: la copia no \
                                     inventa una conversión"
                                ));
                            }
                        },
                        None => b.append_null(),
                    }
                }
                Arc::new(b.finish()) as ArrayRef
            }
            DataType::Boolean => {
                let mut b = BooleanBuilder::new();
                for f in filas {
                    match f.get(nombre).map(String::as_str) {
                        Some("true") => b.append_value(true),
                        Some("false") => b.append_value(false),
                        Some(v) => {
                            return Err(format!(
                                "`{nombre}` se declaró `Boolean` y llegó `{v}`: solo `true` y \
                                 `false`"
                            ));
                        }
                        None => b.append_null(),
                    }
                }
                Arc::new(b.finish()) as ArrayRef
            }
            _ => {
                let mut b = StringBuilder::new();
                for f in filas {
                    match f.get(nombre) {
                        Some(v) => b.append_value(v),
                        None => b.append_null(),
                    }
                }
                Arc::new(b.finish()) as ArrayRef
            }
        });
    }

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
    Ok(out)
}

/// **Volver a leer el Parquet.** La mitad que faltaba del formato.
///
/// Se escribía desde M0 y no lo leía nadie, y eso estaba bien mientras una copia
/// solo se poblara. En cuanto se **refresca**, hace falta: fundir un incremento
/// con lo que ya había exige abrir lo que ya había.
///
/// Todo vuelve a texto, que es como entró. El tipo de OOS vive en la cabecera
/// del sobre —ahí es donde el esquema es normativo— así que reconstruirlo aquí
/// sería una segunda fuente para lo mismo.
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
                let v = match campo.data_type() {
                    DataType::Int64 => col
                        .as_any()
                        .downcast_ref::<arrow_array::Int64Array>()
                        .map(|a| a.value(i).to_string()),
                    DataType::Boolean => col
                        .as_any()
                        .downcast_ref::<arrow_array::BooleanArray>()
                        .map(|a| a.value(i).to_string()),
                    _ => col
                        .as_any()
                        .downcast_ref::<arrow_array::StringArray>()
                        .map(|a| a.value(i).to_string()),
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
            ("activo".to_string(), "Boolean".to_string()),
            ("id".to_string(), "Integer".to_string()),
            ("pais".to_string(), "String".to_string()),
            ("total".to_string(), "Decimal".to_string()),
        ]
        .into()
    }

    fn filas() -> Vec<Fila> {
        vec![
            [
                ("activo".to_string(), "true".to_string()),
                ("id".to_string(), "1".to_string()),
                ("pais".to_string(), "ES".to_string()),
                ("total".to_string(), "10.50".to_string()),
            ]
            .into(),
            [
                ("activo".to_string(), "false".to_string()),
                ("id".to_string(), "2".to_string()),
                ("pais".to_string(), "PT".to_string()),
            ]
            .into(),
        ]
    }

    /// **Lo que el nombre exige.** Si dos escrituras de las mismas filas dieran
    /// bytes distintos, cada re-materialización sería otra copia y el almacén
    /// crecería sin que nada cambiase.
    #[test]
    fn dos_escrituras_de_las_mismas_filas_dan_los_mismos_bytes() {
        let a = escribir(&esquema(), &filas()).expect("escribe");
        let b = escribir(&esquema(), &filas()).expect("escribe");
        assert_eq!(a, b, "el Parquet no es determinista");
        assert_eq!(&a[..4], b"PAR1", "y es Parquet de verdad");
    }

    /// El hueco es un nulo, no una cadena vacía: la fila que no trae la columna
    /// y la que la trae vacía no son la misma fila.
    #[test]
    fn una_columna_ausente_es_nula_y_no_la_cadena_vacia() {
        let sin = escribir(&esquema(), &filas()).expect("escribe");
        let mut con = filas();
        con[1].insert("total".to_string(), String::new());
        assert_ne!(sin, escribir(&esquema(), &con).expect("escribe"));
    }

    /// **No se inventa una conversión.** Un `Integer` que llega como texto es un
    /// defecto de quien lo produjo, y callarlo escribiría una copia que dice ser
    /// una cosa y es otra.
    #[test]
    fn un_valor_que_no_es_del_tipo_declarado_se_rechaza_y_se_dice_cual() {
        let mut malas = filas();
        malas[0].insert("id".to_string(), "uno".to_string());
        let e = escribir(&esquema(), &malas).expect_err("tiene que negarse");
        assert!(e.contains("`id`"), "{e}");
        assert!(e.contains("Integer"), "{e}");
    }
}
