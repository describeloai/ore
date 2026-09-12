# -*- coding: utf-8 -*-
"""Lo que cuesta el cluster encendido, y que se ahorra apagandolo de noche.

Medido el 2026-09-12. Ni una cifra de memoria: el inventario sale de `gcloud`
y `kubectl`, y los precios del catalogo de facturacion (Cloud Billing API),
para europe-west1 y on-demand. Lo que NO sale de aqui es la factura real —no
hay exportacion a BigQuery en el proyecto— asi que esto es lo CALCULADO con lo
que hay ahora mismo, y se dice.

  A. ESTA ENCENDIDO 24/7?      las operaciones del cluster, 14 dias
  B. QUE HAY                    nodos, discos, IPs, NAT, balanceadores
  C. QUE CUESTA POR HORA        y por mes, con y sin la cuota de GKE
  D. QUE SE AHORRA              pool a 0 de noche · spot · borrar el cluster

⭐⭐ La respuesta corta a «bajar el pool a 0 de noche» es: se ahorra SOLO el
  nodo, y el nodo es menos de la mitad. La cuota de gestion de GKE, las IPs
  estaticas, las reglas de balanceo, los discos y la pasarela NAT se cobran
  igual con cero nodos. Un cluster con cero nodos no es un cluster apagado.
"""
import json
import re
import subprocess
import sys
import urllib.request

for f in (sys.stdout, sys.stderr):
    try:
        f.reconfigure(errors="replace")
    except AttributeError:
        pass

P = "project-8853a180-450d-47be-b83"
REGION = "europe-west1"
H_MES = 730.0


def sh(cmd):
    return subprocess.run(cmd, capture_output=True, text=True, shell=True, encoding="utf-8", errors="replace").stdout


def titulo(t):
    print()
    print(t)
    print("=" * 74)


# ── precios ──────────────────────────────────────────────────────────────────
TOK = sh("gcloud auth print-access-token").strip()
SERVICIOS = {"computo": "6F81-5844-456A", "red": "E505-1604-58F8", "gke": "CCD8-9BF1-090E"}
_skus = {}


def skus(servicio):
    if servicio in _skus:
        return _skus[servicio]
    out, tok = [], None
    while True:
        u = "https://cloudbilling.googleapis.com/v1/services/%s/skus?pageSize=5000" % SERVICIOS[servicio]
        if tok:
            u += "&pageToken=" + tok
        r = urllib.request.Request(u, headers={"Authorization": "Bearer " + TOK})
        d = json.load(urllib.request.urlopen(r, timeout=90))
        out += d.get("skus", [])
        tok = d.get("nextPageToken")
        if not tok:
            _skus[servicio] = out
            return out


def precio(servicio, rx, excluye=("Spot", "Preemptible", "Commitment", "Sole Tenancy", "Custom", "Reserved", "Regional")):
    for s in skus(servicio):
        d = s["description"]
        if not re.search(rx, d) or any(x in d for x in excluye):
            continue
        if REGION not in s.get("serviceRegions", []) and "global" not in s.get("serviceRegions", []):
            continue
        t = s["pricingInfo"][0]["pricingExpression"]
        r = t["tieredRates"][-1]["unitPrice"]
        return int(r.get("units", 0)) + r.get("nanos", 0) / 1e9, t["usageUnitDescription"], d
    raise SystemExit("sin SKU para %s" % rx)


# ── A ────────────────────────────────────────────────────────────────────────
titulo("A - ESTA ENCENDIDO 24/7? Las operaciones del cluster, 14 dias")
ops = sh('gcloud container operations list --project %s --filter="startTime>-P14D" '
         '--format="csv[no-heading](startTime,operationType,targetLink.basename())"' % P)
apagados = [l for l in ops.splitlines() if "SET_NODE_POOL_SIZE" in l or "DELETE_CLUSTER" in l]
creado = [l for l in ops.splitlines() if "CREATE_CLUSTER" in l]
for l in ops.splitlines():
    if not any(x in l for x in ("AUTO_REPAIR", "UPGRADE_MASTER")):
        print("   " + l.replace(",", "  "))
print()
if creado:
    print("   creado el %s" % creado[0].split(",")[0][:16])
print("   operaciones de apagado (SET_NODE_POOL_SIZE / DELETE_CLUSTER): %d" % len(apagados))
print("   => %s" % ("se ha apagado alguna vez" if apagados else "ENCENDIDO 24/7 desde que se creo"))

# ── B ────────────────────────────────────────────────────────────────────────
titulo("B - QUE HAY")
nodos = [l.split() for l in sh("kubectl get nodes -o custom-columns=T:.metadata.labels.node\\.kubernetes\\.io/instance-type,S:.metadata.labels.cloud\\.google\\.com/gke-spot --no-headers").splitlines() if l.strip()]
discos = [l.split(",") for l in sh('gcloud compute disks list --project %s --format="csv[no-heading](sizeGb,type.basename())"' % P).splitlines() if l.strip()]
ips = [l.split(",") for l in sh('gcloud compute addresses list --project %s --format="csv[no-heading](name,status)"' % P).splitlines() if l.strip()]
lbs = set(l.split(",")[0] for l in sh('gcloud compute forwarding-rules list --project %s --format="csv[no-heading](IPAddress)"' % P).splitlines() if l.strip())
nats = [l for l in sh('gcloud compute routers list --project %s --format="value(name)"' % P).splitlines() if l.strip()]

