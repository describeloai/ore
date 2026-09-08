# -*- coding: utf-8 -*-
"""Cual forja, y por que no vale un `git init --bare`.

`medida-donde-vive-el-arbol.py` dejo decidido el QUE: git, dentro del cluster.
Esto decide el CUAL, y las tres razones son medibles: lo que cabe en el nodo
que hay, de donde puede bajarse la imagen, y que hace una forja que un
repositorio pelado no hace.

  A. QUE CABE                 el hueco real del nodo, contra el suelo de cada una
  B. DE DONDE VIENE LA IMAGEN los nodos privados, y lo que eso obliga
  C. QUE NO DA UN REPO PELADO seis cosas, y ninguna es cosmetica
  D. EL DISCO QUE NO SE BORRA  ninguna clase retiene, y hay que escribirla
"""
import json
import subprocess
import textwrap

PROYECTO = "project-8853a180-450d-47be-b83"
CLUSTER = "ore-mesh"
ZONA = "europe-west1-b"


def parrafo(t, sangria="     ", ancho=72):
    for l in textwrap.wrap(t, ancho):
        print("%s%s" % (sangria, l))


def correr(*args):
    try:
        r = subprocess.run(list(args), capture_output=True, text=True, timeout=120)
    except (OSError, subprocess.TimeoutExpired):
        return None
    return r.stdout if r.returncode == 0 else None


def kubectl(*args):
    return correr("kubectl", *args)


def mili(v):
    """De `1930m` o `2` a milicores."""
    return int(v[:-1]) if v.endswith("m") else int(v) * 1000


def mib(v):
    """De `6170284Ki`, `32M` o `512Mi` a MiB.

    Los dos juegos de sufijos conviven en un manifiesto real: los binarios de
    Kubernetes (`Ki`, `Mi`, `Gi`) y los decimales de SI (`K`, `M`, `G`). Se
    tratan por separado porque no son lo mismo, y confundirlos es como se
    calcula mal el hueco de un nodo.
    """
    v = v.strip()
    for suf, f in [("Ki", 1 / 1024.0), ("Mi", 1.0), ("Gi", 1024.0), ("Ti", 1024.0 * 1024)]:
        if v.endswith(suf):
            return float(v[: -len(suf)]) * f
    for suf, f in [("k", 1e3), ("K", 1e3), ("M", 1e6), ("G", 1e9), ("T", 1e12)]:
        if v.endswith(suf):
            return float(v[: -len(suf)]) * f / (1024.0 * 1024.0)
    return float(v) / (1024.0 * 1024.0)


print("== cual forja, medido ==")

# -- A -----------------------------------------------------------------------
print()
print("A - QUE CABE EN EL NODO QUE HAY")
print()
nodos = kubectl("get", "nodes", "-o", "json")
libre_cpu = libre_mem = None
if nodos is None:
    print("   (sin cluster)")
else:
    for n in json.loads(nodos)["items"]:
        nombre = n["metadata"]["name"]
        pool = n["metadata"]["labels"].get("cloud.google.com/gke-nodepool", "?")
        cpu = mili(n["status"]["allocatable"]["cpu"])
        mem = mib(n["status"]["allocatable"]["memory"])
        # Lo que ya esta pedido en ese nodo.
        pods = kubectl("get", "pods", "-A", "--field-selector", "spec.nodeName=%s" % nombre, "-o", "json")
        pedido_cpu = pedido_mem = 0
        if pods:
            for p in json.loads(pods)["items"]:
                if p["status"].get("phase") not in ("Running", "Pending"):
                    continue
                for c in p["spec"]["containers"]:
                    r = c.get("resources", {}).get("requests", {})
                    if "cpu" in r:
                        pedido_cpu += mili(r["cpu"])
                    if "memory" in r:
                        pedido_mem += mib(r["memory"])
        print("   %s  (%s)" % (nombre, pool))
        print("     CPU      %5dm de %5dm    libre %5dm" % (pedido_cpu, cpu, cpu - pedido_cpu))
        print("     memoria  %5d MiB de %5d MiB  libre %5d MiB" % (pedido_mem, mem, mem - pedido_mem))
        if pool == "default-pool":
            libre_cpu, libre_mem = cpu - pedido_cpu, mem - pedido_mem

