"""v2-juzgar.py (ADR 0067 G0·3, caso V2): juzga lo que `ore discover` propuso para las fuentes del banco.

Lee el árbol inducido (`<ws>/packages/banco_geo_{pg,bq,gcs}`): el tipo de cada columna en su `Table` y
las preguntas `geo/…` de la cola (`discover.pending.json`). Compara con la verdad del banco y cuenta las
propuestas geo que no están en ella (los falsos positivos). Emite líneas «V2|…» para vara.py.

    python v2-juzgar.py <ws>          (en local, o en la celda: vara-celda.sh lo lleva dentro)
"""
import glob
import json
import os
import re
import sys

WS = sys.argv[1] if len(sys.argv) > 1 else "/tmp/v/ws"
# (fuente, tabla, columna, lo que tiene que proponerse, cómo se reconoce)
#   tipo:<T>  la columna sale con ese tipo en la Table
#   pregunta:<id>  la cola tiene esa pregunta geo
#   g3:<paso>  todavía no puede: lo pone en verde ese paso de G3
VERDAD = [
    ("pg", "municipios", "geom", "tipo:Geometry<MultiPolygon, EPSG:25830>"),
    ("pg", "lugares", "geom", "tipo:Geometry<Point, EPSG:4326>"),
    ("pg", "lugares_geog", "geog", "tipo:Geography<Point>"),
    ("pg", "carreteras", "geom", "tipo:Geometry<Any, EPSG:4326>"),
    ("bq", "lugares", "geo", "tipo:Geography"),
    ("bq", "municipios", "geo", "tipo:Geography"),
    ("gcs", "lugares_madrid_latlon", "lat+lon", "pregunta:geo/default.lugares_madrid_latlon"),
    ("gcs", "lugares_madrid_wkt", "wkt", "pregunta:geo/default.lugares_madrid_wkt.wkt"),
    ("gcs", "lugares_madrid_direcciones", "direccion", "pregunta:geo/default.lugares_madrid_direcciones:direccion"),
    ("gcs", "municipios_madrid", "geom", "g3:G3·3 (el driver de ficheros no lee la clave `geo` del GeoParquet)"),
] + [
    ("gcs", f"municipios_madrid.{e}", "geom", "g3:G3·3 (un objeto de medios, no una tabla)")
    for e in ("geojson", "gpkg", "kml", "fgb", "gdb", "shp.zip")
]


def columnas(fuente, tabla):
    """columna → el texto de su entrada en la Table inducida."""
    out = {}
    for p in glob.glob(f"{WS}/packages/banco_geo_{fuente}/**/tables/*.yaml", recursive=True):
        s = open(p, encoding="utf-8").read()
        if not re.search(rf"object:\s*\"?[\w.]*\b{re.escape(tabla)}\"?\s*$", s, re.M):
            continue
        for m in re.finditer(r"^    (\S+):\s*(\{.*\})\s*$", s, re.M):
            out[m.group(1).strip('"')] = m.group(2)
    return out


def preguntas(fuente):
    p = f"{WS}/packages/banco_geo_{fuente}/discover.pending.json"
    if not os.path.exists(p):
        return []
    return [x["id"] for x in json.load(open(p, encoding="utf-8")).get("pending", []) if str(x.get("id", "")).startswith("geo/")]


def tipo_de(entrada):
    m = re.search(r'\btype:\s*(?:"([^"]+)"|([^,}]+))', entrada or "")
    return (m.group(1) or m.group(2)).strip() if m else None


aciertos, puede = 0, 0
vistas = set()
for f, t, c, quiero in VERDAD:
    clase, valor = quiero.split(":", 1)
    if clase == "g3":
        print(f"V2|banco_geo_{f}|{t}|{c}|todavía no: {valor} · fallo esperado")
        continue
    puede += 1
    if clase == "tipo":
        e = columnas(f, t).get(c)
        visto = tipo_de(e)
        bien = visto == valor
        vistas.add((f, t, c))
        print(f"V2|banco_geo_{f}|{t}|{c}|quiero `{valor}` · propone `{visto}` · {'acierto' if bien else 'fallo'}")
    else:
        bien = valor in preguntas(f)
        vistas.add(valor)
        print(f"V2|banco_geo_{f}|{t}|{c}|quiero la pregunta `{valor}` · {'está' if bien else 'no está'} · {'acierto' if bien else 'fallo'}")
    aciertos += bien

# Lo geo propuesto que no está en la verdad.
falsos = []
for f in ("pg", "bq", "gcs"):
    falsos += [q for q in preguntas(f) if q not in vistas]
    for p in glob.glob(f"{WS}/packages/banco_geo_{f}/**/tables/*.yaml", recursive=True):
        for m in re.finditer(r"^    (\S+):\s*(\{.*\})\s*$", open(p, encoding="utf-8").read(), re.M):
            tt = tipo_de(m.group(2)) or ""
            if tt.startswith(("Geometry", "Geography")) and not any(
                    (f, x, m.group(1)) in vistas for x in [os.path.basename(p)] + [v[1] for v in vistas if isinstance(v, tuple)]):
                falsos.append(f"{f}:{os.path.basename(p)}:{m.group(1)}")
total = len(VERDAD)
precision = aciertos / (aciertos + len(falsos)) if aciertos + len(falsos) else 0.0
print(f"V2|resumen|-|-|exhaustividad {aciertos}/{total} ({aciertos}/{puede} de lo que G1 puede; el resto es de G3) · "
      f"precisión {precision:.2f} · falsos positivos {len(falsos)} {falsos[:5]}")
