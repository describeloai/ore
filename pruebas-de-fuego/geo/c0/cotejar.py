"""ORE 0068 C0 · Cotejar los motores: una tabla por conjunto con segundos, filas y si la huella coincide.

    python cotejar.py resultados/*.jsonl
Para cada (conjunto, datos, caso) toma la última corrida de cada motor.
"""
import json, sys

ultimas = {}
for f in sys.argv[1:]:
    for l in open(f, encoding="utf-8"):
        r = json.loads(l)
        ultimas[(r["conjunto"], r["datos"], r["caso"], r["motor"])] = r

claves = sorted({k[:3] for k in ultimas}, key=lambda k: (k[0], k[1], int("".join(ch for ch in k[2] if ch.isdigit())), k[2]))


def celda(r):
    if not r:
        return "—"
    if r["estado"] == "bien":
        return "%.2f s (%d)" % (r["segundos"], r["filas"])
    return "%s %s" % (r["estado"], (r.get("error") or "")[:60])


print("| conjunto | datos | caso | DuckDB | Spark + Sedona | ¿igual? |")
print("|---|---|---|---|---|---|")
for c, d, q in claves:
    a, b = ultimas.get((c, d, q, "duckdb")), ultimas.get((c, d, q, "spark"))
    igual = "—"
    if a and b and a["estado"] == b["estado"] == "bien":
        igual = "sí" if a["huella"] == b["huella"] else "**no**: %s / %s" % (a["muestra"][:1], b["muestra"][:1])
    print(f"| {c} | {d.split('/')[-1]} | {q} | {celda(a)} | {celda(b)} | {igual} |")
