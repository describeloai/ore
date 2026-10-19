package ore;

import java.util.Map;
import org.apache.spark.sql.Dataset;
import org.apache.spark.sql.Row;
import org.apache.spark.sql.SparkSession;

/**
 * <b>The ORE SDK for Java in a Heavyweight repository</b> (ORE 0068 C2·3): the same names as in
 * Lightweight —{@code import static ore.Ore.*}, {@code over}, {@code sql}, {@code write}— on Spark.
 * {@code over()} and {@code sql()} return a {@code Dataset<Row>}, with geometry columns as Sedona
 * geometries in their CRS; {@code write()} writes one, with its CRS and provenance.
 *
 * <pre>{@code
 * import static ore.Ore.*;
 *
 * import ore.Transform;
 * import org.apache.spark.sql.Dataset;
 * import org.apache.spark.sql.Row;
 *
 * public class Zones {
 *     @Transform(inputs = {"geo.places"}, output = "geo.zones")
 *     public static Object zones() throws Exception {
 *         Dataset<Row> places = over("geo.places");
 *         return write("geo.zones", places.selectExpr("id", "ST_Buffer(geom, 100) zone"));
 *     }
 * }
 * }</pre>
 *
 * <p>⭐ This class only forwards: what ORE adds to a read or a write lives in the Python SDK of the
 * same job ({@code ore/spark.py}), which runs Spark and calls this code. Both engines give the
 * same answer because there is one implementation.
 */
public final class Ore {
    private Ore() {}

    /** The SDK API this code is written against (the same number as Lightweight). */
    public static final int API = 2;

    static volatile Puente puente;
    /** El fichero del build mientras su clase carga (D15), o {@code null}. */
    static volatile String cargando;

    /** El SDK de Python lo dice al empezar el build. Interno. */
    public static void puente(Puente p) { puente = p; }

    /** A Transform's read or write was called while the build loads its class (0055 D15). */
    public static final class TransformCalledWhileLoading extends IllegalStateException {
        private final Integer linea;

        TransformCalledWhileLoading(String mensaje, Integer linea) {
            super(mensaje);
            this.linea = linea;
        }

        /** @return the line of the build's file that called it, or {@code null} */
        public Integer line() { return linea; }
    }

    private static Puente puente(String que) {
        String f = cargando;
        if (f != null) {
            throw new TransformCalledWhileLoading(que + "() was called while the build loads the class: "
                + "the build calls the @Transform method itself, once", ArnesDeSpark.lineaEn(new Throwable(), f));
        }
        Puente p = puente;
        if (p == null) throw new IllegalStateException("ore.Ore: this JVM is not a Heavyweight build of ORE");
        return p;
    }

    /** @return the build's Spark session */
    public static SparkSession spark() { return SparkSession.active(); }

    /**
     * Read a dataset or view of the tree.
     *
     * @param view {@code <database>.<schema>.<name>} (or {@code <database>.<name>}, in {@code default})
     * @return a {@code Dataset<Row>}; geometry columns are Sedona geometries with their SRID
     */
    @SuppressWarnings("unchecked")
    public static Dataset<Row> over(String view) { return (Dataset<Row>) puente("over").over(view); }

    /**
     * Spark SQL over the tree: names of the tree read like {@link #over}.
     *
     * @param query the statement
     * @return its result
     */
    @SuppressWarnings("unchecked")
    public static Dataset<Row> sql(String query) { return (Dataset<Row>) puente("sql").sql(query); }

    /** {@link #write(String, Dataset, String)} with {@code overwrite}. */
    public static Map<String, Object> write(String name, Dataset<Row> data) { return write(name, data, "overwrite"); }

    /**
     * Write {@code data} as the dataset {@code name}: geometry columns go as WKB with their CRS
     * ({@code ore.geo.<col>}, one CRS per column), with the provenance of the build.
     *
     * @param name {@code <database>.<schema>.<name>}
     * @param data what to write
     * @param mode {@code overwrite} or {@code append}
     * @return what was written: {@code rows}, {@code snapshot}, {@code geo}…
     */
    public static Map<String, Object> write(String name, Dataset<Row> data, String mode) {
        return Json.objeto(puente("write").write(name, data, mode));
    }
}
