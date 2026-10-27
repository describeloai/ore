//! **La huella de lo que no es binario no cambia** (v1alpha35).
//!
//! `huella` ganó la rama de `Binary` y `LargeBinary` (antes caían en «su nombre de tipo»). Esta tabla —un
//! valor y un nulo de cada uno de los nueve físicos de antes— fija su huella: si una refactorización la
//! mueve, el digest de las copias ya escritas se movería con ella (0031 §11 ④, la clave de operación).
//! El valor se midió el 2026-10-26 con el `huella` de antes de añadir `Binary`, y con el de después.

use arrow_array::{
    ArrayRef, BooleanArray, Date32Array, Decimal128Array, Float64Array, Int64Array, RecordBatch,
    StringArray, Time64MicrosecondArray, TimestampMicrosecondArray,
};
use arrow_schema::{DataType, Field, Schema, TimeUnit};
use ore_store::carga::huella;
use std::sync::Arc;

#[test]
fn la_huella_de_los_nueve_fisicos_de_antes_no_se_mueve() {
    let columnas: Vec<(Field, ArrayRef)> = vec![
        (
            Field::new("t", DataType::Utf8, true),
            Arc::new(StringArray::from(vec![Some("hola"), None])),
        ),
        (
            Field::new("e", DataType::Int64, true),
            Arc::new(Int64Array::from(vec![Some(42), None])),
        ),
        (
            Field::new("r", DataType::Float64, true),
            Arc::new(Float64Array::from(vec![Some(1.5), None])),
        ),
        (
            Field::new("l", DataType::Boolean, true),
            Arc::new(BooleanArray::from(vec![Some(true), None])),
        ),
        (
            Field::new("d", DataType::Decimal128(10, 2), true),
            Arc::new(
                Decimal128Array::from(vec![Some(12345), None])
                    .with_precision_and_scale(10, 2)
                    .unwrap(),
            ),
        ),
        (
            Field::new("f", DataType::Date32, true),
            Arc::new(Date32Array::from(vec![Some(20_000), None])),
        ),
        (
            Field::new("h", DataType::Time64(TimeUnit::Microsecond), true),
            Arc::new(Time64MicrosecondArray::from(vec![
                Some(3_600_000_000),
                None,
            ])),
        ),
        (
            Field::new("fh", DataType::Timestamp(TimeUnit::Microsecond, None), true),
            Arc::new(TimestampMicrosecondArray::from(vec![
                Some(1_700_000_000_000_000),
                None,
            ])),
        ),
        (
            Field::new(
                "i",
                DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
                true,
            ),
            Arc::new(
                TimestampMicrosecondArray::from(vec![Some(1_700_000_000_000_000), None])
                    .with_timezone("UTC"),
            ),
        ),
    ];
    let esquema = Arc::new(Schema::new(
        columnas.iter().map(|(f, _)| f.clone()).collect::<Vec<_>>(),
    ));
    let lote =
        RecordBatch::try_new(esquema, columnas.into_iter().map(|(_, c)| c).collect()).unwrap();
    assert_eq!(
        huella(&[lote]),
        "b2e966a21aee02042147449f926457f6fd0c04915eed8991fb941f81ff718a4d"
    );
}
