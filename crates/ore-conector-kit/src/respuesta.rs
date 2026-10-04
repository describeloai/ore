//! **Lo que el conector contestó**, leído a valores.
//!
//! Dos formas, por su primer byte (ADR 0043): un flujo Arrow IPC, o el texto de
//! v1 —una fila JSON por línea, un nulo es la propiedad ausente—. El texto se
//! analiza con el físico que la columna debería tener; Arrow se lee por el tipo
//! que trae, y si no es el debido, el caso 5 lo dice y los valores no casan.

use arrow_array::cast::AsArray;
use arrow_array::types::{
    Date32Type, Decimal128Type, Float32Type, Float64Type, Int16Type, Int32Type, Int64Type,
    Time64MicrosecondType, TimestampMicrosecondType, TimestampMillisecondType,
    TimestampNanosecondType, TimestampSecondType,
};
use arrow_array::{Array, ArrayRef};
use arrow_schema::{DataType, TimeUnit};
use ore_core::tipos::{Fisico, Valor};

/// En qué forma llegó.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Forma {
    Arrow,
    Texto,
}

/// Una respuesta leída.
#[derive(Debug)]
pub struct Respuesta {
    pub forma: Forma,
    /// Los campos, con su tipo Arrow si llegó en Arrow.
    pub campos: Vec<(String, Option<DataType>)>,
    /// Las filas, en el orden de `campos`.
    pub filas: Vec<Vec<Option<Valor>>>,
    /// Cuántos lotes trajo el flujo (1 en texto).
    pub lotes: usize,
}

impl Respuesta {
    /// La posición de un campo.
    pub fn campo(&self, nombre: &str) -> Option<usize> {
        self.campos.iter().position(|(n, _)| n == nombre)
    }

    /// Los valores enteros de un campo, en orden (los `id`).
    pub fn enteros(&self, nombre: &str) -> Result<Vec<i64>, String> {
        let i = self
            .campo(nombre)
            .ok_or_else(|| format!("no vuelve el campo `{nombre}`"))?;
        self.filas
            .iter()
            .map(|f| match &f[i] {
                Some(Valor::Entero(n)) => Ok(*n),
                otro => Err(format!("`{nombre}` trae {otro:?}, no un entero")),
            })
            .collect()
    }
}

/// El tipo Arrow que el contrato da a un físico (0032; el mismo que escribe el
/// almacén y que lee `ore-read-s3`). Compárese con [`mismo_tipo`].
pub fn tipo_arrow(f: &Fisico) -> DataType {
    match f {
        Fisico::Texto => DataType::Utf8,
        Fisico::Entero => DataType::Int64,
        Fisico::Real => DataType::Float64,
        Fisico::Logico => DataType::Boolean,
        Fisico::Decimal { precision, escala } => DataType::Decimal128(*precision, *escala as i8),
        Fisico::Fecha => DataType::Date32,
        Fisico::Hora => DataType::Time64(TimeUnit::Microsecond),
        Fisico::FechaHora => DataType::Timestamp(TimeUnit::Microsecond, None),
        Fisico::Instante => DataType::Timestamp(TimeUnit::Microsecond, Some("+00:00".into())),
    }
}

/// **El mismo tipo Arrow**, con UTC dicho de cualquiera de sus formas: el lago
/// lo escribe `+00:00` (el nombre de Iceberg, `ore-store` `carga::UTC`) y
/// BigQuery lo da como `UTC`. Es la misma zona, y un conector no falla por
/// llamarla de otra manera.
pub fn mismo_tipo(a: &DataType, b: &DataType) -> bool {
    let utc = |z: &str| matches!(z, "UTC" | "+00:00" | "Z" | "Etc/UTC" | "+0000");
    match (a, b) {
        (DataType::Timestamp(u, Some(x)), DataType::Timestamp(v, Some(y))) => {
            u == v && (x == y || (utc(x) && utc(y)))
        }
        _ => a == b,
    }
}

/// Un instante en microsegundos, sea cual sea su unidad.
fn micros(a: &ArrayRef, i: usize, u: &TimeUnit) -> i64 {
    match u {
        TimeUnit::Second => a.as_primitive::<TimestampSecondType>().value(i) * 1_000_000,
        TimeUnit::Millisecond => a.as_primitive::<TimestampMillisecondType>().value(i) * 1_000,
        TimeUnit::Microsecond => a.as_primitive::<TimestampMicrosecondType>().value(i),
        TimeUnit::Nanosecond => a.as_primitive::<TimestampNanosecondType>().value(i) / 1_000,
    }
}

