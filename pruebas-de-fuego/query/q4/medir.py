"""Query Q4·4 y Q5·0 · Lo que Query hace de verdad en victor, por `POST /consultas`, como una persona.

    ORE_TOKEN=<el token de acceso de una persona de victor> python pruebas-de-fuego/query/q4/medir.py [fase ...]

Fases (todas si no se dice ninguna, en este orden):
  escalera     las nueve consultas de Q0 a 6, 60 y 600 M filas (los datasets de `sembrar.py`): la
               clase, la razón, lo que la admisión estimó frente a los `gb` de Q0 (Q2, ±20 %) y el
               tiempo por partes (decidir, resolver, cola, correr, total)
  aislamiento  la sonda de Q0b contra el trabajador de verdad: cada intento tiene que salir negado
  concurrencia 16 L y 2 G a la vez: la espera en cola y el reparto entre trabajadores
  arranque     Q5·0 (con `--consultas-techo 2`): dos G a la vez; la regla pide un trabajador de
               quita y pon, y se mide cuánto tarda en pedir trabajo (nodo frío y, a la segunda,
               caliente) y en volver al suelo

⛔ El token lo pone quien corre la prueba (como `ore-postgres/p46.sh`): no se imprime ni se guarda.
   Dura minutos: con `ORE_TOKEN_FICHERO` se relee en cada petición, y se puede renovar mientras corre.
Una línea JSON por medida en `pruebas-de-fuego/query/resultados/<fecha>-q4.jsonl`.
"""
import concurrent.futures
import json
import os
import statistics
import sys
import time
import urllib.error
import urllib.request

AQUI = os.path.dirname(os.path.abspath(__file__))
URL = os.environ.get("ORE_URL", "https://victor.ore.paladio.io").rstrip("/")
BASE = os.environ.get("BASE_DATOS", "query")
SAL = os.environ.get("ORE_SALIDA") or os.path.join(AQUI, "..", "resultados", time.strftime("%Y%m%d") + "-q4.jsonl")
BANCO = "https://storage.googleapis.com/project-8853a180-450d-47be-b83-banco-geo/"
T0 = time.time()

# Las de `consultas.py` (Q0), con `{T}` = el dataset de la escala y `customer` el del lago.
CONSULTAS = [
    ("contar", "select count(*) from {T}"),
    ("punto", "select * exclude (t_pickuploc, t_dropoffloc) from {T} where t_tripkey = 4242"),
    ("agregar", "select t_custkey % 100 g, count(*) n, sum(t_fare) f, avg(t_distance) d from {T} group by 1 order by 1"),
    ("top_n", "select t_tripkey, t_custkey, t_totalamount, t_pickuptime from {T} order by t_totalamount desc limit 100"),
    ("muchos_grupos", "select t_custkey, sum(t_fare) f from {T} group by 1 order by f desc limit 10"),
    ("distintos", "select count(*) from (select distinct t_driverkey, t_vehiclekey from {T})"),
    ("junta", "select c_nation, count(*) n, sum(t_fare) f from {T} t join {C} c on c.c_custkey = t.t_custkey group by 1 order by 1"),
    ("ventana", "select count(*) from (select t_custkey, t_fare - lag(t_fare) over (partition by t_custkey order by t_pickuptime) d from {T}) where d > 0"),
    # En el lago la columna ya es geometría (`sembrar.py`): sin `ST_GeomFromWKB`, que es lo que Q0 hacía con el WKB crudo.
    ("geo_caja", "select count(*) from {T} where ST_Intersects(t_pickuploc, ST_MakeEnvelope(-74.05, 40.6, -73.9, 40.8))"),
]


def token():
    f = os.environ.get("ORE_TOKEN_FICHERO")
    if f:
        return open(f, encoding="utf-8").read().strip()
    t = os.environ.get("ORE_TOKEN")
    if not t:
        sys.exit("✗ falta ORE_TOKEN (o ORE_TOKEN_FICHERO): el token de acceso de una persona de victor")
    return t


def di(**k):
    k["t"] = round(time.time() - T0, 1)
    linea = json.dumps(k, ensure_ascii=False)
    print(linea, flush=True)
    with open(SAL, "a", encoding="utf-8") as f:
        f.write(linea + "\n")


