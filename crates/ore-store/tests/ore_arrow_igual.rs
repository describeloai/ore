//! **`ore-arrow` y `carga` dicen lo mismo** (ORE 0075, fase 1).
//!
//! El mapa de un tipo de OOS a Arrow está dos veces hasta que la fase 2 borre la copia de `carga`. Esta prueba
//! es lo que permite borrarla: para cada tipo, el campo y el tipo de Arrow son idénticos, metadatos incluidos.

use ore_base::tipos::Forma;
use ore_store::carga;

const TIPOS: &[&str] = &[
    "String",
    "Integer",
    "Float",
    "Boolean",
    "Date",
    "Time",
    "DateTime",
    "DateTimeTz",
    "Opaque",
    "Binary",
    "Decimal",
    "Decimal<12, 2>",
    "Money<EUR, 2>",
    "Quantity<kg, 3>",
    "Vector<3>",
    "list<Integer>",
    "list<Binary>",
    "Struct<a: Integer, b: String>",
    "Struct<a: list<Decimal>, b: Boolean>",
    "list<Struct<k: Integer, v: list<String>>>",
    "Geometry",
    "Geometry<Point, EPSG:25830>",
    "Geometry<MultiPolygon Z, EPSG:4326>",
    "Geography<Point>",
    "Geography<LineString>",
    "list<Geometry<Point, EPSG:25830>>",
    "Struct<g: Geometry<Point, EPSG:25830>, v: Vector<3>>",
];

#[test]
fn el_campo_y_el_tipo_de_cada_forma_son_los_de_carga() {
    assert_eq!(ore_arrow::ELEMENTO, carga::ELEMENTO);
    assert_eq!(ore_arrow::UTC, carga::UTC);
    assert_eq!(ore_arrow::META_TIPO, carga::META_TIPO);
    assert_eq!(ore_arrow::GEOARROW_WKB, carga::GEOARROW_WKB);
    let mal: Vec<String> = TIPOS
        .iter()
        .filter_map(|t| {
            ore_base::types::parse_type(t)
                .err()
                .map(|e| format!("`{t}`: {e:?}"))
        })
        .collect();
    assert!(
        mal.is_empty(),
        "tipos de la batería que OOS no lee: {mal:#?}"
    );
    for t in TIPOS {
        let tipo = ore_base::types::parse_type(t).unwrap();
        let forma = Forma::de(&tipo);
        assert_eq!(
            ore_arrow::tipo_arrow(&forma),
            carga::arrow_de_forma(&forma),
            "el tipo de `{t}`"
        );
        assert_eq!(
            ore_arrow::campo("c", &forma),
            carga::campo("c", &forma),
            "el campo de `{t}`"
        );
    }
}
