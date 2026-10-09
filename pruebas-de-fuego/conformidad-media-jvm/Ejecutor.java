package conformidad;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.stream.Stream;

import ore.Json;
import ore.Ore;

/**
 * EL EJECUTOR DE LA JVM (0049 JM0): `conformidad/media/casos/*.json` contra el
 * SDK de Java, dentro de la imagen `puesto-jvm` y contra el banco de la media.
 * Lo lanza `pruebas-de-fuego/la-media-en-java.py`; no se corre a mano.
 *
 * Un caso es datos (el README de la suite): `op`, `pide`, `espera`. Cada `op`
 * que el SDK ya sabe hacer tiene aquí su manera de correrse; la que todavía no,
 * cuenta como PENDIENTE —ni bien ni mal— para que el recuento diga siempre
 * cuánto falta: `N/51`. Al cerrar JM4 no queda ninguna pendiente.
 *
 *     java conformidad.Ejecutor <conformidad/media> [filtro]
 *
 * `filtro`: el nombre de un fichero de casos (`open`) o un `id` (`open-003`).
 */
public final class Ejecutor {

    /** Lo que un caso dio. */
    enum Veredicto { BIEN, MAL, PENDIENTE }

    record Resultado(String id, String op, Veredicto v, String nota) {}

    /** Una `op` que el SDK sabe correr. Vacío en JM0: cada hito añade las suyas. */
    interface Op { String correr(Map<String, Object> caso) throws Exception; }

    static final Map<String, Op> OPS = new LinkedHashMap<>();

    public static void main(String[] args) throws Exception {
        Path raiz = Path.of(args.length > 0 ? args[0] : "conformidad/media");
        String filtro = args.length > 1 && !args[1].isEmpty() ? args[1] : null;

        int fallos = humo();

        List<Resultado> rs = new ArrayList<>();
        try (Stream<Path> fs = Files.list(raiz.resolve("casos"))) {
            for (Path f : fs.filter(p -> p.toString().endsWith(".json")).sorted().toList()) {
                String fichero = f.getFileName().toString().replaceAll("\\.json$", "");
                for (Map<String, Object> caso : casosDe(f)) {
                    String id = String.valueOf(caso.get("id"));
                    if (filtro != null && !filtro.equals(fichero) && !filtro.equals(id)) continue;
                    rs.add(correr(caso));
                }
            }
        }
        int bien = 0, mal = 0, pendientes = 0;
        for (Resultado r : rs) {
            switch (r.v()) {
                case BIEN -> { bien++; System.out.println("  ✓ " + r.id() + " · " + r.nota()); }
                case MAL -> { mal++; System.out.println("  ✗ " + r.id() + " · " + r.nota()); }
                case PENDIENTE -> pendientes++;
            }
        }
        Map<String, Integer> porOp = new LinkedHashMap<>();
        for (Resultado r : rs) if (r.v() == Veredicto.PENDIENTE) porOp.merge(r.op(), 1, Integer::sum);
        System.out.println("conformidad: " + bien + "/" + rs.size() + " · " + mal + " mal · " + pendientes + " pendientes"
            + (porOp.isEmpty() ? "" : " " + porOp));
        System.exit(fallos + mal > 0 ? 1 : 0);
    }

    @SuppressWarnings("unchecked")
    static List<Map<String, Object>> casosDe(Path f) throws Exception {
        Object d = Json.leer(Files.readString(f));
        if (d instanceof Map<?, ?> m && m.get("casos") instanceof List<?> l) d = l;
        return (List<Map<String, Object>>) d;
    }

    static Resultado correr(Map<String, Object> caso) {
        String id = String.valueOf(caso.get("id")), op = String.valueOf(caso.get("op"));
        Op o = OPS.get(op);
        if (o == null) return new Resultado(id, op, Veredicto.PENDIENTE, "");
        try {
            String nota = o.correr(caso);
            return new Resultado(id, op, Veredicto.BIEN, nota == null ? String.valueOf(caso.get("norma")) : nota);
        } catch (AssertionError e) {
            return new Resultado(id, op, Veredicto.MAL, e.getMessage());
        } catch (Exception e) {
            return new Resultado(id, op, Veredicto.MAL, e.getClass().getSimpleName() + ": " + e.getMessage());
        }
    }

    /**
     * Lo que JM0 prueba de verdad: que el puesto llega al banco por la puerta de
     * siempre. El listado de una colección en SQL (0057 C4) es lo único de la
     * media que la JVM ya tiene: `POST /puestos/p1/sql` dice que es una
     * colección, y las páginas de `GET /media/…/items` llegan a DuckDB.
     */
    static int humo() {
        try {
            Ore.Rows filas = Ore.sql("select path, size from conformidad.default.derivar order by path");
            List<Object> caminos = new ArrayList<>();
            for (Map<String, Object> f : filas) caminos.add(f.get("path"));
            if (!caminos.equals(List.of("docs/a.pdf", "docs/c.pdf", "img/b.png")))
                throw new AssertionError("el listado dio " + caminos);
            System.out.println("  ✓ humo · el puesto llega al banco: el listado en SQL de `derivar` da " + caminos);
            return 0;
        } catch (Throwable e) {
            System.out.println("  ✗ humo · el puesto no llega al banco: " + e);
            return 1;
        }
    }
}
