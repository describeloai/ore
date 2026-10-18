"""ORE 0068 C2·3 pieza 2 · Lo que el arnés PySpark necesita saber de Spark + Iceberg contra el /v1 de
ORE, antes de escribirlo. Corre en `ore-puesto-spark:local` en la red del banco de C2·4
(`medir.sh`), con la identidad de cabecera de ana. Una línea `caso · resultado` por caso.
"""
import json
import urllib.request
import zipfile, os

from pyspark.sql import SparkSession
from pyspark.sql import functions as F

URI = "http://127.0.0.1:8080"
spark = (SparkSession.builder.master("local[2]")
         .config("spark.sql.catalog.ore", "org.apache.iceberg.spark.SparkCatalog")
         .config("spark.sql.catalog.ore.type", "rest")
         .config("spark.sql.catalog.ore.uri", URI)
         .config("spark.sql.catalog.ore.header.x-ore-sujeto", "persona:ana")
         .config("spark.sql.catalog.ore.header.X-Iceberg-Access-Delegation", "vended-credentials")
         .config("spark.sql.catalog.ore.io-impl", "org.apache.iceberg.aws.s3.S3FileIO")
         .config("spark.sql.catalog.ore.s3.path-style-access", "true")
         .config("spark.sql.catalog.ore.client.region", "auto")
         .config("spark.sql.defaultCatalog", "ore")
         .getOrCreate())


def caso(nombre, f):
    try:
        r = f()
    except Exception as e:  # noqa: BLE001
        l = [x.strip() for x in str(e).splitlines() if x.strip() and not x.strip().startswith("at ")]
        r = "FALLA " + (([x for x in l if "Exception" in x] or l or ["?"])[0])[:260]
    print(f"{nombre} · {r}", flush=True)


def v1(t):
    q = urllib.request.Request(f"{URI}/v1/namespaces/geo/tables/{t}", headers={"x-ore-sujeto": "persona:ana"})
    return json.load(urllib.request.urlopen(q))["metadata"]


spark.sql("create namespace if not exists ore.geo")

# a · ¿Sedona toma un `binary` WKB como geometría sin ST_GeomFromWKB? (si no, el arnés convierte)
caso("a · ST_Area sobre binary WKB, sin convertir", lambda: spark.sql(
    "select round(ST_Area(ST_Buffer(ST_AsBinary(ST_Point(0, 0)), 1)), 2) a").first().a)

# b · procedencia en el resumen del snapshot, y propiedades de tabla al crear (DataFrameWriterV2)
df = spark.sql("select id, ST_AsBinary(ST_Buffer(ST_Point(440000 + id, 4474000), 100)) zona from range(3)")
geo = json.dumps({"crs": "EPSG:25830", "type": "Geometry<Polygon, EPSG:25830>"})
proc = json.dumps({"transform": "packages/geo/t.yaml", "commit": "abc"})


def crear():
    (df.writeTo("ore.geo.arnes").using("iceberg")
       .tableProperty("ore.geo.zona", geo)
       .option("snapshot-property.ore.procedencia", proc)
       .createOrReplace())
    m = v1("arnes")
    s = m["snapshots"][-1]["summary"]
    return {"v": m.get("format-version"), "ore.geo.zona": m["properties"].get("ore.geo.zona"),
            "ore.procedencia": s.get("ore.procedencia"), "op": s.get("operation"), "snaps": len(m["snapshots"])}


caso("b · createOrReplace (nueva) con tableProperty y snapshot-property", crear)
caso("c · createOrReplace otra vez (la tabla existe)", crear)


def sobrescribir():
    (df.writeTo("ore.geo.arnes").option("snapshot-property.ore.procedencia", proc).overwrite(F.lit(True)))
    m = v1("arnes")
    s = m["snapshots"][-1]["summary"]
    return {"op": s.get("operation"), "ore.procedencia": bool(s.get("ore.procedencia")),
            "snaps": len(m["snapshots"]), "ore.geo.zona": bool(m["properties"].get("ore.geo.zona"))}


caso("d · overwrite(lit(True)) sobre la existente", sobrescribir)


def anadir():
    df.writeTo("ore.geo.arnes").option("snapshot-property.ore.procedencia", proc).append()
    m = v1("arnes")
    return {"op": m["snapshots"][-1]["summary"].get("operation"), "filas": spark.table("ore.geo.arnes").count()}


caso("e · append", anadir)
caso("f · ALTER TABLE SET TBLPROPERTIES ore.geo.*", lambda: (
    spark.sql("alter table ore.geo.arnes set tblproperties ('ore.geo.zona' = '%s')" % geo.replace("'", "\\'")),
    v1("arnes")["properties"].get("ore.geo.zona"))[1])

# g · ¿manda el cliente REST de Iceberg 1.12 una Idempotency-Key? (en el jar)
def idem():
    j = os.environ["SPARK_HOME"] + "/jars/iceberg-spark-runtime-4.1_2.13-1.12.0.jar"
    z = zipfile.ZipFile(j)
    hits = sorted({n for n in z.namelist() if n.endswith(".class") and b"Idempotency-Key" in z.read(n)})
    return hits[:6] or "ninguna clase la nombra"


caso("g · Idempotency-Key en el cliente REST", idem)

# h · leer: la tabla como DataFrame y su columna geo
caso("h · leer la tabla y convertir con ST_GeomFromWKB + ST_SetSRID", lambda: spark.table("ore.geo.arnes")
     .selectExpr("ST_SRID(ST_SetSRID(ST_GeomFromWKB(zona), 25830)) s", "round(ST_Area(ST_GeomFromWKB(zona))) a").first())
# i · SRID y tipo de una columna Sedona, en una pasada (lo que write() necesita para ore.geo)
caso("i · SRID y tipos de una columna geometry", lambda: spark.sql(
    "select ST_SetSRID(ST_Point(id, id), 25830) g from range(5)").agg(
    F.expr("collect_set(ST_SRID(g))"), F.expr("collect_set(ST_GeometryType(g))")).first())
spark.stop()
