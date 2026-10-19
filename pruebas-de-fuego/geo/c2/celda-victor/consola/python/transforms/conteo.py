from pyspark.sql import functions as F

from ore import over, transform, write

# ORE 0068 6c · el join espacial en Python (Heavyweight): over() da DataFrames de Spark con la geometría
# de Sedona y su SRID.


@transform(inputs=["geo.zonas_20k", "geo.puntos_20m"], output="geo.conteo_py")
def conteo():
    z = over("geo.zonas_20k").alias("z")
    p = over("geo.puntos_20m").alias("p")
    return write("geo.conteo_py", z.join(p, F.expr("ST_Contains(z.zona, p.geom)"))
                 .groupBy("z.id").agg(F.count("*").alias("n")))
