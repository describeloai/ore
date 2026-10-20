"""ORE 0068 C3 · El previo de Q10–Q12, segunda ronda (lo ejecuta `medir.py`: `PREVIO=lentas2.py`).

La primera (`lentas.py`) dijo: el join de Q10/Q11 se ahoga en los polígonos enormes de `zone` y,
subdivididos, va rápido, pero subdividir con 41 tareas tardó 9 min; Q12 hace su KNN en 4 tareas. Aquí:
  R0 · qué configuración de Sedona tiene de verdad la sesión (`spark-defaults.conf` descarta las claves
       que no empiezan por `spark.`: ¿está la rejilla de C3·b2?);
  R1 · subdividir `zone` UNA vez, repartida antes (`repartition`), y guardarla (`persist`, disco local);
  R2 · el join de dentro de Q10/Q11 con las piezas, sin repetir (trip, zona);
  R3 · Q11 entera con las piezas;
  R4 · Q12 con más particiones para el KNN;
  R5 · el join de E2 sin subdividir, pero con la rejilla bien puesta (separa los dos efectos).
Usa `medir()` y `etapas()` de `lentas.py`, que va delante en el mismo fichero (medir.sh los junta).
"""
from pyspark import StorageLevel

for k in ("sedona.join.spatitionside", "spark.sedona.join.spatitionside", "sedona.join.numpartition",
          "spark.sedona.join.numpartition", "sedona.join.indexbuildside", "sedona.global.index",
          "sedona.join.gridtype", "spark.sql.shuffle.partitions"):
    linea("R0-conf", clave=k, valor=str(s.conf.get(k, "-")).replace(" ", "_"))  # noqa: F821

# La rejilla de C3·b2, en la sesión: en `spark-defaults.conf` como `sedona.join.spatitionside` Spark la
# descarta (no empieza por `spark.`) y la imagen corre con la de Sedona por defecto, sobre la izquierda.
for k in ("sedona.join.spatitionside", "spark.sedona.join.spatitionside"):
    s.conf.set(k, os.environ.get("REJILLA", "right"))  # noqa: F821
linea("R0-rejilla", valor=s.conf.get("sedona.join.spatitionside"))  # noqa: F821

t = time.time()  # noqa: F821
piezas = (s.table("zone").repartition(int(os.environ.get("REPARTO", "112")))  # noqa: F821
          .selectExpr("z_zonekey", "ST_SubDivideExplode(ST_GeomFromWKB(z_boundary), %d) g" % PIEZAS)  # noqa: F821
          .persist(StorageLevel.DISK_ONLY))
n = piezas.count()
piezas.createOrReplaceTempView("piezas")
linea("R1-subdividir", s="%.1f" % (time.time() - t), piezas=n)  # noqa: F821

medir("R2-join", """
  select count(*) pares from (
    select distinct t.t_tripkey, p.z_zonekey from trip t, piezas p
    where ST_Within(ST_GeomFromWKB(t.t_pickuploc), p.g))""")
medir("R3-q11", """
  select count(*) cruzan from (
    select distinct t.t_tripkey, a.z_zonekey za, b.z_zonekey zb from trip t
      join piezas a on ST_Within(ST_GeomFromWKB(t.t_pickuploc), a.g)
      join piezas b on ST_Within(ST_GeomFromWKB(t.t_dropoffloc), b.g)
    where a.z_zonekey != b.z_zonekey)""")
medir("R5-sin-subdividir", """
  select count(*) pares from trip t, zone z
  where ST_Within(ST_GeomFromWKB(t.t_pickuploc), ST_GeomFromWKB(z.z_boundary))""")
for k in ("sedona.join.numpartition", "spark.sedona.join.numpartition"):
    s.conf.set(k, os.environ.get("KNN_PARTICIONES", "112"))  # noqa: F821
medir("R4-q12", consultas["q12"])  # noqa: F821
