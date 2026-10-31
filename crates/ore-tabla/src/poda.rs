//! **La poda al planificar** (0081 §7.6, K2): los filtros de la consulta, como predicado de Iceberg, para que el
//! escaneo de iceberg-rust descarte manifests y ficheros que no pueden casar.
//!
//! - **Sólo lo que se traduce exacto.** Una columna de primer nivel comparada con un literal del tipo de esa columna
//!   en el snapshot que se lee (`=`, `<>`, `<`, `<=`, `>`, `>=`, `IN`, `BETWEEN`, `IS [NOT] NULL`), y `AND`, `OR` y
//!   `NOT` de eso. Lo demás no se traduce: un cast, una función o un literal de otro tipo (la clase de error de
//!   iceberg-rust #3483).
//! - **Inexacto.** DataFusion vuelve a aplicar cada filtro sobre lo leído, así que la poda sólo quita ficheros, nunca
//!   filas: lo que no se traduce se lee entero, y una poda que se queda corta da el mismo resultado.
//! - **La guarda** ([`comprobar`]): lo planificado no puede pasar de los ficheros del snapshot, y sin poda tiene que
//!   ser exactamente ellos. Un escaneo que planifica cero ficheros de un snapshot que tiene es un error, no un
//!   resultado vacío (S3·4).

use datafusion::common::{DataFusionError, Result, ScalarValue};
use datafusion::logical_expr::{Between, BinaryExpr, Expr, Operator, expr::InList};
use iceberg::expr::{Predicate, Reference};
use iceberg::spec::{Datum, PrimitiveType, Schema, Type};

/// **El predicado de Iceberg** de los filtros que se traducen exactos, en `AND`; `None` si no se traduce ninguno.
pub(crate) fn predicado(filtros: &[Expr], esquema: &Schema) -> Option<Predicate> {
    filtros
        .iter()
        .filter_map(|f| traducir(f, esquema))
        .reduce(Predicate::and)
}

/// Si un filtro se traduce entero.
pub(crate) fn se_traduce(filtro: &Expr, esquema: &Schema) -> bool {
    traducir(filtro, esquema).is_some()
}

fn traducir(e: &Expr, esquema: &Schema) -> Option<Predicate> {
    match e {
        Expr::BinaryExpr(BinaryExpr { left, op, right }) => match op {
            Operator::And => Some(traducir(left, esquema)?.and(traducir(right, esquema)?)),
            Operator::Or => Some(traducir(left, esquema)?.or(traducir(right, esquema)?)),
            Operator::Eq
            | Operator::NotEq
            | Operator::Lt
            | Operator::LtEq
            | Operator::Gt
            | Operator::GtEq => {
                let (columna, valor, op) = match (left.as_ref(), right.as_ref()) {
                    (Expr::Column(c), Expr::Literal(v, _)) => (c, v, *op),
                    (Expr::Literal(v, _), Expr::Column(c)) => (c, v, op.swap()?),
                    _ => return None,
                };
                let d = dato(valor, tipo(esquema, &columna.name)?)?;
                let r = Reference::new(columna.name.clone());
                Some(match op {
                    Operator::Eq => r.equal_to(d),
                    Operator::NotEq => r.not_equal_to(d),
                    Operator::Lt => r.less_than(d),
                    Operator::LtEq => r.less_than_or_equal_to(d),
                    Operator::Gt => r.greater_than(d),
                    _ => r.greater_than_or_equal_to(d),
                })
            }
            _ => None,
        },
        Expr::Not(e) => Some(traducir(e, esquema)?.negate()),
        Expr::IsNull(e) => Some(Reference::new(columna(e, esquema)?).is_null()),
        Expr::IsNotNull(e) => Some(Reference::new(columna(e, esquema)?).is_not_null()),
        Expr::InList(InList {
            expr,
            list,
            negated,
        }) => {
            let nombre = columna(expr, esquema)?;
            let t = tipo(esquema, &nombre)?;
            let datos = list
                .iter()
                .map(|v| match v {
                    Expr::Literal(v, _) => dato(v, t),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()?;
            let r = Reference::new(nombre);
            Some(if *negated {
                r.is_not_in(datos)
            } else {
                r.is_in(datos)
            })
        }
        Expr::Between(Between {
            expr,
            negated,
            low,
            high,
        }) => {
            let nombre = columna(expr, esquema)?;
            let t = tipo(esquema, &nombre)?;
            let (Expr::Literal(a, _), Expr::Literal(b, _)) = (low.as_ref(), high.as_ref()) else {
                return None;
            };
            let entre = Reference::new(nombre.clone())
                .greater_than_or_equal_to(dato(a, t)?)
                .and(Reference::new(nombre).less_than_or_equal_to(dato(b, t)?));
            Some(if *negated { entre.negate() } else { entre })
        }
        _ => None,
    }
}

/// El nombre de una columna de primer nivel que el esquema tiene.
fn columna(e: &Expr, esquema: &Schema) -> Option<String> {
    match e {
        Expr::Column(c) => tipo(esquema, &c.name).map(|_| c.name.clone()),
        _ => None,
    }
}

/// El tipo primitivo de una columna de primer nivel.
fn tipo<'a>(esquema: &'a Schema, nombre: &str) -> Option<&'a PrimitiveType> {
    let campo = esquema.as_struct().field_by_name(nombre)?;
    match campo.field_type.as_ref() {
        Type::Primitive(p) => Some(p),
        _ => None,
    }
}

