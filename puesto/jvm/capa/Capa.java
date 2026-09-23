import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.time.Instant;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.TreeMap;
import java.util.TreeSet;
import java.util.zip.ZipEntry;
import java.util.zip.ZipFile;
import javax.xml.XMLConstants;
import javax.xml.parsers.DocumentBuilder;
import javax.xml.parsers.DocumentBuilderFactory;
import org.w3c.dom.Document;
import org.w3c.dom.Element;
import org.w3c.dom.Node;
import org.w3c.dom.NodeList;

/**
 * LA CAPA DE LA JVM (ADR 0037 ③c): lo que un repositorio de Java declara,
 * leído del árbol y convertido en un pom que Maven sepa resolver, y el informe
 * de lo que salió.
 *
 * <p>Vive en la imagen {@code capa-jvm:1} y corre DENTRO del Job
 * {@code 54-la-capa-jvm.yaml}. Tres verbos, y ni uno más:
 *
 * <pre>
 *   declarar &lt;árbol&gt; &lt;alcance&gt; &lt;trabajo&gt;   lee los pom del alcance → deps.txt · digest.txt
 *   pom      &lt;trabajo&gt; &lt;provisto.txt&gt;        escribe el pom que Maven resuelve
 *   informe  &lt;trabajo&gt; &lt;estado&gt;              lo resuelto → informe.json
 * </pre>
 *
 * <p>⭐ POR QUÉ EN JAVA Y NO EN EL GUION. Porque aquí hay un JDK y no hay
 * python, y porque lo que hace falta —analizar XML sin entidades externas,
 * resumir con SHA-256, mirar la versión de clase de un jar— lo trae la
 * biblioteca estándar. Un `sed` sobre XML sería el mismo error que un `grep`
 * sobre YAML.
 *
 * <p>⛔ LO QUE NO HACE: honrar nada de un pom que no sea {@code <dependencies>}.
 * Ni {@code <dependencyManagement>}, ni {@code <properties>}, ni
 * {@code <build>}, ni {@code <profiles>}. Es la misma regla que
 * {@code entorno.rs} aplica en el servidor, y las dos tienen que decir lo
 * mismo: si difieren, el Job avisa de que el árbol declara otro digest del que
 * se le pidió, y quien manda es el árbol.
 */
public final class Capa {

    /** El alfabeto de una coordenada. Estricto A PROPÓSITO: de aquí sale un
     *  fichero XML, y lo que entra lo escribe el cliente en su árbol. Es el
     *  mismo que comprueba `entorno.rs`, para que los dos digests coincidan. */
    private static boolean nombre(String s) {
        if (s.isEmpty()) {
            return false;
        }
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            boolean bien = (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z')
                    || (c >= '0' && c <= '9') || c == '.' || c == '_' || c == '-';
            if (!bien) {
                return false;
            }
        }
        return true;
    }

    /** Y de una versión, que además admite `+` (`1.2.3+build`). */
    private static boolean version(String s) {
        return !s.isEmpty() && s.chars().allMatch(c -> (c >= 'a' && c <= 'z')
                || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9')
                || c == '.' || c == '_' || c == '-' || c == '+');
    }

    // ── declarar ───────────────────────────────────────────────────────────

    /** Los mismos ficheros que `entorno::declaracion_en`: sin alcance, la
     *  celda entera (la raíz y todos los paquetes); con alcance, la raíz, su
     *  paquete y cada nivel hasta el repositorio. */
    private static List<Path> ficheros(Path arbol, String alcance) throws Exception {
        List<Path> fuera = new ArrayList<>();
        fuera.add(arbol.resolve("pom.xml"));
        String a = alcance == null ? "" : alcance.trim();
        while (a.startsWith("/")) {
            a = a.substring(1);
        }
        while (a.endsWith("/")) {
            a = a.substring(0, a.length() - 1);
        }
        if (a.isEmpty()) {
            Path paquetes = arbol.resolve("packages");
            if (Files.isDirectory(paquetes)) {
                try (var d = Files.list(paquetes)) {
                    d.sorted().forEach(p -> fuera.add(p.resolve("pom.xml")));
                }
            }
            return fuera;
        }
        String[] partes = a.split("/");
        if (partes.length >= 2 && partes[0].equals("packages")) {
            Path acc = arbol.resolve("packages").resolve(partes[1]);
            fuera.add(acc.resolve("pom.xml"));
            for (int i = 2; i < partes.length; i++) {
                acc = acc.resolve(partes[i]);
                fuera.add(acc.resolve("pom.xml"));
            }
        }
        return fuera;
    }

