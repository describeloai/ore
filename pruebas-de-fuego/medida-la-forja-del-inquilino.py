# -*- coding: utf-8 -*-
"""Medida: la forja, antes de partirla — que es, quien la alcanza, que hay dentro
de cada inquilino, y que hace falta para que `t-demo` tenga la suya. Es la
E3-(c) de la 0024: «`t-demo` con su forja propia y Flux tirando de la nuestra».

La 0024 ya decidio el QUE (⑥ y «lo que se acepta a cambio»): si el arbol es
soberano su forja tambien, una por inquilino en su cluster; y el compartimento
—lo que el cluster OBEDECE— se queda en la nuestra (0022-①), que es de la que
Flux tira. Esto mide el COMO: cuanto pesa, quien tiene que llegar, y que se hacia
a mano y ahora tiene que hacerse solo.

    PYTHONIOENCODING=utf-8 python pruebas-de-fuego/medida-la-forja-del-inquilino.py [demo]

Secciones:
  A  la forja hoy: que corre, que pide, que ocupa
  B  que hay dentro, por inquilino, y de quien es cada repositorio
  C  quien llega a ella y por donde (politicas, GitRepositories, argumentos)
  D  lo que una forja EN la celda necesita, medido contra lo que hay
"""
import json
import os
import re
import subprocess
import sys

INQ = sys.argv[1] if len(sys.argv) > 1 else "demo"
NS = "t-" + INQ
RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def correr(*args):
    r = subprocess.run(list(args), capture_output=True, text=True, encoding="utf-8", errors="replace",
                       timeout=90, shell=(args[0] == "gcloud"), env=dict(os.environ, MSYS_NO_PATHCONV="1"))
    return r.stdout


def kj(*args):
    s = correr("kubectl", *args, "-o", "json")
    return json.loads(s) if s.strip() else {}


def leer(rel):
    with open(os.path.join(RAIZ, rel), encoding="utf-8") as f:
        return f.read()


def titulo(t):
    print()
    print(t)
    print("-" * len(t))


falta = []


def hace_falta(que, det=""):
    falta.append((que, det))


# ── A ───────────────────────────────────────────────────────────────────────
titulo("A - LA FORJA HOY")
st = kj("-n", "forja", "get", "statefulset", "forja")
c = st["spec"]["template"]["spec"]["containers"][0]
pvc = kj("-n", "forja", "get", "pvc", "forja-datos")
print("   StatefulSet forja/forja · %s" % c["image"].rsplit("/", 1)[1])
print("   pide  %s · tope %s" % (c["resources"]["requests"], c["resources"]["limits"]))
print("   disco PVC %s (%s) · %s" % (pvc["spec"]["resources"]["requests"]["storage"], pvc["spec"].get("storageClassName"),
                                     correr("kubectl", "-n", "forja", "exec", "forja-0", "--", "sh", "-c", "df -h /data | tail -1 | awk '{print $3\" usados de \"$2}'").strip()))
env = {e["name"]: e.get("value") for e in c.get("env", [])}
print("   base  %s · registro %s · webhooks a %s" % (env.get("FORGEJO__database__DB_TYPE"),
      "cerrado" if env.get("FORGEJO__service__DISABLE_REGISTRATION") == "true" else "ABIERTO", env.get("FORGEJO__webhook__ALLOWED_HOST_LIST")))
print("   pool  %s" % st["spec"]["template"]["spec"].get("nodeSelector"))

