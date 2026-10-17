// G2·4·0 (c): la tabla de ORE (v3) antes (binary) y después de promoverla (geometry), leída por
// Spark 4.1 + Iceberg 1.12 y por el lector genérico de Iceberg Java 1.12.
def r(s: String) = println("RESULTADO " + s)
def intenta(n: String)(f: => Any) = try { r(n + " · ok · " + f) } catch { case e: Throwable => r(n + " · FALLA · " + e.getClass.getSimpleName + ": " + String.valueOf(e.getMessage).take(240).replace("\n", " ")) }
val dir = new java.io.File("/g/ore/tabla/ore/v2/copias/vara_v3/metadata")
for (m <- dir.listFiles.map(_.getName).filter(_.endsWith(".metadata.json")).sorted) {
  val p = "file:///g/ore/tabla/ore/v2/copias/vara_v3/metadata/" + m
  val q = m.take(5)
  intenta(q + " spark esquema") { spark.read.format("iceberg").load(p).schema("geom").dataType.toString }
  intenta(q + " spark filas") { spark.read.format("iceberg").load(p).count() }
  intenta(q + " spark st_astext") { spark.read.format("iceberg").load(p).selectExpr("st_astext(geom)").head().getString(0) }
  intenta(q + " java genérico") {
    val io = new org.apache.iceberg.hadoop.HadoopFileIO(new org.apache.hadoop.conf.Configuration())
    val t = new org.apache.iceberg.StaticTableOperations(p, io)
    val tabla = new org.apache.iceberg.BaseTable(t, "ore")
    val it = org.apache.iceberg.data.IcebergGenerics.read(tabla).build().iterator()
    var n = 0; var primero: Any = null
    while (it.hasNext) { val x = it.next(); if (n == 0) primero = x.getField("geom"); n += 1 }
    s"$n filas, geom ${if (primero == null) "null" else primero.getClass.getSimpleName}"
  }
}
System.exit(0)
