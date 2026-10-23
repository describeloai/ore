"""El tope de disco de la sesión (Query Q1, `docs/query.md` §2.4).

    PYTHONPATH=puesto/python python -m unittest discover -s puesto/python/pruebas

Medido en victor el 2026-10-23: sin tope, una ventana sobre 600 M filas derramó 10 GiB
en una sesión y puso el nodo del sistema en `DiskPressure`. Aquí: que DuckDB derrama
hasta la mitad de `ORE_DISCO_MB` (la plantilla lo pone junto al `sizeLimit` de
`/trabajo`), que sin la variable manda el tope de DuckDB, y que una consulta que no
cabe falla con un error que lo dice en vez de llenar el disco.
"""

import os
import unittest
from unittest import mock

import duckdb

import ore


def _limpia():
    if ore._con is not None:
        ore._con.close()
    ore._con = None


class ElDiscoDeLaSesion(unittest.TestCase):
    def setUp(self):
        _limpia()

    def tearDown(self):
        _limpia()

    def test_derrama_la_mitad_de_lo_que_dice_el_pod(self):
        for dicho, mb in (("4096", 2048), ("245760", 122880), ("", 0), ("no", 0), ("0", 0)):
            with mock.patch.dict(os.environ, {"ORE_DISCO_MB": dicho}):
                self.assertEqual(ore._derrame_mb(), mb, dicho)

    def test_con_la_cifra_duckdb_lleva_el_tope(self):
        with mock.patch.dict(os.environ, {"ORE_DISCO_MB": "4096", "ORE_MEMORIA_MB": "4096"}):
            puesto = ore._duckdb().execute("select current_setting('max_temp_directory_size')").fetchone()[0]
        c = duckdb.connect()
        c.execute("set max_temp_directory_size='2048MB'")
        self.assertEqual(puesto, c.execute("select current_setting('max_temp_directory_size')").fetchone()[0])

    def test_sin_la_cifra_manda_duckdb(self):
        env = {k: v for k, v in os.environ.items() if k != "ORE_DISCO_MB"}
        with mock.patch.dict(os.environ, env, clear=True):
            puesto = ore._duckdb().execute("select current_setting('max_temp_directory_size')").fetchone()[0]
        self.assertEqual(puesto, duckdb.connect().execute("select current_setting('max_temp_directory_size')").fetchone()[0])

    def test_lo_que_no_cabe_falla_y_lo_dice(self):
        # 256 MB de memoria (el suelo) y 2 MB de derrame: 30 M distintos al azar no caben.
        # (Un `order by … limit` no sirve: es un top-N y no ordena todo.)
        with mock.patch.dict(os.environ, {"ORE_DISCO_MB": "4", "ORE_MEMORIA_MB": "256", "ORE_HILOS": "2"}):
            with self.assertRaises(Exception) as e:
                ore.sql("select count(distinct x) from (select random() x from range(30000000))")
        self.assertIn("needs more than this session holds", str(e.exception))
        self.assertIn("the 2 MB this session allows", str(e.exception))

    def test_otro_error_sube_tal_cual(self):
        self.assertIsNone(ore._sin_disco(RuntimeError("Binder Error: no existe la columna")))


if __name__ == "__main__":
    unittest.main()
