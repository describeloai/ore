"""ORE 0072 F2·3·2 · prueba de concepto de la pirámide (dentro de un Job de victor, imagen del puesto).

Por cada fuente: lee del lago con la credencial prestada (como el banco), exporta a FlatGeobuf, construye la
pirámide con tippecanoe (z0–16, la selección por zoom de tippecanoe), cuenta las teselas por zoom (peso medio,
p95 y máximo) y sube el `.pmtiles` al bucket del banco. Todo lo que importa sale en líneas `PC|que|{json}`.

  PC_FILAS=100000   filas por fuente (0 = todas)
  PC_FUENTES=...    trip_3m_puntos,trip_3m_lineas,edificios (y se describen trip_3m, trip_30m)
  PC_MAXZOOM=16     el zoom más alto de la pirámide (en el diseño, el suelo del crudo: por encima sirve el motor)
  PC_SUBIR=1        dejar los .pmtiles al lanzador, que los sube a gs://<banco>/piramide/
"""
import json
import math
import os
import resource
import sqlite3
import subprocess
import sys
import tarfile
import io
import time
import urllib.parse
import urllib.request

sys.path.insert(0, "/opt/ore")
os.environ.setdefault("ORE_SERVE", "http://ore-serve.%s.svc.cluster.local:8080" % os.environ.get("NS", "t-victor"))
import ore  # noqa: E402

FILAS = int(os.environ.get("PC_FILAS", "100000"))
# por fuente (PC_FILAS_trip_3m_lineas=100000): las líneas no caben enteras en una pasada rápida


def filas_de_fuente(n):
    return int(os.environ.get("PC_FILAS_" + n, FILAS))
FUENTES = os.environ.get("PC_FUENTES", "trip_3m_puntos,trip_3m_lineas,edificios").split(",")
SUBIR = os.environ.get("PC_SUBIR", "1") == "1"
BUCKET = "project-8853a180-450d-47be-b83-banco-geo"
BANCO = "https://storage.googleapis.com/" + BUCKET
DELEGAR = {"x-iceberg-access-delegation": "vended-credentials"}
T = "/tmp/pc"
os.makedirs(T, exist_ok=True)

# lo que cada tipo de geometría pide a tippecanoe (lo que se itera en PC1)
OPCIONES = {
    "puntos": ["--drop-densest-as-needed", "--extend-zooms-if-still-dropping"],
    "lineas": ["--drop-densest-as-needed", "--simplification=4"],
    # `--detect-shared-borders` pasó de 3,5 GiB con 1,3 M de edificios (OOM): fuera
    "poligonos": ["--coalesce-densest-as-needed", "--simplification=4"],
}
OPCIONES = json.loads(os.environ["PC_OPCIONES"]) if os.environ.get("PC_OPCIONES") else OPCIONES


def di(que, **kw):
    print("PC|" + que + "|" + json.dumps(kw, ensure_ascii=False, default=str), flush=True)


def tiempo(f):
    t0 = time.time()
    r = f()
    return r, round(time.time() - t0, 1)


def testigo():
    d = os.environ["DIRECCION"].rstrip("/")
    cli = open("/puesto/agente-cliente").read().strip()
    sec = open("/puesto/agente-secreto").read().strip()
    datos = urllib.parse.urlencode({"grant_type": "client_credentials", "client_id": cli, "client_secret": sec}).encode()
    url = d + "/realms/%s/protocol/openid-connect/token" % os.environ.get("REALM", "rubix")
    with urllib.request.urlopen(url, data=datos, timeout=20) as r:
        return {"authorization": "Bearer " + json.load(r)["access_token"]}


def tippecanoe():
    """tippecanoe de conda-forge con micromamba (la imagen no lo trae; ~11 s medido)."""
    if os.path.exists("/tmp/tip/bin/tippecanoe"):
        return "/tmp/tip/bin/"
    url = "https://micro.mamba.pm/api/micromamba/linux-64/latest"
    tarfile.open(fileobj=io.BytesIO(urllib.request.urlopen(url).read()), mode="r:bz2").extract("bin/micromamba", "/tmp")
    subprocess.run(["/tmp/bin/micromamba", "create", "-y", "-q", "-p", "/tmp/tip", "-c", "conda-forge", "tippecanoe"],
                   check=True, capture_output=True, env=dict(os.environ, MAMBA_ROOT_PREFIX="/tmp/mamba"))
    return "/tmp/tip/bin/"


