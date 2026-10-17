package ore;

import java.io.ByteArrayInputStream;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.file.Files;
import java.nio.file.Path;
import java.sql.ResultSet;
import java.sql.Statement;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import org.apache.arrow.memory.RootAllocator;
import org.apache.arrow.vector.VarBinaryVector;
import org.apache.arrow.vector.ipc.ArrowStreamReader;
import org.apache.arrow.vector.types.pojo.Field;

/**
 * La geometría en el SDK de la JVM (ORE 0067 G2·3·4). Fuera del paquete de la imagen: se compila
 * junto a {@code Ore.java} y se corre con {@code java ore.GeoPrueba}. Sin {@code spatial} (la imagen
 * de hoy no la trae, G2·3·5), lo de DuckDB se salta y lo dice.
 */
public class GeoPrueba {
    static void igual(Object a, Object b, String que) {
        if (!java.util.Objects.equals(a, b)) throw new AssertionError(que + ": " + a + " != " + b);
        System.out.println("ok · " + que);
    }

    static byte[] cuadrado() {
        ByteBuffer b = ByteBuffer.allocate(1 + 4 + 4 + 4 + 5 * 16).order(ByteOrder.LITTLE_ENDIAN);
        b.put((byte) 1).putInt(3).putInt(1).putInt(5);
        double[][] p = { { 440000, 4474000 }, { 441000, 4474000 }, { 441000, 4475000 }, { 440000, 4475000 }, { 440000, 4474000 } };
        for (double[] xy : p) b.putDouble(xy[0]).putDouble(xy[1]);
        return b.array();
    }

    @SuppressWarnings("unchecked")
    public static void main(String[] args) throws Exception {
        igual(Ore.crsDeGeo("geometry"), "OGC:CRS84", "geometry es OGC:CRS84");
        igual(Ore.crsDeGeo("geometry(EPSG:25830)"), "EPSG:25830", "geometry(EPSG:25830)");
        igual(Ore.crsDeGeo("Geometry<Point, EPSG:25830>"), "EPSG:25830", "el tipo de OOS");
        igual(Ore.crsDeGeo("Geography<LineString>"), "OGC:CRS84", "Geography");
        igual(Ore.crsDeGeo("string"), null, "lo que no es geo");

        // write(): una List<Map> con una columna `geometry(EPSG:25830)` va como WKB marcado.
        List<Map<String, Object>> filas = new ArrayList<>();
        Map<String, Object> f1 = new LinkedHashMap<>(); f1.put("id", "a"); f1.put("geom", cuadrado()); filas.add(f1);
        Map<String, Object> f2 = new LinkedHashMap<>(); f2.put("id", "b"); f2.put("geom", null); filas.add(f2);
        Ore.Rows rows = new Ore.Rows(new LinkedHashMap<>(Map.of("id", "string", "geom", "geometry(EPSG:25830)")), 2L, false);
        rows.addAll(filas);
        var ipcDe = Ore.class.getDeclaredMethod("ipcDe", Object.class, List.class);
        ipcDe.setAccessible(true);
        List<Map<String, Object>> esquema = new ArrayList<>();
        byte[] ipc = (byte[]) ipcDe.invoke(null, rows, esquema);
        Map<String, Object> campoGeo = esquema.stream().filter(c -> "geom".equals(c.get("name"))).findFirst().orElseThrow();
        igual(campoGeo.get("type"), "binary", "en Iceberg la geometría es binary");
        try (var a = new RootAllocator(); var r = new ArrowStreamReader(new ByteArrayInputStream(ipc), a)) {
            r.loadNextBatch();
            Field f = r.getVectorSchemaRoot().getSchema().findField("geom");
            igual(f.getMetadata().get("ARROW:extension:name"), "geoarrow.wkb", "la marca viaja en el IPC");
            igual(f.getMetadata().get("ARROW:extension:metadata"), "{\"crs\":\"EPSG:25830\"}", "con su CRS");
            igual(Ore.esGeo(f), true, "esGeo la reconoce");
            VarBinaryVector v = (VarBinaryVector) r.getVectorSchemaRoot().getVector("geom");
            igual(v.getObject(0).length, cuadrado().length, "el WKB, tal cual");
            igual(v.isNull(1), true, "la nula, nula");
        }

        // Leer: con `geo` en el puntero, el BLOB es GEOMETRY con su CRS.
        var duckdb = Ore.class.getDeclaredMethod("duckdb");
        duckdb.setAccessible(true);
        var con = (java.sql.Connection) duckdb.invoke(null);
        Path f = Files.createTempFile("blob", ".parquet");
        StringBuilder hex = new StringBuilder();
        for (byte x : cuadrado()) hex.append(String.format("%02x", x));
        try (Statement s = con.createStatement()) {
            s.execute("copy (select 'a' id, from_hex('" + hex + "') geom) to '" + f + "' (format parquet)");
        }
        Map<String, Object> r = Map.of("geo", Map.of("geom", Map.of("type", "Geometry<Polygon, EPSG:25830>", "crs", "EPSG:25830")));
        String fuente;
        try {
            fuente = Ore.conGeo("read_parquet('" + f + "')", r);
        } catch (Exception e) {
            System.out.println("saltada · leer como geometría: sin spatial (" + e.getMessage() + ")");
            return;
        }
        try (Statement s = con.createStatement(); ResultSet rs = s.executeQuery("select typeof(geom), ST_Area(geom) from " + fuente)) {
            rs.next();
            igual(rs.getString(1).startsWith("GEOMETRY"), true, "el BLOB se lee como GEOMETRY");
            igual(rs.getDouble(2), 1_000_000.0, "y ST_Area funciona");
        }
        // G2·3·7 · intersectsBox: && y ST_Intersects, y filtra.
        igual(Ore.intersectsBox("t.geom", 0, 0, 5, 5).startsWith("(\"t\".\"geom\" && ST_MakeEnvelope(0.0, 0.0, 5.0, 5.0) AND ST_Intersects("), true, "intersectsBox lleva los dos");
        try (Statement s = con.createStatement(); ResultSet rs = s.executeQuery(
                "select count(*) from (select ST_Point(1, 2) geom union all select ST_Point(10, 20)) where " + Ore.intersectsBox("geom", 0, 0, 5, 5))) {
            rs.next();
            igual(rs.getLong(1), 1L, "e intersectsBox filtra");
        }
        System.out.println("## ok");
    }
}
