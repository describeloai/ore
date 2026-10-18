"""ORE 0068 C0·2–C0·3 · La vara de cómputo: los mismos casos en DuckDB y en Spark + Sedona.

    correr.py <motor> <conjunto> <datos> <salida.jsonl> [limite_s] [casos]
      motor     duckdb | spark
      conjunto  spatialbench (las 12 de SpatialBench, en el dialecto de cada motor)
                banco        (B1–B4 sobre el banco de cómputo de preparar-banco.py)
      casos     p. ej. q1,q4,b2 (por defecto, todos)

DuckDB corre en la imagen del puesto (python + duckdb 1.5.6 con spatial y h3 en /opt/ore/duckdb).
Spark corre con spark-submit en apache/spark:4.1.0 y los jars de Sedona en el classpath (ver
sedona-en-spark41.sh). Las consultas de SpatialBench salen de su repositorio (`SB_QUERIES`, el
directorio spatialbench-queries, commit 8e44bdd).

Por cada caso, una línea JSON: motor, caso, estado (bien | error | limite), segundos, filas y una
huella del resultado (filas ordenadas, flotantes a 4 decimales, binarios por su longitud) para
cotejar los dos motores.
"""
import hashlib, json, os, sys, threading, time

motor, conjunto, datos, salida = sys.argv[1:5]
LIMITE = int(sys.argv[5]) if len(sys.argv) > 5 else 600
SOLO = set(sys.argv[6].split(",")) if len(sys.argv) > 6 and sys.argv[6] else None

# ── Los casos ──────────────────────────────────────────────────────────────────────────────

TABLAS_SB = ["trip", "zone", "building", "customer", "driver", "vehicle"]
TABLAS_BANCO = ["edificios", "lugares", "municipios", "ais"]

# El banco (B1–B4). Mismo SQL en los dos salvo H3. Geometrías en WKB, como SpatialBench.
MADRID = ("(select geom from municipios where nombre = 'Madrid' "
          "order by ST_Area(ST_GeomFromWKB(geom)) desc limit 1)")
BANCO = {
    # Filtro por zona: los edificios del término municipal de Madrid.
    "b1": f"""select count(*) as n, round(sum(altura), 1) as altura
              from edificios e, {MADRID} m
              where ST_Intersects(ST_GeomFromWKB(e.geom), ST_GeomFromWKB(m.geom))""",
    # Punto en polígono (la forma de enriquecer): cada lugar, su municipio.
    "b2": """select count(*) as municipios, sum(n) as lugares from (
               select m.id, count(*) as n from lugares l join municipios m
               on ST_Contains(ST_GeomFromWKB(m.geom), ST_GeomFromWKB(l.geom)) group by m.id) x""",
    # Cercanía en metros (EPSG:25830): pares lugar–edificio a 20 m o menos.
    "b3": """select count(*) as pares from lugares l join edificios e
             on ST_DWithin(ST_GeomFromWKB(l.utm), ST_GeomFromWKB(e.utm), 20)""",
    # Agregación H3: posiciones AIS por celda de resolución 8.
    # B2 y Q2 con la difusión explícita (C0·3: sin ella Spark parte las dos tablas en una RangeJoin,
    # y la subconsulta escalar de Q2 es un Filter fila a fila contra un polígono sin preparar).
    "b2d": {
        "duckdb": None,
        "spark": """select count(*) as municipios, sum(n) as lugares from (
               select /*+ BROADCAST(m) */ m.id, count(*) as n from lugares l join municipios m
               on ST_Contains(ST_GeomFromWKB(m.geom), ST_GeomFromWKB(l.geom)) group by m.id) x""",
    },
    "b3d": {
        "duckdb": None,
        "spark": """select /*+ BROADCAST(l) */ count(*) as pares from lugares l join edificios e
             on ST_DWithin(ST_GeomFromWKB(l.utm), ST_GeomFromWKB(e.utm), 20)""",
    },
    "b4": {
        "duckdb": """select count(*) as celdas, max(n) as max_n, sum(n) as posiciones from (
                       select h3_latlng_to_cell(ST_Y(g), ST_X(g), 8) as c, count(*) as n
                       from (select ST_GeomFromWKB(geom) as g from ais) group by 1) x""",
        "spark": """select count(*) as celdas, max(n) as max_n, sum(n) as posiciones from (
                      select ST_H3CellIDs(ST_GeomFromWKB(geom), 8, false)[0] as c, count(*) as n
                      from ais group by 1) x""",
    },
}


