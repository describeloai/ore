"""ORE 0072 I4·6 · el índice sigue a su tabla, en victor y con la capa abierta: la tabla de I4·0 (30 M viajes de
SpatialBench sf100: una base del 70 % en cuatro ficheros, 24 cargas horarias, una retención y un borrado disperso
copy-on-write) en el lago de victor, y por cada instantánea, en orden:

  · **el Job del índice de la rama** (`ore-piramide`, tarea `indice`, estático) contra el mismo destino —la primera
    construye, las demás siguen y compactan cada 4—: segundos, qué hizo y la profundidad que deja;
  · **la capa abierta**: un hilo que hace de quien mira, contra el motor de la rama (`ore-teselas`, de al lado, en
    localhost): pregunta por el índice de la instantánea de ahora (`/v1/indice`, como el descriptor) y pide una vista
    z14 de NYC entera de una vez con el plazo de producción (10 s). Mide cuánto tarda cada instantánea en servirse
    desde que llega, y cómo van las teselas mientras el Job escribe y compacta.

Al final, el Job reconstruye la última instantánea en otro destino, y DuckDB (`iceberg_scan`, un lector que no es
nuestro y aplica los borrados por posición) cuenta filas y suma `t_tripkey` del índice seguido, del rehecho y de la
tabla: tienen que decir lo mismo. Y una vista z14 y z16 aislada sobre cada uno: lo que cuesta servir lo seguido.

Sin pirámide: lo que se mide es el índice; el momento (I4·5) lo elige el descriptor, que aquí no está.
Emite «I46|…». Lo lanza `correr.py victor` (nodo pesado, imagen de notebooks, DuckDB de `ore-piramide:main` en
/herr y los binarios de la rama desde el banco). Limpia la tabla al acabar.
"""
import json
import os
import shutil
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request

import google.auth
import google.auth.transport.requests
import pyarrow.parquet as pq
from google.cloud import storage

PROYECTO = "project-8853a180-450d-47be-b83"
BANCO = "https://storage.googleapis.com/" + PROYECTO + "-banco-geo"
TRIP = BANCO + "/spatialbench/sf100/trip.parquet"
CUBO = PROYECTO + "-t-victor-copia"
RAIZ = "ore/v2/banco-i4/victor-%d" % int(time.time())
FILAS = int(os.environ.get("I46_FILAS", "30000000"))
CARGAS = int(os.environ.get("I46_CARGAS", "24"))
BINARIOS = os.environ["I46_BINARIOS"]  # gs://…/i4/bin/<marca>/ con ore-piramide y ore-teselas
HERR = "/herr"
T = "/trabajo"
MOTOR = "http://127.0.0.1:8099"
CENTRO = (-73.95, 40.73)


def di(que, **kw):
    print("I46|" + que + "|" + json.dumps(kw, ensure_ascii=False, default=str), flush=True)


_cred, _ = google.auth.default(scopes=["https://www.googleapis.com/auth/cloud-platform"])


def token():
    if not _cred.valid or (_cred.expiry and _cred.expiry.timestamp() - time.time() < 600):
        _cred.refresh(google.auth.transport.requests.Request())
    return _cred.token


gcs = storage.Client(project=PROYECTO)
cubo = gcs.bucket(CUBO)
for d in (T + "/temp", T + "/job", T + "/rehecho"):
    os.makedirs(d, exist_ok=True)


# ── los binarios de la rama ─────────────────────────────────────────────────────────────────────
def traer_binarios():
    b, _, pre = BINARIOS.removeprefix("gs://").partition("/")
    for n in ("ore-piramide", "ore-teselas"):
        f = f"{HERR}/{n}"
        gcs.bucket(b).blob(pre.rstrip("/") + "/" + n).download_to_filename(f)
        os.chmod(f, 0o755)
    assert os.path.exists(HERR + "/duckdb/duckdb"), "falta DuckDB en /herr (el initContainer de ore-piramide:main)"


# ── el motor y quien mira ───────────────────────────────────────────────────────────────────────
def al_motor(ruta, cuerpo, plazo=60):
    pet = urllib.request.Request(MOTOR + ruta, data=json.dumps(cuerpo).encode(), method="POST",
                                 headers={"content-type": "application/json"})
    t0 = time.time()
    try:
        r = urllib.request.urlopen(pet, timeout=plazo)
        b = r.read()
        return r.status, b, r.headers, round((time.time() - t0) * 1000)
    except urllib.error.HTTPError as e:
        return e.code, e.read(), e.headers, round((time.time() - t0) * 1000)
    except Exception as e:  # noqa: BLE001
        return 0, str(e).encode(), {}, round((time.time() - t0) * 1000)


