"""ORE 0068 C2·3 pieza 0 · Lo que se manda al puesto pesado, celda a celda, como lo haría la consola.
Corre DENTRO del banco (`docker exec c2-banco python3 …`): ore-serve en 127.0.0.1:8080, identidad de
cabecera. Ana abre el puesto; el agente (en `ore-puesto-spark:local`, otro contenedor en la misma red)
lo reclama; cada celda es PySpark. Imprime una línea `caso · resultado` por celda.
"""
import json
import sys
import time
import urllib.error
import urllib.request

BASE = "http://127.0.0.1:8080"
ANA = {"x-ore-sujeto": "persona:ana"}
P = sys.argv[1] if len(sys.argv) > 1 else "puesto-ana-python"


def pide(m, ruta, cuerpo=None, cab=ANA):
    q = urllib.request.Request(BASE + ruta, method=m, data=json.dumps(cuerpo).encode() if cuerpo is not None else None)
    for k, v in cab.items():
        q.add_header(k, v)
    q.add_header("content-type", "application/json")
    try:
        with urllib.request.urlopen(q, timeout=60) as r:
            return r.status, json.loads(r.read() or b"null")
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()[:400]


def celda(texto, plazo=900):
    c, r = pide("POST", f"/puestos/{P}/ejecutar", {"texto": texto, "lenguaje": "python"})
    assert c == 202, (c, r)
    n, t = r["celda"], time.time()
    while time.time() - t < plazo:
        c, r = pide("GET", f"/puestos/{P}/celdas/{n}")
        if c == 200 and r.get("salida"):
            s = r["salida"]
            if s.get("tipo") == "error":
                return f"ERROR {s.get('nombre')}: {(s.get('mensaje') or s.get('traza', ''))[-500:]}"
            return (s.get("texto") or json.dumps(s)[:400]).strip()
        time.sleep(1)
    return "PLAZO"


def caso(nombre, texto, plazo=900):
    t = time.time()
    print(f"{nombre} · {celda(texto, plazo)} · {time.time() - t:.1f} s", flush=True)


# ── 0 · el puesto vivo (el agente ya lo reclamó) ─────────────────────────────
for _ in range(120):
    c, r = pide("GET", f"/puestos/{P}")
    if c == 200 and r.get("estado") == "vivo":
        break
    time.sleep(1)
print(f"puesto · {r.get('estado') if c == 200 else (c, r)}", flush=True)

# ── 1 · Spark dentro del agente: el catálogo `ore` con la identidad DEL PUESTO ──
#   En el clúster la cabecera sería `x-ore-pod` (el token proyectado); aquí, la de cabecera del banco.
#   `spark.driver.memory` se dice ANTES de que arranque la JVM (la arranca getOrCreate).
FUENTE_AUTH = open("/repo/pruebas-de-fuego/geo/c2/trabajo-spark/AuthPod.java", encoding="utf-8").read()
caso("1 · sesión Spark en una celda", f"""
import os, subprocess, time
from pyspark.sql import SparkSession
# el AuthManager de prueba (AuthPod.java), compilado aquí: en el producto iría en la imagen
os.makedirs("/tmp/auth/src", exist_ok=True)
open("/tmp/auth/src/AuthPod.java", "w").write({json.dumps(FUENTE_AUTH)})
subprocess.run(["sh", "-c", 'javac -proc:none -d /tmp/auth/c -cp "$SPARK_HOME/jars/*" /tmp/auth/src/AuthPod.java'
                " && cd /tmp/auth/c && jar cf /tmp/auth/authpod.jar ore"], check=True)
open("/tmp/identidad", "w").write(os.environ["ORE_SUJETO"])
t = time.time()
spark = (SparkSession.builder.master("local[*]").appName("trabajo")
  .config("spark.driver.memory", os.environ.get("DRIVER", "2g"))
  .config("spark.driver.extraClassPath", "/tmp/auth/authpod.jar")
  # `orep`: el mismo catálogo, con la identidad leída del fichero en cada petición
  .config("spark.sql.catalog.orep", "org.apache.iceberg.spark.SparkCatalog")
  .config("spark.sql.catalog.orep.type", "rest")
  .config("spark.sql.catalog.orep.uri", os.environ["ORE_SERVE"])
  .config("spark.sql.catalog.orep.rest.auth.type", "ore.AuthPod")
  .config("spark.sql.catalog.orep.ore.identidad.fichero", "/tmp/identidad")
  .config("spark.sql.catalog.orep.ore.identidad.cabecera", "x-ore-sujeto")
  .config("spark.sql.catalog.orep.header.x-ore-puesto", "{P}")
  .config("spark.sql.catalog.orep.header.X-Iceberg-Access-Delegation", "vended-credentials")
  .config("spark.sql.catalog.orep.io-impl", "org.apache.iceberg.aws.s3.S3FileIO")
  .config("spark.sql.catalog.orep.s3.path-style-access", "true")
  .config("spark.sql.catalog.orep.client.region", "auto")
  .config("spark.sql.catalog.ore", "org.apache.iceberg.spark.SparkCatalog")
  .config("spark.sql.catalog.ore.type", "rest")
  .config("spark.sql.catalog.ore.uri", os.environ["ORE_SERVE"])
  .config("spark.sql.catalog.ore.header.x-ore-sujeto", os.environ["ORE_SUJETO"])
  .config("spark.sql.catalog.ore.header.x-ore-puesto", "{P}")
  .config("spark.sql.catalog.ore.header.X-Iceberg-Access-Delegation", "vended-credentials")
  .config("spark.sql.catalog.ore.io-impl", "org.apache.iceberg.aws.s3.S3FileIO")
  .config("spark.sql.catalog.ore.s3.path-style-access", "true")
  .config("spark.sql.catalog.ore.client.region", "auto")
  .getOrCreate())
print(spark.version, spark.conf.get("spark.driver.memory"), round(time.time() - t, 1), "s")
""")

