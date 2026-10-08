package ore;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.PrintStream;
import java.net.URI;
import java.net.URLEncoder;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.time.Duration;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import jdk.jshell.Diag;
import jdk.jshell.EvalException;
import jdk.jshell.ExpressionSnippet;
import jdk.jshell.JShell;
import jdk.jshell.Snippet;
import jdk.jshell.SnippetEvent;
import jdk.jshell.SourceCodeAnalysis;
import jdk.jshell.TypeDeclSnippet;
import jdk.jshell.UnresolvedReferenceException;
import jdk.jshell.VarSnippet;

/**
 * EL AGENTE DEL PUESTO — JVM (0031 W3.4): lo que corre dentro de la sesión Java.
 *
 * <p>El mismo agente que {@code puesto/python/agente.py}, en su lenguaje: pide
 * trabajo a {@code ore-serve} por HTTP (polling largo: el puesto no acepta
 * conexiones, «sin entrada»), ejecuta cada celda en una sesión de JShell que
 * dura todo el puesto, y devuelve la salida TIPADA —tabla, texto, error,
 * vacía— al mismo sitio.
 *
 * <pre>
 *   GET  /puestos/{id}/pendiente            → 200 {celda, lenguaje, texto}
 *   POST /puestos/{id}/celdas/{n}/salida    ← {tipo, ms, …}
 * </pre>
 *
 * <h2>Y el servidor de lenguaje (0037 ③b)</h2>
 *
 * <p>A diferencia del de Python —que ARRANCA pyright y hace de correa—, aquí
 * el servidor de lenguaje es esta misma JVM: {@link Lenguaje} contesta con el
 * compilador del JDK y con {@code Trees}, que ya están dentro. Lo que llega por
 * el flujo se atiende y lo que salga se devuelve agrupado.
 *
 * <pre>
 *   GET  /puestos/{id}/lsp/agente           → flujo de eventos: lo que el editor manda
 *   POST /puestos/{id}/lsp/salida           ← lo que este servidor contesta
 * </pre>
 *
 * <h2>El kernel</h2>
 *
 * <p>Medido antes de escribirlo ({@code medida-w3-ts-jvm.py}, en victor sobre
 * un JDK 21): JShell EN PROCESO ({@code executionEngine("local")}) se crea en
 * 25 ms y evalúa una celda en 20–250 ms (la primera, más: carga clases); el
 * motor por defecto (otra JVM por JDI) tarda 770 ms sólo en arrancar. Una
 * celda puede traer varios snippets (imports, un record, un método, una
 * expresión): se parten con {@code SourceCodeAnalysis} y se evalúan en orden;
 * el valor de la última EXPRESIÓN (la variable temporal {@code $n} de JShell)
 * se recoge como objeto de verdad —no como su {@code toString}— pasándolo por
 * {@link #guarda(Object)}, que vive en esta misma JVM. Una clase con
 * {@code static void main} (un {@code .java} del árbol) se declara y se llama.
 *
 * <p>Quién es y cuándo muere: como el de Python (client credentials contra el
 * IdP dentro del clúster; {@code ORE_SUJETO} en las pruebas; TTL de
 * inactividad; 410 = cerrado).
 */
public final class Agente {
    static final int FILAS_MAXIMAS = 200;
    /** Lo que el agente dice, en UTF-8 pase lo que pase con la consola; una celda escribe en el System.out de turno. */
    static final PrintStream LOG = new PrintStream(new java.io.FileOutputStream(java.io.FileDescriptor.out), true, StandardCharsets.UTF_8);

    static void log(String s) { LOG.println("agente · " + s); LOG.flush(); }

    // ── Quién soy: el token ─────────────────────────────────────────────────
    static final class Testigo {
        final String sujeto = System.getenv("ORE_SUJETO");
        final String direccion = Ore.env("DIRECCION", "").replaceAll("/+$", "");
        final String realm = Ore.env("REALM", "rubix");
        final String cliente = fichero("agente-cliente");
        final String secreto = fichero("agente-secreto");
        String token;
        long caduca = 0;

        static String fichero(String nombre) {
            try { return Files.readString(Path.of(Ore.env("PUESTO_DIR", "/puesto"), nombre)).strip(); } catch (IOException e) { return null; }
        }

        Map<String, String> cabeceras() throws IOException, InterruptedException {
            if (sujeto != null && !sujeto.isEmpty()) return Map.of("x-ore-sujeto", sujeto);
            // ⭐ R1 · El token de ESTE pod, si lo trae: el servidor sabe qué puesto
            //   habla. Se lee cada vez: el kubelet lo renueva en el sitio. Con la
            //   credencial del agente también en el pod, van las dos.
            String pod = null;
            try {
                pod = Files.readString(Path.of(Ore.env("ORE_TOKEN_DEL_POD", "/var/run/ore/pod/token"))).strip();
                if (pod.isEmpty()) pod = null;
            } catch (IOException e) { /* sin token de pod */ }
            if (pod != null && (cliente == null || secreto == null)) return Map.of("x-ore-pod", pod);
            Map<String, String> h = delAgente();
            if (pod == null) return h;
            Map<String, String> ambas = new java.util.HashMap<>(h);
            ambas.put("x-ore-pod", pod);
            return ambas;
        }

