"""Query Q0 · ¿Sirve Spark Connect como el motor elástico de Query? Corre DENTRO del driver de un Build
elástico (cuenta `driver-pesado`, `cola-driver`, la configuración de `ore.spark._elastico()`), pero en
vez de un Build arranca un **servidor Spark Connect** y le habla un cliente en el mismo pod. Mide:
  · si la imagen trae el servidor (el jar) y qué le falta al cliente (grpcio…: se instala del bucket);
  · cuánto tarda el servidor en escuchar, y una consulta trivial en caliente (el suelo de latencia);
  · la primera consulta con trabajo (el ejecutor en frío), la misma en caliente, y una que pide más
    ejecutores; cuántos ejecutores hay en cada momento (los pods `spark-role=executor`);
  · cuándo se devuelven los ejecutores sobrantes, y dos sesiones a la vez (aislamiento y reparto).
Una línea JSON por medida."""
import json
import os
import socket
import ssl
import subprocess
import sys
import threading
import time
import urllib.request

SPARK_HOME = os.environ["SPARK_HOME"]
B = "project-8853a180-450d-47be-b83-banco-geo"
RUEDAS = ["grpcio-1.84.1-cp314-cp314-manylinux2014_x86_64.manylinux_2_17_x86_64.whl",
          "grpcio_status-1.84.1-py3-none-any.whl",
          "zstandard-0.25.0-cp314-cp314-manylinux2014_x86_64.manylinux_2_17_x86_64.manylinux_2_28_x86_64.whl"]
CUENTA = "/var/run/secrets/kubernetes.io/serviceaccount"
T0 = time.time()


def di(**k):
    k["t"] = round(time.time() - T0, 1)
    print(json.dumps(k), flush=True)


def token_gcs():
    req = urllib.request.Request("http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token",
                                 headers={"Metadata-Flavor": "Google"})
    return json.load(urllib.request.urlopen(req, timeout=10))["access_token"]


def ejecutores():
    """Los pods de ejecutor de este driver y su fase, por la API de Kubernetes con la cuenta del pod."""
    ns = open(CUENTA + "/namespace").read().strip()
    ctx = ssl.create_default_context(cafile=CUENTA + "/ca.crt")
    req = urllib.request.Request("https://kubernetes.default.svc/api/v1/namespaces/%s/pods?labelSelector=spark-role%%3Dexecutor" % ns,
                                 headers={"Authorization": "Bearer " + open(CUENTA + "/token").read().strip()})
    pods = json.load(urllib.request.urlopen(req, context=ctx, timeout=10))["items"]
    mio = os.environ["ORE_POD_NOMBRE"]
    return sorted((p["metadata"]["name"][-6:], p["status"].get("phase")) for p in pods
                  if any(o.get("name") == mio for o in p["metadata"].get("ownerReferences", [])))


def vigia(parar):
    antes = None
    while not parar.is_set():
        try:
            e = ejecutores()
        except Exception as x:  # noqa: BLE001
            e = "error: %s" % x
        if e != antes:
            di(evento="ejecutores", pods=e, corriendo=sum(1 for _, f in e if f == "Running") if isinstance(e, list) else None)
            antes = e
        time.sleep(2)


def instalar_cliente():
    d = "/trabajo/ruedas"
    os.makedirs(d, exist_ok=True)
    tok = token_gcs()
    for r in RUEDAS:
        req = urllib.request.Request("https://storage.googleapis.com/%s/query/ruedas/%s" % (B, r), headers={"Authorization": "Bearer " + tok})
        with urllib.request.urlopen(req, timeout=60) as resp, open(os.path.join(d, r), "wb") as f:
            f.write(resp.read())
    p = subprocess.run([sys.executable, "-m", "pip", "install", "-q", "--no-index", "--no-deps", "--target", "/trabajo/py"]
                       + [os.path.join(d, r) for r in RUEDAS], capture_output=True, text=True)
    sys.path.insert(0, "/trabajo/py")
    import importlib
    importlib.invalidate_caches()
    return p.returncode, p.stderr[-300:]


def escucha(puerto, plazo):
    t = time.time()
    while time.time() - t < plazo:
        try:
            socket.create_connection(("127.0.0.1", puerto), timeout=1).close()
            return True
        except OSError:
            time.sleep(0.5)
    return False


def medir(nombre, f, veces=1):
    for vez in range(1, veces + 1):
        t = time.time()
        try:
            r = f()
            di(consulta=nombre, vez=vez, s=round(time.time() - t, 2), resultado=str(r)[:120])
        except Exception as e:  # noqa: BLE001
            di(consulta=nombre, vez=vez, s=round(time.time() - t, 2), error=str(e).splitlines()[0][:300])


