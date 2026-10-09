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

    record Resultado(String id, String op, Veredicto v, String nota, long ms) {
        Resultado(String id, String op, Veredicto v, String nota) { this(id, op, v, nota, 0); }
    }

    /** Una `op` que el SDK sabe correr, en una colección de su `en`; lo que devuelve es su nota (las medidas). */
    interface Op { String correr(Map<String, Object> caso, String en) throws Exception; }

    /** Lo que un caso pide y el hito que lo hace todavía no ha llegado. */
    static final class Pendiente extends RuntimeException {
        Pendiente(String porque) { super(porque); }
    }

    static final Map<String, Op> OPS = new LinkedHashMap<>();

    static {
        Lectura.registrar(OPS);   // JM1
    }

    public static void main(String[] args) throws Exception {
        Path raiz = Path.of(args.length > 0 ? args[0] : "conformidad/media");
        String filtro = args.length > 1 && !args[1].isEmpty() ? args[1] : null;
        Banco.leerMuestra(raiz);

        // El SDK primero: su prueba 6 cuenta con el primer permiso del banco, y la 13 con la primera ficha.
        int fallos = filtro == null || filtro.equals("sdk") ? Sdk.correr() : 0;
        fallos += humo();

        List<Resultado> rs = new ArrayList<>();
        try (Stream<Path> fs = Files.list(raiz.resolve("casos"))) {
            for (Path f : fs.filter(p -> p.toString().endsWith(".json")).sorted().toList()) {
                String fichero = f.getFileName().toString().replaceAll("\\.json$", "");
                for (Map<String, Object> caso : casosDe(f)) {
                    String id = String.valueOf(caso.get("id"));
                    if (filtro != null && !filtro.equals(fichero) && !filtro.equals(id)) continue;
                    long t = System.nanoTime();
                    Resultado r = correr(caso);
                    rs.add(new Resultado(r.id(), r.op(), r.v(), r.nota(), (System.nanoTime() - t) / 1_000_000));
                }
            }
        }
        int bien = 0, mal = 0, pendientes = 0;
        for (Resultado r : rs) {
            switch (r.v()) {
                case BIEN -> { bien++; System.out.println("  ✓ " + r.id() + " · " + r.nota() + lento(r)); }
                case MAL -> { mal++; System.out.println("  ✗ " + r.id() + " · " + r.nota() + lento(r)); }
                case PENDIENTE -> {
                    pendientes++;
                    if (!r.nota().isEmpty()) System.out.println("  · " + r.id() + " · " + r.nota());
                }
            }
        }
        Map<String, Integer> porOp = new LinkedHashMap<>();
        for (Resultado r : rs) if (r.v() == Veredicto.PENDIENTE) porOp.merge(r.op(), 1, Integer::sum);
        System.out.println("conformidad: " + bien + "/" + rs.size() + " · " + mal + " mal · " + pendientes + " pendientes"
            + (porOp.isEmpty() ? "" : " " + porOp));
        System.exit(fallos + mal > 0 ? 1 : 0);
    }

    /** Lo que tarda un caso, si pasa de un segundo: la corrida entera tiene que ser rápida. */
    static String lento(Resultado r) { return r.ms() >= 1000 ? " (" + r.ms() + " ms)" : ""; }

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
        // En cada colección de su `en`: que la misma expectativa valga en la
        // mantenida y en la virtual es el punto de 0049 D1.
        List<String> notas = new ArrayList<>();
        Object en = caso.get("en");
        for (Object c : en instanceof List<?> l ? l : List.of("mantenida")) {
            try {
                String en1 = String.valueOf(c);
                String nota = Lectura.enSuAmbito(caso, () -> o.correr(caso, en1));
                Lectura.despuesDelAmbito(caso, en1);
                if (nota != null) notas.add(c + ": " + nota);
            } catch (Pendiente e) {
                return new Resultado(id, op, Veredicto.PENDIENTE, e.getMessage());
            } catch (AssertionError e) {
                return new Resultado(id, op, Veredicto.MAL, e.getMessage());
            } catch (Throwable e) {
                return new Resultado(id, op, Veredicto.MAL, "[" + c + "] " + e.getClass().getSimpleName() + ": " + e.getMessage());
            }
        }
        return new Resultado(id, op, Veredicto.BIEN, notas.isEmpty() ? String.valueOf(caso.get("norma")) : String.join(" · ", notas));
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
