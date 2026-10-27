"""La tabla llega al escritor en flujo, lote a lote (2026-10-24: 3 M de filas con
geometría mataron un puesto de 4 GiB con tres copias de la tabla en Python).

El escritor de aquí es un proceso de verdad que lee lo mismo que `ore-store`:
la petición en la primera línea y detrás un flujo Arrow IPC con su marca de fin."""

import json
import os
import stat
import sys
import tempfile
import unittest

import pyarrow as pa

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
import ore  # noqa: E402

ESCRITOR = """#!%s
import json, sys
import pyarrow as pa
entrada = sys.stdin.buffer
peticion = json.loads(entrada.readline())
if peticion.get("falla"):
    sys.stderr.write("error: el escritor no quiso")
    sys.exit(1)
lector = pa.ipc.open_stream(entrada)
filas, lotes = 0, 0
for lote in lector:
    filas += lote.num_rows
    lotes += 1
fin = entrada.read()
print(json.dumps({"filas": filas, "lotes": lotes, "sobra": len(fin), "dataset": peticion["dataset"]}))
"""


class EscrituraEnFlujo(unittest.TestCase):
    def setUp(self):
        d = tempfile.mkdtemp()
        self.binario = os.path.join(d, "ore-store-falso")
        with open(self.binario, "w") as f:
            f.write(ESCRITOR % sys.executable)
        os.chmod(self.binario, os.stat(self.binario).st_mode | stat.S_IXUSR)

    def test_la_tabla_va_en_lotes_y_entera(self):
        t = pa.table({"id": pa.array(range(200000), pa.int64())})
        r = ore._escribir_ficheros(self.binario, dict(os.environ), {"dataset": "d"}, t)
        self.assertEqual(r["filas"], 200000)
        self.assertGreater(r["lotes"], 1)
        self.assertEqual(r["sobra"], 0)
        self.assertEqual(r["dataset"], "d")

    def test_el_error_del_escritor_llega_aunque_no_lea_la_tabla(self):
        t = pa.table({"id": pa.array(range(500000), pa.int64())})
        with self.assertRaises(RuntimeError) as e:
            ore._escribir_ficheros(self.binario, dict(os.environ), {"dataset": "d", "falla": True}, t)
        self.assertIn("el escritor no quiso", str(e.exception))

    def test_lo_mismo_que_el_ipc_entero(self):
        t = pa.table({"id": pa.array([1, 2, 3], pa.int64()), "n": pa.array(["a", None, "c"])})
        r = ore._escribir_ficheros(self.binario, dict(os.environ), {"dataset": "d"}, t)
        self.assertEqual(r["filas"], pa.ipc.open_stream(ore._ipc(t)).read_all().num_rows)


class DesdeDuckDB(unittest.TestCase):
    """`create or replace dataset … as` escribe de `sql(…, format="reader")`: el
    resultado lote a lote, nunca entero (300M filas: 43 GB, 2026-10-24)."""

    def setUp(self):
        EscrituraEnFlujo.setUp(self)

    def test_el_lector_va_en_lotes_y_no_junta_la_tabla(self):
        lector = ore.sql("select range as id, '[1]'::JSON as j from range(250000)", format="reader")
        self.assertIsInstance(lector, pa.RecordBatchReader)
        self.assertEqual(lector.schema.field("j").metadata.get(ore.MARCA_JSON.encode()), b"1")
        r = ore._escribir_ficheros(self.binario, dict(os.environ), {"dataset": "d"}, ore._con_la_primera(lector))
        self.assertEqual(r["filas"], 250000)
        self.assertGreaterEqual(r["lotes"], 3)

    def test_un_flujo_vacio_no_se_escribe(self):
        self.assertIsNone(ore._con_la_primera(ore.sql("select 1 as x where false", format="reader")))


if __name__ == "__main__":
    unittest.main()
