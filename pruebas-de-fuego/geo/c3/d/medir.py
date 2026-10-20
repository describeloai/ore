"""ORE 0068 C3·d · SpatialBench a escala 100 con el Build elástico: este pod es el driver (como el de
c0 y el que rinde `cola::al_driver_elastico`), pide de 1 a MAX ejecutores según la carga, y corre las
12 consultas de SpatialBench (commit 8e44bdd, `consultas.json`) sobre el Parquet del bucket del banco.
Una línea `C3D|caso|…` por consulta (segundos, filas, huella como C0, ejecutores) y
`C3D|ejecutores|t=… n=…` cada vez que cambia el número de ejecutores.

Con la imagen desplegada (`puesto-spark:1`): `ReglasDeOre` (el `LEFT JOIN` de C3·b y la rejilla sobre
el lado grande de c2) va en la sesión por `spark-defaults.conf`. La difusión de C3·a la pone `over()`
del SDK, que aquí no se usa: estas consultas son el SQL de SpatialBench tal cual.
"""
import hashlib
import json
import os
import threading
import time

T0 = time.time()
NS = os.environ["NS"]
IMAGEN = os.environ["IMAGEN"]
MAX = int(os.environ.get("MAX", "2"))
LIMITE = int(os.environ.get("LIMITE_S", "600"))
DATOS = os.environ["DATOS"]          # gs://…/spatialbench/sf100
MUESTRA = int(os.environ.get("MUESTRA", "0"))  # cuántas filas imprimir por consulta
CRUDO = os.environ.get("CRUDO") == "1"         # y sin normalizar (repr), para cotejar con otro motor
# Unas `ReglasDeOre` por delante de las de la imagen (`/m/ore-spark-reglas.jar`, de `c3/b/regla.sh`): para
# medir una regla antes de desplegarla. Las reglas corren en el driver: basta con su classpath.
REGLAS = "/m/ore-spark-reglas.jar" if os.path.exists("/m/ore-spark-reglas.jar") else None
SOLO = set(filter(None, os.environ.get("CASOS", "").split(",")))
SA = "/var/run/secrets/kubernetes.io/serviceaccount"


def linea(caso, **kv):
    print("C3D|%s|%s" % (caso, " ".join("%s=%s" % (k, v) for k, v in kv.items())), flush=True)


from pyspark.sql import SparkSession  # noqa: E402

PREFIJO = "c3d-" + os.environ["POD_NAME"][-5:]
b = (SparkSession.builder.master("k8s://https://kubernetes.default.svc:443").appName("c3d")
     .config("spark.kubernetes.namespace", NS)
     .config("spark.kubernetes.driver.pod.name", os.environ["POD_NAME"])
     .config("spark.kubernetes.authenticate.caCertFile", SA + "/ca.crt")
     .config("spark.kubernetes.authenticate.oauthTokenFile", SA + "/token")
     .config("spark.driver.host", os.environ["POD_IP"])
     .config("spark.driver.port", "7078").config("spark.driver.blockManager.port", "7079")
     .config("spark.blockManager.port", "7079")
     .config("spark.driver.memory", "8g").config("spark.driver.maxResultSize", "4g")
     .config("spark.kubernetes.container.image", IMAGEN)
     .config("spark.kubernetes.container.image.pullPolicy", "Always")
     .config("spark.kubernetes.executor.podTemplateFile", "/m/plantilla.yaml")
     .config("spark.kubernetes.executor.podTemplateContainerName", "ejecutor")
     .config("spark.kubernetes.executor.podNamePrefix", PREFIJO)
     .config("spark.kubernetes.executor.request.cores", "7")
     .config("spark.executor.cores", "7")
     .config("spark.executor.memory", "34g").config("spark.executor.memoryOverhead", "4g")
     .config("spark.executor.pyspark.memory", "4g")
     .config("spark.pyspark.python", "python3")
     .config("spark.authenticate", "true")
     .config("spark.dynamicAllocation.enabled", "true")
     .config("spark.dynamicAllocation.shuffleTracking.enabled", "true")
     .config("spark.dynamicAllocation.shuffleTracking.timeout", "120s")
     .config("spark.dynamicAllocation.minExecutors", "1")
     .config("spark.dynamicAllocation.initialExecutors", "1")
     .config("spark.dynamicAllocation.maxExecutors", str(MAX))
     .config("spark.dynamicAllocation.executorIdleTimeout", "60s")
     .config("spark.dynamicAllocation.schedulerBacklogTimeout", "1s")
     .config("spark.scheduler.minRegisteredResourcesRatio", "0")
     .config("spark.decommission.enabled", "true")
     .config("spark.storage.decommission.enabled", "true")
     .config("spark.storage.decommission.shuffleBlocks.enabled", "true")
     .config("spark.kubernetes.decommission.script", "/opt/ore/spark/decom.sh")
     .config("spark.sql.shuffle.partitions", str(64 * MAX))
     # El Parquet del bucket: el conector de Google (lo baja medir.sh), repartido a los ejecutores;
     # la identidad, la del pod (Workload Identity de la cuenta `sb`).
     .config("spark.jars", "/tmp/gcs-connector.jar")
     .config("spark.hadoop.fs.gs.impl", "com.google.cloud.hadoop.fs.gcs.GoogleHadoopFileSystem")
     .config("spark.hadoop.fs.AbstractFileSystem.gs.impl", "com.google.cloud.hadoop.fs.gcs.GoogleHadoopFS")
     .config("spark.hadoop.fs.gs.auth.type", "APPLICATION_DEFAULT")
     # El conector 3.0.4 no casa con la lectura vectorizada del Hadoop de Spark 4.1
     # (`NoSuchMethodError` en `VectoredReadUtils.validateRangeRequest`): Parquet lee como siempre.
     .config("spark.hadoop.parquet.hadoop.vectored.io.enabled", "false"))
