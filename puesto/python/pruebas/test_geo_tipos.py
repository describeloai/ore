"""Los tipos geo en las firmas y el filtro por zona (ORE 0067 G2·3·6 y G2·3·7).

    PYTHONPATH=puesto/python python -m unittest discover -s puesto/python/pruebas
"""

import unittest

try:
    import shapely
except ImportError:
    shapely = None

import ore
from ore.contrato import ContractError, llamada
from ore.tipos import Geography, Geometry

HEX = "0101000000000000000000F03F0000000000000040"  # POINT (1 2)


class Firmas(unittest.TestCase):
    def test_la_anotacion_es_el_tipo_de_oos(self):
        self.assertEqual(repr(Geometry["Point", "EPSG:25830"].__metadata__[0]), "Geometry<Point, EPSG:25830>")
        self.assertEqual(repr(Geometry["Point"].__metadata__[0]), "Geometry<Point>")
        self.assertEqual(repr(Geometry["Any"].__metadata__[0]), "Geometry")
        self.assertEqual(repr(Geography["LineString"].__metadata__[0]), "Geography<LineString>")
        with self.assertRaises(TypeError):
            Geometry[3]

    @unittest.skipIf(shapely is None, "sin shapely")
    def test_el_contrato_da_una_geometria(self):
        @llamada
        def x_de(p: Geometry["Point", "EPSG:25830"]) -> float:
            return p.x

        self.assertEqual(x_de(HEX), 1.0)
        self.assertEqual(x_de(bytes.fromhex(HEX)), 1.0)
        self.assertEqual(x_de(shapely.Point(1, 2)), 1.0)
        with self.assertRaises(ContractError):
            x_de("POINT(1 2)")
        with self.assertRaises(ContractError):
            x_de(42)


class IntersectsBox(unittest.TestCase):
    def test_el_filtro_lleva_los_dos(self):
        f = ore.geo.intersects_box("t.geom", -3.75, 40.38, -3.65, 40.46)
        self.assertEqual(
            f, '("t"."geom" && ST_MakeEnvelope(-3.75, 40.38, -3.65, 40.46) AND ST_Intersects("t"."geom", ST_MakeEnvelope(-3.75, 40.38, -3.65, 40.46)))')
        with self.assertRaises(TypeError):
            ore.geo.intersects_box("geom", "1", 0, 2, 2)
        with self.assertRaises(ValueError):
            ore.geo.intersects_box("geom", 2, 0, 1, 2)

    def test_filtra_en_duckdb(self):
        con = ore._duckdb()
        try:
            ore._cargar(con, "spatial")
        except Exception as e:
            self.skipTest("sin spatial: %s" % e)
        n = con.sql("select count(*) from (select ST_Point(1, 2) geom union all select ST_Point(10, 20)) where %s"
                    % ore.geo.intersects_box("geom", 0, 0, 5, 5)).fetchone()[0]
        self.assertEqual(n, 1)


if __name__ == "__main__":
    unittest.main()
