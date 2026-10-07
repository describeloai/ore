"""vendorizar.py <entrada> — la base del cómputo de ORE Serverless Postgres (ADR 0058, P3·3), de las
releases de fuera a manifiestos nuestros, preparados para ore-mesh (GKE 1.35, Dataplane V2):

    cert-manager/cert-manager.yaml   cert-manager (los certificados de los webhooks de NeonVM)
    red/multus.yaml                  Multus 4.3.1 para GKE (B.3: multus-gke.yaml, lo único escrito a mano)
    red/whereabouts.yaml             el IPAM de la overlay
    neonvm/neonvm.yaml               CRDs, webhooks, device plugin, controller, vxlan, runner precargado
    neonvm/autoscaling.yaml          autoscale-scheduler y autoscaler-agent

Lo que se descarga en <entrada> (y nada más; las versiones se fijan aquí):

    gh release download v0.49.1 -R neondatabase/autoscaling -p '*.yaml' -D <entrada>
    gh release download v1.21.2 -R cert-manager/cert-manager -p cert-manager.yaml -D <entrada>

Lo que se cambia (era `preparar.py` en D0b; cada regla, medida allí):
- Todo Deployment/DaemonSet/StatefulSet, al pool `pg` (nodeSelector ore.dev/pool=neon + tolera ore.dev/neon):
  la base del cómputo es del producto, no del clúster (decidido 8).
- Las imágenes de Neon, las NUESTRAS: compiladas del fork por ci/neon/autoscaling.yaml en la misma etiqueta.
  El device plugin, fijado por digest (la release lo trae sin etiqueta: `latest`).
- Las rutas CNI de GKE: SOLO el hostPath /opt/cni/bin -> /home/kubernetes/bin; nunca las de dentro de la
  imagen ni los puntos de montaje (cambiarlos rompió vxlan y whereabouts).
- Reservas (requests) dimensionadas para nodos de 16–32 vCPU que no caben en un n2-standard-2: controller
  2 CPU × 3 → 200m × 1; scheduler 1 CPU → 200m; y lo que va en CADA nodo, a lo que usa (RESERVAS). Los
  LÍMITES no se tocan. En la puerta de producción (nodos grandes) se revisa.
- Los duplicados de la release (el runner precargado y las NADs vienen en dos ficheros), una sola vez.
- GKE sólo admite prioridad system-node-critical en un namespace con ResourceQuota para ella (B.3).
"""
import pathlib, sys, yaml

AUTOSCALING = "v0.49.1"
REGISTRO = "europe-west1-docker.pkg.dev/project-8853a180-450d-47be-b83/ore"
DEVICE_PLUGIN = "squat/generic-device-plugin@sha256:dc192e164c69b03f156765793a1be62ca437709ae477b27ca7d8f3dcf5021576"
NUESTRAS = ["neonvm-controller", "neonvm-runner", "neonvm-vxlan-controller", "autoscaler-agent", "autoscale-scheduler"]
TOL = {"key": "ore.dev/neon", "operator": "Equal", "value": "true", "effect": "NoSchedule"}
# Lo que ESTÁ en cada nodo del pool pide poco: con lo de Neon, un nodo nuevo reservaba 471m para
# DaemonSets que usan ~10m (medido en P3·3, `kubectl top`), y eso son VMs que no caben. Lo de una
# vez por clúster (controller, scheduler) pesa menos: 200m.
RESERVAS = {"neonvm-controller": {"cpu": "200m", "memory": "512Mi"},
            "autoscale-scheduler": {"cpu": "200m", "memory": "2000Mi"},       # usa ~90m
            "autoscaler-agent": {"cpu": "20m", "memory": "600Mi"},            # por nodo; usa 1–3m
            "neonvm-vxlan-controller": {"cpu": "20m", "memory": "50Mi"},      # por nodo; usa 1–5m
            "whereabouts": {"cpu": "20m", "memory": "100Mi"},                 # por nodo; usa ~1m
            "neonvm-device-plugin": {"cpu": "10m", "memory": "10Mi"}}         # por nodo; usa 3–4m

