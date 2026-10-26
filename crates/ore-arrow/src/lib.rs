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
    BinaryBuilder, BooleanBuilder, Date32Builder, Decimal128Builder, Float64Builder, Int64Builder,
    StringBuilder, Time64MicrosecondBuilder, TimestampMicrosecondBuilder,
};
use arrow_array::{ArrayRef, RecordBatch};
use arrow_schema::{DataType, Field, Schema, SchemaRef, TimeUnit};
use ore_base::tipos::{Fisico, Forma, Valor};
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

mod columna;
mod forma;
pub use columna::{columna_de_bytes, columna_de_textos, columna_de_valores};
pub use forma::{
    Dato, ELEMENTO, GEOARROW_WKB, META_TIPO, UTC, campo, campo_del_oos, tipo_arrow, tipo_del_oos,
    tipo_fisico,
};

/// Una columna del lote.
#[derive(Debug, Clone, PartialEq)]
pub struct Columna {
    /// El nombre que sale en el lote (0008: la propiedad, no la columna física).
    pub nombre: String,
    pub fisico: Fisico,
    /// El tipo de OOS tal como lo escribe el catálogo (`Decimal<12, 2>`, `Opaque`…): va en los metadatos.
    pub oos: String,
    /// La forma, si la columna es compuesta (struct, lista, vector, geometría): `fisico` no la dice entera.
    pub forma: Option<Forma>,
}

impl Columna {
    pub fn new(nombre: impl Into<String>, fisico: Fisico, oos: impl Into<String>) -> Columna {
        Columna {
            nombre: nombre.into(),
            fisico,
            oos: oos.into(),
            forma: None,
        }
    }

    /// Una columna de cualquier forma. Una forma escalar es la de [`Columna::new`]; lo compuesto se arma con
    /// [`RecordBatchBuilder::push_datos`].
    pub fn de_forma(nombre: impl Into<String>, forma: Forma, oos: impl Into<String>) -> Columna {
        match forma {
            Forma::Escalar(f) => Columna::new(nombre, f, oos),
            forma => Columna {
                nombre: nombre.into(),
                fisico: Fisico::Texto,
                oos: oos.into(),
                forma: Some(forma),
            },
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
    /// Una columna compuesta: sus datos se recogen y se arman al cerrar el lote ([`forma`]).
    Forma(Forma, Vec<Option<Dato>>),
}

impl Col {
    /// ¿Cabe este valor (o el nulo) en esta columna?
    pub(crate) fn admite(&self, v: Option<&Valor>) -> bool {
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
    pub(crate) fn escribir(&mut self, v: Option<&Valor>) {
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
            (Col::Forma(_, d), None) => d.push(None),
            _ => unreachable!("`admite` lo comprobó antes de escribir"),
        }
    }

    pub(crate) fn terminar(&mut self) -> ArrayRef {
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
            Col::Forma(f, d) => {
                let datos = std::mem::take(d);
                forma::construir(f, &datos).expect("`admite` lo comprobó antes de escribir")
            }
        }
    }
}

/// El tipo de Arrow y el constructor de un físico escalar.
pub(crate) fn col_de(f: &Fisico) -> Result<(DataType, Col), ErrorArrow> {
    col_de_cap(f, 0)
}

/// Lo mismo, con sitio para `n` filas ya reservado.
pub(crate) fn col_de_cap(f: &Fisico, n: usize) -> Result<(DataType, Col), ErrorArrow> {
    Ok(match f {
        Fisico::Entero => (DataType::Int64, Col::Entero(Int64Builder::with_capacity(n))),
        Fisico::Real => (
            DataType::Float64,
            Col::Real(Float64Builder::with_capacity(n)),
        ),
        Fisico::Decimal { precision, escala } => (
            DataType::Decimal128(*precision, *escala as i8),
            Col::Decimal(
                Decimal128Builder::with_capacity(n)
                    .with_precision_and_scale(*precision, *escala as i8)
                    .map_err(|e| ErrorArrow(e.to_string()))?,
            ),
        ),
        Fisico::Logico => (
            DataType::Boolean,
            Col::Logico(BooleanBuilder::with_capacity(n)),
        ),
        Fisico::Instante => (
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
            Col::Instante(TimestampMicrosecondBuilder::with_capacity(n).with_timezone("UTC")),
        ),
        Fisico::Fecha => (
            DataType::Date32,
            Col::Fecha(Date32Builder::with_capacity(n)),
        ),
        Fisico::FechaHora => (
            DataType::Timestamp(TimeUnit::Microsecond, None),
            Col::FechaHora(TimestampMicrosecondBuilder::with_capacity(n)),
        ),
        Fisico::Hora => (
            DataType::Time64(TimeUnit::Microsecond),
            Col::Hora(Time64MicrosecondBuilder::with_capacity(n)),
        ),
        Fisico::Texto => (
            DataType::Utf8,
            Col::Texto(StringBuilder::with_capacity(n, n * 16)),
        ),
        Fisico::Binario => (
            DataType::Binary,
            Col::Binario(BinaryBuilder::with_capacity(n, n * 16)),
        ),
    })
}

impl Col {
    fn longitud(&self) -> usize {
        use arrow_array::builder::ArrayBuilder;
        match self {
            Col::Entero(b) => b.len(),
            Col::Real(b) => b.len(),
            Col::Decimal(b) => b.len(),
            Col::Logico(b) => b.len(),
            Col::Instante(b) | Col::FechaHora(b) => b.len(),
            Col::Fecha(b) => b.len(),
            Col::Hora(b) => b.len(),
            Col::Texto(b) => b.len(),
            Col::Binario(b) => b.len(),
            Col::Forma(_, d) => d.len(),
        }
    }
}

/// **La escritura tipada de una columna**: una llamada por celda, sin `Valor` ni reservas por celda. El que llama
/// escribe desde su propio búfer (`&str`, `&[u8]`); la variante de la columna se resuelve en cada llamada con un
/// solo `match`. Un valor de otro físico es un error y no escribe nada.
pub struct Escritor<'a>(&'a mut Col);

