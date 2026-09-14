# -*- coding: utf-8 -*-
"""Que ata a un inquilino al cluster compartido. Es la medida PREVIA a la E3 de la 0024.

`t-demo` parece autocontenido —un namespace, catorce politicas de red— y no lo
es: depende de cosas que estan fuera y que nadie habia listado. La E3 («nuestro
cluster como primer cluster de cliente») no existe para mover a `demo`: existe
para DESCUBRIR este listado aqui, donde equivocarse cuesta un `git revert`,
antes de que haya un cluster de cliente de verdad. Un dedicado o un BYOC que
funcione es exactamente este listado, resuelto.

Lo que se mide, en vivo:

  A. a QUE sale el namespace        sus NetworkPolicy de egreso
  B. con que arranca `ore-serve`    todo argumento que apunta fuera
  C. que hay FUERA y lo nombra      flux-system, identidad, ambito de cluster
  D. quien puso sus secretos        el `manager` de cada Secret
  E. donde guarda el cofre          la URL de su base
  F. por donde llega la consola     HTTPRoute y Gateway

Y cada acoplamiento se CLASIFICA, que es lo que la E3 necesita saber:

  viaja      va en el manifiesto del inquilino tal cual; todo cluster lo tiene
  se parte   hoy compartido, en E3 propio del inquilino (la forja)
  central    se queda con nosotros y hace falta ALCANZARLO desde fuera (puerta)
  decidir    no tiene respuesta escrita todavia; la E3 la toma

Los dos «decidir» que salieron la primera vez (E y F) los decidio la 0024 en sus
puntos 5 y 6 el 2026-09-13, y desde entonces se clasifican como lo que van a ser
— «se parte» y «viaja» — con la ADR al lado. La medida sigue viendo el ESTADO,
que no ha cambiado: lo que cambio es que ya hay respuesta escrita.
"""
import json
import subprocess
import sys

for f in (sys.stdout, sys.stderr):
    try:
        f.reconfigure(errors="replace")
    except AttributeError:
        pass

NS = sys.argv[1] if len(sys.argv) > 1 else "t-demo"
INQ = NS.removeprefix("t-")

# ⛔ Un namespace que no existe medía «5 acoplamientos, 0 por decidir» con todas
#   las secciones vacías y parecía un resultado. Se pasó `demo` por `t-demo` y
#   la medida contestó en vez de negarse.
if subprocess.run(["kubectl", "get", "ns", NS], capture_output=True).returncode:
    print("el namespace `%s` no existe (¿`t-%s`?)" % (NS, NS))
    sys.exit(65)


def k(*args):
    r = subprocess.run(["kubectl"] + list(args), capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=60)
    return r.stdout


def kj(*args):
    try:
        return json.loads(k(*args) or "{}")
    except json.JSONDecodeError:
        return {}


def titulo(t):
    print()
    print(t)
    print("=" * 74)


hallazgos = []  # (clase, que, detalle)


def anota(clase, que, detalle=""):
    hallazgos.append((clase, que, detalle))


# ── A ───────────────────────────────────────────────────────────────────────
titulo("A - A QUE SALE `%s`, segun sus NetworkPolicy" % NS)
destinos = {}
for np in kj("-n", NS, "get", "networkpolicy", "-o", "json").get("items", []):
    for e in np["spec"].get("egress", []):
        puertos = ",".join(str(p.get("port")) for p in e.get("ports", []))
        for to in e.get("to", []):
            ns = to.get("namespaceSelector", {}).get("matchLabels", {})
            ip = to.get("ipBlock", {}).get("cidr")
            clave = json.dumps(ns, sort_keys=True) if ns else ip
            if clave:
                destinos.setdefault(clave, set()).add(puertos)
for d, ps in sorted(destinos.items()):
    print("   %-52s :%s" % (d, " ".join(sorted(ps))))

anota("viaja", "DNS de kube-system :53", "todo cluster lo tiene")
if any("forja" in d for d in destinos):
    anota("se parte", "la forja :3000 (ontologia, trabajo, compartimento)",
          "una Gitea + 10 GB en el cluster del inquilino; central romperia la soberania por debajo")
if any("identidad" in d for d in destinos):
    anota("central", "el emisor (Keycloak) :8080",
          "ya tiene IP publica; la politica pasa de selector de namespace a host externo")
if "169.254.169.254/32" in destinos:
    anota("cambia por nube", "metadatos de GCE + KMS de Google (Workload Identity)",
          "BYOC en otra nube exige abstraer la llave de fuera (0023); dicho en la 0024")
if any("0.0.0.0/0" in d for d in destinos):
    anota("viaja", "salida a internet :443/:5432 (los origenes del cliente)",
          "y mejora: desde su cluster, las listas blancas son suyas")

# ── B ───────────────────────────────────────────────────────────────────────
titulo("B - CON QUE ARRANCA `ore-serve`")
args = kj("-n", NS, "get", "deploy", "ore-serve", "-o", "json").get("spec", {}).get("template", {}).get("spec", {}).get("containers", [{}])[0].get("args", [])
pares = {args[i]: args[i + 1] for i in range(len(args) - 1) if args[i].startswith("--")}
for kk in ("--forja", "--cola", "--cofre", "--emisor", "--jwks"):
    if kk in pares:
        print("   %-14s %s" % (kk, pares[kk]))
if "forja.forja.svc" in pares.get("--forja", ""):
    anota("se parte", "`--forja` y `--cola` apuntan a la forja compartida", "pasan a la forja del inquilino")

# ── C ───────────────────────────────────────────────────────────────────────
titulo("C - LO QUE HAY FUERA DE `%s` Y LO NOMBRA" % NS)
fuera = []
for ns in ("flux-system", "identidad", "ore-system", "forja"):
    for linea in k("-n", ns, "get", "gitrepository,kustomization,receiver,serviceaccount,job,cronjob,secret", "-o", "name").splitlines():
        if INQ in linea:
            fuera.append("%s/%s" % (ns, linea))
