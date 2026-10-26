//! **El lote de Arrow de un origen.**
//!
//! El conector v2 contesta Arrow en flujo, un lote cada pocos miles de filas mientras se lee
//! (`docs/federation.md` §1.3). Cada origen convierte **su** celda nativa a un [`Valor`] de OOS (un `CqlValue`, un
//! `BsonRef`, el texto canónico de Oracle…) y decide qué error dar si no cabe; **armar el lote** es lo mismo en todos
//! y está aquí una vez.
//!
//! ```text
//!   celda nativa ──(origen)──▶ Option<Valor> ──[RecordBatchBuilder::push]──▶ builders de Arrow ──[RecordBatchBuilder::finish]──▶ RecordBatch
//! ```
//!
//! # Lo que es y lo que NO es
//!
//! Es **el lado del origen**: de los [`Valor`] que produce un conector a un `RecordBatch`. Hoy solo conoce los
//! **10 físicos escalares** ([`Fisico`], con `Binario` desde v1alpha35); no conoce `Forma` (struct, lista,
//! vector, geometría) ni `Map`, que OOS aún no tiene. La meta, por fases, es ser el único sitio que traduce un tipo de OOS a Arrow
//! (ADR 0075, «`ore-arrow`…»; ficha 0075 §11).
//!
//! No es:
//! - el Arrow de los SDK ni del agente del puesto (`over()`, `sql()`, `arrow()`, `write()`);
//! - el flujo IPC (ADR 0043): cada lector escribe el suyo;
//! - el contrato del lago (`ore-store::carga`: `al_contrato`, `conformar`, `huella`), que valida un lote que ya es
//!   Arrow y tiene su propia regla de error;
//! - una dependencia de `ore`: Arrow no puede entrar en el cierre del compilador (`dependencias.rs`).
//!
//! Hoy `ore-store::carga` y `ore-read-postgres` hacen por su cuenta el mismo `Fisico` → Arrow.
//!
//! # Lo que garantiza
//!
//! - **Un físico por columna** ([`Fisico`]): `Entero` es `int64`, `Real` `float64`, `Decimal` `decimal128(p, s)`,
//!   `Instante` `timestamp[us, UTC]`, `FechaHora` `timestamp[us]`, `Fecha` `date32`, `Hora` `time64[us]`, `Binario` `binary`; lo demás
//!   es `utf8`. El tipo de OOS viaja en los metadatos del campo (`ore:tipo`).
//! - **Una fila entra entera o no entra**: [`RecordBatchBuilder::push`] comprueba todos los valores antes de escribir ninguno, así
//!   que un valor de otro físico no deja columnas desiguales (un origen que fallaba a media fila dejaba el lote
//!   en curso a medias, y era quien lo recibía el que tenía que saberlo).
//! - **Nada se convierte aquí**: un valor que no cabe en su tipo es del origen, que es quien sabe de qué columna y de
//!   qué fila viene (0032 §4); este crate solo ve `Valor`s ya hechos.

use arrow_array::builder::{
    BinaryBuilder, BooleanBuilder, Date32Builder, Decimal128Builder, Float64Builder, Int64Builder, StringBuilder,
    Time64MicrosecondBuilder, TimestampMicrosecondBuilder,
};
use arrow_array::{ArrayRef, RecordBatch};
use arrow_schema::{DataType, Field, Schema, SchemaRef, TimeUnit};
use ore_base::tipos::{Fisico, Valor};
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

/// Una columna del lote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Columna {
    /// El nombre que sale en el lote (0008: la propiedad, no la columna física).
    pub nombre: String,
    pub fisico: Fisico,
    /// El tipo de OOS tal como lo escribe el catálogo (`Decimal<12, 2>`, `Opaque`…): va en los metadatos.
    pub oos: String,
}

impl Columna {
    pub fn new(nombre: impl Into<String>, fisico: Fisico, oos: impl Into<String>) -> Columna {
        Columna {
            nombre: nombre.into(),
            fisico,
            oos: oos.into(),
        }
    }
}

/// Lo que falla al armar un lote: de Arrow o de quien lo llama, nunca de un valor que no cabe (eso es del origen).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorArrow(pub String);

impl fmt::Display for ErrorArrow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ErrorArrow {}

enum Col {
    Entero(Int64Builder),
    Real(Float64Builder),
    Decimal(Decimal128Builder),
    Logico(BooleanBuilder),
    Instante(TimestampMicrosecondBuilder),
    Fecha(Date32Builder),
    FechaHora(TimestampMicrosecondBuilder),
    Hora(Time64MicrosecondBuilder),
    Texto(StringBuilder),
    Binario(BinaryBuilder),
}

