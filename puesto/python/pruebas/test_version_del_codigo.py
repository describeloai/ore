"""El código que corre es la versión (ORE 0056 V1).

    PYTHONPATH=puesto/python python -m unittest discover -s puesto/python/pruebas

Medido antes (2026-10-05): la huella del SQL anclado era la consulta y la
FIRMA de cada función; cambiar el cuerpo de `pdf_pages` sin tocar su firma
daba «4 saltados» y dejaba las filas viejas. Y `get_function` guardaba la
función por su nombre: una sesión larga seguía corriendo el código de antes
del commit.
"""

import unittest

import ore
from ore.medios import _version_de

DOC = {"paquete": "ventas", "spec": {"runtime": "python", "entrypoint": "repo/f.py:doble",
                                     "input": {"x": {"type": "Integer"}}, "output": {"type": "Integer"}}}
V1 = "def doble(x: int) -> int:\n    return x * 2\n"
V2 = "def doble(x: int) -> int:\n    return x + x + 0\n"  # misma firma, otro cuerpo
V3 = "def doble(x: int) -> int:\n    return x * 3\n"


class _Arbol:
    """Lo que `ore-serve` contesta: el documento y el fichero de hoy."""

    def __init__(self, texto):
        self.texto = texto
        self.lecturas = 0

    def pedir(self, metodo, ruta, cuerpo=None, plazo=30, cabeceras=None, seguir=True):
        if ruta.startswith("/documentos/Function/"):
            return 200, DOC
        if ruta == "/arbol/packages/ventas/repo/f.py":
            self.lecturas += 1
            return 200, {"texto": self.texto}
        return 404, {}


class ElCodigoEsLaVersion(unittest.TestCase):
    def setUp(self):
        self.antes = ore.session.pedir
        self.arbol = _Arbol(V1)
        ore.session.pedir = self.arbol.pedir
        ore._FUNCIONES.clear()
        ore._FUNCIONES_SPEC.clear()
        ore._FUNCIONES_CODIGO.clear()

    def tearDown(self):
        ore.session.pedir = self.antes

    def test_un_commit_nuevo_se_corre_sin_reiniciar_la_sesion(self):
        self.assertEqual(ore.get_function("ventas.doble")(x=5), 10)
        self.arbol.texto = V3
        self.assertEqual(ore.get_function("ventas.doble")(x=5), 15)

    def test_el_mismo_codigo_no_se_reconstruye(self):
        f = ore.get_function("ventas.doble")
        self.assertIs(ore.get_function("ventas.doble"), f)
        self.assertEqual(self.arbol.lecturas, 2)  # se lee siempre, se reusa si es igual

    def test_otro_cuerpo_con_la_misma_firma_es_otra_huella(self):
        q = "select ventas.doble(c.n) from ventas.c as c"
        ore.get_function("ventas.doble")
        antes = ore._huella_sql(q, ["ventas.doble"])
        self.arbol.texto = V2
        ore.get_function("ventas.doble")
        self.assertNotEqual(ore._huella_sql(q, ["ventas.doble"]), antes)

    def test_el_mismo_cuerpo_es_la_misma_huella(self):
        q = "select ventas.doble(c.n) from ventas.c as c"
        ore.get_function("ventas.doble")
        antes = ore._huella_sql(q, ["ventas.doble"])
        ore.get_function("ventas.doble")
        self.assertEqual(ore._huella_sql(q, ["ventas.doble"]), antes)

    def test_apply_sin_version_lleva_la_del_fichero(self):
        f1 = ore.get_function("ventas.doble")
        self.arbol.texto = V2
        f2 = ore.get_function("ventas.doble")
        self.assertTrue(_version_de(f1).startswith("codigo:"))
        self.assertNotEqual(_version_de(f1), _version_de(f2))


if __name__ == "__main__":
    unittest.main()