        Map<String, String> delAgente() throws IOException, InterruptedException {
            if (cliente == null || secreto == null || direccion.isEmpty()) {
                log("sin identidad: ni ORE_SUJETO ni /puesto/agente-{cliente,secreto} con DIRECCION");
                System.exit(2);
            }
            if (System.currentTimeMillis() > caduca - 60_000) {
                String datos = "grant_type=client_credentials&client_id=" + URLEncoder.encode(cliente, StandardCharsets.UTF_8)
                    + "&client_secret=" + URLEncoder.encode(secreto, StandardCharsets.UTF_8);
                HttpRequest q = HttpRequest.newBuilder(URI.create(direccion + "/realms/" + realm + "/protocol/openid-connect/token"))
                    .header("content-type", "application/x-www-form-urlencoded").timeout(Duration.ofSeconds(20))
                    .POST(HttpRequest.BodyPublishers.ofString(datos)).build();
                HttpResponse<String> r = Ore.HTTP.send(q, HttpResponse.BodyHandlers.ofString());
                if (r.statusCode() != 200) throw new IOException("el emisor contestó " + r.statusCode());
                Map<String, Object> t = Json.objeto(r.body());
                token = String.valueOf(t.get("access_token"));
                long dura = ((Number) t.getOrDefault("expires_in", 300L)).longValue();
                caduca = System.currentTimeMillis() + dura * 1000;
                log("token del agente renovado · caduca en " + dura + "s");
            }
            return Map.of("authorization", "Bearer " + token);
        }
    }

    // ── El kernel: una sesión de JShell para todo el puesto ─────────────────
    /** Donde una celda deja el valor de su última expresión (misma JVM que JShell). */
    private static volatile Object guardado;
    private static volatile boolean hayGuardado;

    public static void guarda(Object o) { guardado = o; hayGuardado = true; }

    static final class Kernel {
        final JShell js;
        final SourceCodeAnalysis analisis;

        Kernel() {
            js = JShell.builder().executionEngine("local").build();
            for (String p : classpath()) {
                js.addToClasspath(p);
            }
            analisis = js.sourceCodeAnalysis();
            for (String s : List.of("import java.util.*;", "import java.util.stream.*;", "import static ore.Ore.*;")) js.eval(s);
        }

        /**
         * El classpath de este proceso, CON LOS COMODINES EXPANDIDOS.
         *
         * <p>⚠️ `-cp /opt/ore/lib/*:/capa/*` lo expande el lanzador de Java al
         * arrancar, pero `System.getProperty("java.class.path")` devuelve la
         * cadena TAL CUAL, y `JShell.addToClasspath` no expande nada: una celda
         * que usara una biblioteca de la capa compilaría mal aunque la clase
         * estuviera cargada. Se expande aquí, y EN EL MISMO ORDEN —lo de la
         * imagen antes que lo de `/capa` (0037 ③c)—, porque cuando una clase
         * está en dos sitios gana la primera.
         */
        private static List<String> classpath() {
            List<String> fuera = new ArrayList<>();
            for (String p : System.getProperty("java.class.path", "").split(java.io.File.pathSeparator)) {
                if (p.isBlank()) {
                    continue;
                }
                if (!p.endsWith("*")) {
                    fuera.add(p);
                    continue;
                }
                java.io.File dir = new java.io.File(p.substring(0, p.length() - 1));
                java.io.File[] jars = dir.listFiles((d, n) -> n.endsWith(".jar") || n.endsWith(".JAR"));
                if (jars == null) {
                    continue; // sin capa, `/capa/*` no aporta nada y no estorba.
                }
                java.util.Arrays.sort(jars);
                for (java.io.File j : jars) {
                    fuera.add(j.getPath());
                }
            }
            return fuera;
        }

        Map<String, Object> correr(String texto, String lenguaje) {
            long t0 = System.nanoTime();
            ByteArrayOutputStream buffer = new ByteArrayOutputStream();
            PrintStream captura = new PrintStream(buffer, true, StandardCharsets.UTF_8);
            PrintStream err = System.err;
            System.setOut(captura);
            System.setErr(captura);
            Partes partes = new Partes(buffer, t0);
            try {
                Object valor;
                boolean hayValor;
                if (lenguaje.equals("sql")) {
                    valor = Ore.sql(texto);
                    hayValor = true;
                } else {
                    // J3: lo que la celda enseña con display(), en orden.
                    Ore.mostrar = partes::mostrar;
                    Resultado r = evaluar(texto);
                    if (r.error != null) return error(r.error, r.traza, buffer, t0);
                    valor = r.valor;
                    hayValor = r.hayValor;
                }
                if (partes.lista.isEmpty() && partes.fuera == 0) return salidaDe(hayValor ? valor : null, hayValor, texto(buffer), t0);
                return partes.cerrar(valor, hayValor);
            } catch (Exception e) {
                return error(e.getClass().getSimpleName() + ": " + e.getMessage(), traza(e), buffer, t0);
            } finally {
                Ore.mostrar = null;
                System.setOut(LOG);
                System.setErr(err);
            }
        }

        record Resultado(Object valor, boolean hayValor, String error, String traza) {}