impl Escritor<'_> {
    fn otro<T>(&self, que: &str) -> Result<T, ErrorArrow> {
        Err(ErrorArrow(format!(
            "un {que} en una columna de otro físico"
        )))
    }

    pub fn nulo(&mut self) {
        self.0.escribir(None)
    }

    /// Un [`Valor`] ya hecho, en la columna: el atajo de quien analiza texto y ya tiene uno.
    pub fn valor(&mut self, v: &Valor) -> Result<(), ErrorArrow> {
        if self.0.admite(Some(v)) {
            self.0.escribir(Some(v));
            Ok(())
        } else {
            self.otro("valor")
        }
    }

    pub fn entero(&mut self, x: i64) -> Result<(), ErrorArrow> {
        match self.0 {
            Col::Entero(b) => {
                b.append_value(x);
                Ok(())
            }
            _ => self.otro("entero"),
        }
    }

    pub fn real(&mut self, x: f64) -> Result<(), ErrorArrow> {
        match self.0 {
            Col::Real(b) => {
                b.append_value(x);
                Ok(())
            }
            _ => self.otro("real"),
        }
    }

    pub fn logico(&mut self, x: bool) -> Result<(), ErrorArrow> {
        match self.0 {
            Col::Logico(b) => {
                b.append_value(x);
                Ok(())
            }
            _ => self.otro("lógico"),
        }
    }

    /// Un decimal **sin escala**: el entero que, dividido por 10^escala de la columna, es el valor.
    pub fn decimal(&mut self, x: i128) -> Result<(), ErrorArrow> {
        match self.0 {
            Col::Decimal(b) => {
                b.append_value(x);
                Ok(())
            }
            _ => self.otro("decimal"),
        }
    }

    pub fn fecha(&mut self, dias: i32) -> Result<(), ErrorArrow> {
        match self.0 {
            Col::Fecha(b) => {
                b.append_value(dias);
                Ok(())
            }
            _ => self.otro("fecha"),
        }
    }

    pub fn hora(&mut self, micros: i64) -> Result<(), ErrorArrow> {
        match self.0 {
            Col::Hora(b) => {
                b.append_value(micros);
                Ok(())
            }
            _ => self.otro("hora"),
        }
    }

    /// Una hora de pared (`FechaHora`) o un instante (`Instante`), en microsegundos.
    pub fn micros(&mut self, x: i64) -> Result<(), ErrorArrow> {
        match self.0 {
            Col::Instante(b) | Col::FechaHora(b) => {
                b.append_value(x);
                Ok(())
            }
            _ => self.otro("instante"),
        }
    }

    pub fn texto(&mut self, x: &str) -> Result<(), ErrorArrow> {
        match self.0 {
            Col::Texto(b) => {
                b.append_value(x);
                Ok(())
            }
            _ => self.otro("texto"),
        }
    }

    pub fn bytes(&mut self, x: &[u8]) -> Result<(), ErrorArrow> {
        match self.0 {
            Col::Binario(b) => {
                b.append_value(x);
                Ok(())
            }
            _ => self.otro("binario"),
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
    /// Las filas con las que se reserva cada columna escalar, al crear y tras cada lote: un `finish()` de Arrow
    /// devuelve los búfers y deja los builders vacíos, así que sin esto solo el primer lote tendría sitio.
    capacidad: usize,
    /// El físico de cada columna escalar (`None` en las compuestas): para volver a reservar.
    fisicos: Vec<Option<Fisico>>,
}

impl RecordBatchBuilder {
    /// Un lote con estas columnas, en este orden.
    pub fn nuevo(columnas: &[Columna]) -> Result<RecordBatchBuilder, ErrorArrow> {
        let mut cols = Vec::with_capacity(columnas.len());
        let mut campos = Vec::with_capacity(columnas.len());
        for c in columnas {
            let (dt, col, mut meta) = match &c.forma {
                None => {
                    let (dt, col) = col_de(&c.fisico)?;
                    (dt, col, HashMap::new())
                }
                Some(f) => {
                    let campo = campo(&c.nombre, f);
                    (
                        campo.data_type().clone(),
                        Col::Forma(f.clone(), Vec::new()),
                        campo.metadata().clone(),
                    )
                }
            };
            meta.insert("ore:tipo".to_string(), c.oos.clone());
            campos.push(Field::new(&c.nombre, dt, true).with_metadata(meta));
            cols.push(col);
        }
        Ok(RecordBatchBuilder {
            schema: Arc::new(Schema::new(campos)),
            nombres: columnas.iter().map(|c| c.nombre.clone()).collect(),
            cols,
            filas: 0,
            capacidad: 0,
            fisicos: columnas
                .iter()
                .map(|c| c.forma.is_none().then_some(c.fisico))
                .collect(),
        })
    }

    /// Un lote con sitio para `filas` ya reservado en cada columna escalar: sin redimensionados.
    pub fn con_capacidad(
        columnas: &[Columna],
        filas: usize,
    ) -> Result<RecordBatchBuilder, ErrorArrow> {
        let mut b = RecordBatchBuilder::nuevo(columnas)?;
        b.capacidad = filas;
        b.reservar()?;
        Ok(b)
    }

    /// Deja cada columna escalar con sitio para `capacidad` filas.
    fn reservar(&mut self) -> Result<(), ErrorArrow> {
        if self.capacidad == 0 {
            return Ok(());
        }
        for (col, f) in self.cols.iter_mut().zip(&self.fisicos) {
            if let Some(f) = f {
                *col = col_de_cap(f, self.capacidad)?.1;
            }
        }
        Ok(())
    }

    /// La escritura tipada de la columna `i` ([`Escritor`]). Quien la usa escribe **una celda por columna y fila**; que
    /// las columnas queden iguales lo comprueba [`RecordBatchBuilder::finish`], que si no devuelve un error y descarta
    /// el lote (una celda escrita no se deshace).
    pub fn col(&mut self, i: usize) -> Escritor<'_> {
        Escritor(&mut self.cols[i])
    }

    /// Cuenta una fila escrita con [`RecordBatchBuilder::col`].
    pub fn fila_escrita(&mut self) {
        self.filas += 1;
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
            if !c.admite(v.as_ref()) {
                return Err(ErrorArrow(format!(
                    "la columna `{}` recibió un valor de otro físico",
                    self.nombres[i]
                )));
            }
        }
        for (c, v) in self.cols.iter_mut().zip(fila) {
            c.escribir(v.as_ref());
        }
        self.filas += 1;
        Ok(())
    }

    /// Una fila con columnas de cualquier forma. **Entra entera o no entra**, como [`RecordBatchBuilder::push`]; el
    /// error dice la columna y, dentro de lo compuesto, la ruta (`a.b[2]`).
    pub fn push_datos(&mut self, fila: Vec<Option<Dato>>) -> Result<(), ErrorArrow> {
        if fila.len() != self.cols.len() {
            return Err(ErrorArrow(format!(
                "la fila trae {} valores y el lote {} columnas",
                fila.len(),
                self.cols.len()
            )));
        }
        for (i, (c, d)) in self.cols.iter().zip(&fila).enumerate() {
            let cabe = match (c, d) {
                (_, None) => Ok(()),
                (Col::Forma(f, _), Some(d)) => forma::admite(f, d),
                (c, Some(Dato::Escalar(v))) if c.admite(Some(v)) => Ok(()),
                _ => Err("un dato que no es de la forma de la columna".to_string()),
            };
            if let Err(porque) = cabe {
                return Err(ErrorArrow(format!(
                    "la columna `{}`: {porque}",
                    self.nombres[i]
                )));
            }
        }
        for (c, d) in self.cols.iter_mut().zip(fila) {
            match (c, d) {
                (c, None) => c.escribir(None),
                (Col::Forma(_, datos), d) => datos.push(d),
                (c, Some(Dato::Escalar(v))) => c.escribir(Some(&v)),
                _ => unreachable!("se comprobó antes de escribir"),
            }
        }
        self.filas += 1;
        Ok(())
    }

    /// Cierra el lote en curso y deja el constructor listo para el siguiente.
    pub fn finish(&mut self) -> Result<RecordBatch, ErrorArrow> {
        if let Some(c) = self.cols.iter().position(|c| c.longitud() != self.filas) {
            let n = self.cols[c].longitud();
            let (filas, nombre) = (self.filas, self.nombres[c].clone());
            for col in self.cols.iter_mut() {
                col.terminar();
            }
            self.filas = 0;
            self.reservar()?;
            return Err(ErrorArrow(format!(
                "la columna `{nombre}` tiene {n} valores y el lote {filas} filas: se escribió una celda de más o de menos"
            )));
        }
        let arrays: Vec<ArrayRef> = self.cols.iter_mut().map(Col::terminar).collect();
        self.filas = 0;
        self.reservar()?;
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

    // ── Lo compuesto (ADR 0075, fase 1) ──

    fn lote_de(c: Vec<Columna>, filas: Vec<Vec<Option<Dato>>>) -> RecordBatch {
        let mut b = RecordBatchBuilder::nuevo(&c).unwrap();
        for f in filas {
            b.push_datos(f).unwrap();
        }
        b.finish().unwrap()
    }

    fn real(x: f64) -> Option<Dato> {
        Some(Dato::Escalar(Valor::Real(x)))
    }

    #[test]
    fn una_lista_con_nulos_vacios_y_elementos_nulos() {
        let f = Forma::Lista(Box::new(Forma::Escalar(Fisico::Entero)));
        let e = |x: i64| Some(Dato::Escalar(Valor::Entero(x)));
        let lote = lote_de(
            vec![Columna::de_forma("xs", f, "list<Integer>")],
            vec![
                vec![Some(Dato::Lista(vec![e(1), None, e(3)]))],
                vec![None],
                vec![Some(Dato::Lista(vec![]))],
            ],
        );
        assert_eq!(lote.num_rows(), 3);
        let xs = lote.column(0).as_list::<i32>();
        assert!(xs.is_null(1));
        assert_eq!(xs.value(0).len(), 3);
        assert_eq!(xs.value(2).len(), 0, "vacía no es nula");
        let v0 = xs.value(0);
        let v0 = v0.as_primitive::<Int64Type>();
        assert_eq!((v0.value(0), v0.is_null(1), v0.value(2)), (1, true, 3));
        assert_eq!(
            lote.schema().field(0).metadata()["ore:tipo"],
            "list<Integer>"
        );
    }

    #[test]
    fn un_vector_es_una_lista_de_float32() {
        let lote = lote_de(
            vec![Columna::de_forma("v", Forma::Vector(3), "Vector<3>")],
            vec![
                vec![Some(Dato::Lista(vec![real(0.5), real(1.0), real(-2.0)]))],
                vec![None],
            ],
        );
        assert_eq!(
            lote.schema().field(0).data_type(),
            &DataType::List(Arc::new(Field::new(ELEMENTO, DataType::Float32, true)))
        );
        let v = lote.column(0).as_list::<i32>().value(0);
        assert_eq!(
            v.as_primitive::<arrow_array::types::Float32Type>()
                .values()
                .to_vec(),
            vec![0.5, 1.0, -2.0]
        );
    }

    #[test]
    fn un_struct_con_una_lista_y_otro_struct_dentro() {
        let interno = Forma::Struct(vec![("n".into(), Forma::Escalar(Fisico::Entero))]);
        let f = Forma::Struct(vec![
            ("nombre".into(), Forma::Escalar(Fisico::Texto)),
            (
                "tags".into(),
                Forma::Lista(Box::new(Forma::Escalar(Fisico::Texto))),
            ),
            ("dentro".into(), interno),
        ]);
        let t = |s: &str| Some(Dato::Escalar(Valor::Texto(s.into())));
        let lote = lote_de(
            vec![Columna::de_forma("p", f, "Struct<…>")],
            vec![
                vec![Some(Dato::Struct(vec![
                    t("a"),
                    Some(Dato::Lista(vec![t("x"), t("y")])),
                    Some(Dato::Struct(vec![Some(Dato::Escalar(Valor::Entero(7)))])),
                ]))],
                vec![Some(Dato::Struct(vec![None, None, None]))],
                vec![None],
            ],
        );
        let p = lote.column(0).as_struct();
        assert!(p.is_null(2));
        assert!(
            !p.is_null(1),
            "un struct con campos nulos no es un struct nulo"
        );
        assert_eq!(p.column(0).as_string::<i32>().value(0), "a");
        assert_eq!(p.column(1).as_list::<i32>().value(0).len(), 2);
        assert_eq!(
            p.column(2)
                .as_struct()
                .column(0)
                .as_primitive::<Int64Type>()
                .value(0),
            7
        );
        assert!(p.column(0).is_null(1));
    }

    #[test]
    fn una_lista_de_structs() {
        let f = Forma::Lista(Box::new(Forma::Struct(vec![(
            "k".into(),
            Forma::Escalar(Fisico::Entero),
        )])));
        let s = |k: i64| Some(Dato::Struct(vec![Some(Dato::Escalar(Valor::Entero(k)))]));
        let lote = lote_de(
            vec![Columna::de_forma("l", f, "list<Struct<k: Integer>>")],
            vec![
                vec![Some(Dato::Lista(vec![s(1), s(2)]))],
                vec![Some(Dato::Lista(vec![s(3)]))],
            ],
        );
        let l = lote.column(0).as_list::<i32>();
        let h = l.value(0);
        let h = h.as_struct();
        assert_eq!(
            h.column(0).as_primitive::<Int64Type>().values().to_vec(),
            vec![1, 2]
        );
        assert_eq!(l.value(1).len(), 1);
    }

    #[test]
    fn una_geometria_es_su_wkb_con_la_marca_de_geoarrow() {
        let f = Forma::Geo {
            subtipo: "Point".into(),
            crs: "EPSG:25830".into(),
            esferica: false,
        };
        let lote = lote_de(
            vec![Columna::de_forma("g", f, "Geometry<Point, EPSG:25830>")],
            vec![
                vec![Some(Dato::Escalar(Valor::Bytes(vec![1, 1, 0, 0, 0])))],
                vec![None],
            ],
        );
        let c = lote.schema().field(0).clone();
        assert_eq!(c.data_type(), &DataType::Binary);
        assert_eq!(c.metadata()["ARROW:extension:name"], GEOARROW_WKB);
        assert_eq!(
            c.metadata()["ARROW:extension:metadata"],
            "{\"crs\":\"EPSG:25830\"}"
        );
        assert_eq!(lote.column(0).as_binary::<i32>().value(0), &[1, 1, 0, 0, 0]);
        assert!(lote.column(0).is_null(1));
    }

    #[test]
    fn un_dato_de_otra_forma_se_rechaza_con_su_ruta_y_no_deja_la_fila_a_medias() {
        let f = Forma::Struct(vec![(
            "tags".into(),
            Forma::Lista(Box::new(Forma::Escalar(Fisico::Entero))),
        )]);
        let mut b = RecordBatchBuilder::nuevo(&[
            Columna::new("id", Fisico::Entero, "Integer"),
            Columna::de_forma("p", f, "Struct<…>"),
        ])
        .unwrap();
        let mala = vec![
            Some(Dato::Escalar(Valor::Entero(1))),
            Some(Dato::Struct(vec![Some(Dato::Lista(vec![Some(
                Dato::Escalar(Valor::Texto("x".into())),
            )]))])),
        ];
        let e = b.push_datos(mala).unwrap_err();
        assert!(e.0.contains("`p`") && e.0.contains("tags[0]"), "{e}");
        assert_eq!(b.filas(), 0);
        assert!(
            b.push(&[Some(Valor::Entero(1)), None]).is_ok(),
            "un nulo cabe en lo compuesto"
        );
        // `push` con un valor escalar en una columna compuesta: no.
        assert!(
            b.push(&[Some(Valor::Entero(1)), Some(Valor::Entero(2))])
                .is_err()
        );
        assert_eq!(b.finish().unwrap().num_rows(), 1);
    }

    // ── El mapa de tipos (ADR 0075, fase 2): lo que antes comparaba una prueba de `ore-store` con `carga` ──

    #[test]
    fn el_tipo_de_arrow_de_cada_tipo_de_oos() {
        let ts = |tz: Option<&str>| DataType::Timestamp(TimeUnit::Microsecond, tz.map(Into::into));
        let lista = |t: DataType| DataType::List(Arc::new(Field::new(ELEMENTO, t, true)));
        for (oos, esperado) in [
            ("String", DataType::Utf8),
            ("Integer", DataType::Int64),
            ("Float", DataType::Float64),
            ("Boolean", DataType::Boolean),
            ("Date", DataType::Date32),
            ("Time", DataType::Time64(TimeUnit::Microsecond)),
            ("DateTime", ts(None)),
            // El contrato del lago: `+00:00`, no `UTC`.
            ("DateTimeTz", ts(Some("+00:00"))),
            ("Binary", DataType::Binary),
            ("Decimal<12, 2>", DataType::Decimal128(12, 2)),
            ("Vector<3>", lista(DataType::Float32)),
            ("list<Integer>", lista(DataType::Int64)),
            ("Geometry<Point, EPSG:25830>", DataType::Binary),
            ("algo que no es un tipo", DataType::Utf8),
        ] {
            assert_eq!(tipo_del_oos(oos), esperado, "el tipo de `{oos}`");
        }
        assert_eq!(
            tipo_del_oos("Struct<a: Integer, b: list<String>>"),
            DataType::Struct(
                vec![
                    Field::new("a", DataType::Int64, true),
                    Field::new("b", lista(DataType::Utf8), true),
                ]
                .into()
            )
        );
    }

    #[test]
    fn el_campo_de_una_geometria_lleva_su_marca_y_el_de_lo_demas_no() {
        let g = campo_del_oos("g", "Geometry<Point, EPSG:25830>");
        assert_eq!(g.metadata()["ARROW:extension:name"], GEOARROW_WKB);
        assert_eq!(
            g.metadata()["ARROW:extension:metadata"],
            "{\"crs\":\"EPSG:25830\"}"
        );
        assert_eq!(g.metadata()[META_TIPO], "Geometry<Point, EPSG:25830>");
        let e = campo_del_oos("e", "Geography<Polygon>");
        assert!(e.metadata()["ARROW:extension:metadata"].contains("spherical"));
        assert!(campo_del_oos("n", "Integer").metadata().is_empty());
        assert_eq!(tipo_fisico(&Fisico::Instante), tipo_del_oos("DateTimeTz"));
    }

    // ── La escritura tipada (ADR 0075, fase 2.5) ──

    #[test]
    fn la_escritura_tipada_da_el_mismo_lote_que_push() {
        let mut a = RecordBatchBuilder::nuevo(&columnas()).unwrap();
        a.push(&completa()).unwrap();
        a.push(&vec![None; 9]).unwrap();
        let por_push = a.finish().unwrap();

        let mut b = RecordBatchBuilder::con_capacidad(&columnas(), 2).unwrap();
        b.col(0).entero(42).unwrap();
        b.col(1).real(0.5).unwrap();
        b.col(2).decimal(123_456_789_012).unwrap();
        b.col(3).logico(true).unwrap();
        b.col(4).micros(1_792_914_255_123_456).unwrap();
        b.col(5).micros(1_792_934_055_123_456).unwrap();
        b.col(6).fecha(20_751).unwrap();
        b.col(7).hora(3_600_000_000).unwrap();
        b.col(8).texto("ñandú").unwrap();
        b.fila_escrita();
        for c in 0..9 {
            b.col(c).nulo();
        }
        b.fila_escrita();
        let tipado = b.finish().unwrap();
        assert_eq!(por_push, tipado);
    }

    #[test]
    fn la_escritura_tipada_rechaza_un_fisico_ajeno_sin_escribir_y_detecta_columnas_desiguales() {
        let mut b = RecordBatchBuilder::nuevo(&[
            Columna::new("id", Fisico::Entero, "Integer"),
            Columna::new("t", Fisico::Texto, "String"),
        ])
        .unwrap();
        assert!(b.col(0).texto("x").is_err(), "un texto en un entero");
        assert!(b.col(1).bytes(&[1]).is_err(), "bytes en un texto");
        // Una celda de menos: la segunda columna se queda corta.
        b.col(0).entero(1).unwrap();
        b.fila_escrita();
        let e = b.finish().unwrap_err();
        assert!(e.0.contains("`t`") && e.0.contains("0 valores"), "{e}");
        // Tras el error el constructor queda limpio y sirve.
        b.col(0).entero(2).unwrap();
        b.col(1).texto("a").unwrap();
        b.fila_escrita();
        assert_eq!(b.finish().unwrap().num_rows(), 1);
    }
}