def con_lago(nombre):
    """`(con, from)` de un dataset del catálogo: loadTable con la credencial prestada e `iceberg_scan`."""
    ore.session._cabeceras = testigo()
    c, r = ore.session.pedir("GET", "/v1/query/namespaces/default/tables/%s" % nombre, cabeceras=DELEGAR)
    if c != 200:
        raise RuntimeError("loadTable %s: %s %s" % (nombre, c, str(r)[:200]))
    cfg = r.get("config", {})
    token = cfg.get("gcs.oauth2.token")
    if not token:
        raise RuntimeError("sin gcs.oauth2.token en la credencial (claves: %s)" % sorted(cfg))
    fuente = ore._iceberg(r["metadata-location"], prestada=token)
    return ore._duckdb(), fuente


def con_banco():
    con = ore._duckdb()
    for e in ("spatial", "httpfs"):
        con.execute("load " + e)
    con.execute("create or replace secret banco (type http, bearer_token '%s', scope '%s')"
                % (ore._token_de_google().replace("'", "''"), BANCO))
    return con, "read_parquet('%s/g22/a_tal_cual.parquet')" % BANCO


def describir(con, fuente):
    return [(n, t) for n, t, *_ in con.execute("describe select * from %s" % fuente).fetchall()]


def exportar(nombre, con, fuente, filas_de=None, filas=0):
    """A FlatGeobuf (`id` y la geometría en 4326), con GDAL de `spatial`; si no, GeoJSON por líneas."""
    ore._cargar(con, "spatial")
    cols = dict(describir(con, fuente))
    geom = "geom" if "geom" in cols else next((n for n, t in cols.items() if "GEOMETRY" in t or t == "BLOB"), None)
    if geom is None:
        raise RuntimeError("sin columna de geometría: %s" % cols)
    g = '"%s"' % geom if "GEOMETRY" in cols[geom] else 'ST_GeomFromWKB("%s")' % geom
    ident = '"id"' if "id" in cols else "row_number() over ()"
    donde = " where " + filas_de.format(g=g) if filas_de else ""
    q = "select %s::bigint as id, %s as geom from %s%s" % (ident, g, fuente, donde)
    if filas:
        # una muestra uniforme: `limit` daría las primeras en orden Hilbert, una esquina del mapa
        q = "select * from (%s) using sample reservoir(%d rows) repeatable (1)" % (q, filas)
    fgb = "%s/%s.fgb" % (T, nombre)
    try:
        _, s = tiempo(lambda: con.execute("copy (%s) to '%s' (format gdal, driver 'FlatGeobuf', srs 'EPSG:4326')" % (q, fgb)))
        n = con.execute("select count(*) from st_read('%s')" % fgb).fetchone()[0]
        return fgb, n, s, "fgb"
    except Exception as e:  # noqa: BLE001
        di("sin_fgb", fuente=nombre, error=str(e)[:200])
    gj = "%s/%s.geojsonl" % (T, nombre)
    t0 = time.time()
    n = 0
    lector = con.execute("select id, ST_AsGeoJSON(geom) from (%s)" % q).fetch_record_batch(50_000)
    with open(gj, "w") as f:
        for lote in lector:
            for i, j in zip(lote.column(0).to_pylist(), lote.column(1).to_pylist()):
                if j:
                    f.write('{"type":"Feature","properties":{"id":%d},"geometry":%s}\n' % (i, j))
                    n += 1
    return gj, n, round(time.time() - t0, 1), "geojsonl"


TOPE_S = int(os.environ.get("PC_TOPE_S", "900"))


def piramide(bin_, nombre, entrada, tipo, zooms=None, opciones=None):
    """tippecanoe sobre `entrada`, con tope de tiempo (una fuente no se come el Job)."""
    mb = "%s/%s.mbtiles" % (T, nombre)
    pm = "%s/%s.pmtiles" % (T, nombre)
    for f in (mb, pm):
        if os.path.exists(f):
            os.remove(f)
    zooms = zooms or (0, int(os.environ.get("PC_MAXZOOM", "16")))
    orden = [bin_ + "tippecanoe", "-q", "-f", "-o", mb, "-Z%d" % zooms[0], "-z%d" % zooms[1], "-l", nombre,
             "--no-tile-size-limit" if os.environ.get("PC_SIN_TOPE") else "--maximum-tile-bytes=500000"] + (
        OPCIONES[tipo] if opciones is None else opciones) + [entrada]
    antes = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
    t0 = time.time()
    try:
        p = subprocess.run(orden, capture_output=True, text=True, timeout=TOPE_S)
    except subprocess.TimeoutExpired:
        raise RuntimeError("tippecanoe pasó de %d s: %s" % (TOPE_S, " ".join(orden[1:-1])))
    s = round(time.time() - t0, 1)
    rss = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
    if p.returncode != 0:
        raise RuntimeError("tippecanoe: " + p.stderr[-500:])
    subprocess.run([bin_ + "tile-join", "-q", "-f", "-o", pm, mb], check=True, capture_output=True)
    db = sqlite3.connect(mb)
    zooms = []
    for z, n, media, maximo in db.execute("select zoom_level, count(*), avg(length(tile_data)), max(length(tile_data)) "
                                          "from tiles group by zoom_level order by zoom_level"):
        tam = sorted(r[0] for r in db.execute("select length(tile_data) from tiles where zoom_level=?", (z,)))
        zooms.append({"z": z, "teselas": n, "kb_media": round(media / 1024, 1),
                      "kb_p95": round(tam[int(0.95 * (len(tam) - 1))] / 1024, 1), "kb_max": round(maximo / 1024, 1)})
    # cuántos elementos lleva cada zoom (lo que el ojo ve al acercarse): de los metadatos de tippecanoe
    meta = dict(db.execute("select name, value from metadata").fetchall())
    db.close()
    return {"orden": " ".join(orden[1:-1]), "s": s, "rss_mb": round(max(rss, antes) / 1024),
            "mb_pmtiles": round(os.path.getsize(pm) / 2**20, 1), "zooms": zooms,
            "json": json.loads(meta.get("json", "{}")).get("tilestats", {}).get("layers", [{}])[0].get("count")}, pm


