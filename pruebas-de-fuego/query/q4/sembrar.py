"""Query Q4·4 · La escalera de Q0 como datasets del lago de victor (una vez, antes de medir).

Un trabajador de Query sólo lee datasets del lago, con la credencial que `ore-serve` presta para
cada uno: el Parquet crudo del banco que leía Q0 no lo puede leer (y está bien que no). Esto copia
`trip` a 6, 60 y 600 M filas —el mismo corte que Q0, `t_tripkey <= N·10⁶`— y `customer`, con los
lugares tipados como geometría (también sirven para Maps).

⛔ Es un fichero de TRANSFORMS, no una celda: en un repositorio Heavyweight la sesión interactiva es
   ligera (DuckDB) y Spark sólo corre en los Builds. Se pone como `transforms/escalera.py` en un
   repositorio Heavyweight Python de victor, con la base `query` creada, y se lanza un Build por
   salida (las pequeñas primero: si algo falla, falla en segundos). Lo escribe la persona.
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
