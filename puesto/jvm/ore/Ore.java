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
import java.sql.SQLException;
import java.sql.Statement;
import java.time.Duration;
import java.time.Instant;
import java.time.LocalDate;
import java.time.LocalDateTime;
import java.time.LocalTime;
import java.time.ZoneOffset;
import java.util.ArrayList;
import java.util.Base64;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;
import java.util.regex.Matcher;
import java.util.regex.Pattern;
import java.io.ByteArrayOutputStream;
import java.io.OutputStream;
import java.math.BigDecimal;
import java.time.ZonedDateTime;
import java.util.LinkedHashSet;
import java.util.Set;
import org.apache.arrow.vector.ipc.ArrowStreamWriter;
import org.apache.arrow.vector.types.DateUnit;
import org.apache.arrow.vector.types.FloatingPointPrecision;
import org.apache.arrow.vector.types.TimeUnit;
import org.apache.arrow.vector.types.pojo.FieldType;
import org.apache.arrow.vector.types.pojo.Schema;
import org.apache.arrow.memory.BufferAllocator;
import org.apache.arrow.memory.RootAllocator;
import org.apache.arrow.vector.BigIntVector;
import org.apache.arrow.vector.BitVector;
import org.apache.arrow.vector.DateDayVector;
import org.apache.arrow.vector.DecimalVector;
import org.apache.arrow.vector.FieldVector;
import org.apache.arrow.vector.Float4Vector;
import org.apache.arrow.vector.Float8Vector;
import org.apache.arrow.vector.IntVector;
import org.apache.arrow.vector.LargeVarBinaryVector;
import org.apache.arrow.vector.LargeVarCharVector;
import org.apache.arrow.vector.SmallIntVector;
import org.apache.arrow.vector.TimeMicroVector;
import org.apache.arrow.vector.TimeStampMicroTZVector;
import org.apache.arrow.vector.TimeStampMicroVector;
import org.apache.arrow.vector.TimeStampMilliTZVector;
import org.apache.arrow.vector.TimeStampMilliVector;
import org.apache.arrow.vector.TimeStampNanoTZVector;
import org.apache.arrow.vector.TimeStampNanoVector;
import org.apache.arrow.vector.TimeStampSecTZVector;
import org.apache.arrow.vector.TimeStampSecVector;
import org.apache.arrow.vector.TinyIntVector;
import org.apache.arrow.vector.UInt1Vector;
import org.apache.arrow.vector.UInt2Vector;
import org.apache.arrow.vector.UInt4Vector;
import org.apache.arrow.vector.UInt8Vector;
import org.apache.arrow.vector.VarBinaryVector;
import org.apache.arrow.vector.VarCharVector;
import org.apache.arrow.vector.VectorSchemaRoot;
import org.apache.arrow.vector.complex.ListVector;
import org.apache.arrow.vector.complex.StructVector;
import org.apache.arrow.vector.ipc.ArrowReader;
import org.apache.arrow.vector.types.pojo.ArrowType;
import org.apache.arrow.vector.types.pojo.Field;