        /** Los snippets de la celda, en orden; el valor de la última expresión. */
        Resultado evaluar(String texto) {
            String resto = texto;
            Object valor = null;
            boolean hayValor = false;
            List<String> conMain = new ArrayList<>();
            while (!resto.isBlank()) {
                SourceCodeAnalysis.CompletionInfo ci = analisis.analyzeCompletion(resto);
                String fuente;
                if (ci.completeness() == SourceCodeAnalysis.Completeness.COMPLETE || ci.completeness() == SourceCodeAnalysis.Completeness.COMPLETE_WITH_SEMI) {
                    fuente = ci.source();
                    resto = ci.remaining() == null ? "" : ci.remaining();
                } else if (ci.completeness() == SourceCodeAnalysis.Completeness.EMPTY) {
                    break;
                } else {
                    fuente = resto;
                    resto = "";
                }
                hayGuardado = false;
                guardado = null;
                for (SnippetEvent e : js.eval(fuente)) {
                    if (e.causeSnippet() != null) continue;
                    if (e.exception() != null) return new Resultado(null, false, mensajeDe(e.exception()), traza(e.exception()));
                    if (e.status() == Snippet.Status.REJECTED || e.status() == Snippet.Status.RECOVERABLE_NOT_DEFINED) {
                        StringBuilder d = new StringBuilder();
                        js.diagnostics(e.snippet()).filter(Diag::isError).forEach(x -> d.append(x.getMessage(Locale.ENGLISH).strip()).append("\n"));
                        String m = d.length() == 0 ? "el snippet no compila" : d.toString().strip();
                        return new Resultado(null, false, "CompilationError: " + m, m + "\n\n" + fuente.strip());
                    }
                    Snippet s = e.snippet();
                    // La expresión: su valor, como objeto, por `guarda`. Una expresión
                    // cualquiera es la variable temporal `$n`; una variable a secas
                    // (`df`) es un ExpressionSnippet con su nombre. Una asignación
                    // (`x = 5`) no enseña nada, como en Python.
                    String nombre = null;
                    if (s instanceof VarSnippet v && s.subKind() == Snippet.SubKind.TEMP_VAR_EXPRESSION_SUBKIND) nombre = v.name();
                    else if (s instanceof ExpressionSnippet x && s.subKind() == Snippet.SubKind.VAR_VALUE_SUBKIND) nombre = x.name();
                    if (nombre != null) {
                        hayGuardado = false;
                        js.eval("ore.Agente.guarda(" + nombre + ");");
                        if (hayGuardado) { valor = guardado; hayValor = true; }
                    } else if (s instanceof TypeDeclSnippet t && s.source().contains("static void main(")) {
                        conMain.add(t.name());
                    }
                }
            }
            // Un `.java` del árbol con `main`: se llama, como `java Fichero.java`.
            for (String c : conMain) {
                for (SnippetEvent e : js.eval(c + ".main(new String[0]);")) {
                    if (e.exception() != null) return new Resultado(null, false, mensajeDe(e.exception()), traza(e.exception()));
                }
            }
            return new Resultado(valor, hayValor, null, null);
        }

        static String mensajeDe(Exception e) {
            if (e instanceof EvalException ev) return ev.getExceptionClassName() + (ev.getMessage() == null ? "" : ": " + ev.getMessage());
            if (e instanceof UnresolvedReferenceException u) return "UnresolvedReference: " + u.getSnippet().name();
            return e.getClass().getSimpleName() + (e.getMessage() == null ? "" : ": " + e.getMessage());
        }

        static Map<String, Object> error(String mensaje, String traza, ByteArrayOutputStream buffer, long t0) {
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("tipo", "error");
            int i = mensaje.indexOf(':');
            m.put("nombre", i > 0 ? mensaje.substring(0, i) : mensaje);
            m.put("mensaje", i > 0 ? mensaje.substring(i + 1).strip() : mensaje);
            m.put("traza", traza == null ? mensaje : traza);
            m.put("texto", texto(buffer));
            m.put("ms", ms(t0));
            return m;
        }

        /** Lo escrito por la celda; con LF venga de donde venga (`println` en Windows pone CRLF). */
        static String texto(ByteArrayOutputStream buffer) {
            return buffer.toString(StandardCharsets.UTF_8).replace("\r\n", "\n");
        }

        static Map<String, Object> salidaDe(Object valor, boolean hayValor, String texto, long t0) {
            Map<String, Object> m = new LinkedHashMap<>();
            Map<String, Object> tabla = comoTabla(valor);
            if (tabla != null) {
                m.putAll(tabla);
                m.put("tipo", "tabla");
                m.put("texto", texto);
                m.put("ms", ms(t0));
                return m;
            }
            Map<String, Object> imagen = comoImagen(valor, IMAGEN_BYTES);
            if (imagen != null) {
                m.putAll(imagen);
                m.put("tipo", "imagen");
                m.put("texto", texto);
                m.put("ms", ms(t0));
                return m;
            }
            Map<String, Object> arbol = comoJson(valor);
            if (arbol != null) {
                m.putAll(arbol);
                m.put("tipo", "json");
                m.put("texto", texto);
                m.put("ms", ms(t0));
                return m;
            }
            if (!hayValor) {
                m.put("tipo", texto.isBlank() ? "vacia" : "texto");
                if (!texto.isBlank()) m.put("texto", texto);
                m.put("ms", ms(t0));
                return m;
            }
            m.put("tipo", "texto");
            m.put("texto", texto + (valor instanceof String s ? "\"" + s + "\"" : valor instanceof byte[] b ? "bytes · " + b.length + " B" : String.valueOf(valor)));
            m.put("ms", ms(t0));
            return m;
        }
    }

