package ore;

import com.sun.source.tree.CompilationUnitTree;
import com.sun.source.tree.ExpressionTree;
import com.sun.source.tree.Tree;
import com.sun.source.util.JavacTask;
import com.sun.source.util.SourcePositions;
import com.sun.source.util.TreePath;
import com.sun.source.util.TreePathScanner;
import com.sun.source.util.Trees;
import java.net.URI;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import javax.lang.model.element.Element;
import javax.lang.model.element.ElementKind;
import javax.lang.model.element.ExecutableElement;
import javax.lang.model.element.TypeElement;
import javax.lang.model.type.TypeMirror;
import javax.tools.Diagnostic;
import javax.tools.DiagnosticCollector;
import javax.tools.JavaCompiler;
import javax.tools.JavaFileObject;
import javax.tools.SimpleJavaFileObject;
import javax.tools.ToolProvider;

/**
 * EL SERVIDOR DE LENGUAJE DE JAVA — <b>en esta misma JVM</b> (ORE 0037 ③b).
 *
 * <p>Python necesitó traer uno de fuera (pyright: 29 MB de paquete y 127 MB de
 * Node, porque su imagen no lo tenía). La JVM no: <b>el JDK ya trae las dos
 * piezas</b>, y las trae como API, no como programas aparte —
 *
 * <ul>
 *   <li>{@code javax.tools.JavaCompiler}: los diagnósticos, con línea y columna,
 *       sobre el texto que el editor tiene EN MEMORIA (no hace falta fichero);
 *   <li>{@code com.sun.source.util.Trees}: qué se ve desde una posición —las
 *       variables locales, los parámetros, los campos, los tipos importados— y
 *       de qué tipo es una expresión, que es el autocompletado.
 * </ul>
 *
 * <p><b>Medido antes de escribirlo</b> ({@code medida-el-java-del-arbol.py}):
 * jdtls hace lo mismo y algo más, pero pide 493–851 MB en una SEGUNDA JVM y
 * ~6 s de arranque, y apretarlo no sirve —con {@code -Xmx256m} y sin los
 * importadores de Maven y Gradle se queda en 514 MB, porque lo que pesa no es
 * el montón: son las clases de su framework OSGi—. Esto cuesta 0 MB de imagen,
 * 0 procesos y <b>75–107 ms en caliente</b> por vuelta completa (analizar +
 * mirar el alcance), contra los 140–265 MB que el agente ya ocupaba.
 *
 * <h2>⛔ Tres verbos, y ni uno más</h2>
 *
 * <p>{@code publishDiagnostics}, {@code completion} y {@code hover}. Renombrar,
 * ir a la definición, buscar referencias y los arreglos rápidos NO están, y no
 * se van a añadir aquí: el día que hagan falta, la respuesta es jdtls con un
 * proyecto de verdad detrás, no seguir escribiendo un IDE a mano.
 *
 * <h2>Lo que ve, y lo que no</h2>
 *
 * <p>Ve <b>el fichero que el editor le manda</b> y el classpath del propio
 * agente ({@code /opt/ore/clases} y {@code /opt/ore/lib/*}), que es donde vive
 * el SDK. No ve a los ficheros vecinos del repositorio: en el puesto no está el
 * repositorio, está la sesión (0037 ③a). Cuando lo esté, entra aquí y ya.
 */
final class Lenguaje {

    /** Lo que el editor tiene abierto: uri → texto de ahora. */
    private final Map<String, String> abiertos = new ConcurrentHashMap<>();
    private final JavaCompiler compilador = ToolProvider.getSystemJavaCompiler();
    /** El classpath del agente: ahí están el SDK y sus jars, ya resueltos. */
    private final String classpath = System.getProperty("java.class.path", "");

    /**
     * ⭐ LA PROSA DE LO NUESTRO, que es lo único que un servidor de fuera daría
     *   mejor que nosotros — y sólo si alguien le pone un {@code -sources.jar}
     *   al lado (medido). Aquí se dice y ya: son nueve funciones y son nuestras.
     */
    private static final Map<String, String> PROSA = Map.of(
        "over", "La copia de `<paquete>.<vista>` como filas. Sólo lo que el transform declaró en `inputs`.",
        "sql", "Una consulta sobre las copias de esta celda, con DuckDB. Lee; no escribe.",
        "write", "Escribe una tabla como `<paquete>.<nombre>` y devuelve el informe del commit. Sólo el `output` declarado.",
        "transform", "Declara qué lee y qué escribe, y lo ejecuta: mientras corre, la sesión sólo resuelve esos `inputs` y sólo deja escribir ese `output`.",
        "declare", "Declara un documento del árbol (una Entity, un TrainedModel…) como lo haría `ore apply`.",
        "persona", "Quién abrió este puesto (`persona:…`): la identidad con la que corre lo que escribes aquí.",
        "arrow", "La misma copia, como flujo de Arrow: los tipos sobreviven la vuelta.",
        "arrowSql", "Una consulta, como flujo de Arrow.");

