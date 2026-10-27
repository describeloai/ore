"""Query Q0 · ¿Puede Query mandar a Spark el SQL que el cliente escribió para DuckDB?

Cada consulta del corpus —TPC-H (22), TPC-DS (99) e `idiomas.sql` (lo «amigable» de DuckDB)— se
traduce con sqlglot (`duckdb` → `spark`) y, con `--ejecutar`, corre en los dos motores sobre los
mismos Parquet (escala 0,1) y se comparan los resultados. Lo que sale de cada una:
  igual · distinto · no_traduce · falla_spark · falla_duckdb
  python3 dialecto.py DIR_DATOS [--ejecutar]
"""
import json
import math
import os
import re
import sys
import time

aqui = os.path.dirname(os.path.abspath(__file__))
datos = sys.argv[1]
ejecutar = "--ejecutar" in sys.argv


def bajar():
    """En el pod: los Parquet, las consultas y la rueda de sqlglot, del bucket del banco."""
    import subprocess
    import urllib.parse
    import urllib.request

    req = urllib.request.Request("http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token",
                                 headers={"Metadata-Flavor": "Google"})
    tok = json.load(urllib.request.urlopen(req, timeout=10))["access_token"]
    b = "project-8853a180-450d-47be-b83-banco-geo"
    url = "https://storage.googleapis.com/storage/v1/b/%s/o?prefix=query/dialecto/" % b
    for o in json.load(urllib.request.urlopen(urllib.request.Request(url, headers={"Authorization": "Bearer " + tok})))["items"]:
        destino = os.path.join(datos, o["name"][len("query/dialecto/"):])
        os.makedirs(os.path.dirname(destino), exist_ok=True)
        r = urllib.request.Request("https://storage.googleapis.com/%s/%s" % (b, urllib.parse.quote(o["name"])), headers={"Authorization": "Bearer " + tok})
        with urllib.request.urlopen(r, timeout=60) as resp, open(destino, "wb") as f:
            f.write(resp.read())
    rueda = [f for f in os.listdir(datos) if f.startswith("sqlglot-")][0]
    subprocess.run([sys.executable, "-m", "pip", "install", "-q", "--no-index", "--no-deps", "--target", "/trabajo/py", os.path.join(datos, rueda)], check=True)
    sys.path.insert(0, "/trabajo/py")
    import importlib
    importlib.invalidate_caches()  # PYTHONPATH vio /trabajo/py sin crear: su buscador vacío quedó en caché


if os.environ.get("BAJAR"):
    bajar()
import sqlglot  # noqa: E402


def corpus():
    tpc = json.load(open(os.path.join(datos, "consultas_tpc.json")))
    for grupo in ("tpch", "tpcds"):
        for n, q in tpc[grupo]:
            yield grupo, "%s-q%02d" % (grupo, n), q
    texto = open(os.path.join(aqui, "idiomas.sql"), encoding="utf-8").read()
    for bloque in re.split(r";\s*\n", texto):
        m = re.search(r"--\s*\((\w+)\)\s*\n(.*)", bloque, re.S)
        if m and m.group(2).strip():
            yield "idiomas", m.group(1), m.group(2).strip().rstrip(";")


def traducir(q):
    try:
        return sqlglot.transpile(q, read="duckdb", write="spark", unsupported_level=sqlglot.ErrorLevel.RAISE)[0], None
    except Exception as e:  # noqa: BLE001
        return None, type(e).__name__ + ": " + str(e).splitlines()[0][:200]


def normal(filas):
    def v(x):
        if isinstance(x, float):
            return None if math.isnan(x) else round(x, 2)
        if hasattr(x, "isoformat"):
            return x.isoformat()[:19]
        if isinstance(x, (list, tuple)):
            return tuple(v(y) for y in x)
        if isinstance(x, dict):  # un STRUCT de DuckDB: sus valores en orden (Spark da un Row)
            return tuple(v(y) for y in x.values())
        if x.__class__.__name__ == "Decimal":
            return round(float(x), 2)
        if x.__class__.__name__ == "Row":
            return tuple(v(y) for y in x)
        return x
    return sorted((tuple(v(c) for c in f) for f in filas), key=repr)


def main():
    salida = []
    if ejecutar:
        import duckdb
        from pyspark.sql import SparkSession

        d = duckdb.connect()
        if os.path.isdir("/opt/ore/duckdb"):
            d.execute("set extension_directory = '/opt/ore/duckdb'")
        for e in ("spatial", "h3", "json", "icu"):
            try:
                d.execute("load %s" % e)
            except Exception:  # noqa: BLE001
                pass
        sp = (SparkSession.builder.master("local[*]").config("spark.ui.enabled", "false")
              .config("spark.sql.ansi.enabled", "false").getOrCreate())
        os.chdir(datos)
    puestas = None
    for grupo, nombre, q in corpus():
        # TPC-H y TPC-DS comparten nombres (`customer`): cada grupo con SUS tablas (idiomas, las de TPC-H).
        tablas = "tpcds" if grupo == "tpcds" else "tpch"
        if ejecutar and tablas != puestas:
            for f in os.listdir(os.path.join(datos, tablas)):
                t, ruta = f[:-8], os.path.join(datos, tablas, f)
                d.execute("create or replace view %s as select * from read_parquet('%s')" % (t, ruta))
                sp.read.parquet(ruta).createOrReplaceTempView(t)
            puestas = tablas
        r = {"grupo": grupo, "consulta": nombre}
        spark_sql, error = traducir(q)
        if spark_sql is None:
            r["estado"], r["error"] = "no_traduce", error
        elif not ejecutar:
            r["estado"] = "traduce"
        else:
            try:
                a = normal(d.execute(q).fetchall())
            except Exception as e:  # noqa: BLE001
                a, r["estado"], r["error"] = None, "falla_duckdb", str(e).splitlines()[0][:200]
            if a is not None:
                t = time.time()
                try:
                    b = normal(sp.sql(spark_sql).collect())
                    r["estado"] = "igual" if a == b else "distinto"
                    if a != b:
                        r["duckdb"], r["spark"] = repr(a[:2])[:200], repr(b[:2])[:200]
                except Exception as e:  # noqa: BLE001
                    r["estado"], r["error"] = "falla_spark", str(e).splitlines()[0][:240]
                r["s_spark"] = round(time.time() - t, 2)
        if r["estado"] not in ("igual", "traduce"):
            r["spark_sql"] = (spark_sql or "")[:300]
        salida.append(r)
        print(json.dumps(r, ensure_ascii=False), flush=True)
    resumen = {}
    for r in salida:
        resumen.setdefault(r["grupo"], {}).setdefault(r["estado"], 0)
        resumen[r["grupo"]][r["estado"]] += 1
    print(json.dumps({"resumen": resumen}), flush=True)


if __name__ == "__main__":
    main()
