"""ORE 0072 F2·0 · M3: el borde y el navegador. El motor de laboratorio sirviendo los edificios de F1·3
(`F2_ML`) detrás de la entrada real de victor (Cloudflare → túnel → Envoy), en una ruta aparte y temporal
(`/f2-teselas/…`), con `Cache-Control: immutable`, `ETag`, CORS y la firma en la consulta (`?firma=`). Desde
fuera: ¿la cachea Cloudflare (`cf-cache-status`), con qué latencia en frío y en caliente, y la firma entra en la
clave de caché? El navegador (MapLibre desde otro origen), con `m3.html`.

  python pruebas-de-fuego/geo/f2/m3.py montar F2_ML=gs://…     # el Job, el Service, la NetworkPolicy, la HTTPRoute
  python pruebas-de-fuego/geo/f2/m3.py medir                    # desde aquí, por https://victor.ore.paladio.io
  python pruebas-de-fuego/geo/f2/m3.py desmontar                # todo fuera

Nada toca `ore-serve` ni la ruta `entrada`: es otra `HTTPRoute` sobre el mismo host, con su prefijo.
"""
import json
import math
import os
import subprocess
import sys
import time
import urllib.request

NS = "t-victor"
HOST = "victor.ore.paladio.io"
PREFIJO = "/f2-teselas"
G = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))) + os.sep
ETIQUETA = {"ore.dev/f2": "teselas"}


def kubectl(*a, entrada=None):
    p = subprocess.run(["kubectl", "-n", NS, *a], input=entrada, capture_output=True, text=True)
    if p.returncode != 0:
        raise SystemExit(p.stderr)
    return p.stdout


SERVIR = r'''
import os, subprocess, sys, urllib.request
sys.path.insert(0, "/opt/ore")
import ore
tok = ore._token_de_google()
r = urllib.request.Request("https://storage.googleapis.com/project-8853a180-450d-47be-b83-banco-geo/computo/f2/f2-motor",
                           headers={"authorization": "Bearer " + tok})
open("/tmp/f2-motor", "wb").write(urllib.request.urlopen(r, timeout=120).read())
os.chmod("/tmp/f2-motor", 0o755)
con = ore._duckdb(); con.execute("load httpfs")
meta = ore._iceberg(os.environ["F2_ML"]).replace("iceberg_scan(", "iceberg_metadata(", 1)
fs = [p for p, st in con.execute(f"select file_path, status from {meta}").fetchall()
      if str(p).endswith(".parquet") and str(st) != "DELETED"]
print("M3|ficheros|%d" % len(fs), flush=True)
os.execve("/tmp/f2-motor", ["/tmp/f2-motor", "servir", "8097"], dict(os.environ, F2_FICHEROS=",".join(fs)))
'''