    // ── el protocolo ────────────────────────────────────────────────────────

    /**
     * Un mensaje del editor. Devuelve lo que haya que mandarle de vuelta: la
     * respuesta si era una pregunta, y los avisos que salgan (diagnósticos).
     */
    List<Map<String, Object>> atender(Map<String, Object> m) {
        String metodo = String.valueOf(m.getOrDefault("method", ""));
        Object id = m.get("id");
        List<Map<String, Object>> fuera = new ArrayList<>();
        try {
            switch (metodo) {
                case "initialize" -> fuera.add(respuesta(id, capacidades()));
                case "initialized", "workspace/didChangeConfiguration" -> { }
                case "textDocument/didOpen" -> {
                    Map<String, Object> d = doc(m);
                    abiertos.put(uri(d), String.valueOf(d.getOrDefault("text", "")));
                    fuera.add(diagnosticos(uri(d)));
                }
                case "textDocument/didChange" -> {
                    Map<String, Object> d = doc(m);
                    Object cambios = params(m).get("contentChanges");
                    if (cambios instanceof List<?> l && !l.isEmpty()
                            && l.get(l.size() - 1) instanceof Map<?, ?> c && c.get("text") != null) {
                        abiertos.put(uri(d), String.valueOf(c.get("text")));
                    }
                    fuera.add(diagnosticos(uri(d)));
                }
                case "textDocument/didClose" -> abiertos.remove(uri(doc(m)));
                case "textDocument/completion" -> fuera.add(respuesta(id, completar(m)));
                case "textDocument/hover" -> fuera.add(respuesta(id, hover(m)));
                case "shutdown" -> fuera.add(respuesta(id, null));
                default -> {
                    // ⛔ A una PREGUNTA que no se entiende se le contesta igual: un
                    //   editor que espera una respuesta que no llega se queda
                    //   colgado, y eso se nota como «el editor va lento».
                    if (id != null) fuera.add(respuesta(id, null));
                }
            }
        } catch (RuntimeException e) {
            if (id != null) fuera.add(respuesta(id, null));
            Agente.log("el servidor de lenguaje tropezó con `" + metodo + "`: " + e);
        }
        fuera.removeIf(x -> x == null);
        return fuera;
    }

    private static Map<String, Object> capacidades() {
        return Map.of("capabilities", Map.of(
            // 1 = el editor manda el documento ENTERO en cada cambio, que es lo
            // que la consola hace (no lleva la cuenta de los rangos).
            "textDocumentSync", 1,
            "completionProvider", Map.of("triggerCharacters", List.of(".")),
            "hoverProvider", true));
    }

    // ── los diagnósticos: el compilador del propio JDK ───────────────────────

    /**
     * Compila EN MEMORIA y devuelve el aviso de LSP con lo que diga.
     *
     * <p>⛔ Siempre se manda, aunque no haya ni un error: un editor que no
     * recibe la lista vacía se queda con los subrayados de la vez anterior.
     */
    private Map<String, Object> diagnosticos(String uri) {
        String texto = abiertos.get(uri);
        if (texto == null) return null;
        DiagnosticCollector<JavaFileObject> recoge = new DiagnosticCollector<>();
        try {
            JavacTask t = tarea(uri, texto, recoge);
            t.parse();
            t.analyze();
        } catch (Exception e) {
            // Un fichero a medio escribir hace tropezar al compilador; lo que
            // haya recogido hasta ahí vale igual.
            Agente.log("analizando " + uri + ": " + e);
        }
        List<Object> lista = new ArrayList<>();
        int[] lineas = saltos(texto);
        for (Diagnostic<? extends JavaFileObject> d : recoge.getDiagnostics()) {
            long ini = d.getStartPosition(), fin = d.getEndPosition();
            if (ini < 0) ini = 0;
            if (fin < ini) fin = ini + 1;
            lista.add(Map.of(
                "range", Map.of("start", posicion(lineas, (int) ini), "end", posicion(lineas, (int) fin)),
                // 1 error · 2 aviso
                "severity", d.getKind() == Diagnostic.Kind.ERROR ? 1 : 2,
                "source", "javac",
                "message", d.getMessage(Locale.ENGLISH)));
        }
        return aviso("textDocument/publishDiagnostics", Map.of("uri", uri, "diagnostics", lista));
    }

