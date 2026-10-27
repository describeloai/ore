"""Elastic Sessions S3b: crecer solo (docs/products/0073-elastic-sessions.md §2.3).

El reparto del motor sale de lo que mide el pod AHORA (su cgroup), el 60 % o el
45 % con una operación reservada; el vigilante pide crecer cuando el motor se
llena o derrama y lo aplica en caliente; y una sesión que creció vuelve a su base
tras un rato sin celdas.
"""
import os
import sys
import unittest
from unittest import mock

import duckdb

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
import agente  # noqa: E402
import ore  # noqa: E402


def cgroup(memoria=None, cpu=None):
    """`ore._del_cgroup` como si el pod midiera esto."""
    return lambda f: {"memory.max": memoria, "cpu.max": cpu}.get(f)


def limite_mb(con):
    """El `memory_limit` del motor, en los MB que se le dijeron (DuckDB entiende
    `MB` como 10^6 bytes y lo enseña en MiB o GiB, redondeado)."""
    v = con.execute("select current_setting('memory_limit')").fetchone()[0]
    n, unidad = v.split()
    return float(n) * {"KiB": 2**10, "MiB": 2**20, "GiB": 2**30, "MB": 1e6, "GB": 1e9}[unidad] / 1e6


def cerca(test, mb, esperado):
    test.assertLess(abs(mb - esperado) / esperado, 0.05, (mb, esperado))


