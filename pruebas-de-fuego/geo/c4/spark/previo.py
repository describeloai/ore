"""ORE 0068 C4·3 · El previo de Spark de Enrich en victor, con el Build elástico (c3/d/medir.sh, `PREVIO`):
  bash pruebas-de-fuego/geo/c3/d/medir.sh MAX=2 EXPERIMENTOS=C4 LIMITE_S=1800 \
    PREVIO="lentas.py ../../c4/spark/previo.py" C4_CLIENTES=1000000,10000000
(`lentas.py` va delante por su `medir()`; con `EXPERIMENTOS=C4`, sus E1–E4 no corren.)

Las capas sintéticas del previo de C4·2 (España en EPSG:25830: 8 000 municipios de ~500 vértices, 36 000
secciones, 100 000 competidores) y N clientes, generadas aquí; la sentencia es la que ORE escribe para el
árbol del cotejo (`cotejo/arbol`, la prueba `escribe_el_sql_de_un_arbol`, a `enrich-spark.sql`), con sus
nombres cambiados por estas vistas. Una línea `C3D|C4-…` por medida: lo que tarda la sentencia entera,
contada columna a columna para que se calcule todo.
"""
from pyspark import StorageLevel

SQL = """with t as (select *, monotonically_increasing_id() __ore_k from clientes),
p1 as (select t.__ore_k, l.`nombre` __ore_c_1 from t join municipios l on ST_Within(t.`ubicacion`, l.`geom`)),
c1 as (select __ore_k, min(__ore_c_1) `municipio` from p1 group by 1),
p2 as (select t.__ore_k, l.`ventas` __ore_c_1 from t join competidores l on ST_DWithin(t.`ubicacion`, l.`geom`, 500)),
c2 as (select __ore_k, count(*) `competidores_500m`, sum(__ore_c_1) `ventas_500m` from p2 group by 1),
p3 as (select t.__ore_k, l.`nombre` __ore_c_1, ST_Distance(t.`ubicacion`, l.`geom`) __ore_d from t join competidores l on ST_DWithin(t.`ubicacion`, l.`geom`, 5000)),
c3 as (select __ore_k, min_by(__ore_c_1, __ore_d) `competidor_cercano`, min(__ore_d) `dist_competidor` from p3 group by 1),
p4 as (select *, sum(__ore_a) over (partition by __ore_k, __ore_c_3) __ore_w_3 from (select t.__ore_k, l.`poblacion` __ore_c_1, l.`renta_media` __ore_c_2, l.`uso` __ore_c_3, l.`id` __ore_c_4, ST_Area(ST_Intersection(t.__ore_b, l.`geom`)) __ore_a, ST_Area(l.`geom`) __ore_al from (select __ore_k, ST_Buffer(`ubicacion`, 500) __ore_b from t) t join secciones l on ST_Intersects(t.__ore_b, l.`geom`)) q),
c4 as (select __ore_k, sum(__ore_c_1 * __ore_a / nullif(__ore_al, 0)) `poblacion_500m`, sum(__ore_c_2 * __ore_a) / nullif(sum(__ore_a), 0) `renta_500m`, min_by(__ore_c_3, named_struct('w', -__ore_w_3, 'v', __ore_c_3)) `uso_500m`, min_by(__ore_c_4, named_struct('w', -__ore_a, 'v', __ore_c_4)) `seccion` from p4 group by 1)
select t.`cliente_id`, t.`ubicacion`, c1.`municipio`, coalesce(c2.`competidores_500m`, 0) `competidores_500m`, c2.`ventas_500m`, c3.`competidor_cercano`, c3.`dist_competidor`, c4.`poblacion_500m`, c4.`renta_500m`, c4.`uso_500m`, c4.`seccion`
from t left join c1 using (__ore_k) left join c2 using (__ore_k) left join c3 using (__ore_k) left join c4 using (__ore_k)"""
for nombre, q in (
    ("municipios", """select concat('m', id) nombre, cast(rand(1) * 50000 as int) poblacion,
        ST_SetSRID(ST_Buffer(ST_Point((id % 90) * 6700 + rand(2) * 2000, floor(id / 90) * 6700 + rand(3) * 2000),
                             4200, false, 'quad_segs=125'), 25830) geom from range(8000)"""),
    ("secciones", """select concat('s', id) id, cast(rand(4) * 2000 as int) poblacion, rand(5) * 30000 renta_media,
        element_at(array('residencial', 'comercial', 'industrial', 'verde'), 1 + cast(rand(6) * 3.999 as int)) uso,
        ST_SetSRID(ST_Buffer(ST_Point((id % 190) * 3160 + rand(7) * 500, floor(id / 190) * 3160 + rand(8) * 500),
                             1900, false, 'quad_segs=16'), 25830) geom from range(36000)"""),
    ("competidores", """select concat('c', id) nombre, cast(rand(9) * 1e6 as int) ventas,
        ST_SetSRID(ST_Point(rand(10) * 600000, rand(11) * 600000), 25830) geom from range(100000)"""),
):
    s.sql(q).persist(StorageLevel.MEMORY_AND_DISK).createOrReplaceTempView("_" + nombre)  # noqa: F821
    linea("C4-capa", nombre=nombre, filas=s.table("_" + nombre).count())  # noqa: F821
CAPAS = ("municipios", "secciones", "competidores")

CUENTA = """select count(*) filas, count(municipio) municipio, sum(competidores_500m) competidores,
  count(competidor_cercano) cercano, sum(poblacion_500m) poblacion, count(uso_500m) uso, count(seccion) seccion
  from (%s)""" % SQL
# C4·3b · las variantes: `v0`, cada capa como sale (Spark decide si la difunde); `v1`, todas con
# `hint("broadcast")`, lo que haría un umbral de difusión mayor para las capas de Enrich.
VARIANTES = os.environ.get("C4_VARIANTES", "v0,v1").split(",")  # noqa: F821


def poner(variante):
    for c in CAPAS:
        df = s.table("_" + c)  # noqa: F821
        (df.hint("broadcast") if variante == "v1" else df).createOrReplaceTempView(c)


def joins_del_plan(sql):
    """Cada join espacial del plan con su predicado: dice qué capa cae en cuál."""
    p = s.sql(sql)._jdf.queryExecution().executedPlan().toString()  # noqa: F821
    for l in p.splitlines():
        for j in ("RangeJoin", "BroadcastIndexJoin", "DistanceJoin", "BroadcastNestedLoopJoin", "CartesianProduct"):
            if j in l:
                pred = l.strip().split(j, 1)[1][:160].replace(" ", "_")
                linea("C4-join", join=j, pred=pred)  # noqa: F821


for n in [int(x) for x in os.environ.get("C4_CLIENTES", "1000000,10000000").split(",")]:  # noqa: F821
    t = time.time()  # noqa: F821
    s.sql("""select concat('k', id) cliente_id, ST_SetSRID(ST_Point(rand(12) * 600000, rand(13) * 600000), 25830)
             ubicacion from range(%d)""" % n).repartition(64).persist(StorageLevel.DISK_ONLY) \
        .createOrReplaceTempView("clientes")  # noqa: F821
    linea("C4-clientes", n=n, filas=s.table("clientes").count(), s="%.1f" % (time.time() - t))  # noqa: F821
    for v in VARIANTES:
        poner(v)
        linea("C4-variante", v=v, n=n)  # noqa: F821
        joins_del_plan(CUENTA)
        medir("C4-entera-%s-%d" % (v, n), CUENTA)  # noqa: F821
    s.catalog.dropTempView("clientes")  # noqa: F821
