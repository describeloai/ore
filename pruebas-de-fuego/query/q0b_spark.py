"""Query Q0b · ¿Puede un Spark caliente servir a varias personas, cada sesión con su identidad?

Un servidor Spark Connect (local, en este pod) y un catálogo REST de mentira en 127.0.0.1:8181 que
apunta las cabeceras de cada petición. Dos sesiones (`create()`, no `getOrCreate()`): Ana y Bea.
  1 · ¿Puede una sesión registrar su propio catálogo (`spark.sql.catalog.lago.*`) en tiempo de ejecución?
  2 · ¿Llega a cada petición la cabecera de identidad de su sesión, y sólo la suya?
  3 · ¿Puede Bea poner la cabecera de Ana? (si sí, la identidad la dice el cliente: no vale tal cual)
  4 · Un catálogo puesto en el servidor al arrancar (`lago_srv`, cabecera del servidor): ¿lo puede
      reescribir una sesión?
  5 · ¿Cuánto tarda una sesión nueva y su primera consulta con el servidor caliente?
"""
import json
import os
import socket
import subprocess
import sys
import threading
import time
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

SPARK_HOME = os.environ["SPARK_HOME"]
T0 = time.time()
PETICIONES = []


def di(**k):
    k["t"] = round(time.time() - T0, 1)
    print(json.dumps(k, ensure_ascii=False), flush=True)


class Catalogo(BaseHTTPRequestHandler):
    def _contesta(self):
        PETICIONES.append({"ruta": self.path, "sujeto": self.headers.get("x-ore-sujeto")})
        if self.path.startswith("/v1/config"):
            cuerpo, codigo = {"defaults": {}, "overrides": {}}, 200
        else:
            cuerpo, codigo = {"error": {"message": "no existe", "type": "NoSuchTableException", "code": 404}}, 404
        b = json.dumps(cuerpo).encode()
        self.send_response(codigo)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(b)))
        self.end_headers()
        self.wfile.write(b)

    do_GET = do_POST = do_HEAD = _contesta

    def log_message(self, *a):
        pass


def instalar_cliente():
    """Las ruedas del cliente de Spark Connect, del bucket del banco (como en `connect.py`)."""
    req = urllib.request.Request("http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token",
                                 headers={"Metadata-Flavor": "Google"})
    tok = json.load(urllib.request.urlopen(req, timeout=10))["access_token"]
    d = "/trabajo/ruedas"
    os.makedirs(d, exist_ok=True)
    ruedas = ["grpcio-1.84.1-cp314-cp314-manylinux2014_x86_64.manylinux_2_17_x86_64.whl", "grpcio_status-1.84.1-py3-none-any.whl",
              "zstandard-0.25.0-cp314-cp314-manylinux2014_x86_64.manylinux_2_17_x86_64.manylinux_2_28_x86_64.whl"]
    for r in ruedas:
        q = urllib.request.Request("https://storage.googleapis.com/project-8853a180-450d-47be-b83-banco-geo/query/ruedas/" + r,
                                   headers={"Authorization": "Bearer " + tok})
        with urllib.request.urlopen(q, timeout=60) as resp, open(os.path.join(d, r), "wb") as f:
            f.write(resp.read())
    subprocess.run([sys.executable, "-m", "pip", "install", "-q", "--no-index", "--no-deps", "--target", "/trabajo/py"]
                   + [os.path.join(d, r) for r in ruedas], check=True)
    sys.path.insert(0, "/trabajo/py")
    import importlib
    importlib.invalidate_caches()


def catalogo_conf(nombre, sujeto):
    return {"spark.sql.catalog.%s" % nombre: "org.apache.iceberg.spark.SparkCatalog",
            "spark.sql.catalog.%s.type" % nombre: "rest",
            "spark.sql.catalog.%s.uri" % nombre: "http://127.0.0.1:8181",
            "spark.sql.catalog.%s.header.x-ore-sujeto" % nombre: sujeto}


def leer(ses, tabla):
    try:
        ses.sql("select * from %s" % tabla).collect()
        return "leyó"
    except Exception as e:  # noqa: BLE001
        return str(e).splitlines()[0][:140]


