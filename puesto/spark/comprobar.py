"""ORE 0068 C2·2 · La construcción de `puesto-spark` lo demuestra: una sesión SOLO con lo que la imagen
pone (`spark-defaults.conf`, ningún `.config` aquí) arranca como el uid de los puestos, ve Iceberg con
sus dos FileIO y Sedona, y da el área de la pieza 0. Un fallo tumba la construcción.
(Sin red lo prueba `pruebas-de-fuego/geo/c2/puesto-spark/en-la-imagen.sh`: aquí hay red.)
"""
import os
import sys

from pyspark.sql import SparkSession

s = SparkSession.builder.master("local[1]").appName("comprobar").getOrCreate()
try:
    assert s.conf.get("spark.sedona.crs.geotools") == "all", "spark-defaults.conf no se lee"
    for c in ("org.apache.iceberg.spark.SparkCatalog", "org.apache.iceberg.rest.RESTCatalog",
              "org.apache.iceberg.gcp.gcs.GCSFileIO", "org.apache.iceberg.aws.s3.S3FileIO",
              "org.apache.sedona.sql.SedonaSqlExtensions",
              # C2·3: la identidad del puesto en cada petición, compilada aquí.
              "ore.spark.IdentidadDelPuesto",
              # C2·3 pieza 4: Spark Java (el SDK, el arnés y la anotación de siempre).
              "ore.Ore", "ore.ArnesDeSpark", "ore.Transform", "ore.Puente"):
        s._jvm.java.lang.Class.forName(c)
    a = s.sql("select round(ST_Area(ST_Buffer(ST_Point(440000, 4474000), 100))) a").first().a
    assert a == 31214, a
    import sedona.spark  # noqa: F401  (Sedona de Python, contra el shapely del puesto)
    print(f"puesto-spark · Spark {s.version} · Java {s._jvm.java.lang.System.getProperty('java.version')} · "
          f"Python {sys.version.split()[0]} · uid {os.getuid()} · ST_Buffer {a:.0f} m²")
finally:
    s.stop()