    static long ms(long t0) { return (System.nanoTime() - t0) / 1_000_000; }

    static String traza(Throwable e) {
        java.io.StringWriter w = new java.io.StringWriter();
        e.printStackTrace(new java.io.PrintWriter(w));
        return w.toString();
    }

    /** La salida {@code tabla} del contrato: la hace el SDK ({@link Ore#table}); aquí sólo se le pone el límite de la consola. */
    static Map<String, Object> comoTabla(Object valor) { return Ore.table(valor, FILAS_MAXIMAS); }

    // ── J1 (las salidas de una celda, S1) · `json`: un valor compuesto, como árbol ──
    // Los mismos topes que el agente de Python (`agente.py`): por nivel, por cadena,
    // de hondo y en total. Lo que no cabe se recorta y se dice (`recortado`); si ni
    // así cabe, la celda sale como texto.
    static final int JSON_POR_NIVEL = 500;
    static final int JSON_CADENA = 5000;
    static final int JSON_HONDO = 20;
    // ⛔ ore-serve admite como mucho 1 MB por cuerpo (`ore_entrada::http::CUERPO_MAXIMO`).
    static final int JSON_BYTES = 768 * 1024;

    static boolean esCompuesto(Object v) {
        return v instanceof Map<?, ?> || v instanceof java.util.Collection<?>
            || (v != null && v.getClass().isArray() && !(v instanceof byte[])) || (v != null && v.getClass().isRecord());
    }

    /**
     * {@code {valor, recortado}} de un valor compuesto —un {@code Map}, una
     * {@code Collection} (no un {@code Iterable} cualquiera: un {@code Path} lo es),
     * un array, un {@code record}—, o {@code null} si no
     * lo es (una lista de maps ya es una {@code tabla}). Las hojas van como el JSON
     * del contrato ({@link Ore#toJson}); los bytes, como {@code "bytes · N B"}; un
     * objeto que no es JSON, por su {@code toString()}.
     */
    static Map<String, Object> comoJson(Object valor) {
        if (!esCompuesto(valor)) return null;
        boolean[] recortado = {false};
        Object arbol;
        try {
            arbol = arbolDe(valor, 0, recortado);
        } catch (RuntimeException e) {
            return null;   // una colección que falla al recorrerse: como antes, por su toString()
        }
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("valor", arbol);
        m.put("recortado", recortado[0]);
        if (Json.escribir(m).getBytes(StandardCharsets.UTF_8).length > JSON_BYTES) return null;
        return m;
    }

    static Object arbolDe(Object v, int hondo, boolean[] recortado) {
        if (v == null) return null;
        if (v instanceof byte[] b) return "bytes · " + b.length + " B";
        if (v instanceof CharSequence cs) {
            String s = cs.toString();
            if (s.length() <= JSON_CADENA) return s;
            recortado[0] = true;
            return s.substring(0, JSON_CADENA) + "… (" + (s.length() - JSON_CADENA) + " more characters)";
        }
        if (!esCompuesto(v)) {
            Object j = Ore.toJson(v);
            return j == null || j instanceof Boolean || j instanceof Number || j instanceof String ? j : String.valueOf(v);
        }
        if (hondo >= JSON_HONDO) { recortado[0] = true; return "…"; }
        if (v.getClass().isRecord()) {
            Map<String, Object> out = new LinkedHashMap<>();
            for (java.lang.reflect.RecordComponent c : v.getClass().getRecordComponents()) {
                Object x;
                try {
                    java.lang.reflect.Method a = c.getAccessor();
                    a.setAccessible(true);
                    x = a.invoke(v);
                } catch (ReflectiveOperationException | RuntimeException e) {
                    return String.valueOf(v);   // un record que no se deja leer: por su toString()
                }
                out.put(c.getName(), arbolDe(x, hondo + 1, recortado));
            }
            return out;
        }
        if (v instanceof Map<?, ?> mapa) {
            Map<String, Object> out = new LinkedHashMap<>();
            int n = 0;
            for (Map.Entry<?, ?> e : mapa.entrySet()) {
                if (n++ == JSON_POR_NIVEL) break;
                out.put(String.valueOf(e.getKey()), arbolDe(e.getValue(), hondo + 1, recortado));
            }
            if (mapa.size() > JSON_POR_NIVEL) { recortado[0] = true; out.put("…", (mapa.size() - JSON_POR_NIVEL) + " more keys"); }
            return out;
        }
        List<Object> out = new ArrayList<>();
        int sobran = 0;
        if (v.getClass().isArray()) {
            int largo = java.lang.reflect.Array.getLength(v);
            for (int i = 0; i < Math.min(largo, JSON_POR_NIVEL); i++) out.add(arbolDe(java.lang.reflect.Array.get(v, i), hondo + 1, recortado));
            sobran = largo - out.size();
        } else {
            java.util.Collection<?> c = (java.util.Collection<?>) v;
            for (Object x : c) {
                if (out.size() == JSON_POR_NIVEL) break;
                out.add(arbolDe(x, hondo + 1, recortado));
            }
            sobran = c.size() - out.size();
        }
        if (sobran > 0) { recortado[0] = true; out.add("… " + sobran + " more items"); }
        return out;
    }

