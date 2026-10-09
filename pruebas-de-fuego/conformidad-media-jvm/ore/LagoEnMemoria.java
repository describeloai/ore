package ore;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.concurrent.atomic.AtomicInteger;

import org.apache.arrow.vector.FieldVector;
import org.apache.arrow.vector.VectorSchemaRoot;
import org.apache.arrow.vector.types.pojo.Field;

/**
 * EL LAGO EN MEMORIA (0049 JM4b): donde {@code apply()} en filas lee y escribe en
 * el laboratorio, como {@code la-derivacion-en-python.py} en Python. Lo que se
 * escribe es el {@code VectorSchemaRoot} que {@code apply()} construyó —structs,
 * listas, el timestamp de {@code created}—, leído de vuelta con {@code valueAt},
 * como lo leería {@code over()}; y su esquema pasa por el de Iceberg que
 * {@code write()} manda ({@code tipoIcebergDe}), que falla si algo no cabe.
 */
public final class LagoEnMemoria implements Media.Lago {
    public final Map<String, List<Map<String, Object>>> tablas = new LinkedHashMap<>();
    public final Map<String, List<Field>> esquemas = new LinkedHashMap<>();
    public final Map<String, List<Object>> iceberg = new LinkedHashMap<>();
    /** {@code (tabla, filas, ancladaA)} de cada escritura. */
    public final List<List<Object>> escrituras = new ArrayList<>();

    /** Lo pone (o lo quita, con {@code null}) en lugar del lago de verdad. */
    public static void usar(LagoEnMemoria l) { Media.lago = l == null ? Media.LAGO : l; }

    @Override public synchronized List<Map<String, Object>> leer(String tabla) {
        List<Map<String, Object>> t = tablas.get(tabla);
        return t == null ? null : new ArrayList<>(t);
    }

    @Override public synchronized Map<String, Object> escribir(String tabla, VectorSchemaRoot filas, String ancladaA) {
        List<Map<String, Object>> leidas = new ArrayList<>();
        for (int i = 0; i < filas.getRowCount(); i++) {
            Map<String, Object> f = new LinkedHashMap<>();
            for (FieldVector v : filas.getFieldVectors()) f.put(v.getName(), Ore.valueAt(v, i));
            leidas.add(f);
        }
        AtomicInteger ids = new AtomicInteger(1_000_000);
        List<Object> ice = new ArrayList<>();
        for (Field f : filas.getSchema().getFields()) ice.add(Ore.tipoIcebergDe(f.getName(), f, ids, false));
        tablas.put(tabla, leidas);
        esquemas.put(tabla, filas.getSchema().getFields());
        iceberg.put(tabla, ice);
        escrituras.add(List.of(tabla, (long) leidas.size(), ancladaA == null ? "" : ancladaA));
        return Map.of("rows", (long) leidas.size());
    }
}
