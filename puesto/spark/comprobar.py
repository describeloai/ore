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
              "ore.Ore", "ore.ArnesDeSpark", "ore.Transform", "ore.Puente",
              # C3·b: las reglas de ORE (spark.sql.extensions).
              "ore.spark.ReglasDeOre"):
        s._jvm.java.lang.Class.forName(c)
    a = s.sql("select round(ST_Area(ST_Buffer(ST_Point(440000, 4474000), 100))) a").first().a
    assert a == 31214, a
    # C3·b: la regla está puesta, no sólo compilada: un LEFT JOIN espacial sale reescrito.
    q = s.sql("select * from (select ST_Point(0, 0) g) a left join (select ST_Point(0, 0) h) b on ST_Intersects(a.g, b.h)")
    assert "__ore_k" in q._jdf.queryExecution().analyzed().toString(), "ReglasDeOre no está en spark.sql.extensions"
    assert q.count() == 1
    # C3·b2: la rejilla de Sedona, sobre el lado grande. Lo que vale es lo que VE Sedona: escrita sin el
    # prefijo `spark.`, Spark la descartaba del fichero y la imagen corrió con LEFT (2026-10-20).
    lado = s._jvm.org.apache.sedona.core.utils.SedonaConf.fromActiveSession().getJoinSpartitionDominantSide()
    assert str(lado) == "RIGHT", f"la rejilla de Sedona va sobre {lado}, no sobre el lado grande (RIGHT)"
    # C3·e: la geometría de una subconsulta escalar se difunde (DifundirLaSubconsulta).
    q2 = s.sql("select count(*) n from (select ST_Point(0, 0) g) a "
               "where ST_Intersects(a.g, (select ST_Buffer(ST_Point(0, 0), 1) limit 1))")
    assert "Join" in q2._jdf.queryExecution().optimizedPlan().toString(), "DifundirLaSubconsulta no reescribe"
    assert q2.first().n == 1
    import sedona.spark  # noqa: F401  (Sedona de Python, contra el shapely del puesto)
    print(f"puesto-spark · Spark {s.version} · Java {s._jvm.java.lang.System.getProperty('java.version')} · "
          f"Python {sys.version.split()[0]} · uid {os.getuid()} · ST_Buffer {a:.0f} m²")
finally:
    s.stop()
