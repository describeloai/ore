"""El editor SQL con `system.default.executions` en el índice (docs/ejecuciones.md X2).

`system` es un catálogo de DuckDB, de solo lectura: el editor crea un schema por cada
base del índice, y con `system` DuckDB dice «Cannot create schema in system catalog».
Visto en victor el 2026-10-24: el índice entero fallaba y el editor de Query se quedaba
sin nombres. Ahora se registra como `__ore_system`, como en `sql()`, y la sentencia se
comprueba reescrita; lo que se subraya sigue en su sitio del texto.
"""
import unittest

from ore import lsp_sql

INDICE = {"items": {
    "x": {"kind": "Dataset", "paquete": "system", "schema": "default", "name": "executions",
          "expone": [{"name": "id", "type": "String"}, {"name": "peak_memory_mb", "type": "Integer"}]},
    "v": {"kind": "Dataset", "paquete": "ventas", "schema": "default", "name": "pedidos",
          "expone": [{"name": "id", "type": "Integer"}, {"name": "total", "type": "Double"}]},
}}


def errores(sql):
    return [d for d in lsp_sql.diagnosticar(sql, lsp_sql.Catalogo(INDICE)) if d.get("severity") == 1]


class SystemEnElEditor(unittest.TestCase):
    def test_el_indice_con_system_se_carga_entero(self):
        cat = lsp_sql.Catalogo(INDICE)  # antes: «Cannot create schema in system catalog»
        self.assertIn("system", cat.paquetes)
        self.assertIn("ventas", cat.paquetes)

    def test_leer_executions_no_se_subraya(self):
        self.assertEqual(errores("select id, peak_memory_mb from system.default.executions"), [])
        self.assertEqual(errores("select count(*) from system.executions"), [])

    def test_lo_demas_sigue_comprobandose(self):
        self.assertEqual(errores("select total from ventas.default.pedidos"), [])
        self.assertTrue(errores("select nada from ventas.default.pedidos"))

    def test_lo_que_se_subraya_esta_en_su_sitio_y_dice_system(self):
        sql = "select p.id from system.default.executions e join ventas.default.pedidos p on e.id = p.nada"
        d = errores(sql)
        self.assertEqual(len(d), 1, d)
        ini = d[0]["range"]["start"]["character"]
        self.assertEqual(sql[ini:ini + len("p.nada")], "p.nada", d)
        self.assertNotIn("__ore_system", d[0]["message"])

    def test_system_en_una_cadena_no_se_toca(self):
        s, sitios = lsp_sql.con_system_de_ore("select 'system.x' as t, e.id from system.default.executions e")
        self.assertEqual(len(sitios), 1)
        self.assertIn("'system.x'", s)
        self.assertIn("__ore_system.default.executions", s)


if __name__ == "__main__":
    unittest.main()
