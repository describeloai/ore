package ore;

import java.io.IOException;
import java.net.URI;
import java.net.URLEncoder;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.sql.Connection;
import java.sql.DriverManager;
import java.sql.ResultSet;
import java.sql.ResultSetMetaData;
import java.sql.SQLException;
import java.sql.Statement;
import java.time.Duration;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.TreeSet;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * {@code ore} · el SDK del puesto para Java (0031 W3.4). El mismo contrato que
 * {@code puesto/python/ore}; la sesión lo importa estático
 * ({@code import static ore.Ore.*;}), así que una celda escribe
 * {@code sql("select …")}, {@code over("hr.espanoles")} y {@code persona()}.
 *
 * <ul>
 *   <li>{@code over("<paquete>.<vista>")} → las filas de la copia de esa vista
 *       ({@code List<Map<String,Object>>}), leídas por DuckDB (JDBC).</li>
 *   <li>{@code sql("select … from p.v")} → las filas del resultado: cada
 *       {@code paquete.vista} tras FROM/JOIN se resuelve, se baja una vez y
 *       queda como vista de DuckDB.</li>
 *   <li>{@code persona()} → quién abrió el puesto ({@code persona:…}).</li>
 * </ul>
 *
 * El código nunca ve el bucket ni una credencial: pregunta a {@code ore-serve}
 * QUÉ copia es (con la identidad del puesto, que la resuelve en nombre de la
 * persona) y baja el artefacto con la identidad del pod (el token del servidor
 * de metadatos y la API JSON de GCS). El sobre {@code ORECOPY1} se desenvuelve
 * aquí. Fuera del clúster (las pruebas) {@code ORE_ALMACEN=dir:/ruta}.
 */
public final class Ore {
    private Ore() {}

    /** Lo que el agente sabe de sí. */
    public static final class Puesto {
        public final String servidor = env("ORE_SERVE", "http://127.0.0.1:8080").replaceAll("/+$", "");
        public final String id = env("PUESTO", "");
        public final String bucket = env("BUCKET", "");
        public final String almacen = env("ORE_ALMACEN", "gcs");
        /** Quién abrió el puesto: lo pone el agente al reclamarlo (de la ficha). */
        public volatile String persona = "";
        /** El token lo pone el agente y lo renueva; una celda no lo ve. */
        volatile Map<String, String> cabeceras = Map.of();

        /** {@code [código, cuerpo]}: el cuerpo, JSON como mapa (o {@code {error}}). */
        public Respuesta pedir(String metodo, String ruta, Object cuerpo, Duration plazo) throws IOException, InterruptedException {
            HttpRequest.Builder b = HttpRequest.newBuilder(URI.create(servidor + ruta)).timeout(plazo).header("accept", "application/json");
            for (Map.Entry<String, String> e : cabeceras.entrySet()) b.header(e.getKey(), e.getValue());
            if (cuerpo == null) b.method(metodo, HttpRequest.BodyPublishers.noBody());
            else b.header("content-type", "application/json").method(metodo, HttpRequest.BodyPublishers.ofString(Json.escribir(cuerpo)));
            HttpResponse<String> r = HTTP.send(b.build(), HttpResponse.BodyHandlers.ofString());
            String t = r.body();
            return new Respuesta(r.statusCode(), t == null || t.isBlank() ? new LinkedHashMap<>() : Json.objeto(t));
        }
    }

    public record Respuesta(int codigo, Map<String, Object> cuerpo) {
        public String error() { Object e = cuerpo.get("error"); return e == null ? cuerpo.toString() : String.valueOf(e); }
    }

    static final HttpClient HTTP = HttpClient.newBuilder().connectTimeout(Duration.ofSeconds(10)).build();
    public static final Puesto puesto = new Puesto();

    static String env(String k, String d) { String v = System.getenv(k); return v == null || v.isEmpty() ? d : v; }

    /** Quién abrió el puesto ({@code persona:…}): la identidad con la que corre lo que haces aquí. */
    public static String persona() {
        if (puesto.persona.isEmpty()) throw new IllegalStateException("persona(): el agente aún no sabe quién abrió el puesto");
        return puesto.persona;
    }

    // ── el almacén ──────────────────────────────────────────────────────────
    private static String tokenDelPod = "";
    private static long caducaElToken = 0;

