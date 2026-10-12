#!/usr/bin/env python3
"""DORMIR LA MALLA DE NOCHE — desde DENTRO del clúster (ADR 0060 C1).

La malla vive de 08:00 a 00:00, hora de Madrid. La CronJob `dormir-de-noche`
(malla/18-el-horario.yaml, `timeZone: Europe/Madrid`) lo corre a las 00:00, 00:30… hasta las
02:30: la primera pasada duerme, y las siguientes sólo hacen algo si un trabajo de Kueue la tuvo
despierta (o alguien la despertó a mano de madrugada). Despertar a las 08:00 es de Cloud Scheduler
(`ore-despertar`), por la consola y `horario.yml`: dormida no hay CronJob que corra. Dormir se decide dentro porque las CronJobs del clúster corren
puntuales y el cron de GitHub no.

Con un workload de Kueue admitido y sin terminar no duerme: la pasada siguiente vuelve a mirar.

Al dormir: las copias (un Job desde cada CronJob de copia, y esperarlo), y los grupos a 0 por
la API de GKE (`gcloud … resize --async`). No vacía los nodos él: corre en uno de ellos. GKE
retira las máquinas y los pods reciben su parada.

Habla con la API de Kubernetes con la cuenta del pod (`horario`, permisos en
18-el-horario.yaml) y con Google como `ore-horario` (Workload Identity).
"""
import json
import os
import ssl
import subprocess
import sys
import time
import urllib.parse
import urllib.request
from datetime import datetime, timezone

P = os.environ.get("PROYECTO", "project-8853a180-450d-47be-b83")
ZONA = os.environ.get("ZONA", "europe-west1-b")
CLUSTER = os.environ.get("CLUSTER", "ore-mesh")
SECRETOS = "/var/run/secrets/kubernetes.io/serviceaccount"
API = f"https://{os.environ.get('KUBERNETES_SERVICE_HOST', 'kubernetes.default.svc')}:{os.environ.get('KUBERNETES_SERVICE_PORT', '443')}"
COPIAS = [("forja", "copia-de-la-forja"), ("identidad", "copia-del-idp")]
T0 = time.time()


def paso(m):
    print(f"{int(time.time() - T0):4d}s  {m}", flush=True)


def _ctx():
    return ssl.create_default_context(cafile=f"{SECRETOS}/ca.crt")


def k8s(metodo, ruta, cuerpo=None, texto=False):
    token = open(f"{SECRETOS}/token").read().strip()
    req = urllib.request.Request(API + ruta, method=metodo,
                                 data=json.dumps(cuerpo).encode() if cuerpo is not None else None,
                                 headers={"Authorization": f"Bearer {token}", "Content-Type": "application/json"})
    with urllib.request.urlopen(req, context=_ctx(), timeout=30) as r:
        datos = r.read().decode("utf-8", "replace")
    return datos if texto else json.loads(datos)


def trabajo_en_curso():
    """Los workloads de Kueue admitidos y sin terminar: el trabajo de alguien."""
    try:
        wl = k8s("GET", "/apis/kueue.x-k8s.io/v1beta2/workloads")["items"]
    except Exception:
        wl = []
    vivos = 0
    for w in wl:
        c = {x["type"]: x["status"] for x in w.get("status", {}).get("conditions", [])}
        if c.get("Admitted") == "True" and c.get("Finished") != "True":
            vivos += 1
    return vivos


def copiar(ns, cj):
    """Un Job desde la CronJob de copia, y esperarlo (10 min)."""
    try:
        c = k8s("GET", f"/apis/batch/v1/namespaces/{ns}/cronjobs/{cj}")
    except Exception:
        return True
    nombre = f"{cj}-al-dormir-{datetime.now(timezone.utc):%Y%m%d%H%M%S}"[:63]
    job = {"apiVersion": "batch/v1", "kind": "Job",
           "metadata": {"name": nombre, "namespace": ns,
                        "annotations": {"cronjob.kubernetes.io/instantiate": "manual"},
                        "labels": c["spec"]["jobTemplate"].get("metadata", {}).get("labels", {})},
           "spec": c["spec"]["jobTemplate"]["spec"]}
    k8s("POST", f"/apis/batch/v1/namespaces/{ns}/jobs", job)
    fin = time.time() + 600
    while time.time() < fin:
        s = k8s("GET", f"/apis/batch/v1/namespaces/{ns}/jobs/{nombre}").get("status", {})
        if s.get("succeeded"):
            paso(f"copia {ns}/{cj} hecha")
            return True
        if s.get("failed"):
            break
        time.sleep(5)
    paso(f"⛔ copia {ns}/{cj} no terminó (job {nombre})")
    return False


def gcloud(*args):
    r = subprocess.run(["gcloud", *args, "--project", P, "--quiet"], capture_output=True, text=True)
    if r.returncode:
        paso(f"⚠ gcloud {' '.join(args[:4])}: {r.stderr.strip().splitlines()[-1] if r.stderr.strip() else r.returncode}")
    return r.returncode == 0