def densidad(entrada, nombre, hasta=12, celdas=5, modo="poligonos"):
    """Por debajo del suelo, lo que no se aclara se agrega: por cada zoom z ≤ `hasta`, una rejilla de
    2^`celdas` × 2^`celdas` celdas por tesela (en WebMercator). La rejilla más fina se calcula una vez y las
    demás salen de ella (cx >> s): todos los zooms cuentan lo mismo. Cada celda, un cuadrado sólo en su zoom
    (`tippecanoe.minzoom/maxzoom`) con `n`, `a` y `cobertura`:
      · polígonos: por su centroide; `a` = área construida (m²), cobertura = a / área de la celda;
      · líneas: muestreadas a un lado de la celda más fina; `a` = metros de línea en la celda,
        cobertura = km de línea por km² (a / área de la celda × 1000: no depende del zoom)."""
    con = ore._duckdb()
    ore._cargar(con, "spatial")
    # el disco del nodo es de todos: lo que no quepa, que falle aquí (el 25-oct, 10 GB de derrame
    # desalojaron este Job y el aprovisionador de ore-system del nodo del sistema)
    con.execute("set max_temp_directory_size = '2GB'")
    con.execute("set preserve_insertion_order = false")
    t0 = time.time()
    Lmax = hasta + celdas
    celda = """floor((lon + 180) / 360 * pow(2, {L}))::bigint cx,
               floor((1 - ln(tan(radians(lat)) + 1 / cos(radians(lat))) / pi()) / 2 * pow(2, {L}))::bigint cy""".format(L=Lmax)
    if modo == "poligonos":
        puntos = """select ST_X(ST_Centroid(geom)) lon, ST_Y(ST_Centroid(geom)) lat, ST_Area_Spheroid(geom) a
                    from st_read('%s')""" % entrada
    else:
        paso = 360 / 2 ** Lmax
        puntos = """
            with v as (select ST_X(ST_PointN(geom, i::integer)) x0, ST_Y(ST_PointN(geom, i::integer)) y0,
                              ST_X(ST_PointN(geom, (i + 1)::integer)) x1, ST_Y(ST_PointN(geom, (i + 1)::integer)) y1
                       from st_read('{e}'), generate_series(1, ST_NPoints(geom) - 1) t(i)),
                 s as (select *, greatest(1, ceil(sqrt((x1 - x0) ^ 2 + (y1 - y0) ^ 2) / {p}))::int k,
                              sqrt(((x1 - x0) * cos(radians(y0)) * 111320) ^ 2 + ((y1 - y0) * 110540) ^ 2) m from v)
            select x0 + (x1 - x0) * (j + 0.5) / k lon, y0 + (y1 - y0) * (j + 0.5) / k lat, m / k a
            from s, generate_series(0, k - 1) u(j)""".format(e=entrada, p=paso)
    con.execute("create or replace temp table fina as select cx, cy, count(*) n, sum(a) a from (select %s, a from (%s)) group by all"
                % (celda, puntos))
    t_fina = round(time.time() - t0, 1)
    salida = "%s/%s.geojsonl" % (T, nombre)
    n = 0
    with open(salida, "w") as f:
        for z in range(0, hasta + 1):
            L = z + celdas
            d = Lmax - L
            filas = con.execute("select cx >> %d, cy >> %d, sum(n), sum(a) from fina group by all" % (d, d)).fetchall()
            m = 2 ** L
            for cx, cy, cuenta, a in filas:
                lon0, lon1 = cx / m * 360 - 180, (cx + 1) / m * 360 - 180
                lat0 = math.degrees(math.atan(math.sinh(math.pi * (1 - 2 * (cy + 1) / m))))
                lat1 = math.degrees(math.atan(math.sinh(math.pi * (1 - 2 * cy / m))))
                lado = 40075016.7 * math.cos(math.radians((lat0 + lat1) / 2)) / m
                # las dos, por m² de celda (si no, depende del zoom): área construida / área; metros de línea / área
                cob = (a or 0) / (lado * lado)
                f.write('{"type":"Feature","tippecanoe":{"minzoom":%d,"maxzoom":%d},"properties":{"n":%d,"a":%d,"cobertura":%.4f},'
                        '"geometry":{"type":"Polygon","coordinates":[[[%.6f,%.6f],[%.6f,%.6f],[%.6f,%.6f],[%.6f,%.6f],[%.6f,%.6f]]]}}\n'
                        % (z, z, cuenta, a or 0, min(cob, 1.0) if modo == "poligonos" else cob * 1000,
                           lon0, lat0, lon1, lat0, lon1, lat1, lon0, lat1, lon0, lat0))
                n += 1
    con.execute("drop table fina")
    con.close()
    ore._con = None
    return salida, n, round(time.time() - t0, 1), t_fina


