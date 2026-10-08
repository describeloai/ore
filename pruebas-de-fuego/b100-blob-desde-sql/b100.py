# 0049 B10·0 · Bytes de una Function hasta SQL: ¿llegan como BLOB, con su ancla?
#
# Con Run en el mismo repositorio functions-python, después de publicar
# `pdf_a_png.py`. Sólo lee: no escribe nada en ninguna colección.
#
#   ① el tipo que DuckDB ve de `data` y de `anchor` (typeof);
#   ② cada página: nombre, tamaño, ancla, y que los bytes son un PNG de verdad;
#   ③ lo que tarda la consulta entera, y otra vez (en caliente).
import time

import ore

ORIGEN = "s3_stuff.nueva_carpeta.contratos"
PNG = b"\x89PNG\r\n\x1a\n"

CONSULTA = """
select c.path, p.name, p.data, p.anchor,
       typeof(p.data) as tipo_data, typeof(p.anchor) as tipo_anchor
from %s as c
cross join lateral functions.pdf_a_png(c.item) as p
where c.content_type = 'application/pdf'
""" % ORIGEN


def correr(etiqueta):
    t = time.perf_counter()
    df = ore.sql(CONSULTA)
    s = time.perf_counter() - t
    print("%s: %d filas en %.2f s" % (etiqueta, len(df), s))
    return df


# ── ① ② ───────────────────────────────────────────────────────────────────────
df = correr("① primera")
if df.empty:
    raise SystemExit("✗ ninguna fila: ¿hay PDF con content_type application/pdf en %s?" % ORIGEN)
print("   tipos en DuckDB: data %s · anchor %s" % (df["tipo_data"].iloc[0], df["tipo_anchor"].iloc[0]))

malos = 0
print("\n② las páginas")
for _, f in df.iterrows():
    d = f["data"]
    es_png = isinstance(d, (bytes, bytearray, memoryview)) and bytes(d[:8]) == PNG
    malos += not es_png
    print("   %-40s %-9s %7s B · %-28s %s" % (f["path"][-40:], f["name"], len(d) if d is not None else "—",
                                          dict(f["anchor"]) if f["anchor"] is not None else None,
                                          "PNG" if es_png else "✗ %s" % type(d).__name__))

# ── ③ ─────────────────────────────────────────────────────────────────────────
print()
correr("③ otra vez")

print("\n%s %d de %d páginas son PNG; data llega como %s" % (
    "✓" if not malos else "✗", len(df) - malos, len(df), df["tipo_data"].iloc[0]))