ENTRADA = pathlib.Path(sys.argv[1])
AQUI = pathlib.Path(__file__).parent
CABECERA = ("# GENERADO por malla/postgres-computo/vendorizar.py (ADR 0058, P3·3) desde {}.\n"
            "# No se edita a mano: se cambia vendorizar.py y se vuelve a generar.\n")


def imagen(i):
    for n in NUESTRAS:
        if i == f"ghcr.io/neondatabase/{n}:{AUTOSCALING}":
            return f"{REGISTRO}/{n}:{AUTOSCALING}"
    return DEVICE_PLUGIN if i == "squat/generic-device-plugin" else i


def preparar(d):
    if d.get("kind") not in ("Deployment", "DaemonSet", "StatefulSet"):
        return d
    s = d["spec"]["template"]["spec"]
    s.setdefault("nodeSelector", {})["ore.dev/pool"] = "neon"
    if TOL not in s.setdefault("tolerations", []):
        s["tolerations"].append(TOL)
    for v in s.get("volumes", []):
        if (v.get("hostPath") or {}).get("path") == "/opt/cni/bin":
            v["hostPath"]["path"] = "/home/kubernetes/bin"
    for c in s.get("containers", []) + s.get("initContainers", []):
        c["image"] = imagen(c["image"])
        for e in c.get("env", []):
            if "value" in e:
                e["value"] = imagen(e["value"])
    n = d["metadata"]["name"]
    if n in RESERVAS:
        s["containers"][0].setdefault("resources", {})["requests"] = RESERVAS[n]
    if n == "neonvm-controller":
        d["spec"]["replicas"] = 1
    return d


def leer(*nombres, base=ENTRADA):
    vistos, docs = set(), []
    for nombre in nombres:
        for d in yaml.safe_load_all(open(base / nombre, encoding="utf-8")):
            if not d:
                continue
            clave = (d["kind"], d["metadata"].get("namespace"), d["metadata"]["name"])
            if clave not in vistos:
                vistos.add(clave)
                docs.append(preparar(d))
    return docs


def escribir(ruta, docs, origen):
    ruta = AQUI / ruta
    ruta.parent.mkdir(exist_ok=True)
    with open(ruta, "w", encoding="utf-8", newline="\n") as f:
        f.write(CABECERA.format(origen))
        yaml.safe_dump_all(docs, f, sort_keys=False, width=1000)
    print(f"{ruta.relative_to(AQUI)}: {len(docs)} objetos")


CUOTA = {"apiVersion": "v1", "kind": "ResourceQuota",
         "metadata": {"name": "pods-criticos", "namespace": "neonvm-system"},
         "spec": {"hard": {"pods": "50"}, "scopeSelector": {"matchExpressions": [
             {"operator": "In", "scopeName": "PriorityClass", "values": ["system-node-critical"]}]}}}

escribir("cert-manager/cert-manager.yaml", leer("cert-manager.yaml"), "cert-manager v1.21.2")
escribir("red/multus.yaml", leer("multus-gke.yaml", base=AQUI / "red"), "red/multus-gke.yaml (B.3)")
escribir("red/whereabouts.yaml", leer("whereabouts.yaml"), f"neondatabase/autoscaling {AUTOSCALING}")
escribir("neonvm/neonvm.yaml",
         leer("neonvm.yaml", "neonvm-controller.yaml", "neonvm-vxlan-controller.yaml", "neonvm-runner-image-loader.yaml")
         + [CUOTA], f"neondatabase/autoscaling {AUTOSCALING}")
escribir("neonvm/autoscaling.yaml", leer("autoscale-scheduler.yaml", "autoscaler-agent.yaml"),
         f"neondatabase/autoscaling {AUTOSCALING}")
