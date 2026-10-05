"""El repositorio en disco, pytest y pyright para la consola (ORE 0050 P4).

`correr_pytest` y `correr_pyright` se prueban sobre un repositorio de verdad en
una carpeta temporal —lo que `materializar` deja en disco—, con el pytest de
esta máquina; pyright, sólo si está (en la imagen, `/opt/ore/pyright`).
"""
import json
import os
import shutil
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "..")))

import repositorio  # noqa: E402
from test_semilla import _cruda, _semilla  # noqa: E402

REPO = "packages/ventas/billing"
SDK = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))

ROTAS = '''import pytest


def test_suma():
    print("hola desde la prueba")
    assert 1 + 1 == 2


def test_falla():
    assert {"a": 1, "b": [2, 3]} == {"a": 1, "b": [2, 4]}


@pytest.mark.skip(reason="todavia no")
def test_saltada():
    pass


@pytest.mark.parametrize("x, y", [(1, 2), (2, 4)])
def test_doble(x, y):
    assert x * 2 == y


class TestUnaClase:
    def test_dentro(self):
        assert True


def test_lenta():
    import time
    time.sleep(10)
'''


def sembrar(raiz, extra=None):
    d = os.path.join(raiz, REPO)
    os.makedirs(os.path.join(d, "functions"))
    ficheros = {"functions/example.py": _semilla(), "functions/test_example.py": _cruda("TEST_PY"),
                "pyproject.toml": _cruda("PYPROJECT_FUNCTIONS_PY")}
    ficheros.update(extra or {})
    for rel, texto in ficheros.items():
        os.makedirs(os.path.dirname(os.path.join(d, rel)), exist_ok=True)
        with open(os.path.join(d, rel), "w", encoding="utf-8") as f:
            f.write(texto)
    return ["%s/%s" % (REPO, f) for f in repositorio.ficheros_que(repositorio.es_de_prueba, d)]