def vista(z, ancho=1600, alto=900):
    import math
    n = 2 ** z
    cx = (CENTRO[0] + 180) / 360 * n
    cy = (1 - math.asinh(math.tan(math.radians(CENTRO[1]))) / math.pi) / 2 * n
    mx, my = ancho / 512 / 2, alto / 512 / 2
    return [(z, x, y) for x in range(math.floor(cx - mx), math.floor(cx + mx) + 1)
            for y in range(math.floor(cy - my), math.floor(cy + my) + 1)]


def tesela(ml_indice, z, x, y, plazo_ms):
    cuerpo = {"protocolo": "1", "tms": "WebMercatorQuad", "z": str(z), "x": str(x), "y": str(y), "formato": "mvt",
              "capa": "i46", "fuente": {"metadata_location": ml_indice, "geometria": "geom", "crs": "EPSG:3857",
                                        "id": "_ore_id"},
              "limites": {"plazo_ms": str(plazo_ms), "max_bytes": "2000000"},
              "credencial": {"gcs.oauth2.token": token()}}
    c, b, h, ms = al_motor("/v1/teselas", cuerpo, plazo_ms / 1000 + 40)
    cuenta = {}
    try:
        cuenta = json.loads(h.get("x-ore-cuenta") or "{}")
    except Exception:  # noqa: BLE001
        pass
    return {"codigo": c, "ms": ms, "kb": round(len(b) / 1024, 1), "grupos_leidos": cuenta.get("grupos_leidos"),
            "motivo": b[:120].decode("utf-8", "replace") if c not in (200, 204) else None}


def pct(v, p):
    v = sorted(v)
    return v[min(len(v) - 1, int(round(p * (len(v) - 1))))] if v else None


def resumen(rs):
    ok = [r for r in rs if r["codigo"] in (200, 204)]
    return {"teselas": len(rs), "ok": len(ok), "p50_ms": pct([r["ms"] for r in ok], 0.5),
            "p95_ms": pct([r["ms"] for r in ok], 0.95), "max_ms": max([r["ms"] for r in ok], default=None),
            "grupos_leidos_p50": pct([r["grupos_leidos"] or 0 for r in ok], 0.5),
            "fallos": sorted({"%s %s" % (r["codigo"], (r["motivo"] or "")[:60]) for r in rs if r not in ok})}


def a_la_vez(ml_indice, ts, plazo_ms):
    rs, hilos = [None] * len(ts), []
    for i, t in enumerate(ts):
        h = threading.Thread(target=lambda i=i, t=t: rs.__setitem__(i, tesela(ml_indice, *t, plazo_ms)))
        h.start()
        hilos.append(h)
    for h in hilos:
        h.join()
    return rs


class Miron(threading.Thread):
    """Quien tiene la capa abierta: el índice de la instantánea de ahora y una vista z14 entera, una y otra vez."""

    def __init__(self, tabla):
        super().__init__(daemon=True)
        self.tabla, self.ml, self.desde, self.paso = tabla, None, None, None
        self.parar = False
        self.servido = {}  # ml → segundos desde que llegó hasta que el motor da su índice
        self.rondas = []  # (paso, generación servida, al día, resumen)
        self.ts = vista(14)

    def llega(self, paso, ml):
        self.paso, self.ml, self.desde = paso, ml, time.time()

    def run(self):
        while not self.parar:
            ml, paso, desde = self.ml, self.paso, self.desde
            if not ml:
                time.sleep(1)
                continue
            c, b, _, ms = al_motor("/v1/indice", {"ubicacion": self.tabla, "metadata_location": ml,
                                                 "credencial": {"gcs.oauth2.token": token()}})
            try:
                r = json.loads(b) if c == 200 else {}
            except Exception:  # noqa: BLE001
                r = {}
            m = r.get("indice") or {}
            if m.get("snapshot_fuente") == ml and ml not in self.servido:
                self.servido[ml] = round(time.time() - desde, 1)
                di("servido", paso=paso, segundos_desde_que_llego=self.servido[ml], generacion=m.get("generacion"))
            if m.get("indice"):
                t0 = time.time()
                rs = a_la_vez(m["indice"], self.ts, 10_000)
                self.rondas.append((paso, m.get("generacion"), m.get("snapshot_fuente") == ml,
                                    round((time.time() - t0) * 1000), resumen(rs), ms))
            time.sleep(2)

    def de(self, paso):
        rs = [r for r in self.rondas if r[0] == paso]
        if not rs:
            return {"rondas": 0}
        ok = sum(r[4]["ok"] for r in rs)
        n = sum(r[4]["teselas"] for r in rs)
        return {"rondas": len(rs), "teselas_ok": f"{ok}/{n}",
                "vista_ms_p50": pct([r[3] for r in rs], 0.5), "vista_ms_max": max(r[3] for r in rs),
                "tesela_p95_ms_max": max((r[4]["p95_ms"] or 0) for r in rs),
                "grupos_leidos_p50": pct([r[4]["grupos_leidos_p50"] or 0 for r in rs], 0.5),
                "indice_ms_p50": pct([r[5] for r in rs], 0.5),
                "generaciones": sorted({str(r[1]) for r in rs}),
                "fallos": sorted({f for r in rs for f in r[4]["fallos"]})}