class ElReparto(unittest.TestCase):
    def test_sale_del_cgroup_y_no_de_lo_que_dijo_la_plantilla(self):
        with mock.patch.dict(os.environ, {"ORE_MEMORIA_MB": "4096"}), \
                mock.patch.object(ore, "_del_cgroup", cgroup(str(8 * 2**30))):
            self.assertEqual(ore._pod_mb(), 8192)
            self.assertEqual(ore._tropo_mb(), 8192 * 60 // 100)
            self.assertEqual(ore._tropo_mb(reservada=True), 8192 * 45 // 100)
        # Sin tope en el cgroup (un portátil, Docker sin límite): la plantilla.
        with mock.patch.dict(os.environ, {"ORE_MEMORIA_MB": "4096"}), \
                mock.patch.object(ore, "_del_cgroup", cgroup("max")):
            self.assertEqual(ore._pod_mb(), 4096)
            self.assertEqual(ore._tropo_mb(), 2457)
        with mock.patch.dict(os.environ, {"ORE_MEMORIA_MB": ""}), \
                mock.patch.object(ore, "_del_cgroup", cgroup()):
            self.assertEqual(ore._tropo_mb(), 512)

    def test_los_nucleos_del_cgroup(self):
        for cpu, n in (("400000 100000", 4), ("150000 100000", 2), ("max 100000", 0), (None, 0)):
            with mock.patch.object(ore, "_del_cgroup", cgroup(cpu=cpu)):
                self.assertEqual(ore._pod_cpus(), n, cpu)

    def test_las_reservadas_por_su_nombre(self):
        for sql in ("select k, mode(v) from t group by k", "select MEDIAN (x) from t",
                    "select string_agg(s, ',') from t", "select approx_quantile(v, 0.9) from t",
                    "select k, list(v) from t group by k", "select histogram(s) from t"):
            self.assertTrue(ore._RESERVADAS.search(sql), sql)
        for sql in ("select model(x) from t", "select * from listing", "select modes from t", "select 1"):
            self.assertFalse(ore._RESERVADAS.search(sql), sql)

    def test_ajustar_lo_dice_al_motor_y_solo_si_cambia(self):
        con = duckdb.connect()
        ore._aplicado.update(mb=None, hilos=None)
        with mock.patch.dict(os.environ, {"ORE_HILOS": ""}), \
                mock.patch.object(ore, "_del_cgroup", cgroup(str(8 * 2**30), "400000 100000")):
            self.assertEqual(ore._ajustar(con), 4915)
            cerca(self, limite_mb(con), 4915)
            self.assertEqual(con.execute("select current_setting('threads')").fetchone()[0], 4)
            self.assertEqual(ore._ajustar(con, "select mode(v) from t"), 3686)
            cerca(self, limite_mb(con), 3686)
        ore._aplicado.update(mb=None, hilos=None)


class ElVigilante(unittest.TestCase):
    def setUp(self):
        agente.Vigilante.sin_pool = False
        self.v = agente.Vigilante("select 1")
        self.v._otra = duckdb.connect()
        ore._aplicado.update(mb=None, hilos=None)

    def tearDown(self):
        agente.Vigilante.sin_pool = False
        ore._aplicado.update(mb=None, hilos=None)

    def crecer(self, respuesta, pods=(4096, 8192)):
        pedido = []

        def pedir(metodo, ruta, cuerpo=None, **_):
            pedido.append((metodo, ruta, cuerpo))
            return respuesta

        medidas = iter(pods)
        ultimo = [pods[0]]

        def pod_mb():
            ultimo[0] = next(medidas, ultimo[0])
            return ultimo[0]

        with mock.patch.object(ore.session, "id", "puesto-ana-python"), \
                mock.patch.object(ore.session, "pedir", pedir), \
                mock.patch.object(ore, "_pod_mb", pod_mb):
            ok = self.v._crecer("spilling")
        return ok, pedido

    def test_pide_el_doble_y_aplica_lo_nuevo(self):
        ok, pedido = self.crecer((200, {"estado": "hecho", "escalon": 1, "memoria_mb": 8192}))
        self.assertTrue(ok)
        self.assertEqual(pedido, [("POST", "/puestos/puesto-ana-python/scale",
                                   {"memoria_mb": 8192, "motivo": "spilling"})])
        # El motor, al 60 % del pod nuevo.
        cerca(self, limite_mb(self.v._otra), 8192 * 60 // 100)

    def test_si_no_cabe_deja_de_pedir_y_la_consulta_sigue(self):
        for estado in ("quota", "infeasible", "deferred", "igual"):
            ok, _ = self.crecer((200, {"estado": estado}))
            self.assertFalse(ok, estado)
        self.assertFalse(agente.Vigilante.sin_pool)

    def test_fuera_del_session_pool_no_vuelve_a_pedir(self):
        ok, _ = self.crecer((409, {"error": "esta sesión no está en el session pool: no crece"}))
        self.assertFalse(ok)
        self.assertTrue(agente.Vigilante.sin_pool)
        with mock.patch.object(ore.session, "id", "puesto-ana-python"):
            with agente.Vigilante() as v:
                self.assertIsNone(v._hilo)

    def test_una_consulta_que_derrama_le_hace_pedir(self):
        """Con el motor pequeño (256 MB), 30 M claves distintas derraman: el
        vigilante lo ve y pide crecer. `ore-serve` dice `quota`, y la consulta
        termina igual (más lenta, derramando)."""
        pedidos = []

        def pedir(metodo, ruta, cuerpo=None, **_):
            pedidos.append(cuerpo)
            return 200, {"estado": "quota"}

        ore._con = None
        with mock.patch.dict(os.environ, {"ORE_MEMORIA_MB": "256", "ORE_HILOS": "2", "ORE_DISCO_MB": ""}), \
                mock.patch.object(ore, "_del_cgroup", cgroup("max")), \
                mock.patch.object(ore.session, "id", "puesto-ana-python"), \
                mock.patch.object(ore.session, "pedir", pedir):
            con = ore._duckdb()
            with agente.Vigilante("select count(distinct x)") as v:
                n = con.execute("select count(distinct x) from (select random() x from range(30000000))").fetchone()[0]
        ore._con = None
        self.assertGreater(n, 1_000_000)
        self.assertEqual(len(pedidos), 1, pedidos)
        self.assertEqual(pedidos[0]["memoria_mb"], 512)
        self.assertIn(v.pedidos[0]["motivo"].split()[0], ("spilling", "memory"))


class LoPrevisto(unittest.TestCase):
    """S4a (§1.7): con `previsto_mb` en el sobre, la celda empieza crecida."""

    def setUp(self):
        agente.Vigilante.sin_pool = False
        ore._con = None
        ore._aplicado.update(mb=None, hilos=None)

    def tearDown(self):
        agente.Vigilante.sin_pool = False
        ore._con = None
        ore._aplicado.update(mb=None, hilos=None)

    def entrar(self, previsto_mb, respuesta, pods=(4096, 4096, 32768), texto="select 1", motivo=None):
        """Entra en el vigilante con el pod midiendo `pods` (y luego el último);
        devuelve lo pedido a `ore-serve`, el vigilante y el `memory_limit` con que
        empieza la celda."""
        pedidos = []

        def pedir(metodo, ruta, cuerpo=None, **_):
            pedidos.append((metodo, ruta, cuerpo))
            return respuesta

        medidas = iter(pods)
        ultimo = [pods[0]]

        def pod_mb():
            ultimo[0] = next(medidas, ultimo[0])
            return ultimo[0]

        with mock.patch.dict(os.environ, {"ORE_HILOS": "2", "ORE_DISCO_MB": ""}), \
                mock.patch.object(ore.session, "id", "puesto-ana-python"), \
                mock.patch.object(ore.session, "pedir", pedir), \
                mock.patch.object(ore, "_pod_mb", pod_mb):
            with agente.Vigilante(texto, previsto_mb, motivo) as v:
                limite = ore._ajustar(ore._duckdb(), texto)
        return pedidos, v, limite

    def test_pide_lo_previsto_de_un_golpe_antes_de_correr(self):
        pedidos, v, limite = self.entrar(32768, (200, {"estado": "hecho", "escalon": 3}))
        self.assertEqual(pedidos, [("POST", "/puestos/puesto-ana-python/scale",
                                    {"memoria_mb": 32768, "motivo": "forecast", "de_un_golpe": True})])
        self.assertEqual(v.pedidos[0]["motivo"], "forecast")
        # La celda empieza con el reparto del pod crecido, y el vigilante sigue mirando.
        self.assertEqual(limite, 32768 * 60 // 100)
        self.assertIsNotNone(v._hilo)

    def test_lo_previsto_por_el_plan_se_pide_con_su_motivo(self):
        pedidos, v, _ = self.entrar(32768, (200, {"estado": "hecho", "escalon": 3}), motivo="plan")
        self.assertEqual(pedidos[0][2]["motivo"], "plan")
        self.assertTrue(pedidos[0][2]["de_un_golpe"])

    def test_sin_prevision_o_si_ya_cabe_no_pide(self):
        for previsto in (0, None, 4096, 2048):
            pedidos, v, _ = self.entrar(previsto, (200, {"estado": "hecho"}), pods=(4096,))
            self.assertEqual(pedidos, [], previsto)
            self.assertIsNotNone(v._hilo)

    def test_si_lo_previsto_no_cabe_empieza_como_esta_y_vigila(self):
        pedidos, v, limite = self.entrar(32768, (200, {"estado": "quota"}), pods=(4096,))
        self.assertEqual(len(pedidos), 1)
        self.assertEqual(limite, 4096 * 60 // 100)
        self.assertIsNotNone(v._hilo, "el vigilante sigue: subirá de uno en uno")

    def test_fuera_del_pool_no_vigila(self):
        pedidos, v, _ = self.entrar(32768, (409, {"error": "esta sesión no está en el session pool: no crece"}),
                                    pods=(4096,))
        self.assertTrue(agente.Vigilante.sin_pool)
        self.assertIsNone(v._hilo)

    def test_el_kernel_se_lo_da_al_vigilante(self):
        vistos = []

        class Espia(agente.Vigilante):
            def __init__(self, texto="", previsto_mb=0, previsto_motivo=None):
                vistos.append((texto, previsto_mb, previsto_motivo))
                super().__init__(texto, previsto_mb, previsto_motivo)

        with mock.patch.object(agente, "Vigilante", Espia), \
                mock.patch.object(ore.session, "id", None):
            agente.Kernel().correr("x = 1", "python", 16384)
            agente.Kernel().correr("select 1", "sql", 32768, "plan")
            agente.Kernel().correr("select 2", "sql")
        self.assertEqual(vistos, [("", 16384, None), ("select 1", 32768, "plan"), ("select 2", 0, None)])


class Bajar(unittest.TestCase):
    def setUp(self):
        self.kernel = mock.Mock()
        self.kernel.bajar.return_value = {"motor_mb": 40}
        self.puesto = mock.Mock(id="puesto-ana-python")
        self.puesto.pedir.return_value = (200, {"estado": "hecho"})

    def bajar(self, pod, hace):
        with mock.patch.dict(os.environ, {"ORE_MEMORIA_MB": "4096"}), \
                mock.patch.object(ore, "_pod_mb", return_value=pod):
            return agente.bajar_si_toca(self.kernel, self.puesto, agente.time.time() - hace)

    def test_la_que_crecio_y_lleva_un_rato_vuelve_a_la_base(self):
        r = self.bajar(16384, agente.BAJAR_TRAS + 5)
        self.assertIsNotNone(r)
        self.kernel.bajar.assert_called_once()
        self.puesto.pedir.assert_called_once()
        metodo, ruta, cuerpo = self.puesto.pedir.call_args[0]
        self.assertEqual((metodo, ruta, cuerpo["memoria_mb"]), ("POST", "/puestos/puesto-ana-python/scale", 4096))

    def test_si_no_crecio_o_trabaja_no_baja(self):
        self.assertIsNone(self.bajar(4096, agente.BAJAR_TRAS + 5))
        self.assertIsNone(self.bajar(16384, 10))
        self.kernel.bajar.assert_not_called()
        self.puesto.pedir.assert_not_called()

    def test_el_motor_suelta_antes(self):
        ore._con = duckdb.connect()
        try:
            with mock.patch.object(agente.time, "sleep"):
                r = agente._bajar_el_motor()
            cerca(self, limite_mb(ore._con), 256)
            self.assertIn("motor_mb", r)
        finally:
            ore._con = None
            ore._aplicado.update(mb=None, hilos=None)


if __name__ == "__main__":
    unittest.main()
