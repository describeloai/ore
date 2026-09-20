import java.sql.Connection;
import java.sql.DriverManager;
import java.sql.ResultSet;
import java.sql.Statement;

/**
 * Preinstala en la imagen las extensiones de DuckDB que leen el lago (0031 §10,
 * medido en {@code medida-w3-lago.py}): {@code iceberg} y lo que arrastra. Corre al
 * construir {@code puesto-jvm:1} como programa de un fichero
 * ({@code java -cp duckdb_jdbc.jar Extensiones.java}), donde SÍ hay red; en el pod
 * no la hay, y una extensión que falte cuelga la celda 120 s. Un fallo aquí tumba
 * la construcción, que es lo que se quiere.
 */
public class Extensiones {
    public static void main(String[] a) throws Exception {
        Class.forName("org.duckdb.DuckDBDriver");
        try (Connection c = DriverManager.getConnection("jdbc:duckdb:"); Statement s = c.createStatement()) {
            s.execute("set extension_directory = '" + (a.length > 0 ? a[0] : "/opt/ore/duckdb") + "'");
            for (String e : new String[] {"iceberg", "avro", "httpfs", "json", "icu"}) s.execute("install " + e);
            s.execute("load iceberg");
            s.execute("load httpfs");
            try (ResultSet r = s.executeQuery("select version(), string_agg(extension_name, ' ' order by extension_name) from duckdb_extensions() where loaded")) {
                r.next();
                System.out.println("lago · " + r.getString(1) + " · " + r.getString(2));
            }
        }
    }
}
