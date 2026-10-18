"""H3 en Python (ORE 0068 C1·2, pieza 1).

    bash pruebas-de-fuego/geo/c1/sdk-en-la-imagen.sh 'test_h3.py'

La imagen del puesto trae `h3` (4.5.0, en `provisto.txt`): lo que `h3_*` es en SQL,
desde Python. La misma celda en los dos para el mismo punto y la misma resolución.
Sin `h3` (fuera de la imagen), se salta.
"""

import os
import unittest

import ore

try:
    import h3
except ImportError:
    h3 = None

PUNTOS = [(40.4168, -3.7038), (41.3874, 2.1686), (28.4636, -16.2518), (55.6761, 12.5683)]


@unittest.skipIf(h3 is None, "sin h3")
class H3EnPython(unittest.TestCase):
    def test_la_misma_celda_que_en_sql(self):
        import duckdb

        c = duckdb.connect()
        if os.path.isdir(ore.EXTENSIONES):
            c.execute("set extension_directory = '%s'" % ore.EXTENSIONES)
        c.execute("load h3")
        for lat, lng in PUNTOS:
            for res in (5, 9, 12):
                sql = c.execute("select h3_h3_to_string(h3_latlng_to_cell(?, ?, ?))", [lat, lng, res]).fetchone()[0]
                self.assertEqual(h3.latlng_to_cell(lat, lng, res), sql, (lat, lng, res))

    def test_vecinos_y_contorno(self):
        celda = h3.latlng_to_cell(40.4168, -3.7038, 9)
        self.assertEqual(len(h3.grid_disk(celda, 1)), 7)
        self.assertEqual(len(h3.cell_to_boundary(celda)), 6)


if __name__ == "__main__":
    unittest.main()