    /** Un analizador de XML que NO sale a buscar nada: sin DOCTYPE y sin
     *  entidades externas. Un pom lo escribe el cliente, y un `<!ENTITY>` que
     *  apunte a un fichero del pod convertiría leer un árbol en leerlo todo. */
    private static Document xml(Path f) throws Exception {
        DocumentBuilderFactory fab = DocumentBuilderFactory.newInstance();
        fab.setFeature("http://apache.org/xml/features/disallow-doctype-decl", true);
        fab.setFeature("http://xml.org/sax/features/external-general-entities", false);
        fab.setFeature("http://xml.org/sax/features/external-parameter-entities", false);
        fab.setAttribute(XMLConstants.ACCESS_EXTERNAL_DTD, "");
        fab.setAttribute(XMLConstants.ACCESS_EXTERNAL_SCHEMA, "");
        fab.setXIncludeAware(false);
        fab.setExpandEntityReferences(false);
        DocumentBuilder b = fab.newDocumentBuilder();
        b.setErrorHandler(null);
        return b.parse(f.toFile());
    }

    private static List<Element> hijos(Node padre, String nombre) {
        List<Element> fuera = new ArrayList<>();
        NodeList n = padre.getChildNodes();
        for (int i = 0; i < n.getLength(); i++) {
            if (n.item(i) instanceof Element e && nombre.equals(local(e))) {
                fuera.add(e);
            }
        }
        return fuera;
    }

    private static String local(Element e) {
        String n = e.getLocalName();
        return n != null ? n : e.getTagName();
    }

    private static String campo(Element dep, String nombre) {
        List<Element> h = hijos(dep, nombre);
        return h.isEmpty() ? "" : h.get(0).getTextContent().trim();
    }

    /**
     * `<dependencies>` de `<project>`, y nada más, como
     * `groupId:artifactId:version`.
     *
     * <p>Quedan fuera, cada una con su motivo: los ámbitos `test`, `provided` y
     * `system` (la capa es lo que hace falta para CORRER); lo que no trae
     * `<version>` (sin `<dependencyManagement>` no hay quien la fije); lo que
     * deja una propiedad sin resolver (`${x}`, porque no copiamos
     * `<properties>`); y lo que no encaja en el alfabeto de una coordenada.
     */
    static List<String> declaradas(Path f) {
        List<String> fuera = new ArrayList<>();
        Document d;
        try {
            d = xml(f);
        } catch (Exception e) {
            return fuera; // un pom roto no declara nada, y no tumba el Job.
        }
        Element raiz = d.getDocumentElement();
        if (raiz == null || !"project".equals(local(raiz))) {
            return fuera;
        }
        for (Element deps : hijos(raiz, "dependencies")) {
            for (Element dep : hijos(deps, "dependency")) {
                String g = campo(dep, "groupId");
                String a = campo(dep, "artifactId");
                String v = campo(dep, "version");
                String ambito = campo(dep, "scope");
                if (!(ambito.isEmpty() || ambito.equals("compile") || ambito.equals("runtime"))) {
                    continue;
                }
                if (!nombre(g) || !nombre(a) || !version(v)) {
                    continue;
                }
                fuera.add(g + ":" + a + ":" + v);
            }
        }
        return fuera;
    }

    /** `capa-<12 hex>` de la declaración, como lo nombra `entorno::digest_de`
     *  para la JVM: el entorno entra en el resumen. */
    static String digest(List<String> deps) throws Exception {
        if (deps.isEmpty()) {
            return "";
        }
        byte[] h = MessageDigest.getInstance("SHA-256")
                .digest(("jvm\n" + String.join("\n", deps)).getBytes(StandardCharsets.UTF_8));
        StringBuilder s = new StringBuilder("capa-");
        for (int i = 0; i < 6; i++) {
            s.append(String.format("%02x", h[i]));
        }
        return s.toString();
    }

