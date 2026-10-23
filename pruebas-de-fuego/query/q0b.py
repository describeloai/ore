"""Query Q0b · ¿Puede un trabajador DuckDB caliente servir a varias personas a la vez?

Corre en un pod del pool `pesado` (8 CPU / 48 GiB), como sería un trabajador de `ore-consulta`. Una
instancia de DuckDB por consulta (`duckdb.connect()` en memoria: su propia base, sus secretos, su tope).
Mide, con una línea JSON por medida:
  A · lo que cuesta una instancia (crearla, configurarla, cargar extensiones, ponerle un secreto) y lo
      que ocupa ociosa;
  B · muchas consultas L a la vez (32 hilos, una instancia por consulta): p50/p95 y por segundo;
  C · cuatro consultas G a la vez con 8 GB de tope cada una: ¿se cumple la suma de los topes?;
  D · el aislamiento: con `enable_external_access=false`, las rutas permitidas y la configuración
      cerrada, qué puede y qué no puede hacer una consulta;
  E · el suelo de latencia: de «llega la consulta» a la primera fila, por partes.
"""
import json
import os
import statistics
import threading
import time
import urllib.request
from concurrent.futures import ThreadPoolExecutor

import duckdb

BUCKET = "project-8853a180-450d-47be-b83-banco-geo"
B = "https://storage.googleapis.com/%s/" % BUCKET
TRIP = B + "spatialbench/sf100/trip.parquet"
LUGARES = B + "vectorial/overture/2026-09-23.1/lugares_es.parquet"
EDIFICIOS = B + "g22/a_tal_cual.parquet"
EXT = os.environ.get("EXT", "/opt/ore/duckdb")
T0 = time.time()


def di(**k):
    k["t"] = round(time.time() - T0, 1)
    print(json.dumps(k, ensure_ascii=False), flush=True)


def token():
    if os.environ.get("TOKEN"):  # la prueba del guion en local
        return os.environ["TOKEN"]
    req = urllib.request.Request("http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token",
                                 headers={"Metadata-Flavor": "Google"})
    return json.load(urllib.request.urlopen(req, timeout=10))["access_token"]


TOK = token()


def pico_mb():
    try:
        return int(open("/sys/fs/cgroup/memory.peak").read()) // 2**20
    except OSError:
        return None


def actual_mb():
    try:
        return int(open("/sys/fs/cgroup/memory.current").read()) // 2**20
    except OSError:
        return None


def instancia(memoria="2GB", hilos=2, permitidas=None, cerrar=True, derrame=None, secreto=True):
    """Una consulta, una base: lo que haría `ore-consulta` al admitir. `permitidas`: los prefijos que la
    consulta puede leer (sus datasets); con ellas, nada de fuera."""
    t = time.time()
    c = duckdb.connect(config={"extension_directory": EXT, "autoinstall_known_extensions": "false"})
    c.execute("set memory_limit='%s'; set threads=%d; set TimeZone='UTC'" % (memoria, hilos))
    if derrame:
        os.makedirs(derrame, exist_ok=True)
        c.execute("set temp_directory='%s'; set max_temp_directory_size='40GB'" % derrame)
    c.execute("load httpfs; load spatial")
    t_ext = time.time()
    if secreto:
        c.execute("create secret s (type http, bearer_token '%s', scope '%s')" % (TOK, B))
    if permitidas is not None:
        dirs = list(permitidas) + ([derrame] if derrame else [])
        c.execute("set allowed_directories=%s" % json.dumps(dirs).replace('"', "'"))
        c.execute("set enable_external_access=false")
        if cerrar:
            c.execute("set lock_configuration=true")
    return c, t_ext - t, time.time() - t


def medir_a():
    antes = actual_mb()
    tiempos, ext = [], []
    vivas = []
    for _ in range(50):
        c, te, tt = instancia(permitidas=[B + "vectorial/"])
        ext.append(te)
        tiempos.append(tt)
        vivas.append(c)
    despues = actual_mb()
    di(medida="A_instancia", n=50, crear_y_cerrar_ms_p50=round(statistics.median(tiempos) * 1000, 1),
       ms_p95=round(sorted(tiempos)[47] * 1000, 1), extensiones_ms_p50=round(statistics.median(ext) * 1000, 1),
       ociosa_mb=round((despues - antes) / 50, 1) if antes and despues else None)
    for c in vivas:
        c.close()


def una_l(i):
    sqls = ["select confidence > 0.8, count(*) from read_parquet('%s') group by 1" % LUGARES,
            "select count(*) from read_parquet('%s') where bbox.xmin > -3.8 and bbox.xmax < -3.6 and bbox.ymin > 40.3 and bbox.ymax < 40.5" % EDIFICIOS,
            "select t_custkey %% 100, sum(t_fare) from read_parquet('%s') where t_tripkey <= 6000000 group by 1" % TRIP]
    t = time.time()
    c, _, _ = instancia(permitidas=[B])
    c.execute(sqls[i % 3]).fetchall()
    c.close()
    return time.time() - t


def medir_b():
    for hilos in (8, 32):
        t = time.time()
        with ThreadPoolExecutor(hilos) as ex:
            lat = sorted(ex.map(una_l, range(hilos * 4)))
        total = time.time() - t
        di(medida="B_muchas_L", a_la_vez=hilos, consultas=len(lat), p50_s=round(statistics.median(lat), 2),
           p95_s=round(lat[int(len(lat) * 0.95) - 1], 2), por_segundo=round(len(lat) / total, 1), pico_mb=pico_mb())


