//! **Lo compuesto**: de una [`Forma`] de OOS (struct, lista, `Vector<n>`, geometría) al tipo y al array de Arrow.
//!
//! Es lo que [`crate::RecordBatchBuilder`] hace con una columna que no es un físico escalar. El mapa de tipos es
//! el mismo que hoy tiene `ore-store::carga` (`arrow_de_forma`, `campo`); aquí está una segunda vez hasta que la
//! fase 2 del ADR 0075 borre aquella copia, y una prueba de `ore-store` los compara para todas las formas.

use crate::col_de;
use arrow_array::{ArrayRef, ListArray, StructArray};
use arrow_buffer::{NullBuffer, OffsetBuffer};
use arrow_schema::{DataType, Field, Fields, TimeUnit};
use ore_base::tipos::{Fisico, Forma, Valor};
use std::collections::HashMap;
use std::sync::Arc;

/// La zona de un instante **en el contrato del lago**: `+00:00` y no `UTC` (la misma física, otro nombre; Iceberg
/// la escribe así y el escritor exige que el esquema del lote case byte a byte). Es la de `carga::UTC`.
pub const UTC: &str = "+00:00";

/// El nombre del elemento de una lista, el de Iceberg y Parquet.
pub const ELEMENTO: &str = "element";

/// La clave de los metadatos de un campo que lleva su tipo de OOS (solo las columnas geo, que Arrow no sabe nombrar
/// enteras).
pub const META_TIPO: &str = "ore:type";

/// El nombre de la extensión de GeoArrow para WKB.
pub const GEOARROW_WKB: &str = "geoarrow.wkb";

/// Un valor de una columna de cualquier forma. Los valores de un físico escalar siguen siendo [`Valor`]; esto solo
/// añade lo que los contiene.
///
/// - una geometría es `Escalar(Valor::Bytes)`: su WKB;
/// - un `Vector<n>` es una `Lista` de `Escalar(Valor::Real)`;
/// - un struct es una `Struct` con un dato por campo, **en el orden de la forma**; `None` es un campo nulo.
#[derive(Debug, Clone, PartialEq)]
pub enum Dato {
    Escalar(Valor),
    Lista(Vec<Option<Dato>>),
    Struct(Vec<Option<Dato>>),
}

/// De la forma al `DataType`. Los hijos no llevan id: los pone el esquema de la tabla.
pub fn tipo_arrow(f: &Forma) -> DataType {
    match f {
        Forma::Escalar(Fisico::Instante) => {
            DataType::Timestamp(TimeUnit::Microsecond, Some(UTC.into()))
        }
        Forma::Escalar(e) => col_de(e).map(|(dt, _)| dt).unwrap_or(DataType::Utf8),
        Forma::Struct(cs) => DataType::Struct(
            cs.iter()
                .map(|(n, f)| Field::new(n, tipo_arrow(f), true))
                .collect(),
        ),
        Forma::Lista(f) => DataType::List(Arc::new(Field::new(ELEMENTO, tipo_arrow(f), true))),
        Forma::Vector(_) => DataType::List(Arc::new(Field::new(ELEMENTO, DataType::Float32, true))),
        // v1alpha32: WKB. La extensión y su CRS van en los metadatos del campo ([`campo`]).
        Forma::Geo { .. } => DataType::Binary,
    }
}

/// El tipo de Arrow de un físico escalar **en el contrato del lago** (un instante en [`UTC`], `+00:00`).
pub fn tipo_fisico(f: &Fisico) -> DataType {
    tipo_arrow(&Forma::Escalar(*f))
}

/// El tipo de Arrow de un tipo de OOS tal como la cabecera lo declara: lo que `carga::lote` le da a cada columna.
/// Un tipo que no analiza (no debería llegar: la cabecera la escribe `ore`) va como texto, que es lo que era.
pub fn tipo_del_oos(oos: &str) -> DataType {
    match ore_base::types::parse_type(oos) {
        Ok(t) => tipo_arrow(&Forma::de(&t)),
        Err(_) => DataType::Utf8,
    }
}

