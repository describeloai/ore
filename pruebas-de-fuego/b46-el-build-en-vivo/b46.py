# 0057 B4·6 · Build sobre una foreign database, en vivo (victor).
#
# Va en el repositorio `transforms_python_new` (test_project), como
# `transforms/b46.py`. Commit y Build: dos builds, uno por transform. Leen en
# vivo la tabla sintética de BigQuery y escriben en `sandbox`, el lago.
#
# - b46_sintetica: por la vista de la foreign database `bq_foreign` (la vista
#   declarada cubre la tabla que lee).
# - b46_por_fuente: la misma tabla por el nombre de su fuente, agregada.
from ore import transform, sql, over, write

SINTETICA = "bq_foreign.ventas.ore_e2e_sintetica"
FUENTE = "bigquery_20260927_1428.ventas.ore_e2e_sintetica"


@transform(inputs=[SINTETICA], output="sandbox.b46_sintetica")
def b46_sintetica():
    """La tabla sintética entera, leída en vivo por la foreign database."""
    return write("sandbox.b46_sintetica", over(SINTETICA))


@transform(inputs=[FUENTE], output="sandbox.b46_por_fuente")
def b46_por_fuente():
    """Por la fuente, agregada en el build."""
    return write("sandbox.b46_por_fuente", sql(f"select nombre, count(*) as n, sum(total) as total from {FUENTE} group by nombre"))