    // ── J2 (las salidas de una celda, S2) · `imagen` ────────────────────────
    // Los mismos topes que el agente de Python: una imagen que pasa de 512 KB se
    // reduce (a 1600 px de lado, y a JPEG si hace falta) y se dice.
    static final int IMAGEN_BYTES = 512 * 1024;
    static final int IMAGEN_LADO = 1600;

    /** El tipo de unos bytes, si son una imagen que un navegador pinta. */
    static String tipoDeImagen(byte[] b) {
        if (empieza(b, 0x89, 'P', 'N', 'G', '\r', '\n', 0x1a, '\n')) return "image/png";
        if (empieza(b, 0xff, 0xd8, 0xff)) return "image/jpeg";
        if (empieza(b, 'G', 'I', 'F', '8', '7', 'a') || empieza(b, 'G', 'I', 'F', '8', '9', 'a')) return "image/gif";
        if (b.length >= 12 && empieza(b, 'R', 'I', 'F', 'F') && b[8] == 'W' && b[9] == 'E' && b[10] == 'B' && b[11] == 'P') return "image/webp";
        String cabeza = new String(b, 0, Math.min(b.length, 1024), StandardCharsets.ISO_8859_1).stripLeading().toLowerCase(Locale.ROOT);
        if (cabeza.startsWith("<svg") || (cabeza.startsWith("<?xml") && cabeza.contains("<svg"))) return "image/svg+xml";
        return null;
    }

    private static boolean empieza(byte[] b, int... firma) {
        if (b.length < firma.length) return false;
        for (int i = 0; i < firma.length; i++) if ((b[i] & 0xff) != firma[i]) return false;
        return true;
    }

    /**
     * {@code {mime, base64, ancho, alto, bytes, reducida}} de una imagen —unos
     * {@code byte[]} que lo son, un {@code RenderedImage} (un {@code BufferedImage},
     * lo que da JFreeChart)—, o {@code null}. Una que pasa de {@code limite} se
     * reduce y se dice; si ni así cabe, no es imagen.
     */
    static Map<String, Object> comoImagen(Object valor, int limite) {
        byte[] b;
        String tipo;
        try {
            if (valor instanceof byte[] x) {
                b = x;
                tipo = tipoDeImagen(b);
                if (tipo == null) return null;
            } else if (valor instanceof java.awt.image.RenderedImage r) {
                java.io.ByteArrayOutputStream out = new java.io.ByteArrayOutputStream();
                if (!javax.imageio.ImageIO.write(r, "png", out)) return null;
                b = out.toByteArray();
                tipo = "image/png";
            } else {
                return null;
            }
        } catch (IOException | RuntimeException | LinkageError e) {
            return null;
        }
        int original = b.length;
        Integer ancho = null, alto = null;
        boolean reducida = false;
        if (!tipo.equals("image/svg+xml")) {
            try {
                java.awt.image.BufferedImage im = javax.imageio.ImageIO.read(new java.io.ByteArrayInputStream(b));
                if (im != null) {
                    ancho = im.getWidth();
                    alto = im.getHeight();
                    if (b.length > limite) {
                        for (Object[] intento : new Object[][] {{"png", IMAGEN_LADO}, {"jpg", IMAGEN_LADO}, {"jpg", 800}, {"jpg", 400}}) {
                            byte[] o = reducir(im, (String) intento[0], (Integer) intento[1]);
                            if (o != null && o.length <= limite) {
                                b = o;
                                tipo = intento[0].equals("png") ? "image/png" : "image/jpeg";
                                reducida = true;
                                break;
                            }
                        }
                    }
                }
            } catch (IOException | RuntimeException | LinkageError e) {
                // unos bytes que ImageIO no abre (un WEBP): van tal cual
            }
        }
        if (b.length > limite) return null;
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("mime", tipo);
        m.put("base64", java.util.Base64.getEncoder().encodeToString(b));
        m.put("ancho", ancho);
        m.put("alto", alto);
        m.put("bytes", original);
        m.put("reducida", reducida);
        return m;
    }