/// El campo de Arrow de una columna del contrato desde su tipo de OOS, con la marca de geometría si lo es.
pub fn campo_del_oos(nombre: &str, oos: &str) -> Field {
    match ore_base::types::parse_type(oos) {
        Ok(t) => campo(nombre, &Forma::de(&t)),
        Err(_) => Field::new(nombre, DataType::Utf8, true),
    }
}

/// El campo de Arrow de una columna: su tipo y, si es una geometría, la extensión `geoarrow.wkb` con su CRS en los
/// metadatos (`01-la-geometria` §4).
pub fn campo(nombre: &str, f: &Forma) -> Field {
    let c = Field::new(nombre, tipo_arrow(f), true);
    match (f.geoarrow_metadatos(), f) {
        (
            Some(m),
            Forma::Geo {
                subtipo,
                crs,
                esferica,
            },
        ) => {
            let tipo = if *esferica {
                format!("Geography<{subtipo}>")
            } else {
                format!("Geometry<{subtipo}, {crs}>")
            };
            let tipo = ore_base::types::parse_type(&tipo)
                .map(|t| t.to_string())
                .unwrap_or(tipo);
            c.with_metadata(HashMap::from([
                ("ARROW:extension:name".to_string(), GEOARROW_WKB.to_string()),
                ("ARROW:extension:metadata".to_string(), m),
                (META_TIPO.to_string(), tipo),
            ]))
        }
        _ => c,
    }
}

/// ¿Es este dato de esta forma? Dice dónde, si no (`a.b[2]`).
pub(crate) fn admite(f: &Forma, d: &Dato) -> Result<(), String> {
    admite_en(f, d, "")
}

fn admite_en(f: &Forma, d: &Dato, ruta: &str) -> Result<(), String> {
    let donde = |m: &str| {
        if ruta.is_empty() {
            m.to_string()
        } else {
            format!("`{ruta}`: {m}")
        }
    };
    match (f, d) {
        (Forma::Escalar(e), Dato::Escalar(v)) => {
            let (_, col) = col_de(e).map_err(|e| donde(&e.0))?;
            if col.admite(Some(v)) {
                Ok(())
            } else {
                Err(donde("un valor de otro físico"))
            }
        }
        (Forma::Geo { .. }, Dato::Escalar(Valor::Bytes(_))) => Ok(()),
        (Forma::Vector(_), Dato::Lista(xs)) => {
            for (i, x) in xs.iter().enumerate() {
                if !matches!(x, None | Some(Dato::Escalar(Valor::Real(_)))) {
                    return Err(donde(&format!(
                        "el elemento {i} de un vector no es un real"
                    )));
                }
            }
            Ok(())
        }
        (Forma::Lista(h), Dato::Lista(xs)) => {
            for (i, x) in xs.iter().enumerate() {
                if let Some(x) = x {
                    admite_en(h, x, &format!("{ruta}[{i}]"))?;
                }
            }
            Ok(())
        }
        (Forma::Struct(cs), Dato::Struct(xs)) => {
            if cs.len() != xs.len() {
                return Err(donde(&format!(
                    "el struct trae {} campos y su forma {}",
                    xs.len(),
                    cs.len()
                )));
            }
            for ((n, h), x) in cs.iter().zip(xs) {
                if let Some(x) = x {
                    let r = if ruta.is_empty() {
                        n.clone()
                    } else {
                        format!("{ruta}.{n}")
                    };
                    admite_en(h, x, &r)?;
                }
            }
            Ok(())
        }
        _ => Err(donde("un dato que no es de la forma de la columna")),
    }
}

/// El array de una columna de esta forma, con estos datos (uno por fila). Los datos ya pasaron por [`admite`].
pub(crate) fn construir(f: &Forma, datos: &[Option<Dato>]) -> Result<ArrayRef, String> {
    let refs: Vec<Option<&Dato>> = datos.iter().map(Option::as_ref).collect();
    armar(f, &refs)
}

