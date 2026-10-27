"""El plan de una sentencia, sin ejecutarla (Query A1, docs/query.md §3.7).

    PYTHONPATH=puesto/python python -m unittest discover -s puesto/python/pruebas

`ore._plan(texto)` es lo que el agente contesta cuando la admisión de `ore-serve` pide
`POST /puestos/{id}/plan`: el `EXPLAIN` de DuckDB con lo que la sentencia nombra
registrado, sin leer nada en vivo y sin apuntar nada como leído.
"""

import unittest

import ore


def nodos(n, out):
    if isinstance(n, list):
        for x in n:
            nodos(x, out)
    elif isinstance(n, dict):
        ex = n.get("extra_info") or {}
        out.append((n.get("name"), ex.get("Estimated Cardinality") if isinstance(ex, dict) else None))
        for h in n.get("children") or []:
            nodos(h, out)
    return out


class _Servidor:
    def __init__(self, fuentes):
        self.fuentes, self.pedidos = fuentes, []

    def pedir(self, metodo, ruta, cuerpo=None, plazo=30, cabeceras=None, seguir=True):
        self.pedidos.append((metodo, ruta, cuerpo))
        return 200, {"fuentes": self.fuentes}


class ElPlan(unittest.TestCase):
    def setUp(self):
        self.antes = (ore.session.pedir, ore.session.id)

    def tearDown(self):
        ore.session.pedir, ore.session.id = self.antes

    def test_lo_que_se_genera_tiene_su_peso(self):
        r = ore._plan("select range, hash(range) from range(300000000)")
        self.assertNotIn("error", r)
        self.assertIn(("RANGE", "300000000"), [(a, str(b).replace("~", "")) for a, b in nodos(r["plan"], [])])

    def test_lo_vivo_no_se_lee_y_se_dice(self):
        s = _Servidor({"o.s.clientes": {"federada": {"tabla": "o.s.clientes", "columnas": ["id", "pais"]}}})
        ore.session.pedir, ore.session.id = s.pedir, "puesto-ana-python"
        r = ore._plan("select pais, count(*) from o.s.clientes group by pais")
        self.assertNotIn("error", r)
        self.assertEqual(r["en_vivo"], ["o.s.clientes"])
        # Se resolvió como un plan: ore-serve no lo apunta como leído.
        self.assertEqual([(m, c) for m, _, c in s.pedidos],
                         [("POST", {"texto": "select pais, count(*) from o.s.clientes group by pais", "plan": True})])

    def test_el_operador_mayor(self):
        r = ore._plan("select range from range(300000000) where range % 2 = 0")
        self.assertGreaterEqual(ore._filas_del_plan(r["plan"]), 100000000)

    def test_una_sentencia_rota_no_tiene_plan(self):
        self.assertIn("error", ore._plan("select from donde"))


if __name__ == "__main__":
    unittest.main()
