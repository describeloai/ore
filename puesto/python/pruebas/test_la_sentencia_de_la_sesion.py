"""El SQL de la sesión no finge un transform (Escrituras E3).

    PYTHONPATH=puesto/python python -m unittest discover -s puesto/python/pruebas

La celda que `ore-serve` genera para una sentencia SQL que escribe —desde una
hoja de Query o una celda `sql`— corre en `with ore._sentencia(...)`: acota lo
que lee y escribe como un `@transform` (aquí y en el servidor), pero dice la
sentencia y su hoja, y lo escrito no lleva un `transform` que no existe.
"""

import unittest

import ore


class _Servidor:
    """Lo que `ore-serve` contesta a la declaración del alcance."""

    def __init__(self):
        self.pedidos = []

    def pedir(self, metodo, ruta, cuerpo=None, plazo=30, cabeceras=None, seguir=True):
        self.pedidos.append((metodo, ruta.rsplit("/", 1)[-1], cuerpo))
        return 200, {}


class LaSentenciaDeLaSesion(unittest.TestCase):
    def setUp(self):
        self.antes = ore.session.pedir
        self.servidor = _Servidor()
        ore.session.pedir = self.servidor.pedir

    def tearDown(self):
        ore.session.pedir = self.antes
        ore._transform = None

    def test_se_declara_con_su_texto_y_su_hoja_y_se_retira(self):
        texto = "create or replace dataset ventas.resumen as select * from ventas.default.pedidos"
        with ore._sentencia(inputs=["ventas.default.pedidos"], output="ventas.resumen",
                            statement=texto, code="queries/resumen.sql"):
            self.assertEqual(ore._transform.inputs, ["ventas.pedidos"])
            self.assertEqual(ore._transform.output, "ventas.resumen")
        self.assertIsNone(ore._transform)
        self.assertEqual([(m, r) for m, r, _ in self.servidor.pedidos],
                         [("POST", "transform"), ("DELETE", "transform")])
        dicho = self.servidor.pedidos[0][2]
        self.assertEqual(dicho["sentencia"], texto)
        self.assertEqual(dicho["codigo"], "queries/resumen.sql")
        self.assertEqual(dicho["inputs"], ["ventas.pedidos"])

    def test_acota_como_un_transform(self):
        with ore._sentencia(inputs=["ventas.pedidos"], output="ventas.resumen",
                            statement="…", code="queries/x.sql"):
            with self.assertRaises(PermissionError):
                ore.write("ventas.otra", [{"a": 1}])

    def test_lo_escrito_no_dice_un_transform(self):
        with ore._sentencia(inputs=["ventas.pedidos"], output="ventas.resumen",
                            statement="…", code="queries/x.sql"):
            p = ore._procedencia("ventas.resumen")
        self.assertNotIn("transform", p)
        self.assertNotIn("inputs", p)
        self.assertEqual(p["leidas"], ["ventas.pedidos"])

    def test_se_retira_aunque_falle(self):
        with self.assertRaises(ValueError):
            with ore._sentencia(inputs=[], output="ventas.resumen", statement="…", code="x.sql"):
                raise ValueError("la consulta falló")
        self.assertIsNone(ore._transform)
        self.assertEqual(self.servidor.pedidos[-1][:2], ("DELETE", "transform"))

    def test_no_se_anida(self):
        with ore._sentencia(inputs=[], output="ventas.a", statement="…", code="x.sql"):
            with self.assertRaises(RuntimeError):
                with ore._sentencia(inputs=[], output="ventas.b", statement="…", code="x.sql"):
                    pass


if __name__ == "__main__":
    unittest.main()