def pedir(metodo, camino, cuerpo=None, cabeceras=None, plazo=90):
    h = {"authorization": "Bearer " + token(), "content-type": "application/json"}
    h.update(cabeceras or {})
    datos = json.dumps(cuerpo).encode() if cuerpo is not None else None
    req = urllib.request.Request(URL + camino, data=datos, headers=h, method=metodo)
    try:
        with urllib.request.urlopen(req, timeout=plazo) as r:
            return r.status, json.loads(r.read() or b"{}")
    except urllib.error.HTTPError as e:
        try:
            cuerpo = json.loads(e.read() or b"{}")
        except ValueError:
            cuerpo = {}
        if e.code == 401:
            sys.exit("✗ 401: el token caducó o no vale (renuévalo en ORE_TOKEN_FICHERO)")
        return e.code, cuerpo


def consulta(texto, limite=100):
    """Una consulta de punta a punta, como la consola: `POST /consultas` y, si tarda, `GET` hasta que acaba."""
    t0 = time.time()
    c, r = pedir("POST", "/consultas", {"texto": texto, "limite": str(limite)})
    while c == 202 and r.get("id"):
        c, r = pedir("GET", "/consultas/" + r["id"], cabeceras={"x-ore-espera": "25"})
    r["_cliente_s"] = round(time.time() - t0, 2)
    r["_http"] = c
    return r


def resumen(r):
    a, lee = r.get("admision") or {}, r.get("lee") or {}
    filas = r.get("filas")
    if isinstance(filas, str):
        try:
            filas = json.loads(filas)
        except ValueError:
            pass
    n = len(filas.get("filas", [])) if isinstance(filas, dict) else None
    return {"http": r["_http"], "estado": r.get("estado"), "clase": a.get("clase"), "razon": a.get("razon"),
            "avisos": a.get("avisos"), "lee_gb": round(int(lee.get("bytes") or 0) / 1e9, 3) if lee else None,
            "lee_filas": lee.get("filas"), "estimada": lee.get("estimada"), "ms": r.get("ms"),
            "trabajador": r.get("trabajador"), "filas_devueltas": n, "truncada": r.get("truncada"),
            "codigo": r.get("codigo"), "error": (r.get("error") or "")[:200] or None, "cliente_s": r["_cliente_s"]}


def gb_de_q0():
    """Los `gb_estimados` de Q0 (`grande`), por consulta y escala: la vara de la estimación (Q2)."""
    gb = {}
    try:
        for l in open(os.path.join(AQUI, "..", "resultados", "20261023-grande.jsonl"), encoding="utf-8"):
            d = json.loads(l)
            if d.get("gb_estimados") is not None and d.get("consulta"):
                gb[(d["consulta"], d.get("millones"))] = d["gb_estimados"]
    except OSError:
        pass
    return gb


def escalera():
    q0 = gb_de_q0()
    for m in (6, 60, 600):
        for nombre, sql in CONSULTAS:
            texto = sql.format(T="%s.trip_%dm" % (BASE, m), C="%s.customer" % BASE)
            for vez in ((1, 2) if m <= 60 else (1,)):
                d = resumen(consulta(texto))
                g = q0.get((nombre, m))
                if g and d["lee_gb"] is not None:
                    d["gb_q0"], d["estimacion_frente_a_q0"] = g, round(d["lee_gb"] / g, 2) if g else None
                di(fase="escalera", consulta=nombre, millones=m, vez=vez, **d)


# (intento, consulta, lo que tiene que pasar: el código o el http que lo niega)
SONDA = [
    ("fichero_local", "select * from read_text('/etc/passwd')", "consulta/sql"),
    ("entorno_del_proceso", "select * from read_text('/proc/self/environ')", "consulta/sql"),
    ("token_del_pod", "select * from read_text('/var/run/ore/pod/token')", "consulta/sql"),
    ("metadatos_de_google", "select * from read_text('http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token')", "consulta/sql"),
    ("otro_bucket", "select count(*) from read_parquet('%sspatialbench/sf100/customer.parquet')" % BANCO, "consulta/sql"),
    ("saltar_con_puntos", "select count(*) from read_parquet('%svectorial/../spatialbench/sf100/customer.parquet')" % BANCO, 422),
    ("saltar_codificado", "select count(*) from read_parquet('%svectorial/%%2e%%2e/x.parquet')" % BANCO, 422),
    ("subir_la_memoria", "set memory_limit = '40GB'", "consulta/sin-resultado"),
    ("dos_sentencias", "select 1; select 2", "consulta/sin-resultado"),
    ("escribir", "copy (select 1) to '/tmp/fuga.csv'", "consulta/sin-resultado"),
]


