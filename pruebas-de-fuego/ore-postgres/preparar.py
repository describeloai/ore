"""D0b · prepara los manifiestos de neondatabase/autoscaling v0.49.1 para ore-mesh (GKE 1.35, Dataplane V2).

- Todo DaemonSet/Deployment/StatefulSet: nodeSelector ore.dev/pool=neon + tolera ore.dev/neon.
- Las rutas CNI de GKE: SOLO el hostPath /opt/cni/bin -> /home/kubernetes/bin. Las rutas DENTRO de
  las imágenes y los puntos de montaje no se tocan (medido: cambiarlos rompió vxlan y whereabouts).
- Multus: el de la release (bitnami 3.9.3) no entiende CNI 1.1.0, que es lo que escribe GKE 1.35
  («unsupported CNI result version "1.1.0"»). Se usa el DaemonSet oficial de multus v4.3.1 (thin).
- Reservas que no caben en un n2-standard-2: el controlador (2 CPU × 3 réplicas) y el planificador
  (1 CPU) se bajan para la medida; los límites no se tocan.
"""
import yaml, pathlib

M = pathlib.Path("C:/tmp/d0b/m")
OUT = pathlib.Path("C:/tmp/d0b/listo"); OUT.mkdir(exist_ok=True)
FICHEROS = ["cert-manager.yaml", "neonvm.yaml", "multus-v4.yaml", "whereabouts.yaml", "neonvm-controller.yaml",
            "neonvm-vxlan-controller.yaml", "neonvm-runner-image-loader.yaml", "autoscale-scheduler.yaml",
            "autoscaler-agent.yaml"]
TOL = {"key": "ore.dev/neon", "operator": "Equal", "value": "true", "effect": "NoSchedule"}

resumen = []
for nombre in FICHEROS:
    docs = [d for d in yaml.safe_load_all(open(M / nombre, encoding="utf-8")) if d]
    for d in docs:
        if d.get("kind") not in ("DaemonSet", "Deployment", "StatefulSet"):
            continue
        spec = d["spec"]["template"]["spec"]
        spec.setdefault("nodeSelector", {})["ore.dev/pool"] = "neon"
        tols = spec.setdefault("tolerations", [])
        if TOL not in tols:
            tols.append(TOL)
        for v in spec.get("volumes", []):
            hp = v.get("hostPath")
            if hp and hp.get("path") == "/opt/cni/bin":
                hp["path"] = "/home/kubernetes/bin"
        for c in spec.get("containers", []) + spec.get("initContainers", []):
            if c.get("image", "").endswith("multus-cni:snapshot"):
                c["image"] = "ghcr.io/k8snetworkplumbingwg/multus-cni:v4.3.1"
        n = d["metadata"]["name"]
        if n == "neonvm-controller":
            d["spec"]["replicas"] = 1
            spec["containers"][0]["resources"]["requests"] = {"cpu": "200m", "memory": "512Mi"}
        if n == "autoscale-scheduler":
            spec["containers"][0]["resources"]["requests"]["cpu"] = "200m"
        resumen.append(f"{nombre}: {d['kind']}/{n}")
    yaml.safe_dump_all(docs, open(OUT / nombre, "w", encoding="utf-8"), sort_keys=False, width=1000)
print("\n".join(resumen))