/// Una celda de Arrow, como valor.
fn celda(a: &ArrayRef, i: usize) -> Result<Option<Valor>, String> {
    if a.is_null(i) {
        return Ok(None);
    }
    Ok(Some(match a.data_type() {
        DataType::Int64 => Valor::Entero(a.as_primitive::<Int64Type>().value(i)),
        DataType::Int32 => Valor::Entero(a.as_primitive::<Int32Type>().value(i).into()),
        DataType::Int16 => Valor::Entero(a.as_primitive::<Int16Type>().value(i).into()),
        DataType::Float64 => Valor::Real(a.as_primitive::<Float64Type>().value(i)),
        DataType::Float32 => Valor::Real(a.as_primitive::<Float32Type>().value(i).into()),
        DataType::Boolean => Valor::Logico(a.as_boolean().value(i)),
        DataType::Decimal128(_, _) => Valor::Decimal(a.as_primitive::<Decimal128Type>().value(i)),
        DataType::Utf8 => Valor::Texto(a.as_string::<i32>().value(i).to_string()),
        DataType::LargeUtf8 => Valor::Texto(a.as_string::<i64>().value(i).to_string()),
        DataType::Date32 => Valor::Fecha(a.as_primitive::<Date32Type>().value(i)),
        DataType::Time64(TimeUnit::Microsecond) => {
            Valor::Hora(a.as_primitive::<Time64MicrosecondType>().value(i))
        }
        DataType::Timestamp(u, None) => Valor::FechaHora(micros(a, i, u)),
        DataType::Timestamp(u, Some(_)) => Valor::Instante(micros(a, i, u)),
        otro => return Err(format!("un tipo Arrow que el contrato no da: {otro}")),
    }))
}

/// **Lee la salida de `leer`.** `esperado` es propiedad → físico, para el
/// texto (que no trae tipos) y para el orden de sus campos.
pub fn leer(bytes: &[u8], esperado: &[(&str, Fisico)]) -> Result<Respuesta, String> {
    if bytes.starts_with(&[0xFF, 0xFF, 0xFF, 0xFF]) {
        let r = arrow_ipc::reader::StreamReader::try_new(bytes, None)
            .map_err(|e| format!("el flujo Arrow no empieza bien: {e}"))?;
        let esquema = r.schema();
        let campos: Vec<(String, Option<DataType>)> = esquema
            .fields()
            .iter()
            .map(|f| (f.name().clone(), Some(f.data_type().clone())))
            .collect();
        let mut filas = Vec::new();
        let mut lotes = 0;
        for lote in r {
            let lote = lote.map_err(|e| format!("un lote Arrow no se lee: {e}"))?;
            lotes += 1;
            for i in 0..lote.num_rows() {
                filas.push(
                    lote.columns()
                        .iter()
                        .map(|c| celda(c, i))
                        .collect::<Result<Vec<_>, _>>()?,
                );
            }
        }
        return Ok(Respuesta {
            forma: Forma::Arrow,
            campos,
            filas,
            lotes,
        });
    }
    let texto = std::str::from_utf8(bytes).map_err(|_| "la salida no es Arrow ni UTF-8")?;
    let mut filas = Vec::new();
    for l in texto.lines().filter(|l| !l.trim().is_empty()) {
        let n = ore_core::parse::parse(l).map_err(|e| format!("una fila no es JSON: {e:?}"))?;
        let mut fila = Vec::with_capacity(esperado.len());
        for (prop, f) in esperado {
            fila.push(match n.get(prop).and_then(|(_, v)| v.as_str()) {
                None => None,
                Some(t) => Some(
                    f.analizar(t)
                        .ok_or_else(|| format!("`{prop}` = `{t}` no es un {}", f.arrow()))?,
                ),
            });
        }
        filas.push(fila);
    }
    Ok(Respuesta {
        forma: Forma::Texto,
        campos: esperado
            .iter()
            .map(|(p, _)| (p.to_string(), None))
            .collect(),
        filas,
        lotes: 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_dicho_de_dos_maneras_es_el_mismo_tipo() {
        let t = |z: &str| DataType::Timestamp(TimeUnit::Microsecond, Some(z.into()));
        assert!(mismo_tipo(&t("UTC"), &t("+00:00")));
        assert!(!mismo_tipo(&t("UTC"), &t("+01:00")));
        assert!(!mismo_tipo(
            &t("UTC"),
            &DataType::Timestamp(TimeUnit::Microsecond, None)
        ));
        assert!(!mismo_tipo(
            &DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())),
            &t("UTC")
        ));
    }
}