    private JavacTask tarea(String uri, String texto, DiagnosticCollector<JavaFileObject> recoge) {
        JavaFileObject f = new SimpleJavaFileObject(URI.create("file:///" + nombre(uri)), JavaFileObject.Kind.SOURCE) {
            @Override public CharSequence getCharContent(boolean b) { return texto; }
        };
        return (JavacTask) compilador.getTask(null, null, recoge,
            List.of("-classpath", classpath, "-proc:none", "-nowarn"), null, List.of(f));
    }

    // ── el autocompletado: lo que se ve desde esa posición ───────────────────

    private Map<String, Object> completar(Map<String, Object> m) {
        String uri = uri(doc(m));
        String texto = abiertos.get(uri);
        if (texto == null) return Map.of("isIncomplete", false, "items", List.of());
        int[] lineas = saltos(texto);
        int cursor = desplazamiento(lineas, texto, params(m).get("position"));
        // Lo tecleado hasta ahora, y si viene detrás de un punto.
        int ini = cursor;
        while (ini > 0 && (Character.isJavaIdentifierPart(texto.charAt(ini - 1)))) ini--;
        String prefijo = texto.substring(ini, cursor);
        boolean trasPunto = ini > 0 && texto.charAt(ini - 1) == '.';

        List<Object> items = new ArrayList<>();
        try {
            DiagnosticCollector<JavaFileObject> recoge = new DiagnosticCollector<>();
            JavacTask t = tarea(uri, texto, recoge);
            CompilationUnitTree u = t.parse().iterator().next();
            t.analyze();
            Trees trees = Trees.instance(t);
            if (trasPunto) {
                // De qué tipo es lo que hay antes del punto, y sus miembros.
                TreePath ruta = rutaEn(u, trees, ini - 2);
                TypeMirror tipo = ruta == null ? null : trees.getTypeMirror(ruta);
                Element decl = tipo == null ? null : t.getTypes().asElement(tipo);
                if (decl instanceof TypeElement te) {
                    for (Element e : t.getElements().getAllMembers(te)) añade(items, e, prefijo);
                }
            } else {
                TreePath ruta = rutaEn(u, trees, cursor == 0 ? 0 : cursor - 1);
                for (com.sun.source.tree.Scope s = ruta == null ? null : trees.getScope(ruta);
                     s != null; s = s.getEnclosingScope()) {
                    for (Element e : s.getLocalElements()) añade(items, e, prefijo);
                }
            }
        } catch (Exception e) {
            Agente.log("completando en " + uri + ": " + e);
        }
        // ⛔ Con techo: una lista de mil cosas no es ayuda, es ruido, y viaja por
        //   el mismo conducto que todo lo demás.
        if (items.size() > 60) items = items.subList(0, 60);
        return Map.of("isIncomplete", items.size() >= 60, "items", items);
    }

    private void añade(List<Object> items, Element e, String prefijo) {
        String n = e.getSimpleName().toString();
        if (n.isEmpty() || n.equals("<init>") || !n.startsWith(prefijo)) return;
        if (items.size() > 200) return;
        Map<String, Object> i = new LinkedHashMap<>();
        i.put("label", n);
        // 3 función · 6 variable · 7 clase · 5 campo
        i.put("kind", switch (e.getKind()) {
            case METHOD, CONSTRUCTOR -> 3;
            case LOCAL_VARIABLE, PARAMETER, EXCEPTION_PARAMETER -> 6;
            case CLASS, INTERFACE, ENUM, RECORD -> 7;
            case FIELD, ENUM_CONSTANT -> 5;
            default -> 1;
        });
        i.put("detail", firma(e));
        String prosa = PROSA.get(n);
        if (prosa != null && esNuestro(e)) i.put("documentation", prosa);
        items.add(i);
    }

    // ── el hover: la firma, y la prosa si es nuestra ─────────────────────────

    private Map<String, Object> hover(Map<String, Object> m) {
        String uri = uri(doc(m));
        String texto = abiertos.get(uri);
        if (texto == null) return null;
        int[] lineas = saltos(texto);
        int cursor = desplazamiento(lineas, texto, params(m).get("position"));
        try {
            DiagnosticCollector<JavaFileObject> recoge = new DiagnosticCollector<>();
            JavacTask t = tarea(uri, texto, recoge);
            CompilationUnitTree u = t.parse().iterator().next();
            t.analyze();
            Trees trees = Trees.instance(t);
            TreePath ruta = rutaEn(u, trees, cursor);
            Element e = ruta == null ? null : trees.getElement(ruta);
            if (e == null) return null;
            StringBuilder s = new StringBuilder("```java\n").append(firma(e)).append("\n```");
            String prosa = PROSA.get(e.getSimpleName().toString());
            if (prosa != null && esNuestro(e)) s.append("\n\n").append(prosa);
            return Map.of("contents", Map.of("kind", "markdown", "value", s.toString()));
        } catch (Exception e) {
            Agente.log("hover en " + uri + ": " + e);
            return null;
        }
    }

