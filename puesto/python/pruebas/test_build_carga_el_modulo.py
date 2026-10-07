"""Un build carga el módulo y llama él al `def` (ORE 0055 B1, D15).

    PYTHONPATH=puesto/python python -m unittest discover -s puesto/python/pruebas

Mientras el arnés del build ejecuta el fichero (`ore._modulo_en_carga(<ruta>)`),
un `@transform` llamado en el nivel superior no corre: serían dos escrituras, la
suya y la del build. Falla con la línea del fichero que lo llama. Después de
cargar, el `def` se llama como siempre y se declara al servidor.
"""

import json
import os
import unittest

import ore

FICHERO = "packages/ventas/etl/limpios.py"

FUENTE = '''from ore import transform


@transform(inputs=["ventas.clientes"], output="ventas.limpios")
def limpios():
    return {"rows": 3}


limpios()
'''

SIN_LLAMADA = FUENTE.replace("\n\nlimpios()\n", "\n")


class _Servidor:
    """Lo que `ore-serve` contesta a la declaración del transform."""

    def __init__(self):
        self.pedidos = []

    def pedir(self, metodo, ruta, cuerpo=None, plazo=30, cabeceras=None, seguir=True):
        self.pedidos.append((metodo, ruta.rsplit("/", 1)[-1], cuerpo))
        return 200, {}


def cargar(fuente):
    """Lo que hace el arnés: compilar con la ruta del fichero y ejecutar con D15 armado."""
    modulo = {"__name__": "ore_build", "__file__": FICHERO}
    ore._modulo_en_carga(FICHERO)
    try:
        exec(compile(fuente, FICHERO, "exec"), modulo)
    finally:
        ore._modulo_en_carga(None)
    return modulo


class UnBuildCargaElModulo(unittest.TestCase):
    def setUp(self):
        self.antes = ore.session.pedir
        self.servidor = _Servidor()
        ore.session.pedir = self.servidor.pedir

    def tearDown(self):
        ore.session.pedir = self.antes
        ore._modulo_en_carga(None)
        os.environ.pop("ORE_BUILD", None)

    def test_una_llamada_al_cargar_falla_con_su_linea(self):
        with self.assertRaises(ore.TransformCalledWhileLoading) as e:
            cargar(FUENTE)
        self.assertEqual(
            str(e.exception),
            "line 9 calls `limpios()` while the module loads; a Build calls it itself — remove the call",
        )
        self.assertEqual(e.exception.linea, 9)
        # Ni se declaró ni corrió: nada llegó al servidor.
        self.assertEqual(self.servidor.pedidos, [])
        # Y el SDK queda desarmado: una llamada después corre.
        self.assertIsNone(ore._cargando)

    def test_sin_llamada_el_build_llama_al_def_y_se_declara(self):
        modulo = cargar(SIN_LLAMADA)
        self.assertEqual(self.servidor.pedidos, [])
        self.assertEqual(modulo["limpios"](), {"rows": 3})
        self.assertEqual(
            [(m, r) for m, r, _ in self.servidor.pedidos],
            [("POST", "transform"), ("DELETE", "transform")],
        )
        self.assertEqual(self.servidor.pedidos[0][2]["output"], "ventas.limpios")

    def test_fuera_de_un_build_la_llamada_corre(self):
        modulo = {"__name__": "__main__", "__file__": FICHERO}
        exec(compile(FUENTE, FICHERO, "exec"), modulo)
        self.assertEqual(self.servidor.pedidos[0][1], "transform")

    def test_la_procedencia_dice_de_que_build_es(self):
        b = {"id": "trabajo-ana-1", "transform": "packages/ventas/etl/pipeline/ventas.limpios.yaml",
             "entrypoint": "etl/limpios.py:limpios", "commit": "abc1234", "output": "ventas.limpios"}
        os.environ["ORE_BUILD"] = json.dumps(b)
        self.assertEqual(ore._procedencia("ventas.limpios")["build"], b)
        os.environ.pop("ORE_BUILD")
        self.assertNotIn("build", ore._procedencia("ventas.limpios"))


class ElInformeDeLaCelda(unittest.TestCase):
    """B2: lo que un build deja para su informe —filas, snapshot— lo toma el
    agente al terminar la celda, una vez."""

    def tearDown(self):
        os.environ.pop("ORE_BUILD", None)
        ore._tomar_informe()

    def test_en_un_build_la_escritura_deja_sus_filas_y_su_snapshot(self):
        os.environ["ORE_BUILD"] = "{}"
        ore._resultado_de_escritura({"rows": 3, "added": 3, "snapshot": "8812", "mode": "overwrite"})
        self.assertEqual(ore._tomar_informe(), {"filas": 3, "snapshot": "8812"})
        self.assertIsNone(ore._tomar_informe())

    def test_fuera_de_un_build_no_deja_nada(self):
        ore._resultado_de_escritura({"rows": 3, "added": 3, "snapshot": "8812", "mode": "overwrite"})
        self.assertIsNone(ore._tomar_informe())


if __name__ == "__main__":
    unittest.main()
