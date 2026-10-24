"""`system.default.executions` desde `sql()` (docs/ejecuciones.md X2).

    PYTHONPATH=puesto/python python -m unittest discover -s puesto/python/pruebas

`system` es un catálogo de DuckDB, de solo lectura: el SDK registra el de ORE
como `__ore_system`, y ore-serve le devuelve la sentencia reescrita (`query`).
"""

import unittest

import duckdb

import ore


class System(unittest.TestCase):
    def test_system_se_registra_por_su_catalogo(self):
        con = duckdb.connect()
        ore._registra(con, "system.executions", "(select 'query' as kind)")
        self.assertEqual(con.execute("select kind from __ore_system.default.executions").fetchall(), [("query",)])
        self.assertEqual(con.execute("select kind from __ore_system.executions").fetchall(), [("query",)])

    def test_las_demas_bases_como_siempre(self):
        con = duckdb.connect()
        ore._registra(con, "ventas.pedidos", "(select 1 as id)")
        self.assertEqual(con.execute("select id from ventas.default.pedidos").fetchall(), [(1,)])


if __name__ == "__main__":
    unittest.main()