# ── 2 · escribir por /v1 desde el puesto, con Sedona ────────────────────────────
caso("2 · CREATE + INSERT por /v1", """
spark.sql("create namespace if not exists ore.geo")
spark.sql('''create table if not exists ore.geo.desde_puesto (id bigint, zona binary) using iceberg''')
spark.sql('''insert into ore.geo.desde_puesto select id, ST_AsBinary(ST_Buffer(ST_Point(440000 + id, 4474000), 100))
             from range(3)''')
print(spark.sql("select count(*) n, round(sum(ST_Area(ST_GeomFromWKB(zona)))) a from ore.geo.desde_puesto").first())
""")

# ── 2b · la identidad que cambia mientras corre (el token del pod se renueva) ───
caso("2b · identidad leída en cada petición", """
def escribir(quien):
    open("/tmp/identidad", "w").write(quien)
    try:
        spark.sql("insert into orep.geo.desde_puesto select 9, ST_AsBinary(ST_Point(0, 0))")
        return "va"
    except Exception as e:
        l = [x for x in str(e).splitlines() if "Exception" in x or "403" in x or "401" in x]
        return "NO: " + (l[0] if l else str(e).splitlines()[0])[:160]
import os
print([escribir(os.environ["ORE_SUJETO"]), escribir("agente:otro"), escribir(os.environ["ORE_SUJETO"])])
""")

# ── 3 · la memoria: el pod, la JVM y Python ─────────────────────────────────────
MEM = """
import os, subprocess
def kb(pid):
    for l in open(f"/proc/{pid}/status"):
        if l.startswith("VmRSS"): return int(l.split()[1]) // 1024
jvm = [p for p in os.listdir("/proc") if p.isdigit() and b"java" in open(f"/proc/{p}/cmdline", "rb").read()[:200]]
leer = lambda f: int(open(f).read().strip()) // 2**20 if os.path.exists(f) else None
print("cgroup max", leer("/sys/fs/cgroup/memory.max"), "MiB · actual", leer("/sys/fs/cgroup/memory.current"),
      "· pico", leer("/sys/fs/cgroup/memory.peak"), "· JVM", [kb(p) for p in jvm], "· python", kb(os.getpid()))
"""
caso("3 · memoria tras la sesión", MEM)

# ── 4 · un join espacial que pesa: puntos en polígonos ─────────────────────────
PUNTOS = int(sys.argv[2]) if len(sys.argv) > 2 else 2_000_000
caso(f"4 · ST_Contains {PUNTOS:,} puntos × 20k polígonos", """
import time
t = time.time()
spark.sql('''create or replace temp view pol as select id pid, ST_Buffer(ST_Point(440000 + (id % 200) * 500, 4474000 + (id / 200) * 500), 200) g from range(20000)''')
spark.sql('''create or replace temp view pun as select id, ST_Point(440000 + rand(1) * 100000, 4474000 + rand(2) * 50000) g from range(PUNTOS_N)''')
n = spark.sql("select count(*) n from pol join pun on ST_Contains(pol.g, pun.g)").first().n
print(n, "pares ·", round(time.time() - t, 1), "s")
""".replace("PUNTOS_N", str(PUNTOS)), plazo=1800)
caso("5 · memoria tras el join", MEM)

# ── 6 · lo que ORE ve de la tabla escrita desde el puesto ──────────────────────
c, r = pide("GET", "/v1/namespaces/geo/tables/desde_puesto")
m = r.get("metadata", {}) if isinstance(r, dict) else {}
s = (m.get("snapshots") or [{}])[-1].get("summary", {})
print("6 · /v1 · v%s · snapshots %d · ore.* %s · resumen %s" % (
    m.get("format-version"), len(m.get("snapshots") or []),
    {k: v for k, v in (m.get("properties") or {}).items() if k.startswith("ore.")},
    {k: v for k, v in s.items() if k.startswith("ore.") or k in ("operation", "added-records")}) if c == 200 else f"6 · /v1 · {c} {r}")