class Pytest(unittest.TestCase):
    def setUp(self):
        self.t = tempfile.mkdtemp(prefix="ore-repo-")

    def tearDown(self):
        shutil.rmtree(self.t, ignore_errors=True)

    def test_la_semilla_en_verde(self):
        todos = sembrar(self.t)
        self.assertEqual(todos, [REPO + "/functions/test_example.py"])
        r = repositorio.correr_pytest(self.t, REPO, todos, rutas=[SDK])
        r["resumen"] = repositorio.resumen_de(r)
        self.assertEqual(r["resumen"], {"total": 5, "ok": 5, "fallo": 0, "saltada": 0, "ficherosConError": 0}, r["registro"])
        p = r["pruebas"][0]
        self.assertEqual(p["fichero"], REPO + "/functions/test_example.py")
        self.assertTrue(p["linea"] > 1 and p["ruta"] == [])
        self.assertIn("test_the_surcharge_rounds_half_to_even_to_the_cent[10.00-0.00]", [x["nombre"] for x in r["pruebas"]])
        self.assertTrue(r["registro"].startswith("$ pytest -v"), r["registro"][:200])
        self.assertIn("5 passed", r["registro"])
        self.assertTrue(r["registro"].rstrip().endswith("exit code 0"))

    def test_lo_que_falla_se_dice(self):
        os.environ["ORE_TOPE_PRUEBA"] = "2"
        try:
            todos = sembrar(self.t, {"tests/test_rotas.py": ROTAS, "tests/test_no_carga.py": "import no_existe\n"})
            r = repositorio.correr_pytest(self.t, REPO, todos, rutas=[SDK])
        finally:
            del os.environ["ORE_TOPE_PRUEBA"]
        por = {(tuple(p["ruta"]), p["nombre"]): p for p in r["pruebas"] if p["fichero"].endswith("test_rotas.py")}
        self.assertEqual(por[((), "test_suma")]["estado"], "ok")
        f = por[((), "test_falla")]
        self.assertEqual(f["estado"], "fallo")
        self.assertIn("AssertionError", f["mensaje"])
        self.assertEqual((f["obtenido"], f["esperado"]), ("{'a': 1, 'b': [2, 3]}", "{'a': 1, 'b': [2, 4]}"))
        self.assertEqual((por[((), "test_saltada")]["estado"], por[((), "test_saltada")]["mensaje"]), ("saltada", "todavia no"))
        self.assertEqual(por[((), "test_doble[2-4]")]["estado"], "ok")
        self.assertEqual(por[(("TestUnaClase",), "test_dentro")]["estado"], "ok")
        lenta = por[((), "test_lenta")]
        self.assertEqual(lenta["estado"], "fallo")
        self.assertIn("2 s", lenta["mensaje"])
        fic = {x["ruta"]: x for x in r["ficheros"]}
        self.assertEqual(fic[REPO + "/tests/test_no_carga.py"]["estado"], "error")
        self.assertIn("no_existe", fic[REPO + "/tests/test_no_carga.py"]["mensaje"])
        self.assertEqual(fic[REPO + "/tests/test_rotas.py"]["estado"], "fallo")
        self.assertIn("hola desde la prueba", fic[REPO + "/tests/test_rotas.py"]["salida"])
        self.assertIn("hola desde la prueba", r["registro"])
        # Y la semilla, al lado, sigue en verde: un fichero que no carga no para el resto.
        self.assertEqual(sum(1 for p in r["pruebas"] if p["fichero"].endswith("test_example.py") and p["estado"] == "ok"), 5)

    def test_una_por_su_nombre(self):
        todos = sembrar(self.t, {"tests/test_rotas.py": ROTAS})
        r = repositorio.correr_pytest(self.t, REPO, [REPO + "/tests/test_rotas.py"], ["test_doble[2-4]", "test_dentro"], rutas=[SDK])
        self.assertEqual(sorted(p["nombre"] for p in r["pruebas"]), ["test_dentro", "test_doble[2-4]"])
        self.assertIn(REPO + "/tests/test_rotas.py", todos)

    def test_sin_la_identidad_de_la_sesion(self):
        sembrar(self.t, {"functions/test_entorno.py": 'import os\n\ndef test_x():\n    assert not [k for k in os.environ if k.startswith(("PUESTO", "ORE_SUJETO", "ORE_TOKEN"))]\n'})
        os.environ["PUESTO"] = "puesto-x"
        try:
            r = repositorio.correr_pytest(self.t, REPO, [REPO + "/functions/test_entorno.py"], rutas=[SDK])
        finally:
            del os.environ["PUESTO"]
        self.assertEqual([p["estado"] for p in r["pruebas"]], ["ok"], r["registro"])


class _Puesto:
    """Un ore-serve de mentira: la ficha del puesto, el índice y los ficheros."""

    def __init__(self, ficheros, rama="ana/x"):
        self.id, self.ficheros, self.rama, self.pedidas = "puesto-1", ficheros, rama, []

    def pedir(self, metodo, ruta, cuerpo=None, plazo=30, cabeceras=None):
        self.pedidas.append((ruta, dict(cabeceras or {})))
        if ruta == "/puestos/puesto-1":
            return 200, {"repositorio": REPO, "rama": self.rama}
        if ruta == "/arbol":
            return 200, {"ficheros": [{"ruta": r, "bytes": len(t)} for r, t in self.ficheros.items()]}
        r = repositorio.urllib.parse.unquote(ruta[len("/arbol/"):])
        return (200, {"texto": self.ficheros[r]}) if r in self.ficheros else (404, {})


class _Testigo:
    def cabeceras(self):
        return {"authorization": "Bearer x"}


