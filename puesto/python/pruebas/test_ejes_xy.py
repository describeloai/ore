"""ORE 0068 (cierre de C1) · x es la longitud, siempre: lo que guarda el lago (GeoParquet, WKB) y lo
que hace Sedona (Heavyweight). El SDK pone `geometry_always_xy` al cargar `spatial`; sin él, DuckDB
toma el orden de la autoridad EPSG (en 4326, lat/lon). Medido en `pruebas-de-fuego/geo/c1/ejes/`.
Sin `spatial` (fuera de la imagen y sin red), se salta."""

import unittest

import ore


def _hay_spatial():
    try:
        ore._cargar(ore._duckdb(), "spatial")
        return True
    except Exception:  # noqa: BLE001
        return False


@unittest.skipUnless(_hay_spatial(), "sin spatial")
class LosEjes(unittest.TestCase):
    def test_a_4326_sale_lon_lat(self):
        p = ore.sql("select ST_AsText(ST_Transform(ST_Point(440000, 4474000), 'EPSG:25830', 'EPSG:4326')) p").iloc[0, 0]
        self.assertTrue(p.startswith("POINT (-3.707"), p)

    def test_desde_un_lon_lat_del_lago(self):
        # Un punto del lago en 4326 (x = longitud) a 25830: la Puerta del Sol, no el Atlántico.
        x = ore.sql("select ST_X(ST_Transform(ST_Point(-3.7072, 40.4145), 'EPSG:4326', 'EPSG:25830')) x").iloc[0, 0]
        self.assertAlmostEqual(x, 440000, delta=5)

    def test_la_distancia_esferica_es_la_de_verdad(self):
        d = ore.sql("select ST_Distance_Spheroid(ST_Point(-3.7072, 40.4145), ST_Point(2.1734, 41.3851)) / 1000 d").iloc[0, 0]
        self.assertAlmostEqual(d, 507, delta=2)  # sin el ajuste, 659 km


if __name__ == "__main__":
    unittest.main()
