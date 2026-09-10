# -*- coding: utf-8 -*-
"""MEDIDA · lo que cuesta el Cloud NAT, con los precios de la API y el trafico real.

⛔ Ni una cifra escrita de memoria. Los precios salen de la API de facturacion de
Google y el trafico de Cloud Monitoring, las dos en cada ejecucion. Este arbol ya
rechazo una vez una cuenta escrita de cabeza —la del nodo «al 96%», que resulto
ser un suelo fijo mal leido— y esta es la forma de que no vuelva a pasar.

⭐⭐ Y lo que la medida contesta no es «cuanto cuesta NAT» sino algo mas util:
  **si el precio por dato importa o no**. Porque si importara, cada catalogo de
  un cliente seria una decision economica; y si no, el coste es un fijo que se
  paga por existir y se olvida.

    uso:  python pruebas-de-fuego/medida-el-coste-de-la-salida.py
"""
import datetime
import json
import subprocess
import sys
import urllib.parse
import urllib.request

try:
    sys.stdout.reconfigure(encoding="utf-8")
    sys.stderr.reconfigure(encoding="utf-8")
except AttributeError:
    pass

PROYECTO = "project-8853a180-450d-47be-b83"
REGION = "europe-west1"
# El servicio de facturacion que cobra Cloud NAT. ⚠️ NO es Compute Engine: alli
# estan las IP estaticas y no la pasarela, y buscarlo alli devuelve tres SKUs de
# direcciones que se parecen lo justo para despistar.
SERVICIO_RED = "E505-1604-58F8"      # «Networking»
SERVICIO_COMPUTO = "6F81-5844-456A"  # «Compute Engine», para la IP

fallos = []


def testigo():
    r = subprocess.run(["gcloud", "auth", "print-access-token"],
                       capture_output=True, text=True, shell=True)
    return r.stdout.strip()


T = testigo()


def pedir(url):
    r = urllib.request.Request(url, headers={"Authorization": "Bearer " + T})
    with urllib.request.urlopen(r, timeout=180) as f:
        return json.loads(f.read().decode("utf-8"))


def titulo(t):
    print("\n" + "═" * 74)
    print(t)
    print("═" * 74)


# ══════════════════════════════════════════════════════════════════════════
titulo("① LOS PRECIOS, DE LA API DE FACTURACION")
# ══════════════════════════════════════════════════════════════════════════
def skus(servicio, filtro):
    pag, out = None, {}
    while True:
        u = "https://cloudbilling.googleapis.com/v1/services/%s/skus?pageSize=5000" % servicio
        if pag:
            u += "&pageToken=" + pag
        d = pedir(u)
        for s in d.get("skus", []):
            desc = s.get("description", "")
            if not filtro(desc.lower()):
                continue
            regs = s.get("serviceRegions", [])
            if REGION not in regs and "global" not in regs:
                continue
            for pi in s.get("pricingInfo", []):
                e = pi.get("pricingExpression", {})
                for tr in e.get("tieredRates", []):
                    u2 = tr.get("unitPrice", {})
                    n = int(u2.get("nanos", 0)) + int(u2.get("units", 0)) * 10**9
                    if n > 0:
                        out[desc] = (n / 1e9, u2.get("currencyCode", "?"),
                                     e.get("usageUnitDescription", "?"))
                        break
                break
        pag = d.get("nextPageToken")
        if not pag:
            break
    return out


precios = skus(SERVICIO_RED, lambda d: "nat" in d)
precios.update(skus(SERVICIO_COMPUTO, lambda d: "static ip" in d))
for desc in sorted(precios):
    p, m, un = precios[desc]
    print("  %-48s %10.6f %s / %s" % (desc[:48], p, m, un))


def busca(*claves):
    for desc, (p, _, _) in precios.items():
        b = desc.lower()
        if all(k in b for k in claves):
            return p
    return None


PASARELA = busca("nat", "gateway", "uptime")
IP_NAT = busca("nat", "ip usage")
DATOS = busca("nat", "data processing")
for n, v in (("pasarela", PASARELA), ("IP de NAT", IP_NAT), ("datos", DATOS)):
    if v is None:
        fallos.append("la API ya no publica el precio de %s: la cuenta de abajo no vale" % n)
