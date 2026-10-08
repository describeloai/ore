package ore;

import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.regex.Matcher;
import java.util.regex.Pattern;
import javax.tools.Diagnostic;
import javax.tools.DiagnosticCollector;
import javax.tools.JavaCompiler;
import javax.tools.JavaFileObject;
import javax.tools.StandardJavaFileManager;
import javax.tools.ToolProvider;

/**
 * <b>El arnés de un build de Java</b> (ORE 0055 T1·7, JT3): lo que corre la celda que
 * {@code ore-serve} escribe para un {@code Transform} con {@code runtime: java}. Interno: el
 * código del usuario no lo llama.
 *
 * <p>Lo mismo que el de Python, en el orden de Python:
 * <ol>
 *   <li><b>compilar</b> los {@code .java} del paquete con javac, en memoria del puesto (JT0:
 *       javac da la línea del fichero, JShell la de su snippet); un error es {@code syntax}
 *       con su fichero y su línea;</li>
 *   <li><b>cargar</b> la clase del fichero con D15 armado: un {@code transform()} o un
 *       {@code write()} mientras carga —un bloque {@code static}, un inicializador— falla
 *       ({@code called-while-loading}); otra excepción al cargar es {@code load};</li>
 *   <li>el método es un {@code @Transform} {@code public static} sin parámetros, o
 *       {@code not-a-transform};</li>
 *   <li><b>llamarlo</b> dentro de {@code transform(…)}, con lo que su anotación declara
 *       como techo; lo que lanza es {@code runtime}, con la línea de su fichero.</li>
 * </ol>
 * Cada fallo va al informe de la celda ({@code error: {tipo, fichero, linea}}) y como
 * excepción con {@code (fichero, line N)} en el texto. Lo escrito lleva la procedencia del
 * build ({@link Ore#BUILD}) y deja sus filas y su snapshot en el informe.
 */
public final class Arnes {
    private Arnes() {}

    private static final Pattern PAQUETE = Pattern.compile("^\\s*package\\s+([\\w.]+)\\s*;", Pattern.MULTILINE);