impl Col {
    /// ¿Cabe este valor (o el nulo) en esta columna?
    fn admite(&self, v: &Option<Valor>) -> bool {
        matches!(
            (self, v),
            (_, None)
                | (Col::Entero(_), Some(Valor::Entero(_)))
                | (Col::Real(_), Some(Valor::Real(_)))
                | (Col::Decimal(_), Some(Valor::Decimal(_)))
                | (Col::Logico(_), Some(Valor::Logico(_)))
                | (Col::Instante(_), Some(Valor::Instante(_)))
                | (Col::Fecha(_), Some(Valor::Fecha(_)))
                | (Col::FechaHora(_), Some(Valor::FechaHora(_)))
                | (Col::Hora(_), Some(Valor::Hora(_)))
                | (Col::Texto(_), Some(Valor::Texto(_)))
                | (Col::Binario(_), Some(Valor::Bytes(_)))
        )
    }

    /// Añade un valor que [`Col::admite`] ya aprobó.
    fn escribir(&mut self, v: &Option<Valor>) {
        match (self, v) {
            (Col::Entero(b), Some(Valor::Entero(x))) => b.append_value(*x),
            (Col::Real(b), Some(Valor::Real(x))) => b.append_value(*x),
            (Col::Decimal(b), Some(Valor::Decimal(x))) => b.append_value(*x),
            (Col::Logico(b), Some(Valor::Logico(x))) => b.append_value(*x),
            (Col::Instante(b), Some(Valor::Instante(x))) => b.append_value(*x),
            (Col::Fecha(b), Some(Valor::Fecha(x))) => b.append_value(*x),
            (Col::FechaHora(b), Some(Valor::FechaHora(x))) => b.append_value(*x),
            (Col::Hora(b), Some(Valor::Hora(x))) => b.append_value(*x),
            (Col::Texto(b), Some(Valor::Texto(x))) => b.append_value(x),
            (Col::Binario(b), Some(Valor::Bytes(x))) => b.append_value(x),
            (Col::Entero(b), None) => b.append_null(),
            (Col::Real(b), None) => b.append_null(),
            (Col::Decimal(b), None) => b.append_null(),
            (Col::Logico(b), None) => b.append_null(),
            (Col::Instante(b), None) => b.append_null(),
            (Col::Fecha(b), None) => b.append_null(),
            (Col::FechaHora(b), None) => b.append_null(),
            (Col::Hora(b), None) => b.append_null(),
            (Col::Texto(b), None) => b.append_null(),
            (Col::Binario(b), None) => b.append_null(),
            _ => unreachable!("`admite` lo comprobó antes de escribir"),
        }
    }

    fn terminar(&mut self) -> ArrayRef {
        match self {
            Col::Entero(b) => Arc::new(b.finish()),
            Col::Real(b) => Arc::new(b.finish()),
            Col::Decimal(b) => Arc::new(b.finish()),
            Col::Logico(b) => Arc::new(b.finish()),
            Col::Instante(b) => Arc::new(b.finish()),
            Col::Fecha(b) => Arc::new(b.finish()),
            Col::FechaHora(b) => Arc::new(b.finish()),
            Col::Hora(b) => Arc::new(b.finish()),
            Col::Texto(b) => Arc::new(b.finish()),
            Col::Binario(b) => Arc::new(b.finish()),
        }
    }
}

/// Arma lotes de filas.
pub struct RecordBatchBuilder {
    schema: SchemaRef,
    nombres: Vec<String>,
    cols: Vec<Col>,
    /// Filas del lote en curso.
    filas: usize,
}

impl RecordBatchBuilder {
    /// Un lote con estas columnas, en este orden.
    pub fn nuevo(columnas: &[Columna]) -> Result<RecordBatchBuilder, ErrorArrow> {
        let mut cols = Vec::with_capacity(columnas.len());
        let mut campos = Vec::with_capacity(columnas.len());
        for c in columnas {
            let (dt, col) = match c.fisico {
                Fisico::Entero => (DataType::Int64, Col::Entero(Int64Builder::new())),
                Fisico::Real => (DataType::Float64, Col::Real(Float64Builder::new())),
                Fisico::Decimal { precision, escala } => (
                    DataType::Decimal128(precision, escala as i8),
                    Col::Decimal(
                        Decimal128Builder::new()
                            .with_precision_and_scale(precision, escala as i8)
                            .map_err(|e| ErrorArrow(e.to_string()))?,
                    ),
                ),
                Fisico::Logico => (DataType::Boolean, Col::Logico(BooleanBuilder::new())),
                Fisico::Instante => (
                    DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
                    Col::Instante(TimestampMicrosecondBuilder::new().with_timezone("UTC")),
                ),
                Fisico::Fecha => (DataType::Date32, Col::Fecha(Date32Builder::new())),
                Fisico::FechaHora => (
                    DataType::Timestamp(TimeUnit::Microsecond, None),
                    Col::FechaHora(TimestampMicrosecondBuilder::new()),
                ),
                Fisico::Hora => (
                    DataType::Time64(TimeUnit::Microsecond),
                    Col::Hora(Time64MicrosecondBuilder::new()),
                ),
                Fisico::Texto => (DataType::Utf8, Col::Texto(StringBuilder::new())),
                Fisico::Binario => (DataType::Binary, Col::Binario(BinaryBuilder::new())),
            };
            let meta: HashMap<String, String> = [("ore:tipo".to_string(), c.oos.clone())].into();
            campos.push(Field::new(&c.nombre, dt, true).with_metadata(meta));
            cols.push(col);
        }
        Ok(RecordBatchBuilder {
            schema: Arc::new(Schema::new(campos)),
            nombres: columnas.iter().map(|c| c.nombre.clone()).collect(),
            cols,
            filas: 0,
        })
    }

