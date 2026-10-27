"""ORE 0072 I4·0 · el previo de «el índice sigue a su tabla»: una tabla de prueba en el lago de victor, hecha de 30 M viajes de
SpatialBench sf100 (una base del 70 % en cuatro ficheros por tiempo y 24 cargas horarias del 1,25 %; las 12 primeras
con `add_files`, las 12 últimas con `append` y los manifiestos fusionándose), una retención (el cuarto más viejo de la
base) y un borrado disperso (una fila de cada mil). Mide:

  1. la diferencia entre instantáneas en los metadatos: lo añadido y lo retirado de cada paso, por las entradas de los
     manifiestos, frente a la verdad (los ficheros vivos antes y después); y de S0 a S24 de una vez, por la cadena;
  2. seguir frente a reconstruir: el índice de la base, luego sólo cada carga, y al final el índice entero de S24;
  3. la profundidad tras cada carga (la vara de I2: 2 000 puntos, los grupos que los cubren);
  4. quitar filas: las posiciones en el índice de lo que la retención retira, y escribir sus borrados.

Emite «I40|…». Lo lanza `correr.py` (nodo pesado, la imagen de notebooks: trae PyIceberg). Limpia la tabla al acabar.
"""
import json
import os
import shutil
import sys
import time

import duckdb
import google.auth
import google.auth.transport.requests
import pyarrow as pa
import pyarrow.parquet as pq
from google.cloud import storage

PROYECTO = "project-8853a180-450d-47be-b83"
BANCO = "https://storage.googleapis.com/" + PROYECTO + "-banco-geo"
TRIP = BANCO + "/spatialbench/sf100/trip.parquet"
# la tabla de prueba, en el lago de victor (la cuenta del Job escribe ahí, no en el banco); se borra al acabar
CUBO = PROYECTO + "-t-victor-copia"
HTTPS = "https://storage.googleapis.com/" + CUBO
RAIZ = "ore/v2/banco-i4/previo-%d" % int(time.time())
FILAS = int(os.environ.get("I40_FILAS", "30000000"))
CARGAS = int(os.environ.get("I40_CARGAS", "24"))
T = "/trabajo"
IDX = T + "/indice"


def di(que, **kw):
    print("I40|" + que + "|" + json.dumps(kw, ensure_ascii=False, default=str), flush=True)


def token():
    c, _ = google.auth.default(scopes=["https://www.googleapis.com/auth/cloud-platform"])
    c.refresh(google.auth.transport.requests.Request())
    return c.token


tok = token()
gcs = storage.Client(project=PROYECTO)
cubo = gcs.bucket(CUBO)
os.makedirs(T + "/temp", exist_ok=True)
os.makedirs(IDX, exist_ok=True)
try:  # las extensiones del puesto, sin red (como I0); si no, de la red
    sys.path.insert(0, "/opt/ore")
    import ore  # noqa: E402

    con = ore._duckdb()
except Exception:  # noqa: BLE001
    con = duckdb.connect()
    for e in ("spatial", "httpfs"):
        con.execute("install " + e)
for e in ("spatial", "httpfs"):
    con.execute("load " + e)
con.execute("set geometry_always_xy = true")
con.execute("set memory_limit = '36GB'")
con.execute("set threads = 8")
con.execute("set temp_directory = '%s/temp'" % T)
con.execute("set preserve_insertion_order = true")
for i, u in enumerate((BANCO, HTTPS)):
    con.execute("create or replace secret s%d (type http, bearer_token '%s', scope '%s/')" % (i, tok.replace("'", "''"), u))


def subir(local, clave):
    cubo.blob(clave).upload_from_filename(local)
    return "gs://%s/%s" % (CUBO, clave)


def https(uri):
    return uri.replace("gs://%s" % CUBO, HTTPS)


