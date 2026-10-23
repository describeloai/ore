"""Query · La escalera de Q0 como datasets del lago de victor (una vez, antes de medir).

Copia `trip` a 6, 60 y 600 M filas —el mismo corte que Q0, `t_tripkey <= N·10⁶`— y `customer`, con
los lugares tipados como geometría (también sirven para Maps). Lo que se mida de Query lee estos
datasets por su nombre, gobernados, y no el Parquet crudo del banco.

⛔ Es un fichero de TRANSFORMS, no una celda: se pone en `transforms/` de un repositorio Heavyweight
   Python de victor, con la base `query` creada, y se lanza un Build por salida (las pequeñas
   primero: si algo falla, falla en segundos). Lo escribe la persona.
"""
from pyspark.sql import functions as F

from ore import transform, write
from ore import spark as ore_spark

BANCO = "gs://project-8853a180-450d-47be-b83-banco-geo/spatialbench/sf100/"


def _trip(millones):
    # Las dos columnas de lugar vienen de SpatialBench como WKB sin tipo: geometría de Sedona con su
    # CRS (lon/lat, EPSG:4326), y `write()` las guarda tipadas (`ore.geo.<col>`) con su cobertura
    # `<col>_bbox`. `withColumn` con el mismo nombre la sustituye en su sitio.
    df = ore_spark.sesion().read.parquet(BANCO + "trip.parquet").where("t_tripkey <= %d" % (millones * 1_000_000))
    for c in ("t_pickuploc", "t_dropoffloc"):
        df = df.withColumn(c, F.expr("ST_SetSRID(ST_GeomFromWKB(%s), 4326)" % c))
    return df


@transform(inputs=[], output="query.trip_6m")
def trip_6m():
    return write("query.trip_6m", _trip(6))


@transform(inputs=[], output="query.trip_60m")
def trip_60m():
    return write("query.trip_60m", _trip(60))


@transform(inputs=[], output="query.trip_600m")
def trip_600m():
    return write("query.trip_600m", _trip(600))


@transform(inputs=[], output="query.customer")
def customer():
    return write("query.customer", ore_spark.sesion().read.parquet(BANCO + "customer.parquet"))
