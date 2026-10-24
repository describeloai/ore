"""Lo que gasta una celda (docs/ejecuciones.md X3).

    PYTHONPATH=puesto/python python -m unittest discover -s puesto/python/pruebas

El agente mide cada celda —CPU, memoria residente más alta, derrame de DuckDB—
y lo manda en su salida (`consumo`): `ore-serve` lo lleva a
`system.default.executions`.
"""

import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import agente  # noqa: E402


class ElConsumo(unittest.TestCase):
    def test_cada_celda_dice_lo_que_gasto(self):
        r = agente.Kernel().correr("x = sum(i * i for i in range(2_000_000))")
        c = r["consumo"]
        self.assertEqual(sorted(c), ["cpu_ms", "derrame_mb", "memoria_pico_mb"])
        self.assertGreater(c["cpu_ms"], 0)
        self.assertGreaterEqual(c["derrame_mb"], 0)
        if os.path.exists("/proc/self/statm"):
            self.assertGreater(c["memoria_pico_mb"], 0)

    def test_una_celda_que_falla_tambien(self):
        r = agente.Kernel().correr("1 / 0")
        self.assertEqual(r["tipo"], "error")
        self.assertIn("cpu_ms", r["consumo"])


if __name__ == "__main__":
    unittest.main()
