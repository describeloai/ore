// G2·0 (ADR 0067): el paso de `binary` a `geometry` en Iceberg v3, medido con Iceberg Java 1.11 y Spark 4.1.
import org.apache.iceberg.{DataFiles, FileFormat, HasTableOperations, PartitionSpec, Schema, TableMetadata}
import org.apache.iceberg.catalog.{Namespace, TableIdentifier}
import org.apache.iceberg.data.IcebergGenerics
import org.apache.iceberg.types.Types
import scala.jdk.CollectionConverters._
import scala.util.{Try, Success, Failure}

val sesion = org.apache.spark.sql.SparkSession.active
val cat = sesion.sessionState.catalogManager.catalog("lab").asInstanceOf[org.apache.iceberg.spark.SparkCatalog].icebergCatalog()
def r(que: String)(f: => Any): Unit = Try(f) match {
  case Success(v) => println(s"RESULTADO | $que | bien | $v")
  case Failure(e) => println(s"RESULTADO | $que | falla | ${e.getClass.getSimpleName}: ${Option(e.getMessage).getOrElse("").split("\n").head.take(220)}")
}
val esquemaBin = new Schema(
  Types.NestedField.optional(1, "id", Types.LongType.get()),
  Types.NestedField.optional(2, "geom", Types.BinaryType.get()))
val esquemaGeo = new Schema(
  Types.NestedField.optional(1, "id", Types.LongType.get()),
  Types.NestedField.optional(2, "geom", Types.GeometryType.crs84()))

for ((nombre, fichero, version) <- Seq(("nativo_v2", "geo-nativo", "2"), ("nativo_v3", "geo-nativo", "3"), ("plano_v3", "geo-plano", "3"))) {
  val id = TableIdentifier.of("g20", nombre)
  if (cat.tableExists(id)) cat.dropTable(id, true)
  val t = cat.createTable(id, esquemaBin, PartitionSpec.unpartitioned(), Map("format-version" -> version).asJava)
  val ruta = s"/salida/$fichero.parquet"
  val df = DataFiles.builder(PartitionSpec.unpartitioned()).withPath(ruta).withFormat(FileFormat.PARQUET)
    .withFileSizeInBytes(new java.io.File(ruta).length()).withRecordCount(1000).build()
  t.newAppend().appendFile(df).commit()
  val s1 = t.currentSnapshot().snapshotId()
  println(s"TABLA | $nombre | ${t.asInstanceOf[HasTableOperations].operations().current().metadataFileLocation()}")
  // A · hoy: la columna es `binary` y el fichero, GEOMETRY nativo (o plano).
  r(s"$nombre · A · Java genérico lee binary") { IcebergGenerics.read(t).build().asScala.size }
  r(s"$nombre · A · Spark SQL lee binary") { sesion.sql(s"select count(*), max(length(geom)) from lab.g20.$nombre").collect().mkString }
  // B · el paso por la API: ¿es una promoción válida?
  r(s"$nombre · B · updateColumn a geometry por la API") { t.updateSchema().updateColumn("geom", Types.GeometryType.crs84()).commit(); "aceptado" }
  // B · el paso sólo en metadatos: un esquema nuevo con el mismo id de campo.
  r(s"$nombre · B · esquema nuevo solo en metadatos") {
    val ops = t.asInstanceOf[HasTableOperations].operations()
    val base = ops.refresh()
    val nueva = TableMetadata.buildFrom(base).setCurrentSchema(esquemaGeo, base.lastColumnId()).build()
    ops.commit(base, nueva)
    ops.refresh().schema().toString.replace("\n", " ")
  }
  t.refresh()
  r(s"$nombre · C · Java genérico lee geometry (actual)") { IcebergGenerics.read(t).build().asScala.take(1).map(_.getField("geom").getClass.getSimpleName).mkString }
  r(s"$nombre · C · Spark SQL lee geometry (actual)") { sesion.sql(s"select count(*) from lab.g20.$nombre").collect().mkString }
  r(s"$nombre · C · Spark SQL, el snapshot de antes del paso") { sesion.sql(s"select count(*) from lab.g20.$nombre VERSION AS OF $s1").collect().mkString }
  r(s"$nombre · C · Java genérico, el snapshot de antes") { IcebergGenerics.read(t).useSnapshot(s1).build().asScala.size }
  println(s"TABLA2 | $nombre | ${t.asInstanceOf[HasTableOperations].operations().current().metadataFileLocation()}")
}
System.exit(0)
