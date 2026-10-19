from ore import sql, transform, write

# ORE 0068 6c · los datos de la medida, generados en Spark (Heavyweight): 20 M puntos al azar y 20 k
# polígonos (buffers de 300 m en rejilla) en una caja de 100 km × 100 km de EPSG:25830.


@transform(inputs=[], output="geo.puntos_20m")
def puntos():
    return write("geo.puntos_20m", sql(
        "select id, ST_SetSRID(ST_Point(400000 + rand(1) * 100000, 4430000 + rand(2) * 100000), 25830) geom "
        "from range(20000000)"))


@transform(inputs=[], output="geo.zonas_20k")
def zonas():
    return write("geo.zonas_20k", sql(
        "select id, ST_SetSRID(ST_Buffer(ST_Point(400000 + (id % 141) * (100000 / 141), "
        "4430000 + floor(id / 141) * (100000 / 141)), 300), 25830) zona from range(20000)"))
