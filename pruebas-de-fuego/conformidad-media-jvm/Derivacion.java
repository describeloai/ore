package conformidad;

import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Base64;
import java.util.Collections;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeSet;
import java.util.function.Function;

import ore.Media;
import ore.Ore;
import ore.ParaElBanco;

/**
 * JM4 · {@code apply()}: los once casos por pasos de {@code derivar.json}
 * (0049 B9, ficheros que dan ficheros), como los corre
 * {@code la-derivacion-a-ficheros.py} en Python: cada caso instala la entrada
 * ({@code conformidad.default.derivar}), crea una colección escrita nueva para
 * su salida y corre sus pasos con el SDK de Java.
 */
final class Derivacion {
    private Derivacion() {}

    static final String ENTRADA = "conformidad.default.derivar";
    static int salidas = 0;
    /** Las rutas sobre las que se llamó la función, en todo el caso. */
    static final List<String> LLAMADAS = Collections.synchronizedList(new ArrayList<>());

    static void registrar(Map<String, Ejecutor.Op> ops) { ops.put("apply", Derivacion::apply); }

    static void exige(boolean cierto, String porque) { if (!cierto) throw new AssertionError(porque); }

    // ── las funciones con nombre (README · un caso por pasos) ───────────────

    static Object lineasPdf(Media.Item item) {
        LLAMADAS.add(item.ref().path());
        if (!item.ref().path().endsWith(".pdf")) return List.of();
        List<String> lineas = new ArrayList<>(List.of(new String(item.readBytes(), StandardCharsets.UTF_8).split("\n", -1)));
        if (!lineas.isEmpty() && lineas.get(lineas.size() - 1).isEmpty()) lineas.remove(lineas.size() - 1);
        List<Media.File> fs = new ArrayList<>();
        for (int n = 1; n <= lineas.size(); n++)
            fs.add(Media.File.of("l" + n + ".txt", lineas.get(n - 1).getBytes(StandardCharsets.UTF_8), "text/plain", Map.of("kind", "page", "page", n)));
        return fs;
    }

    static Object nombreRepetido(Media.Item item) {
        LLAMADAS.add(item.ref().path());
        if (!item.ref().path().endsWith(".pdf")) return List.of();
        return List.of(Media.File.of("x.txt", "uno".getBytes()), Media.File.of("x.txt", "dos".getBytes()));
    }

    /** Una función escrita en esta clase (para D-JM2, la prueba 19 del SDK). */
    static final Function<Media.Item, Object> ESCRITA_AQUI = i -> List.of();

    static final Map<String, Function<Media.Item, Object>> FUNCIONES = Map.of("lineas_pdf", Derivacion::lineasPdf,
        "nombre_repetido", Derivacion::nombreRepetido);

    /** Lo que el ejecutor lanza para cortar una pasada ({@code cortar_tras_guardar}). */
    static final class Corte extends RuntimeException {
        Corte() { super("corte"); }
    }

    // ── el estado de la salida, como lo ve el banco ─────────────────────────

    record Visto(long tx, Map<String, Object> actuales, Map<String, Map<String, Object>> registro, Map<String, Object> entrada) {}

    @SuppressWarnings("unchecked")
    static Visto ver(String salida) throws Exception {
        Map<String, Object> e = Banco.mando("escrita", "coleccion", salida);
        Map<String, Map<String, Object>> reg = new LinkedHashMap<>();
        for (Object o : Lectura.l(e.get("registro"))) {
            Map<String, Object> d = Lectura.m(o);
            String uri = String.valueOf(Lectura.m(d.get("source")).get("uri"));
            reg.put(origen(uri), d);
        }
        return new Visto(((Number) e.get("tx")).longValue(), Lectura.m(e.get("items")), reg, Lectura.m(e.get("entrada")));
    }

    /** {@code ore://conformidad.default.derivar/docs/a.pdf?v=…} → {@code docs/a.pdf}. */
    static String origen(String uri) {
        String[] p = uri.split("/", 4);
        return p[p.length - 1].split("\\?")[0];
    }

    // ── un caso ─────────────────────────────────────────────────────────────