/// El literal, en el tipo de la columna, si cabe exacto. Un nulo no se traduce: no casa con nada.
fn dato(v: &ScalarValue, t: &PrimitiveType) -> Option<Datum> {
    use ScalarValue as S;
    let entero = match v {
        S::Int8(Some(x)) => Some(i64::from(*x)),
        S::Int16(Some(x)) => Some(i64::from(*x)),
        S::Int32(Some(x)) => Some(i64::from(*x)),
        S::Int64(Some(x)) => Some(*x),
        S::UInt8(Some(x)) => Some(i64::from(*x)),
        S::UInt16(Some(x)) => Some(i64::from(*x)),
        S::UInt32(Some(x)) => Some(i64::from(*x)),
        S::UInt64(Some(x)) => i64::try_from(*x).ok(),
        _ => None,
    };
    let micros = |v: &ScalarValue| match v {
        S::TimestampSecond(Some(x), z) => x.checked_mul(1_000_000).map(|m| (m, z.is_some())),
        S::TimestampMillisecond(Some(x), z) => x.checked_mul(1_000).map(|m| (m, z.is_some())),
        S::TimestampMicrosecond(Some(x), z) => Some((*x, z.is_some())),
        _ => None,
    };
    match (t, v) {
        (PrimitiveType::Boolean, S::Boolean(Some(b))) => Some(Datum::bool(*b)),
        (PrimitiveType::Int, _) => i32::try_from(entero?).ok().map(Datum::int),
        (PrimitiveType::Long, _) => entero.map(Datum::long),
        (PrimitiveType::Float, S::Float32(Some(x))) if !x.is_nan() => Some(Datum::float(*x)),
        (PrimitiveType::Double, S::Float32(Some(x))) if !x.is_nan() => Some(Datum::double(*x)),
        (PrimitiveType::Double, S::Float64(Some(x))) if !x.is_nan() => Some(Datum::double(*x)),
        (
            PrimitiveType::String,
            S::Utf8(Some(s)) | S::LargeUtf8(Some(s)) | S::Utf8View(Some(s)),
        ) => Some(Datum::string(s)),
        (PrimitiveType::Date, S::Date32(Some(d))) => Some(Datum::date(*d)),
        (PrimitiveType::Timestamp, _) => match micros(v)? {
            (m, false) => Some(Datum::timestamp_micros(m)),
            _ => None,
        },
        (PrimitiveType::Timestamptz, _) => match micros(v)? {
            (m, true) => Some(Datum::timestamptz_micros(m)),
            _ => None,
        },
        _ => None,
    }
}