    pub fn schema(&self) -> SchemaRef {
        self.schema.clone()
    }

    /// Las filas del lote en curso.
    pub fn filas(&self) -> usize {
        self.filas
    }

    /// Una fila: un valor por columna, `None` si es nulo. **Entra entera o no entra**: se comprueban todos antes de
    /// escribir ninguno, y un valor de otro físico no deja columnas desiguales.
    pub fn push(&mut self, fila: &[Option<Valor>]) -> Result<(), ErrorArrow> {
        if fila.len() != self.cols.len() {
            return Err(ErrorArrow(format!(
                "la fila trae {} valores y el lote {} columnas",
                fila.len(),
                self.cols.len()
            )));
        }
        for (i, (c, v)) in self.cols.iter().zip(fila).enumerate() {
            if !c.admite(v) {
                return Err(ErrorArrow(format!(
                    "la columna `{}` recibió un valor de otro físico",
                    self.nombres[i]
                )));
            }
        }
        for (c, v) in self.cols.iter_mut().zip(fila) {
            c.escribir(v);
        }
        self.filas += 1;
        Ok(())
    }

    /// Cierra el lote en curso y deja el constructor listo para el siguiente.
    pub fn finish(&mut self) -> Result<RecordBatch, ErrorArrow> {
        let arrays: Vec<ArrayRef> = self.cols.iter_mut().map(Col::terminar).collect();
        self.filas = 0;
        RecordBatch::try_new(self.schema.clone(), arrays).map_err(|e| ErrorArrow(e.to_string()))
    }
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use arrow_array::Array;
    use arrow_array::cast::AsArray;
    use arrow_array::types::{
        Date32Type, Decimal128Type, Float64Type, Int64Type, Time64MicrosecondType,
        TimestampMicrosecondType,
    };

    fn columnas() -> Vec<Columna> {
        vec![
            Columna::new("id", Fisico::Entero, "Integer"),
            Columna::new("real", Fisico::Real, "Float"),
            Columna::new(
                "importe",
                Fisico::Decimal {
                    precision: 12,
                    escala: 2,
                },
                "Decimal<12, 2>",
            ),
            Columna::new("ok", Fisico::Logico, "Boolean"),
            Columna::new("instante", Fisico::Instante, "DateTimeTz"),
            Columna::new("local", Fisico::FechaHora, "DateTime"),
            Columna::new("dia", Fisico::Fecha, "Date"),
            Columna::new("hora", Fisico::Hora, "Time"),
            Columna::new("nota", Fisico::Texto, "String"),
        ]
    }

    fn completa() -> Vec<Option<Valor>> {
        vec![
            Some(Valor::Entero(42)),
            Some(Valor::Real(0.5)),
            Some(Valor::Decimal(123_456_789_012)),
            Some(Valor::Logico(true)),
            Some(Valor::Instante(1_792_914_255_123_456)),
            Some(Valor::FechaHora(1_792_934_055_123_456)),
            Some(Valor::Fecha(20_751)),
            Some(Valor::Hora(3_600_000_000)),
            Some(Valor::Texto("ñandú".into())),
        ]
    }

