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
          python pruebas-de-fuego/medida-el-estado-de-la-celda.py [celda] --snapshot

⭐ `--snapshot` (0026 E0): imprime el SNAPSHOT de la 0026-② rendido desde fuera
   con `kubectl` — byte a byte como lo rendirá el informador de la celda. Es la
   implementación de referencia: contra esto se coteja lo que `GET /celdas`
   devuelva en `estado_medido` (E2, sección D). Y `validar()` es el contrato:
   la misma forma que `ore-iam` exige al recibirlo.
"""
import json
import os
import subprocess
import sys

CELDA = next((a for a in sys.argv[1:] if not a.startswith("--")), "victor")
NS = "t-" + CELDA
SNAPSHOT = "--snapshot" in sys.argv


# ── el contrato (0026-②) ─────────────────────────────────────────────────────
def validar(s):
    """Devuelve la lista de faltas; vacía si el snapshot cumple la 0026-②."""
    faltas = []
    if s.get("v") != 1:
        faltas.append("v debe ser 1")
    if not isinstance(s.get("medido_en"), str) or not s["medido_en"].endswith("Z"):
        faltas.append("medido_en: ISO-8601 en UTC, con Z")
    cuota = s.get("cuota")
    if not isinstance(cuota, dict):
        faltas.append("cuota: objeto")
    else:
        for k in ("cpu", "memoria", "jobs"):
            v = cuota.get(k)
            if not (isinstance(v, list) and len(v) == 2):
                faltas.append("cuota.%s: [usado, duro]" % k)
    jobs = s.get("jobs")
    if not isinstance(jobs, dict) or any(not isinstance(jobs.get(k), int) for k in ("activos", "ok", "fallidos")):
        faltas.append("jobs: {activos, ok, fallidos} enteros")
    control = s.get("control")
    if not isinstance(control, dict) or not isinstance(control.get("listo"), bool):
        faltas.append("control: {listo bool, desde, reinicios}")
    if len(json.dumps(s, separators=(",", ":"))) > 8192:
        faltas.append("pasa de 8 KB")
    return faltas


def snapshot():
    """La 0026-②, desde fuera. Mismo orden de claves y mismos valores que el informador."""
    q = kubectl("get", "resourcequota", "-n", NS)
    st = (q["items"][0]["status"] if q and q["items"] else {})
    used, hard = st.get("used", {}), st.get("hard", {})

    def par(k, entero=False):
        u, h = used.get(k, "0"), hard.get(k, "0")
        return [int(u), int(h)] if entero else [u, h]

    j = kubectl("get", "jobs", "-n", NS) or {"items": []}
    activos = sum(x["status"].get("active", 0) for x in j["items"])
    ok = sum(x["status"].get("succeeded", 0) for x in j["items"])
    fallidos = sum(x["status"].get("failed", 0) for x in j["items"])
    ultimo = None
    for x in sorted(j["items"], key=lambda x: x["status"].get("startTime", ""), reverse=True)[:1]:
        s = x["status"]
        ultimo = {
            "nombre": x["metadata"]["name"],
            "estado": "activo" if s.get("active") else ("fallido" if s.get("failed") else "ok"),
            "inicio": s.get("startTime"),
            "fin": s.get("completionTime"),
        }
    p = kubectl("get", "pods", "-n", NS, "-l", "ore.dev/rol=control") or {"items": []}
    control = {"listo": False, "desde": None, "reinicios": 0}
    for pod in p["items"]:
        cs = (pod["status"].get("containerStatuses") or [{}])[0]
        control = {
            "listo": bool(cs.get("ready")),
            "desde": pod["status"].get("startTime"),
            "reinicios": int(cs.get("restartCount") or 0),
        }
    import datetime
    ahora = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    s = {
        "v": 1,
        "medido_en": ahora,
        "cuota": {"cpu": par("requests.cpu"), "memoria": par("requests.memory"), "jobs": par("count/jobs.batch", True)},
        "jobs": {"activos": activos, "ok": ok, "fallidos": fallidos, "ultimo": ultimo},
        "control": control,
    }
    return s


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


if SNAPSHOT:
    s = snapshot()
    faltas = validar(s)
    print(json.dumps(s, indent=2, ensure_ascii=False))
    if faltas:
        print("\n  ⛔ el snapshot no cumple la 0026-②: " + "; ".join(faltas), file=sys.stderr)
        sys.exit(1)
    sys.exit(0)

PROYECTO = "project-8853a180-450d-47be-b83"
IDP = "https://login.paladio.io/realms/rubix"
IAM = "https://iam.ore.paladio.io"


def sh(cmd):
    """`gcloud` en Windows es un .cmd: va como UNA cadena con shell=True."""
    r = subprocess.run(cmd, shell=True, capture_output=True, text=True, encoding="utf-8")
    return r.stdout.strip() if r.returncode == 0 else None


def token_del_agente():
    """El token REAL de `ore-agente-<celda>`: cliente y secreto del almacén, `client_credentials` al IdP."""
    cli = sh("gcloud secrets versions access latest --secret=%s-agente-cliente --project=%s" % (NS, PROYECTO))
    sec = sh("gcloud secrets versions access latest --secret=%s-agente-secreto --project=%s" % (NS, PROYECTO))
    if not cli or not sec:
        return None
    r = subprocess.run(
        ["curl", "-s", "-m", "10", "-X", "POST", IDP + "/protocol/openid-connect/token",
         "-d", "grant_type=client_credentials", "-d", "client_id=" + cli, "--data-urlencode", "client_secret=" + sec],
        capture_output=True, text=True, encoding="utf-8")
    try:
        return json.loads(r.stdout)["access_token"]
    except (ValueError, KeyError):
        return None


def guardado():
    """La fila de `iam.celda_estado` de esta celda, leída en la base (kubectl exec)."""
    sql = ("select json_build_object('medido_en', ce.medido_en, 'recibido_en', ce.recibido_en, "
           "'hace_s', extract(epoch from now() - ce.recibido_en)::int, 'cuerpo', ce.cuerpo) "
           "from iam.celda_estado ce join iam.celda c on c.id = ce.celda where c.nombre = '%s'" % CELDA)
    r = subprocess.run(
        ["kubectl", "exec", "-n", "identidad", "idp-db-0", "--", "psql", "-U", "keycloak", "-d", "iam", "-tAc", sql],
        capture_output=True, text=True, encoding="utf-8", env={**os.environ, "MSYS_NO_PATHCONV": "1"})
    out = (r.stdout or "").strip()
    return json.loads(out) if out else None


if "--empujar" in sys.argv:
    # E1 (aceptación en producción): el snapshot de referencia entra por la puerta pública con
    # el token del agente de la celda — exactamente lo que hará el informador.
    tok = token_del_agente()
    if not tok:
        print("  ✗ no se pudo acuñar el token de %s-agente" % NS, file=sys.stderr)
        sys.exit(1)
    s = snapshot()
    r = subprocess.run(
        ["curl", "-s", "-m", "10", "-o", "-", "-w", "\n%{http_code}", "-X", "POST",
         "%s/celdas/%s/estado" % (IAM, CELDA), "-H", "authorization: Bearer " + tok,
         "-H", "content-type: application/json", "-d", json.dumps(s, separators=(",", ":"))],
        capture_output=True, text=True, encoding="utf-8")
    cuerpo, _, cod = (r.stdout or "").rpartition("\n")
    print("  POST /celdas/%s/estado → %s %s" % (CELDA, cod, cuerpo))
    sys.exit(0 if cod == "200" else 1)

if "--cotejar" in sys.argv:
    # D (E2): lo que ore-iam guarda frente a lo que kubectl dice ahora. Los contadores pueden
    # moverse entre una lectura y otra; lo que NO puede es que difiera la forma o que el
    # snapshot tenga más de 90 s.
    g = guardado()
    if not g:
        print("  ✗ ore-iam no tiene estado de %s: nadie ha informado" % CELDA)
        sys.exit(1)
    ahora = snapshot()
    c = g["cuerpo"]
    print("  guardado   medido_en %s · recibido hace %s s" % (g["medido_en"], g["hace_s"]))
    difs = []
    for k in ("cuota", "control"):
        if c.get(k) != ahora.get(k):
            difs.append("%s: guardado %s · ahora %s" % (k, json.dumps(c.get(k)), json.dumps(ahora.get(k))))
    for k in ("activos", "ok", "fallidos"):
        if c.get("jobs", {}).get(k) != ahora["jobs"][k]:
            difs.append("jobs.%s: guardado %s · ahora %s" % (k, c.get("jobs", {}).get(k), ahora["jobs"][k]))
    faltas = validar(c)
    for d in difs:
        print("  ≠ " + d)
    if faltas:
        print("  ⛔ lo guardado no cumple la 0026-②: " + "; ".join(faltas))
    fresco = g["hace_s"] <= 90
    print("  %s snapshot %s · %s" % ("✓" if fresco and not faltas else "✗",
                                   "fresco (≤ 90 s)" if fresco else "VIEJO (> 90 s)",
                                   "sin diferencias" if not difs else "%d diferencia(s)" % len(difs)))
    sys.exit(0 if fresco and not faltas else 1)


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
