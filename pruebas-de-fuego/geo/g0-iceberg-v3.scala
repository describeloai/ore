// g0-iceberg-v3.scala (ADR 0067 G0·2): la tabla Iceberg v3 AJENA, con la API de Iceberg Java 1.11
// directamente (Spark 4.1 + Iceberg 1.11 aún no traducen GEOMETRY de Spark: «Not a supported type»).
// Esquema con `Types.GeometryType` (CRS84), datos con el escritor genérico de Parquet de Iceberg.
import org.apache.iceberg.{HasTableOperations, PartitionSpec, Schema}
import org.apache.iceberg.catalog.TableIdentifier
import org.apache.iceberg.data.{GenericRecord, Record}
import org.apache.iceberg.data.parquet.GenericParquetWriter
import org.apache.iceberg.parquet.Parquet
import org.apache.iceberg.types.Types
import scala.jdk.CollectionConverters._

val sesion = org.apache.spark.sql.SparkSession.active
val cat = sesion.sessionState.catalogManager.catalog("banco").asInstanceOf[org.apache.iceberg.spark.SparkCatalog].icebergCatalog()
val id = TableIdentifier.of("ajena", "lugares_madrid")
if (cat.tableExists(id)) cat.dropTable(id, true)
val esquema = new Schema(
  Types.NestedField.optional(1, "id", Types.LongType.get()),
  Types.NestedField.optional(2, "nombre", Types.StringType.get()),
  Types.NestedField.optional(3, "categoria", Types.StringType.get()),
  Types.NestedField.optional(4, "geom", Types.GeometryType.crs84()))
val tabla = cat.createTable(id, esquema, PartitionSpec.unpartitioned(), Map("format-version" -> "3").asJava)
val filas = sesion.read.parquet("/salida/lug_wkb.parquet").collect()
val salida = tabla.io().newOutputFile(tabla.locationProvider().newDataLocation("lugares-00000.parquet"))
val w = Parquet.writeData(salida).schema(esquema).createWriterFunc(GenericParquetWriter.create _).overwrite().withSpec(PartitionSpec.unpartitioned()).build[Record]()
val r = GenericRecord.create(esquema)
filas.foreach { f =>
  w.write(r.copy(Map[String, Object](
    "id" -> java.lang.Long.valueOf(f.getLong(0)), "nombre" -> f.getString(1), "categoria" -> f.getString(2),
    "geom" -> java.nio.ByteBuffer.wrap(f.getAs[Array[Byte]](3))).asJava))
}
w.close()
tabla.newAppend().appendFile(w.toDataFile()).commit()
val t2 = cat.loadTable(id)
println(s"RESULTADO filas=${filas.length} version=${t2.asInstanceOf[HasTableOperations].operations().current().formatVersion()} esquema=${t2.schema()} metadatos=${t2.asInstanceOf[HasTableOperations].operations().current().metadataFileLocation()}")
System.exit(0)
