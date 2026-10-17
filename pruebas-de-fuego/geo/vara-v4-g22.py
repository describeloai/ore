"""vara-v4-g22.py (ADR 0067 G2·2, caso V4): leer por zona sobre las variantes de los 34 M de edificios.

Corre en un puesto de victor (imagen del puesto, cuenta `puesto`), contra el bucket del banco por HTTPS, con
`spatial` y `h3` traídas del bucket (`puesto-extensiones.py`, que vara.py pone delante). Por variante:
  - los grupos de filas que el `geo_bbox` (las estadísticas del tipo lógico GEOMETRY) deja descartar en cada zona;
  - el tiempo de la consulta por zona de tres formas: por las columnas `bbox` (como Overture), con
    `ST_Intersects` sobre la geometría, y con `&&`;
  - p50 y p95 con 1, 8 y 32 lectores concurrentes (`ST_Intersects`);
  - la agregación por celda H3 r8 de una zona: con la columna precalculada (B) y calculándola (C).
Emite «V4|…».
"""
import concurrent.futures as cf
import json
import statistics
import time
import urllib.request

import duckdb

B = "https://storage.googleapis.com/project-8853a180-450d-47be-b83-banco-geo/g22"
VARIANTES = {
    "a_tal_cual": f"{B}/a_tal_cual.parquet",
    "b_hilbert_100k": f"{B}/b_hilbert_100k.parquet",
    "c_hilbert_25k": f"{B}/c_hilbert_25k.parquet",
    "d_hilbert_400k": f"{B}/d_hilbert_400k.parquet",
}
ZONAS = {  # xmin, ymin, xmax, ymax: el centro de cada ciudad, unos 8 × 8 km
    "madrid": (-3.75, 40.38, -3.65, 40.46), "barcelona": (2.10, 41.35, 2.22, 41.45),
    "valencia": (-0.42, 39.43, -0.33, 39.51), "sevilla": (-6.03, 37.35, -5.94, 37.42),
    "bilbao": (-2.97, 43.23, -2.90, 43.29), "zaragoza": (-0.93, 41.62, -0.84, 41.69),
    "malaga": (-4.47, 36.69, -4.38, 36.75), "coruna": (-8.44, 43.34, -8.37, 43.39),
}
TOKEN = token()  # noqa: F821 (de puesto-extensiones.py)


def conexion(memoria="1GB", hilos=4):
    c = cargar(duckdb.connect())  # noqa: F821
    c.execute(f"create secret g (type http, bearer_token '{TOKEN}')")
    # 32 lectores en un pod de 4 GiB (el máximo de la cola): cada conexión, acotada.
    c.execute(f"set threads = {hilos}; set memory_limit = '{memoria}'")
    return c


def caja(z):
    x0, y0, x1, y1 = ZONAS[z]
    return f"ST_MakeEnvelope({x0}, {y0}, {x1}, {y1})"


def por_bbox(url, z):
    x0, y0, x1, y1 = ZONAS[z]
    return (f"select count(*) from read_parquet('{url}') where bbox.xmax >= {x0} and bbox.xmin <= {x1} "
            f"and bbox.ymax >= {y0} and bbox.ymin <= {y1}")


def por_geom(url, z):
    # Medido en la primera pasada (v4-g22-a-intersects.log): `ST_Intersects` solo NO poda por `geo_bbox`
    # (19–30 s, todo el fichero); `&&` sí. La consulta exacta lleva los dos.
    return f"select count(*) from read_parquet('{url}') where geom && {caja(z)} and ST_Intersects(geom, {caja(z)})"


def por_y(url, z):
    return f"select count(*) from read_parquet('{url}') where geom && {caja(z)}"


def una(sql, concurrente=False):
    c = conexion("110MB", 1) if concurrente else conexion()
    t0 = time.time()
    n = c.execute(sql).fetchone()[0]
    return time.time() - t0, n