def entregar(pm):
    """El pod no puede escribir en el bucket del banco (403): deja el `.pmtiles` y espera a que el lanzador lo
    copie (`kubectl cp`) y lo suba con la cuenta de quien lo lanza; `<fichero>.liberar` lo suelta. Uno a uno,
    según salen: lo que falle después no se lleva lo hecho."""
    di("listo", fichero=pm)
    fin = time.time() + 900
    while time.time() < fin and not os.path.exists(pm + ".liberar"):
        time.sleep(2)
    os.remove(pm)


def main():
    bin_, s = tiempo(tippecanoe)
    v = subprocess.run([bin_ + "tippecanoe", "--version"], capture_output=True, text=True)
    di("tippecanoe", s=s, version=(v.stdout + v.stderr).strip(), filas=FILAS, cpus=os.cpu_count())
    # lo que hay detrás de las capas que no dieron `geom` (trip_3m, trip_30m): su esquema, para PC1
    for n in ("trip_3m", "trip_30m"):
        try:
            con, f = con_lago(n)
            di("esquema", fuente=n, columnas=describir(con, f))
        except Exception as e:  # noqa: BLE001
            di("falla", fuente=n, paso="esquema", error=str(e)[:300])
    tipos = {"trip_3m_puntos": "puntos", "trip_3m_lineas": "lineas", "edificios": "poligonos"}
    for n in FUENTES:
        try:
            con, f = con_banco() if n == "edificios" else con_lago(n)
            filtro = "ST_Intersects({g}, ST_MakeEnvelope(-4.6, 39.8, -2.8, 41.0))" if n == "edificios" else None
            entrada, filas, s_exp, formato = exportar(n, con, f, filtro, filas_de_fuente(n))
            di("exportar", fuente=n, filas=filas, s=s_exp, formato=formato,
               mb=round(os.path.getsize(entrada) / 2**20, 1))
            # la memoria de DuckDB, fuera antes de tippecanoe (la conexión es la única de `ore`: se reabre)
            con.close()
            ore._con = None
            if tipos[n] in ("poligonos", "lineas") and os.environ.get("PC_AGREGAR_" + tipos[n].upper(), "1") == "1":
                # las líneas, más densas: su suelo, más alto
                suelo = int(os.environ.get("PC_SUELO_" + tipos[n].upper(), "13" if tipos[n] == "poligonos" else "14"))
                gj, celdas, s_d, s_fina = densidad(entrada, n + "_densidad", suelo - 1, modo=tipos[n])
                di("densidad", fuente=n, celdas=celdas, s=s_d, s_rejilla_fina=s_fina, mb=round(os.path.getsize(gj) / 2**20, 1))
                r, pm = piramide(bin_, n + "_densidad", gj, tipos[n], (0, suelo - 1),
                                 ["--no-tile-size-limit", "--no-feature-limit", "--no-tiny-polygon-reduction",
                                  "--no-simplification-of-shared-nodes", "--detect-shared-borders"])
                os.remove(gj)
                di("piramide", fuente=n + "_densidad", **r)
                if SUBIR:
                    entregar(pm)
                if os.environ.get("PC_SIN_CRUDO"):
                    continue
                # encima del suelo, los edificios tal cual (en el diseño los sirve el motor; aquí, para verlo)
                r, pm = piramide(bin_, n, entrada, tipos[n], (suelo, suelo + 1), ["--drop-densest-as-needed"])
            else:
                r, pm = piramide(bin_, n, entrada, tipos[n])
            os.remove(entrada)
            di("piramide", fuente=n, **r)
            if SUBIR:
                entregar(pm)
        except Exception as e:  # noqa: BLE001
            import traceback
            di("falla", fuente=n, error=str(e)[:400], donde=traceback.format_exc().splitlines()[-3:])
    di("fin")


main()
