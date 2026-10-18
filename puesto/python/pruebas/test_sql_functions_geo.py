"""Las Functions del árbol desde SQL con geometría (ORE 0068 C1·1, pieza 1).

    bash pruebas-de-fuego/geo/c1/sdk-en-la-imagen.sh 'test_sql_functions_geo.py'

DuckDB 1.5 no recibe una GEOMETRY de vuelta de Python: la función se registra sobre
WKB y una macro con su nombre interno le pone el tipo y el CRS a la entrada y a la
salida. El contrato recibe shapely, como desde Python. Sin `spatial` o sin shapely,
se salta.
"""

import os
import unittest

import ore
from ore import contrato, sql_functions
from ore.tipos import Geometry

try:
    import duckdb
    import shapely

    _c = duckdb.connect()
    if os.path.isdir(ore.EXTENSIONES):
        _c.execute("set extension_directory = '%s'" % ore.EXTENSIONES)
    _c.execute("load spatial")
    LISTO = True
except Exception:
    LISTO = False


def buffer(g: Geometry["Point", "EPSG:25830"], metros: float) -> Geometry["Polygon", "EPSG:25830"]:
    return g.buffer(metros)


def area(g: Geometry["Any", "EPSG:25830"]) -> float:
    return g.area


def punto(x: float, y: float) -> Geometry["Point", "EPSG:4326"]:
    return shapely.Point(x, y)


def es_punto(g: Geometry) -> bool:
    return g.geom_type == "Point"


def doble(x: int) -> int:
    return 2 * x


SPECS = {
    "buffer": {"input": {"g": {"type": "Geometry<Point, EPSG:25830>"}, "metros": {"type": "Float"}},
               "output": {"type": "Geometry<Polygon, EPSG:25830>"}},
    "area": {"input": {"g": {"type": "Geometry<Any, EPSG:25830>"}}, "output": {"type": "Float"}},
    "punto": {"input": {"x": {"type": "Float"}, "y": {"type": "Float"}},
              "output": {"type": "Geometry<Point, EPSG:4326>"}},
    "es_punto": {"input": {"g": {"type": "Geography<Point>"}}, "output": {"type": "Boolean"}},
    "doble": {"input": {"x": {"type": "Integer"}}, "output": {"type": "Integer"}},
}
FUNCIONES = {"buffer": buffer, "area": area, "punto": punto, "es_punto": es_punto, "doble": doble}
UTM = "ST_SetCRS(ST_Point(440000, 4474000), 'EPSG:25830')"
WGS = "ST_SetCRS(ST_Point(-3.7, 40.4), 'EPSG:4326')"


@unittest.skipIf(not LISTO, "sin spatial o sin shapely")
class FuncionesGeoDesdeSql(unittest.TestCase):
    def setUp(self):
        self.con = duckdb.connect()
        if os.path.isdir(ore.EXTENSIONES):
            self.con.execute("set extension_directory = '%s'" % ore.EXTENSIONES)
        self.con.execute("load spatial")
        llamadas = [{"name": n, "internal": "__ore_fn_%d" % i, "arity": len(SPECS[n]["input"]), "table": False}
                    for i, n in enumerate(SPECS, 1)]
        sql_functions.register(self.con, llamadas, lambda n: (contrato.llamada(FUNCIONES[n]), SPECS[n]))

    def uno(self, sql):
        return self.con.sql(sql).fetchone()

    def test_entra_y_sale_con_su_crs(self):
        tipo, a = self.uno("select typeof(__ore_fn_1(%s, 100)), round(ST_Area(__ore_fn_1(%s, 100)))" % (UTM, UTM))
        self.assertEqual(tipo, "GEOMETRY('EPSG:25830')")
        self.assertEqual(a, 31365)

    def test_una_vista_la_describe_con_su_crs(self):
        filas = self.con.sql("describe select __ore_fn_1(%s, 10) as zona" % UTM).fetchall()
        self.assertEqual(filas[0][1], "GEOMETRY('EPSG:25830')")

    def test_otro_crs_se_niega_al_compilar(self):
        with self.assertRaisesRegex(duckdb.Error, "different CRS"):
            self.con.sql("select __ore_fn_1(%s, 100)" % WGS)

    def test_geo_dentro_y_un_numero_fuera(self):
        self.assertEqual(self.uno("select round(__ore_fn_2(ST_Buffer(%s, 10)))" % UTM)[0], 312)  # un círculo aproximado

    def test_un_numero_dentro_y_geo_fuera(self):
        tipo, x = self.uno("select typeof(__ore_fn_3(-3.7, 40.4)), ST_X(__ore_fn_3(-3.7, 40.4))")
        self.assertEqual((tipo, x), ("GEOMETRY('EPSG:4326')", -3.7))

    def test_geography_va_en_crs84(self):
        self.assertTrue(self.uno("select __ore_fn_4(ST_SetCRS(ST_Point(1, 2), 'OGC:CRS84'))")[0])

    def test_null_es_null(self):
        self.assertIsNone(self.uno("select __ore_fn_1(null::GEOMETRY('EPSG:25830'), 100)")[0])

    def test_sin_geo_como_siempre(self):
        self.assertEqual(self.uno("select __ore_fn_5(21)")[0], 42)
        macros = self.con.sql("select function_name from duckdb_functions() where function_type = 'macro' "
                              "and function_name = '__ore_fn_5'").fetchall()
        self.assertEqual(macros, [])

    def test_en_filas_no_todavia(self):
        with self.assertRaisesRegex(TypeError, "inside a `Struct` or a `list` yet"):
            sql_functions.register(self.con, [{"name": "punto", "internal": "__ore_fn_9", "arity": 2, "table": True}],
                                   lambda n: (contrato.llamada(FUNCIONES[n]), SPECS[n]))


if __name__ == "__main__":
    unittest.main()
