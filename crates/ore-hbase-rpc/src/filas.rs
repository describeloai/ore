//! **Un filtro para pedir filas concretas** (ficha 0072, N5): `MultiRowRangeFilter`, de HBase 2.x, con un rango de una
//! sola fila por clave. El servidor salta de una a otra sin leer lo que hay entre medias, asi que traer las filas
//! completas de las claves que cambiaron es **un solo `Scan`** (con sus reintentos, su reanudacion y su cancelacion) y
//! no un `Get` por clave.
//!
//! El mensaje es el de `Filter.proto` (`MultiRowRangeFilter { repeated RowRange row_range_list = 1; }`); son cuatro
//! campos y no se genera el fichero entero para ellos.

use prost::Message;

/// La clase Java del filtro, tal como va en `Filter.name`.
pub const MULTI_RANGO_DE_FILAS: &str = "org.apache.hadoop.hbase.filter.MultiRowRangeFilter";

#[derive(Clone, PartialEq, Message)]
pub struct RangoDeFilas {
    #[prost(bytes = "vec", optional, tag = "1")]
    pub start_row: Option<Vec<u8>>,
    #[prost(bool, optional, tag = "2")]
    pub start_row_inclusive: Option<bool>,
    #[prost(bytes = "vec", optional, tag = "3")]
    pub stop_row: Option<Vec<u8>>,
    #[prost(bool, optional, tag = "4")]
    pub stop_row_inclusive: Option<bool>,
}

#[derive(Clone, PartialEq, Message)]
pub struct MultiRangoDeFilas {
    #[prost(message, repeated, tag = "1")]
    pub row_range_list: Vec<RangoDeFilas>,
}

/// El filtro que deja pasar **solo** las filas con esas claves, como `(clase, mensaje)` para `Consulta::filtro`.
/// Las claves pueden venir en cualquier orden; el servidor las ordena y las une.
pub fn filtro_de_filas(claves: &[Vec<u8>]) -> (String, Vec<u8>) {
    let m = MultiRangoDeFilas {
        row_range_list: claves
            .iter()
            .map(|k| RangoDeFilas {
                start_row: Some(k.clone()),
                start_row_inclusive: Some(true),
                stop_row: Some(k.clone()),
                stop_row_inclusive: Some(true),
            })
            .collect(),
    };
    (MULTI_RANGO_DE_FILAS.to_string(), m.encode_to_vec())
}

/// Las claves que un filtro de filas deja pasar (para un servidor simulado o una prueba).
pub fn claves_del_filtro(datos: &[u8]) -> Option<Vec<Vec<u8>>> {
    let m = MultiRangoDeFilas::decode(datos).ok()?;
    m.row_range_list
        .into_iter()
        .map(|r| match (r.start_row, r.stop_row) {
            (Some(a), Some(b)) if a == b => Some(a),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_filtro_va_y_vuelve_con_las_claves() {
        let claves = vec![b"a".to_vec(), vec![0xff, 0x00], Vec::new()];
        let (clase, datos) = filtro_de_filas(&claves);
        assert!(clase.ends_with("MultiRowRangeFilter"));
        assert_eq!(claves_del_filtro(&datos), Some(claves));
    }

    #[test]
    fn los_bytes_son_los_del_protocolo() {
        // RowRange{start="a",true,stop="a",true}: 0a 01 61 10 01 1a 01 61 20 01, dentro de un campo 1 de 10 bytes
        let (_, datos) = filtro_de_filas(&[b"a".to_vec()]);
        assert_eq!(
            datos,
            [
                0x0a, 0x0a, 0x0a, 0x01, 0x61, 0x10, 0x01, 0x1a, 0x01, 0x61, 0x20, 0x01
            ]
        );
    }
}