print("   nodos:   %s" % ", ".join("%s%s" % (n[0], " (spot)" if len(n) > 1 and n[1] == "true" else "") for n in nodos))
print("   discos:  %s GB pd-balanced en %d discos" % (sum(int(d[0]) for d in discos), len(discos)))
print("   IPs:     %d estaticas (%s)" % (len(ips), ", ".join(i[0] for i in ips)))
print("   LBs:     %d balanceadores (por IP distinta)" % len(lbs))
print("   NAT:     %d pasarela(s)" % len(nats))

# ── C ────────────────────────────────────────────────────────────────────────
titulo("C - QUE CUESTA POR HORA, del catalogo")
vcpu, _, _ = precio("computo", r"^E2 Instance Core running in EMEA$")
ram, _, _ = precio("computo", r"^E2 Instance Ram running in EMEA$")
vcpu_spot, _, _ = precio("computo", r"^Spot Preemptible E2 Instance Core running in EMEA$", excluye=())
ram_spot, _, _ = precio("computo", r"^Spot Preemptible E2 Instance Ram running in EMEA$", excluye=())
pd, _, _ = precio("computo", r"^Balanced PD Capacity$")
ip, _, _ = precio("computo", r"^Static Ip Charge$")
nat_h, _, _ = precio("red", r"Cloud Nat Gateway Uptime")
nat_ip, _, _ = precio("red", r"Cloud NAT IP Usage")
lb, _, _ = precio("red", r"Forwarding Rule Minimum for Belgium")
gke, _, _ = precio("gke", r"^Zonal Kubernetes Clusters$")

# e2-standard-4: 4 vCPU, 16 GiB. Se lee del tipo y no se supone.
def maquina(tipo):
    m = re.match(r"e2-standard-(\d+)", tipo)
    n = int(m.group(1))
    return n, n * 4

filas = []
for n in nodos:
    c, g = maquina(n[0])
    # El nodo se cobra como lo que ES. Desde el 2026-09-12 es spot, y una medida
    # que siguiera cobrandolo on-demand diria $46/mes de mas.
    es_spot = len(n) > 1 and n[1] == "true"
    if es_spot:
        filas.append(("nodo %s SPOT" % n[0], c * vcpu_spot + g * ram_spot))
    else:
        filas.append(("nodo %s on-demand" % n[0], c * vcpu + g * ram))
filas.append(("GKE gestion (cluster zonal)", gke))
gb = sum(int(d[0]) for d in discos)
filas.append(("discos %d GB pd-balanced" % gb, gb * pd / H_MES))
# La IP de la NAT se cobra por el SKU de NAT, no por el de IP estatica.
filas.append(("%d IPs estaticas (2 LB + 1 NAT)" % len(ips), 2 * ip + nat_ip))
filas.append(("%d reglas de balanceo" % len(lbs), len(lbs) * lb))
filas.append(("pasarela NAT", nat_h))

total = sum(v for _, v in filas)
nodo = filas[0][1]
for k, v in filas:
    print("   %-34s $%.4f/h   %5.1f%%" % (k, v, 100 * v / total))
print("   %-34s $%.4f/h" % ("TOTAL", total))
print()
print("   al mes (730 h):            $%6.0f" % (total * H_MES))
print("   si la cuota de GKE va gratis: $%6.0f   (una zonal por cuenta de facturacion entra en el free tier)" % ((total - gke) * H_MES))
print("   ⚠️ calculado, no facturado: no hay exportacion de facturacion. No incluye trafico,")
print("      Artifact Registry ni el pool `jobs-p`, que escala de 0 y dura lo que dura un Job.")

# ── D ────────────────────────────────────────────────────────────────────────
titulo("D - QUE SE AHORRA")
suelo = total - nodo
print("   Con el pool a 0, sigue costando:  $%.4f/h = $%.0f/mes   (%.0f%% del total)" % (suelo, suelo * H_MES, 100 * suelo / total))
print("   El nodo es lo unico que se apaga:  $%.4f/h = $%.0f/mes   (%.0f%%)" % (nodo, nodo * H_MES, 100 * nodo / total))
print()
for horas in (8, 12, 14):
    print("   pool a 0 %2d h/dia:              ahorra $%3.0f/mes" % (horas, nodo * horas * 30))
c, g = maquina(nodos[0][0])
spot = c * vcpu_spot + g * ram_spot
demanda = c * vcpu + g * ram
if nodo == spot:
    print("   el nodo YA es spot:             ahorra $%3.0f/mes frente a on-demand, 24/7" % ((demanda - spot) * H_MES))
else:
    print("   nodo en SPOT, 24/7:             ahorra $%3.0f/mes   (nodo a $%.4f/h; se puede desalojar)" % ((nodo - spot) * H_MES, spot))
    print("   pool a 0 12h + spot:            ahorra $%3.0f/mes" % (nodo * 12 * 30 + (nodo - spot) * 12 * 30))
print("   BORRAR el cluster de noche:     ahorra $%3.0f/mes a 12h/dia (todo menos discos e IPs, que se quedan)" % ((total - gb * pd / H_MES - (2 * ip + nat_ip)) * 12 * 30))
print()
print("   ⛔ «Suspender» no existe en GKE: o bajas nodos, o borras el cluster. Y borrarlo es")
print("      volver a aprovisionar: Flux, Keycloak, la forja, las NetworkPolicy — minutos,")
print("      y todo lo que no este en git se pierde.")
