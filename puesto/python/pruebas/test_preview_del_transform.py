"""Preview de un transform (ORE 0055 P1, D18–D21).

    PYTHONPATH=puesto/python python -m unittest discover -s puesto/python/pruebas

Preview corre el arnés del build sobre el código del editor, en la sesión:
carga el módulo con D15 armado y llama al `def` elegido. `write()` no escribe:
devuelve lo que escribiría —esquema, las primeras filas, el recuento— y nada
de lo que la sesión pida al servidor escribe. Lo demás, como en un build: una
entrada no declarada falla con su línea y el mismo mensaje.

Las pruebas del SDK corren siempre. Las del arnés de verdad —la celda que
`ore-serve` genera— corren con él a mano: `cargo test` lo vuelca con
`ORE_CELDAS_GENERADAS=<dir>` (`92-arnes-de-preview.py`), y aquí se lee de
`ORE_ARNES_DE_PREVIEW` o de ese directorio.
"""

import json
import os
import re
import tempfile
import unittest

import ore

FICHERO = "packages/ventas/etl/limpios.py"


class _Respuesta:
    status = 200

    def read(self):
        return b"{}"

    def __enter__(self):
        return self

    def __exit__(self, *_):
        return False


class _Servidor:
    """`ore-serve` por debajo de `Session.pedir` (que es donde está la guarda
    del Preview): anota cada petición que llega y contesta 200 `{}`."""

    def __init__(self):
        self.pedidos = []

    def urlopen(self, req, timeout=None):
        ruta = req.full_url[len(ore.session.servidor):]
        cuerpo = json.loads(req.data) if req.data else None
        self.pedidos.append((req.get_method(), ruta, cuerpo))
        return _Respuesta()

    def escrituras(self):
        """Lo que se pidió que no es leer ni lo del puesto con el servidor."""
        return [(m, r) for m, r, _ in self.pedidos if m not in ("GET", "HEAD") and not r.startswith("/puestos/")]


class _ConServidor(unittest.TestCase):
    def setUp(self):
        self.servidor = _Servidor()
        self.urlopen = ore.urllib.request.urlopen
        ore.urllib.request.urlopen = self.servidor.urlopen
        self.id = ore.session.id
        ore.session.id = "puesto-ana"

    def tearDown(self):
        ore.urllib.request.urlopen = self.urlopen
        ore.session.id = self.id
        ore._ensayo(None)
        ore._modulo_en_carga(None)
        ore._tomar_informe()
        os.environ.pop("ORE_BUILD", None)


class WriteNoEscribeEnUnPreview(_ConServidor):
    def test_write_devuelve_esquema_filas_y_recuento_y_no_escribe(self):
        import pyarrow as pa

        ore._ensayo("ventas.default.limpios", "limpios")
        r = ore.write("ventas.limpios", pa.table({"id": list(range(250)), "pais": ["es"] * 249 + [None]}))
        self.assertTrue(r["preview"])
        self.assertEqual(r["rows"], 250)
        self.assertEqual(r["snapshot"], "")
        visto = ore._ensayo(None)
        self.assertEqual(visto["output"], "ventas.limpios")
        self.assertEqual(visto["total"], 250)
        self.assertEqual(visto["limite"], 100)
        self.assertEqual(len(visto["filas"]), 100)
        self.assertEqual(visto["filas"][0], [0, "es"])
        self.assertEqual([(c["name"], c["type"], c["iceberg"]) for c in visto["columnas"]],
                         [("id", "int64", "long"), ("pais", "string", "string")])
        # Nada llegó al servidor: ni el catálogo ni el almacén.
        self.assertEqual(self.servidor.pedidos, [])
        # Desarmado: `_ensayo(None)` otra vez no tiene nada.
        self.assertIsNone(ore._ensayo(None))

    def test_otra_salida_falla_como_en_un_build(self):
        import pyarrow as pa

        ore._ensayo("ventas.limpios", "limpios")
        with self.assertRaises(PermissionError) as e:
            ore.write("ventas.otra", pa.table({"id": [1]}))
        self.assertEqual(str(e.exception),
                         "`ventas.otra` is not the output of `limpios` (ventas.limpios): a transform only writes what it declares")
        self.assertEqual(self.servidor.pedidos, [])

    def test_nada_que_escriba_sale_de_la_sesion(self):
        ore._ensayo("ventas.limpios", "limpios")
        for metodo, ruta in [("POST", "/v1/ventas/namespaces/default/tables"),
                             ("PUT", "/documentos/ventas/x"),
                             ("POST", "/paquetes"),
                             ("POST", "/media/legal/archivo/contratos/transactions")]:
            with self.assertRaises(PermissionError) as e:
                ore.session.pedir(metodo, ruta, {})
            self.assertIn("Preview writes nothing", str(e.exception))
        # Lo del puesto con el servidor —el techo, sql(), el latido— sí pasa.
        self.assertEqual(ore.session.pedir("POST", "/puestos/puesto-ana/transform", {})[0], 200)
        self.assertEqual(ore.session.pedir("POST", "/puestos/puesto-ana/latido")[0], 200)
        self.assertEqual(self.servidor.escrituras(), [])
        # Desarmado, todo como siempre.
        ore._ensayo(None)
        self.assertEqual(ore.session.pedir("POST", "/paquetes", {})[0], 200)

    def test_una_entrada_no_declarada_falla_como_en_un_build(self):
        @ore.transform(inputs=["ventas.clientes"], output="ventas.limpios")
        def limpios():
            return ore.over("rrhh.nominas")

        ore._ensayo("ventas.limpios", "limpios")
        with self.assertRaises(PermissionError) as e:
            limpios()
        self.assertEqual(str(e.exception),
                         "`rrhh.nominas` is not among the inputs of `limpios` (ventas.clientes): a transform only reads what it declares")
        # Ni se pidió el dato.
        self.assertFalse([r for m, r, _ in self.servidor.pedidos if "/datos/" in r])