try:
    # ── 0 · los datos: 30 M viajes, partidos por tiempo ───────────────────────────────────────────
    t0 = time.time()
    con.execute(f"""create table v as select t_tripkey, t_pickuptime, t_fare, t_pickuploc
                    from read_parquet('{TRIP}') limit {FILAS}""")
    q = [0.70 * i / 4 for i in (1, 2, 3)] + [0.70 + 0.30 * i / CARGAS for i in range(CARGAS)]
    cortes = con.execute("select quantile_disc(t_pickuptime, %s) from v" % q).fetchone()[0]
    base_cortes, carga_cortes = cortes[:3] + [cortes[3]], cortes[3:] + [None]
    di("datos", filas=FILAS, leer_s=round(time.time() - t0, 1), desde=str(cortes[0]), cargas=CARGAS)

    def trozo(desde, hasta, f):
        w = []
        if desde is not None:
            w.append("t_pickuptime >= '%s'" % desde)
        if hasta is not None:
            w.append("t_pickuptime < '%s'" % hasta)
        con.execute("copy (select * from v where %s) to '%s' (format parquet, row_group_size 100000)"
                    % (" and ".join(w) or "true", f))
        return f

    base = []
    lim = [None] + base_cortes
    for i in range(4):
        base.append(subir(trozo(lim[i], lim[i + 1], f"{T}/base-{i}.parquet"), f"{RAIZ}/datos/base-{i}.parquet"))
    cargas_locales = [trozo(carga_cortes[i], carga_cortes[i + 1], f"{T}/carga-{i + 1:02d}.parquet")
                      for i in range(CARGAS)]

    # ── 1 · la tabla y su historia (PyIceberg, catálogo de un fichero) ───────────────────────────
    # el catálogo SQL de PyIceberg pide `sqlalchemy`, que la imagen no trae: en el disco del Job
    import subprocess
    subprocess.run([sys.executable, "-m", "pip", "install", "-q", "--target", T + "/py", "sqlalchemy"], check=True)
    sys.path.insert(0, T + "/py")
    from pyiceberg.catalog.sql import SqlCatalog
    props = {"gcs.oauth2.token": tok, "gcs.project-id": PROYECTO,
             "gcs.oauth2.token-expires-at": str(int((time.time() + 3000) * 1000))}
    cat = SqlCatalog("i40", uri="sqlite:///%s/cat.db" % T, warehouse="gs://%s/%s/tablas" % (CUBO, RAIZ), **props)
    cat.create_namespace_if_not_exists("i4")
    esquema = pq.read_schema(T + "/base-0.parquet")
    tabla = cat.create_table("i4.viajes", schema=esquema)
    pasos = []  # (nombre, snapshot_id)
    tabla.add_files(base)
    tabla = cat.load_table("i4.viajes")
    pasos.append(("base", tabla.current_snapshot().snapshot_id))
    for i, f in enumerate(cargas_locales, 1):
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
        pasos.append(("carga-%02d" % i, tabla.current_snapshot().snapshot_id))
    tabla.delete("t_pickuptime < '%s'" % str(base_cortes[0]).replace(" ", "T"))
    tabla = cat.load_table("i4.viajes")
    pasos.append(("retencion", tabla.current_snapshot().snapshot_id))
    from pyiceberg.expressions import In
    claves = [r[0] for r in con.execute("select t_tripkey from v where t_tripkey % 1000 = 0").fetchall()]
    tabla.delete(In("t_tripkey", set(claves)))
    tabla = cat.load_table("i4.viajes")
    pasos.append(("borrado-disperso", tabla.current_snapshot().snapshot_id))
    di("historia", pasos=len(pasos), manifiestos_al_final=len(tabla.current_snapshot().manifests(tabla.io)))

    # ── 2 · la diferencia de cada paso, por las entradas, frente a la verdad ─────────────────────
    def entradas(sid):
        e = tabla.inspect.entries(snapshot_id=sid).to_pylist()
        return [(x["status"], x["snapshot_id"], x["data_file"]["file_path"], x["data_file"]["content"],
                 x["data_file"]["record_count"]) for x in e]

    def vivos(sid):
        return {p for st, _, p, c, _ in entradas(sid) if st in (0, 1) and c == 0}

    def diferencia(sid):
        """Lo que dicen las entradas que escribió `sid`: añadidos (1) y retirados (2) por él."""
        e = entradas(sid)
        return ({p for st, s, p, c, _ in e if st == 1 and s == sid and c == 0},
                {p for st, s, p, c, _ in e if st == 2 and s == sid and c == 0},
                sum(1 for st, s, p, c, _ in e if s == sid and c == 1))

    fallos = 0
    for (n0, s0), (n1, s1) in zip(pasos, pasos[1:]):
        a, r, borrados = diferencia(s1)
        v0, v1 = vivos(s0), vivos(s1)
        ok = (a == v1 - v0) and (r == v0 - v1)
        fallos += not ok
        di("diferencia", paso=n1, ok=ok, anadidos=len(a), retirados=len(r), borrados_por_posicion=borrados,
           vivos=len(v1), manifiestos=len(tabla.snapshot_by_id(s1).manifests(tabla.io)))
    # de S0 a S24 de una vez, por la cadena de padres
    padres, s = [], pasos[CARGAS][1]
    while s and s != pasos[0][1]:
        padres.append(s)
        s = tabla.snapshot_by_id(s).parent_snapshot_id
    a_ = set().union(*(diferencia(x)[0] for x in padres))
    r_ = set().union(*(diferencia(x)[1] for x in padres))
    v0, v24 = vivos(pasos[0][1]), vivos(pasos[CARGAS][1])
    di("diferencia_de_una_vez", desde="base", hasta=pasos[CARGAS][0], instantaneas=len(padres),
       ok=(a_ - r_ == v24 - v0) and (r_ - a_ == v0 - v24), anadidos=len(a_ - r_), fallos_paso_a_paso=fallos)

    # ── 3 · seguir frente a reconstruir, y la profundidad ────────────────────────────────────────
    ordinal = {}

    def ordenes(ficheros):
        for f in ficheros:
            ordinal.setdefault(https(f), len(ordinal))
        con.execute("create or replace temp table ords as select * from (values %s) t(ruta, ordinal)"
                    % ", ".join("('%s', %d)" % (https(f), ordinal[https(f)]) for f in ficheros))

    ext = con.execute("""select min(ST_XMin(g)), min(ST_YMin(g)), max(ST_XMax(g)), max(ST_YMax(g)) from (
        select ST_Transform(ST_GeomFromWKB(t_pickuploc), 'EPSG:4326', 'EPSG:3857') g from v
        where t_pickuptime < '%s')""" % base_cortes[-1]).fetchone()
    m = lambda a, b: (a - (b - a) * 0.1, b + (b - a) * 0.1)  # noqa: E731
    (x0, x1), (y0, y1) = m(ext[0], ext[2]), m(ext[1], ext[3])
    clave = (f"ST_Hilbert(ST_Point(greatest(least(ST_X(g), {x1}), {x0}), greatest(least(ST_Y(g), {y1}), {y0})), "
             f"ST_MakeBox2D(ST_Point({x0}, {y0}), ST_Point({x1}, {y1})))")

    def indexar(ficheros, salida):
        ordenes(ficheros)
        lista = ", ".join("'%s'" % https(f) for f in ficheros)
        t = time.time()
        con.execute(f"""copy (
            select o.ordinal::INTEGER _ore_fichero, s.file_row_number::BIGINT _ore_fila, ST_AsWKB(g) geom,
              {{'xmin': ST_XMin(g), 'ymin': ST_YMin(g), 'xmax': ST_XMax(g), 'ymax': ST_YMax(g)}} geom_bbox,
              s.t_pickuptime
            from (select *, ST_Transform(ST_GeomFromWKB(t_pickuploc), 'EPSG:4326', 'EPSG:3857') g
                  from read_parquet([{lista}], filename = true, file_row_number = true)) s
            join ords o on o.ruta = s.filename
            order by {clave}, o.ordinal, s.file_row_number
          ) to '{salida}' (format parquet, compression zstd, row_group_size 100000)""")
        return time.time() - t

    def profundidad(ficheros):
        l = ", ".join("'%s'" % f for f in ficheros)
        return con.execute(f"""
          with grupos as (
            select file_name, row_group_id,
              min(try_cast(stats_min_value as double)) filter (where path_in_schema = 'geom_bbox, xmin') xmin,
              min(try_cast(stats_min_value as double)) filter (where path_in_schema = 'geom_bbox, ymin') ymin,
              max(try_cast(stats_max_value as double)) filter (where path_in_schema = 'geom_bbox, xmax') xmax,
              max(try_cast(stats_max_value as double)) filter (where path_in_schema = 'geom_bbox, ymax') ymax
            from parquet_metadata([{l}]) group by all),
          puntos as (
            select row_number() over () n, (geom_bbox.xmin + geom_bbox.xmax) / 2 x, (geom_bbox.ymin + geom_bbox.ymax) / 2 y
            from read_parquet([{l}]) using sample reservoir(2000 rows) repeatable (7))
          select avg(c), (select count(*) from grupos) from (
            select count(*) c from puntos join grupos on x between xmin and xmax and y between ymin and ymax group by n)
        """).fetchone()

    vivos_por_paso = {n: sorted(vivos(s)) for n, s in pasos}
    ficheros_idx = [f"{IDX}/0000-base.parquet"]
    s_base = indexar(vivos_por_paso["base"], ficheros_idx[0])
    p, g = profundidad(ficheros_idx)
    di("indice_base", filas=con.execute(f"select count(*) from '{ficheros_idx[0]}'").fetchone()[0],
       segundos=round(s_base, 1), profundidad=round(p, 2), grupos=g)
    anterior = set(vivos_por_paso["base"])
    for k in range(1, CARGAS + 1):
        n = "carga-%02d" % k
        nuevos = sorted(set(vivos_por_paso[n]) - anterior)
        anterior = set(vivos_por_paso[n])
        f = f"{IDX}/{k:04d}-{n}.parquet"
        s = indexar(nuevos, f)
        ficheros_idx.append(f)
        p, g = profundidad(ficheros_idx)
        di("sigue", carga=k, ficheros_nuevos=len(nuevos),
           filas=con.execute(f"select count(*) from '{f}'").fetchone()[0], segundos=round(s, 2),
           profundidad=round(p, 2), grupos=g)
    entero = f"{T}/entero-s24.parquet"
    s_entero = indexar(vivos_por_paso["carga-%02d" % CARGAS], entero)
    p, g = profundidad([entero])
    di("reconstruir", hasta="carga-%02d" % CARGAS, segundos=round(s_entero, 1), profundidad=round(p, 2), grupos=g)

    # ── 4 · quitar filas: lo que la retención retira, en el índice ──────────────────────────────
    retirados = set(vivos_por_paso["carga-%02d" % CARGAS]) - set(vivos_por_paso["retencion"])
    ords = [ordinal[https(f)] for f in retirados if https(f) in ordinal]
    l = ", ".join("'%s'" % f for f in ficheros_idx)
    t = time.time()
    con.execute(f"""create or replace table posiciones as
        select filename file_path, file_row_number pos from read_parquet([{l}], filename = true, file_row_number = true)
        where _ore_fichero in ({", ".join(map(str, ords)) or "-1"})""")
    s_buscar = time.time() - t
    t = time.time()
    con.execute(f"copy (select * from posiciones order by file_path, pos) to '{T}/borrados.parquet' (format parquet)")
    s_escribir = time.time() - t
    por = con.execute("select count(*), count(distinct file_path) from posiciones").fetchone()
    total = con.execute(f"select count(*) from read_parquet([{l}])").fetchone()[0]
    di("quitar", paso="retencion", ficheros_retirados=len(retirados), filas=por[0], de=total,
       ficheros_del_indice_tocados=por[1], de_ficheros=len(ficheros_idx), buscar_s=round(s_buscar, 2),
       escribir_s=round(s_escribir, 2), bytes_de_borrados=os.path.getsize(f"{T}/borrados.parquet"))
    a, r, _ = diferencia(pasos[-1][1])
    di("quitar", paso="borrado-disperso", ficheros_retirados=len(r), ficheros_anadidos=len(a),
       vivos_antes=len(vivos_por_paso["retencion"]),
       nota="copy-on-write: reescribe lo que toca; para el índice, retirar y añadir esos ficheros")
finally:
    n = 0
    for b in gcs.list_blobs(CUBO, prefix=RAIZ + "/"):
        b.delete()
        n += 1
    shutil.rmtree(T + "/temp", ignore_errors=True)
    di("limpio", objetos=n, prefijo=RAIZ)