    private static boolean esNuestro(Element e) {
        Element d = e.getEnclosingElement();
        return d != null && d.toString().startsWith("ore.");
    }

    private static String firma(Element e) {
        if (e instanceof ExecutableElement x) {
            StringBuilder s = new StringBuilder();
            s.append(x.getReturnType()).append(' ').append(x.getSimpleName()).append('(');
            for (int i = 0; i < x.getParameters().size(); i++) {
                if (i > 0) s.append(", ");
                s.append(x.getParameters().get(i).asType()).append(' ').append(x.getParameters().get(i).getSimpleName());
            }
            return s.append(')').toString();
        }
        if (e.getKind() == ElementKind.LOCAL_VARIABLE || e.getKind() == ElementKind.PARAMETER
                || e.getKind() == ElementKind.FIELD) {
            return e.asType() + " " + e.getSimpleName();
        }
        return e.toString();
    }

    // ── lo de siempre: posiciones, rutas y sobres ────────────────────────────

    private static TreePath rutaEn(CompilationUnitTree u, Trees trees, int p) {
        SourcePositions pos = trees.getSourcePositions();
        final TreePath[] fuera = {null};
        new TreePathScanner<Void, Void>() {
            @Override public Void scan(Tree t, Void v) {
                if (t != null) {
                    long a = pos.getStartPosition(u, t), b = pos.getEndPosition(u, t);
                    // El más pequeño que contiene la posición: el último que gana.
                    if (a >= 0 && a <= p && p <= b && (t instanceof ExpressionTree || fuera[0] == null)) {
                        fuera[0] = new TreePath(getCurrentPath(), t);
                    }
                }
                return super.scan(t, v);
            }
        }.scan(u, null);
        return fuera[0];
    }

    /** Dónde empieza cada línea. LSP cuenta líneas y columnas; javac, caracteres. */
    private static int[] saltos(String texto) {
        List<Integer> l = new ArrayList<>();
        l.add(0);
        for (int i = 0; i < texto.length(); i++) if (texto.charAt(i) == '\n') l.add(i + 1);
        int[] r = new int[l.size()];
        for (int i = 0; i < r.length; i++) r[i] = l.get(i);
        return r;
    }

    private static Map<String, Object> posicion(int[] lineas, int desplazamiento) {
        int n = 0;
        while (n + 1 < lineas.length && lineas[n + 1] <= desplazamiento) n++;
        return Map.of("line", n, "character", desplazamiento - lineas[n]);
    }

    private static int desplazamiento(int[] lineas, String texto, Object posicion) {
        if (!(posicion instanceof Map<?, ?> p)) return 0;
        int l = numero(p.get("line")), c = numero(p.get("character"));
        if (l >= lineas.length) return texto.length();
        return Math.min(texto.length(), lineas[l] + c);
    }

    private static int numero(Object o) { return o instanceof Number n ? n.intValue() : 0; }

    @SuppressWarnings("unchecked")
    private static Map<String, Object> params(Map<String, Object> m) {
        Object p = m.get("params");
        return p instanceof Map<?, ?> x ? (Map<String, Object>) x : Map.of();
    }

    @SuppressWarnings("unchecked")
    private static Map<String, Object> doc(Map<String, Object> m) {
        Object d = params(m).get("textDocument");
        return d instanceof Map<?, ?> x ? (Map<String, Object>) x : Map.of();
    }

    private static String uri(Map<String, Object> d) { return String.valueOf(d.getOrDefault("uri", "")); }

    /** El nombre del fichero ES el nombre de la clase pública: javac lo exige. */
    private static String nombre(String uri) {
        String s = uri.substring(uri.lastIndexOf('/') + 1);
        return s.isEmpty() ? "Ejemplo.java" : s;
    }

    private static Map<String, Object> respuesta(Object id, Object resultado) {
        if (id == null) return null;
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("jsonrpc", "2.0");
        m.put("id", id);
        m.put("result", resultado);
        return m;
    }

    private static Map<String, Object> aviso(String metodo, Object params) {
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("jsonrpc", "2.0");
        m.put("method", metodo);
        m.put("params", params);
        return m;
    }
}