    /** La imagen con su lado mayor en {@code lado} como mucho, en {@code formato}. */
    static byte[] reducir(java.awt.image.BufferedImage im, String formato, int lado) throws IOException {
        double f = Math.min(1.0, (double) lado / Math.max(im.getWidth(), im.getHeight()));
        int w = Math.max(1, (int) Math.round(im.getWidth() * f)), h = Math.max(1, (int) Math.round(im.getHeight() * f));
        boolean jpg = formato.equals("jpg");
        java.awt.image.BufferedImage e = new java.awt.image.BufferedImage(w, h, jpg ? java.awt.image.BufferedImage.TYPE_INT_RGB : java.awt.image.BufferedImage.TYPE_INT_ARGB);
        java.awt.Graphics2D g = e.createGraphics();
        try {
            g.setRenderingHint(java.awt.RenderingHints.KEY_INTERPOLATION, java.awt.RenderingHints.VALUE_INTERPOLATION_BILINEAR);
            if (jpg) { g.setColor(java.awt.Color.WHITE); g.fillRect(0, 0, w, h); }
            g.drawImage(im, 0, 0, w, h, null);
        } finally {
            g.dispose();
        }
        java.io.ByteArrayOutputStream out = new java.io.ByteArrayOutputStream();
        return javax.imageio.ImageIO.write(e, formato, out) ? out.toByteArray() : null;
    }

    // ── J3 (las salidas de una celda, S3) · varias salidas en una celda ─────
    static final int PARTES_MAXIMAS = 50;
    /** Lo que caben todas juntas: bajo el cuerpo máximo de ore-serve (1 MB). */
    static final int PARTES_BYTES = 900 * 1024;

    /**
     * Lo que una celda enseña con {@code display()}, en orden y con el texto
     * impreso entre medias (como {@code Partes} en {@code agente.py}). Cada parte
     * es una salida de las de siempre, sin {@code ms}. Lo que no cabe —más de
     * {@link #PARTES_MAXIMAS}, o pasar de {@link #PARTES_BYTES}— no va, y se cuenta.
     */
    static final class Partes {
        final ByteArrayOutputStream salida;
        final long t0;
        final List<Map<String, Object>> lista = new ArrayList<>();
        int desde, bytes, fuera;

        Partes(ByteArrayOutputStream salida, long t0) { this.salida = salida; this.t0 = t0; }

        private void texto() {
            String todo = Kernel.texto(salida);
            if (todo.length() > desde) {
                String t = todo.substring(desde);
                desde = todo.length();
                Map<String, Object> p = new LinkedHashMap<>();
                p.put("tipo", "texto");
                p.put("texto", t);
                poner(p);
            }
        }

        private void poner(Map<String, Object> p) {
            int n = Json.escribir(p).getBytes(StandardCharsets.UTF_8).length;
            if (lista.size() >= PARTES_MAXIMAS || bytes + n > PARTES_BYTES) { fuera++; return; }
            lista.add(p);
            bytes += n;
        }

        void mostrar(Object valor) {
            if (valor == null) return;
            texto();
            Map<String, Object> p = Kernel.salidaDe(valor, true, "", t0);
            if ("imagen".equals(p.get("tipo"))) {
                // Una imagen, a lo que queda: más pequeña si hace falta para caber.
                int queda = PARTES_BYTES - bytes - 4096;
                if (String.valueOf(p.get("base64")).length() > queda) {
                    Map<String, Object> menor = comoImagen(valor, Math.max(16 * 1024, queda * 3 / 4));
                    if (menor != null) { menor.put("tipo", "imagen"); p = menor; }
                }
            }
            p.remove("ms");
            if ("".equals(p.get("texto")) && !"texto".equals(p.get("tipo"))) p.remove("texto");
            poner(p);
        }

        /** La salida de la celda: {@code varias}; si sólo hay una parte, ésa sola, como si fuera el último valor. */
        Map<String, Object> cerrar(Object valor, boolean hayValor) {
            if (hayValor) mostrar(valor);
            texto();
            if (lista.size() == 1 && fuera == 0) {
                Map<String, Object> p = new LinkedHashMap<>(lista.get(0));
                p.putIfAbsent("texto", "");
                p.put("ms", ms(t0));
                return p;
            }
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("tipo", "varias");
            m.put("partes", lista);
            m.put("fuera", fuera);
            m.put("ms", ms(t0));
            return m;
        }
    }

    // ── El servidor de lenguaje, y su correa (0037 ③b) ──────────────────────