def _arnes_generado():
    ruta = os.environ.get("ORE_ARNES_DE_PREVIEW")
    if not ruta and os.environ.get("ORE_CELDAS_GENERADAS"):
        ruta = os.path.join(os.environ["ORE_CELDAS_GENERADAS"], "92-arnes-de-preview.py")
    if not ruta or not os.path.isfile(ruta):
        return None
    with open(ruta, encoding="utf-8") as f:
        return f.read()


ARNES = _arnes_generado()

FUENTE = '''import pyarrow as pa
from ore import transform, over, write


@transform(inputs=["ventas.clientes"], output="ventas.limpios")
def limpios():
    t = over("ventas.clientes", format="arrow")
    return write("ventas.limpios", t.filter(pa.compute.greater(t["id"], 1)))


@transform(inputs=["ventas.clientes"], output="ventas.otros")
def otros():
    return write("ventas.otros", pa.table({"n": [1, 2, 3]}))
'''


@unittest.skipIf(ARNES is None, "sin el arnés generado: ORE_CELDAS_GENERADAS=<dir> cargo test -p ore-serve --bin ore-serve el_arnes_del_preview")
class ElArnesDelPreview(_ConServidor):
    """La celda que `ore-serve` genera, con el texto y el `def` de cada caso."""

    def setUp(self):
        super().setUp()
        import pyarrow as pa
        import pyarrow.parquet as pq

        self.dir = tempfile.TemporaryDirectory()
        parquet = os.path.join(self.dir.name, "clientes.parquet")
        pq.write_table(pa.table({"id": list(range(1, 301))}), parquet)
        self.fuente_antes = ore._fuente_de_respuesta
        ore._fuente_de_respuesta = lambda vista, r: ("read_parquet('%s')" % parquet, {"_parquet": parquet})

    def tearDown(self):
        ore._fuente_de_respuesta = self.fuente_antes
        self.dir.cleanup()
        super().tearDown()

    def correr(self, fuente, def_, salida):
        celda = ARNES
        celda = re.sub(r"(?m)^    _codigo = compile\(.*\)$",
                       lambda _: "    _codigo = compile(%s, _FICHERO, \"exec\")" % json.dumps(fuente), celda)
        celda = re.sub(r"(?m)^_DEF = .*$", lambda _: "_DEF = %s" % json.dumps(def_), celda)
        celda = re.sub(r"(?m)^_SALIDA = .*$", lambda _: "_SALIDA = %s" % json.dumps(salida), celda)
        celda = re.sub(r"(?m)^_FICHERO = .*$", lambda _: "_FICHERO = %s" % json.dumps(FICHERO), celda)
        espacio = {"__name__": "__main__"}
        try:
            exec(compile(celda, "<celda>", "exec"), espacio)
            return None, ore._tomar_informe()
        except Exception as e:  # noqa: BLE001
            return e, ore._tomar_informe()
        finally:
            # El arnés siempre desarma.
            self.assertIsNone(ore._ensayando)

    def test_el_preview_devuelve_esquema_filas_y_recuento_y_no_escribe(self):
        e, informe = self.correr(FUENTE, "limpios", "ventas.limpios")
        self.assertIsNone(e)
        p = informe["preview"]
        self.assertEqual(p["output"], "ventas.limpios")
        self.assertEqual(p["total"], 299)
        self.assertEqual(len(p["filas"]), 100)
        self.assertEqual(p["filas"][0], [2])
        self.assertEqual(p["columnas"][0]["name"], "id")
        self.assertNotIn("filas", informe)
        self.assertNotIn("ORE_BUILD", os.environ)
        # Se declaró al servidor (dentro del techo) y nada se escribió.
        self.assertIn(("POST", "/puestos/puesto-ana/transform"), [(m, r) for m, r, _ in self.servidor.pedidos])
        self.assertEqual(self.servidor.escrituras(), [])

    def test_se_elige_un_def_entre_varios(self):
        e, informe = self.correr(FUENTE, "otros", "ventas.otros")
        self.assertIsNone(e)
        self.assertEqual(informe["preview"]["output"], "ventas.otros")
        self.assertEqual(informe["preview"]["total"], 3)
        # El otro `def` ni se llamó: no leyó nada.
        self.assertFalse([r for m, r, _ in self.servidor.pedidos if "/datos/" in r])

    def test_una_llamada_al_cargar_falla_con_su_linea_d15(self):
        e, informe = self.correr(FUENTE + "\n\nlimpios()\n", "limpios", "ventas.limpios")
        self.assertIsInstance(e, RuntimeError)
        self.assertIn("line 16 calls `limpios()` while the module loads", str(e))
        self.assertEqual(informe["error"], {"tipo": "called-while-loading", "fichero": FICHERO, "linea": 16})
        self.assertEqual(self.servidor.pedidos, [])

    def test_una_entrada_no_declarada_falla_con_su_linea(self):
        mala = FUENTE.replace('over("ventas.clientes", format="arrow")', 'over("rrhh.nominas", format="arrow")')
        e, informe = self.correr(mala, "limpios", "ventas.limpios")
        self.assertIsInstance(e, RuntimeError)
        self.assertEqual(str(e), "PermissionError: `rrhh.nominas` is not among the inputs of `limpios` (ventas.clientes): "
                                 "a transform only reads what it declares (%s, line 7)" % FICHERO)
        self.assertEqual(informe["error"], {"tipo": "runtime", "fichero": FICHERO, "linea": 7})
        self.assertNotIn("preview", informe)

    def test_escribir_otra_salida_falla_con_su_linea(self):
        mala = FUENTE.replace('write("ventas.limpios"', 'write("ventas.otra"')
        e, informe = self.correr(mala, "limpios", "ventas.limpios")
        self.assertIn("`ventas.otra` is not the output of `limpios` (ventas.limpios)", str(e))
        self.assertEqual(informe["error"]["linea"], 8)
        self.assertEqual(self.servidor.escrituras(), [])

    def test_un_write_al_cargar_tampoco_escribe(self):
        e, informe = self.correr(FUENTE + '\n\nwrite("ventas.limpios", pa.table({"x": [1]}))\n', "otros", "ventas.otros")
        self.assertIn("`ventas.limpios` is not the output of `otros` (ventas.otros)", str(e))
        self.assertEqual(informe["error"]["tipo"], "load")
        self.assertEqual(self.servidor.escrituras(), [])

    def test_un_texto_que_no_se_lee_falla_con_su_linea(self):
        e, informe = self.correr(FUENTE.replace("def otros():", "def otros(:"), "limpios", "ventas.limpios")
        self.assertIsInstance(e, RuntimeError)
        self.assertEqual(informe["error"]["tipo"], "syntax")
        self.assertEqual(informe["error"]["linea"], 12)

    def test_sin_write_no_hay_nada_que_ensenar(self):
        e, informe = self.correr(FUENTE.replace('    return write("ventas.otros", pa.table({"n": [1, 2, 3]}))', "    return 1"),
                                 "otros", "ventas.otros")
        self.assertIn("returned without calling `write()`", str(e))
        self.assertEqual(informe["error"]["tipo"], "not-written")


if __name__ == "__main__":
    unittest.main()
