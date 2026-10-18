"""ORE 0068 C2·2 pieza 0 · La sonda del prototipo: corre DENTRO de la imagen, con `--network none`.
Una línea `OK|FALLA · caso · detalle` por caso; al final, el resumen. Nada se descarga: si un jar
falta, lo dice el caso que lo necesita.
    docker run --rm --network none ore-puesto-spark:previo python /sonda.py
"""
import os, platform, subprocess, sys, time

casos = []


def caso(nombre, f):
    t = time.time()
    try:
        d = f()
        casos.append((True, nombre, f"{d} · {time.time() - t:.1f} s"))
    except Exception as e:  # el previo quiere el error entero, no un traceback
        # de un error de la JVM, la causa más honda («Caused by»), no la primera línea
        ls = [l.strip() for l in str(e).splitlines() if l.strip() and not l.strip().startswith("at ")]
        causa = ([l for l in ls if "Caused by" in l or "Exception" in l] or ls or ["?"])[-1]
        casos.append((False, nombre, f"{type(e).__name__}: {causa[:400]}"))
    print(("OK    · " if casos[-1][0] else "FALLA · ") + nombre + " · " + casos[-1][2], flush=True)


caso("python", lambda: platform.python_version())
caso("java", lambda: subprocess.run(["java", "-version"], capture_output=True, text=True).stderr.splitlines()[0])

import pyspark  # noqa: E402

caso("pyspark", lambda: pyspark.__version__)
jars = open("/spark-jars.txt").read().strip()
caso("jars", lambda: f"{len(os.listdir(jars))} jars, {sum(os.path.getsize(os.path.join(jars, j)) for j in os.listdir(jars)) // 2**20} MiB")

from pyspark.sql import SparkSession  # noqa: E402

spark = None


def sesion():
    global spark
    spark = (SparkSession.builder.master("local[*]").appName("previo")
             .config("spark.ui.enabled", "false")
             .config("spark.sql.extensions", "org.apache.iceberg.spark.extensions.IcebergSparkSessionExtensions,"
                                             "org.apache.sedona.sql.SedonaSqlExtensions")
             .config("spark.serializer", "org.apache.spark.serializer.KryoSerializer")
             .config("spark.kryo.registrator", "org.apache.sedona.core.serde.SedonaKryoRegistrator")
             # ⚠️ Medido (pieza 0): sin esto `ST_Transform` (proj4sedona) baja la definición EPSG de
             #   spatialreference.org; con `all`, GeoTools la lleva dentro. `true` y `vector` no bastan.
             .config("spark.sedona.crs.geotools", "all")
             .config("spark.sql.execution.arrow.pyspark.enabled", "true")
             .config("spark.sql.catalog.ore", "org.apache.iceberg.spark.SparkCatalog")
             .config("spark.sql.catalog.ore.type", "rest")
             .config("spark.sql.catalog.ore.uri", "http://127.0.0.1:1")
             .getOrCreate())
    return spark.version


caso("sesión local con Iceberg y Sedona (Java 21)", sesion)


def clase(n):
    return lambda: spark._jvm.java.lang.Class.forName(n).getName()


for n in ("org.apache.iceberg.gcp.gcs.GCSFileIO", "org.apache.iceberg.aws.s3.S3FileIO",
          "org.apache.iceberg.rest.RESTCatalog", "org.apache.sedona.sql.SedonaSqlExtensions"):
    caso("clase " + n.rsplit(".", 1)[1], clase(n))

# El área del laboratorio: un buffer de 100 m en EPSG:25830, 32 lados → 31 214 m² (DuckDB y Sedona, C2·4).
caso("Sedona SQL · ST_Buffer", lambda: spark.sql(
    "select round(ST_Area(ST_Buffer(ST_Point(440000, 4474000), 100))) a").first().a)
# La base EPSG de geotools tiene que ir dentro: sin red, ¿transforma?
caso("Sedona SQL · ST_Transform 25830→4326 sin red", lambda: spark.sql(
    "select ST_AsText(ST_Transform(ST_Point(440000, 4474000), 'EPSG:25830', 'EPSG:4326')) t").first().t)


def sedona_py():
    from sedona.spark import SedonaContext
    s = SedonaContext.create(spark)
    from shapely.geometry import Point
    from sedona.spark import GeometryType
    from pyspark.sql.types import LongType, StructField, StructType
    esquema = StructType([StructField("id", LongType()), StructField("g", GeometryType())])
    df = s.createDataFrame([(1, Point(440000, 4474000))], esquema)
    return df.selectExpr("round(ST_Area(ST_Buffer(g, 100))) a").first().a


caso("Sedona Python · DataFrame con shapely", sedona_py)


def arrow():
    t = spark.sql("select id, ST_AsBinary(ST_Buffer(ST_Point(id, id), 1)) wkb from range(1000)").toPandas()
    return f"{len(t)} filas, wkb {type(t.wkb[0]).__name__}"


caso("PySpark → pandas por Arrow", arrow)
caso("Spark Java · javac", lambda: subprocess.run(["javac", "-version"], capture_output=True, text=True).stdout.strip())


def cli():
    b = os.path.join(os.path.dirname(pyspark.__file__), "bin", "spark-sql")
    r = subprocess.run([b, "-S", "-e", "select 1 + 1"], capture_output=True, text=True, timeout=180)
    if r.returncode:
        raise RuntimeError((r.stderr.strip().splitlines() or ["?"])[-1])
    return r.stdout.strip()


caso("spark-sql (la CLI)", cli)

spark and spark.stop()
print(f"\n{sum(c[0] for c in casos)}/{len(casos)} casos")
sys.exit(0 if all(c[0] for c in casos) else 1)