    /**
     * Construye el transform que {@code especificacion} (JSON) nombra:
     * {@code {fichero, metodo, build: {transform, entrypoint, output}, fuentes: {ruta: texto}}},
     * con {@code fichero} y cada ruta desde la raíz del árbol.
     *
     * @param especificacion lo que la celda trae
     * @throws Exception lo que impide construirlo, con su fichero y su línea
     */
    @SuppressWarnings("unchecked")
    public static void construir(String especificacion) throws Exception {
        Map<String, Object> e = Json.objeto(especificacion);
        String fichero = String.valueOf(e.get("fichero"));
        String metodo = String.valueOf(e.get("metodo"));
        Map<String, Object> fuentes = (Map<String, Object>) e.getOrDefault("fuentes", Map.of());
        Map<String, Object> build = new LinkedHashMap<>((Map<String, Object>) e.getOrDefault("build", Map.of()));
        String codigo = Ore.CODIGO != null ? Ore.CODIGO : System.getenv("ORE_CODIGO");
        build.put("id", Ore.puesto.id);
        build.put("commit", codigo == null ? "" : codigo.substring(codigo.lastIndexOf('@') + 1));
        Ore.BUILD = build;

        // ── 1 · compilar ─────────────────────────────────────────────────
        Path dir = Files.createTempDirectory("ore-build");
        Path src = dir.resolve("src"), clases = dir.resolve("clases");
        Files.createDirectories(clases);
        List<Path> rutas = new ArrayList<>();
        Map<Path, String> deVuelta = new LinkedHashMap<>();
        for (Map.Entry<String, Object> f : fuentes.entrySet()) {
            String r = f.getKey();
            if (r.startsWith("/") || r.contains("\\") || List.of(r.split("/")).contains("..") || !r.endsWith(".java")) {
                throw falla("syntax", "`" + r + "` is not a `.java` path of the tree", fichero, null);
            }
            Path p = src.resolve(r);
            Files.createDirectories(p.getParent());
            Files.writeString(p, String.valueOf(f.getValue()), StandardCharsets.UTF_8);
            rutas.add(p);
            deVuelta.put(p.toAbsolutePath().normalize(), r);
        }
        if (!fuentes.containsKey(fichero)) throw falla("syntax", "`" + fichero + "` is not among the sources of the build", fichero, null);
        JavaCompiler javac = ToolProvider.getSystemJavaCompiler();
        if (javac == null) throw falla("syntax", "this job runs without a Java compiler: rebuild the image", fichero, null);
        DiagnosticCollector<JavaFileObject> dichos = new DiagnosticCollector<>();
        try (StandardJavaFileManager fm = javac.getStandardFileManager(dichos, Locale.ENGLISH, StandardCharsets.UTF_8)) {
            List<String> opciones = List.of("-d", clases.toString(), "-cp", String.join(java.io.File.pathSeparator, Agente.Kernel.classpath()),
                "-encoding", "UTF-8", "-proc:none", "-g", "-Xlint:none", "-nowarn");
            boolean bien = javac.getTask(null, fm, dichos, opciones, null, fm.getJavaFileObjectsFromPaths(rutas)).call();
            if (!bien) {
                Diagnostic<? extends JavaFileObject> d = dichos.getDiagnostics().stream()
                    .filter(x -> x.getKind() == Diagnostic.Kind.ERROR).findFirst().orElse(null);
                String donde = fichero;
                Integer linea = null;
                String mensaje = "the sources do not compile";
                if (d != null) {
                    if (d.getSource() != null) donde = deVuelta.getOrDefault(Path.of(d.getSource().toUri()).toAbsolutePath().normalize(), fichero);
                    linea = d.getLineNumber() > 0 ? (int) d.getLineNumber() : null;
                    mensaje = d.getMessage(Locale.ENGLISH);
                }
                throw falla("syntax", "CompilationError: " + mensaje.strip(), donde, linea);
            }
        }

        // ── 2 · cargar, con D15 armado ───────────────────────────────────
        String fuente = String.valueOf(fuentes.get(fichero));
        String simple = fichero.substring(fichero.lastIndexOf('/') + 1).replaceFirst("\\.java$", "");
        Matcher m = PAQUETE.matcher(fuente);
        String clase = m.find() ? m.group(1) + "." + simple : simple;
        URLClassLoader cargador = new URLClassLoader(new URL[] {clases.toUri().toURL()}, Arnes.class.getClassLoader());
        Class<?> c;
        Ore.cargando = fichero;
        try {
            c = Class.forName(clase, true, cargador);
        } catch (ExceptionInInitializerError x) {
            Throwable causa = x.getCause() == null ? x : x.getCause();
            // D15, aunque el código la envuelva (un `catch` que la relanza en otra).
            for (Throwable y = causa; y != null; y = y.getCause()) {
                if (y instanceof Ore.TransformCalledWhileLoading t) {
                    Ore.paraElInforme(Map.of("error", error("called-while-loading", fichero, t.line())));
                    throw new IllegalStateException(fichero + ": " + t.getMessage()
                        + (t.line() != null ? " (" + fichero + ", line " + t.line() + ")" : ""));
                }
            }
            throw falla("load", causa.getClass().getSimpleName() + " while loading the class: " + causa.getMessage(), fichero, Ore.lineaEn(causa, fichero));
        } catch (ClassNotFoundException | LinkageError x) {
            throw falla("load", "`" + fichero + "` does not define the class `" + clase + "` (its file name): " + x, fichero, null);
        } finally {
            Ore.cargando = null;
        }

        // ── 3 · el método es un @Transform ───────────────────────────────
        Method metodoDado = null;
        for (Method x : c.getDeclaredMethods()) {
            if (x.getName().equals(metodo) && x.getParameterCount() == 0) metodoDado = x;
        }
        if (metodoDado == null || !metodoDado.isAnnotationPresent(Transform.class) || !Modifier.isStatic(metodoDado.getModifiers())) {
            throw falla("not-a-transform", "`" + metodo + "` is not a `@Transform` method of " + fichero + " at this commit", fichero, null);
        }
        Transform t = metodoDado.getAnnotation(Transform.class);
        String salida = String.valueOf(build.getOrDefault("output", t.output()));

        // ── 4 · llamarlo, una vez y dentro de su techo ───────────────────
        final Method aLlamar = metodoDado;
        aLlamar.setAccessible(true);
        Object hecho;
        try {
            hecho = Ore.transform(metodo, List.of(t.inputs()), t.output(), () -> aLlamar.invoke(null));
        } catch (InvocationTargetException x) {
            Throwable causa = x.getCause() == null ? x : x.getCause();
            throw falla("runtime", causa.getClass().getSimpleName() + ": " + causa.getMessage(), donde(causa, fichero, fuentes), Ore.lineaEn(causa, donde(causa, fichero, fuentes)));
        } catch (RuntimeException x) {
            throw falla("runtime", x.getClass().getSimpleName() + ": " + x.getMessage(), fichero, Ore.lineaEn(x, fichero));
        }
        if (hecho instanceof Map<?, ?> r && r.get("rows") != null) {
            System.out.println(salida + " · built from " + fichero + ":" + metodo + " · " + r.get("rows") + " rows"
                + (Boolean.TRUE.equals(r.get("repeated")) ? " · the same write: nothing new" : ""));
        } else {
            System.out.println(salida + " · built from " + fichero + ":" + metodo + " · the method did not return what `write()` returns");
        }
    }

    /** El fichero del árbol de la línea más alta de la traza que es de las fuentes del build. */
    private static String donde(Throwable e, String fichero, Map<String, Object> fuentes) {
        for (Throwable t = e; t != null; t = t.getCause()) {
            for (StackTraceElement m : t.getStackTrace()) {
                if (m.getFileName() == null) continue;
                for (String r : fuentes.keySet()) {
                    if (r.endsWith("/" + m.getFileName()) || r.equals(m.getFileName())) return r;
                }
            }
        }
        return fichero;
    }

    private static Map<String, Object> error(String tipo, String fichero, Integer linea) {
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("tipo", tipo);
        m.put("fichero", fichero);
        m.put("linea", linea);
        return m;
    }

    /** El error del build: al informe con su tipo, su fichero y su línea, y como excepción con {@code (fichero, line N)}. */
    private static RuntimeException falla(String tipo, String mensaje, String fichero, Integer linea) {
        Ore.paraElInforme(Map.of("error", error(tipo, fichero, linea)));
        return new IllegalStateException(linea != null ? mensaje + " (" + fichero + ", line " + linea + ")" : mensaje);
    }
}
