"""El túnel de marimo (0064 Pieza 3): la primera petición del navegador puede
llegar antes de que marimo escuche (victor, 2026-10-19: 0,8 s antes, un 502).
El túnel espera a que acepte conexiones; si no llega nunca, sigue siendo 502.

    PYTHONPATH=puesto/python python -m unittest discover -s puesto/python/pruebas
"""
import http.server
import os
import socket
import sys
import threading
import time
import unittest
from unittest import mock

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
import marimo_tunel  # noqa: E402


def _puerto_libre():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class _Hola(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.send_header("content-length", "4")
        self.end_headers()
        self.wfile.write(b"hola")

    def log_message(self, *a):
        pass


def _atender(puerto):
    t = marimo_tunel.Tunel(p=None, testigo=None, marimo=mock.MagicMock())
    with mock.patch.object(marimo_tunel, "PUERTO", puerto):
        t._atender({"id": 1, "metodo": "GET", "ruta": "/", "cabeceras": [], "cuerpo": ""})
    return t.salientes


class LaEsperaAMarimo(unittest.TestCase):
    def test_espera_a_que_marimo_escuche(self):
        puerto = _puerto_libre()
        servidor = []

        def arrancar_tarde():
            time.sleep(0.8)
            s = http.server.HTTPServer(("127.0.0.1", puerto), _Hola)
            servidor.append(s)
            s.serve_forever()

        threading.Thread(target=arrancar_tarde, daemon=True).start()
        try:
            salientes = _atender(puerto)
        finally:
            while not servidor:
                time.sleep(0.05)
            servidor[0].shutdown()
        self.assertEqual(salientes[0]["estado"], 200)
        self.assertEqual(salientes[-1]["t"], "end")

    def test_si_marimo_no_llega_es_502(self):
        with mock.patch.object(marimo_tunel, "ESPERA_A_MARIMO", 0.5):
            salientes = _atender(_puerto_libre())
        self.assertEqual(salientes[0]["estado"], 502)


class LaEntrega(unittest.TestCase):
    """Lo que no entra en ore-serve (503, conexión cortada) vuelve delante y se
    reintenta en orden; lo que ore-serve niega de verdad (400) no."""

    def _tunel(self, respuestas):
        p = mock.MagicMock()
        p.id = "puesto-a"
        p.pedir.side_effect = respuestas
        return marimo_tunel.Tunel(p=p, testigo=mock.MagicMock(), marimo=mock.MagicMock())

    def test_un_503_y_un_corte_se_reintentan_en_orden(self):
        t = self._tunel([(503, {}), ConnectionResetError("reset"), (204, None)])
        t.salientes = [{"id": 1, "t": "head"}]
        self.assertFalse(t._entregar_una_vez())
        t._encolar({"id": 1, "t": "end"})
        self.assertFalse(t._entregar_una_vez())
        self.assertTrue(t._entregar_una_vez())
        enviado = t.p.pedir.call_args_list[-1].kwargs["cuerpo"]["respuestas"]
        self.assertEqual([m["t"] for m in enviado], ["head", "end"])
        self.assertEqual((t.salientes, t.fallos), ([], 0))

    def test_lo_que_ore_serve_niega_no_vuelve(self):
        t = self._tunel([(400, {"error": "no"})])
        t.salientes = [{"id": 1, "t": "head"}]
        self.assertTrue(t._entregar_una_vez())
        self.assertEqual(t.salientes, [])


if __name__ == "__main__":
    unittest.main()
