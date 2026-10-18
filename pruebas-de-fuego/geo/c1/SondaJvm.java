package ore;

import java.io.ByteArrayInputStream;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import org.apache.arrow.memory.RootAllocator;
import org.apache.arrow.vector.ipc.ArrowStreamReader;
import org.apache.arrow.vector.types.pojo.Field;

/**
 * ORE 0068 C1·3 · El previo de Java: lo que un Java Transform ve y escribe de una geometría.
 * Se compila con el SDK del árbol dentro de la imagen del puesto JVM (jvm-lab) y se corre sin sesión.
 *   S1 · sql() → Rows: el tipo y el valor de una columna geo.
 *   S2 · Rows de sql() → el IPC de write(): ¿lleva la marca y el CRS?
 *   S3 · arrowSql() → el IPC de write(): ¿lleva la marca y el CRS?
 *   S4 · JTS y H3 de Java en el classpath, y el DuckDB del JDBC.
 */
public class SondaJvm {
    static final String GEO = "select 1 as id, ST_Buffer(ST_SetCRS(ST_Point(440000, 4474000), 'EPSG:25830'), 100) as zona";

    static void caso(String n, java.util.concurrent.Callable<Object> f) {
        try { System.out.println(n + ": " + f.call()); }
        catch (Throwable e) { Throwable c = e.getCause() != null ? e.getCause() : e; System.out.println(n + ": FALLA " + c.getClass().getSimpleName() + " " + String.valueOf(c.getMessage()).split("\n")[0]); }
    }

    static String marca(byte[] ipc, String col) throws Exception {
        try (var a = new RootAllocator(); var r = new ArrowStreamReader(new ByteArrayInputStream(ipc), a)) {
            r.loadNextBatch();
            Field f = r.getVectorSchemaRoot().getSchema().findField(col);
            Map<String, String> md = f.getMetadata();
            String m = md.get("ARROW:extension:metadata");
            return f.getType() + " · " + md.get("ARROW:extension:name") + " · " + (m == null ? null : m.substring(0, Math.min(90, m.length())));
        }
    }

    static byte[] ipc(Object datos) throws Exception {
        var ipcDe = Ore.class.getDeclaredMethod("ipcDe", Object.class, List.class);
        ipcDe.setAccessible(true);
        return (byte[]) ipcDe.invoke(null, datos, new ArrayList<Map<String, Object>>());
    }

    public static void main(String[] args) throws Exception {
        caso("S1 tipo en Rows", () -> { Ore.Rows r = Ore.sql(GEO); return r.types + " · valor " + r.get(0).get("zona").getClass().getSimpleName(); });
        caso("S2 Rows → write", () -> marca(ipc(Ore.sql(GEO)), "zona"));
        caso("S3 arrowSql → write", () -> marca(ipc(Ore.arrowSql(GEO)), "zona"));
        caso("S6 en una transacción", () -> {
            var duckdb = Ore.class.getDeclaredMethod("duckdb"); duckdb.setAccessible(true);
            java.sql.Connection con = (java.sql.Connection) duckdb.invoke(null);
            con.setAutoCommit(false);
            try (var st = con.createStatement()) {
                var rs = st.executeQuery(GEO);
                var r = (org.apache.arrow.vector.ipc.ArrowReader) ((org.duckdb.DuckDBResultSet) rs).arrowExportStream(new RootAllocator(), 1024);
                r.loadNextBatch();
                Field f = r.getVectorSchemaRoot().getSchema().findField("zona");
                String m = f.getMetadata().get("ARROW:extension:metadata");
                return f.getType() + " · " + f.getMetadata().get("ARROW:extension:name") + " · " + (m == null ? null : m.substring(0, Math.min(80, m.length())));
            } finally { con.commit(); con.setAutoCommit(true); }
        });
        caso("S5 sin CRS", () -> Ore.sql("select ST_Point(1, 2) as g").types);
        caso("S5 WKB (BLOB)", () -> Ore.sql("select ST_AsWKB(ST_Point(1, 2)) as g").types);
        caso("S5 geometría a texto", () -> Ore.sql("select ST_AsText(ST_SetCRS(ST_Point(1, 2), 'EPSG:4326')) as g").get(0));
        caso("S4 JTS", () -> Class.forName("org.locationtech.jts.geom.Geometry").getProtectionDomain().getCodeSource().getLocation());
        caso("S4 H3", () -> Class.forName("com.uber.h3core.H3Core").getProtectionDomain().getCodeSource().getLocation());
        caso("S4 DuckDB", () -> Ore.sql("select version() as v").get(0).get("v"));
        caso("S4 h3 en SQL", () -> Ore.sql("select h3_h3_to_string(h3_latlng_to_cell(40.4168, -3.7038, 9)) as c").get(0).get("c"));
    }
}
