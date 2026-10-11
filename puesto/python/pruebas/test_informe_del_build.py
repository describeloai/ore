"""0055 · Lo que un build dice al acabar (`ore._informe_del_build`), lo mismo que Java
(`ore.Arnes.informeDelBuild`): la línea del log según lo que devolvió el `def` —un `write()`, un
`apply()` o el commit de una colección— y, en el informe, el resumen del `apply()` y la muestra
de lo escrito, leída del lago.

Medido en victor (2026-10-11): el segundo build de un `apply()` no recalculó nada y el panel sólo
decía «4 rows»; y el Build no enseñaba ninguna fila de lo que escribió.

    python -m unittest discover -s puesto/python/pruebas -p 'test_informe_del_build.py'
"""
import contextlib
import io
import unittest

import pandas as pd

import ore


def informe_de(hecho, sql=None):
    """La línea impresa y el informe que deja `_informe_del_build`, con `sql` cambiado."""
    ore._tomar_informe()
    de_antes = ore.sql
    if sql is not None:
        ore.sql = sql
    impreso = io.StringIO()
    try:
        with contextlib.redirect_stdout(impreso):
            ore._informe_del_build("sandbox.default.salida", "packages/p/t.py", "t", hecho)
    finally:
        ore.sql = de_antes
    return impreso.getvalue(), ore._tomar_informe() or {}


class InformeDelBuild(unittest.TestCase):
    def test_un_apply_sin_nada_nuevo_dice_su_resumen(self):
        linea, informe = informe_de({"items": 4, "new": 0, "recomputed": 0, "skipped": 4, "errors": 0,
                                     "removed": 0, "rows": 4, "written": False},
                                    sql=lambda q, **_: pd.DataFrame({"pagina": [1, 2]}))
        self.assertIn("sandbox.default.salida · built from packages/p/t.py:t · 4 rows · new 0 · recomputed 0 · skipped 4 · nothing new",
                      linea)
        self.assertEqual(informe["apply"]["skipped"], 4)
        self.assertIs(informe["apply"]["written"], False)
        # La muestra: de lo que hay en el lago, con el total del apply.
        self.assertEqual([c["name"] for c in informe["muestra"]["columnas"]], ["pagina"])
        self.assertEqual(informe["muestra"]["total"], 4)

    def test_un_write_dice_sus_filas_y_lee_la_muestra_del_lago(self):
        consultas = []

        def sql(q, **_):
            consultas.append(q)
            return pd.DataFrame({"pais": ["ES", "FR"], "n": [3, 2]})

        linea, informe = informe_de({"rows": 5, "repeated": False}, sql=sql)
        self.assertIn("· 5 rows", linea)
        self.assertNotIn("nothing new", linea)
        self.assertEqual(consultas, ["select * from sandbox.default.salida limit 100"])
        self.assertEqual(informe["muestra"]["filas"], [["ES", 3], ["FR", 2]])
        self.assertEqual(informe["muestra"]["total"], 5)
        self.assertNotIn("apply", informe)

    def test_una_muestra_que_no_se_lee_no_tumba_el_build(self):
        def falla(q, **_):
            raise RuntimeError("sin copia")

        linea, informe = informe_de({"rows": 5}, sql=falla)
        self.assertIn("· 5 rows", linea)
        self.assertIn("could not be read: sin copia", linea)
        self.assertNotIn("muestra", informe)

    def test_lo_que_no_es_un_write_lo_dice(self):
        linea, informe = informe_de(None)
        self.assertIn("the def did not return what `write()` returns", linea)
        self.assertEqual(informe, {})


if __name__ == "__main__":
    unittest.main()
