"""Lo que nunca es nulo, en el esquema de Arrow que `over()` devuelve (ORE 0051 P7).

    PYTHONPATH=puesto/python python -m unittest discover -s puesto/python/pruebas

DuckDB no mira el `required` de Iceberg: sin esto, una columna que el origen
garantiza salía nulable en el esquema que recibe el código del puesto.
"""

import unittest
import warnings

try:
    import pyarrow as pa
except ImportError:  # el puesto lo trae; una máquina sin él no prueba esto
    pa = None

from ore import _nunca_nulas


@unittest.skipIf(pa is None, "sin pyarrow")
class NuncaNulas(unittest.TestCase):
    def tabla(self, ids):
        return pa.table({"id": pa.array(ids, pa.int64()), "nota": pa.array(["a"] * len(ids))})

    def test_lo_que_el_arbol_garantiza_sale_no_nulable(self):
        t = _nunca_nulas(self.tabla([1, 2]), {"nunca_nulas": ["id"]}, "ventas.pedidos")
        self.assertFalse(t.schema.field("id").nullable)
        self.assertTrue(t.schema.field("nota").nullable)
        self.assertEqual(t.column("id").to_pylist(), [1, 2])

    def test_sin_nunca_nulas_la_tabla_es_la_misma(self):
        t = self.tabla([1])
        self.assertIs(_nunca_nulas(t, {}, "ventas.pedidos"), t)

    def test_un_nulo_no_se_marca_y_se_avisa(self):
        with warnings.catch_warnings(record=True) as w:
            warnings.simplefilter("always")
            t = _nunca_nulas(self.tabla([1, None]), {"nunca_nulas": ["id"]}, "ventas.pedidos")
        self.assertTrue(t.schema.field("id").nullable)
        self.assertEqual(len(w), 1)
        self.assertIn("ventas.pedidos.id", str(w[0].message))


if __name__ == "__main__":
    unittest.main()
