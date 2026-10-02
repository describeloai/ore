"""El contrato de un `@function` (ORE 0050 G3): los tipos que anota, cumplidos.

    PYTHONPATH=puesto/python python -m unittest discover -s puesto/python/pruebas

Medido antes (2026-10-02, t-victor): `importe: Decimal` llegaba como `float` y
`fecha: date` como `str`, y la función calculó mal sin ningún error.
"""

import dataclasses
import datetime
import json
import unittest
from decimal import Decimal

from ore import function
from ore.contrato import ErrorDeContrato, convertir


@dataclasses.dataclass
class Eco:
    tipo_importe: str
    tipo_fecha: str
    importe: Decimal
    dias: int
    nota: str | None = None


@function
def eco_tipos(importe: Decimal, fecha: datetime.date, etiquetas: list[str] = []) -> Eco:  # noqa: B006
    dias = (datetime.date(2026, 12, 31) - fecha).days
    return Eco(type(importe).__name__, type(fecha).__name__, importe, dias, ",".join(etiquetas) or None)


@function
def valor_mal(x: int) -> str:
    return x * 2


@function
def falla(x: int) -> int:
    raise ValueError("a propósito con x=%d" % x)


@function(over="hr.numeros_v")
def por_fila(fila, umbral: int) -> bool:
    return fila["n"] > umbral


class LoQueLlegaAlDef(unittest.TestCase):
    def test_lo_que_viaja_en_json_llega_del_tipo_que_anota(self):
        # Lo que el arnés le pasa: el JSON de la invocación, con los números exactos.
        p = json.loads('{"importe": 12.50, "fecha": "2026-10-02"}', parse_float=Decimal)
        e = eco_tipos(**p)
        self.assertEqual((e.tipo_importe, e.tipo_fecha), ("Decimal", "date"))
        self.assertEqual(e.importe, Decimal("12.50"))
        self.assertEqual(e.dias, 90)

    def test_lo_que_ya_es_del_tipo_pasa_tal_cual(self):
        e = eco_tipos(Decimal("1"), datetime.date(2026, 12, 30), ["a", "b"])
        self.assertEqual((e.dias, e.nota), (1, "a,b"))

    def test_un_float_es_un_decimal_exacto_y_un_int_tambien(self):
        self.assertEqual(eco_tipos(12.5, "2026-12-31").importe, Decimal("12.5"))
        self.assertEqual(eco_tipos(3, "2026-12-31").importe, Decimal(3))

    def test_no_se_adivina(self):
        with self.assertRaisesRegex(ErrorDeContrato, "`fecha` es `date`.*AAAA-MM-DD"):
            eco_tipos(1, "02/10/2026")
        with self.assertRaisesRegex(ErrorDeContrato, "`x` es `int`"):
            valor_mal("3")
        with self.assertRaisesRegex(ErrorDeContrato, "`x` es `int`"):
            valor_mal(True)
        with self.assertRaisesRegex(ErrorDeContrato, r"`etiquetas\[1\]` es `str`"):
            eco_tipos(1, "2026-12-31", ["a", 2])

    def test_lo_que_devuelve_es_del_tipo_que_anota(self):
        with self.assertRaisesRegex(ErrorDeContrato, "`valor_mal` devolvió 8 y anota `-> str`"):
            valor_mal(4)

    def test_una_dataclass_se_comprueba_campo_a_campo(self):
        @function
        def mala() -> Eco:
            return Eco("x", "y", "no es decimal", 1)

        with self.assertRaisesRegex(ErrorDeContrato, "`mala` devolvió un `Eco` con `importe` es `Decimal`"):
            mala()

    def test_los_opcionales_admiten_none(self):
        self.assertIsNone(eco_tipos(1, "2026-12-31").nota)
        self.assertIsNone(convertir("n", None, int | None))
        with self.assertRaisesRegex(ErrorDeContrato, "llegó None"):
            convertir("n", None, int)

    def test_lo_que_falla_dentro_sale_como_es(self):
        with self.assertRaisesRegex(ValueError, "a propósito con x=3"):
            falla(3)

    def test_la_fila_de_over_no_se_toca(self):
        self.assertTrue(por_fila({"n": 5}, Decimal("2")))
        self.assertEqual(por_fila.__ore_function__["over"], "hr.numeros_v")

    def test_sigue_siendo_su_def(self):
        self.assertEqual(eco_tipos.__name__, "eco_tipos")
        self.assertEqual(eco_tipos.__wrapped__.__name__, "eco_tipos")
        self.assertTrue(eco_tipos.__ore_contrato__)

    def test_fechas_y_horas(self):
        self.assertEqual(convertir("t", "2026-10-02T08:00:00Z", datetime.datetime).tzinfo, datetime.timezone.utc)
        self.assertEqual(convertir("t", "08:30", datetime.time), datetime.time(8, 30))
        with self.assertRaisesRegex(ErrorDeContrato, "`d` es `date`"):
            convertir("d", datetime.datetime(2026, 1, 1), datetime.date)


if __name__ == "__main__":
    unittest.main()


class LlamarUnaPublicada(unittest.TestCase):
    """`ore.funcion("p.def")`: el código de su `entrypoint`, aquí y con su contrato."""

    CODIGO = (
        "from decimal import Decimal\nfrom ore import function\n\n\n"
        "@function\ndef doble(x: Decimal) -> Decimal:\n    return x * 2\n\n\n"
        "@function(over='hr.v')\ndef fila(f, n: int) -> int:\n    return n\n"
    )

    def setUp(self):
        import ore

        self.ore = ore
        ore._FUNCIONES.clear()
        self.pedidas = []
        docs = {
            "/documentos/Function/ventas/doble": {"paquete": "ventas", "spec": {"runtime": "python", "entrypoint": "riesgo/f.py:doble"}},
            "/documentos/Function/ventas/fila": {"paquete": "ventas", "spec": {"runtime": "python", "entrypoint": "riesgo/f.py:fila", "over": "hr.v"}},
        }

        def pedir(metodo, ruta, **_):
            self.pedidas.append(ruta)
            if ruta in docs:
                return 200, docs[ruta]
            if ruta == "/arbol/packages/ventas/riesgo/f.py":
                return 200, {"texto": self.CODIGO}
            return 404, {"error": "no"}

        self._pedir, ore.puesto.pedir = ore.puesto.pedir, pedir

    def tearDown(self):
        self.ore.puesto.pedir = self._pedir

    def test_se_llama_con_su_contrato(self):
        doble = self.ore.funcion("ventas.doble")
        self.assertEqual(doble("1.25"), Decimal("2.50"))
        self.assertEqual(self.pedidas, ["/documentos/Function/ventas/doble", "/arbol/packages/ventas/riesgo/f.py"])
        self.assertIs(self.ore.funcion("ventas.doble"), doble)  # una vez por sesión

    def test_las_que_no_se_llaman_asi(self):
        with self.assertRaisesRegex(NotImplementedError, "pipeline"):
            self.ore.funcion("ventas.fila")
        with self.assertRaisesRegex(LookupError, "ninguna función `ventas.nadie`"):
            self.ore.funcion("ventas.nadie")
        with self.assertRaisesRegex(ValueError, "<paquete>.<def>"):
            self.ore.funcion("sinpunto")