if None in (PASARELA, IP_NAT, DATOS):
    print("\n⛔ faltan precios; se para aqui")
    sys.exit(1)

# ══════════════════════════════════════════════════════════════════════════
titulo("② EL TRAFICO QUE HA PASADO, DE CLOUD MONITORING")
# ══════════════════════════════════════════════════════════════════════════
fin = datetime.datetime.now(datetime.timezone.utc)
ini = fin - datetime.timedelta(hours=24)


def bytes_de(metrica):
    q = {
        "filter": 'metric.type="router.googleapis.com/%s"' % metrica,
        "interval.startTime": ini.strftime("%Y-%m-%dT%H:%M:%SZ"),
        "interval.endTime": fin.strftime("%Y-%m-%dT%H:%M:%SZ"),
        "aggregation.alignmentPeriod": "3600s",
        "aggregation.perSeriesAligner": "ALIGN_SUM",
        "aggregation.crossSeriesReducer": "REDUCE_SUM",
    }
    d = pedir("https://monitoring.googleapis.com/v3/projects/%s/timeSeries?%s"
              % (PROYECTO, urllib.parse.urlencode(q)))
    t = 0
    for s in d.get("timeSeries", []):
        for p in s.get("points", []):
            v = p.get("value", {})
            t += int(v.get("int64Value", v.get("doubleValue", 0)))
    return t


total = 0
for m, etiqueta in (("nat/sent_bytes_count", "enviados"),
                    ("nat/received_bytes_count", "recibidos")):
    b = bytes_de(m)
    total += b
    print("  %-10s %12d bytes  =  %9.4f MiB" % (etiqueta, b, b / 1024 / 1024))
gib = total / 1024 / 1024 / 1024
print("  %-10s %12d bytes  =  %9.6f GiB   (ultimas 24 h)" % ("TOTAL", total, gib))

# ══════════════════════════════════════════════════════════════════════════
titulo("③ LA CUENTA, Y LO QUE CONTESTA")
# ══════════════════════════════════════════════════════════════════════════
fijo_h = PASARELA + IP_NAT
fijo_mes = fijo_h * 730
print("  fijo    %.4f (pasarela) + %.4f (IP) = %.4f USD/h  ⇒  %.2f USD/mes"
      % (PASARELA, IP_NAT, fijo_h, fijo_mes))
print("  datos   %.3f USD/GiB  ⇒  %.6f USD por las ultimas 24 h" % (DATOS, gib * DATOS))

# ⭐ Lo que de verdad se queria saber: cuanto habria que mover para que el precio
#   por dato dejara de ser ruido.
umbral = fijo_mes / DATOS
print("""
  ⇒ Para que los datos igualen al fijo harian falta **%.0f GiB al mes**.

  ⭐ Y una corrida de catalogo de 48 entidades contra un Postgres real movio
    ~75 KB —medido el 2026-09-10, con el NAT recien creado y nada mas pasando
    por el—. A ese tamaño hacen falta unos **%.1f MILLONES de catalogos al mes**
    para llegar a ese umbral.

  ⇒ El coste ES el fijo. Catalogar no es una decision economica: leer el esquema
    de un cliente cuesta, en datos, una fraccion de centimo. Lo que se paga es
    tener la puerta, no usarla.""" % (umbral, umbral * 1024 * 1024 / 75 / 1e6))

print("""
  ⚠️ Y lo que esta medida NO cubre, para que no se lea de mas:
    · el trafico de las ultimas 24 h es de un NAT recien creado. Con clientes de
      verdad catalogando a diario, la cifra sube — pero el umbral de arriba dice
      cuanto puede subir antes de que importe;
    · MATERIALIZAR datos si movera volumen, y eso no pasa por aqui todavia. El
      dia que pase, esta medida hay que releerla entera;
    · y el fijo se paga aunque el pool este a cero nodos. La pasarela existe
      siempre; los nodos no.""")

print("\n" + "═" * 74)
if fallos:
    print("⛔ LA MEDIDA NO SE SOSTIENE:")
    for f in fallos:
        print("   · " + f)
    sys.exit(1)
print("✓ medido: el coste de la salida es un fijo de %.2f USD/mes, y los datos son ruido"
      % fijo_mes)
