"""ORE 0068 C3·c0 · El driver de un Build elástico, a mano: Spark en modo cliente contra la API de Kubernetes,
con dynamic allocation (ORE pone el suelo y el techo, Spark decide por la carga). Corre los casos de
`CASOS` (separados por comas) y escribe una línea `C3C|caso|clave=valor …` por resultado, y
`C3C|ejecutores|t=… n=…` cada vez que cambia el número de ejecutores vivos.

  vap      el código del cliente usa el token del driver: lo que no es un ejecutor, la política lo niega
  pequeno  20 M puntos × 20 k zonas difundidas (el join de 6c): un Build pequeño en frío
  grande   200 M puntos × 20 k zonas: tiene que escalar
  ocioso   tras `grande`, cuánto tarda en devolver ejecutores (el shuffle tracking los retiene)
  sesgo    90 % de los puntos en una zona, 300 k zonas sin difundir (RangeJoin): ¿OOM con un ejecutor?
  udf      una UDF de Python que pide más memoria que `pyspark.memory`
"""
import json
import os
import ssl
import threading
import time
import urllib.error
import urllib.request

T0 = time.time()
NS = os.environ["NS"]
IMAGEN = os.environ["IMAGEN"]
MAX = int(os.environ.get("MAX", "2"))
CASOS = [c for c in os.environ.get("CASOS", "pequeno").split(",") if c]
SA = "/var/run/secrets/kubernetes.io/serviceaccount"


def linea(caso, **kv):
    print("C3C|%s|%s" % (caso, " ".join("%s=%s" % (k, v) for k, v in kv.items())), flush=True)


