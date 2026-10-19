package ore;

import java.lang.reflect.Method;
import java.net.URL;
import java.net.URLClassLoader;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
import javax.tools.Diagnostic;
import javax.tools.DiagnosticCollector;
import javax.tools.JavaCompiler;
import javax.tools.JavaFileObject;
import javax.tools.StandardJavaFileManager;
import javax.tools.ToolProvider;
import org.apache.spark.sql.Dataset;
import org.apache.spark.sql.Row;

/** El previo de la pieza 4: el SDK pesado de Java en lo justo para medir el puente. */
public final class Ore {
    private Ore() {}

    static Puente puente;

    public static void puente(Puente p) { puente = p; }

    @SuppressWarnings("unchecked")
    public static Dataset<Row> over(String nombre) { return (Dataset<Row>) puente.over(nombre); }

    public static String write(String nombre, Dataset<Row> datos) { return puente.write(nombre, datos, "overwrite"); }

    /** El classpath del driver, como lo ve la JVM (¿con comodines?). */
    public static String classpath() { return System.getProperty("java.class.path"); }

    /** Compilar `fuente` (clase `clase`) con javac en este proceso; devuelve el método `metodo`, o el error con su línea. */
    public static Method compilar(String clase, String fuente, String metodo, boolean expandir) throws Exception {
        Path src = Files.createTempDirectory("previo-src"), cls = Files.createTempDirectory("previo-cls");
        Path f = src.resolve(clase.replace('.', '/') + ".java");
        Files.createDirectories(f.getParent());
        Files.writeString(f, fuente, StandardCharsets.UTF_8);
        List<String> cp = new ArrayList<>();
        for (String p : classpath().split(java.io.File.pathSeparator)) {
            if (expandir && p.endsWith("/*")) {
                try (var d = Files.list(Path.of(p.substring(0, p.length() - 2)))) {
                    d.filter(x -> x.toString().endsWith(".jar")).forEach(x -> cp.add(x.toString()));
                }
            } else cp.add(p);
        }
        JavaCompiler javac = ToolProvider.getSystemJavaCompiler();
        if (javac == null) throw new IllegalStateException("sin javac");
        DiagnosticCollector<JavaFileObject> dichos = new DiagnosticCollector<>();
        try (StandardJavaFileManager fm = javac.getStandardFileManager(dichos, Locale.ENGLISH, StandardCharsets.UTF_8)) {
            boolean bien = javac.getTask(null, fm, dichos, List.of("-d", cls.toString(), "-cp", String.join(java.io.File.pathSeparator, cp),
                "-proc:none", "-g", "-nowarn"), null, fm.getJavaFileObjectsFromPaths(List.of(f))).call();
            if (!bien) {
                Diagnostic<? extends JavaFileObject> d = dichos.getDiagnostics().stream().filter(x -> x.getKind() == Diagnostic.Kind.ERROR).findFirst().orElseThrow();
                throw new IllegalStateException("CompilationError línea " + d.getLineNumber() + ": " + d.getMessage(Locale.ENGLISH));
            }
        }
        Class<?> c = Class.forName(clase, true, new URLClassLoader(new URL[] {cls.toUri().toURL()}, Ore.class.getClassLoader()));
        return c.getDeclaredMethod(metodo);
    }

    /** Llamar al método; lo que lance, con la línea de su fichero. */
    public static Object llamar(Method m, String fichero) throws Exception {
        try {
            return m.invoke(null);
        } catch (java.lang.reflect.InvocationTargetException x) {
            Throwable c = x.getCause();
            Integer linea = null;
            for (StackTraceElement e : c.getStackTrace()) if (fichero.equals(e.getFileName())) { linea = e.getLineNumber(); break; }
            throw new IllegalStateException("runtime línea " + linea + ": " + c.getClass().getSimpleName() + ": " + c.getMessage(), c);
        }
    }
}
