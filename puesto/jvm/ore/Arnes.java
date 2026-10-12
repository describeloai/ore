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
 *   <li><b>compilar</b> el {@code .java} del build con javac, en memoria del puesto (JT0:
 *       javac da la línea del fichero, JShell la de su snippet), y de los demás del
 *       repositorio <b>sólo lo que usa</b> ({@link #compilar}); un error es {@code syntax}
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
 *
 * <p><b>Preview</b> (JT4, D18–D21) es el mismo arnés sobre el código del editor, en la
 * sesión: sin procedencia de build, y con {@code write()} interceptado desde antes de cargar
 * la clase —no escribe; lo que escribiría (esquema, primeras filas, recuento) va al informe
 * como {@code preview}—. Exige lo mismo que el build (D21).
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
    public static void construir(String especificacion) throws Exception {
        correr(especificacion, false);
    }

    /**
     * El Preview del transform que {@code especificacion} nombra: como {@link #construir},
     * con {@code output} —la salida que el código del editor declara— y sin escribir nada.
     *
     * @param especificacion lo que la celda trae
     * @throws Exception lo que impide ensayarlo, con su fichero y su línea
     */
    public static void ensayar(String especificacion) throws Exception {
        correr(especificacion, true);
    }

    @SuppressWarnings("unchecked")
    private static void correr(String especificacion, boolean preview) throws Exception {
        Map<String, Object> e = Json.objeto(especificacion);
        String fichero = String.valueOf(e.get("fichero"));
        String metodo = String.valueOf(e.get("metodo"));
        Map<String, Object> fuentes = (Map<String, Object>) e.getOrDefault("fuentes", Map.of());
        Map<String, Object> build = new LinkedHashMap<>((Map<String, Object>) e.getOrDefault("build", Map.of()));
        if (preview) {
            // Un Preview no es un build: ni su procedencia ni su informe de build.
            Ore.BUILD = null;
            build.put("output", String.valueOf(e.get("output")));
        } else {
            String codigo = Ore.CODIGO != null ? Ore.CODIGO : System.getenv("ORE_CODIGO");
            build.put("id", Ore.puesto.id);
            build.put("commit", codigo == null ? "" : codigo.substring(codigo.lastIndexOf('@') + 1));
            Ore.BUILD = build;
        }
        try {
            if (preview) Ore.ensayo(String.valueOf(e.get("output")), metodo);
            fases(fichero, metodo, fuentes, build, preview);
        } finally {
            Map<String, Object> visto = preview ? Ore.finDelEnsayo() : null;
            if (preview && visto != null) Ore.paraElInforme(Map.of("preview", visto));
        }
    }

    /**
     * <b>Compilar el fichero del build, y del resto del repositorio sólo lo que usa</b>
     * (2026-10-12). Devuelve el directorio de las clases; un error es {@code syntax} con
     * el fichero y la línea de quien no compila.
     *
     * <p>Antes javac compilaba TODOS los {@code .java} del repositorio, y uno que nadie usa
     * —la semilla de la plantilla v6, un snippet de JShell que una actualización conserva—
     * tumbaba el build de cualquier transform con «unnamed classes are a preview feature»
     * (medido en {@code t-victor}, 2026-10-12). En Python se carga el fichero y lo que importa;
     * aquí, lo mismo: a javac va <b>sólo {@code fichero}</b>, y los demás como
     * {@code -sourcepath}, de donde javac saca lo que el fichero nombra
     * ({@code -implicit:class}: también sus clases).
     *
     * <p>⭐ Cada fuente va bajo {@code src/} por su {@code package} —sin él, en la raíz—,
     * porque javac busca en el {@code -sourcepath} por paquete y nombre, y en el árbol un
     * {@code .java} vive en su carpeta ({@code packages/<p>/<repo>/transforms/}). Si dos se
     * llaman igual en el mismo paquete, gana la del build; la otra sólo importaría si se usa.
     */
    static Path compilar(String fichero, Map<String, Object> fuentes, Path dir) throws Exception {
        Path src = dir.resolve("src"), clases = dir.resolve("clases");
        Files.createDirectories(clases);
        if (!fuentes.containsKey(fichero)) throw falla("syntax", "`" + fichero + "` is not among the sources of the build", fichero, null);
        List<Map.Entry<String, Object>> orden = new ArrayList<>(fuentes.entrySet());
        orden.sort(java.util.Comparator.comparing(e -> !e.getKey().equals(fichero))); // la del build, primero
        Map<Path, String> deVuelta = new LinkedHashMap<>();
        Path objetivo = null;
        for (Map.Entry<String, Object> f : orden) {
            String r = f.getKey();
            if (r.startsWith("/") || r.contains("\\") || List.of(r.split("/")).contains("..") || !r.endsWith(".java")) {
                throw falla("syntax", "`" + r + "` is not a `.java` path of the tree", fichero, null);
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
        if (javac == null) throw falla("syntax", "this job runs without a Java compiler: rebuild the image", fichero, null);
        DiagnosticCollector<JavaFileObject> dichos = new DiagnosticCollector<>();
        try (StandardJavaFileManager fm = javac.getStandardFileManager(dichos, Locale.ENGLISH, StandardCharsets.UTF_8)) {
            List<String> opciones = List.of("-d", clases.toString(), "-cp", String.join(java.io.File.pathSeparator, Agente.Kernel.classpath()),
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
                throw falla("syntax", "CompilationError: " + mensaje.strip(), donde, linea);
            }
        }
        return clases;
    }

    @SuppressWarnings("unchecked")
    private static void fases(String fichero, String metodo, Map<String, Object> fuentes, Map<String, Object> build, boolean preview) throws Exception {

        // ── 1 · compilar ─────────────────────────────────────────────────
        Path clases = compilar(fichero, fuentes, Files.createTempDirectory("ore-build"));

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
        if (preview) {
            Object visto = Ore.vistoDelEnsayo();
            if (visto == null) throw falla("not-written", "`" + metodo + "` returned without calling `write()`: there is nothing to preview", fichero, null);
            System.out.println(salida + " · preview of " + fichero + ":" + metodo + " · " + ((Map<String, Object>) visto).get("total") + " rows · nothing was written");
        } else {
            informeDelBuild(salida, fichero, metodo, hecho);
        }
    }

    /** Las filas (o los ítems) de lo escrito que un build enseña al acabar. */
    static final int FILAS_DE_LA_MUESTRA = 100;

    /** Lo que un {@code apply()} devuelve y el informe del build lleva. */
    private static final List<String> CLAVES_DEL_APPLY = List.of("items", "new", "recomputed", "skipped", "errors", "removed",
        "rows", "files_written", "files_retired", "written");

    private static long n(Object o) { return o instanceof Number x ? x.longValue() : 0; }

    /**
     * 0055 · Lo que un build deja dicho al acabar, lo mismo que Python ({@code _informe_del_build}):
     * la línea del log según lo que devolvió el método —un {@code write()}, un {@code apply()} o el
     * commit de una colección escrita— y, en el informe, el resumen de un {@code apply()} y la
     * MUESTRA de lo que quedó escrito: las primeras filas leídas del lago (no las que se iban a
     * escribir), o los primeros ítems de una colección, en la forma de {@code table()}, que la
     * consola pinta como en el Preview. Leer la muestra no tumba el build.
     */
    @SuppressWarnings("unchecked")
    static void informeDelBuild(String salida, String fichero, String metodo, Object hecho) {
        String base = salida + " · built from " + fichero + ":" + metodo;
        Map<String, Object> r = hecho instanceof Map<?, ?> m ? (Map<String, Object>) m : null;
        boolean coleccion = false;
        Long total = null;
        if (r != null && r.containsKey("skipped")) {
            // 0049 JM4: un apply(), con su resumen incremental.
            List<String> partes = new ArrayList<>();
            if (r.get("rows") != null) { partes.add(n(r.get("rows")) + " rows"); total = n(r.get("rows")); }
            if (r.get("files_written") != null) { partes.add(n(r.get("files_written")) + " files written"); coleccion = true; }
            partes.add("new " + n(r.get("new")));
            partes.add("recomputed " + n(r.get("recomputed")));
            partes.add("skipped " + n(r.get("skipped")));
            for (String k : List.of("errors", "removed")) if (n(r.get(k)) > 0) partes.add(k + " " + n(r.get(k)));
            if (!Boolean.TRUE.equals(r.get("written"))) partes.add("nothing new");
            System.out.println(base + " · " + String.join(" · ", partes));
            Map<String, Object> resumen = new LinkedHashMap<>();
            for (String k : CLAVES_DEL_APPLY) if (r.containsKey(k)) resumen.put(k, r.get(k));
            Ore.paraElInforme(Map.of("apply", resumen));
        } else if (r != null && r.get("transaccion") != null && r.get("items") instanceof Map<?, ?> its) {
            // 0049 JM3: la salida es una colección escrita, y lo que devolvió es su commit.
            System.out.println(base + " · transaction " + r.get("transaccion") + " · " + its.get("actuales") + " items"
                + (Boolean.TRUE.equals(r.get("sin_cambios")) ? " · nothing new" : ""));
            coleccion = true;
            total = n(its.get("actuales"));
        } else if (r != null && r.get("rows") != null) {
            System.out.println(base + " · " + r.get("rows") + " rows"
                + (Boolean.TRUE.equals(r.get("repeated")) ? " · the same write: nothing new" : ""));
            total = n(r.get("rows"));
        } else {
            System.out.println(base + " · the method did not return what `write()` returns");
            return;
        }
        try {
            Map<String, Object> muestra;
            if (coleccion) {
                List<Map<String, Object>> filas = new ArrayList<>();
                for (Media.Item it : Ore.collection(salida).items()) {
                    Media.MediaRef ref = it.ref();
                    Map<String, Object> f = new LinkedHashMap<>();
                    f.put("path", ref.path());
                    f.put("size", ref.size());
                    f.put("content_type", ref.contentType());
                    f.put("version", ref.version());
                    f.put("modified", ref.modified());
                    filas.add(f);
                    if (filas.size() >= FILAS_DE_LA_MUESTRA) break;
                }
                if (filas.isEmpty()) return;
                muestra = new LinkedHashMap<>(Ore.table(filas, FILAS_DE_LA_MUESTRA));
            } else {
                muestra = new LinkedHashMap<>(Ore.table(Ore.sql("select * from " + salida + " limit " + FILAS_DE_LA_MUESTRA,
                    FILAS_DE_LA_MUESTRA, false), FILAS_DE_LA_MUESTRA));
            }
            if (total != null) muestra.put("total", total);
            Ore.paraElInforme(Map.of("muestra", muestra));
        } catch (Exception | Error e) {
            System.out.println("(the sample of " + salida + " could not be read: " + e.getMessage() + ")");
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