# ── el Job del índice ───────────────────────────────────────────────────────────────────────────
def job(ml, destino, trabajo):
    cred = {"gcs.oauth2.token": token()}
    pet = {"tarea": "indice", "capa": "i46.viajes", "suelo": 13, "trabajo": trabajo,
           "fuente": {"metadata_location": ml, "geometria": "t_pickuploc", "crs": "EPSG:4326",
                      "columnas": ["t_tripkey", "t_fare"], "credencial": cred},
           "destino": {"ubicacion": destino, "credencial": cred}}
    env = dict(os.environ, ORE_DUCKDB=HERR + "/duckdb/duckdb", ORE_DUCKDB_EXTENSIONES=HERR + "/duckdb/ext")
    t0 = time.time()
    p = subprocess.run([HERR + "/ore-piramide"], input=json.dumps(pet), capture_output=True, text=True, env=env)
    lineas = [l.split("|", 2)[1:] for l in p.stdout.splitlines() if l.startswith("INDICE|")]
    hecho = {k: json.loads(v) for k, v in lineas}
    return p.returncode, round(time.time() - t0, 1), hecho, p.stderr[-1500:]


motor = None
try:
    traer_binarios()
    motor = subprocess.Popen([HERR + "/ore-teselas"], env=dict(os.environ, ORE_TESELAS_PUERTO="8099"),
                             stdout=open(T + "/motor.log", "w"), stderr=subprocess.STDOUT)
    import duckdb  # noqa: E402

    sys.path.insert(0, "/opt/ore")
    import ore  # noqa: E402

    con = ore._duckdb()
    for e in ("spatial", "httpfs", "iceberg"):
        con.execute("load " + e)
    con.execute("set memory_limit = '24GB'")
    con.execute("set temp_directory = '%s/temp'" % T)
    con.execute("set preserve_insertion_order = true")
    con.execute("create or replace secret b (type http, bearer_token '%s', scope '%s/')"
                % (token().replace("'", "''"), BANCO))

    # ── 0 · los datos y la historia de la tabla (como I4·0) ─────────────────────────────────────
    t0 = time.time()
    con.execute(f"""create table v as select t_tripkey, t_pickuptime, t_fare, t_pickuploc
                    from read_parquet('{TRIP}') limit {FILAS}""")
    q = [0.70 * i / 4 for i in (1, 2, 3)] + [0.70 + 0.30 * i / CARGAS for i in range(CARGAS)]
    cortes = con.execute("select quantile_disc(t_pickuptime, %s) from v" % q).fetchone()[0]
    base_cortes, carga_cortes = cortes[:3] + [cortes[3]], cortes[3:] + [None]

    def trozo(desde, hasta, f):
        w = []
        if desde is not None:
            w.append("t_pickuptime >= '%s'" % desde)
        if hasta is not None:
            w.append("t_pickuptime < '%s'" % hasta)
        con.execute("copy (select * from v where %s) to '%s' (format parquet, row_group_size 100000)"
                    % (" and ".join(w) or "true", f))
        return f

    def subir(local, clave):
        cubo.blob(clave).upload_from_filename(local)
        return "gs://%s/%s" % (CUBO, clave)

    lim = [None] + base_cortes
    base = [subir(trozo(lim[i], lim[i + 1], f"{T}/base-{i}.parquet"), f"{RAIZ}/datos/base-{i}.parquet")
            for i in range(4)]
    cargas = [trozo(carga_cortes[i], carga_cortes[i + 1], f"{T}/carga-{i + 1:02d}.parquet") for i in range(CARGAS)]
    subprocess.run([sys.executable, "-m", "pip", "install", "-q", "--target", T + "/py", "sqlalchemy"], check=True)
    sys.path.insert(0, T + "/py")
    from pyiceberg.catalog.sql import SqlCatalog  # noqa: E402
    from pyiceberg.expressions import In  # noqa: E402

    props = {"gcs.oauth2.token": token(), "gcs.project-id": PROYECTO,
             "gcs.oauth2.token-expires-at": str(int((time.time() + 3000) * 1000))}
    cat = SqlCatalog("i46", uri="sqlite:///%s/cat.db" % T, warehouse="gs://%s/%s/tablas" % (CUBO, RAIZ), **props)
    cat.create_namespace_if_not_exists("i4")
    tabla = cat.create_table("i4.viajes", schema=pq.read_schema(T + "/base-0.parquet"))
    pasos = []  # (nombre, metadata_location)
    tabla.add_files(base)
    tabla = cat.load_table("i4.viajes")
    pasos.append(("base", tabla.metadata_location))
    for i, f in enumerate(cargas, 1):
        if i == CARGAS // 2 + 1:
            with tabla.transaction() as tx:
                tx.set_properties({"commit.manifest-merge.enabled": "true",
                                   "commit.manifest.min-count-to-merge": "2"})
            tabla = cat.load_table("i4.viajes")
        if i <= CARGAS // 2:
            tabla.add_files([subir(f, f"{RAIZ}/datos/carga-{i:02d}.parquet")])
        else:
            tabla.append(pq.read_table(f))
        tabla = cat.load_table("i4.viajes")
        pasos.append(("carga-%02d" % i, tabla.metadata_location))
    tabla.delete("t_pickuptime < '%s'" % str(base_cortes[0]).replace(" ", "T"))
    tabla = cat.load_table("i4.viajes")
    pasos.append(("retencion", tabla.metadata_location))
    claves = [r[0] for r in con.execute("select t_tripkey from v where t_tripkey % 1000 = 0").fetchall()]
    tabla.delete(In("t_tripkey", set(claves)))
    tabla = cat.load_table("i4.viajes")
    pasos.append(("borrado-disperso", tabla.metadata_location))
    destino = tabla.location().rstrip("/")
    con.execute("drop table v")
    di("historia", pasos=len(pasos), filas=FILAS, cargas=CARGAS, preparar_s=round(time.time() - t0), tabla=destino)

    # ── 1 · el índice sigue a su tabla, con la capa abierta ──────────────────────────────────────
    for _ in range(120):
        try:
            urllib.request.urlopen(MOTOR + "/salud", timeout=2).read()
            break
        except Exception:  # noqa: BLE001
            time.sleep(0.5)
    miron = Miron(destino)
    miron.start()
    for n, ml in pasos:
        miron.llega(n, ml)
        c, s, hecho, err = job(ml, destino, T + "/job")
        shutil.rmtree(T + "/job", ignore_errors=True)
        os.makedirs(T + "/job", exist_ok=True)
        if c != 0:
            di("falla", paso=n, salida=c, error=err)
            break
        # que quien mira lo vea servido (el motor se fía 10 s de lo que dijo)
        fin = time.time() + 40
        while ml not in miron.servido and time.time() < fin:
            time.sleep(1)
        di("paso", paso=n, job_s=s, hecho=hecho, servido_s=miron.servido.get(ml), capa=miron.de(n))
    miron.parar = True
    miron.join(30)

    # ── 2 · reconstruir la última y cotejar con iceberg_scan ─────────────────────────────────────
    ultimo = pasos[-1][1]
    rehecho = destino + "/_rehecho"
    c, s, hecho, err = job(ultimo, rehecho, T + "/rehecho")
    di("reconstruir", salida=c, job_s=s, hecho=hecho, error=err if c else None)
    stem = ultimo.rsplit("/", 1)[1].removesuffix(".metadata.json")
    con.execute("create or replace secret g (type gcs, bearer_token '%s')" % token().replace("'", "''"))
    lado = {}
    for nombre, d in (("seguido", destino), ("rehecho", rehecho)):
        clave = d.removeprefix("gs://%s/" % CUBO) + f"/_ore_capa/i1/{stem}/manifiesto.json"
        m = json.loads(cubo.blob(clave).download_as_text())
        t1 = time.time()
        r = con.execute("select count(*), sum(t_tripkey)::varchar from iceberg_scan('%s') where _ore_trozo = 0"
                        % m["indice"]).fetchone()
        lado[nombre] = m
        di("leido", lado=nombre, filas=r[0], suma=r[1], scan_s=round(time.time() - t1, 1),
           generacion=m.get("generacion"), profundidad=m.get("profundidad"), filas_manifiesto=m.get("filas"))
    r = con.execute("select count(*), sum(t_tripkey)::varchar from iceberg_scan('%s')" % ultimo).fetchone()
    di("leido", lado="tabla", filas=r[0], suma=r[1])
    # ── 3 · servir lo seguido frente a lo rehecho: vistas aisladas, de una en una ─────────────────
    for nombre, m in lado.items():
        for z in (14, 16):
            rs = [tesela(m["indice"], *t, 60_000) for t in vista(z)]
            di("servir", lado=nombre, z=z, **resumen(rs))
finally:
    if motor:
        motor.terminate()
        try:
            print("## motor (lo último):\n" + open(T + "/motor.log").read()[-3000:], flush=True)
        except Exception:  # noqa: BLE001
            pass
    n = 0
    for b in gcs.list_blobs(CUBO, prefix=RAIZ + "/"):
        b.delete()
        n += 1
    shutil.rmtree(T + "/temp", ignore_errors=True)
    di("limpio", objetos=n, prefijo=RAIZ)
