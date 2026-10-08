"""Las salidas de una celda, S1 · `json`: un valor compuesto sale como árbol.

`Kernel.salida_de` decide por el último valor de la celda: tabla, `json`,
texto o vacía; `como_json` lo convierte con sus topes.
"""
import dataclasses
import datetime
import decimal
import json
import os
import sys
import time
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import agente  # noqa: E402
from ore.medios import MediaRef  # noqa: E402


@dataclasses.dataclass
class Ancla:
    kind: str
    page: int


def salida(valor, texto=""):
    return agente.Kernel.salida_de(valor, texto, time.time())


class Json(unittest.TestCase):
    def test_un_dict_es_json_con_lo_impreso_debajo(self):
        s = salida({"a": 1, "b": [1, 2], "c": None}, "impreso\n")
        self.assertEqual((s["tipo"], s["valor"], s["texto"], s["recortado"]),
                         ("json", {"a": 1, "b": [1, 2], "c": None}, "impreso\n", False))

    def test_listas_tuplas_conjuntos_y_dataclasses(self):
        self.assertEqual(salida([1, (2, 3)])["valor"], [1, [2, 3]])
        self.assertEqual(salida({3, 1, 2})["valor"], [1, 2, 3])
        self.assertEqual(salida(Ancla("page", 1))["valor"], {"kind": "page", "page": 1})
        self.assertEqual(salida({"anchor": Ancla("page", 2)})["valor"], {"anchor": {"kind": "page", "page": 2}})

    def test_un_mediaref_y_lo_que_devuelve_el_sdk(self):
        r = MediaRef(uri="ore://a.b.c/x.pdf?v=1", collection="a.b.c", path="x.pdf", version="1", size=717)
        v = salida(r)["valor"]
        self.assertEqual((v["path"], v["size"], v["digest"]), ("x.pdf", 717, None))
        from ore import _Result
        self.assertEqual(salida(_Result({"items": 4, "written": False}))["valor"], {"items": 4, "written": False})

    def test_las_hojas_van_como_el_contrato(self):
        v = salida({"d": decimal.Decimal("1.50"), "f": datetime.date(2026, 10, 8),
                    "t": datetime.datetime(2026, 10, 8, 12, 0, tzinfo=datetime.timezone.utc),
                    "grande": 2 ** 60, "nan": float("nan"), "b": b"\x89PNG" * 10})["valor"]
        self.assertEqual(v, {"d": "1.50", "f": "2026-10-08", "t": "2026-10-08T12:00:00Z",
                             "grande": str(2 ** 60), "nan": "NaN", "b": "bytes · 40 B"})
        json.dumps(v)   # y es JSON

    def test_un_objeto_que_no_es_json_va_por_su_repr(self):
        class Raro:
            def __repr__(self):
                return "<Raro>"
        self.assertEqual(salida({"x": Raro()})["valor"], {"x": "<Raro>"})

    def test_los_topes_recortan_y_lo_dicen(self):
        s = salida(list(range(agente.JSON_POR_NIVEL + 7)))
        self.assertTrue(s["recortado"])
        self.assertEqual(s["valor"][-1], "… 7 more items")
        self.assertEqual(len(s["valor"]), agente.JSON_POR_NIVEL + 1)
        d = salida({str(i): i for i in range(agente.JSON_POR_NIVEL + 2)})["valor"]
        self.assertEqual(d["…"], "2 more keys")
        largo = salida({"s": "x" * (agente.JSON_CADENA + 3)})
        self.assertTrue(largo["recortado"] and largo["valor"]["s"].endswith("(3 more characters)"))
        hondo = []
        for _ in range(agente.JSON_HONDO + 5):
            hondo = [hondo]
        self.assertTrue(salida(hondo)["recortado"])

    def test_lo_que_no_es_compuesto_sigue_como_antes(self):
        self.assertEqual(salida(2)["tipo"], "texto")
        self.assertEqual(salida(2)["texto"], "2")
        self.assertEqual(salida("hola")["texto"], "'hola'")
        self.assertEqual(salida(None, "impreso")["tipo"], "texto")
        self.assertEqual(salida(None)["tipo"], "vacia")
        self.assertEqual(salida(Ancla)["tipo"], "texto")   # la clase, no una instancia

    def test_una_tabla_sigue_siendo_tabla(self):
        import pyarrow as pa
        s = salida(pa.table({"a": [1, 2]}))
        self.assertEqual((s["tipo"], s["total"]), ("tabla", 2))

    def test_lo_que_no_cabe_ni_recortado_sale_como_texto(self):
        viejo = agente.JSON_BYTES
        agente.JSON_BYTES = 100
        try:
            self.assertEqual(salida({"a": "x" * 500})["tipo"], "texto")
        finally:
            agente.JSON_BYTES = viejo

    def test_la_celda_entera_por_el_kernel(self):
        k = agente.Kernel.__new__(agente.Kernel)
        k.espacio = {}
        s = k._correr("import dataclasses\nprint('hola')\n{'n': 1, 'l': [1, 2]}")
        self.assertEqual((s["tipo"], s["valor"], s["texto"]), ("json", {"n": 1, "l": [1, 2]}, "hola\n"))


if __name__ == "__main__":
    unittest.main()
