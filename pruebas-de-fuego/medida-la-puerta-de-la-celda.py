# -*- coding: utf-8 -*-
"""Medida: la puerta de la celda — que sabe el plano de control, que dice el
mundo, y que pasaria con una segunda celda. Es la E3-(a) de la 0024 (punto 6).

La 022 puso la ENTRADA de una organizacion en su fila (`demo.ore.paladio.io`) y
dijo a proposito que la IP, el certificado y el Gateway no van ahi: son
carretera. La 0024-6 dice que cada celda tiene su puerta y que el plano de
control escribe el DNS. Entre las dos hay un hueco: la celda no dice su puerta,
y el registro DNS lo cubre hoy un comodin que apunta a UNA IP — la de la unica
celda. Esto mide el hueco antes de cerrarlo.

    PYTHONIOENCODING=utf-8 python pruebas-de-fuego/medida-la-puerta-de-la-celda.py

Secciones:
  A  que sabe `iam`: entrada por organizacion, celda, y si la celda dice su puerta
  B  que dice el mundo: a que IP resuelve cada entrada, que Gateways hay, quien tiene la zona
  C  que hace la consola con eso
  D  la segunda celda, razonada sobre lo medido
"""
import json
import os
import re
import socket
import subprocess
import sys

RAIZ = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CONSOLA = os.environ.get("CONSOLA", r"C:\rubix-platform")


def correr(*args):
    r = subprocess.run(list(args), capture_output=True, text=True, encoding="utf-8", errors="replace",
                       timeout=90, shell=(args[0] in ("gcloud", "nslookup")))
    return r.stdout


def kj(*args):
    s = correr("kubectl", *args, "-o", "json")
    return json.loads(s) if s.strip() else {}


def sql(q):
    return correr("kubectl", "-n", "identidad", "exec", "idp-db-0", "--", "psql", "-U", "keycloak", "-d", "iam", "-qtAc", q)


def resolver(host):
    try:
        return sorted({a[4][0] for a in socket.getaddrinfo(host, 443, socket.AF_INET)})
    except socket.gaierror:
        return []


def titulo(t):
    print()
    print(t)
    print("-" * len(t))


hallazgos = []


def anota(clase, que, det=""):
    hallazgos.append((clase, que, det))


# ── A ───────────────────────────────────────────────────────────────────────
titulo("A - QUE SABE `iam`")
cols = sql("select column_name from information_schema.columns where table_schema='iam' and table_name='celda' order by ordinal_position").split()
print("   iam.celda: %s" % ", ".join(cols))
tiene_puerta = "puerta" in cols
orgs = []
sel = "select o.nombre, o.entrada, c.nombre, c.tier%s from iam.organizacion o left join iam.celda c on c.organizacion = o.id order by 1" % (
    ", c.puerta" if tiene_puerta else "")
for l in sql(sel).splitlines():
    if l.strip():
        f = l.split("|")
        orgs.append(f)
        print("   %-8s entrada=%-24s celda=%s/%s%s" % (f[0], f[1], f[2], f[3], ("  puerta=" + f[4]) if tiene_puerta else ""))
if not tiene_puerta:
    anota("hueco", "la celda NO dice su puerta: `iam.celda` no tiene columna para ello",
          "la 022 guardo la entrada de la organizacion (identidad); la 0024-6 dice que la puerta es de la CELDA, y no esta escrita")
else:
    for f in orgs:
        if not f[4]:
            anota("hueco", "la celda de `%s` tiene la columna y esta vacia" % f[0])

# ── B ───────────────────────────────────────────────────────────────────────
titulo("B - QUE DICE EL MUNDO")
gws = {}
for g in kj("get", "gateway", "-A").get("items", []):
    ip = ",".join(a["value"] for a in g.get("status", {}).get("addresses", []))
    hosts = [l.get("hostname", "*") for l in g["spec"].get("listeners", [])]
    gws[ip] = "%s/%s" % (g["metadata"]["namespace"], g["metadata"]["name"])
    print("   Gateway %-22s ip=%-16s hostnames=%s" % (gws[ip], ip, hosts))
print()
for f in orgs:
    ips = resolver(f[1])
    quien = " · ".join(gws.get(ip, "¿de quien?") for ip in ips) or "NO RESUELVE"
    print("   %-24s → %-16s %s" % (f[1], ",".join(ips) or "—", quien))
    if not ips:
        anota("hueco", "`%s` no resuelve" % f[1])