    #[test]
    fn un_lote_con_cada_fisico_y_sus_nulos() {
        let mut l = RecordBatchBuilder::nuevo(&columnas()).unwrap();
        l.push(&completa()).unwrap();
        l.push(&[None, None, None, None, None, None, None, None, None])
            .unwrap();
        assert_eq!(l.filas(), 2);
        let lote = l.finish().unwrap();
        assert_eq!((lote.num_rows(), l.filas()), (2, 0));

        assert_eq!(lote.column(0).as_primitive::<Int64Type>().value(0), 42);
        assert_eq!(lote.column(1).as_primitive::<Float64Type>().value(0), 0.5);
        assert_eq!(
            lote.column(2).as_primitive::<Decimal128Type>().value(0),
            123_456_789_012
        );
        assert!(lote.column(3).as_boolean().value(0));
        assert_eq!(
            lote.column(4)
                .as_primitive::<TimestampMicrosecondType>()
                .value(0),
            1_792_914_255_123_456
        );
        assert_eq!(
            lote.column(5)
                .as_primitive::<TimestampMicrosecondType>()
                .value(0),
            1_792_934_055_123_456
        );
        assert_eq!(lote.column(6).as_primitive::<Date32Type>().value(0), 20_751);
        assert_eq!(
            lote.column(7)
                .as_primitive::<Time64MicrosecondType>()
                .value(0),
            3_600_000_000
        );
        assert_eq!(lote.column(8).as_string::<i32>().value(0), "ñandú");
        for i in 0..9 {
            assert!(lote.column(i).is_null(1), "columna {i}");
        }

        // El esquema: el tipo de OOS en los metadatos, la zona del instante y la ausencia de zona de la hora local.
        let sch = lote.schema();
        assert_eq!(sch.field(2).metadata()["ore:tipo"], "Decimal<12, 2>");
        assert_eq!(
            sch.field(4).data_type(),
            &DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into()))
        );
        assert_eq!(
            sch.field(5).data_type(),
            &DataType::Timestamp(TimeUnit::Microsecond, None)
        );
        assert_eq!(sch.field(6).data_type(), &DataType::Date32);
        assert_eq!(
            sch.field(7).data_type(),
            &DataType::Time64(TimeUnit::Microsecond)
        );
        assert_eq!(sch.field(8).data_type(), &DataType::Utf8);
    }

    /// Una fila entra entera o no entra: la columna 7 (la última) trae un valor de otro físico y las anteriores no
    /// se escriben.
    #[test]
    fn un_valor_de_otro_fisico_no_deja_columnas_desiguales() {
        let mut l = RecordBatchBuilder::nuevo(&columnas()).unwrap();
        l.push(&completa()).unwrap();
        let mut mala = completa();
        mala[8] = Some(Valor::Entero(1));
        let e = l.push(&mala).unwrap_err();
        assert!(e.0.contains("`nota`") && e.0.contains("otro físico"), "{e}");
        // El lote sigue sano: una fila, todas las columnas con una.
        assert_eq!(l.filas(), 1);
        let lote = l.finish().unwrap();
        assert_eq!(lote.num_rows(), 1);
        assert!(lote.columns().iter().all(|c| c.len() == 1));
    }

    #[test]
    fn una_fila_con_otro_numero_de_valores_se_rechaza() {
        let mut l = RecordBatchBuilder::nuevo(&columnas()).unwrap();
        let e = l.push(&[Some(Valor::Entero(1))]).unwrap_err();
        assert!(e.0.contains("trae 1 valores y el lote 9 columnas"), "{e}");
    }

    #[test]
    fn un_decimal_que_arrow_no_admite_se_dice_al_crear() {
        let mala = Columna::new(
            "x",
            Fisico::Decimal {
                precision: 40,
                escala: 2,
            },
            "Decimal<40, 2>",
        );
        assert!(RecordBatchBuilder::nuevo(&[mala]).is_err());
    }

    /// v1alpha35: un `Binary` es `binary` en Arrow; el nulo y el vacío son distintos.
    #[test]
    fn un_binario_distingue_nulo_y_vacio() {
        let mut l =
            RecordBatchBuilder::nuevo(&[Columna::new("firma", Fisico::Binario, "Binary")]).unwrap();
        l.push(&[Some(Valor::Bytes(vec![0xDE, 0xAD]))]).unwrap();
        l.push(&[Some(Valor::Bytes(vec![]))]).unwrap();
        l.push(&[None]).unwrap();
        assert!(l.push(&[Some(Valor::Texto("x".into()))]).is_err());
        let b = l.finish().unwrap();
        assert_eq!(b.schema().field(0).data_type(), &DataType::Binary);
        assert_eq!(b.schema().field(0).metadata()["ore:tipo"], "Binary");
        let a = b.column(0).as_binary::<i32>();
        assert_eq!(a.value(0), &[0xDE, 0xAD]);
        assert!(!a.is_null(1) && a.value(1).is_empty());
        assert!(a.is_null(2));
    }

    /// Cerrar un lote deja el constructor listo para el siguiente, sin arrastrar filas.
    #[test]
    fn un_lote_tras_otro() {
        let mut l =
            RecordBatchBuilder::nuevo(&[Columna::new("id", Fisico::Entero, "Integer")]).unwrap();
        l.push(&[Some(Valor::Entero(1))]).unwrap();
        assert_eq!(l.finish().unwrap().num_rows(), 1);
        l.push(&[Some(Valor::Entero(2))]).unwrap();
        l.push(&[None]).unwrap();
        let segundo = l.finish().unwrap();
        assert_eq!(segundo.num_rows(), 2);
        assert_eq!(segundo.column(0).as_primitive::<Int64Type>().value(0), 2);
    }
}
