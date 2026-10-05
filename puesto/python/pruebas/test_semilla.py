"""La semilla de `functions-python` (ORE 0050 G4) corre: Run en la sesión la ejecuta
como `__main__` con el contrato del SDK, e imprime lo que el comentario dice.

Se lee de `crates/ore-core/src/clases.rs` —la única copia—, como la siembra ore-serve.
"""

import contextlib
import io
import os
import re
import subprocess
import sys
import tempfile
import unittest

RAIZ = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", ".."))


def _semilla():
    t = io.open(os.path.join(RAIZ, "crates", "ore-core", "src", "clases.rs"), encoding="utf-8").read()
    m = re.search(r'const FUNCTIONS_PY: &str = "\\\n(.*?)";\n', t, re.S)
    crudo = m.group(1).replace('\\"', '"').replace("\\\\", "\\")
    return crudo.replace("{{paquete}}", "ventas").replace("{{funcion}}", "billing_invoice_status")


def _cruda(nombre):
    """Una constante `r##"…"##` de `clases.rs`, con sus huecos rellenos."""
    t = io.open(os.path.join(RAIZ, "crates", "ore-core", "src", "clases.rs"), encoding="utf-8").read()
    m = re.search(r'const %s: &str = r##"(.*?)"##;' % nombre, t, re.S)
    return (m.group(1).replace("{{paquete}}", "ventas").replace("{{carpeta}}", "billing")
            .replace("{{funcion}}", "billing_invoice_status"))


class LaSemilla(unittest.TestCase):
    def test_nace_en_verde(self):
        """0050 P3: el repositorio sembrado pasa sus pruebas con pytest, como
        las corre la sesión: el ejemplo, su prueba y su pyproject."""
        with tempfile.TemporaryDirectory() as d:
            os.makedirs(os.path.join(d, "functions"))
            for rel, texto in [("functions/example.py", _semilla()),
                               ("functions/test_example.py", _cruda("TEST_PY")),
                               ("pyproject.toml", _cruda("PYPROJECT_FUNCTIONS_PY"))]:
                io.open(os.path.join(d, rel), "w", encoding="utf-8").write(texto)
            entorno = dict(os.environ, PYTHONPATH=os.path.join(RAIZ, "puesto", "python"), PYTHONDONTWRITEBYTECODE="1")
            r = subprocess.run([sys.executable, "-m", "pytest", "-q", "-p", "no:cacheprovider", d],
                               cwd=d, env=entorno, capture_output=True, text=True, timeout=120)
            self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
            self.assertIn("5 passed", r.stdout)

    def test_run_imprime_el_estado_de_la_factura(self):
        salida = io.StringIO()
        with contextlib.redirect_stdout(salida):
            exec(compile(_semilla(), "funciones/ejemplo.py", "exec"), {"__name__": "__main__"})
        self.assertEqual(
            salida.getvalue().strip(),
            "InvoiceStatus(status='overdue', outstanding=Decimal('120.50'), days=-17, surcharge=Decimal('1.02'))",
        )

    def test_invocada_no_corre_el_bloque_de_run(self):
        espacio = {"__name__": "ore_funcion"}
        salida = io.StringIO()
        with contextlib.redirect_stdout(salida):
            exec(compile(_semilla(), "funciones/ejemplo.py", "exec"), espacio)
        self.assertEqual(salida.getvalue(), "")
        f = espacio["billing_invoice_status"]
        self.assertEqual(f(amount="100", due="2026-10-10", paid=100, today="2026-10-02").status, "paid")
        self.assertEqual(f(amount=100, due="2026-10-10", today="2026-10-02").days, 8)


if __name__ == "__main__":
    unittest.main()