comodin = resolver("medida-%d.ore.paladio.io" % os.getpid())
print("   %-24s → %-16s %s" % ("<cualquiera>.ore.paladio.io", ",".join(comodin) or "—",
                                 "COMODIN: todo nombre va a esa IP" if comodin else "sin comodin"))
if comodin:
    anota("carretera", "un comodin `*.ore.paladio.io` manda TODO nombre a %s (%s)" % (",".join(comodin), " · ".join(gws.get(i, "?") for i in comodin)),
          "vale mientras haya UNA celda; con dos, la entrada de una organizacion en la segunda llegaria a la primera")
ns = correr("nslookup", "-type=NS", "paladio.io")
servidores = sorted(set(re.findall(r"nameserver\s*=\s*(\S+)", ns)))
print()
print("   zona `paladio.io` en: %s" % ", ".join(servidores))
zonas = correr("gcloud", "dns", "managed-zones", "list", "--format=value(dnsName)").split()
print("   zonas en Cloud DNS del proyecto: %s" % (", ".join(zonas) or "ninguna"))
escribible = any(z.rstrip(".").endswith("ore.paladio.io") or z.rstrip(".") == "paladio.io" for z in zonas)
if not escribible:
    anota("hueco", "el plano de control NO puede escribir el DNS: `ore.paladio.io` vive en el registrador (%s), no en Cloud DNS" % ", ".join(servidores),
          "la 0024-6 dice «el registro DNS lo escribe el plano de control»; hoy lo escribe una persona en el panel del registrador. Delegar `ore.paladio.io` a Cloud DNS (un NS en el registrador, una vez) lo hace escribible con `gcloud dns`")

# ── C ───────────────────────────────────────────────────────────────────────
titulo("C - QUE HACE LA CONSOLA")
org_ts = open(os.path.join(CONSOLA, "lib", "server", "organizacion.ts"), encoding="utf-8").read()
cel_ts = open(os.path.join(CONSOLA, "lib", "server", "celdas.ts"), encoding="utf-8").read()
desde_entrada = "org.entrada" in org_ts and "entradaActual" in org_ts
print("   la direccion del arbol sale de `org.entrada` (022), por `entradaActual`: %s" % ("si" if desde_entrada else "NO"))
print("   la pantalla de clusters lee la celda de `/celdas` y pregunta `/salud` por la entrada: %s" % (
    "si" if "salud-del-arbol" in cel_ts else "NO"))
coteja = "apuntaALaCelda" in cel_ts and "resolve4" in cel_ts
print("   la consola coteja que la entrada APUNTE a la celda: %s" % ("si" if coteja else "NO"))
if not coteja:
    anota("hueco", "la consola pinta la celda pero no coteja que la entrada de la organizacion lleve a ella",
          "es la figura de la 0024-4: `control` dice que se declaro, y por el camino se mira si el mundo converge. Aqui falta la segunda mitad")

# ── D ───────────────────────────────────────────────────────────────────────
titulo("D - LA SEGUNDA CELDA, RAZONADA SOBRE LO MEDIDO")
print("""
   Una organizacion `acme` fundada en una celda `gke-acme` con IP Y:
     1. `acme.ore.paladio.io` la cubre el comodin → llega a %s, la celda de `demo`
     2. ese Gateway no tiene HTTPRoute para ese host → contesta 404 (u otro inquilino, si
        alguien pusiera la ruta ahi)
     3. la consola pregunta `/salud` por la entrada → 404 → «no responde», y senala al
        cluster de `acme`, que esta perfectamente vivo en Y
   ⇒ No es una fuga: es una puerta que no lleva, y un diagnostico que miente.
""" % (",".join(comodin) or "la IP del comodin"))

titulo("LO QUE HAY QUE CERRAR")
for clase in ("hueco", "carretera"):
    los = [h for h in hallazgos if h[0] == clase]
    if not los:
        continue
    print("   [%s]" % clase.upper())
    for _, que, det in los:
        print("     · %s" % que)
        if det:
            print("       %s" % det)
huecos = sum(1 for h in hallazgos if h[0] == "hueco")
print()
if huecos:
    print("  => %d huecos. La E3-(a) es: la celda dice su puerta (un NOMBRE, no una IP); la entrada" % huecos)
    print("     de la organizacion resuelve a ella; el aprovisionador escribe el registro cuando la")
    print("     zona es suya y dice cual falta cuando no; y la consola coteja que el mundo converge.")
    sys.exit(1)
print("  => Sin huecos: la celda dice su puerta, la entrada lleva a ella, y la consola lo coteja.")
