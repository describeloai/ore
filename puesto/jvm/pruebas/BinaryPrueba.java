package ore;

import java.io.ByteArrayInputStream;
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
 * `Binary` en el SDK de la JVM (OOS v1alpha35, ORE 0076). Se compila junto a {@code Ore.java} y se
 * corre con {@code java ore.BinaryPrueba} (imagen del puesto JVM: {@code bash
 * pruebas-de-fuego/geo/c1/jvm-en-la-imagen.sh ore.BinaryPrueba}).
 */
public class BinaryPrueba {
    static void igual(Object a, Object b, String que) {
        if (!java.util.Objects.equals(a, b)) throw new AssertionError(que + ": " + a + " != " + b);
        System.out.println("ok · " + que);
    }

    public static void main(String[] args) throws Exception {
        byte[] bytes = { (byte) 0xde, (byte) 0xad, (byte) 0xbe, (byte) 0xef };

        // write(): una columna de byte[] se infiere binary y sale como binary de Iceberg, sin marca geo.
        List<Map<String, Object>> filas = new ArrayList<>();
        Map<String, Object> f1 = new LinkedHashMap<>(); f1.put("id", "a"); f1.put("firma", bytes); filas.add(f1);
        Map<String, Object> f2 = new LinkedHashMap<>(); f2.put("id", "b"); f2.put("firma", java.nio.ByteBuffer.wrap(bytes)); filas.add(f2);
        Map<String, Object> f3 = new LinkedHashMap<>(); f3.put("id", "c"); f3.put("firma", null); filas.add(f3);
        Ore.Rows rows = new Ore.Rows(new LinkedHashMap<>(Map.of("id", "string", "firma", "binary")), 3L, false);
        rows.addAll(filas);
        var ipcDe = Ore.class.getDeclaredMethod("ipcDe", Object.class, List.class);
        ipcDe.setAccessible(true);
        List<Map<String, Object>> esquema = new ArrayList<>();
        byte[] ipc = (byte[]) ipcDe.invoke(null, rows, esquema);
        Map<String, Object> campo = esquema.stream().filter(c -> "firma".equals(c.get("name"))).findFirst().orElseThrow();
        igual(campo.get("type"), "binary", "en Iceberg un Binary es binary");
        try (var a = new RootAllocator(); var r = new ArrowStreamReader(new ByteArrayInputStream(ipc), a)) {
            r.loadNextBatch();
            Field f = r.getVectorSchemaRoot().getSchema().findField("firma");
            igual(Ore.esGeo(f), false, "sin marca de geometría");
            VarBinaryVector v = (VarBinaryVector) r.getVectorSchemaRoot().getVector("firma");
            igual(java.util.Arrays.equals(v.getObject(0), bytes), true, "los bytes de un byte[], tal cual");
            igual(java.util.Arrays.equals(v.getObject(1), bytes), true, "y los de un ByteBuffer");
            igual(v.isNull(2), true, "la nula, nula");
        }

        // Un texto no es un Binary: no se toma por hex (eso es de la geometría).
        List<Map<String, Object>> mal = new ArrayList<>();
        Map<String, Object> m1 = new LinkedHashMap<>(); m1.put("firma", "deadbeef"); mal.add(m1);
        Ore.Rows filasMal = new Ore.Rows(new LinkedHashMap<>(Map.of("firma", "binary")), 1L, false);
        filasMal.addAll(mal);
        try {
            ipcDe.invoke(null, filasMal, new ArrayList<Map<String, Object>>());
            throw new AssertionError("un String en una columna binary debía fallar");
        } catch (java.lang.reflect.InvocationTargetException e) {
            igual(e.getCause().getMessage().contains("is binary: its values are bytes"), true, "un String en binary: error claro");
        }

        // Leer: el BLOB de DuckDB llega como byte[] y en JSON como base64.
        var duckdb = Ore.class.getDeclaredMethod("duckdb");
        duckdb.setAccessible(true);
        var con = (java.sql.Connection) duckdb.invoke(null);
        try (Statement s = con.createStatement(); ResultSet rs = s.executeQuery("select from_hex('deadbeef') b")) {
            rs.next();
            igual(java.util.Arrays.equals(rs.getBytes(1), bytes), true, "el BLOB se lee como bytes");
        }
        var toJson = Ore.class.getDeclaredMethod("toJson", Object.class);
        toJson.setAccessible(true);
        igual(toJson.invoke(null, (Object) bytes), "3q2+7w==", "en JSON, base64");
    }
}