    private static synchronized String tokenDeGoogle() throws IOException, InterruptedException {
        if (System.currentTimeMillis() < caducaElToken - 60_000) return tokenDelPod;
        HttpRequest q = HttpRequest.newBuilder(URI.create("http://169.254.169.254/computeMetadata/v1/instance/service-accounts/default/token"))
            .header("Metadata-Flavor", "Google").timeout(Duration.ofSeconds(10)).GET().build();
        HttpResponse<String> r = HTTP.send(q, HttpResponse.BodyHandlers.ofString());
        if (r.statusCode() != 200) throw new IOException("el servidor de metadatos contestó " + r.statusCode());
        Map<String, Object> t = Json.objeto(r.body());
        tokenDelPod = String.valueOf(t.get("access_token"));
        caducaElToken = System.currentTimeMillis() + ((Number) t.getOrDefault("expires_in", 300L)).longValue() * 1000;
        return tokenDelPod;
    }

    private static byte[] bajar(String bucket, String clave) throws IOException, InterruptedException {
        if (puesto.almacen.startsWith("dir:")) {
            return Files.readAllBytes(Path.of(puesto.almacen.substring(4).replaceAll("/+$", ""), clave));
        }
        if (puesto.almacen.equals("gcs")) {
            String url = "https://storage.googleapis.com/storage/v1/b/" + URLEncoder.encode(bucket, StandardCharsets.UTF_8)
                + "/o/" + URLEncoder.encode(clave, StandardCharsets.UTF_8) + "?alt=media";
            HttpRequest q = HttpRequest.newBuilder(URI.create(url)).header("authorization", "Bearer " + tokenDeGoogle()).timeout(Duration.ofMinutes(10)).GET().build();
            HttpResponse<byte[]> r = HTTP.send(q, HttpResponse.BodyHandlers.ofByteArray());
            if (r.statusCode() != 200) throw new IOException("GCS contestó " + r.statusCode() + " por " + clave);
            return r.body();
        }
        throw new IllegalStateException("ORE_ALMACEN=" + puesto.almacen + " no es un almacén: vale `gcs` o `dir:<ruta>`");
    }

    /** El sobre {@code ORECOPY1}: 8 de magia, 4 de largo (LE), la cabecera JSON, la carga Parquet. */
    private static byte[] desenvolver(byte[] crudo) {
        if (crudo.length < 12 || !new String(crudo, 0, 8, StandardCharsets.ISO_8859_1).equals("ORECOPY1"))
            throw new IllegalArgumentException("el artefacto no es una copia de ORE (sin `ORECOPY1`)");
        int n = (crudo[8] & 0xff) | (crudo[9] & 0xff) << 8 | (crudo[10] & 0xff) << 16 | (crudo[11] & 0xff) << 24;
        byte[] carga = new byte[crudo.length - 12 - n];
        System.arraycopy(crudo, 12 + n, carga, 0, carga.length);
        return carga;
    }

    private static Map<String, Object> resolver(String vista) throws IOException, InterruptedException {
        if (vista == null || vista.chars().filter(c -> c == '.').count() != 1)
            throw new IllegalArgumentException("se quiere `<paquete>.<vista>`, no " + vista);
        Respuesta r = puesto.pedir("GET", "/puestos/" + puesto.id + "/datos/" + vista, null, Duration.ofSeconds(30));
        if (r.codigo() == 409) throw new IllegalStateException("la copia de `" + vista + "` no está hecha: " + r.error());
        if (r.codigo() == 404) throw new IllegalArgumentException("no hay ninguna `View` `" + vista + "` en el árbol");
        if (r.codigo() != 200) throw new IOException("ore-serve contestó " + r.codigo() + " por `" + vista + "`: " + r.error());
        return r.cuerpo();
    }

    private static Path copiasPorDefecto() {
        String c = System.getenv("ORE_COPIAS");
        if (c != null && !c.isEmpty()) return Path.of(c);
        Path t = Path.of("/trabajo");
        if (Files.isDirectory(t) && Files.isWritable(t)) return t.resolve("copias");
        return Path.of(System.getProperty("java.io.tmpdir"), "ore-copias");
    }

    /** La copia de la vista como Parquet local, bajado UNA vez por sesión (la clave es el digest del artefacto). */
    private static Path parquetDe(String vista) throws IOException, InterruptedException {
        Map<String, Object> r = resolver(vista);
        Path d = copiasPorDefecto();
        Files.createDirectories(d);
        String clave = String.valueOf(r.get("clave"));
        Path f = d.resolve(clave.replace("/", "_") + ".parquet");
        if (!Files.exists(f)) {
            Object b = r.get("bucket");
            byte[] carga = desenvolver(bajar(b == null || String.valueOf(b).isEmpty() ? puesto.bucket : String.valueOf(b), clave));
            Path parte = d.resolve(f.getFileName() + ".parte");
            Files.write(parte, carga);
            Files.move(parte, f, StandardCopyOption.REPLACE_EXISTING, StandardCopyOption.ATOMIC_MOVE);
        }
        return f;
    }

