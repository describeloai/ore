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
 * ORE 0068 C2·3 pieza 4 · <b>El arnés de un build de Java en Spark</b>: lo que el de Lightweight
 * ({@code puesto/jvm/ore/Arnes.java}) hace, partido en dos para que lo conduzca el SDK de Python
 * ({@code ore/spark.py}, {@code _java_en_spark}), que corre Spark en este mismo proceso:
 * <ol>
 *   <li>{@link #preparar}: <b>compilar</b> el fichero del build con javac en la JVM de Spark
 *       (con el classpath del driver, que trae Spark, Sedona y este SDK) y del resto del
 *       repositorio sólo lo que usa; <b>cargar</b> la clase con D15 armado; y comprobar que el
 *       método es un {@code @Transform} {@code public static} sin parámetros;</li>
 *   <li>{@link Preparado#llamar}: llamarlo, una vez. El techo lo pone Python alrededor (el mismo
 *       {@code transform()} que un build de Python): lo que la anotación declara.</li>
 * </ol>
 * Cada fallo es una {@link Falla} con su tipo, su fichero y su línea, que Python lleva al informe.
 * Medido antes de escribirlo ({@code pruebas-de-fuego/geo/c2/spark-java/}): javac en el driver,
 * 0,8 s; un {@code Dataset<Row>} de ida y vuelta por py4j, por referencia. Interno.
 */
public final class ArnesDeSpark {
    private ArnesDeSpark() {}

    private static final Pattern PAQUETE = Pattern.compile("^\\s*package\\s+([\\w.]+)\\s*;", Pattern.MULTILINE);

    /** Un fallo del build, con lo que el informe lleva. */
    public static final class Falla extends RuntimeException {
        private final String tipo, fichero;
        private final Integer linea;

        Falla(String tipo, String mensaje, String fichero, Integer linea, Throwable causa) {
            super(mensaje, causa);
            this.tipo = tipo;
            this.fichero = fichero;
            this.linea = linea;
        }

        public String tipo() { return tipo; }

        public String fichero() { return fichero; }

        /** @return la línea, o -1 si no se sabe (py4j no da {@code null} de un {@code Integer} sin más) */
        public int linea() { return linea == null ? -1 : linea; }
    }

    /** Lo que {@link #preparar} deja listo para llamar. */
    public static final class Preparado {
        private final Method metodo;
        private final Transform t;
        private final String fichero;
        private final Map<String, Object> fuentes;

        Preparado(Method metodo, Transform t, String fichero, Map<String, Object> fuentes) {
            this.metodo = metodo;
            this.t = t;
            this.fichero = fichero;
            this.fuentes = fuentes;
        }

        public String[] inputs() { return t.inputs(); }

        public String output() { return t.output(); }

        /**
         * Llama al método. Lo que devuelve, en JSON si es lo que {@code write()} devuelve (un
         * {@code Map}); si no, {@code null}. Lo que lanza es {@code runtime}, con la línea de
         * su fichero.
         */
        public String llamar() {
            Object hecho;
            try {
                metodo.setAccessible(true);
                hecho = metodo.invoke(null);
            } catch (InvocationTargetException x) {
                Throwable c = x.getCause() == null ? x : x.getCause();
                String donde = donde(c, fichero, fuentes);
                throw new Falla("runtime", c.getClass().getSimpleName() + ": " + c.getMessage(), donde, lineaEn(c, donde), c);
            } catch (ReflectiveOperationException | RuntimeException x) {
                throw new Falla("runtime", x.getClass().getSimpleName() + ": " + x.getMessage(), fichero, lineaEn(x, fichero), x);
            }
            return hecho instanceof Map<?, ?> m ? Json.escribir(m) : null;
        }
    }

    /**
     * Compila, carga y halla el método que {@code especificacion} (JSON) nombra:
     * {@code {fichero, metodo, fuentes: {ruta: texto}}}, con las rutas desde la raíz del árbol.
     */
    @SuppressWarnings("unchecked")
    public static Preparado preparar(String especificacion) throws Exception {
        Map<String, Object> e = Json.objeto(especificacion);
        String fichero = String.valueOf(e.get("fichero"));
        String metodo = String.valueOf(e.get("metodo"));
        Map<String, Object> fuentes = (Map<String, Object>) e.getOrDefault("fuentes", Map.of());

        Path clases = compilar(fichero, fuentes, Files.createTempDirectory("ore-build"));

        String fuente = String.valueOf(fuentes.get(fichero));
        String simple = fichero.substring(fichero.lastIndexOf('/') + 1).replaceFirst("\\.java$", "");
        Matcher m = PAQUETE.matcher(fuente);
        String clase = m.find() ? m.group(1) + "." + simple : simple;
        URLClassLoader cargador = new URLClassLoader(new URL[] {clases.toUri().toURL()}, ArnesDeSpark.class.getClassLoader());
        Class<?> c;
        Ore.cargando = fichero;
        try {
            c = Class.forName(clase, true, cargador);
        } catch (ExceptionInInitializerError x) {
            Throwable causa = x.getCause() == null ? x : x.getCause();
            for (Throwable y = causa; y != null; y = y.getCause()) {
                if (y instanceof Ore.TransformCalledWhileLoading t) {
                    throw new Falla("called-while-loading", fichero + ": " + t.getMessage(), fichero, t.line(), t);
                }
            }
            throw new Falla("load", causa.getClass().getSimpleName() + " while loading the class: " + causa.getMessage(), fichero, lineaEn(causa, fichero), causa);
        } catch (ClassNotFoundException | LinkageError x) {
            throw new Falla("load", "`" + fichero + "` does not define the class `" + clase + "` (its file name): " + x, fichero, null, x);
        } finally {
            Ore.cargando = null;
        }

        Method dado = null;
        for (Method x : c.getDeclaredMethods()) {
            if (x.getName().equals(metodo) && x.getParameterCount() == 0) dado = x;
        }
        if (dado == null || !dado.isAnnotationPresent(Transform.class) || !Modifier.isStatic(dado.getModifiers())) {
            throw new Falla("not-a-transform", "`" + metodo + "` is not a `@Transform` method of " + fichero + " at this commit", fichero, null, null);
        }
        return new Preparado(dado, dado.getAnnotation(Transform.class), fichero, fuentes);
    }

    /**
     * Compilar el fichero del build, y del resto del repositorio sólo lo que usa: lo mismo que
     * {@code Arnes.compilar} de Lightweight (a javac va sólo {@code fichero}, los demás como
     * {@code -sourcepath} bajo su {@code package}), con el classpath de este proceso: el del
     * driver de Spark, que su lanzador ya da expandido (medido en el previo).
     */
    static Path compilar(String fichero, Map<String, Object> fuentes, Path dir) throws Exception {
        Path src = dir.resolve("src"), clases = dir.resolve("clases");
        Files.createDirectories(clases);
        if (!fuentes.containsKey(fichero)) throw new Falla("syntax", "`" + fichero + "` is not among the sources of the build", fichero, null, null);
        List<Map.Entry<String, Object>> orden = new ArrayList<>(fuentes.entrySet());
        orden.sort(java.util.Comparator.comparing(x -> !x.getKey().equals(fichero))); // la del build, primero
        Map<Path, String> deVuelta = new LinkedHashMap<>();
        Path objetivo = null;
        for (Map.Entry<String, Object> f : orden) {
            String r = f.getKey();
            if (r.startsWith("/") || r.contains("\\") || List.of(r.split("/")).contains("..") || !r.endsWith(".java")) {
                throw new Falla("syntax", "`" + r + "` is not a `.java` path of the tree", fichero, null, null);
            }
            String texto = String.valueOf(f.getValue());
            Matcher m = PAQUETE.matcher(texto);
            String paquete = m.find() ? m.group(1) : "";
            Path p = (paquete.isEmpty() ? src : src.resolve(paquete.replace('.', '/'))).resolve(r.substring(r.lastIndexOf('/') + 1));
            if (Files.exists(p)) continue;
            Files.createDirectories(p.getParent());
            Files.writeString(p, texto, StandardCharsets.UTF_8);
            deVuelta.put(p.toAbsolutePath().normalize(), r);
            if (r.equals(fichero)) objetivo = p;
        }
        JavaCompiler javac = ToolProvider.getSystemJavaCompiler();
        if (javac == null) throw new Falla("syntax", "this job runs without a Java compiler: rebuild the image", fichero, null, null);
        DiagnosticCollector<JavaFileObject> dichos = new DiagnosticCollector<>();
        try (StandardJavaFileManager fm = javac.getStandardFileManager(dichos, Locale.ENGLISH, StandardCharsets.UTF_8)) {
            List<String> opciones = List.of("-d", clases.toString(), "-cp", System.getProperty("java.class.path"),
                "-sourcepath", src.toString(), "-implicit:class",
                "-encoding", "UTF-8", "-proc:none", "-g", "-Xlint:none", "-nowarn");
            boolean bien = javac.getTask(null, fm, dichos, opciones, null, fm.getJavaFileObjectsFromPaths(List.of(objetivo))).call();
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
                throw new Falla("syntax", "CompilationError: " + mensaje.strip(), donde, linea, null);
            }
        }
        return clases;
    }

    /** La línea más alta de la traza que es de {@code fichero}, o {@code null}. */
    static Integer lineaEn(Throwable e, String fichero) {
        String nombre = fichero.substring(fichero.lastIndexOf('/') + 1);
        for (Throwable t = e; t != null; t = t.getCause()) {
            for (StackTraceElement m : t.getStackTrace()) {
                if (nombre.equals(m.getFileName()) && m.getLineNumber() > 0) return m.getLineNumber();
            }
        }
        return null;
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
}