for linea in k("get", "namespace,clusterqueue", "-o", "name").splitlines():
    if INQ in linea:
        fuera.append("(cluster)/" + linea)
for x in fuera:
    print("   " + x)
if any("gitrepository" in x for x in fuera):
    anota("se invierte", "Flux: GitRepository + Kustomization en flux-system, tirando del compartimento",
          "en E3 Flux corre en el cluster del inquilino y tira de NUESTRA forja: nunca credenciales hacia dentro")
if any("clusterqueue" in x for x in fuera):
    anota("viaja", "Kueue: ClusterQueue por inquilino", "Kueue pasa a ser parte del manifiesto del inquilino, no de la plataforma")
if any("fundar" in x for x in fuera):
    anota("central", "el Job `fundar` en identidad", "acto de operador, una vez; no viaja")

# ── D ───────────────────────────────────────────────────────────────────────
titulo("D - QUIEN PUSO LOS SECRETOS DE `%s`" % NS)
for s in kj("-n", NS, "get", "secret", "-o", "json", "--show-managed-fields").get("items", []):
    if s.get("type") == "kubernetes.io/service-account-token":
        continue
    mgr = (s["metadata"].get("managedFields") or [{}])[0].get("manager", "?")
    print("   %-16s %s" % (s["metadata"]["name"], mgr))
    if "kubectl" in mgr:
        anota("decidir", "Secret `%s` puesto A MANO (%s)" % (s["metadata"]["name"], mgr),
              "en un cluster nuevo alguien tiene que volver a ponerlo; la 0023 dice que el material lo trae un init desde el almacen, y esto no es eso")

# ── E ───────────────────────────────────────────────────────────────────────
titulo("E - DONDE GUARDA EL COFRE")
# ⚠️ Del ALMACEN, no de un `Secret`: el `Secret` `cofre-url` se borro el
#   2026-09-13 al pasar la base al camino de la 0023, y esta seccion leia de
#   ahi — el acoplamiento desaparecio del listado sin haberse resuelto. Una
#   medida que deja de ver algo porque se movio de sitio no mide.
u = subprocess.run(["gcloud", "secrets", "versions", "access", "latest", "--secret=cofre-url",
                    "--project=project-8853a180-450d-47be-b83"],
                   capture_output=True, text=True, encoding="utf-8", errors="replace", shell=True, timeout=90).stdout.strip()
if u:
    host = u.split("@")[-1].split("/")[0]
    print("   postgres://***@%s" % host)
    # ¿Sigue el MATERIAL ahi, o solo el metadato? Tras la 028 la tabla no existe.
    hay_material = subprocess.run(["kubectl", "-n", "identidad", "exec", "idp-db-0", "--", "psql", "-U", "keycloak", "-d", "iam",
                                   "-qtAc", "select to_regclass('cofre.material') is not null"],
                                  capture_output=True, text=True).stdout.strip() == "t"
    if "identidad" in host and hay_material:
        anota("se parte", "el cofre guarda su material en la base CENTRAL de `iam` (%s)" % host,
              "0024-5: el material pasa al Secret Manager de la celda; `cofre.secreto` e `iam.concesion` se quedan (metadato). Medido en `medida-el-cofre-y-su-almacen.py`")
    elif "identidad" in host:
        anota("central", "el cofre alcanza la base central de `iam` (%s) — por el METADATO, no por el material" % host,
              "0024-5 hecha el 2026-09-14: el material esta en el Secret Manager de la celda (`t-<n>-cofre-*`). Lo que queda es `cofre.secreto` + `iam.concesion`, que es del plano de control; en BYOC ese camino es HTTP contra ore-iam")

# ── F ───────────────────────────────────────────────────────────────────────
titulo("F - POR DONDE LLEGA LA CONSOLA")
for r in kj("-n", NS, "get", "httproute", "-o", "json").get("items", []):
    print("   HTTPRoute %-10s hosts=%s  parent=%s" % (r["metadata"]["name"], r["spec"].get("hostnames"), r["spec"]["parentRefs"][0]["name"]))
    anota("viaja", "la puerta: `%s` por el Gateway `%s`, que es el de la CELDA ore-mesh" % (",".join(r["spec"].get("hostnames", [])), r["spec"]["parentRefs"][0]["name"]),
          "0024-6: cada celda tiene su puerta (Gateway + cert + `<n>.ore.paladio.io`); el plano de control escribe el DNS y no hace de proxy. Falta que `iam.celda` diga la suya")

# ── el listado ──────────────────────────────────────────────────────────────
titulo("LO QUE ATA A `%s`, CLASIFICADO" % INQ)
orden = ["viaja", "se parte", "se invierte", "central", "cambia por nube", "decidir"]
for clase in orden:
    los = [h for h in hallazgos if h[0] == clase]
    if not los:
        continue
    print()
    print("  %s" % clase.upper())
    for _, que, det in los:
        print("   · %s" % que)
        if det:
            print("       %s" % det)

n = {c: sum(1 for h in hallazgos if h[0] == c) for c in orden}
print()
print("  %d acoplamientos · %d viajan · %d se parten/invierten · %d centrales · %d por decidir" % (
    len(hallazgos), n["viaja"], n["se parte"] + n["se invierte"], n["central"] + n["cambia por nube"], n["decidir"]))
print()
if n["decidir"]:
    print("  => La E3 es resolver los %d «decidir». Lo demas es manifiesto." % n["decidir"])
else:
    print("  => Nada por decidir: la 0024 lo tiene escrito. La E3 es manifiesto y tres cambios de codigo.")