def api(metodo, ruta, cuerpo=None):
    """La API de Kubernetes con el token del driver, como lo haría el código de un cliente."""
    ctx = ssl.create_default_context(cafile=SA + "/ca.crt")
    tok = open(SA + "/token").read()
    r = urllib.request.Request("https://kubernetes.default.svc" + ruta, method=metodo,
                               data=json.dumps(cuerpo).encode() if cuerpo else None,
                               headers={"Authorization": "Bearer " + tok, "Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(r, context=ctx, timeout=20) as f:
            return f.status, ""
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read() or b"{}").get("message", "")[:160]


def caso_vap():
    base = {"apiVersion": "v1", "kind": "Pod",
            "metadata": {"name": "intruso", "labels": {"spark-role": "executor"},
                         "ownerReferences": [{"apiVersion": "v1", "kind": "Pod", "name": os.environ["POD_NAME"],
                                              "uid": os.environ["POD_UID"], "controller": True}]},
            "spec": {"serviceAccountName": "ejecutor", "automountServiceAccountToken": False,
                     "securityContext": {"runAsNonRoot": True, "runAsUser": 65532,
                                         "seccompProfile": {"type": "RuntimeDefault"}},
                     "containers": [{"name": "c", "image": IMAGEN, "command": ["sleep", "5"],
                                     "resources": {"limits": {"memory": "1Gi", "cpu": "100m"}},
                                     "securityContext": {"allowPrivilegeEscalation": False,
                                                         "capabilities": {"drop": ["ALL"]}}}]}}

    def con(**cambios):
        p = json.loads(json.dumps(base))
        for ruta, v in cambios.items():
            d = p
            *pre, ult = ruta.split("__")
            for k in pre:
                d = d[k] if not isinstance(d, list) else d[int(k)]
            if isinstance(d, list):
                d[int(ult)] = v
            else:
                d[ult] = v
        return p

    pruebas = [
        ("otra imagen", con(spec__containers__0__image="busybox:1.36"), 422),
        ("la cuenta del driver", con(spec__serviceAccountName="driver-pesado"), 422),
        # Lo para antes Pod Security `restricted` (403): otra capa, el mismo no.
        ("un volumen del nodo", con(spec__volumes=[{"name": "h", "hostPath": {"path": "/"}}]), 403),
        ("sin dueño", con(metadata__ownerReferences=[]), 422),
        ("100Gi", con(spec__containers__0__resources={"limits": {"memory": "100Gi"}}), 422),
        ("fuera del pool pesado", base, 422),
        ("un ConfigMap cualquiera", None, 422),
        ("un Secret (fuera del Role)", None, 403),
        ("un ejecutor como los de Spark", con(spec__nodeSelector={"ore.dev/pool": "pesado"},
                                              spec__tolerations=[{"key": "ore.dev/pesado", "operator": "Equal",
                                                                  "value": "true", "effect": "NoSchedule"}]), 201),
    ]
    for nombre, cuerpo, espera in pruebas:
        if nombre.startswith("un ConfigMap"):
            st, msg = api("POST", "/api/v1/namespaces/%s/configmaps" % NS,
                          {"apiVersion": "v1", "kind": "ConfigMap", "metadata": {"name": "x"}, "data": {"a": "b"}})
        elif nombre.startswith("un Secret"):
            st, msg = api("POST", "/api/v1/namespaces/%s/secrets" % NS,
                          {"apiVersion": "v1", "kind": "Secret", "metadata": {"name": "x"}})
        else:
            st, msg = api("POST", "/api/v1/namespaces/%s/pods" % NS, cuerpo)
        linea("vap", prueba=nombre.replace(" ", "_"), status=st, espera=espera,
              ok=("OK" if st == espera else "FALLA"), msg=msg.replace(" ", "_")[:120])
    api("DELETE", "/api/v1/namespaces/%s/pods/intruso" % NS)


if "vap" in CASOS:
    caso_vap()
    CASOS.remove("vap")
if not CASOS:
    raise SystemExit(0)

from pyspark.sql import SparkSession  # noqa: E402
from pyspark.sql import functions as F  # noqa: E402

PREFIJO = "c3c0-" + os.environ["POD_NAME"][-5:]
b = (SparkSession.builder.master("k8s://https://kubernetes.default.svc:443").appName("c3c0")
     .config("spark.kubernetes.namespace", NS)
     .config("spark.kubernetes.driver.pod.name", os.environ["POD_NAME"])
     .config("spark.kubernetes.authenticate.caCertFile", SA + "/ca.crt")
     .config("spark.kubernetes.authenticate.oauthTokenFile", SA + "/token")
     .config("spark.driver.host", os.environ["POD_IP"])
     .config("spark.driver.port", "7078").config("spark.driver.blockManager.port", "7079")
     .config("spark.blockManager.port", "7079")
     .config("spark.driver.memory", os.environ.get("HEAP_DRIVER", "8g"))
     .config("spark.driver.maxResultSize", "2g")
     .config("spark.kubernetes.container.image", IMAGEN)
     .config("spark.kubernetes.container.image.pullPolicy", "IfNotPresent")
     .config("spark.kubernetes.executor.podTemplateFile", "/m/plantilla.yaml")
     .config("spark.kubernetes.executor.podTemplateContainerName", "ejecutor")
     .config("spark.kubernetes.executor.podNamePrefix", PREFIJO)
     .config("spark.kubernetes.executor.request.cores", "7")
     .config("spark.executor.cores", "7")
     .config("spark.executor.memory", os.environ.get("HEAP_EJECUTOR", "34g"))
     .config("spark.executor.memoryOverhead", "4g")
     .config("spark.executor.pyspark.memory", os.environ.get("PYMEM", "4g"))
     .config("spark.pyspark.python", "python3")
     .config("spark.authenticate", "true")
     .config("spark.dynamicAllocation.enabled", "true")
     .config("spark.dynamicAllocation.shuffleTracking.enabled", "true")
     .config("spark.dynamicAllocation.shuffleTracking.timeout", os.environ.get("RETENER", "120s"))
     .config("spark.dynamicAllocation.minExecutors", "1")
     .config("spark.dynamicAllocation.initialExecutors", "1")
     .config("spark.dynamicAllocation.maxExecutors", str(MAX))
     .config("spark.dynamicAllocation.executorIdleTimeout", "60s")
     .config("spark.dynamicAllocation.schedulerBacklogTimeout", "1s")
     .config("spark.decommission.enabled", "true")
     .config("spark.storage.decommission.enabled", "true")
     .config("spark.storage.decommission.shuffleBlocks.enabled", "true")
     .config("spark.storage.decommission.rddBlocks.enabled", "true")
     .config("spark.kubernetes.decommission.script", "/ore/decom.sh")
     .config("spark.sql.shuffle.partitions", str(64 * MAX)))
# KUEUE=1: los ejecutores, pods sueltos en la cola de su celda (la vía de Kueue para spark-submit).
if os.environ.get("KUEUE") == "1":
    b = (b.config("spark.kubernetes.executor.label.kueue.x-k8s.io/queue-name", "cola-ejecutores")
         .config("spark.kubernetes.executor.label.kueue.x-k8s.io/managed", "true")
         .config("spark.kubernetes.executor.annotation.kueue.x-k8s.io/pod-suspending-parent", "spark"))
# SEDONA_JOIN_NUMPARTITION=… → sedona.join.numpartition=… (y cualquier otra de Sedona).
for k, v in os.environ.items():
    if k.startswith("SEDONA_"):
        b = b.config(k.lower().replace("_", "."), v)
s = b.getOrCreate()
sc = s.sparkContext._jsc.sc()
linea("sesion", s="%.1f" % (time.time() - T0))

vivos = {"n": -1, "max": 0}


def vigia():
    while True:
        n = sc.getExecutorMemoryStatus().size() - 1
        if n != vivos["n"]:
            vivos["n"] = n
            vivos["max"] = max(vivos["max"], n)
            linea("ejecutores", t="%.1f" % (time.time() - T0), n=n)
        time.sleep(1)


threading.Thread(target=vigia, daemon=True).start()


def zonas(z=20000, lado_m=100000):
    lado = int(z ** 0.5)
    return s.sql("select id, ST_SetSRID(ST_Buffer(ST_Point(400000 + (id %% %d) * %f, 4430000 + floor(id / %d) * %f), "
                 "300, false, 'quad_segs=8'), 25830) zona from range(%d)" % (lado, lado_m / lado, lado, lado_m / lado, z))


def puntos(n, partes):
    return s.sql("select id, ST_SetSRID(ST_Point(400000 + rand(1) * 100000, 4430000 + rand(2) * 100000), 25830) geom "
                 "from range(0, %d, 1, %d)" % (n, partes))


def join_6c(nombre, n):
    puntos(n, 224 if n <= 20_000_000 else 448).createOrReplaceTempView("puntos")
    F.broadcast(zonas()).createOrReplaceTempView("zonas")
    t = time.time()
    r = s.sql("select z.id, count(*) n from zonas z join puntos p on ST_Contains(z.zona, p.geom) group by z.id")
    f = r.agg(F.sum("n").alias("pares"), F.count("*").alias("zonas")).first()
    linea(nombre, puntos=n, s="%.1f" % (time.time() - t), desde_el_arranque="%.1f" % (time.time() - T0),
          pares=f["pares"], zonas=f["zonas"], ejecutores_max=vivos["max"])


for caso in CASOS:
    try:
        if caso == "pequeno":
            join_6c("pequeno", 20_000_000)
        elif caso == "grande":
            join_6c("grande", 200_000_000)
        elif caso == "ocioso":
            t = time.time()
            while time.time() - t < int(os.environ.get("OCIOSO_S", "300")) and vivos["n"] > 1:
                time.sleep(2)
            linea("ocioso", s="%.1f" % (time.time() - t), quedan=vivos["n"])
        elif caso == "sesgo":
            # 90 % de los puntos dentro de la zona 0 (un círculo de 300 m); 300 k zonas: no se difunden.
            n = int(os.environ.get("PUNTOS_SESGO", "100000000"))
            # SESGO_DECIMOS=0: los mismos puntos, uniformes (la referencia).
            s.sql(("select id, ST_SetSRID(if(id %% 10 < %s, ST_Point(400000 + rand(1) * 200, 4430000 + rand(2) * 200),"
                   " ST_Point(400000 + rand(3) * 100000, 4430000 + rand(4) * 100000)), 25830) geom "
                   "from range(0, %d, 1, 448)") % (os.environ.get("SESGO_DECIMOS", "9"), n)).createOrReplaceTempView("puntos")
            zonas(300000).createOrReplaceTempView("zonas")
            # En memoria Spark las estima pequeñas y las difundiría: el sesgo que importa es el de RangeJoin.
            s.conf.set("spark.sql.autoBroadcastJoinThreshold", "-1")
            t = time.time()
            r = s.sql("select z.id, count(*) n from zonas z join puntos p on ST_Contains(z.zona, p.geom) group by z.id")
            plan = r._jdf.queryExecution().executedPlan().toString()
            join = next((j for j in ("BroadcastIndexJoin", "RangeJoin", "BroadcastNestedLoopJoin") if j in plan), "otro")
            f = r.agg(F.sum("n").alias("pares"), F.max("n").alias("max_zona")).first()
            linea("sesgo", join=join, s="%.1f" % (time.time() - t), pares=f["pares"], max_zona=f["max_zona"],
                  numpartition=os.environ.get("SEDONA_JOIN_NUMPARTITION", "-1"),
                  lado=os.environ.get("SEDONA_JOIN_SPATITIONSIDE", "left"), ejecutores_max=vivos["max"])
        elif caso == "udf":
            @F.udf("long")
            def come(x):
                b_ = bytearray(int(os.environ.get("UDF_GB", "8")) << 30)
                return len(b_)
            t = time.time()
            s.range(0, 7, 1, 7).select(come("id")).collect()
            linea("udf", resultado="sin_error", s="%.1f" % (time.time() - t))
    except Exception as e:  # noqa: BLE001
        m = str(e)
        clave = next((k for k in ("MemoryError", "OutOfMemoryError", "OOMKilled", "exit code 137", "ExecutorLostFailure",
                                  "Container killed") if k in m), "otro")
        linea(caso, error=clave, s="%.1f" % (time.time() - T0), msg=m.splitlines()[0][:200].replace(" ", "_"))
s.stop()