fn armar(f: &Forma, datos: &[Option<&Dato>]) -> Result<ArrayRef, String> {
    match f {
        Forma::Escalar(e) => {
            let (_, mut col) = match e {
                Fisico::Instante => (
                    DataType::Null,
                    crate::Col::Instante(
                        arrow_array::builder::TimestampMicrosecondBuilder::new().with_timezone(UTC),
                    ),
                ),
                e => col_de(e).map_err(|e| e.0)?,
            };
            for d in datos {
                match d {
                    None => col.escribir(None),
                    Some(Dato::Escalar(v)) => col.escribir(Some(v)),
                    Some(_) => return Err("un dato compuesto en una columna escalar".into()),
                }
            }
            Ok(col.terminar())
        }
        Forma::Geo { .. } => {
            let mut b = arrow_array::builder::BinaryBuilder::new();
            for d in datos {
                match d {
                    None => b.append_null(),
                    Some(Dato::Escalar(Valor::Bytes(w))) => b.append_value(w),
                    Some(_) => return Err("una geometría es su WKB".into()),
                }
            }
            Ok(Arc::new(b.finish()))
        }
        Forma::Vector(_) => {
            let mut hijos: Vec<Option<f32>> = Vec::new();
            let mut offsets = vec![0i32];
            let mut nulos = Vec::with_capacity(datos.len());
            for d in datos {
                nulos.push(d.is_some());
                if let Some(Dato::Lista(xs)) = d {
                    for x in xs {
                        hijos.push(match x {
                            Some(Dato::Escalar(Valor::Real(r))) => Some(*r as f32),
                            _ => None,
                        });
                    }
                }
                offsets.push(hijos.len() as i32);
            }
            let valores: ArrayRef = Arc::new(arrow_array::Float32Array::from(hijos));
            lista(DataType::Float32, offsets, valores, nulos)
        }
        Forma::Lista(h) => {
            let mut hijos: Vec<Option<&Dato>> = Vec::new();
            let mut offsets = vec![0i32];
            let mut nulos = Vec::with_capacity(datos.len());
            for d in datos {
                nulos.push(d.is_some());
                if let Some(Dato::Lista(xs)) = d {
                    hijos.extend(xs.iter().map(Option::as_ref));
                }
                offsets.push(hijos.len() as i32);
            }
            let valores = armar(h, &hijos)?;
            lista(tipo_arrow(h), offsets, valores, nulos)
        }
        Forma::Struct(cs) => {
            let nulos: Vec<bool> = datos.iter().map(Option::is_some).collect();
            let mut campos = Vec::with_capacity(cs.len());
            let mut arrays = Vec::with_capacity(cs.len());
            for (i, (n, h)) in cs.iter().enumerate() {
                let hijos: Vec<Option<&Dato>> = datos
                    .iter()
                    .map(|d| match d {
                        Some(Dato::Struct(xs)) => xs.get(i).and_then(Option::as_ref),
                        _ => None,
                    })
                    .collect();
                arrays.push(armar(h, &hijos)?);
                campos.push(Field::new(n, tipo_arrow(h), true));
            }
            let nulos = nulos.iter().any(|v| !v).then(|| NullBuffer::from(nulos));
            StructArray::try_new(Fields::from(campos), arrays, nulos)
                .map(|a| Arc::new(a) as ArrayRef)
                .map_err(|e| e.to_string())
        }
    }
}

pub(crate) fn lista(
    hijo: DataType,
    offsets: Vec<i32>,
    valores: ArrayRef,
    validos: Vec<bool>,
) -> Result<ArrayRef, String> {
    let nulos = validos
        .iter()
        .any(|v| !v)
        .then(|| NullBuffer::from(validos));
    ListArray::try_new(
        Arc::new(Field::new(ELEMENTO, hijo, true)),
        OffsetBuffer::new(offsets.into()),
        valores,
        nulos,
    )
    .map(|a| Arc::new(a) as ArrayRef)
    .map_err(|e| e.to_string())
}