def montar(ml):
    d = json.load(open(G + "salud/job-base.json", encoding="utf-8"))
    d["metadata"]["name"] = "f2-teselas"
    d["spec"]["activeDeadlineSeconds"] = 7200
    t = d["spec"]["template"]
    t["metadata"]["labels"].update(ETIQUETA)
    s = t["spec"]
    k = s["containers"][0]
    k["command"] = ["python3", "-u", "-c", SERVIR]
    k["image"] = "europe-west1-docker.pkg.dev/project-8853a180-450d-47be-b83/ore/puesto-python:1"
    k["imagePullPolicy"] = "Always"
    k["env"] = [e for e in k["env"] if e["name"] not in ("NS", "F2_ML")] + [{"name": "NS", "value": NS}, {"name": "F2_ML", "value": ml}]
    k["ports"] = [{"containerPort": 8097}]
    k["resources"] = {"requests": {"cpu": "1", "memory": "2Gi"}, "limits": {"cpu": "2", "memory": "2Gi"}}
    k["readinessProbe"] = {"tcpSocket": {"port": 8097}, "periodSeconds": 2}
    kubectl("create", "-f", "-", entrada=json.dumps(d))
    otros = [
        {"apiVersion": "v1", "kind": "Service", "metadata": {"name": "f2-teselas", "labels": ETIQUETA},
         "spec": {"selector": ETIQUETA, "ports": [{"port": 8097, "targetPort": 8097}]}},
        # Envoy (el del túnel) entra al Job, sólo a su puerto: lo mismo que `entrada-de-la-puerta-del-tunel`
        {"apiVersion": "networking.k8s.io/v1", "kind": "NetworkPolicy", "metadata": {"name": "entrada-a-f2-teselas", "labels": ETIQUETA},
         "spec": {"podSelector": {"matchLabels": ETIQUETA}, "policyTypes": ["Ingress"],
                  "ingress": [{"from": [{"namespaceSelector": {"matchLabels": {"kubernetes.io/metadata.name": "envoy-gateway-system"}},
                                         "podSelector": {"matchLabels": {"app.kubernetes.io/name": "envoy"}}}],
                               "ports": [{"protocol": "TCP", "port": 8097}]}]}},
        {"apiVersion": "gateway.networking.k8s.io/v1", "kind": "HTTPRoute", "metadata": {"name": "f2-teselas", "labels": ETIQUETA},
         "spec": {"parentRefs": [{"name": "puerta-tunel", "namespace": "ore-system", "sectionName": "ore"}],
                  "hostnames": [HOST],
                  "rules": [{"matches": [{"path": {"type": "PathPrefix", "value": PREFIJO}}],
                             "backendRefs": [{"name": "f2-teselas", "port": 8097}]}]}},
    ]
    for o in otros:
        kubectl("apply", "-f", "-", entrada=json.dumps(o))
    # Y Envoy sale sólo a `ore-serve` (`envoy` en envoy-gateway-system): una política más, aparte y aditiva,
    # que le deja llegar a este Job y a nada más; se va con `desmontar`.
    salida = {"apiVersion": "networking.k8s.io/v1", "kind": "NetworkPolicy",
              "metadata": {"name": "envoy-a-f2-teselas", "namespace": "envoy-gateway-system", "labels": ETIQUETA},
              "spec": {"podSelector": {"matchLabels": {"app.kubernetes.io/name": "envoy"}}, "policyTypes": ["Egress"],
                       "egress": [{"to": [{"namespaceSelector": {"matchLabels": {"kubernetes.io/metadata.name": NS}},
                                           "podSelector": {"matchLabels": ETIQUETA}}],
                                   "ports": [{"protocol": "TCP", "port": 8097}]}]}}
    subprocess.run(["kubectl", "apply", "-f", "-"], input=json.dumps(salida), text=True, check=True)
    print(kubectl("wait", "--for=condition=ready", "pod", "-l", "ore.dev/f2=teselas", "--timeout=300s"))


def desmontar():
    subprocess.run(["kubectl", "-n", "envoy-gateway-system", "delete", "networkpolicy", "envoy-a-f2-teselas", "--ignore-not-found"])
    for k, n in (("httproute", "f2-teselas"), ("networkpolicy", "entrada-a-f2-teselas"), ("service", "f2-teselas"), ("job", "f2-teselas")):
        subprocess.run(["kubectl", "-n", NS, "delete", k, n, "--ignore-not-found", "--wait=false"])


def tesela_de(lon, lat, z):
    n = 2 ** z
    return int((lon + 180) / 360 * n), int((1 - math.asinh(math.tan(math.radians(lat))) / math.pi) / 2 * n)


def pedir(url):
    t0 = time.time()
    # un agente de navegador: Cloudflare niega el de urllib (403, la protección contra bots)
    r = urllib.request.urlopen(urllib.request.Request(url, headers={"origin": "http://localhost:8765", "user-agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) f2-m3"}), timeout=60)
    b = r.read()
    return {"ms": round((time.time() - t0) * 1000), "kb": round(len(b) / 1024, 1), "cf": r.headers.get("cf-cache-status"),
            "edad": r.headers.get("age"), "etag": r.headers.get("etag"), "cc": r.headers.get("cache-control"),
            "cors": r.headers.get("access-control-allow-origin"), "motor": r.headers.get("server-timing")}


def medir():
    base = "https://%s%s/t" % (HOST, PREFIJO)
    sal = str(int(time.time()))  # teselas que nadie ha pedido antes en el borde
    for z in (13, 14, 15, 16):
        x, y = tesela_de(-3.7038, 40.4168, z)
        out = {"z": z}
        for vez, firma in (("1a", "a" + sal), ("2a_misma", "a" + sal), ("3a_otra_firma", "b" + sal)):
            try:
                out[vez] = pedir("%s/%d/%d/%d.mvt?firma=%s" % (base, z, x, y, firma))
            except Exception as e:
                out[vez] = {"error": str(e)[:200]}
            time.sleep(0.5)
        print("M3|borde|" + json.dumps(out), flush=True)


if __name__ == "__main__":
    orden = sys.argv[1]
    kv = dict(a.split("=", 1) for a in sys.argv[2:] if "=" in a)
    {"montar": lambda: montar(kv["F2_ML"]), "medir": medir, "desmontar": desmontar}[orden]()
