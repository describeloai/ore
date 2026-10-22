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
 * junto a {@code Ore.java} y se corre con {@code java ore.GeoPrueba} (en la imagen del puesto JVM:
 * {@code bash pruebas-de-fuego/geo/c1/jvm-en-la-imagen.sh}). Sin {@code spatial}, lo de DuckDB se salta y lo dice.
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

        // ORE 0067, medido en victor: sin `geo` en el puntero, el SDK lo toma de la tabla.
        java.nio.file.Path raiz = java.nio.file.Path.of(System.getenv().getOrDefault("ORE_CUERPOS", "puesto/python/pruebas/cuerpos/geo-v3-ore"), "metadata");
        String ml;
        try (var fs = java.nio.file.Files.list(raiz)) {
            ml = fs.map(java.nio.file.Path::toString).filter(n -> n.endsWith(".metadata.json")).findFirst().orElseThrow();
        }
        Map<String, Object> geoTabla = Ore.geoDeLaTabla(ml.replace('\\', '/'));
        igual(((Map<String, Object>) geoTabla.get("geom")).get("crs"), "EPSG:25830", "el geo de la tabla, por sus propiedades");

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
        // ORE 0072 F1·1: con la cobertura delante (lo que poda en el lago); sin ella, en una relación propia.
        igual(Ore.intersectsBox("t.geom", 0, 0, 5, 5).startsWith("(\"t\".\"geom_bbox\".xmin <= 5.0 AND \"t\".\"geom_bbox\".xmax >= 0.0 AND"), true, "intersectsBox lleva la cobertura");
        igual(Ore.intersectsBox("t.geom", 0, 0, 5, 5, false).startsWith("(\"t\".\"geom\" && ST_MakeEnvelope(0.0, 0.0, 5.0, 5.0) AND ST_Intersects("), true, "y sin ella, los dos de siempre");
        try (Statement s = con.createStatement(); ResultSet rs = s.executeQuery(
                "select count(*) from (select ST_Point(1, 2) geom union all select ST_Point(10, 20)) where " + Ore.intersectsBox("geom", 0, 0, 5, 5, false))) {
            rs.next();
            igual(rs.getLong(1), 1L, "e intersectsBox filtra");
        }
        // Una tabla del lago sin cobertura (escrita antes) la recibe calculada al leerla: el filtro vale igual.
        String vieja = Ore.conGeo("(select ST_AsWKB(ST_Point(1, 2)) geom union all select ST_AsWKB(ST_Point(10, 20)))",
            Map.<String, Object>of("geo", Map.of("geom", Map.of("crs", "OGC:CRS84"))));
        try (Statement s = con.createStatement(); ResultSet rs = s.executeQuery(
                "select count(*) from " + vieja + " where " + Ore.intersectsBox("geom", 0, 0, 5, 5))) {
            rs.next();
            igual(rs.getLong(1), 1L, "la cobertura calculada al leer filtra igual");
        }

        // ORE 0068 C1·3 · Lo que un Java Transform lee de una geometría calculada, y lo que escribe.
        String zona = "select 1 as id, ST_Buffer(ST_SetCRS(ST_Point(440000, 4474000), 'EPSG:25830'), 100) as zona";
        Ore.Rows calc = Ore.sql(zona);
        igual(calc.types.get("zona"), "geometry(EPSG:25830)", "sql() de una geometría con CRS: su tipo");
        igual(calc.get(0).get("zona") instanceof byte[], true, "y su valor, el WKB");
        igual(con.getAutoCommit(), true, "la transacción del lector, cerrada");
        List<Map<String, Object>> esq2 = new ArrayList<>();
        byte[] ipc2 = (byte[]) ipcDe.invoke(null, calc, esq2);
        try (var a = new RootAllocator(); var rd = new ArrowStreamReader(new ByteArrayInputStream(ipc2), a)) {
            rd.loadNextBatch();
            Field fz = rd.getVectorSchemaRoot().getSchema().findField("zona");
            igual(fz.getMetadata().get("ARROW:extension:metadata"), "{\"crs\":\"EPSG:25830\"}", "leer y escribir: la marca y su CRS");
        }
        byte[] ipc3 = (byte[]) ipcDe.invoke(null, Ore.arrowSql(zona), new ArrayList<Map<String, Object>>());
        try (var a = new RootAllocator(); var rd = new ArrowStreamReader(new ByteArrayInputStream(ipc3), a)) {
            rd.loadNextBatch();
            Field fz = rd.getVectorSchemaRoot().getSchema().findField("zona");
            igual(Ore.crsDeGeoarrow(fz.getMetadata().get("ARROW:extension:metadata")), "EPSG:25830", "arrowSql() también, con su CRS");
        }
        igual(Ore.crsDeGeoarrow("{\"crs\":\"OGC:CRS84\"}"), "OGC:CRS84", "el CRS escrito tal cual");
        igual(Ore.crsDeGeoarrow("{}"), "?", "sin CRS");
        Ore.Rows sin = Ore.sql("select ST_Point(1, 2) as p");
        igual(sin.types.get("p"), "geometry(?)", "una geometría sin CRS se dice");
        try {
            ipcDe.invoke(null, sin, new ArrayList<Map<String, Object>>());
            throw new AssertionError("write() de una geometría sin CRS tenía que fallar");
        } catch (java.lang.reflect.InvocationTargetException e) {
            igual(String.valueOf(e.getCause().getMessage()).contains("ST_SetCRS"), true, "y write() la niega, con su remedio");
        }

        // ORE 0068 C1·3 pieza 3 · La geometría como objeto (JTS 1.20.0) y H3 de Java.
        var g = Ore.geometry(calc.get(0).get("zona"));
        igual(g.getGeometryType(), "Polygon", "Ore.geometry(): un objeto de JTS");
        igual(Math.abs(g.getArea() - Math.PI * 100 * 100) < 0.01 * Math.PI * 100 * 100, true, "y JTS calcula: el área de un círculo de 100 m");
        igual(Ore.geometry(null), null, "null es null");
        var centro = g.getCentroid();
        centro.setSRID(25830);
        List<Map<String, Object>> conJts = new ArrayList<>();
        Map<String, Object> fj = new LinkedHashMap<>(); fj.put("id", 1L); fj.put("centro", centro); conJts.add(fj);
        byte[] ipc4 = (byte[]) ipcDe.invoke(null, conJts, new ArrayList<Map<String, Object>>());
        try (var a = new RootAllocator(); var rd = new ArrowStreamReader(new ByteArrayInputStream(ipc4), a)) {
            rd.loadNextBatch();
            Field fc = rd.getVectorSchemaRoot().getSchema().findField("centro");
            igual(fc.getMetadata().get("ARROW:extension:metadata"), "{\"crs\":\"EPSG:25830\"}", "write() de JTS: el CRS de su SRID");
            byte[] w = ((VarBinaryVector) rd.getVectorSchemaRoot().getVector("centro")).getObject(0);
            igual(Ore.geometry(w).equalsExact(centro, 1e-9), true, "y su WKB vuelve a ser la misma geometría");
        }
        var sinSrid = g.getCentroid();
        List<Map<String, Object>> malJts = new ArrayList<>();
        Map<String, Object> fm = new LinkedHashMap<>(); fm.put("p", sinSrid); malJts.add(fm);
        try {
            ipcDe.invoke(null, malJts, new ArrayList<Map<String, Object>>());
            throw new AssertionError("una geometría de JTS sin SRID tenía que fallar");
        } catch (java.lang.reflect.InvocationTargetException e) {
            igual(String.valueOf(e.getCause().getMessage()).contains("setSRID"), true, "JTS sin SRID se niega, con su remedio");
        }
        var h3 = com.uber.h3core.H3Core.newInstance();
        try (Statement s = con.createStatement(); ResultSet rs = s.executeQuery(
                "select h3_h3_to_string(h3_latlng_to_cell(40.4168, -3.7038, 9)), h3_h3_to_string(h3_latlng_to_cell(55.6761, 12.5683, 12))")) {
            rs.next();
            igual(h3.latLngToCellAddress(40.4168, -3.7038, 9), rs.getString(1), "H3 de Java: la misma celda que en SQL (res 9)");
            igual(h3.latLngToCellAddress(55.6761, 12.5683, 12), rs.getString(2), "y en res 12");
        }
        // ORE 0068 (cierre de C1): x es la longitud, siempre (`geometry_always_xy` al cargar spatial).
        Object p = Ore.sql("select ST_AsText(ST_Transform(ST_Point(440000, 4474000), 'EPSG:25830', 'EPSG:4326')) p").get(0).get("p");
        igual(String.valueOf(p).startsWith("POINT (-3.707"), true, "a 4326 sale lon/lat: " + p);
        double d = ((Number) Ore.sql("select ST_Distance_Spheroid(ST_Point(-3.7072, 40.4145), ST_Point(2.1734, 41.3851)) / 1000 d")
            .get(0).get("d")).doubleValue();
        igual(Math.abs(d - 507) < 2, true, "Sol–Barcelona desde lon/lat, 507 km (sin el ajuste, 659): " + d);
        System.out.println("## ok");
    }
}
