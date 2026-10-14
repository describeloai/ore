"""I4 · la idempotencia en producción (ADR 0031, «La idempotencia, revisada»), en un puesto REAL.

Corre dentro del pod de un puesto abierto DESPUÉS del despliegue (el SDK de la
imagen nueva), como proceso aparte del agente, con la identidad del pod:

    kubectl exec -n t-<celda> <pod-del-puesto> -c puesto -- python3 /tmp/i4.py

Lo que afirma:
  1 · A → B → A: 1000 → 2000 → 1000 → 3000 → 1000 deja cada vez lo escrito
      (antes la tercera y la quinta se perdían sin aviso);
  2 · escribir lo que la tabla ya tiene no deja snapshot (`repeated: true`);
  3 · cada snapshot nuevo lleva su `ore.idempotency-key` (un UUID): el SDK la
      manda y el catálogo la guarda;
  4 · un ciclo en el linaje —leer A, escribir B, leer B, escribir A— y un
      `sql()` después no tumban ore-serve (antes: desbordamiento de pila).
"""
import json
import os
import re
import sys
import time
import urllib.request

sys.path.insert(0, "/opt/ore")
import ore  # noqa: E402
import pandas as pd  # noqa: E402

# Fuera del agente el SDK no tiene identidad: la del pod (R1), como el agente.
ore.session._proveedor = lambda: {"x-ore-pod": open("/var/run/ore/pod/token").read().strip()}

TABLA = os.environ.get("I4_TABLA", "sandbox.m0d_marimo")
CICLO = os.environ.get("I4_CICLO", "sandbox.i4_ciclo")
UUID = re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$")
R = {"puesto": os.environ.get("PUESTO"), "fallos": []}


def falla(m):
    R["fallos"].append(m)
    print("✗", m, flush=True)


def ok(m):
    print("✓", m, flush=True)


def snapshots(nombre):
    partes = nombre.split(".")
    base, schema, t = partes if len(partes) == 3 else (partes[0], "default", partes[1])
    c, r = ore.session.pedir("GET", "/v1/%s/namespaces/%s/tables/%s" % (base, schema, t))
    assert c == 200, (c, r)
    md = r["metadata"]
    return md.get("current-snapshot-id"), md.get("snapshots", [])


# ── 1 y 3 · A → B → A, y la clave en cada snapshot nuevo ──────────────────
pasos = []
for n in (1000, 2000, 1000, 3000, 1000):
    t0 = time.time()
    r = ore.write(TABLA, pd.DataFrame({"i": range(n), "doble": [2 * x for x in range(n)]}))
    filas = len(ore.over(TABLA))
    actual, ss = snapshots(TABLA)
    vigente = next((s for s in ss if s.get("snapshot-id") == actual), {})
    clave = (vigente.get("summary") or {}).get("ore.idempotency-key", "")
    pasos.append({"escrito": n, "repeated": r.get("repeated"), "filas": filas, "clave": clave, "ms": round((time.time() - t0) * 1000)})
    if filas != n:
        falla("A → B → A: escribí %d y la tabla tiene %d" % (n, filas))
    if not r.get("repeated") and not UUID.match(clave):
        falla("el snapshot de write(%d) no lleva una Idempotency-Key UUIDv7: %r" % (n, clave))
R["a_b_a"] = pasos
if not R["fallos"]:
    ok("A → B → A: 1000 → 2000 → 1000 → 3000 → 1000, la tabla tiene cada vez lo escrito y cada snapshot su clave")

# ── 2 · lo que la tabla ya tiene no deja snapshot ─────────────────────────
_, antes = snapshots(TABLA)
r = ore.write(TABLA, pd.DataFrame({"i": range(1000), "doble": [2 * x for x in range(1000)]}))
_, despues = snapshots(TABLA)
R["sin_cambios"] = {"repeated": r.get("repeated"), "snapshots_antes": len(antes), "snapshots_despues": len(despues)}
if r.get("repeated") and len(antes) == len(despues):
    ok("lo que la tabla ya tiene: repeated, sin snapshot nuevo")
else:
    falla("lo mismo que la tabla tiene dejó snapshot o no dijo repeated: %s" % R["sin_cambios"])

# ── 4 · un ciclo en el linaje no tumba ore-serve ──────────────────────────
A, B = CICLO + "_a", CICLO + "_b"
ore.write(A, pd.DataFrame({"x": [1, 2, 3]}))
ore.write(B, ore.over(A))           # B ← A
# A ← B: el ciclo. Con otro contenido: lo mismo que A ya tiene no deja
# snapshot (I1) y entonces no habría ciclo que probar.
ore.write(A, ore.over(B).assign(x=lambda d: d["x"] * 10))
try:
    n = ore.sql("select count(*) as n from %s" % A)
    R["ciclo_sql"] = int(n.iloc[0, 0])
except Exception as e:  # noqa: BLE001
    falla("sql() tras el ciclo: %s" % e)
with urllib.request.urlopen(ore.session.servidor + "/salud", timeout=10) as s:
    R["salud_tras_el_ciclo"] = s.status
if R.get("salud_tras_el_ciclo") == 200 and R.get("ciclo_sql") == 3:
    ok("el ciclo A ← B ← A y un sql() después: ore-serve sigue en pie (/salud 200)")
else:
    falla("tras el ciclo: %s" % {k: R.get(k) for k in ("ciclo_sql", "salud_tras_el_ciclo")})

print("I4-RESULTADO " + json.dumps(R, ensure_ascii=False, default=str))
sys.exit(1 if R["fallos"] else 0)
