# -*- coding: utf-8 -*-
"""LA PRUEBA DE ACEPTACION DE LA 0025 (E6): una organizacion pide su SEGUNDA serverless
por el mismo verbo que la consola, y a los diez minutos la tiene entera; luego la retira
por el mismo camino, y la guarda vuelve a verde. Y la primera celda no cambia ni un byte.

  pedir      POST /organizaciones/{org}/celdas {nombre}   (con el token de una persona de la org)
  esperar    cada 30 s: fila aprovisionada · namespace · forja · cofre · ore-serve · cola ·
             enganche en plataforma/enganches · CNAME en la zona · /salud 200 · agente por el verbo
  cotejar    los objetos de t-<casa> con la misma resourceVersion que antes
  retirar    POST /celdas/{celda}/retirar, y esperar a que no quede nada
  guarda     medida-la-celda-tiene-nombre.py en verde al final

    PYTHONIOENCODING=utf-8 python pruebas-de-fuego/medida-la-segunda-serverless.py \\
        --org prueba --celda prueba-dos --token <fichero> [--solo-pedir | --solo-esperar | --solo-retirar] [--minutos 25]

Habla con ore-iam por un tunel (kubectl port-forward 3132 → identidad/ore-iam:8090) que abre
el mismo.
"""
import json
import os
import re
import socket
import subprocess
import sys
import time
import urllib.request

RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def arg(k, d=None):
    return sys.argv[sys.argv.index(k) + 1] if k in sys.argv else d


ORG = arg("--org", "prueba")
CELDA = arg("--celda")
TOKEN = open(arg("--token")).read().strip() if arg("--token") else None
MINUTOS = int(arg("--minutos", "25"))
SOLO_PEDIR = "--solo-pedir" in sys.argv
SOLO_RETIRAR = "--solo-retirar" in sys.argv
SOLO_ESPERAR = "--solo-esperar" in sys.argv  # la fila ya esta pedida: solo mirar como se levanta
if not CELDA or not TOKEN:
    print(__doc__)
    sys.exit(64)


def correr(*args, **kw):
    r = subprocess.run(list(args), capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=kw.get("timeout", 120))
    return r.stdout.strip()


def sql(q):
    return correr("kubectl", "-n", "identidad", "exec", "idp-db-0", "--", "psql", "-U", "keycloak", "-d", "iam", "-qtAc", q)


def existe(ns, tipo, nombre):
    return correr("kubectl", "-n", ns, "get", tipo, nombre, "-o", "name") != ""


def resuelve(h):
    try:
        return sorted({a[4][0] for a in socket.getaddrinfo(h, 443, socket.AF_INET)})
    except socket.gaierror:
        return []


def salud(h):
    try:
        return urllib.request.urlopen("https://%s/salud" % h, timeout=10).status
    except Exception as e:
        return getattr(e, "code", 0)


def verbo(metodo, camino, cuerpo=None):
    datos = json.dumps(cuerpo).encode() if cuerpo is not None else None
    req = urllib.request.Request("http://127.0.0.1:3132" + camino, data=datos, method=metodo,
                                 headers={"authorization": "Bearer " + TOKEN, "content-type": "application/json"})
    try:
        r = urllib.request.urlopen(req, timeout=30)
        return r.status, r.read().decode()
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()


def ahora():
    return time.strftime("%H:%M:%S")


def foto(ns):
    """Los objetos de un namespace con su resourceVersion: si nada cambio, es la misma foto."""
    return correr("kubectl", "-n", ns, "get", "deploy,sts,svc,networkpolicy,sa,pvc",  # sin ResourceQuota: su resourceVersion cambia con el USO
                  "-o", "jsonpath={range .items[*]}{.kind}/{.metadata.name}:{.metadata.generation} {end}")  # generation: cambia con la SPEC, no con el estado