    private static void declarar(Path arbol, String alcance, Path trabajo) throws Exception {
        var deps = new TreeSet<String>();
        for (Path f : ficheros(arbol, alcance)) {
            if (Files.isRegularFile(f)) {
                deps.addAll(declaradas(f));
            }
        }
        List<String> orden = new ArrayList<>(deps);
        String d = digest(orden);
        Files.writeString(trabajo.resolve("deps.txt"), String.join("\n", orden) + "\n");
        Files.writeString(trabajo.resolve("digest.txt"), d);
        System.out.println("### alcance: " + (alcance == null || alcance.isBlank() ? "(la celda)" : alcance));
        System.out.println("### declarado: " + (orden.isEmpty() ? "(nada)" : String.join(", ", orden))
                + " → " + (d.isEmpty() ? "(sin capa)" : d));
    }

    // ── pom ────────────────────────────────────────────────────────────────

    private static List<String> lineas(Path f) throws Exception {
        if (!Files.isRegularFile(f)) {
            return List.of();
        }
        List<String> fuera = new ArrayList<>();
        for (String l : Files.readAllLines(f, StandardCharsets.UTF_8)) {
            String t = l.trim();
            if (!t.isEmpty() && !t.startsWith("#")) {
                fuera.add(t);
            }
        }
        return fuera;
    }

    private static String dependencia(String gav, String ambito) {
        String[] p = gav.split(":");
        return "    <dependency><groupId>" + p[0] + "</groupId><artifactId>" + p[1]
                + "</artifactId><version>" + p[2] + "</version>"
                + (ambito.isEmpty() ? "" : "<scope>" + ambito + "</scope>")
                + "</dependency>\n";
    }

    /**
     * El pom que Maven resuelve: lo declarado, y DETRÁS lo que la imagen ya
     * pone, como `provided`.
     *
     * <p>⭐ `provided` en Maven quiere decir exactamente lo que aquí hace
     * falta: «el contenedor ya lo pone». Con `-DincludeScope=runtime` no se
     * copia, así que la capa no puede traer un segundo `jackson-databind` — y
     * si el repositorio PIDE otra versión de algo que la imagen pone, Maven
     * avisa («must be unique … 2.19.0 vs 2.18.2») y gana el contenedor, porque
     * lo suyo va escrito al final. Medido en
     * `pruebas-de-fuego/medida-la-capa-de-la-jvm.py` §7.
     */
    private static void pom(Path trabajo, Path provisto) throws Exception {
        StringBuilder s = new StringBuilder();
        s.append("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        s.append("<project xmlns=\"http://maven.apache.org/POM/4.0.0\">\n");
        s.append("  <modelVersion>4.0.0</modelVersion>\n");
        s.append("  <groupId>dev.ore</groupId><artifactId>capa</artifactId><version>0</version>\n");
        s.append("  <properties><project.build.sourceEncoding>UTF-8</project.build.sourceEncoding></properties>\n");
        s.append("  <dependencies>\n");
        s.append("    <!-- lo que el árbol declara -->\n");
        for (String gav : lineas(trabajo.resolve("deps.txt"))) {
            s.append(dependencia(gav, ""));
        }
        s.append("    <!-- y lo que la imagen del puesto ya pone: gana el contenedor -->\n");
        for (String gav : lineas(provisto)) {
            s.append(dependencia(gav, "provided"));
        }
        s.append("  </dependencies>\n</project>\n");
        Files.writeString(trabajo.resolve("pom.xml"), s.toString());
        System.out.println("### pom con " + lineas(trabajo.resolve("deps.txt")).size()
                + " declarada(s) y " + lineas(provisto).size() + " provista(s) por la imagen");
    }

    // ── informe ────────────────────────────────────────────────────────────

    private static String sha256(Path f) throws Exception {
        MessageDigest m = MessageDigest.getInstance("SHA-256");
        byte[] b = Files.readAllBytes(f);
        StringBuilder s = new StringBuilder();
        for (byte x : m.digest(b)) {
            s.append(String.format("%02x", x));
        }
        return s.toString();
    }

    /** La versión de clase de un jar, o 0. Un jar compilado para 25 no corre en
     *  nuestro 21, y eso se sabe AQUÍ —leyendo dos bytes— en vez de al primer
     *  `Run` de quien lo declaró. */
    private static int clase(Path jar) {
        try (ZipFile z = new ZipFile(jar.toFile())) {
            var e = z.entries();
            while (e.hasMoreElements()) {
                ZipEntry x = e.nextElement();
                if (x.getName().endsWith(".class") && !x.getName().startsWith("META-INF/")) {
                    try (InputStream in = z.getInputStream(x)) {
                        byte[] cab = in.readNBytes(8);
                        if (cab.length == 8) {
                            return ((cab[6] & 0xff) << 8) | (cab[7] & 0xff);
                        }
                    }
                }
            }
        } catch (Exception ignorado) {
            return 0;
        }
        return 0;
    }

