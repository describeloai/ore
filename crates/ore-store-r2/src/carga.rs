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
