//! Lo que llega en el cable, ya sin protobuf: las columnas de un resultado y los valores de una fila.
//!
//! Aquí **no se interpreta** el tipo de Phoenix (eso es `ore-phoenix`, M2): se conserva el nombre
//! (`UNSIGNED_INT`, `DATE`…) y la representación (`rep`) que el servidor dio, y cada valor se decodifica
//! por cómo viaja (0071 §3.4): un número, un real, un texto, unos bytes, un decimal exacto como texto
//! o una lista. Lo que se pierde en el cable (los nanosegundos, la hora de un `DATE`) ya está perdido.

use crate::proto::{self, ColumnMetaData, ColumnValue, Rep, TypedValue};

/// Un valor de una celda.
#[derive(Debug, Clone, PartialEq)]
pub enum Valor {
    Nulo,
    Bool(bool),
    /// Todo entero (`TINYINT` … `BIGINT`, los `UNSIGNED_*`) y los instantes: `DATE` en **días**, `TIME` en
    /// milisegundos del día, `TIMESTAMP` en **milisegundos** desde la época.
    Entero(i64),
    /// `FLOAT` y `DOUBLE`.
    Real(f64),
    Texto(String),
    Bytes(Vec<u8>),
    /// `DECIMAL`, **exacto**, como texto (`"12345.678"`): pasarlo por `f64` perdería dígitos.
    Decimal(String),
    Lista(Vec<Valor>),
}

/// Una fila de un resultado.
pub type Fila = Vec<Valor>;

/// Si la columna admite nulos, según JDBC (`columnNoNulls`, `columnNullable`, `columnNullableUnknown`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anulable {
    No,
    Si,
    Desconocido,
}

/// La columna de un resultado.
#[derive(Debug, Clone, PartialEq)]
pub struct Columna {
    pub nombre: String,
    pub etiqueta: String,
    pub esquema: String,
    pub tabla: String,
    /// El nombre del tipo del servidor: `BIGINT`, `UNSIGNED_INT`, `DECIMAL`, `INTEGER ARRAY`…
    /// **Es lo que distingue un tipo sin signo**: su `rep` es el de signo.
    pub tipo: String,
    /// El código JDBC (`java.sql.Types`; los negativos, como `BIGINT` = -5, ya con signo).
    pub codigo_sql: i32,
    pub rep: Option<Rep>,
    pub precision: u32,
    pub escala: u32,
    pub anulable: Anulable,
    pub con_signo: bool,
    /// El tipo del elemento, en un `ARRAY`.
    pub componente: Option<Box<Componente>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Componente {
    pub tipo: String,
    pub codigo_sql: i32,
    pub rep: Option<Rep>,
}

impl Columna {
    pub(crate) fn de(c: &ColumnMetaData) -> Columna {
        let t = c.r#type.as_ref();
        Columna {
            nombre: c.column_name.clone(),
            etiqueta: c.label.clone(),
            esquema: c.schema_name.clone(),
            tabla: c.table_name.clone(),
            tipo: t.map(|t| t.name.clone()).unwrap_or_default(),
            codigo_sql: t.map(|t| t.id as i32).unwrap_or_default(),
            rep: t.and_then(|t| Rep::try_from(t.rep).ok()),
            precision: c.precision,
            escala: c.scale,
            anulable: match c.nullable {
                0 => Anulable::No,
                1 => Anulable::Si,
                _ => Anulable::Desconocido,
            },
            con_signo: c.signed,
            componente: t.and_then(|t| t.component.as_ref()).map(|k| {
                Box::new(Componente {
                    tipo: k.name.clone(),
                    codigo_sql: k.id as i32,
                    rep: Rep::try_from(k.rep).ok(),
                })
            }),
        }
    }
}

/// Decodifica una celda. Un `ColumnValue` trae `array_value` si es una lista, o un `scalar_value`; los
/// servidores viejos mandaban el escalar en `value[0]` (el campo está marcado como obsoleto).
pub(crate) fn valor_de(c: &ColumnValue) -> Valor {
    if c.has_array_value {
        return Valor::Lista(c.array_value.iter().map(tipado).collect());
    }
    match c.scalar_value.as_ref().or_else(|| c.value.first()) {
        Some(t) => tipado(t),
        None => Valor::Nulo,
    }
}

fn tipado(t: &TypedValue) -> Valor {
    if t.null || t.implicitly_null {
        return Valor::Nulo;
    }
    if !t.array_value.is_empty() {
        return Valor::Lista(t.array_value.iter().map(tipado).collect());
    }
    use proto::Rep::*;
    match Rep::try_from(t.r#type).unwrap_or(Object) {
        Null => Valor::Nulo,
        Boolean | PrimitiveBoolean => Valor::Bool(t.bool_value),
        PrimitiveByte | PrimitiveShort | PrimitiveInt | PrimitiveLong | Byte | Short | Integer
        | Long | JavaSqlTime | JavaSqlTimestamp | JavaSqlDate | JavaUtilDate => {
            Valor::Entero(t.number_value)
        }
        PrimitiveChar | Character => Valor::Texto(t.string_value.clone()),
        PrimitiveFloat | PrimitiveDouble | Float | Double => Valor::Real(t.double_value),
        BigDecimal | BigInteger => Valor::Decimal(t.string_value.clone()),
        Number => {
            if t.string_value.is_empty() {
                Valor::Real(t.double_value)
            } else {
                Valor::Decimal(t.string_value.clone())
            }
        }
        String => Valor::Texto(t.string_value.clone()),
        ByteString => Valor::Bytes(t.bytes_value.clone()),
        Array | Multiset => Valor::Lista(t.array_value.iter().map(tipado).collect()),
        Object | Struct => Valor::Texto(t.string_value.clone()),
    }
}