    /**
     * Escucha el flujo del editor, atiende con {@link Lenguaje} y devuelve.
     *
     * <p>⛔ AGRUPANDO, y no de uno en uno: teclear son decenas de mensajes por
     * segundo, {@code ore-entrada} admite 64 conexiones a la vez y no tiene
     * keep-alive. Una petición por mensaje dejaría al inquilino sin plazas.
     *
     * <p>Se reconecta sola: el servidor se despide a los 240 s («vuelve») y lo
     * que se recoge por ahí SE CONSUME, así que no hay nada que retomar.
     */
    static void correa(Ore.Puesto p, Testigo testigo) {
        Lenguaje lenguaje = new Lenguaje();
        List<String> salientes = new ArrayList<>();
        Thread entrega = new Thread(() -> {
            while (true) {
                try { Thread.sleep(20); } catch (InterruptedException e) { return; }
                List<String> lote;
                synchronized (salientes) {
                    if (salientes.isEmpty()) continue;
                    lote = new ArrayList<>(salientes);
                    salientes.clear();
                }
                try {
                    p.cabeceras = testigo.cabeceras();
                    Ore.Respuesta r = p.pedir("POST", "/puestos/" + p.id + "/lsp/salida",
                        Map.of("mensajes", lote), Duration.ofSeconds(30));
                    if (r.codigo() != 200 && r.codigo() != 202) log("la salida del servidor de lenguaje no se aceptó: " + r.codigo() + " " + r.error());
                } catch (Exception e) {
                    log("no pude entregar " + lote.size() + " mensajes del servidor de lenguaje: " + e);
                }
            }
        }, "lsp-entrega");
        entrega.setDaemon(true);
        entrega.start();

        while (true) {
            try {
                HttpRequest.Builder b = HttpRequest.newBuilder(URI.create(p.servidor + "/puestos/" + p.id + "/lsp/agente"))
                    .timeout(Duration.ofMinutes(5)).header("accept", "text/event-stream");
                for (Map.Entry<String, String> e : testigo.cabeceras().entrySet()) b.header(e.getKey(), e.getValue());
                b.header("x-ore-puesto", p.id);
                HttpResponse<java.util.stream.Stream<String>> r =
                    Ore.HTTP.send(b.GET().build(), HttpResponse.BodyHandlers.ofLines());
                if (r.statusCode() != 200) { log("el flujo del servidor de lenguaje contestó " + r.statusCode() + ": vuelvo en 3 s"); dormir(3000); continue; }
                String[] evento = {""};
                r.body().forEach(linea -> {
                    if (linea.startsWith("event: ")) evento[0] = linea.substring(7);
                    else if (linea.startsWith("data: ") && evento[0].equals("lsp")) {
                        for (Map<String, Object> salida : lenguaje.atender(Json.objeto(linea.substring(6)))) {
                            synchronized (salientes) { salientes.add(Json.escribir(salida)); }
                        }
                    }
                });
            } catch (Exception e) {
                log("el flujo del servidor de lenguaje se cortó (" + e + "): vuelvo en 3 s");
                dormir(3000);
            }
        }
    }

    static void dormir(long ms) {
        try { Thread.sleep(ms); } catch (InterruptedException e) { Thread.currentThread().interrupt(); }
    }

    // ── El bucle ────────────────────────────────────────────────────────────
    public static void main(String[] args) {
        try {
            bucle(args);
        } catch (Throwable t) {
            // Por LOG y no por System.err: una celda puede dejar el err cambiado.
            LOG.println("agente · muerto: " + t);
            t.printStackTrace(LOG);
            System.exit(1);
        }
    }