    // ── DuckDB ──────────────────────────────────────────────────────────────
    private static Connection conexion;

    private static synchronized Connection duckdb() throws SQLException {
        if (conexion == null) {
            try { Class.forName("org.duckdb.DuckDBDriver"); } catch (ClassNotFoundException e) {
                throw new SQLException("este puesto no trae DuckDB (duckdb_jdbc.jar en el classpath): sql() y over() no pueden leer Parquet");
            }
            conexion = DriverManager.getConnection("jdbc:duckdb:");
            String hilos = System.getenv("ORE_HILOS");
            if (hilos != null && !hilos.isEmpty()) try (Statement s = conexion.createStatement()) { s.execute("set threads to " + Integer.parseInt(hilos)); }
        }
        return conexion;
    }

    private static final Pattern VISTAS_EN_SQL = Pattern.compile("(?i)\\b(?:from|join)\\s+([a-z_][a-z0-9_]*)\\.([a-z_][a-z0-9_]*)\\b");

    private static String rutaSql(Path f) { return f.toString().replace("\\", "/").replace("'", "''"); }

    /** Las filas de un resultado, con valores llanos (JSON): {@code List<Map>} en el orden de las columnas. */
    private static List<Map<String, Object>> filasDe(Connection con, String texto) throws SQLException {
        try (Statement s = con.createStatement()) {
            boolean hay = s.execute(texto);
            if (!hay) return new ArrayList<>();
            try (ResultSet rs = s.getResultSet()) {
                ResultSetMetaData md = rs.getMetaData();
                int n = md.getColumnCount();
                List<Map<String, Object>> filas = new ArrayList<>();
                while (rs.next()) {
                    Map<String, Object> fila = new LinkedHashMap<>();
                    for (int i = 1; i <= n; i++) fila.put(md.getColumnLabel(i), llano(rs.getObject(i)));
                    filas.add(fila);
                }
                return filas;
            }
        }
    }

    /** Un valor de JDBC → algo que JSON entiende (los agregados de DuckDB son BigDecimal/HugeInt). */
    public static Object llano(Object v) {
        if (v == null || v instanceof String || v instanceof Boolean || v instanceof Integer || v instanceof Long || v instanceof Double) return v;
        if (v instanceof java.math.BigDecimal d) return d.scale() <= 0 || d.stripTrailingZeros().scale() <= 0 ? (Object) d.longValueExact() : (Object) d.doubleValue();
        if (v instanceof java.math.BigInteger b) return b.bitLength() < 63 ? (Object) b.longValue() : (Object) b.toString();
        if (v instanceof Number n) return n.longValue() == n.doubleValue() ? (Object) n.longValue() : (Object) n.doubleValue();
        if (v instanceof java.sql.Timestamp t) return t.toLocalDateTime().toString();
        if (v instanceof java.sql.Date d) return d.toLocalDate().toString();
        if (v instanceof java.util.Date d) return d.toInstant().toString();
        return String.valueOf(v);
    }

    /** La copia de {@code <paquete>.<vista>} como filas. */
    public static List<Map<String, Object>> over(String vista) throws Exception {
        Path f = parquetDe(vista);
        return filasDe(duckdb(), "select * from read_parquet('" + rutaSql(f) + "')");
    }

    /** SQL (DuckDB) sobre las copias: cada {@code paquete.vista} tras FROM/JOIN se resuelve, se baja una vez y queda como vista. */
    public static List<Map<String, Object>> sql(String texto) throws Exception {
        if (texto == null || texto.isBlank()) throw new IllegalArgumentException("sql() quiere una consulta");
        Connection con = duckdb();
        TreeSet<String> vistas = new TreeSet<>();
        Matcher m = VISTAS_EN_SQL.matcher(texto);
        while (m.find()) vistas.add(m.group(1) + "." + m.group(2));
        for (String v : vistas) {
            String[] p = v.split("\\.");
            Path f = parquetDe(v);
            try (Statement s = con.createStatement()) {
                s.execute("create schema if not exists \"" + p[0] + "\"");
                s.execute("create or replace view \"" + p[0] + "\".\"" + p[1] + "\" as select * from read_parquet('" + rutaSql(f) + "')");
            }
        }
        return filasDe(con, texto);
    }
}