/// **La guarda de lo planificado**: `planificados` ficheros de un snapshot que tiene `total` (su resumen,
/// `total-data-files`), con poda o sin ella.
pub(crate) fn comprobar(
    nombre: &str,
    planificados: usize,
    total: Option<u64>,
    podado: bool,
) -> Result<()> {
    let Some(total) = total else {
        return Ok(());
    };
    let n = planificados as u64;
    if n > total || (!podado && n != total) {
        return Err(DataFusionError::Execution(format!(
            "el escaneo de `{nombre}` planificó {n} ficheros de un snapshot que tiene {total}: no se lee algo que no \
             es la tabla"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use datafusion::logical_expr::{col, lit};
    use iceberg::spec::NestedField;
    use std::sync::Arc;

    fn esquema() -> Schema {
        Schema::builder()
            .with_fields(vec![
                Arc::new(NestedField::optional(
                    1,
                    "id",
                    Type::Primitive(PrimitiveType::Long),
                )),
                Arc::new(NestedField::optional(
                    2,
                    "k",
                    Type::Primitive(PrimitiveType::Int),
                )),
                Arc::new(NestedField::optional(
                    3,
                    "t",
                    Type::Primitive(PrimitiveType::String),
                )),
            ])
            .build()
            .unwrap()
    }

    fn texto(e: Expr) -> Option<String> {
        traducir(&e, &esquema()).map(|p| p.to_string())
    }

    #[test]
    fn lo_que_se_traduce_exacto() {
        assert_eq!(texto(col("id").eq(lit(5i64))).unwrap(), "id = 5");
        assert_eq!(texto(lit(5i64).lt(col("id"))).unwrap(), "id > 5");
        assert_eq!(texto(col("k").eq(lit(7i64))).unwrap(), "k = 7");
        assert!(texto(col("t").eq(lit("x")).and(col("id").gt_eq(lit(1i64)))).is_some());
        assert!(texto(col("id").is_null()).is_some());
        assert!(texto(col("id").in_list(vec![lit(1i64), lit(2i64)], false)).is_some());
        assert!(texto(col("id").between(lit(1i64), lit(9i64))).is_some());
    }

    #[test]
    fn lo_que_no_se_traduce_se_lee_entero() {
        // Un literal que no cabe en la columna, de otro tipo, una función, un cast o una columna que no está.
        assert!(texto(col("k").eq(lit(5_000_000_000i64))).is_none());
        assert!(texto(col("id").eq(lit("5"))).is_none());
        assert!(texto(col("id").eq(lit(5.0f64))).is_none());
        assert!(texto(col("id").eq(col("k"))).is_none());
        assert!(texto(col("nadie").eq(lit(1i64))).is_none());
        assert!(texto(col("id").eq(lit(ScalarValue::Int64(None)))).is_none());
        // Un `OR` con una mitad que no se traduce no se traduce: quitarla no sería una poda segura.
        assert!(texto(col("id").eq(lit(1i64)).or(col("id").eq(lit("x")))).is_none());
    }

    #[test]
    fn la_guarda() {
        assert!(comprobar("t", 3, Some(3), false).is_ok());
        assert!(comprobar("t", 1, Some(3), true).is_ok());
        assert!(comprobar("t", 0, Some(3), true).is_ok());
        assert!(comprobar("t", 0, None, false).is_ok());
        let e = comprobar("t", 0, Some(3), false).unwrap_err().to_string();
        assert!(
            e.contains("planificó 0 ficheros de un snapshot que tiene 3"),
            "{e}"
        );
        assert!(comprobar("t", 4, Some(3), true).is_err());
    }
}