def main():
    nodos = k8s("GET", "/api/v1/nodes?labelSelector=cloud.google.com%2Fgke-nodepool%3Dsistema-spot")["items"]
    if not nodos:
        paso("ya duerme: ningún nodo de sistema")
        return 0
    vivos = trabajo_en_curso()
    if vivos:
        paso(f"hay {vivos} trabajo(s) de Kueue en curso: no duerme (la pasada siguiente vuelve a mirar)")
        return 0
    paso("de noche en Madrid: a dormir")
    vecinos = k8s("GET", "/api/v1/nodes?labelSelector=ore.dev%2Fpool%3Dneon")["items"]
    copias = COPIAS + ([("ore-pg", "copia-de-la-base-del-controlador")] if vecinos else [])
    ok = all([copiar(ns, cj) for ns, cj in copias])
    if not ok:
        paso("⛔ una copia falló: NO duerme (la próxima pasada lo vuelve a intentar)")
        return 1
    # Lo que escribe primero (Postgres, los puestos y builds); el sistema, el último: este pod
    # vive en él, y la orden ya está dada cuando GKE lo retire.
    # ⚠️ GKE hace una operación a la vez por clúster: las de antes, esperando; sólo la última
    #   (la del grupo donde vive este pod) va con --async. Y sólo lo que tiene algo que hacer.
    r = subprocess.run(["gcloud", "container", "node-pools", "describe", "pg", "--cluster", CLUSTER,
                        "--zone", ZONA, "--project", P, "--format=value(autoscaling.enabled)"],
                       capture_output=True, text=True)
    if r.stdout.strip().lower() == "true":
        gcloud("container", "node-pools", "update", "pg", "--cluster", CLUSTER, "--zone", ZONA,
               "--no-enable-autoscaling")
        paso("pg sin autoescalado")
    # ⛔ Y se comprueba que quedó en 0, no se da por hecho. El 2026-10-10 a las 21:03 el
    #   autoescalador de `pg` estaba creando un nodo justo cuando se apagó: el resize a 0
    #   terminó «DONE», se llevó el viejo, y el que nacía quedó con el grupo en objetivo 1 y
    #   ore-pg arrancado encima. Tras cada resize se mira otra vez, hasta tres veces.
    # ⛔ Los `PodDisruptionBudget` de ore-pg (`proxy-postgres` minAvailable 1, `safekeeper`
    #   maxUnavailable 1) no dejan desalojar a nadie cuando se apaga el grupo entero: no hay
    #   otro nodo adonde ir. GKE espera hasta UNA HORA antes de forzarlo (2026-10-10: 21:27 →
    #   atascado hasta borrarlos a mano a las 21:43). Las copias ya están hechas; se terminan
    #   los pods de esos nodos con su gracia, que es lo que GKE haría al cabo de la hora.
    for pool in ("pg", "jobs-s"):
        nodos = {n["metadata"]["name"] for n in k8s(
            "GET", f"/api/v1/nodes?labelSelector=cloud.google.com%2Fgke-nodepool%3D{pool}")["items"]}
        for nodo in sorted(nodos):
            sel = urllib.parse.quote(f"spec.nodeName={nodo}")
            for p in k8s("GET", f"/api/v1/pods?fieldSelector={sel}")["items"]:
                m = p["metadata"]
                if m["namespace"] == "kube-system" or any(
                        o.get("kind") == "DaemonSet" for o in m.get("ownerReferences", [])):
                    continue
                try:
                    k8s("DELETE", f"/api/v1/namespaces/{m['namespace']}/pods/{m['name']}")
                except Exception as e:  # que uno falle no impide dormir: el resize lo forzará
                    paso(f"⚠️ no se pudo terminar {m['namespace']}/{m['name']}: {e}")
        if nodos:
            paso(f"{pool}: terminados los pods de {len(nodos)} nodo(s) antes del resize")
    for pool in ("pg", "jobs-s"):
        # ⛔ El primer resize va SIEMPRE, haya nodos o no: «no hay nodos» no es «objetivo 0».
        #   2026-10-11 08:02: el autoescalado de `pg` se había encendido un minuto antes y su nodo
        #   aún no existía; se saltó el resize, el autoescalador lo creó igual, y quedó `pg`
        #   encendido con la malla dormida. Un resize a 0 de un grupo ya a 0 son segundos.
        for intento in range(3):
            hay = k8s("GET", f"/api/v1/nodes?labelSelector=cloud.google.com%2Fgke-nodepool%3D{pool}")["items"]
            if intento and not hay:
                break
            if intento:
                paso(f"{pool}: siguen {len(hay)} nodos tras el resize — otra vez")
                time.sleep(30)
            if gcloud("container", "clusters", "resize", CLUSTER, "--node-pool", pool,
                      "--num-nodes", "0", "--zone", ZONA):
                paso(f"{pool} → 0")
        else:
            paso(f"⛔ {pool}: siguen nodos tras tres resize a 0 — mírese a mano")
    if gcloud("container", "clusters", "resize", CLUSTER, "--node-pool", "sistema-spot",
              "--num-nodes", "0", "--zone", ZONA, "--async"):
        paso("sistema-spot → 0 pedido")
    paso("✓ a dormir: GKE retira las máquinas")
    return 0


if __name__ == "__main__":
    sys.exit(main())