def casos():
    if conjunto == "banco":
        out = {k: (v[motor] if isinstance(v, dict) else v) for k, v in BANCO.items()}
        out = {k: v for k, v in out.items() if v}
    else:
        sys.path.append(os.environ.get("SB_QUERIES", "/sb-queries"))
        import print_queries as pq
        clase = pq.DuckDBSpatialBenchBenchmark if motor == "duckdb" else pq.SpatialBenchBenchmark
        out = clase().queries()
        if motor == "spark":
            # Q2 como join difundido (lo mismo, escrito para que Sedona use su índice).
            out["q2d"] = """SELECT /*+ BROADCAST(z) */ COUNT(*) AS trip_count_in_coconino_county
                FROM trip t JOIN (SELECT z_boundary FROM zone WHERE z_name = 'Coconino County' LIMIT 1) z
                ON ST_Intersects(ST_GeomFromWKB(t.t_pickuploc), ST_GeomFromWKB(z.z_boundary))"""
    return {k: v for k, v in out.items() if not SOLO or k in SOLO}


# ── La huella ─────────────────────────────────────────────────────────────────────────────

def norma(v):
    import datetime, decimal
    # Spark da AVG de decimales en Decimal y DuckDB en double; los intervalos difieren en los
    # microsegundos del promedio. La huella compara el cálculo, no la representación.
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
    return hashlib.sha256("\n".join(filas).encode()).hexdigest()[:16], (filas[:3] if len(filas) <= 3 else filas[:2])


# ── Los motores ───────────────────────────────────────────────────────────────────────────

def con_duckdb():
    import duckdb
    c = duckdb.connect()
    c.execute("set extension_directory = '/opt/ore/duckdb'")
    for e in ("spatial", "h3"):
        c.execute("load " + e)
    if os.environ.get("C0_MEMORIA"):
        c.execute("set memory_limit = '%s'" % os.environ["C0_MEMORIA"])
    for t in (TABLAS_BANCO if conjunto == "banco" else TABLAS_SB):
        p = os.path.join(datos, t + ".parquet")
        if os.path.exists(p) or os.path.isdir(p):
            q = p + "/*.parquet" if os.path.isdir(p) else p
            c.execute(f"create view {t} as select * from read_parquet('{q}')")

    def una(sql):
        r = [None]

        def ir():
            try:
                r[0] = ("bien", c.execute(sql).fetchall())
            except Exception as e:  # interrumpida o fallida
                r[0] = ("error", str(e).splitlines()[0][:300])
        h = threading.Thread(target=ir)
        h.start()
        h.join(LIMITE)
        if h.is_alive():
            c.interrupt()
            h.join()
            return "limite", None
        return r[0]
    return una


def con_spark():
    from pyspark.sql import SparkSession
    s = SparkSession.builder.appName("ore-c0").getOrCreate()
    s.sparkContext.setLogLevel("ERROR")
    for t in (TABLAS_BANCO if conjunto == "banco" else TABLAS_SB):
        p = os.path.join(datos, t + ".parquet")
        if os.path.exists(p):
            s.read.parquet(p).createOrReplaceTempView(t)

    def una(sql):
        r = [None]
        grupo = "c0-%d" % time.time_ns()

        def ir():
            try:
                s.sparkContext.setJobGroup(grupo, grupo, interruptOnCancel=True)
                r[0] = ("bien", [tuple(x) for x in s.sql(sql).collect()])
            except Exception as e:
                r[0] = ("error", str(e).splitlines()[0][:300])
        h = threading.Thread(target=ir)
        h.start()
        h.join(LIMITE)
        if h.is_alive():
            s.sparkContext.cancelJobGroup(grupo)
            h.join()
            return "limite", None
        return r[0]
    return una


una = con_duckdb() if motor == "duckdb" else con_spark()
# Con «-», sólo a la salida: correr.sh la añade al fichero desde fuera, y si el motor muere a
# media corrida (Spark en el laboratorio, Q10–Q12) lo ya medido no se pierde.
f = open(salida, "a") if salida != "-" else None
for nombre, sql in casos().items():
    t = time.time()
    estado, res = una(sql)
    seg = round(time.time() - t, 2)
    linea = {"motor": motor, "conjunto": conjunto, "datos": datos, "caso": nombre, "estado": estado,
             "segundos": seg if estado != "limite" else LIMITE}
    if estado == "bien":
        linea["filas"] = len(res)
        linea["huella"], linea["muestra"] = huella(res)
    elif estado == "error":
        linea["error"] = res
    print(json.dumps(linea, ensure_ascii=False), flush=True)
    if f:
        f.write(json.dumps(linea, ensure_ascii=False) + "\n")
        f.flush()
