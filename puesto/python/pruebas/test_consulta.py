"""El trabajador DuckDB de Query (Query Q4, `ore_consulta.py`, `docs/query.md` §3.2).

    bash pruebas-de-fuego/geo/c1/sdk-en-la-imagen.sh 'test_consulta.py'

Sin lago ni red: que cada consulta corre en su instancia con su tope y la configuración
cerrada (lo de Q0b, §2.7), que lo que todavía no corre en un trabajador se dice, que
sin la credencial prestada no se lee nada, que el resultado vuelve cortado y que la
conversación con `ore-serve` (pedir por espera larga, entregar) va de punta a punta.
"""

import base64
import http.server
import io
import json
import os
import tempfile
import threading
import time
import unittest
from unittest import mock

import duckdb
import pyarrow as pa

import ore
import ore_consulta as oc


def trabajo(texto, **k):
    t = {"id": "q1-prueba", "texto": texto, "fuentes": {}, "limite": 1000, "clase": "L", "memoria_mb": 512, "hilos": 1}
    t.update(k)
    return t


class LaInstancia(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp()

    def calcular(self, t):
        con = oc.preparar()
        try:
            return oc.calcular(con, t, os.path.join(self.dir, t["id"])), con
        except Exception:
            con.close()
            raise

    def test_una_consulta_corre_y_queda_cerrada(self):
        (tabla, truncada), con = self.calcular(trabajo("select 42 as x"))
        self.assertEqual(tabla.to_pylist(), [{"x": 42}])
        self.assertFalse(truncada)
        # Lo del usuario no vuelve a abrir nada (§3.5).
        for intento in ("set memory_limit = '40GB'", "set enable_external_access = true",
                        "set allowed_directories = ['/']", "select * from read_text('/etc/passwd')",
                        "load postgres", "install postgres", "copy (select 1) to '/tmp/fuga.csv'"):
            with self.assertRaises(duckdb.Error, msg=intento):
                con.execute(intento).fetchall()
        self.assertEqual(con.execute("select current_setting('memory_limit')").fetchone()[0].replace(" ", ""), "512.0MiB")
        con.close()

    def test_lo_del_usuario_tampoco_lee_ficheros_locales(self):
        with self.assertRaises(oc.NoSeCorre) as e:
            self.calcular(trabajo("select * from read_text('/etc/passwd')"))
        self.assertEqual(e.exception.codigo, "consulta/sql")

    def test_solo_una_sentencia_y_que_lea(self):
        for texto in ("create table t as select 1", "select 1; select 2", "copy (select 1) to 'x.csv'"):
            with self.assertRaises(oc.NoSeCorre, msg=texto) as e:
                self.calcular(trabajo(texto))
            self.assertIn(e.exception.codigo, ("consulta/sin-resultado", "consulta/sql"), texto)

    def test_el_limite_corta_y_lo_dice(self):
        (tabla, truncada), con = self.calcular(trabajo("select * from range(5000)", limite=10))
        con.close()
        self.assertEqual(tabla.num_rows, 10)
        self.assertTrue(truncada)

    def test_lo_que_vuelve_cabe_en_el_cuerpo_de_ore_serve(self):
        t = pa.table({"x": ["una fila con algo de texto %d" % i for i in range(200_000)]})
        datos, filas, truncada = oc.a_arrow(t, False)
        self.assertLessEqual(len(datos) * 4 // 3, oc.BYTES_DE_VUELTA)
        self.assertTrue(truncada)
        self.assertGreater(filas, 1000)
        self.assertEqual(pa.ipc.open_stream(io.BytesIO(datos)).read_all().num_rows, filas)


class LasFuentes(unittest.TestCase):
    def comprobar(self, rd):
        raices = set()
        oc._comprobar("ventas.default.pedidos", rd, raices)
        return raices

    def test_lo_que_aun_no_corre_en_un_trabajador(self):
        for rd in ({"federada": {"tabla": "pg.t"}}, {"vistaFederada": "select 1"}, {"collection": "a.b.c"}):
            with self.assertRaises(oc.NoSeCorre) as e:
                self.comprobar(rd)
            self.assertEqual(e.exception.codigo, "consulta/en-el-puesto")

    def test_sin_credencial_prestada_no_se_lee(self):
        with self.assertRaises(oc.NoSeCorre) as e:
            self.comprobar({"metadata_location": "gs://b/w/ventas/pedidos/metadata/00001.metadata.json"})
        self.assertEqual(e.exception.codigo, "consulta/sin-credencial")
        with self.assertRaises(oc.NoSeCorre) as e:
            self.comprobar({"clave": "x.parquet"})
        self.assertEqual(e.exception.codigo, "consulta/copia-antigua")

    def test_las_raices_de_sus_datasets_y_las_de_una_view(self):
        cred = {"gcs.oauth2.token": "t"}
        raices = self.comprobar({"consulta": "select 1", "datasets": {
            "a.default.x": {"metadata_location": "gs://b/w/a/x/metadata/00003.metadata.json", "credencial": cred},
            "a.default.y": {"metadata_location": "s3://r/w/a/y/metadata/00001.metadata.json",
                            "credencial": {"s3.access-key-id": "k"}}}})
        self.assertEqual(raices, {"https://storage.googleapis.com/b/w/a/x/", "s3://r/w/a/y/"})

    def test_en_un_trabajador_el_lago_no_se_lee_con_la_identidad_del_pod(self):
        con = oc.preparar()
        with ore._instancia(con), self.assertRaises(PermissionError):
            ore._iceberg("gs://b/w/a/x/metadata/00003.metadata.json")
        con.close()

    def test_cada_hilo_su_instancia_y_el_puesto_la_suya(self):
        antes = ore._con
        vistas = {}

        def en(n):
            con = duckdb.connect()
            with ore._instancia(con):
                time.sleep(0.05)
                vistas[n] = ore._duckdb() is con
            con.close()
        hs = [threading.Thread(target=en, args=(i,)) for i in range(4)]
        [h.start() for h in hs]
        [h.join() for h in hs]
        self.assertEqual(vistas, {0: True, 1: True, 2: True, 3: True})
        self.assertIs(ore._con, antes)
        self.assertFalse(ore._en_instancia())


class ElServidorDePruebas(http.server.BaseHTTPRequestHandler):
    """Un `ore-serve` de mentira: una consulta, y luego nada."""
    trabajos = []
    entregados = {}
    cabeceras = []

    def log_message(self, *a):
        pass

    def do_GET(self):
        ElServidorDePruebas.cabeceras.append(dict(self.headers))
        if self.path != "/consultas/trabajo":
            self.send_response(404)
            self.end_headers()
            return
        if ElServidorDePruebas.trabajos:
            datos = json.dumps(ElServidorDePruebas.trabajos.pop(0)).encode()
            self.send_response(200)
            self.send_header("content-length", str(len(datos)))
            self.end_headers()
            self.wfile.write(datos)
            return
        time.sleep(0.2)
        self.send_response(204)
        self.end_headers()

    def do_POST(self):
        n = int(self.headers.get("content-length") or 0)
        ElServidorDePruebas.entregados[self.path] = json.loads(self.rfile.read(n))
        self.send_response(200)
        self.send_header("content-length", "2")
        self.end_headers()
        self.wfile.write(b"{}")


class DePuntaAPunta(unittest.TestCase):
    def test_pide_corre_y_entrega(self):
        s = http.server.ThreadingHTTPServer(("127.0.0.1", 0), ElServidorDePruebas)
        threading.Thread(target=s.serve_forever, daemon=True).start()
        ElServidorDePruebas.trabajos = [trabajo("select sum(range) as s from range(1000)", id="q7-abc")]
        ElServidorDePruebas.entregados = {}
        ElServidorDePruebas.cabeceras = []
        trabajo_dir = tempfile.mkdtemp()
        parar = threading.Event()
        with mock.patch.multiple(oc, SERVE="http://127.0.0.1:%d" % s.server_address[1], TRABAJO=trabajo_dir,
                                 MEMORIA_MB=4096, HILOS=2, PREPARADAS=1), \
                mock.patch.dict(os.environ, {"ORE_SUJETO": "agente:puesto/ore-consulta/1"}):
            h = threading.Thread(target=oc.servir, args=(parar,))
            h.start()
            hasta = time.time() + 30
            while "/consultas/q7-abc/resultado" not in ElServidorDePruebas.entregados and time.time() < hasta:
                time.sleep(0.1)
            parar.set()
            h.join(10)
        s.shutdown()
        r = ElServidorDePruebas.entregados.get("/consultas/q7-abc/resultado")
        self.assertIsNotNone(r, "el resultado no llegó")
        self.assertTrue(r["ok"], r)
        tabla = pa.ipc.open_stream(io.BytesIO(base64.b64decode(r["arrow"]))).read_all()
        self.assertEqual(tabla.to_pylist(), [{"s": 499500}])
        self.assertEqual(r["filas"], 1)
        # Dijo lo que le cabía, y se presentó (aquí con el sujeto de pruebas).
        h0 = {k.lower(): v for k, v in ElServidorDePruebas.cabeceras[0].items()}
        self.assertEqual(h0["x-ore-libre-mb"], "4096")
        self.assertEqual(h0["x-ore-libres-hilos"], "2")
        self.assertIn("x-ore-trabajador", h0)
        # Y no deja derrame detrás.
        self.assertEqual(os.listdir(trabajo_dir), [])


    def test_el_de_quita_y_pon_se_va_solo_y_el_del_suelo_no(self):
        s = http.server.ThreadingHTTPServer(("127.0.0.1", 0), ElServidorDePruebas)
        threading.Thread(target=s.serve_forever, daemon=True).start()
        ElServidorDePruebas.trabajos = []
        comun = dict(SERVE="http://127.0.0.1:%d" % s.server_address[1], TRABAJO=tempfile.mkdtemp(),
                     MEMORIA_MB=4096, HILOS=2, PREPARADAS=0)
        with mock.patch.dict(os.environ, {"ORE_SUJETO": "agente:puesto/ore-consulta/1"}):
            # De quita y pon: sin nada que hacer, se va sin que nadie lo pare.
            with mock.patch.multiple(oc, OCIOSO_S=1, **comun):
                h = threading.Thread(target=oc.servir)
                t0 = time.time()
                h.start()
                h.join(15)
                self.assertFalse(h.is_alive(), "no se fue")
                self.assertLess(time.time() - t0, 10)
            # El del suelo (0): sigue pidiendo hasta que lo paran.
            parar = threading.Event()
            with mock.patch.multiple(oc, OCIOSO_S=0, **comun):
                h = threading.Thread(target=oc.servir, args=(parar,))
                h.start()
                time.sleep(2)
                self.assertTrue(h.is_alive())
                parar.set()
                h.join(10)
        s.shutdown()


if __name__ == "__main__":
    unittest.main()
