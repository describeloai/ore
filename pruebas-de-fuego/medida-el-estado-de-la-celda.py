# -*- coding: utf-8 -*-
"""MEDIDA · qué puede contestar `GET /estado` de una celda, y quién lo sabe.

Antes de escribir `GET /estado` en `ore-serve` (la overview del clúster, 0026):
medir qué dato existe hoy, dónde vive y quién puede leerlo. Tres columnas:

    A  el árbol      lo que `ore-serve` ya alcanza: la forja (clona por `git`)
    B  kubernetes    cuota usada/dura, jobs por estado, pods — lo sabe el API
                     server, y `ore-serve` NO lo alcanza: la `NetworkPolicy`
                     de la celda deniega el egress salvo DNS/forja/cofre/
                     control/IdP, y el proceso no tiene TLS (Dockerfile:
                     «tres cerraduras»). Medido el 2026-09-15: un `wget` al
                     API server desde el pod se queda colgado.
    C  nadie         tamaño de vistas materializadas (no hay R2 en la celda),
                     coste, latencia/throughput (sin métricas)

Sale una tabla `campo · fuente · valor hoy`. Es un METRO, no un trinquete.

    uso:  python pruebas-de-fuego/medida-el-estado-de-la-celda.py [celda]
"""
import json
import os
import subprocess
import sys

CELDA = sys.argv[1] if len(sys.argv) > 1 else "victor"
NS = "t-" + CELDA


def kubectl(*args):
    r = subprocess.run(["kubectl", *args, "-o", "json"], capture_output=True, text=True, encoding="utf-8")
    if r.returncode != 0:
        return None
    return json.loads(r.stdout)


def curl(url):
    r = subprocess.run(["curl", "-s", "-m", "8", url], capture_output=True, text=True, encoding="utf-8")
    return r.stdout if r.returncode == 0 else None


filas = []


def fila(campo, fuente, valor):
    filas.append((campo, fuente, valor))


# ── A · el árbol ─────────────────────────────────────────────────────────────
v = curl("https://%s.ore.paladio.io/version" % CELDA)
try:
    vj = json.loads(v or "{}")
    fila("motor (versión)", "A ore-serve /version", vj.get("motor", "").split("\n")[0])
    fila("árbol (dónde)", "A ore-serve /version", vj.get("arbol"))
except ValueError:
    fila("motor (versión)", "A ore-serve /version", "✗ no contesta")
s = curl("https://%s.ore.paladio.io/salud" % CELDA)
fila("salud", "A ore-serve /salud", "ok" if s and '"ok":true' in s else "✗")
g = kubectl("get", "gitrepository", "-n", "flux-system", "trabajo-" + CELDA)
if g:
    a = g["status"].get("artifact", {})
    fila("último commit del árbol", "A forja (Flux lo ve; ore-serve lo clona)", "%s · %s" % (a.get("revision", "?").split(":")[-1][:7], a.get("lastUpdateTime")))
fila("fuentes / paquetes / cola", "A ore-serve /fuentes /paquetes (con sujeto)", "ya expuesto; se cuenta en la consola")
fila("vistas materializadas", "C nadie", "0 en la celda: `ore materializar` va a R2 desde la CLI, sin secreto R2 en t-%s" % CELDA)

# ── B · kubernetes ───────────────────────────────────────────────────────────
q = kubectl("get", "resourcequota", "-n", NS)
if q and q["items"]:
    st = q["items"][0]["status"]
    for k in sorted(st.get("hard", {})):
        fila("cuota · " + k, "B ResourceQuota.status", "%s / %s" % (st.get("used", {}).get(k, "0"), st["hard"][k]))
j = kubectl("get", "jobs", "-n", NS)
if j:
    ok = sum(x["status"].get("succeeded", 0) for x in j["items"])
    ko = sum(x["status"].get("failed", 0) for x in j["items"])
    ac = sum(x["status"].get("active", 0) for x in j["items"])
    ult = max((x["status"].get("startTime", "") for x in j["items"]), default="")
    fila("jobs · activos/ok/fallidos", "B Jobs.status", "%d / %d / %d · último %s" % (ac, ok, ko, ult))
p = kubectl("get", "pods", "-n", NS, "-l", "ore.dev/rol=control")
if p and p["items"]:
    pod = p["items"][0]
    cs = (pod["status"].get("containerStatuses") or [{}])[0]
    fila("ore-serve · desde / reinicios", "B Pod.status", "%s / %s" % (pod["status"].get("startTime"), cs.get("restartCount")))

# ── C · nadie ────────────────────────────────────────────────────────────────
fila("coste del periodo", "C nadie", "sin precio por vCPU-hora todavía")
fila("latencia / peticiones", "C nadie", "ore-serve no expone métricas")

# ── ¿alcanza ore-serve el API server? ────────────────────────────────────────
r = subprocess.run(
    ["kubectl", "exec", "-n", NS, "deploy/ore-serve", "-c", "ore-serve", "--", "sh", "-c",
     "timeout 5 wget -q -O - --header \"Authorization: Bearer $(cat /var/run/secrets/kubernetes.io/serviceaccount/token)\" "
     "https://kubernetes.default.svc/api/v1/namespaces/%s/resourcequotas >/dev/null 2>&1; echo $?" % NS],
    capture_output=True, text=True, encoding="utf-8", env={**os.environ, "MSYS_NO_PATHCONV": "1"},
)
cod = (r.stdout or "").strip().splitlines()[-1:] or ["?"]
alcanza = cod[0] == "0"
fila("ore-serve → API server", "B (¿lo alcanza?)", "sí" if alcanza else "NO (código %s: NetworkPolicy deny-all-egress; y sin TLS)" % cod[0])

print()
print("  ═══ GET /estado · celda %s · lo que hay y quién lo sabe ═══" % CELDA)
print()
for campo, fuente, valor in filas:
    print("  %-34s %-44s %s" % (campo, fuente, valor))
print()
print("  A = ore-serve lo alcanza hoy · B = sólo el API server · C = nadie lo mide")
print("  ⇒ B no puede leerlo ore-serve: hace falta quien lo escriba en un FICHERO que")
print("    ore-serve monte (un informador con Role de lectura en t-<celda> → ConfigMap),")
print("    o que lo publique el reconciliador en el plano de control cada 5 min.")