    private static String texto(String s) {
        StringBuilder b = new StringBuilder("\"");
        for (char c : s.toCharArray()) {
            switch (c) {
                case '"' -> b.append("\\\"");
                case '\\' -> b.append("\\\\");
                case '\n' -> b.append("\\n");
                case '\r' -> b.append("\\r");
                case '\t' -> b.append("\\t");
                default -> {
                    if (c < 0x20) {
                        b.append(String.format("\\u%04x", (int) c));
                    } else {
                        b.append(c);
                    }
                }
            }
        }
        return b.append('"').toString();
    }

    private static String lista(List<String> xs) {
        List<String> t = new ArrayList<>();
        for (String x : xs) {
            t.add(texto(x));
        }
        return "[" + String.join(", ", t) + "]";
    }

    /**
     * El informe de la capa: lo que se resolvió, con qué sumas y qué avisos.
     *
     * <p>⭐ EL LOCK VA AQUÍ, y no es adorno: Maven vuelve a mediar versiones en
     * cada resolución, así que «la capa `capa-abc123`» sólo significa lo mismo
     * siempre si el informe lleva el conjunto exacto y el `sha256` de cada jar.
     * Es el equivalente del `requirements.lock` que la capa de Python escribe.
     */
    private static void informe(Path trabajo, String estado) throws Exception {
        List<String> deps = lineas(trabajo.resolve("deps.txt"));
        String digest = Files.isRegularFile(trabajo.resolve("digest.txt"))
                ? Files.readString(trabajo.resolve("digest.txt")).trim() : "";
        Path dir = trabajo.resolve("jars");
        Map<String, String> sumas = new TreeMap<>();
        long bytes = 0;
        List<String> avisos = new ArrayList<>();
        if (Files.isDirectory(dir)) {
            try (var d = Files.list(dir)) {
                for (Path j : d.sorted().toList()) {
                    if (!Files.isRegularFile(j)) {
                        continue;
                    }
                    sumas.put(j.getFileName().toString(), sha256(j));
                    bytes += Files.size(j);
                    int c = clase(j);
                    if (c > 65) {
                        avisos.add(j.getFileName() + " está compilado para Java " + (c - 44)
                                + " y el puesto lleva 21: no cargará");
                    }
                }
            }
        }
        // El conjunto exacto que Maven resolvió, un GAV por línea
        // (`dependency:list -DoutputFile`), que es lo que hace repetible «la
        // capa `capa-…`» aunque Central cambie mañana.
        List<String> lock = new ArrayList<>();
        for (String l : lineas(trabajo.resolve("lista.txt"))) {
            String[] p = l.split(":");
            if (p.length >= 5 && !p[4].equals("provided") && !p[4].equals("test")) {
                lock.add(p[0] + ":" + p[1] + ":" + p[3]);
            }
        }
        // ⭐ EL CHOQUE, EN UNA FRASE. Maven lo grita en su registro; si se queda
        //   ahí, quien declaró una versión que no ganó no se entera nunca.
        var choque = java.util.regex.Pattern.compile(
                "must be unique: ([^\\s:]+):([^\\s:]+):[^\\s]* -> version (\\S+) vs (\\S+)");
        for (String l : leerLog(trabajo.resolve("mvn.log"))) {
            var m = choque.matcher(l);
            if (m.find()) {
                String frase = "pediste " + m.group(1) + ":" + m.group(2) + " " + m.group(3)
                        + ", y esta sesión trae la " + m.group(4) + ": gana la de la sesión";
                if (!avisos.contains(frase)) {
                    avisos.add(frase);
                }
            }
        }
        long mb = (bytes + 1048575) / 1048576;
        long tope = Long.parseLong(System.getenv().getOrDefault("TOPE_MB", "512"));
        String error = "";
        if (estado.equals("error")) {
            error = ultimas(trabajo.resolve("mvn.log"), 800);
        } else if (mb > tope) {
            estado = "error";
            error = "la capa pesa " + mb + " MB y el tope es " + tope
                    + " MB: un puesto que tarda dos minutos en arrancar no es un puesto";
        }
        Map<String, String> j = new LinkedHashMap<>();
        j.put("estado", texto(estado));
        j.put("digest", texto(digest));
        j.put("declarado", lista(deps));
        j.put("jars", lista(new ArrayList<>(sumas.keySet())));
        j.put("lock", lista(lock));
        j.put("mb", texto(String.valueOf(mb)));
        j.put("avisos", lista(avisos));
        j.put("cuando", texto(Instant.now().toString().replaceAll("\\.\\d+Z$", "Z")));
        j.put("entorno", texto("puesto-jvm:1"));
        if (!error.isEmpty()) {
            j.put("error", texto(error));
        }
        StringBuilder sumasJson = new StringBuilder("{");
        boolean primero = true;
        for (var e : sumas.entrySet()) {
            sumasJson.append(primero ? "\n  " : ",\n  ").append(texto(e.getKey())).append(": ")
                    .append(texto(e.getValue()));
            primero = false;
        }
        j.put("sumas", sumasJson.append(sumas.isEmpty() ? "}" : "\n }").toString());
        List<String> campos = new ArrayList<>();
        for (var e : j.entrySet()) {
            campos.add(" " + texto(e.getKey()) + ": " + e.getValue());
        }
        Files.writeString(trabajo.resolve("informe.json"),
                "{\n" + String.join(",\n", campos) + "\n}\n");
        System.out.println("### informe " + estado + " · " + sumas.size() + " jar(s) · " + mb + " MB"
                + (avisos.isEmpty() ? "" : " · " + avisos.size() + " aviso(s)"));
        for (String a : avisos) {
            System.out.println("    ⚠️ " + a);
        }
    }