print()
FORJAS = [
    ("Forgejo", 100, 512, "AGPL, fundacion sin animo de lucro"),
    ("Gitea", 100, 512, "MIT; misma huella, gobierno de una empresa"),
    ("GitLab CE", 8000, 8192, "MIT; pide ocho vCPU y 8 GiB para ir comodo"),
]
print("   forja        CPU     memoria   cabe   licencia y gobierno")
print("   " + "-" * 74)
for nombre, c, m, nota in FORJAS:
    if libre_cpu is None:
        cabe = "?"
    else:
        cabe = "SI" if c <= libre_cpu and m <= libre_mem else "NO"
    print("   %-12s %5dm  %5d MiB  %-5s %s" % (nombre, c, m, cabe, nota))
print()
parrafo("El nodo estable de este cluster no da para GitLab, y no por poco: "
        "pide mas CPU de la que el nodo tiene ENTERO. No es una preferencia "
        "de estilo, es que no arranca.")

# -- B -----------------------------------------------------------------------
print()
print("B - DE DONDE PUEDE VENIR LA IMAGEN")
print()
pools = correr(
    "gcloud", "container", "node-pools", "list",
    "--cluster", CLUSTER, "--zone", ZONA, "--project", PROYECTO,
    "--format=value(name,networkConfig.enablePrivateNodes)",
)
routers = correr(
    "gcloud", "compute", "routers", "list", "--project", PROYECTO, "--format=value(name)"
)
if pools:
    print("   pool           nodos privados")
    print("   " + "-" * 74)
    for linea in pools.strip().splitlines():
        partes = linea.split("\t")
        print("   %-14s %s" % (partes[0], partes[1] if len(partes) > 1 else "no"))
print()
print("   Cloud NAT: %s" % ("SI" if (routers or "").strip() else "NO HAY"))
parrafo("Un pod en un pool privado sin NAT solo alcanza las APIs de Google por "
        "Private Google Access. ==> una imagen de `codeberg.org` NO se puede "
        "bajar ahi. El `default-pool` si tiene IP publica, y de ahi que la "
        "forja arranque HOY: un `nodeSelector` la ancla a ese pool.")
parrafo("Y hay que decir que eso es una atadura y no una solucion. Lo correcto "
        "es espejar la imagen en Artifact Registry, como `ore` y `ore-drivers`. "
        "**No esta hecho**: Cloud Build, recien habilitada, sigue contestando "
        "`SERVICE_DISABLED` por propagacion del agente de servicio. Mientras "
        "tanto, lo unico que impide que la forja se programe donde no puede "
        "arrancar es una etiqueta.")

# -- C -----------------------------------------------------------------------
print()
print("C - QUE NO DA UN `git init --bare`")
print()
FALTA = [
    ("crear el repo del inquilino", "hace falta una API, no un `ssh` y un `mkdir`"),
    ("organizaciones", "el limite entre inquilinos tiene que ser del almacen, no del camino"),
    ("tokens con alcance", "un Job necesita empujar a UN repo, no a todos"),
    ("webhooks", "es el disparo del reconciliador que hoy falta"),
    ("quien es cada quien", "un commit sin autor conocido no es auditoria"),
    ("mirarlo", "una consola que no puede enseñar el arbol no puede explicarlo"),
]
for que, porque in FALTA:
    print("   · %-30s %s" % (que, porque))
print()
parrafo("Guardar lo hace un repositorio pelado. Lo que no hace es GESTIONAR, y "
        "gestionado y centralizado para todos los clientes es justo lo que se "
        "pidio. Las seis se construirian a mano encima de `git init --bare`, y "
        "eso es escribir una forja peor.")

# -- D -----------------------------------------------------------------------
print()
print("D - EL DISCO QUE NO SE BORRA")
print()
sc = kubectl("get", "storageclass", "-o", "json")
if sc:
    clases = json.loads(sc)["items"]
    for c in clases:
        print("   %-16s %s" % (c["metadata"]["name"], c.get("reclaimPolicy")))
    retiene = [c["metadata"]["name"] for c in clases if c.get("reclaimPolicy") == "Retain"]
    print()
    print("   con `Retain`: %s" % (", ".join(retiene) if retiene else "NINGUNA"))
    parrafo("Para el arbol de un cliente eso significa que un `delete pvc` se "
            "lleva su ontologia sin tocar el disco. La clase que retiene hay "
            "que escribirla, y va en `malla/30-forja.yaml`.")
