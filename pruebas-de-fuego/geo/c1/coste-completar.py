"""ORE 0068 C1·1 pieza 4 · Lo que cuesta completar `st_` en el editor: la primera vez (carga spatial
y lee duckdb_functions) y las siguientes (de la caché), y cuánto ocupa la respuesta."""
import json, sys, time
sys.path.insert(0, "/repo/puesto/python")
from ore import lsp_sql
cat = lsp_sql.Catalogo({"items": {}})
for vez in ("primera", "segunda"):
    t = time.time()
    items, inc = lsp_sql.completar_lista("select st_", len("select st_"), cat)
    print(vez, "%.3f s" % (time.time() - t), len(items), "items", len(json.dumps(items)), "bytes", "incompleta" if inc else "")
print("funciones geo:", len(cat.funciones_geo()))
t = time.time(); items, _ = lsp_sql.completar_lista("select st_bu", len("select st_bu"), cat)
print("st_bu %.4f s" % (time.time() - t), [x[0] for x in items])
