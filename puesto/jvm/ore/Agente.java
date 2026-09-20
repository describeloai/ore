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
            for (String p : System.getProperty("java.class.path", "").split(java.io.File.pathSeparator)) {
                if (!p.isBlank()) js.addToClasspath(p);
            }
            analisis = js.sourceCodeAnalysis();
            for (String s : List.of("import java.util.*;", "import java.util.stream.*;", "import static ore.Ore.*;")) js.eval(s);
        }

        Map<String, Object> correr(String texto, String lenguaje) {
            long t0 = System.nanoTime();
            ByteArrayOutputStream buffer = new ByteArrayOutputStream();
            PrintStream captura = new PrintStream(buffer, true, StandardCharsets.UTF_8);
            PrintStream err = System.err;
            System.setOut(captura);
            System.setErr(captura);
            try {
                Object valor;
                boolean hayValor;
                if (lenguaje.equals("sql")) {
                    valor = Ore.sql(texto);
                    hayValor = true;
                } else {
                    Resultado r = evaluar(texto);
                    if (r.error != null) return error(r.error, r.traza, buffer, t0);
                    valor = r.valor;
                    hayValor = r.hayValor;
                }
                return salidaDe(hayValor ? valor : null, hayValor, texto(buffer), t0);
            } catch (Exception e) {
                return error(e.getClass().getSimpleName() + ": " + e.getMessage(), traza(e), buffer, t0);
            } finally {
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
            if (!hayValor) {
                m.put("tipo", texto.isBlank() ? "vacia" : "texto");
                if (!texto.isBlank()) m.put("texto", texto);
                m.put("ms", ms(t0));
                return m;
            }
            m.put("tipo", "texto");
            m.put("texto", texto + (valor instanceof String s ? "\"" + s + "\"" : String.valueOf(valor)));
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

    /** La salida {@code tabla} del contrato: la hace el SDK ({@link Ore#tabla}); aquí sólo se le pone el límite de la consola. */
    static Map<String, Object> comoTabla(Object valor) { return Ore.tabla(valor, FILAS_MAXIMAS); }

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
        if (args.length > 0 && args[0].equals("--comprobar")) {
            Kernel k = new Kernel();
            Map<String, Object> r = k.correr("record P(String n, int e) {}\nvar xs = List.of(new P(\"a\", 1), new P(\"b\", 2));\nxs.size() * 21", "java");
            if (!"42".equals(String.valueOf(r.get("texto")))) throw new IllegalStateException("el kernel no contesta 42: " + Json.escribir(r));
            // Y el contrato de tipos por Arrow (0032 T3): si faltan los jars o el
            // --add-opens, se ve aquí y no en la primera celda de una persona.
            Ore.Filas f = Ore.sql("select 42::bigint n, 1.50::decimal(4,2) d, timestamp '2024-06-01 12:00:00'::timestamptz t");
            Map<String, Object> t = comoTabla(f);
            List<Object> fila = ((List<List<Object>>) t.get("filas")).get(0);
            if (!Long.valueOf(42).equals(f.get(0).get("n")) || !"decimal128(4, 2)".equals(f.tipos.get("d")) || !Long.valueOf(42).equals(fila.get(0)) || !"1.50".equals(fila.get(1)) || !String.valueOf(fila.get(2)).endsWith("Z"))
                throw new IllegalStateException("el sdk no cumple el contrato de tipos: " + Json.escribir(t));
            LOG.println("agente y sdk listos · " + Runtime.version() + " · duckdb " + (tieneDuckdb() ? "sí" : "no") + " · arrow y tipos ok");
            return;
        }
        Ore.Puesto p = Ore.puesto;
        if (p.id.isEmpty()) { log("sin PUESTO en el entorno"); System.exit(2); }
        long ttl = Long.parseLong(Ore.env("TTL", "1800"));
        Testigo testigo = new Testigo();
        Kernel kernel = new Kernel();
        log("puesto " + p.id + " · ore-serve " + p.servidor + " · TTL " + ttl + "s · almacén " + p.almacen + " · java " + Runtime.version());
        long ultimo = System.currentTimeMillis();
        while (true) {
            if (System.currentTimeMillis() - ultimo > ttl * 1000) { log("sin celdas durante " + ttl + "s: cierro"); return; }
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
        }
    }

    static boolean tieneDuckdb() {
        try { Class.forName("org.duckdb.DuckDBDriver"); return true; } catch (ClassNotFoundException e) { return false; }
    }
}