def peticiones_desde(n):
    return [(p["ruta"].split("?")[0], p["sujeto"]) for p in PETICIONES[n:]]


def main():
    srv_http = ThreadingHTTPServer(("127.0.0.1", 8181), Catalogo)
    threading.Thread(target=srv_http.serve_forever, daemon=True).start()
    instalar_cliente()
    args = [os.path.join(SPARK_HOME, "bin", "spark-submit"), "--master", "local[4]",
            "--class", "org.apache.spark.sql.connect.service.SparkConnectServer", "--name", "q0b",
            "--conf", "spark.connect.grpc.binding.port=15002", "--conf", "spark.driver.memory=6g"]
    for k, v in catalogo_conf("lago_srv", "servidor").items():
        args += ["--conf", "%s=%s" % (k, v)]
    args.append("spark-internal")
    srv = subprocess.Popen(args, stdout=open("/trabajo/servidor.log", "w"), stderr=subprocess.STDOUT)
    t = time.time()
    while time.time() - t < 180:
        try:
            socket.create_connection(("127.0.0.1", 15002), timeout=1).close()
            break
        except OSError:
            time.sleep(0.5)
    di(evento="servidor", s=round(time.time() - t, 1))

    from pyspark.sql import SparkSession

    tiempos = []
    for i in range(5):
        t = time.time()
        s = SparkSession.builder.remote("sc://127.0.0.1:15002/;user_id=u%d" % i).create()
        t1 = time.time()
        s.sql("select 1").collect()
        tiempos.append((round((t1 - t) * 1000), round((time.time() - t1) * 1000)))
    di(prueba="5_sesion_nueva", ms_crear_y_primera=tiempos)

    ana = SparkSession.builder.remote("sc://127.0.0.1:15002/;user_id=ana").create()
    bea = SparkSession.builder.remote("sc://127.0.0.1:15002/;user_id=bea").create()
    # 1 · su catálogo, en tiempo de ejecución
    for ses, quien in ((ana, "ana"), (bea, "bea")):
        try:
            for k, v in catalogo_conf("lago", quien).items():
                ses.conf.set(k, v)
            di(prueba="1_registrar_catalogo", sesion=quien, permitido=True)
        except Exception as e:  # noqa: BLE001
            di(prueba="1_registrar_catalogo", sesion=quien, permitido=False, error=str(e).splitlines()[0][:160])
    # 2 · cada petición con la cabecera de su sesión
    n = len(PETICIONES)
    ra = leer(ana, "lago.ns.t")
    pa = peticiones_desde(n)
    n = len(PETICIONES)
    rb = leer(bea, "lago.ns.t")
    pb = peticiones_desde(n)
    di(prueba="2_identidad_por_sesion", ana=ra, peticiones_de_ana=pa, bea=rb, peticiones_de_bea=pb)
    # 3 · Bea se pone la cabecera de Ana (en un catálogo nuevo: el de antes ya está cargado)
    n = len(PETICIONES)
    try:
        for k, v in catalogo_conf("lago2", "ana").items():
            bea.conf.set(k, v)
        r = leer(bea, "lago2.ns.t")
    except Exception as e:  # noqa: BLE001
        r = str(e).splitlines()[0][:140]
    di(prueba="3_suplantar", resultado=r, peticiones=peticiones_desde(n))
    # 4 · el catálogo del servidor: ¿lo reescribe una sesión?
    n = len(PETICIONES)
    try:
        bea.conf.set("spark.sql.catalog.lago_srv.header.x-ore-sujeto", "bea-reescrito")
        cambio = "permitido"
    except Exception as e:  # noqa: BLE001
        cambio = str(e).splitlines()[0][:160]
    r = leer(bea, "lago_srv.ns.t")
    di(prueba="4_reescribir_catalogo_del_servidor", set=cambio, lectura=r, peticiones=peticiones_desde(n),
       modificable=str(bea.conf.isModifiable("spark.sql.catalog.lago_srv.header.x-ore-sujeto")))
    srv.terminate()


if __name__ == "__main__":
    try:
        main()
    finally:
        sys.stdout.flush()
        os._exit(0)