/**
 * {@code ore} · el SDK del puesto para Java (0031 W3.4). El mismo contrato que
 * {@code puesto/python/ore}; la sesión lo importa estático
 * ({@code import static ore.Ore.*;}), así que una celda escribe
 * {@code sql("select …")}, {@code over("hr.espanoles")} y {@code persona()}.
 *
 * <ul>
 *   <li>{@code over("<paquete>.<vista>")} → las filas de la copia de esa vista
 *       ({@link Filas}: una {@code List<Map<String,Object>>} con {@code tipos},
 *       {@code total} y {@code truncada}), leídas por DuckDB y entregadas por
 *       <b>Arrow</b> ({@code arrowExportStream}): exacto en los 23 tipos del
 *       contrato (0032 T4), donde el mapeo de JDBC tenía cuatro mal.</li>
 *   <li>{@code sql("select … from p.v")} → las filas del resultado: cada
 *       {@code paquete.vista} tras FROM/JOIN se resuelve, se baja una vez y
 *       queda como vista de DuckDB.</li>
 *   <li>{@code arrow("<paquete>.<vista>")} / {@code arrowSql("…")} → el
 *       {@code ArrowReader} por lotes ({@code VectorSchemaRoot}), sin objeto
 *       por fila: 14,6 M filas/s. Quien lo pide lo cierra.</li>
 *   <li>{@code persona()} → quién abrió el puesto ({@code persona:…}).</li>
 * </ul>
 *
 * <p>Los valores son los del contrato de tipos (0032 §1): {@code Long} para un
 * entero de 64 bits, {@code BigDecimal} exacto, {@code LocalDate},
 * {@code LocalTime}, {@code LocalDateTime} (hora de pared), {@code Instant}
 * (un instante, en UTC), {@code byte[]}, {@code List}, {@code Map}. Y hay un
 * límite de filas materializadas ({@link #LIMITE}, 1 M): {@code over()} y
 * {@code sql()} lo dicen ({@code truncada}), y con {@code estricto} fallan en
 * vez de recortar; lo masivo va por {@code arrow()} o se agrega en SQL.
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
            return pedir(metodo, ruta, cuerpo, plazo, Map.of());
        }

        public Respuesta pedir(String metodo, String ruta, Object cuerpo, Duration plazo, Map<String, String> extra) throws IOException, InterruptedException {
            HttpRequest.Builder b = HttpRequest.newBuilder(URI.create(servidor + ruta)).timeout(plazo).header("accept", "application/json");
            for (Map.Entry<String, String> e : cabeceras.entrySet()) b.header(e.getKey(), e.getValue());
            // Desde qué puesto: el catálogo escribe en nombre de quien lo abrió.
            if (!id.isEmpty()) b.header("x-ore-puesto", id);
            for (Map.Entry<String, String> e : extra.entrySet()) b.header(e.getKey(), e.getValue());
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

    // Lo que la sesión leyó (por nombre), y el transform activo si lo hay.
    private static final List<String> leidas = new ArrayList<>();
    private record Transform(String nombre, List<String> inputs, String output) {}
    private static Transform transformActivo = null;
    /** El trabajo que corre (W3.7 ④): `<ruta>@<commit>`, para la procedencia; lo pone el agente (la JVM no cambia su entorno). */
    public static String CODIGO = null;

    /** Un transform (0031 §9, W3.7 ③): corre {@code cuerpo} con {@code inputs} como lo único que puede leer ({@code over}, {@code sql}) y {@code output} como lo único que puede escribir ({@code write}); lo demás lanza. Lo escrito lleva {@code procedencia: {inputs, transform, …}}. */
    public static <T> T transform(String nombre, List<String> inputs, String output, java.util.concurrent.Callable<T> cuerpo) throws Exception {
        if (inputs == null || inputs.stream().anyMatch(i -> i == null || i.chars().filter(c -> c == '.').count() != 1)) throw new IllegalArgumentException("transform(): `inputs` es una lista de `<paquete>.<vista>`");
        if (output == null || output.chars().filter(c -> c == '.').count() != 1) throw new IllegalArgumentException("transform(): `output` es `<paquete>.<tabla>`");
        if (inputs.contains(output)) throw new IllegalArgumentException("transform(): `" + output + "` no puede ser input y output a la vez");
        if (transformActivo != null) throw new IllegalStateException("transform(): `" + transformActivo.nombre() + "` ya está corriendo; un transform no llama a otro");
        transformActivo = new Transform(nombre == null || nombre.isEmpty() ? "transform" : nombre, List.copyOf(inputs), output);
        decirTransform(transformActivo);
        try { return cuerpo.call(); } finally { transformActivo = null; decirTransform(null); }
    }

    private static void lee(String vista) {
        if (transformActivo != null && !transformActivo.inputs().contains(vista)) throw new IllegalStateException("`" + vista + "` no está en los inputs de `" + transformActivo.nombre() + "` (" + String.join(", ", transformActivo.inputs()) + "): un transform sólo lee lo que declara");
        if (!leidas.contains(vista)) leidas.add(vista);
    }

    /** Lo declarado, dicho al servidor (W3.7 gobierno ⑤): mientras corre, resuelve sólo
     *  {@code inputs} y deja escribir sólo {@code output}. Un ore-serve viejo no contesta y
     *  el SDK sigue acotando por su cuenta. */
    private static void decirTransform(Transform t) {
        try {
            if (t != null) puesto.pedir("POST", "/puestos/" + puesto.id + "/transform", Map.of("nombre", t.nombre(), "inputs", t.inputs(), "output", t.output()), Duration.ofSeconds(30));
            else puesto.pedir("DELETE", "/puestos/" + puesto.id + "/transform", null, Duration.ofSeconds(30));
        } catch (Exception e) { /* el servidor no lo sabe: el SDK sigue acotando */ }
    }

    private static Map<String, Object> procedencia(String nombre) {
        Map<String, Object> p = new LinkedHashMap<>();
        p.put("puesto", puesto.id);
        if (transformActivo != null) { p.put("inputs", transformActivo.inputs().stream().sorted().toList()); p.put("transform", transformActivo.nombre()); }
        // Fuera de un transform, lo que la sesión leyó SIN lo que se está escribiendo
        // (W3.7 gobierno ③: un dataset no sale de sí mismo).
        else p.put("leidas", leidas.stream().filter(l -> !l.equals(nombre)).sorted().toList());
        String codigo = CODIGO != null ? CODIGO : System.getenv("ORE_CODIGO");
        if (codigo != null && !codigo.isEmpty()) p.put("codigo", codigo);
        return p;
    }

    private static Map<String, Object> resolver(String vista) throws IOException, InterruptedException {
        if (vista == null || vista.chars().filter(c -> c == '.').count() != 1)
            throw new IllegalArgumentException("se quiere `<paquete>.<vista>`, no " + vista);
        lee(vista);
        Respuesta r = puesto.pedir("GET", "/puestos/" + puesto.id + "/datos/" + vista, null, Duration.ofSeconds(30));
        return oElError(r, vista);
    }

    /** Lo que ore-serve contestó por un nombre, o el error de siempre: el mismo para
     *  {@code over()} (GET datos) que para {@code sql()} (POST sql). */
    private static Map<String, Object> oElError(Respuesta r, String vista) throws IOException {
        if (r.codigo() == 409) throw new IllegalStateException("la copia de `" + vista + "` no está hecha: " + r.error());
        if (r.codigo() == 404) throw new IllegalArgumentException("no hay ninguna `View` ni `Dataset` `" + vista + "` en el árbol");
        // El conducto de la lectura (0031 W3.7 gobierno ②): lo que el dataset lleva
        // no cabe por `contextSurface.workspace`. Se dice tal cual.
        if (r.codigo() == 403) throw new SecurityException(r.error() == null || r.error().isEmpty() ? "ore-serve no deja leer `" + vista + "` desde un puesto" : r.error());
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

    /**
     * De qué se lee la vista, como fragmento SQL de DuckDB (0031 §10, todo es un dataset):
     * {@code iceberg_scan(...)} si el puntero trae {@code metadata_location} —una tabla
     * Iceberg en el bucket, leída EN SITIO—, {@code read_parquet('…')} si trae {@code clave}
     * (el sobre heredado, que se baja una vez).
     */
    private static String fuenteDe(String vista) throws Exception {
        return fuenteDeRespuesta(vista, resolver(vista));
    }

    /** Donde se pone la vista de DuckDB de cada dataset que una View lee: aparte de los
     *  nombres del árbol, porque una View y su dataset pueden llamarse igual. */
    private static final String ESQUEMA_DE_DATASETS = "__ore_dataset";

    /**
     * Lo que {@code datos} contestó, como fragmento SQL. Una View llega como su pregunta
     * ({@code consulta}, SQL sobre {@code "__ore_dataset"."<p>.<n>"}) con sus datasets ya
     * resueltos por el servidor: cada uno se pone como vista de DuckDB por el camino de
     * siempre, y la View es la consulta encima. Medido: con {@code select *} sobre la raíz,
     * una View con {@code where} y {@code fields} daba 20 000 filas y 4 columnas donde dice
     * 5 000 y 2 ({@code medida-la-vista-con-filtro.py}).
     */
    @SuppressWarnings("unchecked")
    private static String fuenteDeRespuesta(String vista, Map<String, Object> r) throws Exception {
        Object consulta = r.get("consulta");
        if (consulta != null && !String.valueOf(consulta).isEmpty()) {
            Connection con = duckdb();
            try (Statement s = con.createStatement()) { s.execute("create schema if not exists \"" + ESQUEMA_DE_DATASETS + "\""); }
            Object ds = r.get("datasets");
            if (ds instanceof Map<?, ?> mapaDs) {
                for (Map.Entry<?, ?> e : mapaDs.entrySet()) {
                    String d = String.valueOf(e.getKey());
                    String fuente = fuenteDeRespuesta(d, (Map<String, Object>) e.getValue());
                    try (Statement s = con.createStatement()) {
                        s.execute("create or replace view \"" + ESQUEMA_DE_DATASETS + "\".\"" + d.replace("\"", "\"\"") + "\" as select * from " + fuente);
                    }
                }
            }
            return "(" + consulta + ")";
        }
        Object m = r.get("metadata_location");
        if (m != null && !String.valueOf(m).isEmpty()) {
            // La credencial de lectura que `datos` presta (W3.7 gobierno ②b).
            Object cred = r.get("credencial");
            Map<String, String> credencial = cred instanceof Map<?, ?> ? mapa(cred) : Map.of();
            if (String.valueOf(m).startsWith("s3://") && s3 == null) {
                if (credencial.get("s3.access-key-id") != null) s3 = credencial;
                else {
                    String[] p = vista.split("\\.");
                    Respuesta l = puesto.pedir("GET", "/v1/namespaces/" + p[0] + "/tables/" + p[1], null, Duration.ofSeconds(30), DELEGAR);
                    Object cfg = l.cuerpo().get("config");
                    if (l.codigo() == 200 && cfg instanceof Map<?, ?> c && c.get("s3.access-key-id") != null) s3 = mapa(cfg);
                }
            }
            Object tok = credencial.get("gcs.oauth2.token");
            return iceberg(String.valueOf(m), tok == null ? null : String.valueOf(tok));
        }
        return "read_parquet('" + rutaSql(parquetDe(vista, r)) + "')";
    }

    /** Donde la imagen deja las extensiones de DuckDB preinstaladas. */
    public static final Path EXTENSIONES = Path.of("/opt/ore/duckdb");
    private static final String[] LAGO = { "json", "icu", "avro", "iceberg" };

    /**
     * {@code iceberg_scan} sobre la raíz de la tabla y la versión del puntero (con
     * {@code allow_moved_paths} la raíz es lo que se le pasa, y así no lista nada). En el
     * bucket, la API XML de GCS por https con el token del pod como <i>bearer</i>.
     */
    private static String iceberg(String metadataLocation, String prestada) throws Exception {
        int i = metadataLocation.lastIndexOf("/metadata/");
        String raiz = metadataLocation.substring(0, i);
        String version = metadataLocation.substring(i + "/metadata/".length()).replaceFirst("\\.metadata\\.json$", "");
        Connection con = duckdb();
        // `iceberg` arrastra `avro` (y usa `json` e `icu`); con el autoinstalado apagado
        // hay que cargarlas por su nombre, en orden. `httpfs` sólo para el bucket.
        for (String e : LAGO) cargar(con, e);
        if (raiz.startsWith("gs://")) {
            cargar(con, "httpfs");
            raiz = "https://storage.googleapis.com/" + raiz.substring(5);
            // Con la credencial prestada para este dataset (②b): un secreto por raíz,
            // con `scope`; sin ella, el token del pod (lo de antes).
            if (prestada != null && !prestada.isEmpty()) {
                String n = Integer.toHexString(raiz.hashCode());
                try (Statement s = con.createStatement()) { s.execute("create or replace secret ore_gcs_" + n + " (type http, bearer_token '" + prestada.replace("'", "''") + "', scope '" + raiz.replace("'", "''") + "')"); }
            } else {
                try (Statement s = con.createStatement()) { s.execute("create or replace secret ore_gcs (type http, bearer_token '" + tokenDeGoogle().replace("'", "''") + "')"); }
            }
        } else if (raiz.startsWith("s3://") && s3 != null) {
            // Un S3 (R2, o el de mentira de las pruebas): con la credencial que el
            // catálogo prestó al escribir, o la de la tabla que se pidió leer.
            cargar(con, "httpfs");
            String ep = String.valueOf(s3.getOrDefault("s3.endpoint", ""));
            String ssl = ep.startsWith("https://") ? "true" : "false";
            ep = ep.replaceFirst("^https?://", "").replaceAll("/+$", "");
            try (Statement s = con.createStatement()) {
                s.execute("create or replace secret ore_s3 (type s3, key_id '" + q(s3.get("s3.access-key-id")) + "', secret '" + q(s3.get("s3.secret-access-key"))
                    + "', endpoint '" + q(ep) + "', url_style 'path', use_ssl " + ssl + ", region '" + q(s3.getOrDefault("s3.region", "auto")) + "')");
            }
        }
        return "iceberg_scan('" + raiz.replace("\\", "/").replace("'", "''") + "', version='" + version.replace("'", "''") + "', allow_moved_paths=true)";
    }

    /** {@code LOAD} de una extensión: en la imagen está preinstalada; fuera, si falta, se instala una vez. */
    private static void cargar(Connection con, String extension) throws SQLException {
        try (Statement s = con.createStatement()) { s.execute("load " + extension); return; } catch (SQLException e) {
            if (Files.isDirectory(EXTENSIONES)) throw new SQLException("la imagen no trae la extensión `" + extension + "` de DuckDB: hay que preinstalarla en " + EXTENSIONES, e);
        }
        try (Statement s = con.createStatement()) { s.execute("install " + extension); s.execute("load " + extension); }
    }

    /** La copia de la vista como Parquet local, bajado UNA vez por sesión (la clave es el digest del artefacto). */
    private static Path parquetDe(String vista, Map<String, Object> r) throws IOException, InterruptedException {
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

    // ── El reparto del pod (medido en `medida-la-celda-que-no-cabe.py`) ────
    //
    // ⛔ SIN ESTO, DOS DE LOS TRES CAMINOS MATAN EL POD EN VEZ DE CONTARLO:
    //   el asignador de Arrow nace con `Long.MAX_VALUE` y NO se le aplica el
    //   tope de memoria directa de la JVM —medido: con
    //   `-XX:MaxDirectMemorySize=64m` reservó 200 MB sin rechistar, porque
    //   `arrow-memory-unsafe` le pide al sistema operativo—, y DuckDB se pone
    //   un `memory_limit` sacado de LA MAQUINA (25 GiB medidos en una de 32 GB),
    //   no del pod. Los dos acaban en un SIGKILL del kernel: sin excepción, sin
    //   mensaje y sin informe. Con tope, los dos acaban en una celda con error
    //   y la sesión sigue viva.
    //
    // El reparto de un pod de 4 GiB, y por qué:
    //
    //   heap        50 %   lo que `over()` materializa vive aquí (`-XX:MaxRAMPercentage`)
    //   Arrow       20 %   lotes en vuelo; con `arrow()` se sueltan según se leen
    //   DuckDB      20 %   y lo que no le quepa lo derrama a disco
    //   el resto    10 %   metaspace, hilos, el propio JDK
    //
    // `ORE_MEMORIA_MB` lo pone la plantilla del Job JUNTO A `limits.memory`, y
    // `gen-inquilino.py ⑱` exige que digan lo mismo: un reparto calculado sobre
    // una cifra que no es la del pod es peor que no tener reparto.
    static final int ARROW_POR_CIENTO = 20;
    static final int DUCKDB_POR_CIENTO = 20;

    /** Los MB del pod, o 0 si nadie lo dijo (fuera del clúster). */
    static long memoriaDelPuestoMb() {
        String m = System.getenv("ORE_MEMORIA_MB");
        try {
            return m == null || m.isBlank() ? 0 : Math.max(0, Long.parseLong(m.trim()));
        } catch (NumberFormatException e) {
            return 0;
        }
    }

    /**
     * Lo que le toca a una parte, en MB.
     *
     * <p>⭐ Sin `ORE_MEMORIA_MB` —las pruebas, un portátil— se reparte sobre EL
     * HEAP, que siempre se sabe. Nunca se devuelve «sin límite»: un tope
     * equivocado da un error legible; ninguno da un proceso muerto.
     */
    static long tropoMb(int porCiento) {
        long pod = memoriaDelPuestoMb();
        long base = pod > 0 ? pod : Runtime.getRuntime().maxMemory() / 1048576;
        return Math.max(64, base * porCiento / 100);
    }

    // ── DuckDB → Arrow ─────────────────────────────────────────────────────
    private static Connection conexion;
    private static BufferAllocator asignador;

    private static synchronized Connection duckdb() throws SQLException {
        if (conexion == null) {
            try { Class.forName("org.duckdb.DuckDBDriver"); } catch (ClassNotFoundException e) {
                throw new SQLException("este puesto no trae DuckDB (duckdb_jdbc.jar en el classpath): sql() y over() no pueden leer Parquet");
            }
            conexion = DriverManager.getConnection("jdbc:duckdb:");
            String hilos = System.getenv("ORE_HILOS");
            try (Statement s = conexion.createStatement()) {
                if (hilos != null && !hilos.isEmpty()) s.execute("set threads to " + Integer.parseInt(hilos));
                // ⭐ LO QUE LE TOCA, Y DONDE DERRAMAR LO QUE NO QUEPA. Medido:
                //   agrupando 12 M de claves con 200 MB, sin sitio donde derramar
                //   es «Out of Memory Error … cannot be offloaded to disk» y con
                //   sitio son 12,2 s. La diferencia entre «no se puede» y «tarda».
                s.execute("set memory_limit='" + tropoMb(DUCKDB_POR_CIENTO) + "MB'");
                s.execute("set temp_directory='" + rutaSql(derrame()) + "'");
                // Nunca salir a por una extensión: sin red, DuckDB se rinde a los 120 s
                // (medido en el clúster). Lo que la imagen trae está en /opt/ore/duckdb.
                s.execute("set autoinstall_known_extensions = false");
                // Un instante es un instante: el contrato (0032 §1) lo quiere en UTC, y
                // DuckDB enseña un TIMESTAMPTZ en la zona de la sesión. Aquí la sesión ES UTC.
                s.execute("set TimeZone = 'UTC'");
                if (Files.isDirectory(EXTENSIONES)) s.execute("set extension_directory = '" + EXTENSIONES + "'");
            }
            // Con tope, y no `new RootAllocator()`: ver arriba.
            asignador = new RootAllocator(tropoMb(ARROW_POR_CIENTO) * 1048576L);
        }
        return conexion;
    }

    /**
     * Dónde derrama DuckDB lo que no le cabe.
     *
     * <p>En el pod, el volumen de trabajo (`/trabajo`, un `emptyDir`), que es
     * donde se puede escribir y muere con la sesión. Fuera del clúster, el
     * temporal del sistema — ⛔ y NO el directorio actual, que en las pruebas
     * es el repositorio: un motor derramando gigabytes dentro del árbol de
     * fuentes es un susto que no hace falta darse.
     */
    private static Path derrame() {
        for (Path base : new Path[] {Path.of("/trabajo"), Path.of(System.getProperty("java.io.tmpdir", "."))}) {
            Path d = base.resolve(".duckdb-derrame");
            if (!Files.isDirectory(base)) {
                continue;
            }
            try {
                Files.createDirectories(d);
                return d;
            } catch (IOException e) {
                // el siguiente
            }
        }
        return Path.of(System.getProperty("java.io.tmpdir", "."));
    }

    private static String rutaSql(Path f) { return f.toString().replace("\\", "/").replace("'", "''"); }

    /** Cuántas filas materializan {@code over()} y {@code sql()} si no se dice otra cosa. */
    public static final int LIMITE = 1_000_000;

    /** Las filas que devuelven {@code over()} y {@code sql()}: la lista, y lo que hay que saber de ella. */
    public static final class Filas extends ArrayList<Map<String, Object>> {
        /** Columna → tipo de Arrow, con el nombre que {@code pyarrow} le da. */
        public final Map<String, String> tipos;
        /** Las filas que hay (aunque no se hayan materializado todas), o {@code null} si no se sabe. */
        public final Long total;
        /** Si se paró en el límite. */
        public final boolean truncada;
        Filas(Map<String, String> tipos, Long total, boolean truncada) { this.tipos = tipos; this.total = total; this.truncada = truncada; }
    }

    /** Un lector de Arrow atado a su {@code ResultSet}: cerrarlo cierra los dos. */
    private static ArrowReader exportar(Statement st, String texto, int lote) throws SQLException {
        ResultSet rs = st.executeQuery(texto);
        ArrowReader lector = (ArrowReader) ((org.duckdb.DuckDBResultSet) rs).arrowExportStream(asignador, lote);
        return new ArrowReader(asignador) {
            @Override public boolean loadNextBatch() throws IOException { return lector.loadNextBatch(); }
            @Override public long bytesRead() { return lector.bytesRead(); }
            @Override protected void closeReadSource() throws IOException { try { lector.close(); rs.close(); st.close(); } catch (SQLException e) { throw new IOException(e); } }
            @Override protected org.apache.arrow.vector.types.pojo.Schema readSchema() throws IOException { return lector.getVectorSchemaRoot().getSchema(); }
            @Override public VectorSchemaRoot getVectorSchemaRoot() throws IOException { return lector.getVectorSchemaRoot(); }
        };
    }

    /** Materializa hasta {@code limite} filas de un lector (y lo cierra). */
    private static Filas filasDe(ArrowReader lector, int limite, boolean estricto, Long total, String que) throws Exception {
        try (lector) {
            Map<String, String> tipos = new LinkedHashMap<>();
            List<Map<String, Object>> out = new ArrayList<>();
            boolean truncada = false;
            while (!truncada && lector.loadNextBatch()) {
                VectorSchemaRoot raiz = lector.getVectorSchemaRoot();
                if (tipos.isEmpty()) for (Field f : raiz.getSchema().getFields()) tipos.put(f.getName(), nombreArrow(f));
                List<FieldVector> vs = raiz.getFieldVectors();
                for (int i = 0; i < raiz.getRowCount(); i++) {
                    if (out.size() == limite) { truncada = true; break; }
                    Map<String, Object> fila = new LinkedHashMap<>();
                    for (FieldVector v : vs) fila.put(v.getName(), valorDe(v, i));
                    out.add(fila);
                }
            }
            if (tipos.isEmpty()) for (Field f : lector.getVectorSchemaRoot().getSchema().getFields()) tipos.put(f.getName(), nombreArrow(f));
            if (truncada && estricto) throw new IllegalStateException(que + ": el resultado pasa de " + limite + " filas; sube el límite, agrega en sql(), usa arrow() o quita estricto");
            Filas filas = new Filas(tipos, truncada ? total : Long.valueOf(out.size()), truncada);
            filas.addAll(out);
            return filas;
        }
    }

    /**
     * Cada nombre del árbol que el texto lee, como vista de DuckDB. El texto entero va a
     * ore-serve ({@code POST /puestos/{id}/sql}), que dice qué nombres lee —tokenizador y
     * árbol como filtro, sin regex: un nombre en un comentario o en una cadena no cuenta,
     * {@code from a, b} cuenta los dos, un esquema de la sesión es del motor— y los
     * resuelve como {@code over()}, en una ida y vuelta.
     */
    @SuppressWarnings("unchecked")
    private static void registrar(Connection con, String texto) throws Exception {
        // Sin puesto no hay árbol (--comprobar), y sin un punto no hay `a.b`: el motor solo.
        if (puesto.id.isEmpty() || texto.indexOf('.') < 0) return;
        Respuesta r = puesto.pedir("POST", "/puestos/" + puesto.id + "/sql", Map.of("texto", texto), Duration.ofSeconds(60));
        if (r.codigo() != 200) {
            Object n = r.cuerpo() == null ? null : r.cuerpo().get("nombre");
            oElError(r, n == null ? "?" : String.valueOf(n));
        }
        Object fs = r.cuerpo() == null ? null : r.cuerpo().get("fuentes");
        if (!(fs instanceof Map<?, ?> fuentes)) return;
        for (Map.Entry<?, ?> e : new TreeMap<>(fuentes).entrySet()) {
            String v = String.valueOf(e.getKey());
            lee(v);
            String[] p = v.split("\\.", 2);
            String fuente = fuenteDeRespuesta(v, (Map<String, Object>) e.getValue());
            try (Statement s = con.createStatement()) {
                s.execute("create schema if not exists \"" + p[0].replace("\"", "\"\"") + "\"");
                s.execute("create or replace view \"" + p[0].replace("\"", "\"\"") + "\".\"" + p[1].replace("\"", "\"\"") + "\" as select * from " + fuente);
            }
        }
    }

    /** La copia de {@code <paquete>.<vista>} como filas, hasta {@link #LIMITE}. */
    public static Filas over(String vista) throws Exception { return over(vista, LIMITE, false); }

    /** La copia como filas, hasta {@code limite}; con {@code estricto}, falla si no cabe. */
    public static Filas over(String vista, int limite, boolean estricto) throws Exception {
        String fuente = fuenteDe(vista);
        Connection con = duckdb();
        long total;
        try (Statement s = con.createStatement(); ResultSet rs = s.executeQuery("select count(*) from " + fuente)) { rs.next(); total = rs.getLong(1); }
        if (estricto && total > limite) throw new IllegalStateException("over(" + vista + "): la copia tiene " + total + " filas y el límite es " + limite + "; sube el límite, agrega en sql(), usa arrow() o quita estricto");
        return filasDe(exportar(con.createStatement(), "select * from " + fuente, 8192), limite, estricto, total, "over(" + vista + ")");
    }

    /** SQL (DuckDB) sobre las copias: cada {@code paquete.vista} tras FROM/JOIN se resuelve, se baja una vez y queda como vista. */
    public static Filas sql(String texto) throws Exception { return sql(texto, LIMITE, false); }

    public static Filas sql(String texto, int limite, boolean estricto) throws Exception {
        if (texto == null || texto.isBlank()) throw new IllegalArgumentException("sql() quiere una consulta");
        Connection con = duckdb();
        registrar(con, texto);
        return filasDe(exportar(con.createStatement(), texto, 8192), limite, estricto, null, "sql()");
    }

    /** La copia entera, por lotes de Arrow: {@code while (r.loadNextBatch()) { VectorSchemaRoot raiz = r.getVectorSchemaRoot(); … }}. Cerrar al acabar. */
    public static ArrowReader arrow(String vista) throws Exception {
        String fuente = fuenteDe(vista);
        return exportar(duckdb().createStatement(), "select * from " + fuente, 65_536);
    }

    /** El resultado de una consulta, por lotes de Arrow. Cerrar al acabar. */
    public static ArrowReader arrowSql(String texto) throws Exception {
        if (texto == null || texto.isBlank()) throw new IllegalArgumentException("arrowSql() quiere una consulta");
        Connection con = duckdb();
        registrar(con, texto);
        return exportar(con.createStatement(), texto, 65_536);
    }

    // ── Escribir (0031 §11, W3.6c) ─────────────────────────────────────────
    //
    // `write("p.t", datos)` deja un dataset —una tabla Iceberg en el lago, el
    // `Dataset` escrito en el árbol, el puntero— desde lo que `over()`/`sql()` devolvieron
    // (`Filas`, con sus tipos), un `List<Map>` cualquiera, un `VectorSchemaRoot`
    // o un `ArrowReader`. La tabla se arma como Arrow y va por IPC a `ore-store`
    // (el escritor de Rust), que la lleva al físico del contrato (0032) y escribe
    // los ficheros con la credencial que el catálogo prestó —acotada a esa
    // tabla—; el commit va al catálogo REST de Iceberg de `ore-serve` (`/v1/…`).
    // Idempotente por la clave de operación (del contenido); un 409 se reintenta
    // sobre lo que hay; un 5xx se MIRA antes de darlo por perdido.
    private static final Map<String, String> DELEGAR = Map.of("x-iceberg-access-delegation", "vended-credentials");
    private static Map<String, String> s3;

    private static String q(Object v) { return String.valueOf(v == null ? "" : v).replace("'", "''"); }

    @SuppressWarnings("unchecked")
    private static Map<String, String> mapa(Object o) {
        Map<String, String> m = new LinkedHashMap<>();
        if (o instanceof Map<?, ?> x) for (Map.Entry<?, ?> e : x.entrySet()) m.put(String.valueOf(e.getKey()), String.valueOf(e.getValue()));
        return m;
    }

    /** El tipo de Iceberg del esquema con el que la tabla se esboza (lo mismo que `ore-store` hace al escribir, 0032). */
    private static String tipoIceberg(String columna, String t) {
        switch (t) {
            case "bool": return "boolean";
            case "int8": case "int16": case "int32": case "int64": case "uint8": case "uint16": case "uint32": return "long";
            case "float": case "double": return "double";
            case "string": return "string";
            case "date32[day]": return "date";
            case "time64[us]": return "time";
            case "timestamp[us]": case "timestamp[ms]": case "timestamp[ns]": case "timestamp[s]": return "timestamp";
            case "timestamp[us, tz=UTC]": case "timestamp[ms, tz=UTC]": return "timestamptz";
            case "uint64": throw new IllegalArgumentException("write(): la columna `" + columna + "` es uint64, que no cabe en int64 sin mentir (0032); conviértela antes");
            case "null": throw new IllegalArgumentException("write(): la columna `" + columna + "` no tiene tipo (todo nulo): dale uno antes (0032)");
            default:
                Matcher m = Pattern.compile("^decimal128\\((\\d+), (\\d+)\\)$").matcher(t);
                if (m.matches()) return "decimal(" + m.group(1) + ", " + m.group(2) + ")";
                throw new IllegalArgumentException("write(): la columna `" + columna + "` es `" + t + "`, que el contrato de tipos (0032) no tiene");
        }
    }

    /** El campo de Arrow de un tipo con el nombre de {@code pyarrow}. */
    private static Field campoArrow(String nombre, String t) {
        ArrowType tipo;
        switch (t) {
            case "bool": tipo = ArrowType.Bool.INSTANCE; break;
            case "int8": case "int16": case "int32": case "int64": case "uint8": case "uint16": case "uint32": tipo = new ArrowType.Int(64, true); break;
            case "float": case "double": tipo = new ArrowType.FloatingPoint(FloatingPointPrecision.DOUBLE); break;
            case "string": tipo = ArrowType.Utf8.INSTANCE; break;
            case "date32[day]": tipo = new ArrowType.Date(DateUnit.DAY); break;
            case "time64[us]": tipo = new ArrowType.Time(TimeUnit.MICROSECOND, 64); break;
            case "timestamp[us]": case "timestamp[ms]": case "timestamp[ns]": case "timestamp[s]": tipo = new ArrowType.Timestamp(TimeUnit.MICROSECOND, null); break;
            case "timestamp[us, tz=UTC]": case "timestamp[ms, tz=UTC]": tipo = new ArrowType.Timestamp(TimeUnit.MICROSECOND, "UTC"); break;
            default:
                Matcher m = Pattern.compile("^decimal128\\((\\d+), (\\d+)\\)$").matcher(t);
                if (!m.matches()) throw new IllegalArgumentException("write(): la columna `" + nombre + "` es `" + t + "`, que el contrato de tipos (0032) no tiene");
                tipo = new ArrowType.Decimal(Integer.parseInt(m.group(1)), Integer.parseInt(m.group(2)), 128);
        }
        return new Field(nombre, FieldType.nullable(tipo), null);
    }

    private static long micros(Object v) {
        if (v instanceof Instant i) return Math.multiplyExact(i.getEpochSecond(), 1_000_000L) + i.getNano() / 1000;
        if (v instanceof ZonedDateTime z) return micros(z.toInstant());
        if (v instanceof LocalDateTime l) return micros(l.toInstant(ZoneOffset.UTC));
        if (v instanceof java.util.Date d) return d.getTime() * 1000L;
        if (v instanceof Number n) return n.longValue();
        return micros(Instant.parse(String.valueOf(v)));
    }

    /** Un valor de Java en el vector, en la fila {@code i}. */
    private static void poner(FieldVector v, int i, Object x, String tipo) {
        if (x == null) { v.setNull(i); return; }
        if (v instanceof BigIntVector b) b.setSafe(i, x instanceof Number n ? n.longValue() : Long.parseLong(String.valueOf(x)));
        else if (v instanceof Float8Vector f) f.setSafe(i, x instanceof Number n ? n.doubleValue() : Double.parseDouble(String.valueOf(x)));
        else if (v instanceof BitVector b) b.setSafe(i, Boolean.parseBoolean(String.valueOf(x)) ? 1 : 0);
        else if (v instanceof VarCharVector s) s.setSafe(i, String.valueOf(x).getBytes(StandardCharsets.UTF_8));
        else if (v instanceof DecimalVector d) d.setSafe(i, (x instanceof BigDecimal bd ? bd : new BigDecimal(String.valueOf(x))).setScale(d.getScale(), java.math.RoundingMode.HALF_UP));
        else if (v instanceof DateDayVector d) d.setSafe(i, (int) (x instanceof LocalDate l ? l.toEpochDay() : LocalDate.parse(String.valueOf(x)).toEpochDay()));
        else if (v instanceof TimeMicroVector t) t.setSafe(i, (x instanceof LocalTime l ? l.toNanoOfDay() : LocalTime.parse(String.valueOf(x)).toNanoOfDay()) / 1000L);
        else if (v instanceof TimeStampMicroTZVector t) t.setSafe(i, micros(x));
        else if (v instanceof TimeStampMicroVector t) t.setSafe(i, micros(x));
        else throw new IllegalArgumentException("write(): la columna `" + v.getName() + "` (" + tipo + ") no se sabe rellenar");
    }

    /** Lo que se escribe, como flujo Arrow IPC: {@code VectorSchemaRoot}, {@code ArrowReader}, {@code Filas} o {@code List<Map>}. */
    @SuppressWarnings("unchecked")
    private static byte[] ipcDe(Object datos, List<Map<String, Object>> esquema) throws Exception {
        duckdb();
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        if (datos instanceof VectorSchemaRoot raiz) {
            if (raiz.getRowCount() == 0) throw new IllegalArgumentException("write(): la tabla no tiene filas");
            try (ArrowStreamWriter w = new ArrowStreamWriter(raiz, null, out)) { w.start(); w.writeBatch(); w.end(); }
            for (Field f : raiz.getSchema().getFields()) esquema.add(campoIceberg(esquema.size() + 1, f.getName(), nombreArrow(f)));
            return out.toByteArray();
        }
        if (datos instanceof ArrowReader r) {
            boolean alguna = false;
            try (ArrowStreamWriter w = new ArrowStreamWriter(r.getVectorSchemaRoot(), null, out)) {
                w.start();
                while (r.loadNextBatch()) { if (r.getVectorSchemaRoot().getRowCount() > 0) { alguna = true; w.writeBatch(); } }
                w.end();
            }
            if (!alguna) throw new IllegalArgumentException("write(): la tabla no tiene filas");
            for (Field f : r.getVectorSchemaRoot().getSchema().getFields()) esquema.add(campoIceberg(esquema.size() + 1, f.getName(), nombreArrow(f)));
            return out.toByteArray();
        }
        if (!(datos instanceof List<?> lista)) throw new IllegalArgumentException("write() quiere Filas, List<Map>, VectorSchemaRoot o ArrowReader, no " + (datos == null ? "null" : datos.getClass().getSimpleName()));
        if (lista.isEmpty()) throw new IllegalArgumentException("write(): la tabla no tiene filas");
        Map<String, String> tipos = new LinkedHashMap<>(datos instanceof Filas f ? f.tipos : Map.of());
        Set<String> nombres = new LinkedHashSet<>(tipos.keySet());
        for (Object o : lista) if (o instanceof Map<?, ?> m) for (Object k : m.keySet()) nombres.add(String.valueOf(k));
        for (String n : nombres) if (!tipos.containsKey(n)) {
            Object v = null;
            for (Object o : lista) { Object x = ((Map<String, Object>) o).get(n); if (x != null) { v = x; break; } }
            tipos.put(n, tipoInferido(v));
        }
        List<Field> campos = new ArrayList<>();
        for (String n : nombres) campos.add(campoArrow(n, tipos.get(n)));
        try (VectorSchemaRoot raiz = VectorSchemaRoot.create(new Schema(campos), asignador)) {
            raiz.allocateNew();
            int i = 0;
            for (Object o : lista) {
                Map<String, Object> fila = (Map<String, Object>) o;
                int c = 0;
                for (String n : nombres) poner(raiz.getVector(c++), i, fila.get(n), tipos.get(n));
                i++;
            }
            raiz.setRowCount(i);
            try (ArrowStreamWriter w = new ArrowStreamWriter(raiz, null, out)) { w.start(); w.writeBatch(); w.end(); }
        }
        for (String n : nombres) esquema.add(campoIceberg(esquema.size() + 1, n, tipos.get(n)));
        return out.toByteArray();
    }

    private static Map<String, Object> campoIceberg(int id, String nombre, String tipo) {
        Map<String, Object> f = new LinkedHashMap<>();
        f.put("id", id); f.put("name", nombre); f.put("type", tipoIceberg(nombre, tipo)); f.put("required", false);
        return f;
    }

    /** El escritor y su entorno: `ore-store-gcs` con el token prestado si la tabla vive en `gs://`, `ore-store-r2` si en `s3://`. */
    private static ProcessBuilder escritor(Map<String, String> config, String ubicacion) {
        String nombre;
        Map<String, String> env = new LinkedHashMap<>();
        if (ubicacion.startsWith("gs://")) {
            nombre = "ore-store-gcs";
            env.put("ORE_GCS_BUCKET", ubicacion.substring(5).split("/")[0]);
            String tok = config.getOrDefault("gcs.oauth2.token", "");
            if (tok.isEmpty()) throw new IllegalStateException("write(): el catálogo no prestó credencial para `" + ubicacion + "`");
            env.put("ORE_GCS_TOKEN", tok);
        } else if (ubicacion.startsWith("s3://")) {
            nombre = "ore-store-r2";
            env.put("ORE_R2_BUCKET", ubicacion.substring(5).split("/")[0]);
            env.put("ORE_R2_S3_ENDPOINT", config.getOrDefault("s3.endpoint", ""));
            env.put("ORE_R2_ACCESS_KEY_ID", config.getOrDefault("s3.access-key-id", ""));
            env.put("ORE_R2_SECRET_ACCESS_KEY", config.getOrDefault("s3.secret-access-key", ""));
            env.put("ORE_R2_REGION", config.getOrDefault("s3.region", "auto"));
        } else {
            throw new IllegalStateException("write(): la tabla vive en `" + ubicacion + "`, que no es un lago que este SDK sepa escribir");
        }
        List<String> dirs = new ArrayList<>();
        String d = System.getenv("ORE_STORE_DIR");
        if (d != null && !d.isEmpty()) dirs.add(d);
        String path = System.getenv("PATH");
        if (path != null) dirs.addAll(List.of(path.split(java.io.File.pathSeparator)));
        for (String dir : dirs) for (String ext : new String[] { "", ".exe" }) {
            Path b = Path.of(dir, nombre + ext);
            if (Files.isRegularFile(b)) { ProcessBuilder pb = new ProcessBuilder(b.toString(), "escribir"); pb.environment().putAll(env); return pb; }
        }
        throw new IllegalStateException("write(): no está `" + nombre + "` en el PATH (la imagen del puesto lo lleva; fuera, ORE_STORE_DIR)");
    }

    private static String mensajeDe(Respuesta r) {
        Object e = r.cuerpo().get("error");
        if (e instanceof Map<?, ?> m && m.get("message") != null) return String.valueOf(m.get("message"));
        return e == null ? r.cuerpo().toString() : String.valueOf(e);
    }

    /** Escribe {@code datos} como el dataset {@code <paquete>.<tabla>} del lago, sobrescribiendo. */
    /** Declara un documento de la ontología desde la celda (0031 §9, W3.7 ①): el YAML tal cual → {@code PUT /documentos/{kind}/{ns}/{n}} (la puerta de Forge: compila antes de empujar). Lo firma quien abrió el puesto, en su rama. Devuelve {@code {kind, nombre, fichero, commit, nueva}}; un 422 es {@link IllegalArgumentException} con los diagnósticos. */
    public static Map<String, Object> declare(String yaml) throws Exception {
        java.util.regex.Matcher k = java.util.regex.Pattern.compile("^kind:\\s*([A-Za-z]+)\\s*$", java.util.regex.Pattern.MULTILINE).matcher(yaml);
        if (!k.find()) throw new IllegalArgumentException("declare(): el documento no dice `kind:`");
        java.util.regex.Matcher m = java.util.regex.Pattern.compile("^metadata:[ \\t]*(.*)$", java.util.regex.Pattern.MULTILINE).matcher(yaml);
        if (!m.find()) throw new IllegalArgumentException("declare(): el documento no tiene `metadata:`");
        Map<String, String> campos = new LinkedHashMap<>();
        java.util.function.Consumer<String> par = (s) -> { int i = s.indexOf(':'); if (i > 0) campos.put(s.substring(0, i).trim(), s.substring(i + 1).trim().replaceAll("^[\"']|[\"']$", "")); };
        String resto = m.group(1).trim();
        if (resto.startsWith("{")) { for (String s : resto.replaceAll("^\\{|\\}$", "").split(",")) par.accept(s); }
        else { for (String l : yaml.substring(m.end()).split("\n")) { if (l.isEmpty()) continue; if (!Character.isWhitespace(l.charAt(0))) break; par.accept(l); } }
        if (campos.get("name") == null) throw new IllegalArgumentException("declare(): `metadata.name` no está");
        Map<String, Object> cuerpo = new LinkedHashMap<>(); cuerpo.put("yaml", yaml);
        return declarar(k.group(1), campos.getOrDefault("namespace", ""), campos.get("name"), cuerpo);
    }

    /** Lo mismo, con el documento como mapa {@code {kind, metadata, spec}}. */
    @SuppressWarnings("unchecked")
    public static Map<String, Object> declare(Map<String, Object> documento) throws Exception {
        Object kind = documento.get("kind");
        Map<String, Object> meta = documento.get("metadata") instanceof Map<?, ?> mm ? (Map<String, Object>) mm : Map.of();
        if (kind == null || meta.get("name") == null) throw new IllegalArgumentException("declare(): el documento quiere `kind` y `metadata.name`");
        return declarar(kind.toString(), String.valueOf(meta.getOrDefault("namespace", "")), meta.get("name").toString(), documento);
    }

    @SuppressWarnings("unchecked")
    private static Map<String, Object> declarar(String kind, String ns, String nombre, Map<String, Object> cuerpo) throws Exception {
        if (ns.isEmpty()) throw new IllegalArgumentException("declare(): `metadata.namespace` no está: un documento vive en un paquete");
        Respuesta r = puesto.pedir("PUT", "/documentos/" + kind + "/" + ns + "/" + nombre, cuerpo, Duration.ofSeconds(120));
        Map<String, Object> c = r.cuerpo();
        if (r.codigo() == 200 || r.codigo() == 201) {
            Map<String, Object> out = new LinkedHashMap<>();
            out.put("kind", c.getOrDefault("kind", kind));
            out.put("nombre", c.getOrDefault("namespace", ns) + "." + c.getOrDefault("name", nombre));
            out.put("fichero", c.getOrDefault("fichero", ""));
            out.put("commit", c.getOrDefault("commit", ""));
            out.put("nueva", Boolean.TRUE.equals(c.get("nueva")) || (c.get("nueva") == null && r.codigo() == 201));
            return out;
        }
        if (c.get("diagnosticos") instanceof List<?> ds && !ds.isEmpty()) {
            StringBuilder sb = new StringBuilder();
            for (Object d : ds) { if (d instanceof Map<?, ?> dm) { if (sb.length() > 0) sb.append("; "); sb.append(dm.get("codigo")).append(": ").append(dm.get("mensaje")); } }
            throw new IllegalArgumentException("declare(" + ns + "." + nombre + "): " + sb);
        }
        throw new RuntimeException("declare(" + ns + "." + nombre + "): " + c.getOrDefault("error", "?") + " (" + r.codigo() + ")");
    }

    public static Map<String, Object> write(String nombre, Object datos) throws Exception { return write(nombre, datos, "sobrescribir"); }

    /** Escribe {@code datos} como el dataset {@code <paquete>.<tabla>} del lago; {@code modo} es {@code sobrescribir} o {@code anexar}. */
    public static Map<String, Object> write(String nombre, Object datos, String modo) throws Exception { return write(nombre, datos, modo, null); }

    /** Escribe {@code datos} como el dataset {@code <paquete>.<tabla>} del lago; {@code modo} es {@code sobrescribir}, {@code anexar} o {@code upsert} (con {@code clave}: las columnas que identifican una fila; copy-on-write, y la clave queda declarada en la tabla). */
    @SuppressWarnings("unchecked")
    public static Map<String, Object> write(String nombre, Object datos, String modo, List<String> clave) throws Exception {
        if (nombre == null || nombre.chars().filter(c -> c == '.').count() != 1) throw new IllegalArgumentException("write() quiere `<paquete>.<tabla>`, no " + nombre);
        if (!modo.equals("sobrescribir") && !modo.equals("anexar") && !modo.equals("upsert")) throw new IllegalArgumentException("modo " + modo + ": vale `sobrescribir`, `anexar` o `upsert`");
        if (clave != null && !modo.equals("upsert")) throw new IllegalArgumentException("`clave` es de modo `upsert`");
        String[] p = nombre.split("\\.");
        String ns = p[0], t = p[1];
        List<Map<String, Object>> campos = new ArrayList<>();
        byte[] ipc = ipcDe(datos, campos);
        Map<String, Object> esquema = new LinkedHashMap<>();
        esquema.put("type", "struct"); esquema.put("schema-id", 0); esquema.put("fields", campos);
        String dataset = "datasets/" + ns + "_" + t;
        if (transformActivo != null && !nombre.equals(transformActivo.output())) throw new IllegalStateException("`" + nombre + "` no es el output de `" + transformActivo.nombre() + "` (" + transformActivo.output() + "): un transform sólo escribe lo que declara");
        String semilla = nombre + "|" + modo + (clave != null && !clave.isEmpty() ? "|" + String.join(",", clave) : "");
        String claveOperacion = "";
        for (int intento = 0; intento < 4; intento++) {
            // 1 · la tabla, con la credencial prestada; o esbozada si no existe
            String base = null; Object esbozo = null; Map<String, String> config; String ubicacion;
            Respuesta r = puesto.pedir("GET", "/v1/namespaces/" + ns + "/tables/" + t, null, Duration.ofSeconds(30), DELEGAR);
            if (r.codigo() == 200) {
                base = String.valueOf(r.cuerpo().get("metadata-location"));
                config = mapa(r.cuerpo().get("config"));
                // Prestado sólo para leer (lo de otra persona, un mantenido): el
                // porqué, antes de escribir un fichero con una credencial que no escribe.
                if (config.get("ore.solo-lectura") != null)
                    throw new IllegalStateException("write(" + nombre + "): " + config.get("ore.solo-lectura"));
                ubicacion = String.valueOf(((Map<String, Object>) r.cuerpo().get("metadata")).get("location"));
            } else if (r.codigo() == 404) {
                Map<String, Object> cuerpo = new LinkedHashMap<>();
                cuerpo.put("name", t); cuerpo.put("stage-create", true); cuerpo.put("schema", esquema); cuerpo.put("properties", Map.of());
                Respuesta r2 = puesto.pedir("POST", "/v1/namespaces/" + ns + "/tables", cuerpo, Duration.ofSeconds(30), DELEGAR);
                if (r2.codigo() != 200) throw new IllegalStateException("write(" + nombre + "): " + mensajeDe(r2));
                esbozo = r2.cuerpo().get("metadata");
                config = mapa(r2.cuerpo().get("config"));
                ubicacion = String.valueOf(((Map<String, Object>) esbozo).get("location"));
            } else {
                throw new IllegalStateException("write(" + nombre + "): ore-serve contestó " + r.codigo() + ": " + mensajeDe(r));
            }
            if (config.get("s3.access-key-id") != null) s3 = config;
            // 2 · los ficheros, por ore-store
            Map<String, Object> peticion = new LinkedHashMap<>();
            peticion.put("dataset", dataset); peticion.put("modo", modo); peticion.put("operacion", "contenido"); peticion.put("semilla", semilla); peticion.put("procedencia", procedencia(nombre));
            if (clave != null && !clave.isEmpty()) peticion.put("clave", clave);
            if (base != null) peticion.put("base", base); else peticion.put("esbozo", esbozo);
            Process proc = escritor(config, ubicacion).start();
            try (OutputStream in = proc.getOutputStream()) { in.write((Json.escribir(peticion) + "\n").getBytes(StandardCharsets.UTF_8)); in.write(ipc); }
            byte[] salida = proc.getInputStream().readAllBytes();
            String err = new String(proc.getErrorStream().readAllBytes(), StandardCharsets.UTF_8).trim();
            if (proc.waitFor() != 0) throw new IllegalStateException("write(): " + (err.isEmpty() ? "el escritor falló" : err.replaceFirst("^error: ", "")));
            Map<String, Object> escrito = Json.objeto(new String(salida, StandardCharsets.UTF_8));
            claveOperacion = String.valueOf(escrito.getOrDefault("operacion", claveOperacion));
            // 3 · el commit, por el catálogo
            Map<String, Object> commit = new LinkedHashMap<>();
            commit.put("identifier", Map.of("namespace", List.of(ns), "name", t));
            commit.put("requirements", escrito.get("requirements")); commit.put("updates", escrito.get("updates"));
            Respuesta c = puesto.pedir("POST", "/v1/namespaces/" + ns + "/tables/" + t, commit, Duration.ofSeconds(120));
            if (c.codigo() == 200) {
                Map<String, Object> md = (Map<String, Object>) c.cuerpo().get("metadata");
                Map<String, Object> out = new LinkedHashMap<>();
                out.put("tabla", nombre); out.put("filas", escrito.get("filas")); out.put("snapshot", String.valueOf(md == null ? "" : md.get("current-snapshot-id")));
                out.put("metadata_location", String.valueOf(c.cuerpo().getOrDefault("metadata-location", ""))); out.put("operacion", claveOperacion);
                // repetida: el catálogo contestó con lo que ya había (el mismo puntero)
                out.put("repetida", base != null && base.equals(String.valueOf(c.cuerpo().get("metadata-location"))));
                return out;
            }
            if (c.codigo() == 409) continue; // alguien escribió mientras tanto: otra vez sobre lo que hay
            if (c.codigo() >= 500) {
                // el commit pudo entrar: se MIRA antes de darlo por perdido
                Respuesta v = puesto.pedir("GET", "/v1/namespaces/" + ns + "/tables/" + t, null, Duration.ofSeconds(30));
                if (v.codigo() == 200) {
                    Map<String, Object> md = (Map<String, Object>) v.cuerpo().get("metadata");
                    Object actual = md.get("current-snapshot-id");
                    for (Object sn : (List<Object>) md.getOrDefault("snapshots", List.of())) {
                        Map<String, Object> m = (Map<String, Object>) sn;
                        if (String.valueOf(m.get("snapshot-id")).equals(String.valueOf(actual)) && m.get("summary") instanceof Map<?, ?> su && claveOperacion.equals(String.valueOf(su.get("ore.operacion")))) {
                            Map<String, Object> out = new LinkedHashMap<>();
                            out.put("tabla", nombre); out.put("filas", escrito.get("filas")); out.put("snapshot", String.valueOf(actual));
                            out.put("metadata_location", String.valueOf(v.cuerpo().get("metadata-location"))); out.put("operacion", claveOperacion); out.put("repetida", false);
                            return out;
                        }
                    }
                }
                throw new IllegalStateException("write(" + nombre + "): el catálogo contestó " + c.codigo() + " y el commit no está: " + mensajeDe(c));
            }
            throw new IllegalStateException("write(" + nombre + "): " + mensajeDe(c));
        }
        throw new IllegalStateException("write(" + nombre + "): cuatro veces alguien escribió antes; vuelve a intentarlo");
    }

    // ── El contrato de tipos (0032 §1) ─────────────────────────────────────

    /** El valor de la fila {@code i} de un vector, en el tipo de Java del contrato. */
    public static Object valorDe(FieldVector v, int i) {
        if (v.isNull(i)) return null;
        if (v instanceof BigIntVector x) return x.get(i);
        if (v instanceof IntVector x) return x.get(i);
        if (v instanceof SmallIntVector x) return x.get(i);
        if (v instanceof TinyIntVector x) return x.get(i);
        if (v instanceof UInt8Vector x) return x.getObjectNoOverflow(i);
        if (v instanceof UInt4Vector x) return x.getObjectNoOverflow(i);
        if (v instanceof UInt2Vector x) return (int) x.get(i);
        if (v instanceof UInt1Vector x) return (short) x.getObjectNoOverflow(i);
        if (v instanceof Float8Vector x) return x.get(i);
        if (v instanceof Float4Vector x) return x.get(i);
        if (v instanceof BitVector x) return x.get(i) != 0;
        if (v instanceof VarCharVector x) return new String(x.get(i), StandardCharsets.UTF_8);
        if (v instanceof LargeVarCharVector x) return new String(x.get(i), StandardCharsets.UTF_8);
        if (v instanceof DecimalVector x) return x.getObject(i);
        if (v instanceof DateDayVector x) return LocalDate.ofEpochDay(x.get(i));
        if (v instanceof TimeMicroVector x) return LocalTime.ofNanoOfDay(x.get(i) * 1000L);
        if (v instanceof TimeStampMicroTZVector x) return instante(x.get(i), 1_000_000L);
        if (v instanceof TimeStampMicroVector x) return LocalDateTime.ofEpochSecond(Math.floorDiv(x.get(i), 1_000_000L), (int) (Math.floorMod(x.get(i), 1_000_000L) * 1000), ZoneOffset.UTC);
        if (v instanceof TimeStampNanoTZVector x) return instante(x.get(i), 1_000_000_000L);
        if (v instanceof TimeStampNanoVector x) return LocalDateTime.ofEpochSecond(Math.floorDiv(x.get(i), 1_000_000_000L), (int) Math.floorMod(x.get(i), 1_000_000_000L), ZoneOffset.UTC);
        if (v instanceof TimeStampMilliTZVector x) return instante(x.get(i), 1_000L);
        if (v instanceof TimeStampMilliVector x) return LocalDateTime.ofEpochSecond(Math.floorDiv(x.get(i), 1_000L), (int) (Math.floorMod(x.get(i), 1_000L) * 1_000_000), ZoneOffset.UTC);
        if (v instanceof TimeStampSecTZVector x) return Instant.ofEpochSecond(x.get(i));
        if (v instanceof TimeStampSecVector x) return LocalDateTime.ofEpochSecond(x.get(i), 0, ZoneOffset.UTC);
        if (v instanceof VarBinaryVector x) return x.get(i);
        if (v instanceof LargeVarBinaryVector x) return x.get(i);
        if (v instanceof StructVector st) {
            Map<String, Object> m = new LinkedHashMap<>();
            for (FieldVector c : st.getChildrenFromFields()) m.put(c.getName(), valorDe(c, i));
            return m;
        }
        // Un map de Arrow ES una lista de struct(key, value): sale como List<Map>.
        if (v instanceof ListVector l) {
            FieldVector datos = l.getDataVector();
            List<Object> out = new ArrayList<>();
            for (int j = l.getElementStartIndex(i); j < l.getElementEndIndex(i); j++) out.add(valorDe(datos, j));
            return out;
        }
        Object o = v.getObject(i);
        return o instanceof org.apache.arrow.vector.util.Text t ? t.toString() : o;
    }

    private static Instant instante(long n, long porSegundo) {
        return Instant.ofEpochSecond(Math.floorDiv(n, porSegundo), Math.floorMod(n, porSegundo) * (1_000_000_000L / porSegundo));
    }

    /** El tipo de Arrow de un campo, con el nombre que {@code pyarrow} le da. */
    public static String nombreArrow(Field f) {
        ArrowType t = f.getType();
        if (t instanceof ArrowType.Int x) return (x.getIsSigned() ? "int" : "uint") + x.getBitWidth();
        if (t instanceof ArrowType.FloatingPoint x) return switch (x.getPrecision()) { case HALF -> "halffloat"; case SINGLE -> "float"; case DOUBLE -> "double"; };
        if (t instanceof ArrowType.Bool) return "bool";
        if (t instanceof ArrowType.Utf8) return "string";
        if (t instanceof ArrowType.LargeUtf8) return "large_string";
        if (t instanceof ArrowType.Binary) return "binary";
        if (t instanceof ArrowType.LargeBinary) return "large_binary";
        if (t instanceof ArrowType.Decimal x) return "decimal" + x.getBitWidth() + "(" + x.getPrecision() + ", " + x.getScale() + ")";
        if (t instanceof ArrowType.Date x) return x.getUnit() == org.apache.arrow.vector.types.DateUnit.DAY ? "date32[day]" : "date64[ms]";
        if (t instanceof ArrowType.Time x) return "time" + x.getBitWidth() + "[" + unidad(x.getUnit()) + "]";
        if (t instanceof ArrowType.Timestamp x) return "timestamp[" + unidad(x.getUnit()) + (x.getTimezone() == null ? "]" : ", tz=UTC]");
        if (t instanceof ArrowType.List) return "list<item: " + nombreArrow(f.getChildren().get(0)) + ">";
        if (t instanceof ArrowType.Struct) { StringBuilder b = new StringBuilder("struct<"); for (int i = 0; i < f.getChildren().size(); i++) { Field c = f.getChildren().get(i); if (i > 0) b.append(", "); b.append(c.getName()).append(": ").append(nombreArrow(c)); } return b.append(">").toString(); }
        if (t instanceof ArrowType.Map) { Field par = f.getChildren().get(0); return "map<" + nombreArrow(par.getChildren().get(0)) + ", " + nombreArrow(par.getChildren().get(1)) + ">"; }
        if (t instanceof ArrowType.Null) return "null";
        return t.toString().toLowerCase(java.util.Locale.ROOT);
    }

    private static String unidad(org.apache.arrow.vector.types.TimeUnit u) {
        return switch (u) { case SECOND -> "s"; case MILLISECOND -> "ms"; case MICROSECOND -> "us"; case NANOSECOND -> "ns"; };
    }

    private static final long ENTERO_EXACTO = 1L << 53;

    /**
     * Un valor del contrato → el JSON de la consola (0032 §1): entero → número si
     * |x| ≤ 2⁵³, si no cadena · decimal → cadena siempre (salvo el de escala
     * 0, que es un entero y va como tal) · float → número, y
     * NaN/Infinity/-Infinity como cadena · fecha {@code YYYY-MM-DD} · hora
     * {@code HH:MM:SS[.ffffff]} · fecha-hora sin zona en ISO con {@code T} ·
     * instante en UTC con {@code Z} · bytes en base64 · lista → array · struct →
     * objeto · map → {@code [{key, value}]}. Nada se degrada en silencio. Es el
     * MISMO JSON que emiten los agentes de Python y de Node.
     */
    public static Object jsonDe(Object v) {
        if (v == null || v instanceof String || v instanceof Boolean) return v;
        if (v instanceof Integer || v instanceof Short || v instanceof Byte) return v;
        if (v instanceof Long n) return n >= -ENTERO_EXACTO && n <= ENTERO_EXACTO ? (Object) n : (Object) n.toString();
        if (v instanceof java.math.BigInteger n) return n.bitLength() < 54 ? (Object) n.longValue() : (Object) n.toString();
        // Un decimal de escala 0 (un HUGEINT: `sum(1)`, `count`) es un entero y va como los enteros; con decimales, cadena siempre.
        if (v instanceof java.math.BigDecimal d) return d.scale() == 0 ? jsonDe(d.toBigInteger()) : d.toPlainString();
        if (v instanceof Double d) return d.isNaN() ? "NaN" : d.isInfinite() ? (d > 0 ? "Infinity" : "-Infinity") : (Object) d;
        if (v instanceof Float f) return f.isNaN() ? "NaN" : f.isInfinite() ? (f > 0 ? "Infinity" : "-Infinity") : (Object) f.doubleValue();
        if (v instanceof LocalDate d) return d.toString();
        if (v instanceof LocalTime t) return hora(t);
        if (v instanceof LocalDateTime t) return t.toLocalDate() + "T" + hora(t.toLocalTime());
        if (v instanceof Instant t) { LocalDateTime l = LocalDateTime.ofInstant(t, ZoneOffset.UTC); return l.toLocalDate() + "T" + hora(l.toLocalTime()) + "Z"; }
        if (v instanceof byte[] b) return Base64.getEncoder().encodeToString(b);
        if (v instanceof Map<?, ?> m) { Map<String, Object> out = new LinkedHashMap<>(); for (Map.Entry<?, ?> e : m.entrySet()) out.put(String.valueOf(e.getKey()), jsonDe(e.getValue())); return out; }
        if (v instanceof Iterable<?> it) { List<Object> out = new ArrayList<>(); for (Object x : it) out.add(jsonDe(x)); return out; }
        if (v instanceof Object[] a) return jsonDe(java.util.Arrays.asList(a));
        // Lo que JDBC daría si alguien lo usa a mano: se acerca al contrato.
        if (v instanceof java.sql.Timestamp t) return jsonDe(t.toLocalDateTime());
        if (v instanceof java.sql.Date d) return jsonDe(d.toLocalDate());
        if (v instanceof java.sql.Time t) return jsonDe(t.toLocalTime());
        if (v instanceof java.util.Date d) return jsonDe(d.toInstant());
        return String.valueOf(v);
    }

    /** {@code HH:MM:SS}, y la fracción sólo si no es cero, sin ceros de más. */
    private static String hora(LocalTime t) {
        String s = String.format("%02d:%02d:%02d", t.getHour(), t.getMinute(), t.getSecond());
        if (t.getNano() == 0) return s;
        String f = String.format("%09d", t.getNano()).replaceAll("0+$", "");
        return s + "." + f;
    }

    /**
     * Lo que devuelven {@code over()} y {@code sql()} ({@link Filas}), o una
     * lista de mapas cualquiera → la salida {@code tabla} del contrato (0032 §1,
     * columna «JSON de la consola»): el MISMO JSON que emiten los agentes de
     * Python y de Node. Sin tipos declarados se infieren del primer valor.
     */
    @SuppressWarnings("unchecked")
    public static Map<String, Object> tabla(Object valor, int limite) {
        if (!(valor instanceof List<?> lista) || lista.isEmpty()) return null;
        for (Object f : lista) if (!(f instanceof Map)) return null;
        Map<String, String> tipos = valor instanceof Filas fs ? fs.tipos : null;
        List<String> columnas = new ArrayList<>(tipos != null ? tipos.keySet() : ((Map<String, Object>) lista.get(0)).keySet());
        List<Map<String, Object>> cols = new ArrayList<>();
        for (String c : columnas) {
            String tipo = tipos != null ? tipos.get(c) : null;
            if (tipo == null) {
                tipo = "null";
                for (Object f : lista) {
                    Object v = ((Map<String, Object>) f).get(c);
                    if (v != null) { tipo = tipoInferido(v); break; }
                }
            }
            Map<String, Object> col = new LinkedHashMap<>();
            col.put("name", c);
            col.put("type", tipo);
            cols.add(col);
        }
        List<List<Object>> filas = new ArrayList<>();
        for (Object f : lista.subList(0, Math.min(limite, lista.size()))) {
            List<Object> fila = new ArrayList<>();
            for (String c : columnas) fila.add(jsonDe(((Map<String, Object>) f).get(c)));
            filas.add(fila);
        }
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("columnas", cols);
        m.put("filas", filas);
        m.put("total", valor instanceof Filas fs && fs.total != null ? fs.total : (Object) lista.size());
        m.put("limite", limite);
        return m;
    }

    /** El tipo de Arrow de un valor suelto, para lo que no viene de {@code over()}/{@code sql()}. */
    public static String tipoInferido(Object v) {
        if (v instanceof Long || v instanceof java.math.BigInteger) return "int64";
        if (v instanceof Integer) return "int32";
        if (v instanceof Short) return "int16";
        if (v instanceof Byte) return "int8";
        if (v instanceof Double) return "double";
        if (v instanceof Float) return "float";
        if (v instanceof java.math.BigDecimal d) return "decimal128(" + Math.max(d.precision(), d.scale()) + ", " + Math.max(d.scale(), 0) + ")";
        if (v instanceof Boolean) return "bool";
        if (v instanceof String) return "string";
        if (v instanceof java.time.LocalDate) return "date32[day]";
        if (v instanceof java.time.LocalTime) return "time64[us]";
        if (v instanceof java.time.LocalDateTime) return "timestamp[us]";
        if (v instanceof java.time.Instant || v instanceof java.util.Date) return "timestamp[us, tz=UTC]";
        if (v instanceof byte[]) return "binary";
        if (v instanceof Map) return "struct";
        if (v instanceof Iterable || v instanceof Object[]) return "list";
        return v.getClass().getSimpleName().toLowerCase(java.util.Locale.ROOT);
    }

    /** Lo que era {@code llano}: el nombre se conserva para quien lo llamara. */
    public static Object llano(Object v) { return jsonDe(v); }
}
