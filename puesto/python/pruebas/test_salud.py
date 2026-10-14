"""La salud de lo que se lee, dicha al leer (OOS v1alpha30, ORE 0063 S2).

    PYTHONPATH=puesto/python python -m unittest discover -s puesto/python/pruebas

Lo medido en 0063 M5: con la última pasada fallida, `sql()` decía «la copia no
está hecha» y no leía nada, aunque la versión buena seguía en el lago. Ahora
ore-serve la sirve con su bloque de salud, y esto es lo que el SDK hace con él.
"""

import unittest
import warnings

from ore import DegradedDataError, HealthWarning, _avisar_salud, _con_salud, _salud_de

DEGRADADA = {
    "state": "degraded",
    "dataAsOf": "2026-10-13T13:37:02Z",
    "servingLastGood": True,
    "reasons": [{"code": "refresh-failed", "since": "2026-10-13T13:44:51Z",
                 "message": "the source has no column `cliente`"}],
    "layers": {"monitoring": False, "profiling": False},
}
DESCONOCIDA = {"state": "unknown", "dataAsOf": None, "reasons": [{"code": "unverified", "since": "x"}],
               "layers": {"monitoring": False, "profiling": False}}


class Salud(unittest.TestCase):
    def test_lo_degradado_se_lee_y_se_avisa(self):
        saludes = {}
        _salud_de(saludes, "pg_test_s.ore_salud.pedidos", {"salud": DEGRADADA})
        with warnings.catch_warnings(record=True) as w:
            _avisar_salud(saludes, False)
        self.assertEqual(len(w), 1)
        self.assertIs(w[0].category, HealthWarning)
        m = str(w[0].message)
        for trozo in ("degraded", "2026-10-13T13:37:02Z", "the last refresh failed", "cliente", "last good version"):
            self.assertIn(trozo, m)

    def test_estricto_no_lee_lo_degradado(self):
        with self.assertRaises(DegradedDataError):
            _avisar_salud({"a.b": DEGRADADA}, True)

    def test_lo_desconocido_no_avisa_y_queda_en_attrs(self):
        with warnings.catch_warnings(record=True) as w:
            _avisar_salud({"a.b": DESCONOCIDA}, True)
        self.assertEqual(w, [])

        class Df:
            attrs = {}

        d = _con_salud(Df(), {"a.b": DESCONOCIDA})
        self.assertEqual(d.attrs["health"]["a.b"]["state"], "unknown")

    def test_sin_bloque_no_pasa_nada(self):
        saludes = {}
        _salud_de(saludes, "a.b", {"metadata_location": "m"})
        self.assertEqual(saludes, {})



class HoraDeLoLeido(unittest.TestCase):
    """0063 S6b: lo escrito no es más nuevo que lo leído. La procedencia lleva la hora de
    cada cosa leída, la más vieja si se leyó varias veces; lo que no se sabe, manda."""

    def setUp(self):
        import ore
        ore._datos_de.clear()
        ore._leidas.clear()

    def test_la_procedencia_lleva_la_hora_de_lo_leido(self):
        import ore
        ore._leidas.extend(["a.x", "a.y", "a.salida"])
        _salud_de({}, "a.x", {"salud": DEGRADADA})
        _salud_de({}, "a.x", {"salud": dict(DEGRADADA, dataAsOf="2026-10-14T09:00:00Z")})
        _salud_de({}, "a.y", {"salud": DESCONOCIDA})
        p = ore._procedencia("a.salida")
        self.assertEqual(p["leidas"], ["a.x", "a.y"])
        self.assertEqual(p["datos_de"], {"a.x": "2026-10-13T13:37:02Z", "a.y": None})

    def test_lo_desconocido_manda(self):
        import ore
        ore._anota_hora("a.x", None)
        ore._anota_hora("a.x", "2026-10-14T09:00:00Z")
        self.assertIsNone(ore._datos_de["a.x"])


if __name__ == "__main__":
    unittest.main()