class Materializar(unittest.TestCase):
    def test_trae_la_rama_retira_lo_que_ya_no_esta_y_prueba_con_borradores(self):
        t = tempfile.mkdtemp(prefix="ore-mat-")
        try:
            os.makedirs(os.path.join(t, REPO, "functions"))
            with open(os.path.join(t, REPO, "functions", "test_borrada.py"), "w") as f:
                f.write("def test_x():\n    assert False\n")
            fic = {REPO + "/functions/example.py": _semilla(), REPO + "/functions/test_example.py": _cruda("TEST_PY"),
                   REPO + "/pyproject.toml": _cruda("PYPROJECT_FUNCTIONS_PY"), REPO + "/pylock.toml": "no se trae",
                   REPO + "/datos/caso uno.csv": "a,b\n1,2\n", REPO + "/imagen.png": "no es código"}
            p = _Puesto(fic)
            rep = repositorio.Repositorio(p, _Testigo(), trabajo=t)
            rep.rutas = [SDK]
            self.assertEqual(rep.materializar(), REPO)
            self.assertTrue(os.path.exists(os.path.join(t, REPO, "datos", "caso uno.csv")))
            self.assertFalse(os.path.exists(os.path.join(t, REPO, "functions", "test_borrada.py")))
            self.assertFalse(os.path.exists(os.path.join(t, REPO, "pylock.toml")))
            self.assertFalse(os.path.exists(os.path.join(t, REPO, "imagen.png")))
            self.assertEqual(dict(p.pedidas)["/arbol"], {"x-ore-rama": "ana/x", "x-ore-raiz": REPO})
            # Test: un borrador que rompe una prueba, sin guardar.
            roto = _cruda("TEST_PY").replace('Decimal("100.25")', 'Decimal("99")')
            r = rep.probar({"borradores": [{"ruta": REPO + "/functions/test_example.py", "texto": roto},
                                           {"ruta": "packages/otro/x/test_y.py", "texto": "fuera"}]})
            self.assertEqual((r["resumen"]["ok"], r["resumen"]["fallo"]), (4, 1), r.get("registro"))
            self.assertEqual(r["todos"], [REPO + "/functions/test_example.py"])
            self.assertFalse(os.path.exists(os.path.join(t, "packages/otro")))
            # El espejo del editor, dentro del árbol y nunca fuera.
            uri = "file://" + os.path.join(t, REPO, "functions", "nuevo.py")
            rep.espejo({"method": "textDocument/didOpen", "params": {"textDocument": {"uri": uri, "text": "x = 1\n"}}})
            self.assertTrue(os.path.exists(os.path.join(t, REPO, "functions", "nuevo.py")))
            self.assertIsNone(repositorio.en_disco("file:///etc/passwd", t))
        finally:
            shutil.rmtree(t, ignore_errors=True)


class Pyright(unittest.TestCase):
    def test_lo_que_dice_pyright_es_del_arbol(self):
        salida = json.dumps({"generalDiagnostics": [
            {"file": "/t/packages/v/b/functions/example.py", "severity": "error", "message": "Argument missing for parameter \"currency\"",
             "range": {"start": {"line": 4, "character": 11}}, "rule": "reportCallIssue"},
            {"file": "/t/packages/v/b/functions/x.py", "severity": "information", "message": "no cuenta"}]})
        r = repositorio.leer_pyright(salida, "/t")
        self.assertEqual(r["diagnosticos"], [{"fichero": "packages/v/b/functions/example.py", "linea": 5, "columna": 12,
                                              "codigo": "pyright:reportCallIssue", "mensaje": "Argument missing for parameter \"currency\"",
                                              "severidad": "error"}])
        self.assertEqual((r["errores"], r["ficheros"]), (1, 1))

    @unittest.skipUnless(os.path.exists(os.environ.get("ORE_PYRIGHT_JS", "/opt/ore/pyright/index.js")), "sin pyright")
    def test_un_cambio_que_rompe_otro_fichero(self):
        t = tempfile.mkdtemp(prefix="ore-pyright-")
        try:
            sembrar(t, {"functions/helpers.py": "from decimal import Decimal\n\n\ndef to_cents(amount: Decimal, currency: str) -> int:\n    return int(amount * 100)\n",
                        "functions/uses.py": "from decimal import Decimal\n\nfrom helpers import to_cents\n\n\ndef total() -> int:\n    return to_cents(Decimal(\"1.5\"))\n"})
            r = repositorio.correr_pyright(t, REPO, [SDK], ["node", os.environ.get("ORE_PYRIGHT_JS", "/opt/ore/pyright/index.js")])
            self.assertEqual(r["errores"], 1, r)
            d = r["diagnosticos"][0]
            self.assertEqual((d["fichero"], d["linea"], d["codigo"]), (REPO + "/functions/uses.py", 7, "pyright:reportCallIssue"))
        finally:
            shutil.rmtree(t, ignore_errors=True)


if __name__ == "__main__":
    unittest.main()
