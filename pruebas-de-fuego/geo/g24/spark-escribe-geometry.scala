// G2·4·0 (c): Spark 4.1 + Iceberg 1.12, catálogo hadoop local. ¿Escribe filas geometry en v3? ¿Las lee?
def r(s: String) = println("RESULTADO " + s)
def intenta(n: String)(f: => Any) = try { r(n + " · ok · " + f) } catch { case e: Throwable => r(n + " · FALLA · " + e.getClass.getSimpleName + ": " + String.valueOf(e.getMessage).take(220).replace("\n", " ")) }
for (t <- Seq("geometry(4326)", "geometry(OGC:CRS84)", "geometry(any)")) intenta("crear v3 con " + t) { spark.sql(s"create table if not exists l.d.t (id bigint, geom $t) using iceberg tblproperties ('format-version'='3')"); "creada" }
intenta("tipo de Spark para geom") { spark.table("l.d.t").schema("geom").dataType.toString }
intenta("insert con ST_GeomFromWKB") { spark.sql("insert into l.d.t select 1, st_setsrid(st_geomfromwkb(X'0101000000000000000000F03F0000000000000040'), 4326)"); "insertada" }
intenta("insert de binario") { spark.sql("insert into l.d.t select 2, X'0101000000000000000000F03F0000000000000040'"); "insertada" }
intenta("leer") { spark.sql("select id, typeof(geom), hex(st_asbinary(geom)) from l.d.t").collect().mkString(" ") }
intenta("leer sin funciones") { spark.table("l.d.t").collect().mkString(" ") }
intenta("ficheros") { spark.sql("select file_path from l.d.t.files").collect().mkString(" ") }
System.exit(0)