    @SuppressWarnings("unchecked")
    static String apply(Map<String, Object> caso, String en) throws Exception {
        Banco.mando("derivar", "accion", "instalar");
        LLAMADAS.clear();
        String salida = String.format("conformidad.default.salida_jvm_%02d", ++salidas);
        Ore.createCollection(salida, "document", List.of("txt"));
        Media.Collection entrada = Ore.collection(ENTRADA);
        for (Object o : Lectura.l(caso.get("pasos"))) {
            Map<String, Object> paso = Lectura.m(o);
            Visto antes = ver(salida);
            Map<String, Object> ctx = new LinkedHashMap<>();
            ctx.put("tx_antes", antes.tx());
            ctx.put("llamadas_antes", LLAMADAS.size());
            ctx.put("confirmados_antes", new HashSet<>(antes.registro().keySet()));
            if (paso.containsKey("apply")) {
                Map<String, Object> a = new LinkedHashMap<>(Lectura.m(paso.get("apply")));
                Function<Media.Item, Object> base = FUNCIONES.get(String.valueOf(a.remove("funcion")));
                Set<String> falla = new HashSet<>();
                for (Object f : Lectura.l(a.remove("falla_en"))) falla.add(String.valueOf(f));
                Object cortar = a.remove("cortar_tras_guardar");
                Function<Media.Item, Object> fn = falla.isEmpty() ? base : item -> {
                    if (falla.contains(item.ref().path())) {
                        LLAMADAS.add(item.ref().path());
                        throw new IllegalArgumentException("falla a propósito en " + item.ref().path());
                    }
                    return base.apply(item);
                };
                Media.Apply op = Media.applying().output(salida).threads(1)
                    .name(String.valueOf(Lectura.m(paso.get("apply")).get("funcion")));
                if (a.get("version") != null) op.version(String.valueOf(a.get("version")));
                if (Boolean.TRUE.equals(a.get("retry_errors"))) op.retryErrors(true);
                if (a.get("save_every_s") instanceof Number n) op.saveEverySeconds(n.doubleValue());
                if (cortar instanceof Number n) {
                    int[] hechos = {0};
                    ParaElBanco.trasConfirmar(() -> { if (++hechos[0] >= n.intValue()) throw new Corte(); });
                }
                try {
                    ctx.put("resumen", entrada.apply(fn, op));
                } catch (Corte c) {
                    ctx.put("resumen", Map.of());
                } finally {
                    ParaElBanco.trasConfirmar(null);
                }
            } else if (paso.containsKey("sobrescribir")) {
                Map<String, Object> s = Lectura.m(paso.get("sobrescribir"));
                Banco.mando("derivar", "accion", "sobrescribir", "path", s.get("path"), "texto", s.get("texto"));
            } else if (paso.containsKey("borrar")) {
                Banco.mando("derivar", "accion", "borrar", "path", Lectura.m(paso.get("borrar")).get("path"));
            } else if (paso.containsKey("copiar")) {
                Map<String, Object> c = Lectura.m(paso.get("copiar"));
                Banco.mando("derivar", "accion", "copiar", "de", c.get("de"), "a", c.get("a"));
            } else if (paso.containsKey("derivations")) {
                List<Map<String, Object>> leido = new ArrayList<>();
                for (Map<String, Object> d : Ore.collection(salida).derivations()) leido.add(d);
                ctx.put("status", 200);
                ctx.put("derivations", leido);
            }
            mira(Lectura.m(paso.get("espera")), ctx, salida);
        }
        return null;
    }

    // ── el vocabulario de `espera` ──────────────────────────────────────────

