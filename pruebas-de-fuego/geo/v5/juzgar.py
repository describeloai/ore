"""ADR 0067 G2·5 · Juzga las líneas `V5R|…` de los lectores contra los fijos y dibuja la matriz.

    python juzgar.py <fijos.json> <crs> < salida-de-los-lectores

Verde: lee sin conversión, la columna llega como geometría, con el mismo CRS, las mismas filas y los
mismos WKB por `id`. Ámbar: lee, pero pierde algo (se dice qué). Rojo: no lee.
Escribe `V5|<lector>|<artefacto>-<crs>|-|<color>: <motivo>` (lo que lee la vara) y la tabla.
"""
import json
import sys

EQUIVALENTES = {
    # EPSG:4326 solo cambia el orden de los ejes en su definición; el WKB va siempre en lon/lat, y
    # Spark (srid 4326) y GDAL lo nombran así: se acepta, y el motivo lo dice.
    "OGC:CRS84": {"OGC:CRS84", "CRS84", "OGC:84", "WGS 84 (CRS84)", "EPSG:4326"},
    "EPSG:25830": {"EPSG:25830"},
}


def juzgar(esp, crs, d):
    if "error" in d:
        return "rojo", d["error"]
    faltas = []
    if not d.get("geo"):
        faltas.append("llega como %s, no como geometría" % d.get("tipo"))
    visto = d.get("crs")
    if visto not in EQUIVALENTES[crs]:
        faltas.append("CRS %s" % (visto or "desconocido"))
    filas = esp["filas"]
    if d.get("wkb") is None and "filas" in d:
        if d["filas"] != len(filas):
            faltas.append("%d filas de %d" % (d["filas"], len(filas)))
        faltas.append(d.get("nota", "sin WKB"))
        return "ámbar", "; ".join(faltas)
    wkb = d.get("wkb") or {}
    if len(wkb) != len(filas):
        faltas.append("%d filas de %d" % (len(wkb), len(filas)))
    distintos = {}
    for f in filas:
        w = wkb.get(str(f["id"]))
        e = f["wkb"].upper() if f["wkb"] else None
        if w != e:
            distintos.setdefault(f["subtipo"], 0)
            distintos[f["subtipo"]] += 1
    if distintos:
        faltas.append("WKB distintos: " + ", ".join("%s ×%d" % kv for kv in sorted(distintos.items())))
    if d.get("nota"):
        faltas.append(d["nota"])
    if not faltas:
        return "verde", "%s, %s, %d filas" % (d.get("tipo"), visto, len(wkb))
    return "ámbar", "; ".join(faltas)


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8")
    fijos = json.load(open(sys.argv[1], encoding="utf-8"))
    crs = sys.argv[2]
    esp = fijos[crs]
    celdas = {}
    for l in sys.stdin:
        if not l.startswith("V5R|"):
            continue
        _, lector, art, j = l.rstrip("\n").split("|", 3)
        color, motivo = juzgar(esp, crs, json.loads(j))
        celdas[(lector, art)] = (color, motivo)
        print("V5|%s|%s-%s|-|%s: %s" % (lector, art, crs, color, motivo))
    arts = sorted({a for _, a in celdas})
    lectores = sorted({l for l, _ in celdas}, key=lambda x: list(dict.fromkeys(k[0] for k in celdas)).index(x))
    sigla = {"verde": "🟢", "ámbar": "🟡", "rojo": "🔴"}
    print("\n| %s | %s |" % (crs, " | ".join(arts)))
    print("|---|%s|" % "|".join("---" for _ in arts))
    for l in lectores:
        print("| %s | %s |" % (l, " | ".join(sigla[celdas[(l, a)][0]] if (l, a) in celdas else "—" for a in arts)))