# ── B ───────────────────────────────────────────────────────────────────────
titulo("B - QUE HAY DENTRO, Y DE QUIEN ES")
tk = correr("gcloud", "secrets", "versions", "access", "latest", "--secret=forja-admin-token").strip()
tunel = subprocess.Popen(["kubectl", "port-forward", "-n", "forja", "svc/forja", "3129:3000"],
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
import time
time.sleep(3)


def api(ruta):
    r = subprocess.run(["curl", "-s", "-H", "Authorization: token " + tk, "http://localhost:3129/api/v1" + ruta],
                       capture_output=True, text=True, encoding="utf-8")
    try:
        return json.loads(r.stdout)
    except ValueError:
        return []


DUENNO = {"ontologia": "del INQUILINO (su arbol)", "trabajo": "del INQUILINO (su cola de Jobs)",
          "compartimento": "NUESTRO (lo que su cluster obedece, 0022-1)"}
repos = {}
for o in ("t-demo", "t-prueba"):
    for r in api("/orgs/%s/repos" % o):
        repos[(o, r["name"])] = r["size"]
        print("   %-9s %-14s %5d KiB   %s" % (o, r["name"], r["size"], DUENNO.get(r["name"], "?")))
users = [u["login"] for u in api("/admin/users")] if isinstance(api("/admin/users"), list) else []
print("   usuarios: %s" % ", ".join(users))
tunel.terminate()
mios = sum(v for (o, n), v in repos.items() if o == NS and n != "compartimento")
print("   ⇒ lo que se va con `%s`: ontologia + trabajo = %d KiB. Lo que se queda: compartimento." % (INQ, mios))
print("   ⇒ el admin `ore-admin` y su testigo `forja-admin-token` se crearon A MANO (31-copias lo cuenta):")
print("     una forja por inquilino exige que eso lo haga un contenedor de inicio, sin nadie delante.")
hace_falta("bootstrap automatico del admin de la forja del inquilino",
           "hoy `forgejo admin user create` + `generate-access-token` a mano; en la celda lo hace un init y deja el testigo en el almacen como `t-<n>-forja-admin`")

# ── C ───────────────────────────────────────────────────────────────────────
titulo("C - QUIEN LLEGA A LA FORJA, Y POR DONDE")
pol = kj("-n", "forja", "get", "networkpolicy", "entrada-a-la-forja")
print("   entrada-a-la-forja admite:")
for f in pol["spec"]["ingress"][0]["from"]:
    ns = (f.get("namespaceSelector") or {}).get("matchLabels", {})
    pod = (f.get("podSelector") or {}).get("matchLabels", {})
    print("     · namespaces %s · pods %s" % (ns, pod or "todos"))
print()
for g in kj("-n", "flux-system", "get", "gitrepository").get("items", []):
    url = g["spec"]["url"]
    print("   GitRepository %-18s → %s" % (g["metadata"]["name"], url.replace("http://forja.forja.svc.cluster.local:3000/", "forja/")))
print()
usos = []
for f in sorted(os.listdir(os.path.join(RAIZ, "malla"))):
    if not f.endswith(".yaml") and not f.endswith(".sh"):
        continue
    for i, l in enumerate(leer("malla/" + f).splitlines(), 1):
        if "forja.forja.svc" in l and not l.strip().startswith("#"):
            usos.append((f, i, l.strip()[:70]))
for f, i, l in usos:
    print("   %-36s:%-4d %s" % (f, i, l))
del_inq = sorted({f for f, _, _ in usos if f[:2] in ("40", "42", "44", "13")})
print()
print("   ⇒ del inquilino apuntan a la forja central: %s" % ", ".join(del_inq))
hace_falta("los argumentos y GitRepositories del inquilino apuntan a `forja.forja.svc`",
           "40 (`--forja`, `--cola`), 42 (semilla), 44 (catalogo) pasan a `forja.%s.svc`; el GitRepository `trabajo-%s` tambien; `inquilino-%s` (compartimento) NO" % (NS, INQ, INQ))
hace_falta("Flux tiene que poder llegar a la forja del inquilino para tirar de `trabajo`",
           "hoy `entrada-a-la-forja` vive en `forja`; en `%s` hace falta una politica que admita `flux-system` en :3000 — y la forja del inquilino tiene que poder avisar a `webhook-receiver.flux-system`" % NS)
sm = correr("gcloud", "secrets", "list", "--filter=name~forja", "--format=value(name)").split()
print("   testigos en el almacen: %s" % ", ".join(sm))
hace_falta("el testigo `t-%s-forja-token` es de la forja central; en la del inquilino hay que acuñar otro" % INQ,
           "lo hace el aprovisionador ④ contra la forja del inquilino; `ore-serve` y el driver leen el mismo nombre del almacen, asi que solo cambia el valor")

# ── D ───────────────────────────────────────────────────────────────────────
titulo("D - LO QUE UNA FORJA EN `%s` NECESITA, CONTRA LO QUE HAY" % NS)
q = kj("-n", NS, "get", "resourcequota").get("items", [{}])[0]
usado, tope = q.get("status", {}).get("used", {}), q.get("status", {}).get("hard", {})
print("   cuota de %s: cpu %s de %s · memoria %s de %s · jobs %s de %s" % (
    NS, usado.get("requests.cpu"), tope.get("requests.cpu"), usado.get("requests.memory"), tope.get("requests.memory"),
    usado.get("count/jobs.batch"), tope.get("count/jobs.batch")))
print("   la forja pide %s: cabe de sobra" % c["resources"]["requests"])
print("   disco: %s por inquilino (pd-balanced ≈ $0.10/GiB·mes ⇒ ≈ $1/mes), y hoy la central usa 13 MB de 9.7 GB" % pvc["spec"]["resources"]["requests"]["storage"])
print("   copias: `31-copias-de-la-forja` copia LA forja; con una por inquilino, copia N o cada inquilino la suya")
hace_falta("copias por inquilino", "la CronJob de copias apunta a `forja.forja.svc`; o recorre las celdas o se rinde una por inquilino (plantilla)")
hace_falta("mover `ontologia` y `trabajo` de la central a la del inquilino, UNA vez",
           "un `git clone --mirror` + `push --mirror` desde un Job en `%s`; despues los de la central se quedan como copia de solo lectura hasta borrarlos" % NS)

titulo("LO QUE HAY QUE HACER")
for i, (que, det) in enumerate(falta, 1):
    print("   %d. %s" % (i, que))
    if det:
        print("      %s" % det)
print()
print("  => %d piezas. Ninguna es una decision: la 0024 ya las tomo. Es manifiesto, un init, y una mudanza." % len(falta))