if REGLAS:
    b = b.config("spark.driver.extraClassPath", REGLAS)
s = b.getOrCreate()
s.sparkContext.setLogLevel("ERROR")
sc = s.sparkContext._jsc.sc()
linea("sesion", s="%.1f" % (time.time() - T0), reglas="locales" if REGLAS else "imagen")

vivos = {"n": -1, "max": 0}


def vigia():
    while True:
        try:
            n = sc.getExecutorMemoryStatus().size() - 1
        except Exception:  # noqa: BLE001 — al cerrar
            return
        if n != vivos["n"]:
            vivos["n"] = n
            vivos["max"] = max(vivos["max"], n)
            linea("ejecutores", t="%.1f" % (time.time() - T0), n=n)
        time.sleep(1)


threading.Thread(target=vigia, daemon=True).start()

for t in ["trip", "zone", "building", "customer", "driver", "vehicle"]:
    p = "%s/%s.parquet" % (DATOS, t)
    try:
        s.read.parquet(p).createOrReplaceTempView(t)
    except Exception as e:  # noqa: BLE001
        p2 = "%s/%s" % (DATOS, t)
        s.read.parquet(p2).createOrReplaceTempView(t)
linea("tablas", s="%.1f" % (time.time() - T0))


def norma(v):
    import datetime
    import decimal
    if isinstance(v, (float, decimal.Decimal)):
        return round(float(v), 4)
    if isinstance(v, datetime.timedelta):
        return round(v.total_seconds(), 1)
    if isinstance(v, (bytes, bytearray, memoryview)):
        return "bin%d" % len(bytes(v))
    if isinstance(v, (list, tuple)):
        return [norma(x) for x in v]
    if hasattr(v, "isoformat"):
        return v.isoformat()
    return v if isinstance(v, (int, str, bool, type(None))) else str(v)


def huella(filas):
    filas = sorted(json.dumps(norma(list(f)), default=str) for f in filas)
    return hashlib.sha256("\n".join(filas).encode()).hexdigest()[:16]


consultas = json.load(open("/m/consultas.json"))
# Un previo en vez de las consultas (`PREVIO=lentas.py bash medir.sh`): con esta sesión y estas tablas.
if os.path.exists("/m/previo.py"):
    exec(open("/m/previo.py").read())
    s.stop()
    raise SystemExit(0)
for caso, sql in consultas.items():
    if SOLO and caso not in SOLO:
        continue
    vivos["max"] = max(vivos["n"], 0)
    r, grupo, t = {}, "c3d-" + caso, time.time()
    plan = s.sql(sql)._jdf.queryExecution().executedPlan().toString()
    join = ",".join(j for j in ("BroadcastIndexJoin", "RangeJoin", "DistanceJoin", "BroadcastNestedLoopJoin",
                                "SortMergeJoin", "BroadcastHashJoin") if j in plan) or "-"

    def ir():
        s.sparkContext.setJobGroup(grupo, grupo, True)
        try:
            r["f"] = [tuple(x) for x in s.sql(sql).collect()]
        except Exception as e:  # noqa: BLE001
            # La causa de verdad: la última línea «…Exception: …» que no es de py4j.
            lineas = [x.strip() for x in str(e).splitlines() if "Exception" in x or "Error:" in x]
            lineas = [x for x in lineas if "Py4J" not in x and not x.startswith("at ")] or [str(e).splitlines()[0]]
            r["e"] = lineas[-1][:300].replace(" ", "_")

    h = threading.Thread(target=ir, daemon=True)
    h.start()
    h.join(LIMITE)
    if h.is_alive():
        s.sparkContext.cancelJobGroup(grupo)
        linea(caso, estado="limite", s=LIMITE, join=join, ejecutores_max=vivos["max"])
        h.join(120)
        continue
    if "e" in r:
        linea(caso, estado="error", s="%.1f" % (time.time() - t), join=join, error=r["e"])
        continue
    linea(caso, estado="bien", s="%.1f" % (time.time() - t), filas=len(r["f"]), huella=huella(r["f"]),
          join=join, ejecutores_max=vivos["max"])
    if MUESTRA:  # las filas normalizadas, para cotejar una huella que no casa
        filas = sorted((repr(tuple(f)) for f in r["f"]) if CRUDO else
                       (json.dumps(norma(list(f)), default=str) for f in r["f"]))
        for f in filas[:MUESTRA]:
            print("C3D|%s|fila|%s" % (caso, f), flush=True)
s.stop()