# ── el tunel ────────────────────────────────────────────────────────────────
subprocess.run(["taskkill", "//F", "//IM", "kubectl.exe"], capture_output=True)
tunel = subprocess.Popen(["kubectl", "port-forward", "-n", "identidad", "svc/ore-iam", "3132:8090"],
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
for _ in range(30):
    try:
        urllib.request.urlopen("http://127.0.0.1:3132/salud", timeout=2)
        break
    except Exception:
        time.sleep(1)
NS = "t-" + CELDA
CASA = "t-" + ORG
linea = []


def di(s):
    print("%s  %s" % (ahora(), s))
    linea.append((ahora(), s))


try:
    foto_casa = foto(CASA)
    di("foto de %s: %d objetos" % (CASA, len(foto_casa.split())))

    if not SOLO_RETIRAR:
        # ── pedir ───────────────────────────────────────────────────────────
        t0 = time.time()
        if not SOLO_ESPERAR:
            cod, cuerpo = verbo("POST", "/organizaciones/%s/celdas" % ORG, {"nombre": CELDA, "tier": "compartido"})
            di("POST /organizaciones/%s/celdas {%s} → %s %s" % (ORG, CELDA, cod, cuerpo[:200]))
            if cod != 200:
                sys.exit(1)
            huella = sql("select quien from iam.huella where operacion='celda:crear' order by cuando desc limit 1")
            di("huella celda:crear por %s" % huella)
        # ── esperar ─────────────────────────────────────────────────────────
        hitos = {}
        fin = t0 + MINUTOS * 60
        while time.time() < fin:
            estado = {
                "fila aprovisionada": sql("select coalesce(aprovisionada::text,'') from iam.celda where nombre='%s'" % CELDA) != "",
                "namespace": existe("", "namespace", NS),
                "forja": correr("kubectl", "-n", NS, "get", "statefulset", "forja", "-o", "jsonpath={.status.readyReplicas}") == "1",
                "cofre": correr("kubectl", "-n", NS, "get", "deploy", "ore-cofre", "-o", "jsonpath={.status.availableReplicas}") == "1",
                "ore-serve": correr("kubectl", "-n", NS, "get", "deploy", "ore-serve", "-o", "jsonpath={.status.availableReplicas}") == "1",
                "cola": correr("kubectl", "get", "clusterqueue", "cq-" + CELDA, "-o", "name") != "",
                "enganche (Kustomization inquilino-%s)" % CELDA: existe("flux-system", "kustomization", "inquilino-" + CELDA),
                "CNAME en la zona": bool(correr("gcloud", "dns", "record-sets", "describe", "%s.ore.paladio.io." % CELDA, "--zone=ore-paladio-io", "--type=CNAME", "--format=value(name)") if os.name != "nt" else subprocess.run("gcloud dns record-sets describe %s.ore.paladio.io. --zone=ore-paladio-io --type=CNAME --format=value(name)" % CELDA, shell=True, capture_output=True, text=True).stdout.strip()),
                "agente por el verbo": sql("select count(*) from iam.huella h join iam.agente a on a.id=h.sobre where h.operacion='agente:registrar' and a.nombre='ore-agente-%s' and h.quien <> 'operador'" % CELDA) not in ("", "0"),
                "/salud 200": salud("%s.ore.paladio.io" % CELDA) == 200,
            }
            for k, v in estado.items():
                if v and k not in hitos:
                    hitos[k] = int(time.time() - t0)
                    di("✓ %-40s a los %d s" % (k, hitos[k]))
            if all(estado.values()):
                break
            time.sleep(30)
        faltan = [k for k, v in estado.items() if not v]
        di("entera en %d s" % (time.time() - t0) if not faltan else "✗ tras %d min faltan: %s" % (MINUTOS, ", ".join(faltan)))
        # ── la de casa, intacta ─────────────────────────────────────────────
        foto2 = foto(CASA)
        di("%s: %s" % (CASA, "los mismos objetos con las mismas resourceVersion ✓" if foto2 == foto_casa else "✗ CAMBIO: " + str(set(foto2.split()) ^ set(foto_casa.split()))))
        if faltan:
            sys.exit(1)

    if not SOLO_PEDIR:
        # ── retirar ─────────────────────────────────────────────────────────
        t1 = time.time()
        cod, cuerpo = verbo("POST", "/celdas/%s/retirar" % CELDA)
        di("POST /celdas/%s/retirar → %s %s" % (CELDA, cod, cuerpo[:200]))
        if cod != 200:
            sys.exit(1)
        fin = t1 + MINUTOS * 60
        hitos = {}
        while time.time() < fin:
            estado = {
                "enganche fuera": not existe("flux-system", "kustomization", "inquilino-" + CELDA),
                "namespace fuera": not existe("", "namespace", NS),
                "CNAME fuera": subprocess.run("gcloud dns record-sets describe %s.ore.paladio.io. --zone=ore-paladio-io --type=CNAME --format=value(name)" % CELDA, shell=True, capture_output=True, text=True).stdout.strip() == "",
                "cuentas fuera": subprocess.run("gcloud iam service-accounts list --filter=email~ore-cofre-%s@ --format=value(email)" % CELDA, shell=True, capture_output=True, text=True).stdout.strip() == "",
                "secretos fuera": subprocess.run("gcloud secrets list --filter=name~secrets/%s- --format=value(name)" % NS, shell=True, capture_output=True, text=True).stdout.strip() == "",
            }
            for k, v in estado.items():
                if v and k not in hitos:
                    hitos[k] = int(time.time() - t1)
                    di("✓ %-40s a los %d s" % (k, hitos[k]))
            if all(estado.values()):
                break
            time.sleep(30)
        faltan = [k for k, v in estado.items() if not v]
        di("desmontada en %d s" % (time.time() - t1) if not faltan else "✗ tras %d min quedan: %s" % (MINUTOS, ", ".join(faltan)))
        foto3 = foto(CASA)
        di("%s: %s" % (CASA, "intacta ✓" if foto3 == foto_casa else "✗ CAMBIO"))
        g = subprocess.run([sys.executable, os.path.join(RAIZ, "pruebas-de-fuego", "medida-la-celda-tiene-nombre.py")],
                           capture_output=True, text=True, encoding="utf-8", errors="replace", env=dict(os.environ, PYTHONIOENCODING="utf-8"))
        di("la guarda: %s" % ("verde ✓" if g.returncode == 0 else "ROJA ✗\n" + g.stdout[-800:]))
        if faltan or g.returncode != 0:
            sys.exit(1)
finally:
    tunel.terminate()