    @SuppressWarnings("unchecked")
    static void bucle(String[] args) throws Exception {
        // J2: las imágenes (ImageIO, BufferedImage) sin pantalla: el puesto no la tiene.
        System.setProperty("java.awt.headless", "true");
        if (args.length > 0 && args[0].equals("--comprobar")) {
            Kernel k = new Kernel();
            Map<String, Object> r = k.correr("record P(String n, int e) {}\nvar xs = List.of(new P(\"a\", 1), new P(\"b\", 2));\nxs.size() * 21", "java");
            if (!"42".equals(String.valueOf(r.get("texto")))) throw new IllegalStateException("el kernel no contesta 42: " + Json.escribir(r));
            // J1: un record, un array y un Map salen como `json` (el árbol), y lo largo, recortado.
            Map<String, Object> j = k.correr("record Q(String n, int[] e, Map<String, Object> m) {}\nnew Q(\"a\", new int[]{1, 2}, Map.of(\"k\", List.of(true)))", "java");
            Map<String, Object> jl = k.correr("java.util.stream.IntStream.range(0, 600).boxed().toList()", "java");
            if (!"json".equals(j.get("tipo")) || !"{\"n\":\"a\",\"e\":[1,2],\"m\":{\"k\":[true]}}".equals(Json.escribir(j.get("valor")))
                || !"json".equals(jl.get("tipo")) || !Boolean.TRUE.equals(jl.get("recortado")) || ((List<?>) jl.get("valor")).size() != JSON_POR_NIVEL + 1)
                throw new IllegalStateException("un valor compuesto no sale como json: " + Json.escribir(j));
            // J2 y J3: un BufferedImage es una `imagen`, y display() da `varias`, en orden con lo impreso.
            Map<String, Object> im = k.correr("var im = new java.awt.image.BufferedImage(40, 30, java.awt.image.BufferedImage.TYPE_INT_RGB);\nim", "java");
            Map<String, Object> v = k.correr("System.out.println(\"a\");\ndisplay(Map.of(\"k\", 1), im);\n7", "java");
            List<String> tipos = new ArrayList<>();
            for (Object x : (List<?>) v.getOrDefault("partes", List.of())) tipos.add(String.valueOf(((Map<?, ?>) x).get("tipo")));
            if (!"imagen".equals(im.get("tipo")) || !Integer.valueOf(40).equals(im.get("ancho")) || !"varias".equals(v.get("tipo"))
                || !List.of("texto", "json", "imagen", "texto").equals(tipos))
                throw new IllegalStateException("una imagen o display() no salen como deben: " + Json.escribir(v).replaceAll("\"base64\":\"[^\"]*\"", "\"base64\":\"…\""));
            // Y el contrato de tipos por Arrow (0032 T3): si faltan los jars o el
            // --add-opens, se ve aquí y no en la primera celda de una persona.
            Ore.Rows f = Ore.sql("select 42::bigint n, 1.50::decimal(4,2) d, timestamp '2024-06-01 12:00:00'::timestamptz t");
            Map<String, Object> t = comoTabla(f);
            List<Object> fila = ((List<List<Object>>) t.get("filas")).get(0);
            if (!Long.valueOf(42).equals(f.get(0).get("n")) || !"decimal128(4, 2)".equals(f.types.get("d")) || f.tipos != f.types || !Long.valueOf(42).equals(fila.get(0)) || !"1.50".equals(fila.get(1)) || !String.valueOf(fila.get(2)).endsWith("Z"))
                throw new IllegalStateException("el sdk no cumple el contrato de tipos: " + Json.escribir(t));
            LOG.println("agente y sdk listos · " + Runtime.version() + " · duckdb " + (tieneDuckdb() ? "sí" : "no") + " · arrow y tipos ok");
            return;
        }
        Ore.Puesto p = Ore.puesto;
        if (p.id.isEmpty()) { log("sin PUESTO en el entorno"); System.exit(2); }
        long ttl = Long.parseLong(Ore.env("TTL", "1800"));
        // Un trabajo (W3.7 ④): `TRABAJO=<ruta>@<commit>` → una sola celda (el
        // fichero) y fuera, con el resultado como código de salida.
        String trabajo = Ore.env("TRABAJO", "").trim();
        if (!trabajo.isEmpty()) Ore.CODIGO = trabajo;
        Testigo testigo = new Testigo();
        Kernel kernel = new Kernel();
        // El servidor de lenguaje escucha desde el principio; no compila nada
        // hasta que el editor abre un fichero. Un trabajo no tiene editor.
        if (trabajo.isEmpty()) {
            Thread t = new Thread(() -> correa(p, testigo), "lsp");
            t.setDaemon(true);
            t.start();
        }
        log("puesto " + p.id + " · ore-serve " + p.servidor + " · TTL " + ttl + "s · almacén " + p.almacen + " · java " + Runtime.version());
        long ultimo = System.currentTimeMillis();
        while (true) {
            if (System.currentTimeMillis() - ultimo > ttl * 1000) {
                log("sin celdas durante " + ttl + "s: cierro");
                // Fuera de la cola: si no, Flux recrea el Job.
                try { p.cabeceras = testigo.cabeceras(); p.pedir("POST", "/puestos/" + p.id + "/cierre", null, Duration.ofSeconds(30)); }
                catch (Exception e) { log("no se pudo notificar el cierre: " + e); }
                return;
            }
            Ore.Respuesta r;
            try {
                p.cabeceras = testigo.cabeceras();
                r = p.pedir("GET", "/puestos/" + p.id + "/pendiente", null, Duration.ofSeconds(40));
            } catch (Exception e) {
                log("ore-serve no contesta (" + e.getMessage() + "): reintento en 5 s");
                Thread.sleep(5000);
                continue;
            }
            if (r.codigo() == 410) { log("el puesto está cerrado: adiós"); return; }
            if (r.codigo() != 200) {
                log("pendiente contestó " + r.codigo() + ": " + r.error() + " · reintento en 5 s");
                Thread.sleep(5000);
                continue;
            }
            if (p.persona.isEmpty()) {
                Ore.Respuesta f = p.pedir("GET", "/puestos/" + p.id, null, Duration.ofSeconds(30));
                Object quien = f.cuerpo().get("persona");
                if (f.codigo() == 200 && quien != null && !String.valueOf(quien).isEmpty()) { p.persona = String.valueOf(quien); log("el puesto es de " + p.persona); }
            }
            if (!Boolean.TRUE.equals(r.cuerpo().get("pendiente"))) continue;
            long n = ((Number) r.cuerpo().get("celda")).longValue();
            String texto = String.valueOf(r.cuerpo().getOrDefault("texto", ""));
            String lenguaje = String.valueOf(r.cuerpo().getOrDefault("lenguaje", "java"));
            log("celda " + n + " · " + lenguaje + " · " + texto.getBytes(StandardCharsets.UTF_8).length + " bytes");
            Map<String, Object> salida = kernel.correr(texto, lenguaje);
            ultimo = System.currentTimeMillis();
            try {
                p.cabeceras = testigo.cabeceras();
                Ore.Respuesta r3 = p.pedir("POST", "/puestos/" + p.id + "/celdas/" + n + "/salida", salida, Duration.ofSeconds(30));
                if (r3.codigo() != 200 && r3.codigo() != 201) log("la salida de la celda " + n + " no se aceptó: " + r3.codigo() + " " + r3.error());
            } catch (Exception e) {
                log("no pude entregar la salida de la celda " + n + ": " + e.getMessage());
            }
            log("celda " + n + " · " + salida.get("tipo") + " · " + salida.get("ms") + " ms");
            if (!trabajo.isEmpty()) {
                boolean error = "error".equals(salida.get("tipo"));
                log("trabajo " + trabajo + ": " + (error ? "error" : "hecho"));
                System.exit(error ? 1 : 0);
            }
        }
    }

    static boolean tieneDuckdb() {
        try { Class.forName("org.duckdb.DuckDBDriver"); return true; } catch (ClassNotFoundException e) { return false; }
    }
}
