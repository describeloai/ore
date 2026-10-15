"""Las promesas de lo que se escribe (OOS v1alpha31, ORE 0063 S7e·4).

    PYTHONPATH=puesto/python python -m unittest discover -s puesto/python/pruebas

`write()` pasa a `ore-store escribir` lo que la tabla pide contar
(`ore.promesas` en el `config` de `loadTable`); si el commit retiene (422
`PromiseHeldException`) es un `PromiseHeldError` con la promesa, lo medido y lo
esperado; `publish_anyway=True` manda la cabecera que publica de todos modos; y
lo que una `warning` vio llega como `HealthWarning`. Sin servidor: la sesión,
`ore-store` y el commit son de mentira.
"""

import json
import unittest
import warnings

try:
    import pyarrow as pa
except ImportError:  # el SDK sin pyarrow no escribe
    pa = None

import ore

PIDE = [{"column": "id", "id": "id_unico", "metric": "duplicateValues"}]
MOTIVO = ("promise `id_unico`: measured 4, expected = 0: held, still serving the published version. "
          "Publish anyway to serve it")
RETENIDO = {"error": {"code": 422, "type": "PromiseHeldException", "message": MOTIVO, "retenidas": [{
    "tabla": "ventas.default.p", "guarda": "id_unico", "motivo": MOTIVO,
    "version_retenida": "s3://lago/catalogo/ventas/default/p/metadata/00003.metadata.json",
    "promesas": [{"id": "id_unico", "resultado": "fail", "medido": "4", "esperado": "= 0", "severidad": "error"}]}]}}


@unittest.skipIf(pa is None, "sin pyarrow")
class PromesasAlEscribir(unittest.TestCase):
    def setUp(self):
        self.peticiones, self.commits = [], []
        self.respuesta = (200, {"metadata-location": "m2", "metadata": {"current-snapshot-id": 2}})

        def pedir(metodo, ruta, cuerpo=None, cabeceras=None, **_):
            if metodo == "GET":
                return 200, {"metadata-location": "m1", "metadata": {"location": "s3://lago/x"},
                             "config": {"ore.promesas": json.dumps(PIDE)}}
            self.commits.append((cuerpo, dict(cabeceras or {})))
            return self.respuesta

        def escribir(binario, env, peticion, ipc):
            self.peticiones.append(peticion)
            return {"requirements": [], "updates": [], "filas": 8, "operacion": "op"}

        self.antes = (ore.session.pedir, ore._ore_store, ore._escribir_ficheros)
        ore.session.pedir = pedir
        ore._ore_store = lambda config, ubicacion: ("ore-store", {})
        ore._escribir_ficheros = escribir

    def tearDown(self):
        ore.session.pedir, ore._ore_store, ore._escribir_ficheros = self.antes

    def tabla(self):
        return pa.table({"id": pa.array([0, 0, 1, 1], pa.int64())})

    def test_lo_que_la_tabla_pide_contar_va_a_escribir(self):
        ore.write("ventas.default.p", self.tabla())
        self.assertEqual(self.peticiones[0]["promesas"], PIDE)
        self.assertNotIn("x-ore-publicar-igual", self.commits[0][1])

    def test_una_promesa_que_retiene_es_un_error_que_se_atrapa(self):
        self.respuesta = (422, RETENIDO)
        with self.assertRaises(ore.PromiseHeldError) as e:
            ore.write("ventas.default.p", self.tabla())
        err = e.exception
        self.assertIsInstance(err, RuntimeError)
        self.assertEqual((err.promise, err.measured, err.expected), ("id_unico", "4", "= 0"))
        self.assertTrue(err.held.endswith("00003.metadata.json"))
        self.assertIn("measured 4, expected = 0: held", str(err))
        self.assertTrue(str(err).startswith("write(") and "ventas" in str(err).split(":")[0])
        self.assertEqual(len(self.commits), 1, "retenido no se reintenta")

    def test_publicar_igual_manda_la_cabecera(self):
        ore.write("ventas.default.p", self.tabla(), publish_anyway=True)
        self.assertEqual(self.commits[0][1].get("x-ore-publicar-igual"), "true")

    def test_lo_que_una_warning_vio_se_dice(self):
        aviso = "promise `pocas`: measured 8, expected ≥ 10"
        self.respuesta = (200, {"metadata-location": "m2", "metadata": {"current-snapshot-id": 2},
                                "ore.avisos": [aviso]})
        with warnings.catch_warnings(record=True) as w:
            warnings.simplefilter("always")
            r = ore.write("ventas.default.p", self.tabla())
        salud = [x for x in w if x.category is ore.HealthWarning]
        self.assertEqual(len(salud), 1)
        self.assertIn(aviso, str(salud[0].message))
        self.assertEqual(r["warnings"], [aviso])


if __name__ == "__main__":
    unittest.main()
