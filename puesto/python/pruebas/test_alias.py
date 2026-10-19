"""SDK S1 · Los argumentos de antes (`modo=`, `como=`…) valen mientras S5 no los retire. Lo que vigila
esto: que cada `@_kw` esté sobre la función cuyos parámetros nombra. En 728c3ef8 una función nueva quedó
entre el decorador y `write`, el alias se lo llevó ella y `write(..., modo=…)` falló en el CI
(`el-puesto.sh` ⑩), sin que la suite del SDK lo viera."""

import inspect
import unittest

import ore


class LosAlias(unittest.TestCase):
    def test_cada_alias_nombra_un_parametro_de_su_funcion(self):
        vistos = 0
        for nombre, f in vars(ore).items():
            mapa = getattr(f, "__ore_kw__", None)
            if not mapa:
                continue
            vistos += 1
            params = inspect.signature(inspect.unwrap(f)).parameters
            for viejo, nuevo in mapa.items():
                self.assertIn(nuevo, params, "%s(): el alias `%s` → `%s`, que no es un parámetro suyo" % (nombre, viejo, nuevo))
        self.assertGreater(vistos, 5)

    def test_write_conserva_sus_alias(self):
        self.assertEqual(getattr(ore.write, "__ore_kw__", {}).get("modo"), "mode")
        self.assertEqual(ore.write.__name__, "write")


if __name__ == "__main__":
    unittest.main()