def main():
    jars = [j for j in os.listdir(os.path.join(SPARK_HOME, "jars")) if "connect" in j.lower()]
    di(evento="imagen", jars_connect=jars, sbin=os.path.exists(os.path.join(SPARK_HOME, "sbin", "start-connect-server.sh")))
    codigo, err = instalar_cliente()
    try:
        import grpc  # noqa: F401
        import pyspark.sql.connect.session  # noqa: F401
        cliente = "ok"
    except Exception as e:  # noqa: BLE001
        cliente = str(e)[:300]
    di(evento="cliente", pip=codigo, pip_error=err if codigo else "", importa=cliente)

    sys.path.insert(0, "/opt/ore")
    import ore.spark as s

    conf = s._elastico()
    master = conf.pop("master")
    conf["spark.driver.memory"] = s._memoria_del_driver()
    conf["spark.connect.grpc.binding.port"] = "15002"
    args = [os.path.join(SPARK_HOME, "bin", "spark-submit"), "--master", master,
            "--class", "org.apache.spark.sql.connect.service.SparkConnectServer", "--name", "query-q0"]
    for k, v in conf.items():
        args += ["--conf", "%s=%s" % (k, v)]
    args.append("spark-internal")
    log = open("/trabajo/servidor.log", "w")
    t = time.time()
    srv = subprocess.Popen(args, stdout=log, stderr=subprocess.STDOUT)
    parar = threading.Event()
    threading.Thread(target=vigia, args=(parar,), daemon=True).start()
    ok = escucha(15002, 180)
    di(evento="servidor", escucha=ok, s=round(time.time() - t, 1), vivo=srv.poll() is None)
    if not ok:
        log.flush()
        print(open("/trabajo/servidor.log").read()[-3000:], flush=True)
        srv.kill()
        return 1

    from pyspark.sql import SparkSession

    t = time.time()
    a = SparkSession.builder.remote("sc://127.0.0.1:15002/;user_id=ana").getOrCreate()
    di(evento="sesion_a", s=round(time.time() - t, 2))
    if os.environ.get("SOLO") == "aislamiento":
        a.range(0, 10).createOrReplaceTempView("t")
        b = SparkSession.builder.remote("sc://127.0.0.1:15002/;user_id=bea").create()
        di(evento="sesiones", a=a.session_id, b=b.session_id)
        try:
            b.sql("select count(*) from t").collect()
            di(evento="aislamiento", ve_la_vista_de_ana=True)
        except Exception as e:  # noqa: BLE001
            di(evento="aislamiento", ve_la_vista_de_ana=False, error=str(e).splitlines()[0][:160])
        # Lo que de verdad aísla a dos personas: ¿ve B una tabla global (la del catálogo de la sesión)?
        a.range(0, 10).write.mode("overwrite").saveAsTable("de_ana")
        try:
            n = b.sql("select count(*) from de_ana").collect()[0][0]
            di(evento="catalogo_compartido", b_lee_la_tabla_de_ana=True, filas=n)
        except Exception as e:  # noqa: BLE001
            di(evento="catalogo_compartido", b_lee_la_tabla_de_ana=False, error=str(e).splitlines()[0][:160])
        # ¿Puede un cliente pedir al servidor que ejecute Java arbitrario? (`reflect`, `java_method`)
        try:
            r = b.sql("select reflect('java.lang.System', 'getProperty', 'user.name') u").collect()[0][0]
            di(evento="reflect", permitido=True, resultado=r)
        except Exception as e:  # noqa: BLE001
            di(evento="reflect", permitido=False, error=str(e).splitlines()[0][:160])
        parar.set()
        srv.terminate()
        return 0
    medir("trivial", lambda: a.sql("select 1 as x").collect(), veces=5)
    medir("rango_contar", lambda: a.range(0, 10_000_000).count(), veces=2)
    medir("agregar_600M", lambda: a.range(0, 600_000_000).selectExpr("id % 100 as g", "(id * 7) % 1000 as f")
          .groupBy("g").agg({"f": "sum", "*": "count"}).count(), veces=3)
    medir("muchos_grupos_600M", lambda: a.range(0, 600_000_000).selectExpr("id % 20000000 as g", "id as f")
          .groupBy("g").agg({"f": "sum"}).orderBy("sum(f)", ascending=False).limit(10).collect(), veces=2)
    # El SQL de Query tal cual: una vista temporal y SQL encima.
    a.range(0, 50_000_000).selectExpr("id", "id % 1000 as k", "rand(1) * 100 as v").createOrReplaceTempView("t")
    medir("sql_sobre_vista", lambda: a.sql("select k, avg(v) m, count(*) n from t group by k order by m desc limit 5").collect(), veces=3)

    # Dos personas a la vez: lo de una no se ve en la otra, y las dos corren.
    # `create()` y no `getOrCreate()`: en el mismo proceso, éste devuelve la sesión de Ana.
    b = SparkSession.builder.remote("sc://127.0.0.1:15002/;user_id=bea").create()
    di(evento="sesiones", a=a.session_id, b=b.session_id)
    try:
        b.sql("select count(*) from t").collect()
        di(evento="aislamiento", ve_la_vista_de_ana=True)
    except Exception as e:  # noqa: BLE001
        di(evento="aislamiento", ve_la_vista_de_ana=False, error=str(e).splitlines()[0][:160])
    r = {}

    def una(nombre, ses):
        t = time.time()
        ses.range(0, 600_000_000).selectExpr("id % 5000000 as g").groupBy("g").count().count()
        r[nombre] = round(time.time() - t, 1)
    h = [threading.Thread(target=una, args=(n, x)) for n, x in (("ana", a), ("bea", b))]
    t = time.time()
    [x.start() for x in h]
    [x.join() for x in h]
    di(consulta="dos_a_la_vez", s=round(time.time() - t, 1), cada_una=r)

    # Ocioso: ¿cuándo se devuelve lo que sobra? (ocioso 60 s, shuffle tracking 120 s)
    di(evento="ocioso_empieza")
    time.sleep(int(os.environ.get("OCIOSO", "240")))
    di(evento="ocioso_acaba")
    medir("agregar_600M_tras_ocioso", lambda: a.range(0, 600_000_000).selectExpr("id % 100 as g").groupBy("g").count().count())
    parar.set()
    srv.terminate()
    try:
        srv.wait(30)
    except subprocess.TimeoutExpired:
        srv.kill()
    log.close()
    errores = [l for l in open("/trabajo/servidor.log") if " ERROR " in l][-8:]
    di(evento="fin", errores_del_servidor=[e.strip()[:240] for e in errores])
    return 0


if __name__ == "__main__":
    c = main()
    sys.stdout.flush()
    os._exit(c or 0)  # los hilos de gRPC del cliente no dejan salir al intérprete (medido: 10 min colgado)