    private static List<String> leerLog(Path f) throws Exception {
        return Files.isRegularFile(f)
                ? Files.readAllLines(f, StandardCharsets.UTF_8) : List.of();
    }

    private static String ultimas(Path f, int cuantas) throws Exception {
        if (!Files.isRegularFile(f)) {
            return "sin registro de Maven";
        }
        String t = Files.readString(f, StandardCharsets.UTF_8);
        return t.length() <= cuantas ? t : t.substring(t.length() - cuantas);
    }

    // ── y el alfabeto de la imagen: `jars.txt` a coordenadas ───────────────

    /**
     * `puesto/jvm/jars.txt` —`grupo/artefacto versión`, la lista que el
     * Dockerfile ya usa para bajar los jars del puesto— convertida en las
     * coordenadas que un pom entiende. UNA LISTA, Y AHORA CUATRO LECTORES: que
     * la imagen y el que resuelve lean el mismo fichero es justo lo que evita
     * que el `provided` se quede corto el día que se añada un jar.
     */
    private static void provisto(Path jars, String duckdb) throws Exception {
        List<String> fuera = new ArrayList<>();
        for (String l : lineas(jars)) {
            String[] p = l.split("\\s+");
            if (p.length != 2) {
                continue;
            }
            int corte = p[0].lastIndexOf('/');
            if (corte <= 0) {
                continue;
            }
            fuera.add(p[0].substring(0, corte).replace('/', '.') + ":"
                    + p[0].substring(corte + 1) + ":" + p[1]);
        }
        if (duckdb != null && !duckdb.isBlank()) {
            fuera.add("org.duckdb:duckdb_jdbc:" + duckdb.trim());
        }
        System.out.println(String.join("\n", fuera));
    }

    public static void main(String[] a) throws Exception {
        String verbo = a.length == 0 ? "" : a[0].toLowerCase(Locale.ROOT);
        switch (verbo) {
            case "declarar" -> declarar(Path.of(a[1]), a.length > 2 ? a[2] : "", Path.of(a[3]));
            case "pom" -> pom(Path.of(a[1]), Path.of(a[2]));
            case "informe" -> informe(Path.of(a[1]), a[2]);
            case "provisto" -> provisto(Path.of(a[1]), a.length > 2 ? a[2] : "");
            default -> {
                System.err.println("uso: Capa declarar <árbol> <alcance> <trabajo>"
                        + " | pom <trabajo> <provisto.txt>"
                        + " | informe <trabajo> <estado>"
                        + " | provisto <jars.txt> [versión de duckdb]");
                System.exit(2);
            }
        }
    }

    private Capa() {
    }
}