    @SuppressWarnings("unchecked")
    static void mira(Map<String, Object> espera, Map<String, Object> ctx, String salida) throws Exception {
        Visto v = ver(salida);
        Media.Collection col = Ore.collection(salida);
        for (Map.Entry<String, Object> e : espera.entrySet()) {
            Object q = e.getValue();
            switch (e.getKey()) {
                case "resumen" -> {
                    Map<String, Object> r = (Map<String, Object>) ctx.get("resumen");
                    for (Map.Entry<String, Object> k : Lectura.m(q).entrySet()) {
                        Object visto = r.get(k.getKey());
                        exige(visto instanceof Number n && k.getValue() instanceof Number m && n.longValue() == m.longValue(),
                            "resumen." + k.getKey() + ": " + visto + " y se esperaba " + k.getValue() + " (" + r + ")");
                    }
                }
                case "ficheros" -> exige(new TreeSet<>(v.actuales().keySet()).equals(new TreeSet<>(Lectura.l(q).stream().map(String::valueOf).toList())),
                    "ficheros: " + new TreeSet<>(v.actuales().keySet()));
                case "ficheros_incluye" -> {
                    for (Object f : Lectura.l(q)) exige(v.actuales().containsKey(String.valueOf(f)), "falta " + f);
                }
                case "no_ficheros" -> {
                    for (Object f : Lectura.l(q)) exige(!v.actuales().containsKey(String.valueOf(f)), "sobra " + f);
                }
                case "origen" -> {
                    for (Map.Entry<String, Object> k : Lectura.m(q).entrySet()) {
                        Map<String, Object> s = Lectura.m(Lectura.m(v.actuales().get(k.getKey())).get("source"));
                        exige(String.valueOf(s.get("uri")).contains("/" + k.getValue() + "?"), k.getKey() + " sale de " + s.get("uri"));
                        exige(("sha256:" + v.entrada().get(String.valueOf(k.getValue()))).equals(s.get("digest")), "digest del origen " + s);
                    }
                }
                case "ancla" -> {
                    for (Map.Entry<String, Object> k : Lectura.m(q).entrySet()) {
                        Object a = Lectura.m(Lectura.m(v.actuales().get(k.getKey())).get("source")).get("anchor");
                        exige(igual(a, k.getValue()), k.getKey() + " anclado en " + a + " y no " + k.getValue());
                    }
                }
                case "derivacion" -> {
                    for (Map.Entry<String, Object> k : Lectura.m(q).entrySet()) {
                        Map<String, Object> d = Lectura.m(Lectura.m(v.actuales().get(k.getKey())).get("derivation"));
                        for (Map.Entry<String, Object> kk : Lectura.m(k.getValue()).entrySet())
                            exige(igual(d.get(kk.getKey()), kk.getValue()), k.getKey() + ": derivation " + d);
                    }
                }
                case "marcas" -> {
                    for (Map.Entry<String, Object> k : Lectura.m(q).entrySet()) {
                        Map<String, Object> d = v.registro().get(k.getKey());
                        if (k.getValue() == null) exige(d == null || "files".equals(d.get("state")), k.getKey() + " sigue con su marca: " + d);
                        else exige(d != null && k.getValue().equals(d.get("state")), k.getKey() + ": " + d);
                    }
                }
                case "sin_escribir" -> exige(v.tx() == (long) ctx.get("tx_antes"), "la salida pasó de la transacción " + ctx.get("tx_antes") + " a " + v.tx());
                case "bytes_de" -> {
                    for (Map.Entry<String, Object> k : Lectura.m(q).entrySet()) {
                        String d = String.valueOf(Lectura.m(v.actuales().get(k.getKey())).get("digest")).substring(7);
                        byte[] b = Base64.getDecoder().decode(String.valueOf(Banco.mando("lago_bytes", "sha256", d).get("base64")));
                        exige(new String(b, StandardCharsets.UTF_8).equals(k.getValue()), k.getKey() + " no tiene esos bytes");
                    }
                }
                case "stat_de" -> {
                    for (Map.Entry<String, Object> k : Lectura.m(q).entrySet()) {
                        try {
                            col.stat(k.getKey());
                            throw new AssertionError(k.getKey() + " se ve con stat");
                        } catch (Media.MediaError x) {
                            exige(x.type.equals(Lectura.m(k.getValue()).get("error")), String.valueOf(x));
                        }
                    }
                }
                case "despues_no_en_list" -> {
                    Set<String> ps = new HashSet<>();
                    for (Media.Item it : col.items()) ps.add(it.ref().path());
                    for (Object f : Lectura.l(q)) exige(!ps.contains(String.valueOf(f)), f + " en el listado");
                }
                case "estados" -> {
                    for (Map.Entry<String, Object> k : Lectura.m(q).entrySet())
                        exige(k.getValue().equals(Lectura.m(v.registro().get(k.getKey())).get("state")), k.getKey() + ": " + v.registro().get(k.getKey()));
                    // Y lo mismo leído por `derivations()`, que es lo que el SDK da.
                    List<Map<String, Object>> leido = (List<Map<String, Object>>) ctx.get("derivations");
                    exige(leido != null && leido.size() == v.registro().size(), "derivations() dio " + leido);
                }
                case "ficheros_de" -> {
                    for (Map.Entry<String, Object> k : Lectura.m(q).entrySet())
                        exige(Lectura.l(Lectura.m(v.registro().get(k.getKey())).get("files")).size() == ((Number) k.getValue()).intValue(),
                            k.getKey() + ": " + v.registro().get(k.getKey()));
                }
                case "status" -> exige(igual(ctx.getOrDefault("status", 200), q), "status " + ctx.get("status"));
                case "invariante" -> {
                    for (Object i : Lectura.l(q)) {
                        exige("sin_recalculo".equals(i), "invariante `" + i + "` desconocida");
                        Set<String> otraVez = new HashSet<>(LLAMADAS.subList((int) ctx.get("llamadas_antes"), LLAMADAS.size()));
                        otraVez.retainAll((Set<String>) ctx.get("confirmados_antes"));
                        exige(otraVez.isEmpty(), "se recalculó lo ya confirmado: " + new TreeSet<>(otraVez));
                    }
                }
                default -> throw new AssertionError("`espera." + e.getKey() + "` no es del vocabulario");
            }
        }
    }

    static boolean igual(Object a, Object b) {
        if (a instanceof Number x && b instanceof Number y) return x.doubleValue() == y.doubleValue();
        if (a instanceof Map<?, ?> x && b instanceof Map<?, ?> y) {
            if (!x.keySet().equals(y.keySet())) return false;
            for (Object k : x.keySet()) if (!igual(x.get(k), y.get(k))) return false;
            return true;
        }
        return a == null ? b == null : a.equals(b);
    }
}
