// ADR 0067 G2·5 · Los lectores de JVM de la matriz: Spark 4.1 + Iceberg 1.12 (`spark`), Sedona sobre
// su Spark (`sedona`, si está en el classpath) y el lector genérico de Iceberg Java (`iceberg-java`).
// Las mismas líneas `V5R|<lector>|<artefacto>|<json>` que `lectores.py`, para el mismo `juzgar.py`.
// Los artefactos, en la variable ARTEFACTOS: `A=<parquet> B=<raíz>@<versión> C=… X=…`.
import scala.util.Try

object U {
  def hex(b: Array[Byte]) = if (b == null) null else b.map("%02X".format(_)).mkString
  def json(m: Map[String, Any]): String = m.map { case (k, v) => "\"" + k + "\":" + valor(v) }.mkString("{", ",", "}")
  def valor(v: Any): String = v match {
    case null => "null"
    case b: Boolean => b.toString
    case n: Int => n.toString
    case n: Long => n.toString
    case m: Map[_, _] => json(m.asInstanceOf[Map[String, Any]])
    case s => "\"" + s.toString.replace("\\", "\\\\").replace("\"", "\\\"").replaceAll("\\p{Cntrl}", " ") + "\""
  }
  def di(lector: String, art: String, m: Map[String, Any]) = println("V5R|" + lector + "|" + art + "|" + json(m))
  def error(e: Throwable) = Map[String, Any]("error" -> (e.getClass.getSimpleName + ": " + String.valueOf(e.getMessage).take(300)))
  def metadatos(ruta: String) = { val Array(raiz, v) = ruta.split("@"); s"$raiz/metadata/$v.metadata.json" }
}
import U._

val sedona = Try(Class.forName("org.apache.sedona.spark.SedonaContext")).isSuccess
if (sedona) Try(Class.forName("org.apache.sedona.spark.SedonaContext").getMethod("create", classOf[org.apache.spark.sql.SparkSession]).invoke(null, spark))
val arts = sys.env.getOrElse("ARTEFACTOS", "").split(" ").filter(_.contains("=")).map { a => val Array(k, v) = a.split("=", 2); (k, v) }

for ((art, ruta) <- arts) {
  val lector = if (sedona) "sedona" else "spark"
  val r = Try {
    // Sedona lee el fichero con su lector GeoParquet (la clave `geo`); Spark, como Parquet.
    val df = if (art == "A") spark.read.format(if (sedona) "geoparquet" else "parquet").load(ruta)
             else spark.read.format("iceberg").load(metadatos(ruta))
    val t = df.schema("geom").dataType
    val tn = t.typeName
    val geo = tn.startsWith("geometry") || tn.startsWith("geography") || t.getClass.getSimpleName.contains("Geometry")
    val crs = "(?:srid[:(]?|type\\()\\s*(\\d+)".r.findFirstMatchIn(t.toString.toLowerCase).map(m => "EPSG:" + m.group(1)).orNull
    df.createOrReplaceTempView("t")
    val expr = if (geo && !sedona) "st_asbinary(geom)" else if (geo) "ST_AsBinary(geom)" else "geom"
    val filas = spark.sql(s"select cast(id as string) as id, $expr as w from t").collect()
    Map[String, Any]("geo" -> geo, "tipo" -> t.toString, "crs" -> crs,
      "wkb" -> filas.map(f => f.getString(0) -> hex(f.getAs[Array[Byte]](1))).toMap)
  }
  di(lector, art, r.getOrElse(error(r.failed.get)))
  if (art != "A" && !sedona) {
    val j = Try {
      val io = new org.apache.iceberg.hadoop.HadoopFileIO(spark.sparkContext.hadoopConfiguration)
      val tabla = new org.apache.iceberg.BaseTable(new org.apache.iceberg.StaticTableOperations(metadatos(ruta), io), "v5")
      val tipo = tabla.schema().findField("geom").`type`()
      val geo = tipo.typeId().toString.startsWith("GEO")
      val crs = Try(tipo.asInstanceOf[org.apache.iceberg.types.Types.GeometryType].crs()).toOption.orNull
      val it = org.apache.iceberg.data.IcebergGenerics.read(tabla).build().iterator()
      var wkb = Map[String, Any]()
      while (it.hasNext) {
        val x = it.next()
        val b = x.getField("geom").asInstanceOf[java.nio.ByteBuffer]
        wkb += x.getField("id").toString -> (if (b == null) null else { val a = new Array[Byte](b.remaining()); b.duplicate().get(a); hex(a) })
      }
      Map[String, Any]("geo" -> geo, "tipo" -> tipo.toString, "crs" -> crs, "wkb" -> wkb)
    }
    di("iceberg-java", art, j.getOrElse(error(j.failed.get)))
  }
}
System.exit(0)