c = conexion()
# `SOLO` (lo pone correr-v4-g22.py): medir solo esas variantes; [] = solo H3 y particiones.
for v, url in [(k, u) for k, u in VARIANTES.items() if globals().get("SOLO") is None or k in SOLO]:  # noqa: F821
    t0 = time.time()
    md = c.execute(f"select row_group_id, geo_bbox from parquet_metadata('{url}') where path_in_schema = 'geom'").fetchall()
    total = len(md)
    print(f"V4|{v}|metadatos|-|{total} grupos de filas, leídos en {time.time() - t0:.1f} s", flush=True)
    for z in ZONAS:
        x0, y0, x1, y1 = ZONAS[z]
        tocan = sum(1 for _, b in md if b and b["xmax"] >= x0 and b["xmin"] <= x1 and b["ymax"] >= y0 and b["ymin"] <= y1)
        tb, nb = una(por_bbox(url, z))
        tg, ng = una(por_geom(url, z))
        ty, ny = una(por_y(url, z))
        print(f"V4|{v}|{z}|-|grupos {tocan}/{total} (descarte {100 * (1 - tocan / total):.1f} %) · "
              f"bbox {tb:.2f} s ({nb}) · && + ST_Intersects {tg:.2f} s ({ng}) · && {ty:.2f} s ({ny})", flush=True)
    for lectores in (1, 8, 32):
        sqls = [por_geom(url, list(ZONAS)[i % len(ZONAS)]) for i in range(max(16, lectores * 2))]
        t0 = time.time()
        with cf.ThreadPoolExecutor(lectores) as ex:
            ts = sorted(t for t, _ in ex.map(lambda q: una(q, lectores > 1), sqls))
        p95 = ts[min(len(ts) - 1, int(0.95 * len(ts)))]
        print(f"V4|{v}|concurrencia|{lectores} lectores|{len(ts)} consultas en {time.time() - t0:.1f} s · "
              f"p50 {statistics.median(ts):.2f} s · p95 {p95:.2f} s", flush=True)

# H3: la agregación de una zona, con la columna (B) o calculándola (C).
z = "madrid"
t0 = time.time()
n1 = c.execute(f"select count(distinct h3_r8) from read_parquet('{VARIANTES['b_hilbert_100k']}') where geom && {caja(z)} and ST_Intersects(geom, {caja(z)})").fetchone()[0]
t1 = time.time() - t0
t0 = time.time()
n2 = c.execute(f"""select count(distinct h3_latlng_to_cell(ST_Y(ST_Centroid(geom)), ST_X(ST_Centroid(geom)), 8))
                   from read_parquet('{VARIANTES['c_hilbert_25k']}') where geom && {caja(z)} and ST_Intersects(geom, {caja(z)})""").fetchone()[0]
t2 = time.time() - t0
print(f"V4|h3|{z}|r8|columna precalculada {t1:.2f} s ({n1} celdas) · calculada al vuelo {t2:.2f} s ({n2} celdas)")

# E: particionado por celda H3 r3 (un fichero por celda); se leen solo las celdas que tocan la zona.
listado = json.loads(urllib.request.urlopen(urllib.request.Request(
    "https://storage.googleapis.com/storage/v1/b/project-8853a180-450d-47be-b83-banco-geo/o/g22%2Fe_h3r3%2Flistado.json?alt=media",
    headers={"authorization": f"Bearer {TOKEN}"})).read())
print(f"V4|e_h3r3|metadatos|-|{len(listado)} particiones")


def por_particion(z, concurrente=False):
    cc = conexion("110MB", 1) if concurrente else conexion()
    x0, y0, x1, y1 = ZONAS[z]
    celdas = {r[0] for r in cc.execute(f"""select unnest(h3_grid_disk(h3_latlng_to_cell(y, x, 3), 1))
        from (select unnest([{x0}, {x1}]) x) cross join (select unnest([{y0}, {y1}]) y)""").fetchall()}
    ficheros = [f"{B}/e_h3r3/{p}" for c_, p in listado.items() if int(c_) in celdas]
    t0 = time.time()
    lista = ", ".join(f"'{f}'" for f in ficheros)
    n = cc.execute(f"select count(*) from read_parquet([{lista}]) where geom && {caja(z)} and ST_Intersects(geom, {caja(z)})").fetchone()[0]
    return time.time() - t0, n, len(ficheros)


for z in ZONAS:
    t, n, k = por_particion(z)
    print(f"V4|e_h3r3|{z}|-|{k} ficheros leídos · ST_Intersects {t:.2f} s ({n})", flush=True)
for lectores in (1, 8, 32):
    zs = [list(ZONAS)[i % len(ZONAS)] for i in range(max(16, lectores * 2))]
    t0 = time.time()
    with cf.ThreadPoolExecutor(lectores) as ex:
        ts = sorted(t for t, _, _ in ex.map(lambda z: por_particion(z, lectores > 1), zs))
    p95 = ts[min(len(ts) - 1, int(0.95 * len(ts)))]
    print(f"V4|e_h3r3|concurrencia|{lectores} lectores|{len(ts)} consultas en {time.time() - t0:.1f} s · "
          f"p50 {statistics.median(ts):.2f} s · p95 {p95:.2f} s", flush=True)
print("## ok")
