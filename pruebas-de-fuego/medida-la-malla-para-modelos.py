# -*- coding: utf-8 -*-
"""MEDIDA · qué modelos puede desplegar un cliente en la malla compartida, y hasta dónde escala.

Antes de construir Models → Deployments (2026-09-15): medir lo que la malla ES,
no lo que la cuota de la celda promete. Cuatro números mandan:

    1  el nodo        lo más grande que puede tener UN pod de servir (no hay servir multinodo)
    2  la malla       cuántos nodos puede llegar a tener (pools × autoescalado)
    3  el proyecto    la cuota de Google de CPU y GPU en la región: el muro de verdad
    4  la celda       lo que su ResourceQuota deja pedir

Es un METRO. Lo que sale se lee así: el modelo cabe si cabe en 1 y no lo prohíben 3 y 4.

    uso:  python pruebas-de-fuego/medida-la-malla-para-modelos.py
"""
import json
import subprocess

P = "project-8853a180-450d-47be-b83"
ZONA = "europe-west1-b"
REGION = "europe-west1"


def sh(cmd):
    r = subprocess.run(cmd, shell=True, capture_output=True, text=True, encoding="utf-8")
    return r.stdout.strip() if r.returncode == 0 else ""


def kj(*args):
    r = subprocess.run(["kubectl", *args, "-o", "json"], capture_output=True, text=True, encoding="utf-8")
    return json.loads(r.stdout) if r.returncode == 0 and r.stdout else {"items": []}


def mi(s):  # cantidad de Kubernetes → GiB
    s = str(s)
    if s.endswith("Ki"):
        return int(s[:-2]) / 2 ** 20
    if s.endswith("Mi"):
        return int(s[:-2]) / 2 ** 10
    if s.endswith("Gi"):
        return float(s[:-2])
    return float(s) / 2 ** 30


def m(s):  # cantidad de CPU → núcleos
    s = str(s)
    return int(s[:-1]) / 1000 if s.endswith("m") else float(s)


print()
print("  ═══ LA MALLA PARA MODELOS · %s · %s ═══" % (P.split("-")[0], ZONA))

# ── 1 · los nodos ────────────────────────────────────────────────────────────
print("\n  ① NODOS VIVOS (lo más grande que puede tener un pod)")
nodos = kj("get", "nodes")["items"]
for n in nodos:
    a = n["status"]["allocatable"]
    pool = n["metadata"]["labels"].get("cloud.google.com/gke-nodepool", "?")
    print("     %-42s %-13s %4.1f vCPU · %5.1f GiB · GPU %s" % (n["metadata"]["name"], pool, m(a["cpu"]), mi(a["memory"]), a.get("nvidia.com/gpu", "0")))
if not nodos:
    print("     (sin acceso a kubectl)")

# ── 2 · la malla: pools y autoescalado ───────────────────────────────────────
print("\n  ② POOLS Y AUTOESCALADO (hasta cuántos nodos)")
c = sh("gcloud container clusters describe ore-mesh --zone %s --project %s --format=json" % (ZONA, P))
maximo_vcpu = 0.0
maximo_gib = 0.0
gpu_pools = 0
if c:
    c = json.loads(c)
    nap = (c.get("autoscaling") or {}).get("enableNodeAutoprovisioning", False)
    for np_ in c.get("nodePools", []):
        mt = np_["config"]["machineType"]
        spec = sh("gcloud compute machine-types describe %s --zone %s --project %s --format=json" % (mt, ZONA, P))
        cpus, mem = (json.loads(spec)["guestCpus"], json.loads(spec)["memoryMb"] / 1024) if spec else (0, 0)
        asc = np_.get("autoscaling") or {}
        maxn = asc.get("maxNodeCount", np_.get("initialNodeCount", 1)) if asc.get("enabled") else np_.get("initialNodeCount", 1)
        acc = np_["config"].get("accelerators") or []
        if acc:
            gpu_pools += 1
        maximo_vcpu += cpus * maxn
        maximo_gib += mem * maxn
        print("     %-14s %-16s %s%s · hasta %d nodo(s) · %s" % (
            np_["name"], mt, "%d vCPU · %.0f GB" % (cpus, mem), " · SPOT" if np_["config"].get("spot") else "",
            maxn, ", ".join("%s×%d" % (x["acceleratorType"], x["acceleratorCount"]) for x in acc) or "sin acelerador"))
    print("     ⇒ tope nominal de la malla: %.0f vCPU · %.0f GB · pools con GPU: %d · auto-provisioning: %s" % (maximo_vcpu, maximo_gib, gpu_pools, "sí" if nap else "NO (nada nace fuera de estos pools)"))

# ── 3 · el proyecto: la cuota de Google ──────────────────────────────────────
print("\n  ③ CUOTA DEL PROYECTO (el muro)")
reg = sh("gcloud compute regions describe %s --project %s --format=json" % (REGION, P))
glob = sh("gcloud compute project-info describe --project %s --format=json" % P)
interes = {"CPUS", "E2_CPUS", "N2_CPUS", "G2_CPUS", "NVIDIA_L4_GPUS", "NVIDIA_T4_GPUS", "NVIDIA_A100_GPUS", "NVIDIA_H100_GPUS", "PREEMPTIBLE_NVIDIA_L4_GPUS"}
if reg:
    for q in json.loads(reg)["quotas"]:
        if q["metric"] in interes:
            print("     %-28s %5.0f / %-5.0f %s" % (q["metric"], q["usage"], q["limit"], "⛔ CERO: hay que pedirla" if q["limit"] == 0 and "GPU" in q["metric"] else ""))
if glob:
    for q in json.loads(glob)["quotas"]:
        if q["metric"] in ("CPUS_ALL_REGIONS", "GPUS_ALL_REGIONS"):
            print("     %-28s %5.0f / %-5.0f %s" % (q["metric"], q["usage"], q["limit"], "⛔ CERO" if q["limit"] == 0 else ""))
acel = sh("gcloud compute accelerator-types list --filter=zone:%s --project %s --format=value(name)" % (ZONA, P)).split()
print("     aceleradores que EXISTEN en la zona: %s" % ", ".join(a for a in acel if "vws" not in a))

# ── 4 · la celda ─────────────────────────────────────────────────────────────
print("\n  ④ LA CELDA (lo que su cuota deja pedir)")
for ns in [x["metadata"]["name"] for x in kj("get", "ns", "-l", "ore.dev/rol=cargas")["items"]]:
    q = kj("get", "resourcequota", "-n", ns)["items"]
    if q:
        st = q[0]["status"]
        print("     %-10s pide %4.1f de %4.1f vCPU · %5.1f de %4.0f GiB · GPU en la cuota: %s" % (
            ns, m(st["used"].get("requests.cpu", 0)), m(st["hard"]["requests.cpu"]),
            mi(st["used"].get("requests.memory", 0)), mi(st["hard"]["requests.memory"]),
            st["hard"].get("requests.nvidia.com/gpu", "no existe")))

print()
print("  Lectura: un modelo cabe si cabe en UN nodo (①), la malla puede levantarlo (②),")
print("  la cuota de Google lo deja (③) y la de la celda también (④). Hoy ③ dice GPU = 0.")