def una_g(i):
    sqls = ["select t_custkey, sum(t_fare) f from read_parquet('%s') group by 1 order by f desc limit 10" % TRIP,
            "select count(*) from (select t_fare - lag(t_fare) over (partition by t_custkey order by t_pickuptime) d from read_parquet('%s')) where d > 0" % TRIP]
    t = time.time()
    c, _, _ = instancia(memoria="8GB", hilos=2, permitidas=[B], derrame=os.environ.get("TRABAJO", "/trabajo") + "/derrame-%d" % i)
    try:
        c.execute(sqls[i % 2]).fetchall()
        e = None
    except Exception as x:  # noqa: BLE001
        e = str(x).splitlines()[0][:160]
    c.close()
    return round(time.time() - t, 1), e


def medir_c():
    base = actual_mb()
    pico = [0]
    parar = threading.Event()

    def vigia():
        while not parar.is_set():
            pico[0] = max(pico[0], actual_mb() or 0)
            time.sleep(0.5)
    threading.Thread(target=vigia, daemon=True).start()
    t = time.time()
    with ThreadPoolExecutor(4) as ex:
        r = list(ex.map(una_g, range(4)))
    parar.set()
    di(medida="C_cuatro_G_con_8GB", s_total=round(time.time() - t, 1), cada_una=r,
       suma_de_topes_mb=4 * 8 * 1024, pico_del_pod_mb=pico[0], base_mb=base)


def probar(c, nombre, sql):
    try:
        r = c.execute(sql).fetchall()
        di(medida="D_aislamiento", intento=nombre, permitido=True, resultado=str(r)[:120])
    except Exception as e:  # noqa: BLE001
        di(medida="D_aislamiento", intento=nombre, permitido=False, error=str(e).splitlines()[0][:160])


def medir_d():
    # A es otra persona: su secreto y su base, en el mismo proceso.
    a, _, _ = instancia(permitidas=[B + "spatialbench/"])
    a.execute("create table de_ana as select 42 x")
    # B sólo puede leer `vectorial/` (sus datasets) y su derrame.
    b, _, _ = instancia(permitidas=[B + "vectorial/"], derrame=os.environ.get("TRABAJO", "/trabajo") + "/derrame-b")
    probar(b, "leer_lo_suyo", "select count(*) from read_parquet('%s')" % LUGARES)
    probar(b, "leer_otro_prefijo_del_bucket", "select count(*) from read_parquet('%s')" % TRIP)
    probar(b, "saltar_con_puntos", "select count(*) from read_parquet('%svectorial/../spatialbench/sf100/customer.parquet')" % B)
    probar(b, "saltar_con_puntos_codificados", "select count(*) from read_parquet('%svectorial/%%2e%%2e/spatialbench/sf100/customer.parquet')" % B)
    probar(b, "prefijo_parecido", "select * from read_text('%svectorialX/nada')" % B)
    probar(b, "leer_otro_bucket_por_url", "select * from read_text('https://storage.googleapis.com/otro/x')")
    probar(b, "fichero_local_passwd", "select * from read_text('/etc/passwd')")
    probar(b, "entorno_del_proceso", "select * from read_text('/proc/self/environ')")
    probar(b, "token_de_kubernetes", "select * from read_text('/var/run/secrets/kubernetes.io/serviceaccount/token')")
    probar(b, "derrame_de_otro", "select * from glob('%s/derrame-*/*'" % os.environ.get("TRABAJO", "/trabajo") + ")")
    probar(b, "metadatos_de_google_crear_secreto",
           "create secret m (type http, extra_http_headers map {'Metadata-Flavor': 'Google'})")
    probar(b, "metadatos_de_google_leer",
           "select * from read_text('http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token')")
    probar(b, "ver_secretos", "select name, scope from duckdb_secrets()")
    probar(b, "ver_la_tabla_de_ana", "select * from de_ana")
    probar(b, "reabrir_acceso", "set enable_external_access=true")
    probar(b, "ampliar_rutas", "set allowed_directories=['/']")
    probar(b, "subir_memoria", "set memory_limit='40GB'")
    probar(b, "cargar_extension", "load h3")
    probar(b, "instalar_extension", "install postgres")
    probar(b, "adjuntar_fichero", "attach '/tmp/x.db' as x")
    probar(b, "escribir_fuera", "copy (select 1) to '/tmp/fuga.csv'")
    probar(b, "escribir_en_el_bucket", "copy (select 1) to '%svectorial/fuga.csv'" % B)
    probar(b, "shell_o_sistema", "select * from read_csv('/bin/sh')")
    a.close()
    b.close()


def medir_e():
    # El suelo: un trabajador caliente, la consulta llega con su credencial y sus rutas.
    for vez in range(3):
        t0 = time.time()
        c, t_ext, t_inst = instancia(permitidas=[B + "vectorial/"])
        t1 = time.time()
        c.execute("create view lugares as select * from read_parquet('%s')" % LUGARES)
        t2 = time.time()
        r = c.execute("select count(*) from lugares where confidence > 0.9").fetchone()
        t3 = time.time()
        c.close()
        di(medida="E_suelo", vez=vez + 1, instancia_ms=round((t1 - t0) * 1000, 1), vista_ms=round((t2 - t1) * 1000, 1),
           primera_fila_ms=round((t3 - t2) * 1000, 1), total_ms=round((t3 - t0) * 1000, 1), filas=r[0])


if __name__ == "__main__":
    di(medida="inicio", duckdb=duckdb.__version__, cpus=os.cpu_count(), pod_mb=actual_mb())
    solo = os.environ.get("SOLO")
    for f in (medir_a, medir_e, medir_d, medir_b, medir_c):
        if solo and f.__name__ not in solo.split(","):
            continue
        try:
            f()
        except Exception as e:  # noqa: BLE001
            di(medida=f.__name__, error=str(e)[:300])
    di(medida="fin")
