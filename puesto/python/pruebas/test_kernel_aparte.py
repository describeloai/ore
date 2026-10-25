"""Elastic Sessions S3a: el kernel en su proceso (docs/decisions/0073-elastic-sessions.md §2.2).

El código de la persona corre en un hijo del agente. Si ese hijo muere —en el
session pool, un OOM mata sólo al proceso que se pasa—, la celda vuelve con un
error que lo dice, y la siguiente corre en un kernel nuevo: la sesión sigue.
"""
import os
import sys
import unittest
from unittest import mock

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
import agente  # noqa: E402

MATARSE = "import os, signal; os.kill(os.getpid(), signal.SIGKILL)"


class KernelAparte(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.k = agente.KernelAparte()

    @classmethod
    def tearDownClass(cls):
        cls.k.proc.kill()

    def test_el_codigo_corre_en_otro_proceso(self):
        r = self.k.correr("import os; os.getpid()")
        self.assertEqual(r["tipo"], "texto", r)
        self.assertNotEqual(int(r["texto"]), os.getpid())
        self.assertEqual(int(r["texto"]), self.k.proc.pid)

    def test_las_variables_duran_la_sesion(self):
        self.assertEqual(self.k.correr("x = 21")["tipo"], "vacia")
        self.assertEqual(self.k.correr("x * 2")["texto"], "42")

    def test_lo_que_mide_viaja_con_la_salida(self):
        r = self.k.correr("y = sum(i for i in range(100_000))")
        self.assertIn("consumo", r)
        self.assertIn("memoria_pico_mb", r["consumo"])

    def test_si_el_kernel_muere_la_celda_lo_dice_y_la_sesion_sigue(self):
        self.k.correr("z = 1")
        antes = self.k.proc.pid
        r = self.k.correr(MATARSE)
        self.assertEqual(r["tipo"], "error", r)
        # Lo mata una señal, no la memoria: el contador del cgroup no se mueve.
        if agente._oom_kills() is not None:
            self.assertEqual(r["nombre"], "KernelDied", r)
        self.assertIn("variables are gone", r["mensaje"])
        self.assertNotEqual(self.k.proc.pid, antes)
        # Un kernel nuevo: sin las variables de antes, y sirve.
        self.assertEqual(self.k.correr("z")["nombre"], "NameError")
        self.assertEqual(self.k.correr("1 + 1")["texto"], "2")

    def test_si_fue_la_memoria_lo_dice(self):
        # El contador `oom_kill` del cgroup sube entre antes y después.
        with mock.patch.object(agente, "_oom_kills", side_effect=[3, 4]), \
                mock.patch.object(agente, "_memoria_del_pod_gib", return_value=4.0):
            r = self.k.correr(MATARSE)
        self.assertEqual(r["nombre"], "OutOfMemoryError", r)
        self.assertIn("ran out of memory", r["mensaje"])
        self.assertIn("4 GiB", r["mensaje"])
        self.assertEqual(self.k.correr("2 + 2")["texto"], "4")

    def test_los_planes_van_al_kernel(self):
        self.assertEqual(self.k.plan(""), {"error": "a plan needs a query"})

    def test_el_sql_tambien(self):
        r = self.k.correr("select 42 as respuesta", "sql")
        self.assertEqual(r["tipo"], "tabla", r)


if __name__ == "__main__":
    unittest.main()