def aislamiento():
    for intento, texto, debe in SONDA:
        d = resumen(consulta(texto))
        negado = d["http"] == debe if isinstance(debe, int) else (d["codigo"] == debe or (d["http"] in (409, 422) and d["estado"] != "hecha"))
        di(fase="aislamiento", intento=intento, negado=bool(negado) and d["estado"] != "hecha", **d)
    # Los secretos de la instancia: sólo los de SUS datasets, y sin el token a la vista.
    texto = "select name, scope, secret_string from duckdb_secrets(), (select count(*) from %s.customer)" % BASE
    r = consulta(texto)
    d = resumen(r)
    crudo = json.dumps(r.get("filas") or "")
    di(fase="aislamiento", intento="ver_secretos", sin_token=("ya29." not in crudo and "bearer_token=" not in crudo.lower()),
       muestra=crudo[:300], **d)


def capacidad():
    c, r = pedir("GET", "/consultas/capacidad")
    return r if c == 200 else {"_http": c}


def concurrencia():
    ls = ["select t_custkey %% 100 g, count(*) n from %s.trip_6m where t_tripkey %% 16 = %d group by 1" % (BASE, i) for i in range(16)]
    gs = ["select count(*) from (select t_custkey, t_fare - lag(t_fare) over (partition by t_custkey order by t_pickuptime) d from %s.trip_600m) where d > 0" % BASE,
          "select t_custkey, sum(t_fare) f from %s.trip_600m group by 1 order by f desc limit 10" % BASE]
    t0 = time.time()
    with concurrent.futures.ThreadPoolExecutor(18) as ex:
        futuros = [ex.submit(consulta, q) for q in gs + ls]
        fotos = []
        while not all(f.done() for f in futuros):
            fotos.append(capacidad())
            time.sleep(2)
        rs = [resumen(f.result()) for f in futuros]
    for i, d in enumerate(rs):
        di(fase="concurrencia", n=i, **d)
    cola = sorted((d["ms"] or {}).get("cola", 0) / 1000 for d in rs)
    di(fase="concurrencia", resumen=True, total_s=round(time.time() - t0, 1), consultas=len(rs),
       bien=sum(d["estado"] == "hecha" for d in rs), cola_p50_s=round(statistics.median(cola), 2),
       cola_p95_s=round(cola[int(len(cola) * 0.95) - 1], 2),
       trabajadores_max=max((f.get("trabajadores") or 0) for f in fotos) if fotos else None)


def arranque():
    """Q5·0: con el suelo ocupado por una G, la regla pide uno de quita y pon. Cuánto tarda en pedir
    trabajo (frío; a la segunda, caliente) y en irse y salir de la cola."""
    cap = capacidad()
    if (cap.get("techo") or 0) < 2:
        di(fase="arranque", saltada=True, motivo="el techo de la celda es %s: hace falta `--consultas-techo 2`" % cap.get("techo"))
        return
    g = "select count(*) from (select t_custkey, t_fare - lag(t_fare) over (partition by t_custkey order by t_pickuptime) d from %s.trip_600m) where d > 0" % BASE
    for vuelta in ("frio", "caliente"):
        t0 = time.time()
        with concurrent.futures.ThreadPoolExecutor(2) as ex:
            futuros = [ex.submit(consulta, g) for _ in range(2)]
            t_pedido = t_llego = None
            while not all(f.done() for f in futuros):
                cap = capacidad()
                ps = cap.get("pedidos") or []
                if ps and t_pedido is None:
                    t_pedido = time.time() - t0
                if any(p.get("llego") for p in ps) and t_llego is None:
                    t_llego = time.time() - t0
                time.sleep(2)
            rs = [resumen(f.result()) for f in futuros]
        di(fase="arranque", vuelta=vuelta, pedido_s=round(t_pedido, 1) if t_pedido else None,
           pide_trabajo_s=round(t_llego, 1) if t_llego else None,
           consultas=[{"estado": d["estado"], "ms": d["ms"]} for d in rs], decision=cap.get("decision"))
        # Hasta que el de quita y pon se va (ocioso, 300 s) y su fichero sale de la cola.
        t1 = time.time()
        while (capacidad().get("pedidos") or []) and time.time() - t1 < 900:
            time.sleep(10)
        di(fase="arranque", vuelta=vuelta, vuelve_al_suelo_s=round(time.time() - t1, 1),
           quedan=len(capacidad().get("pedidos") or []))


def main():
    fases = [a for a in sys.argv[1:] if not a.startswith("-")] or ["escalera", "aislamiento", "concurrencia", "arranque"]
    os.makedirs(os.path.dirname(SAL), exist_ok=True)
    token()
    di(fase="inicio", url=URL, base=BASE, fases=fases, capacidad=capacidad())
    for f in fases:
        {"escalera": escalera, "aislamiento": aislamiento, "concurrencia": concurrencia, "arranque": arranque}[f]()
    di(fase="fin", capacidad=capacidad())


if __name__ == "__main__":
    main()
