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
import org.apache.arrow.vector.ipc.ArrowStreamReader;
import org.apache.arrow.c.ArrowArrayStream;
import org.apache.arrow.c.Data;
import org.apache.arrow.vector.types.pojo.ArrowType;
import org.apache.arrow.vector.types.pojo.Field;

/**
 * {@code ore} · the session SDK for Java (0031 W3.4). Same contract as
 * {@code puesto/python/ore}; the session imports it statically
 * ({@code import static ore.Ore.*;}), so a cell writes
 * {@code sql("select …")}, {@code over("hr.espanoles")} and {@code person()}.
 *
 * <ul>
 *   <li>{@code over("<base>.<view>")} → the rows of that view's copy
 *       ({@link Rows}: a {@code List<Map<String,Object>>} with {@code types},
 *       {@code total} and {@code truncated}), read by DuckDB and delivered via
 *       <b>Arrow</b> ({@code arrowExportStream}): exact for the 23 types of the
 *       contract (0032 T4), where the JDBC mapping got four wrong.</li>
 *   <li>{@code sql("select … from p.v")} → the rows of the result: each
 *       {@code base.view} after FROM/JOIN is resolved, fetched once and
 *       registered as a DuckDB view.</li>
 *   <li>{@code arrow("<base>.<view>")} / {@code arrowSql("…")} → the
 *       batched {@code ArrowReader} ({@code VectorSchemaRoot}), no object per
 *       row: 14.6 M rows/s. Whoever asks for it closes it.</li>
 *   <li>{@code person()} → who opened the session ({@code persona:…}).</li>
 * </ul>
 *
 * <p>Values follow the type contract (0032 §1): {@code Long} for a 64-bit
 * integer, exact {@code BigDecimal}, {@code LocalDate}, {@code LocalTime},
 * {@code LocalDateTime} (wall time), {@code Instant} (an instant, in UTC),
 * {@code byte[]}, {@code List}, {@code Map}. {@code over()} and {@code sql()}
 * return every row, as Python does; given a {@code limit} they stop there and
 * say so ({@code truncated}), and with {@code strict} they fail instead of
 * truncating. Bulk without an object per row goes through {@code arrow()}.
 *
 * <p>The old Spanish names ({@code persona}, {@code Filas}, {@code tabla},
 * {@code jsonDe}, {@code nombreArrow}, {@code valorDe}, {@code tipoInferido},
 * {@code llano}, {@code LIMITE}, {@code EXTENSIONES}) still work as deprecated
 * aliases, and so do the old keys of the maps {@code write()} and
 * {@code declare()} return ({@link Result}).
 *
 * El código nunca ve el bucket ni una credencial: pregunta a {@code ore-serve}
 * QUÉ copia es (con la identidad del puesto, que la resuelve en nombre de la
 * persona) y baja el artefacto con la identidad del pod (el token del servidor
 * de metadatos y la API JSON de GCS). El sobre {@code ORECOPY1} se desenvuelve
 * aquí. Fuera del clúster (las pruebas) {@code ORE_ALMACEN=dir:/ruta}.
 */
public final class Ore {
    private Ore() {}

    // ── Los alias en español (S2) ───────────────────────────────────────────
    // Un solo interruptor: con `false` un alias delega sin decir nada; con
    // `true` avisa una vez por nombre (por el err de verdad, no el de la celda).
    static final boolean AVISAR_ALIAS = false;
    private static final Set<String> AVISADOS = java.util.concurrent.ConcurrentHashMap.newKeySet();

    static void alias(String viejo, String nuevo) {
        if (AVISAR_ALIAS && AVISADOS.add(viejo))
            new java.io.PrintStream(new java.io.FileOutputStream(java.io.FileDescriptor.err), true, StandardCharsets.UTF_8)
                .println("ore: " + viejo + " is deprecated: use " + nuevo);
    }

    /** Las claves en español de lo que el SDK devuelve → las inglesas. */
    static final Map<String, String> ES_EN = Map.ofEntries(
        Map.entry("tabla", "table"), Map.entry("filas", "rows"), Map.entry("operacion", "operation"),
        Map.entry("repetida", "repeated"), Map.entry("nombre", "name"), Map.entry("fichero", "file"),
        Map.entry("nueva", "created"), Map.entry("new", "created"),
        // 0049 JM3 · Transaction.commit(), como el `_ES_EN` de Python
        Map.entry("transaccion", "transaction"), Map.entry("cambios", "changes"), Map.entry("procedencia", "provenance"),
        Map.entry("coleccion", "collection"), Map.entry("creada", "created"));

    /** Un mapa del servidor (claves de antes) → {@link Result} con las inglesas; si la inglesa ya venía, manda. */
    static Result enIngles(Map<String, Object> d) {
        Result r = new Result();
        for (Map.Entry<String, Object> e : d.entrySet()) {
            String en = ES_EN.getOrDefault(e.getKey(), e.getKey());
            if (!en.equals(e.getKey()) && d.containsKey(en)) continue;
            r.put(en, e.getValue());
        }
        return r;
    }

    /**
     * What {@code write()} and {@code declare()} return: a map with English keys
     * that still answers the old Spanish ones ({@code get("filas")} is
     * {@code get("rows")}). Iterating, printing or serializing it shows only
     * the English keys.
     */
    public static final class Result extends LinkedHashMap<String, Object> {
        @Override public Object get(Object k) {
            if (super.containsKey(k)) return super.get(k);
            String e = k == null ? null : ES_EN.get(k);
            return e == null ? null : super.get(e);
        }
        @Override public boolean containsKey(Object k) {
            if (super.containsKey(k)) return true;
            String e = k == null ? null : ES_EN.get(k);
            return e != null && super.containsKey(e);
        }
        @Override public Object getOrDefault(Object k, Object d) { return containsKey(k) ? get(k) : d; }
    }

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
        /** 0049 D3 · Quien da la credencial en cada petición, si el agente lo puso: así se
         *  renueva también en mitad de una celda (la del agente dura 300 s), no sólo entre
         *  celdas. Sin él, {@code cabeceras}. */
        volatile java.util.function.Supplier<Map<String, String>> credencial = null;

        /** {@code [código, cuerpo]}: el cuerpo, JSON como mapa (o {@code {error}}). */
        public Respuesta pedir(String metodo, String ruta, Object cuerpo, Duration plazo) throws IOException, InterruptedException {
            return pedir(metodo, ruta, cuerpo, plazo, Map.of());
        }

        public Respuesta pedir(String metodo, String ruta, Object cuerpo, Duration plazo, Map<String, String> extra) throws IOException, InterruptedException {
            // 0055 P1 (y 0049 JM3): mientras corre un Preview, nada que escriba sale de aquí;
            // el servidor lo exige también (su puerta), esto es para decirlo antes y mejor.
            if (ensayando != null && !metodo.equals("GET") && !metodo.equals("HEAD") && !noEscribe(ruta))
                throw new SecurityException("Preview writes nothing: `" + metodo + " " + ruta.split("\\?", 2)[0]
                    + "` would change the lake or the tree. Build writes");
            HttpRequest.Builder b = HttpRequest.newBuilder(URI.create(servidor + ruta)).timeout(plazo).header("accept", "application/json");
            java.util.function.Supplier<Map<String, String>> c = credencial;
            for (Map.Entry<String, String> e : (c != null ? c.get() : cabeceras).entrySet()) b.header(e.getKey(), e.getValue());
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

    /** Lo que una sesión pide sin escribir nada, fuera de {@code GET}: lo mismo que deja pasar la puerta del Preview en ore-serve. */
    static boolean noEscribe(String ruta) {
        String c = ruta.split("\\?", 2)[0];
        return c.startsWith("/puestos/") || c.equals("/federation/read")
            || c.matches("^/colecciones/[^/]+/[^/]+/[^/]+/items/resolver$")
            || c.matches("^/media/[^/]+/[^/]+/[^/]+/urls$")
            || (c.startsWith("/v1/") && c.endsWith("/metrics"));
    }

    public record Respuesta(int codigo, Map<String, Object> cuerpo) {
        public String error() { Object e = cuerpo.get("error"); return e == null ? cuerpo.toString() : String.valueOf(e); }
    }

    static final HttpClient HTTP = HttpClient.newBuilder().connectTimeout(Duration.ofSeconds(10)).build();
    public static final Puesto puesto = new Puesto();
    /** The session: what the agent knows about itself (the same object as {@code puesto}). */
    public static final Puesto session = puesto;

    static String env(String k, String d) { String v = System.getenv(k); return v == null || v.isEmpty() ? d : v; }

    /** Who opened the session ({@code persona:…}): the identity whatever you run here runs as. */
    public static String person() {
        if (puesto.persona.isEmpty()) throw new IllegalStateException("person(): the agent does not know yet who opened the session");
        return puesto.persona;
    }

    /** @deprecated use {@link #person()}. */
    @Deprecated
    public static String persona() { alias("persona()", "person()"); return person(); }

    // ── el almacén ──────────────────────────────────────────────────────────
    private static String tokenDelPod = "";
    private static long caducaElToken = 0;

    private static synchronized String tokenDeGoogle() throws IOException, InterruptedException {
        if (System.currentTimeMillis() < caducaElToken - 60_000) return tokenDelPod;
        HttpRequest q = HttpRequest.newBuilder(URI.create("http://169.254.169.254/computeMetadata/v1/instance/service-accounts/default/token"))
            .header("Metadata-Flavor", "Google").timeout(Duration.ofSeconds(10)).GET().build();
        HttpResponse<String> r = HTTP.send(q, HttpResponse.BodyHandlers.ofString());
        if (r.statusCode() != 200) throw new IOException("the metadata server answered " + r.statusCode());
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
            if (r.statusCode() != 200) throw new IOException("GCS answered " + r.statusCode() + " for " + clave);
            return r.body();
        }
        throw new IllegalStateException("ORE_ALMACEN=" + puesto.almacen + " is not a store: it is `gcs` or `dir:<path>`");
    }

    /** El sobre {@code ORECOPY1}: 8 de magia, 4 de largo (LE), la cabecera JSON, la carga Parquet. */
    private static byte[] desenvolver(byte[] crudo) {
        if (crudo.length < 12 || !new String(crudo, 0, 8, StandardCharsets.ISO_8859_1).equals("ORECOPY1"))
            throw new IllegalArgumentException("the artifact is not an ORE copy (no `ORECOPY1`)");
        int n = (crudo[8] & 0xff) | (crudo[9] & 0xff) << 8 | (crudo[10] & 0xff) << 16 | (crudo[11] & 0xff) << 24;
        byte[] carga = new byte[crudo.length - 12 - n];
        System.arraycopy(crudo, 12 + n, carga, 0, carga.length);
        return carga;
    }

    // Lo que la sesión leyó (por nombre), y el transform activo si lo hay.
    static final List<String> leidas = new ArrayList<>();
    record Transform(String nombre, List<String> inputs, String output) {}
    private static Transform transformActivo = null;
    /** El transform que corre, o {@code null} (0049 JM1: la media acota como {@code over()} y {@code sql()}). */
    static Transform transformActivo() { return transformActivo; }
    /** El trabajo que corre (W3.7 ④): `<ruta>@<commit>`, para la procedencia; lo pone el agente (la JVM no cambia su entorno). */
    public static String CODIGO = null;

    // ── 0055 T1·7 (JT3) · un build de Java ─────────────────────────────────
    /** El build que corre ({@code {transform, entrypoint, output, id, commit}}): lo pone su arnés
     *  ({@link Arnes}) y va a la procedencia de lo que escribe. {@code null} fuera de un build. */
    static volatile Map<String, Object> BUILD = null;
    /** D15 · El fichero cuya clase carga el arnés: mientras carga, {@code transform()} y
     *  {@code write()} fallan (el build llama al transform él, una vez). */
    static volatile String cargando = null;
    /** B2 · Lo que la celda deja para el informe (filas, snapshot, el error con su línea); el
     *  agente lo toma al terminar la celda. */
    private static Map<String, Object> informe = null;

    static synchronized void paraElInforme(Map<String, Object> d) {
        if (informe == null) informe = new LinkedHashMap<>();
        informe.putAll(d);
    }

    static synchronized Map<String, Object> tomarInforme() {
        Map<String, Object> i = informe;
        informe = null;
        return i;
    }

    /**
     * A transform (or a write) was called while a Build loads its class (ORE 0055 D15): the
     * build calls the transform itself, once. {@code line()}: the line of the build's file
     * that called it, if known.
     */
    public static final class TransformCalledWhileLoading extends IllegalStateException {
        private final Integer linea;

        TransformCalledWhileLoading(String mensaje, Integer linea) {
            super(mensaje);
            this.linea = linea;
        }

        /** @return the line of the build's file that called it, or {@code null} */
        public Integer line() { return linea; }
    }

    // ── 0055 T1·7 (JT4) · Preview: write() no escribe ──────────────────────
    /** Las filas que un Preview enseña de lo que {@code write()} escribiría. */
    static final int FILAS_DEL_PREVIEW = 100;
    /** El Preview armado ({@code {output, transform, visto}}), o {@code null}: mientras lo
     *  está, {@code write()} no escribe; devuelve lo que escribiría y lo guarda en {@code visto}. */
    private static volatile Map<String, Object> ensayando = null;

    /** Arma el Preview con la salida que el código declara y el método que se ensaya. */
    static void ensayo(String output, String transform) {
        Map<String, Object> e = new LinkedHashMap<>();
        e.put("output", corto(output, "the output"));
        e.put("transform", transform);
        e.put("visto", null);
        ensayando = e;
    }

    /** Lo que {@code write()} habría escrito hasta ahora en el Preview armado, sin desarmarlo. */
    static Object vistoDelEnsayo() {
        Map<String, Object> e = ensayando;
        return e == null ? null : e.get("visto");
    }

    /** Lo desarma, y devuelve lo que {@code write()} habría escrito (o {@code null} si no se llamó). */
    @SuppressWarnings("unchecked")
    static Map<String, Object> finDelEnsayo() {
        Map<String, Object> e = ensayando;
        ensayando = null;
        return e == null ? null : (Map<String, Object>) e.get("visto");
    }

    /** {@code write()} en un Preview: lo que escribiría —su esquema con el tipo con el que
     *  nacería en el lago, las primeras filas y el recuento—, sin escribir. Falla donde fallaría
     *  el build: una tabla sin filas, un tipo que el lago no tiene. La última escritura es la que se enseña. */
    private static Result ensayarEscritura(String nombre, Object datos, String modo) throws Exception {
        Map<String, Object> e = ensayando;
        if (!nombre.equals(e.get("output"))) throw new IllegalStateException("`" + nombre + "` is not the output of `" + e.get("transform") + "` (" + e.get("output") + "): a transform only writes what it declares");
        List<Map<String, Object>> campos = new ArrayList<>();
        byte[] ipc = ipcDe(datos, campos);
        Rows filas;
        long total = 0;
        try (org.apache.arrow.vector.ipc.ArrowStreamReader r = new org.apache.arrow.vector.ipc.ArrowStreamReader(new java.io.ByteArrayInputStream(ipc), asignador)) {
            Map<String, String> tipos = new LinkedHashMap<>();
            List<Map<String, Object>> primeras = new ArrayList<>();
            while (r.loadNextBatch()) {
                VectorSchemaRoot raiz = r.getVectorSchemaRoot();
                if (tipos.isEmpty()) for (Field f : raiz.getSchema().getFields()) tipos.put(f.getName(), arrowName(f));
                for (int i = 0; i < raiz.getRowCount() && primeras.size() < FILAS_DEL_PREVIEW; i++) {
                    Map<String, Object> fila = new LinkedHashMap<>();
                    for (FieldVector v : raiz.getFieldVectors()) fila.put(v.getName(), valueAt(v, i));
                    primeras.add(fila);
                }
                total += raiz.getRowCount();
            }
            filas = new Rows(tipos, total, total > primeras.size());
            filas.addAll(primeras);
        }
        Map<String, Object> vista = new LinkedHashMap<>(table(filas, FILAS_DEL_PREVIEW));
        List<Map<String, Object>> columnas = new ArrayList<>();
        List<?> dadas = (List<?>) vista.get("columnas");
        for (int i = 0; i < dadas.size(); i++) {
            @SuppressWarnings("unchecked")
            Map<String, Object> c = new LinkedHashMap<>((Map<String, Object>) dadas.get(i));
            if (i < campos.size()) c.put("iceberg", campos.get(i).get("type"));
            columnas.add(c);
        }
        vista.put("columnas", columnas);
        vista.put("total", total);
        vista.put("output", nombre);
        vista.put("mode", modo);
        e.put("visto", vista);
        Result out = new Result();
        out.put("table", nombre); out.put("rows", total); out.put("snapshot", ""); out.put("metadata_location", "");
        out.put("operation", ""); out.put("repeated", false); out.put("mode", modo); out.put("added", total);
        out.put("before", 0L); out.put("preview", true);
        return out;
    }

    /** D15: si el arnés está cargando una clase, {@code que} no corre. */
    private static void noAlCargar(String que) {
        String f = cargando;
        if (f == null) return;
        // La línea que lo llama al cargar: la del inicializador (`<clinit>`) si está en la traza.
        Throwable aqui = new Throwable();
        String nombre = f.substring(f.lastIndexOf('/') + 1);
        Integer linea = java.util.Arrays.stream(aqui.getStackTrace())
            .filter(m -> nombre.equals(m.getFileName()) && "<clinit>".equals(m.getMethodName()) && m.getLineNumber() > 0)
            .map(StackTraceElement::getLineNumber).findFirst().orElse(lineaEn(aqui, f));
        throw new TransformCalledWhileLoading(que + " was called while the build loads `" + f + "`: the build calls the transform itself, once; call nothing when the class loads (a `static {}` block, a field initializer)", linea);
    }

    /** La línea de {@code fichero} (una ruta) en la traza de {@code e} o de sus causas: el primer marco de ese fichero. */
    static Integer lineaEn(Throwable e, String fichero) {
        String nombre = fichero.substring(fichero.lastIndexOf('/') + 1);
        for (Throwable t = e; t != null; t = t.getCause()) {
            for (StackTraceElement m : t.getStackTrace()) {
                if (nombre.equals(m.getFileName()) && m.getLineNumber() > 0) return m.getLineNumber();
            }
        }
        return null;
    }

    static final String DEFAULT = "default";

    /** {@code base.nombre} o {@code base.schema.nombre} (0038) → la forma corta, la clave del
     *  árbol: {@code base.nombre} en {@code default}, {@code base.schema.nombre} en otro schema. */
    static String corto(String nombre, String que) {
        String[] p = nombre == null ? new String[0] : nombre.split("\\.", -1);
        if ((p.length != 2 && p.length != 3) || java.util.Arrays.stream(p).anyMatch(String::isEmpty))
            throw new IllegalArgumentException(que + " is `<base>.<schema>.<name>` (or `<base>.<name>`, in `default`), not " + nombre);
        return p.length == 3 && p[1].equals(DEFAULT) ? p[0] + "." + p[2] : String.join(".", p);
    }

    /** La forma corta → {base, schema, nombre}. */
    static String[] partes(String corto) {
        String[] p = corto.split("\\.");
        return p.length == 2 ? new String[] {p[0], DEFAULT, p[1]} : p;
    }

    /** La ruta de {@code /v1} de una tabla, como Unity (0038 P4): la base es el {@code prefix}. */
    static String v1Tabla(String corto) {
        String[] p = partes(corto);
        return "/v1/" + p[0] + "/namespaces/" + p[1] + "/tables/" + p[2];
    }

    /** Un identificador de DuckDB entre comillas dobles (no confundir con {@code q}, que escapa un literal). */
    private static String ident(String x) { return "\"" + x.replace("\"", "\"\"") + "\""; }

    /** El nombre del árbol como vista de DuckDB con sus tres niveles (0038): un catálogo por
     *  base y un schema por schema; lo de {@code default}, con su alias en {@code main} (donde
     *  DuckDB busca un nombre de DOS partes). */
    private static void registra(Connection con, String corto, String fuente) throws SQLException {
        String[] p = partes(corto);
        try (Statement s = con.createStatement()) {
            s.execute("attach if not exists ':memory:' as " + ident(p[0]));
            s.execute("create schema if not exists " + ident(p[0]) + "." + ident(p[1]));
            s.execute("create or replace view " + ident(p[0]) + "." + ident(p[1]) + "." + ident(p[2]) + " as select * from " + fuente);
            if (p[1].equals(DEFAULT))
                s.execute("create or replace view " + ident(p[0]) + ".main." + ident(p[2]) + " as select * from " + ident(p[0]) + "." + ident(p[1]) + "." + ident(p[2]));
        }
    }

    /**
     * A transform (0031 §9, W3.7 ③): runs {@code body} with {@code inputs} as the only thing it may read
     * ({@code over}, {@code sql}) and {@code output} as the only thing it may write ({@code write});
     * anything else throws. What it writes carries {@code procedencia: {inputs, transform, …}}.
     *
     * @param name   the transform's name (for provenance); {@code "transform"} if empty
     * @param inputs {@code <base>.<schema>.<name>} it may read
     * @param output {@code <base>.<schema>.<name>} it may write
     * @param body   what it runs
     */
    public static <T> T transform(String name, List<String> inputs, String output, java.util.concurrent.Callable<T> body) throws Exception {
        String nombre = name; List<String> inputsDados = inputs; String outputDado = output; java.util.concurrent.Callable<T> cuerpo = body;
        noAlCargar("transform()");
        if (inputsDados == null) throw new IllegalArgumentException("transform(): `inputs` is a list of `<base>.<schema>.<name>`");
        List<String> entradas = inputsDados.stream().map(i -> corto(i, "transform(): each input")).toList();
        String salida = corto(outputDado, "transform(): `output`");
        if (entradas.contains(salida)) throw new IllegalArgumentException("transform(): `" + salida + "` cannot be both input and output");
        if (transformActivo != null) throw new IllegalStateException("transform(): `" + transformActivo.nombre() + "` is already running; a transform does not call another");
        transformActivo = new Transform(nombre == null || nombre.isEmpty() ? "transform" : nombre, List.copyOf(entradas), salida);
        decirTransform(transformActivo);
        try { return cuerpo.call(); } finally { transformActivo = null; decirTransform(null); }
    }

    /**
     * 0057 B4·6 · Una tabla de un origen se lee por más de un nombre: el de su fuente
     * ({@code s3.datos.clientes}) y el que le da una foreign database
     * ({@code vivo.datos.clientes}). Dentro de un transform vale cualquiera de los que
     * declara; y lo que lee una vista viva que declara, lo cubre ella.
     */
    private static void leeEnVivo(String tabla, List<String> nombres, List<String> vistas) {
        if (transformActivo == null) { lee(tabla); return; }
        List<String> todos = new ArrayList<>(List.of(tabla));
        todos.addAll(nombres.stream().sorted().toList());
        for (String n : todos) if (transformActivo.inputs().contains(corto(n, "a tree name"))) { lee(n); return; }
        for (String v : vistas) if (transformActivo.inputs().contains(corto(v, "a tree name"))) return;
        lee(nombres.isEmpty() ? tabla : nombres.stream().sorted().toList().get(0));
    }

    static void lee(String vistaDada) {
        String vista = corto(vistaDada, "a tree name");
        if (transformActivo != null && !transformActivo.inputs().contains(vista)) throw new IllegalStateException("`" + vista + "` is not in the inputs of `" + transformActivo.nombre() + "` (" + String.join(", ", transformActivo.inputs()) + "): a transform only reads what it declares");
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

    static Map<String, Object> procedencia(String nombre) {
        Map<String, Object> p = new LinkedHashMap<>();
        p.put("puesto", puesto.id);
        if (transformActivo != null) { p.put("inputs", transformActivo.inputs().stream().sorted().toList()); p.put("transform", transformActivo.nombre()); }
        // Fuera de un transform, lo que la sesión leyó SIN lo que se está escribiendo
        // (W3.7 gobierno ③: un dataset no sale de sí mismo).
        else p.put("leidas", leidas.stream().filter(l -> !l.equals(nombre)).sorted().toList());
        String codigo = CODIGO != null ? CODIGO : System.getenv("ORE_CODIGO");
        if (codigo != null && !codigo.isEmpty()) p.put("codigo", codigo);
        // 0055 T1·7: de qué build —`{transform, entrypoint, output, id, commit}`—, que su arnés pone.
        if (BUILD != null) p.put("build", BUILD);
        return p;
    }

    private static Map<String, Object> resolver(String vistaDada) throws IOException, InterruptedException {
        String vista = corto(vistaDada, "a tree name");
        lee(vista);
        Respuesta r = puesto.pedir("GET", "/puestos/" + puesto.id + "/datos/" + vista, null, Duration.ofSeconds(30));
        return oElError(r, vista);
    }

    /** Lo que ore-serve contestó por un nombre, o el error de siempre: el mismo para
     *  {@code over()} (GET datos) que para {@code sql()} (POST sql). */
    private static Map<String, Object> oElError(Respuesta r, String vista) throws IOException {
        if (r.codigo() == 409) throw new IllegalStateException("the copy of `" + vista + "` is not ready: " + r.error());
        if (r.codigo() == 404) throw new IllegalArgumentException("there is no `View` or `Dataset` `" + vista + "` in the tree");
        // El conducto de la lectura (0031 W3.7 gobierno ②): lo que el dataset lleva
        // no cabe por `contextSurface.workspace`. Se dice tal cual.
        if (r.codigo() == 403) throw new SecurityException(r.error() == null || r.error().isEmpty() ? "ore-serve does not allow reading `" + vista + "` from a session" : r.error());
        if (r.codigo() != 200) throw new IOException("ore-serve answered " + r.codigo() + " for `" + vista + "`: " + r.error());
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
            // En `memory`, y nombradas con él: dentro de una vista de un catálogo adjunto (una
            // base, 0038) un schema sin cualificar se busca en ESE catálogo.
            try (Statement s = con.createStatement()) { s.execute("create schema if not exists memory.\"" + ESQUEMA_DE_DATASETS + "\""); }
            Object ds = r.get("datasets");
            if (ds instanceof Map<?, ?> mapaDs) {
                for (Map.Entry<?, ?> e : mapaDs.entrySet()) {
                    String d = String.valueOf(e.getKey());
                    String fuente = fuenteDeRespuesta(d, (Map<String, Object>) e.getValue());
                    try (Statement s = con.createStatement()) {
                        s.execute("create or replace view memory.\"" + ESQUEMA_DE_DATASETS + "\".\"" + d.replace("\"", "\"\"") + "\" as select * from " + fuente);
                    }
                }
            }
            return "(" + String.valueOf(consulta).replace("\"" + ESQUEMA_DE_DATASETS + "\".", "memory.\"" + ESQUEMA_DE_DATASETS + "\".") + ")";
        }
        Object m = r.get("metadata_location");
        if (m != null && !String.valueOf(m).isEmpty()) {
            // La credencial de lectura que `datos` presta (W3.7 gobierno ②b).
            Object cred = r.get("credencial");
            Map<String, String> credencial = cred instanceof Map<?, ?> ? mapa(cred) : Map.of();
            if (String.valueOf(m).startsWith("s3://") && s3 == null) {
                if (credencial.get("s3.access-key-id") != null) s3 = credencial;
                else {
                    Respuesta l = puesto.pedir("GET", v1Tabla(corto(vista, "a tree name")), null, Duration.ofSeconds(30), DELEGAR);
                    Object cfg = l.cuerpo().get("config");
                    if (l.codigo() == 200 && cfg instanceof Map<?, ?> c && c.get("s3.access-key-id") != null) s3 = mapa(cfg);
                }
            }
            Object tok = credencial.get("gcs.oauth2.token");
            return iceberg(String.valueOf(m), tok == null ? null : String.valueOf(tok));
        }
        return "read_parquet('" + rutaSql(parquetDe(vista, r)) + "')";
    }

    /** Where the image leaves DuckDB's preinstalled extensions. */
    public static final Path EXTENSIONS = Path.of("/opt/ore/duckdb");
    /** @deprecated use {@link #EXTENSIONS}. */
    @Deprecated
    public static final Path EXTENSIONES = EXTENSIONS;
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

    // ── 0049 JM · la media en código (`Media`) ───────────────────────────

    /** {@code Ore.collection("db.schema.name")} (or {@code db.name}): a media collection. */
    public static Media.Collection collection(String name) { return new Media.Collection(name); }

    /** The media kinds a collection is of (OOS v1alpha17). */
    public static final List<String> MEDIA = List.of("document", "image", "audio", "video", "spreadsheet", "email");

    /** {@code create media collection name (media, formats)}: an empty <b>written</b> collection, which code fills with {@code collection(name).transaction()}. */
    public static Map<String, Object> createCollection(String name, String media, List<String> formats) throws Exception {
        return createCollection(name, media, formats, null, null, null, null, false);
    }

    /**
     * The same, with all it takes: {@code owner} only to give it to someone else (without it, it
     * belongs to whoever creates it, set by the server); {@code labels} ({@code gdpr.sensitivity:
     * high}) add to what is derived; {@code ifNotExists}: if it exists, {@code {created: false}}
     * instead of an error. An OOS code comes back as {@code IllegalArgumentException}. Returns
     * {@code {collection, created}}.
     */
    public static Map<String, Object> createCollection(String name, String media, List<String> formats, String owner,
                                                       String comment, Map<String, String> labels, String retention,
                                                       boolean ifNotExists) throws Exception {
        String nombre = corto(name, "create media collection: the name");
        String que = "create media collection " + nombre;
        if (!MEDIA.contains(media)) throw new IllegalArgumentException(que + ": `media` is one of " + String.join(", ", MEDIA) + ", not " + media);
        List<String> fs = new ArrayList<>();
        for (String f : formats == null ? List.<String>of() : formats) fs.add(f.toLowerCase(java.util.Locale.ROOT).replaceAll("^\\.+", ""));
        if (fs.isEmpty() || new java.util.HashSet<>(fs).size() != fs.size() || !fs.stream().allMatch(f -> f.matches("^[a-z0-9][a-z0-9.+-]*$")))
            throw new IllegalArgumentException(que + ": `formats` is a list of distinct extensions (`png`, `pdf`), not " + formats);
        String[] p = partes(nombre);
        String ruta = p[1].equals(DEFAULT) ? "/documentos/MediaCollection/" + p[0] + "/" + p[2] : "/documentos/MediaCollection/" + p[0] + "/" + p[1] + "/" + p[2];
        if (puesto.pedir("GET", ruta, null, Duration.ofSeconds(60)).codigo() == 200) {
            if (ifNotExists) {
                Result r = new Result();
                r.put("collection", nombre);
                r.put("created", false);
                return r;
            }
            throw new IllegalStateException(que + ": a collection with that name already exists (`ifNotExists` leaves it as it is)");
        }
        StringBuilder y = new StringBuilder("apiVersion: oos.dev/v1alpha19\nkind: MediaCollection\nmetadata:\n  name: ")
            .append(p[2]).append("\n  namespace: ").append(p[0]).append('\n');
        if (!p[1].equals(DEFAULT)) y.append("  schema: ").append(p[1]).append('\n');
        if (comment != null) y.append("  description: ").append(Json.escribir(comment)).append('\n');
        if (labels != null && !labels.isEmpty()) {
            List<String> ls = new ArrayList<>();
            for (Map.Entry<String, String> e : labels.entrySet()) ls.add(e.getKey() + ": " + e.getValue());
            y.append("  labels: { ").append(String.join(", ", ls)).append(" }\n");
        }
        y.append("spec:\n");
        if (owner != null) y.append("  owner: ").append(owner).append('\n');
        y.append("  media: ").append(media).append("\n  formats: [").append(String.join(", ", fs)).append("]\n");
        if (retention != null) y.append("  retention: ").append(retention).append('\n');
        declare(y.toString());
        Result r = new Result();
        r.put("collection", nombre);
        r.put("created", true);
        return r;
    }

    /** The bytes of many items, 16 at once, as they finish: one's error is a value and does not stop the others. {@code items} may be lazy ({@code c.items()}). */
    public static Iterable<Media.Result> readMany(Iterable<Media.Item> items) { return readMany(items, 16); }

    /** The bytes of many items, {@code threads} at once, as they finish. */
    public static Iterable<Media.Result> readMany(Iterable<Media.Item> items, int threads) { return Media.leerVarios(items, threads); }

    // ── 0057 C4 · una colección del lago en SQL: su listado ───────────────
    //
    // Lo mismo que el SDK de Python (`medios._relacion`): una fila por ítem,
    // sin bytes, con las diez columnas del listado (OOS v1alpha17 `04` §1). Las
    // páginas de `GET /media/{b}/{s}/{c}/items` —gobernadas: dentro de un
    // transform, sólo lo declarado— van a un NDJSON temporal y DuckDB las carga
    // en una tabla temporal con sus tipos; el fichero se borra.

    /** Las columnas del listado, en DuckDB. */
    private static final String COLUMNAS_DEL_LISTADO = "\"_item\" STRUCT(uri VARCHAR, collection VARCHAR, path VARCHAR, version VARCHAR, digest VARCHAR, size BIGINT, content_type VARCHAR, content_type_detected VARCHAR, checksum VARCHAR), \"path\" VARCHAR, \"version\" VARCHAR, "
        + "\"digest\" VARCHAR, \"size\" BIGINT, \"content_type\" VARCHAR, \"content_type_detected\" VARCHAR, "
        + "\"checksum\" VARCHAR, \"modified\" TIMESTAMPTZ, \"transaction\" VARCHAR";
    /** Los campos de `_item` (v1alpha17 `01` §3). */
    private static final List<String> CAMPOS_DEL_ITEM = List.of("uri", "collection", "path", "version", "digest", "size",
        "content_type", "content_type_detected", "checksum");
    private static int listados = 0;
    private static Map<String, String> ramaDelPuesto = null;

    /** La rama del puesto ({@code x-ore-rama}), preguntada una vez: una colección de la rama se ve desde su puesto. */
    static synchronized Map<String, String> ramaDelPuesto() throws IOException, InterruptedException {
        if (ramaDelPuesto == null) {
            Respuesta r = puesto.id.isEmpty() ? null : puesto.pedir("GET", "/puestos/" + puesto.id, null, Duration.ofSeconds(30));
            Object rama = r != null && r.codigo() == 200 ? r.cuerpo().get("rama") : null;
            ramaDelPuesto = rama == null || String.valueOf(rama).isEmpty() ? Map.of() : Map.of("x-ore-rama", String.valueOf(rama));
        }
        return ramaDelPuesto;
    }

    /** El listado de la colección {@code nombre}, como tabla temporal de DuckDB; devuelve su nombre cualificado. */
    @SuppressWarnings("unchecked")
    private static String listadoDeColeccion(Connection con, String nombre) throws Exception {
        String[] p = partes(corto(nombre, "a collection"));
        String ruta = "/media/" + p[0] + "/" + p[1] + "/" + p[2] + "/items";
        String tabla = "__ore_listado_" + (++listados);
        Path f = copiasPorDefecto().resolve(tabla + "-" + ProcessHandle.current().pid() + ".ndjson");
        Files.createDirectories(f.getParent());
        long filas = 0;
        try {
            try (java.io.BufferedWriter w = Files.newBufferedWriter(f, StandardCharsets.UTF_8)) {
                String cursor = null;
                do {
                    String q = "?limit=1000" + (cursor == null ? "" : "&cursor=" + URLEncoder.encode(cursor, StandardCharsets.UTF_8));
                    Respuesta r = puesto.pedir("GET", ruta + q, null, Duration.ofSeconds(90), ramaDelPuesto());
                    if (r.codigo() == 403) throw new SecurityException("the listing of `" + nombre + "`: " + r.error());
                    if (r.codigo() != 200) throw new IOException("the listing of `" + nombre + "`: ore-serve answered " + r.codigo() + ": " + r.error());
                    if (r.cuerpo().get("items") instanceof List<?> its) {
                        for (Object o : its) {
                            Map<String, Object> d = (Map<String, Object>) o;
                            Map<String, Object> item = new LinkedHashMap<>();
                            for (String c : CAMPOS_DEL_ITEM) item.put(c, d.get(c));
                            Map<String, Object> fila = new LinkedHashMap<>();
                            fila.put("_item", item);
                            for (String c : List.of("path", "version", "digest", "size", "content_type", "content_type_detected", "checksum", "modified", "transaction"))
                                fila.put(c, d.get(c));
                            w.write(Json.escribir(fila));
                            w.write('\n');
                            filas++;
                        }
                    }
                    Object c = r.cuerpo().get("cursor");
                    cursor = c == null || String.valueOf(c).isEmpty() ? null : String.valueOf(c);
                } while (cursor != null);
            }
            cargar(con, "json");
            try (Statement s = con.createStatement()) {
                s.execute("create or replace temp table " + ident(tabla) + " (" + COLUMNAS_DEL_LISTADO + ")");
                // `modified` llega como texto: un instante, o nulo si no se entiende.
                if (filas > 0) s.execute("insert into " + ident(tabla) + " select \"_item\", \"path\", \"version\", \"digest\", \"size\", "
                    + "\"content_type\", \"content_type_detected\", \"checksum\", try_cast(\"modified\" as TIMESTAMPTZ), \"transaction\" "
                    + "from read_json('" + rutaSql(f) + "', format='newline_delimited', columns={'_item': 'STRUCT(uri VARCHAR, collection VARCHAR, path VARCHAR, version VARCHAR, digest VARCHAR, size BIGINT, content_type VARCHAR, content_type_detected VARCHAR, checksum VARCHAR)', "
                    + "'path': 'VARCHAR', 'version': 'VARCHAR', 'digest': 'VARCHAR', 'size': 'BIGINT', 'content_type': 'VARCHAR', "
                    + "'content_type_detected': 'VARCHAR', 'checksum': 'VARCHAR', 'modified': 'VARCHAR', 'transaction': 'VARCHAR'})");
            }
        } finally {
            Files.deleteIfExists(f);
        }
        return "temp.main." + ident(tabla);
    }

    /** {@code LOAD} de una extensión: en la imagen está preinstalada; fuera, si falta, se instala una vez. */
    private static void cargar(Connection con, String extension) throws SQLException {
        try (Statement s = con.createStatement()) { s.execute("load " + extension); return; } catch (SQLException e) {
            if (Files.isDirectory(EXTENSIONS)) throw new SQLException("the image does not ship DuckDB's `" + extension + "` extension: it must be preinstalled in " + EXTENSIONS, e);
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
                throw new SQLException("this session has no DuckDB (duckdb_jdbc.jar on the classpath): sql() and over() cannot read Parquet");
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
                if (Files.isDirectory(EXTENSIONS)) s.execute("set extension_directory = '" + EXTENSIONS + "'");
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

    /** The version of this SDK's interface (S3): 2 is the English names. Code that ORE generates checks it. */
    public static final int API = 2;

    /** How many rows {@code over()} and {@code sql()} materialize unless told otherwise: all of them. */
    public static final int LIMIT = Integer.MAX_VALUE;
    /** @deprecated use {@link #LIMIT}. */
    @Deprecated
    public static final int LIMITE = LIMIT;

    /**
     * The old name of {@link Rows}, kept so existing code compiles: {@code over()} and
     * {@code sql()} return a {@link Rows}, which IS a {@code Filas} (it extends it), so
     * {@code Filas f = over(...)}, {@code f.tipos} and {@code f.truncada} keep working.
     * The fields live here; {@code Rows} adds nothing.
     *
     * @deprecated use {@link Rows} (and its fields {@code types}, {@code truncated}).
     */
    @Deprecated
    public static class Filas extends ArrayList<Map<String, Object>> {
        /** Column → Arrow type, with the name {@code pyarrow} gives it. */
        public final Map<String, String> types;
        /** How many rows there are (even if not all were materialized), or {@code null} if unknown. */
        public final Long total;
        /** Whether it stopped at the limit. */
        public final boolean truncated;
        /** @deprecated use {@link #types} (the same map). */
        @Deprecated public final Map<String, String> tipos;
        /** @deprecated use {@link #truncated}. */
        @Deprecated public final boolean truncada;
        Filas(Map<String, String> types, Long total, boolean truncated) {
            this.types = types; this.total = total; this.truncated = truncated;
            this.tipos = types; this.truncada = truncated;
        }
        /** Column → Arrow type. */
        public Map<String, String> types() { return types; }
        /** Total rows, or {@code null} if unknown. */
        public Long total() { return total; }
        /** Whether it stopped at the limit. */
        public boolean truncated() { return truncated; }
    }

    /** The rows {@code over()} and {@code sql()} return: the list, and what to know about it ({@code types}, {@code total}, {@code truncated}). */
    @SuppressWarnings("deprecation")
    public static final class Rows extends Filas {
        Rows(Map<String, String> types, Long total, boolean truncated) { super(types, total, truncated); }
        /** The origin tables this came from, read live (ADR 0053 F7·1); empty if none. */
        public List<String> readLive = List.of();
        /** The origin tables this came from, read live; empty if none. */
        public List<String> readLive() { return readLive; }
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
    private static Rows filasDe(ArrowReader lector, int limite, boolean estricto, Long total, String que) throws Exception {
        try (lector) {
            Map<String, String> tipos = new LinkedHashMap<>();
            List<Map<String, Object>> out = new ArrayList<>();
            boolean truncada = false;
            while (!truncada && lector.loadNextBatch()) {
                VectorSchemaRoot raiz = lector.getVectorSchemaRoot();
                if (tipos.isEmpty()) for (Field f : raiz.getSchema().getFields()) tipos.put(f.getName(), arrowName(f));
                List<FieldVector> vs = raiz.getFieldVectors();
                for (int i = 0; i < raiz.getRowCount(); i++) {
                    if (out.size() == limite) { truncada = true; break; }
                    Map<String, Object> fila = new LinkedHashMap<>();
                    for (FieldVector v : vs) fila.put(v.getName(), valueAt(v, i));
                    out.add(fila);
                }
            }
            if (tipos.isEmpty()) for (Field f : lector.getVectorSchemaRoot().getSchema().getFields()) tipos.put(f.getName(), arrowName(f));
            if (truncada && estricto) throw new IllegalStateException(que + ": the result exceeds " + limite + " rows; raise the limit, aggregate in sql(), use arrow() or drop strict");
            Rows filas = new Rows(tipos, truncada ? total : Long.valueOf(out.size()), truncada);
            filas.addAll(out);
            return filas;
        }
    }

    /**
     * Cada nombre del árbol que el texto lee, como vista de DuckDB. El texto entero va a
     * ore-serve ({@code POST /puestos/{id}/sql}), que dice qué nombres lee —tokenizador y
     * árbol como filtro, sin regex— y los resuelve como {@code over()}, en una ida y vuelta.
     * Lo que se lee de un origen en vivo (0053 F6, 0057 B4·2·1) va primero; las vistas
     * vivas, al final. Devuelve las tablas del origen que se leyeron en vivo.
     */
    @SuppressWarnings("unchecked")
    private static Set<String> registrar(Connection con, String texto, boolean estricto) throws Exception {
        Set<String> vivas = new java.util.TreeSet<>();
        // Sin puesto no hay árbol (--comprobar), y sin un punto no hay `a.b`: el motor solo.
        if (puesto.id.isEmpty() || texto.indexOf('.') < 0) return vivas;
        Respuesta r = puesto.pedir("POST", "/puestos/" + puesto.id + "/sql", Map.of("texto", texto), Duration.ofSeconds(60));
        if (r.codigo() != 200) {
            Object n = r.cuerpo() == null ? null : r.cuerpo().get("nombre");
            // 0053 F6: lo que el reparto niega (coste, gobierno, interruptor) trae su código.
            Object c = r.cuerpo() == null ? null : r.cuerpo().get("codigo");
            if (c != null) throw new OriginReadError(String.valueOf(c), r.error(), n == null ? null : String.valueOf(n));
            oElError(r, n == null ? "?" : String.valueOf(n));
        }
        Object fs = r.cuerpo() == null ? null : r.cuerpo().get("fuentes");
        if (!(fs instanceof Map<?, ?> crudas)) return vivas;
        Map<String, Map<String, Object>> fuentes = new TreeMap<>();
        for (Map.Entry<?, ?> e : crudas.entrySet())
            fuentes.put(String.valueOf(e.getKey()), e.getValue() instanceof Map<?, ?> m ? (Map<String, Object>) m : new LinkedHashMap<>());
        Map<String, Object> avisos = fuentes.remove("__avisos");
        if (avisos != null && avisos.get("avisos") instanceof List<?> l) for (Object a : l) avisa(String.valueOf(a));
        // Lo que se lee en vivo, primero (las vistas vivas lo nombran): una lectura por
        // tabla, y cada nombre que la dice, a ella.
        Map<String, String> enVivo = new LinkedHashMap<>();
        List<String> vistasVivas = fuentes.entrySet().stream().filter(e -> e.getValue().get("vistaFederada") != null).map(Map.Entry::getKey).toList();
        for (Map.Entry<String, Map<String, Object>> e : fuentes.entrySet()) {
            if (!(e.getValue().get("federada") instanceof Map<?, ?> lm)) continue;
            Map<String, Object> l = (Map<String, Object>) lm;
            String t = String.valueOf(l.get("tabla"));
            if (!enVivo.containsKey(t)) {
                List<String> nombres = new ArrayList<>();
                for (Map.Entry<String, Map<String, Object>> x : fuentes.entrySet())
                    if (!x.getKey().equals(t) && x.getValue().get("federada") instanceof Map<?, ?> xl && t.equals(String.valueOf(xl.get("tabla"))))
                        nombres.add(x.getKey());
                leeEnVivo(t, nombres, vistasVivas);
                enVivo.put(t, lecturaEnVivo(con, l, estricto));
                vivas.add(t);
            }
            registra(con, e.getKey(), enVivo.get(t));
        }
        for (Map.Entry<String, Map<String, Object>> e : fuentes.entrySet()) {
            Map<String, Object> rd = e.getValue();
            if (rd.get("federada") != null || rd.get("vistaFederada") != null) continue;
            lee(e.getKey());
            // 0057 C4: una colección del lago, su listado (una fila por ítem, sin bytes).
            if (rd.get("collection") != null) {
                registra(con, e.getKey(), listadoDeColeccion(con, e.getKey()));
                continue;
            }
            registra(con, e.getKey(), fuenteDeRespuesta(e.getKey(), rd));
        }
        // Las vistas vivas, al final: DuckDB enlaza una vista al crearla, y una que junta
        // un origen con un dataset (0057 B4·3·2) necesita los dos ya puestos.
        for (Map.Entry<String, Map<String, Object>> e : fuentes.entrySet()) {
            Object v = e.getValue().get("vistaFederada");
            if (v == null) continue;
            lee(e.getKey());
            registra(con, e.getKey(), "(" + v + ")");
        }
        return vivas;
    }

    /**
     * A live read of an origin (ADR 0053 F6) that could not be done, or was cut and
     * {@code strict} was asked: {@code code()} is what the Federation Engine said
     * ({@code OOS2051}, a cost or governance code, {@code cortado}), {@code table()}
     * the origin table.
     */
    public static final class OriginReadError extends RuntimeException {
        private final String code, table;
        public OriginReadError(String code, String message, String table) {
            super((code == null ? "" : code + ": ") + message);
            this.code = code; this.table = table;
        }
        public String code() { return code; }
        public String table() { return table; }
    }

    /** Un aviso a quien corre la celda (lo que en Python es {@code warnings.warn}). */
    private static void avisa(String m) { System.err.println("warning: " + m); }

    private static int lecturas = 0;

    /**
     * 0053 F6·2 · <b>Una lectura en vivo</b>, ya repartida por ore-serve: se pide a
     * {@code /federation/read} —con su gobierno, su tope y su huella— y el Arrow queda en
     * DuckDB como una tabla temporal: el flujo de Arrow se lee una sola vez, y la consulta
     * la puede nombrar las veces que quiera (una junta consigo misma, una unión). Devuelve
     * el nombre, ya cualificado.
     */
    @SuppressWarnings("unchecked")
    private static String lecturaEnVivo(Connection con, Map<String, Object> l, boolean estricto) throws Exception {
        String tabla = String.valueOf(l.get("tabla"));
        Map<String, Object> cuerpo = new LinkedHashMap<>();
        cuerpo.put("tabla", tabla);
        cuerpo.put("columnas", l.get("columnas") instanceof List<?> c ? c : List.of());
        cuerpo.put("filtros", l.get("empujados") instanceof List<?> f ? f : List.of());
        if (l.get("limit") != null) cuerpo.put("limit", l.get("limit"));
        if (l.get("orderBy") instanceof List<?> ob && !ob.isEmpty()) {
            List<Map<String, Object>> orden = new ArrayList<>();
            for (Object o : ob) {
                Map<String, Object> m = (Map<String, Object>) o;
                orden.add(Map.of("columna", m.get("columna"), "direccion", Boolean.TRUE.equals(m.get("desc")) ? "desc" : "asc"));
            }
            cuerpo.put("orderBy", orden);
        }
        HttpRequest.Builder b = HttpRequest.newBuilder(URI.create(puesto.servidor + "/federation/read")).timeout(Duration.ofSeconds(120))
            .header("content-type", "application/json").header("accept", "application/vnd.apache.arrow.stream")
            .POST(HttpRequest.BodyPublishers.ofString(Json.escribir(cuerpo)));
        for (Map.Entry<String, String> e : puesto.cabeceras.entrySet()) b.header(e.getKey(), e.getValue());
        if (!puesto.id.isEmpty()) b.header("x-ore-puesto", puesto.id);
        HttpResponse<byte[]> resp = HTTP.send(b.build(), HttpResponse.BodyHandlers.ofByteArray());
        if (resp.statusCode() != 200) {
            String t = new String(resp.body(), StandardCharsets.UTF_8);
            Map<String, Object> err;
            try { err = Json.objeto(t); } catch (RuntimeException e) { err = Map.of("mensaje", t.strip()); }
            Object m = err.get("mensaje") != null ? err.get("mensaje") : err.get("error") != null ? err.get("error") : t;
            Object c = err.get("codigo");
            throw new OriginReadError(c == null ? null : String.valueOf(c), String.valueOf(m), tabla);
        }
        // Cómo acabó: lo de los trailers, que `java.net.http` no lee (F6·1).
        String id = resp.headers().firstValue("ore-lectura").orElse(null);
        if (id != null) {
            Respuesta f = puesto.pedir("GET", "/federation/read/" + id, null, Duration.ofSeconds(30));
            Object estado = f.cuerpo().get("estado");
            if (f.codigo() == 200 && estado != null && !"completo".equals(estado)) {
                Object motivo = f.cuerpo().get("motivo");
                String m = "live read of `" + tabla + "` was cut (" + (motivo != null ? motivo : estado) + ") at " + f.cuerpo().get("filas")
                    + " rows: the answer is incomplete; filter more or read from a copy";
                if (estricto) throw new OriginReadError("cortado", m, tabla);
                avisa(m);
            }
        }
        String nombre = "__ore_vivo_" + (++lecturas);
        try (ArrowStreamReader lector = new ArrowStreamReader(new java.io.ByteArrayInputStream(resp.body()), asignador);
             ArrowArrayStream flujo = ArrowArrayStream.allocateNew(asignador)) {
            // Una vista viva se registra tal cual (F6·1) y puede nombrar columnas que la
            // sentencia no usa —por eso no se pidieron al origen—: van como nulos.
            Set<String> hay = new LinkedHashSet<>();
            for (Field f : lector.getVectorSchemaRoot().getSchema().getFields()) hay.add(f.getName());
            StringBuilder nulas = new StringBuilder();
            if (l.get("columnasDeLaTabla") instanceof List<?> todas)
                for (Object c : todas) if (!hay.contains(String.valueOf(c))) nulas.append(", null as ").append(ident(String.valueOf(c)));
            Data.exportArrayStream(asignador, lector, flujo);
            String flujoNombre = nombre + "_flujo";
            ((org.duckdb.DuckDBConnection) con).registerArrowStream(flujoNombre, flujo);
            try (Statement s = con.createStatement()) {
                s.execute("create or replace temp table " + ident(nombre) + " as select *" + nulas + " from " + ident(flujoNombre));
            }
        }
        return "temp.main." + ident(nombre);
    }

    /**
     * What each origin is asked for and what DuckDB does (ADR 0053 F5): per live table,
     * the columns, the filters and the {@code limit} pushed to it, what stays in the
     * engine, its cost and the warnings. Prints it and returns the plan. Opens nothing.
     */
    @SuppressWarnings("unchecked")
    public static Map<String, Object> explain(String query) throws Exception {
        if (query == null || query.isBlank()) throw new IllegalArgumentException("explain() needs a query");
        Respuesta r = puesto.pedir("POST", "/puestos/" + puesto.id + "/explain", Map.of("texto", query), Duration.ofSeconds(60));
        if (r.codigo() != 200) throw new IOException("ore-serve answered " + r.codigo() + " to explain(): " + r.error());
        Object t = r.cuerpo().get("texto");
        if (t != null) System.out.println(String.valueOf(t).stripTrailing());
        Object plan = r.cuerpo().get("plan");
        return plan instanceof Map<?, ?> m ? (Map<String, Object>) m : new LinkedHashMap<>();
    }

    /** The copy of {@code <base>.<view>} as rows, all of them. */
    public static Rows over(String view) throws Exception { return over(view, LIMIT, false); }

    /**
     * The copy as rows, up to {@code limit}; with {@code strict}, it fails if it does not fit.
     *
     * @param view   {@code <base>.<schema>.<name>} (or {@code <base>.<name>})
     * @param limit  how many rows to materialize at most
     * @param strict throw instead of truncating
     */
    public static Rows over(String view, int limit, boolean strict) throws Exception {
        String vista = view; int limite = limit; boolean estricto = strict;
        String fuente;
        try {
            fuente = fuenteDe(vista);
        } catch (IllegalStateException | IllegalArgumentException | IOException sinCopia) {
            // ORE 0057 B4·2·1: lo que no tiene copia —una tabla que expone una foreign
            // database, una vista sobre el origen— se lee en vivo, como lo lee `sql()`
            // (el Federation Engine). Si tampoco así, manda la primera respuesta: es la
            // que dice por qué no hay copia.
            try { return sql("select * from " + vista, limite, estricto); } catch (Exception e) { throw sinCopia; }
        }
        Connection con = duckdb();
        long total;
        try (Statement s = con.createStatement(); ResultSet rs = s.executeQuery("select count(*) from " + fuente)) { rs.next(); total = rs.getLong(1); }
        if (estricto && total > limite) throw new IllegalStateException("over(" + vista + "): the copy has " + total + " rows and the limit is " + limite + "; raise the limit, aggregate in sql(), use arrow() or drop strict");
        return filasDe(exportar(con.createStatement(), "select * from " + fuente, 8192), limite, estricto, total, "over(" + vista + ")");
    }

    /** SQL (DuckDB) over the copies: each {@code base.view} after FROM/JOIN is resolved, fetched once and registered as a view. All the rows. */
    public static Rows sql(String text) throws Exception { return sql(text, LIMIT, false); }

    /**
     * SQL over the copies, up to {@code limit} rows; with {@code strict}, it fails if the result does not fit.
     *
     * @param text   the query (DuckDB)
     * @param limit  how many rows to materialize at most
     * @param strict throw instead of truncating
     */
    public static Rows sql(String text, int limit, boolean strict) throws Exception {
        // 0057 B4·6 · en un build, una lectura en vivo cortada lo hace fallar (como en Python).
        String texto = text; int limite = limit; boolean estricto = strict || BUILD != null;
        if (texto == null || texto.isBlank()) throw new IllegalArgumentException("sql() takes a query");
        Connection con = duckdb();
        Set<String> vivas = registrar(con, texto, estricto);
        Rows filas = filasDe(exportar(con.createStatement(), texto, 8192), limite, estricto, null, "sql()");
        filas.readLive = List.copyOf(vivas);
        return filas;
    }

    /** The whole copy, in Arrow batches: {@code while (r.loadNextBatch()) { VectorSchemaRoot root = r.getVectorSchemaRoot(); … }}. Close it when done. */
    public static ArrowReader arrow(String view) throws Exception {
        String fuente;
        try {
            fuente = fuenteDe(view);
        } catch (IllegalStateException | IllegalArgumentException | IOException sinCopia) {
            try { return arrowSql("select * from " + view); } catch (Exception e) { throw sinCopia; }
        }
        return exportar(duckdb().createStatement(), "select * from " + fuente, 65_536);
    }

    /** The result of a query, in Arrow batches. Close it when done. */
    public static ArrowReader arrowSql(String text) throws Exception {
        String texto = text;
        if (texto == null || texto.isBlank()) throw new IllegalArgumentException("arrowSql() takes a query");
        Connection con = duckdb();
        registrar(con, texto, false);
        return exportar(con.createStatement(), texto, 65_536);
    }

    // ── Escribir (0031 §11, W3.6c) ─────────────────────────────────────────
    //
    // `write("p.t", datos)` deja un dataset —una tabla Iceberg en el lago, el
    // `Dataset` escrito en el árbol, el puntero— desde lo que `over()`/`sql()` devolvieron
    // (`Rows`, con sus tipos), un `List<Map>` cualquiera, un `VectorSchemaRoot`
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
            case "uint64": throw new IllegalArgumentException("write(): column `" + columna + "` is uint64, which does not fit in int64 without lying (0032); convert it first");
            case "null": throw new IllegalArgumentException("write(): column `" + columna + "` has no type (all null): give it one first (0032)");
            default:
                Matcher m = Pattern.compile("^decimal128\\((\\d+), (\\d+)\\)$").matcher(t);
                if (m.matches()) return "decimal(" + m.group(1) + ", " + m.group(2) + ")";
                throw new IllegalArgumentException("write(): column `" + columna + "` is `" + t + "`, which the type contract (0032) does not have");
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
                if (!m.matches()) throw new IllegalArgumentException("write(): column `" + nombre + "` is `" + t + "`, which the type contract (0032) does not have");
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
        else throw new IllegalArgumentException("write(): column `" + v.getName() + "` (" + tipo + ") cannot be filled");
    }

    /** Lo que se escribe, como flujo Arrow IPC: {@code VectorSchemaRoot}, {@code ArrowReader}, {@code Rows} o {@code List<Map>}. */
    @SuppressWarnings("unchecked")
    private static byte[] ipcDe(Object datos, List<Map<String, Object>> esquema) throws Exception {
        duckdb();
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        if (datos instanceof VectorSchemaRoot raiz) {
            if (raiz.getRowCount() == 0) throw new IllegalArgumentException("write(): the table has no rows");
            try (ArrowStreamWriter w = new ArrowStreamWriter(raiz, null, out)) { w.start(); w.writeBatch(); w.end(); }
            for (Field f : raiz.getSchema().getFields()) esquema.add(campoIceberg(esquema.size() + 1, f.getName(), arrowName(f)));
            return out.toByteArray();
        }
        if (datos instanceof ArrowReader r) {
            boolean alguna = false;
            try (ArrowStreamWriter w = new ArrowStreamWriter(r.getVectorSchemaRoot(), null, out)) {
                w.start();
                while (r.loadNextBatch()) { if (r.getVectorSchemaRoot().getRowCount() > 0) { alguna = true; w.writeBatch(); } }
                w.end();
            }
            if (!alguna) throw new IllegalArgumentException("write(): the table has no rows");
            for (Field f : r.getVectorSchemaRoot().getSchema().getFields()) esquema.add(campoIceberg(esquema.size() + 1, f.getName(), arrowName(f)));
            return out.toByteArray();
        }
        if (!(datos instanceof List<?> lista)) throw new IllegalArgumentException("write() takes Rows, List<Map>, VectorSchemaRoot or ArrowReader, not " + (datos == null ? "null" : datos.getClass().getSimpleName()));
        if (lista.isEmpty()) throw new IllegalArgumentException("write(): the table has no rows");
        Map<String, String> tipos = new LinkedHashMap<>(datos instanceof Rows f ? f.types : Map.of());
        Set<String> nombres = new LinkedHashSet<>(tipos.keySet());
        for (Object o : lista) if (o instanceof Map<?, ?> m) for (Object k : m.keySet()) nombres.add(String.valueOf(k));
        for (String n : nombres) if (!tipos.containsKey(n)) {
            Object v = null;
            for (Object o : lista) { Object x = ((Map<String, Object>) o).get(n); if (x != null) { v = x; break; } }
            tipos.put(n, inferredType(v));
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
            if (tok.isEmpty()) throw new IllegalStateException("write(): the catalog vended no credential for `" + ubicacion + "`");
            env.put("ORE_GCS_TOKEN", tok);
        } else if (ubicacion.startsWith("s3://")) {
            nombre = "ore-store-r2";
            env.put("ORE_R2_BUCKET", ubicacion.substring(5).split("/")[0]);
            env.put("ORE_R2_S3_ENDPOINT", config.getOrDefault("s3.endpoint", ""));
            env.put("ORE_R2_ACCESS_KEY_ID", config.getOrDefault("s3.access-key-id", ""));
            env.put("ORE_R2_SECRET_ACCESS_KEY", config.getOrDefault("s3.secret-access-key", ""));
            env.put("ORE_R2_REGION", config.getOrDefault("s3.region", "auto"));
        } else {
            throw new IllegalStateException("write(): the table lives in `" + ubicacion + "`, which is not a lake this SDK can write");
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
        throw new IllegalStateException("write(): `" + nombre + "` is not on the PATH (the session image ships it; elsewhere, ORE_STORE_DIR)");
    }

    private static String mensajeDe(Respuesta r) {
        Object e = r.cuerpo().get("error");
        if (e instanceof Map<?, ?> m && m.get("message") != null) return String.valueOf(m.get("message"));
        return e == null ? r.cuerpo().toString() : String.valueOf(e);
    }

    /**
     * Declares an ontology document from the cell (0031 §9, W3.7 ①): the YAML as is →
     * {@code PUT /documentos/{kind}/{ns}/{n}} (Forge's gate: it compiles before pushing). Signed by
     * whoever opened the session, on their branch. Returns a {@link Result}
     * {@code {kind, name, file, commit, new}} (the old keys {@code nombre}, {@code fichero},
     * {@code nueva} still answer); a 422 is an {@link IllegalArgumentException} with the diagnostics.
     */
    public static Map<String, Object> declare(String yaml) throws Exception {
        java.util.regex.Matcher k = java.util.regex.Pattern.compile("^kind:\\s*([A-Za-z]+)\\s*$", java.util.regex.Pattern.MULTILINE).matcher(yaml);
        if (!k.find()) throw new IllegalArgumentException("declare(): the document has no `kind:`");
        java.util.regex.Matcher m = java.util.regex.Pattern.compile("^metadata:[ \\t]*(.*)$", java.util.regex.Pattern.MULTILINE).matcher(yaml);
        if (!m.find()) throw new IllegalArgumentException("declare(): the document has no `metadata:`");
        Map<String, String> campos = new LinkedHashMap<>();
        java.util.function.Consumer<String> par = (s) -> { int i = s.indexOf(':'); if (i > 0) campos.put(s.substring(0, i).trim(), s.substring(i + 1).trim().replaceAll("^[\"']|[\"']$", "")); };
        String resto = m.group(1).trim();
        if (resto.startsWith("{")) { for (String s : resto.replaceAll("^\\{|\\}$", "").split(",")) par.accept(s); }
        else { for (String l : yaml.substring(m.end()).split("\n")) { if (l.isEmpty()) continue; if (!Character.isWhitespace(l.charAt(0))) break; par.accept(l); } }
        if (campos.get("name") == null) throw new IllegalArgumentException("declare(): `metadata.name` is missing");
        Map<String, Object> cuerpo = new LinkedHashMap<>(); cuerpo.put("yaml", yaml);
        return declarar(k.group(1), campos.getOrDefault("namespace", ""), campos.getOrDefault("schema", "default"), campos.get("name"), cuerpo);
    }

    /** The same, with the document as a map {@code {kind, metadata, spec}}. */
    @SuppressWarnings("unchecked")
    public static Map<String, Object> declare(Map<String, Object> document) throws Exception {
        Map<String, Object> documento = document;
        Object kind = documento.get("kind");
        Map<String, Object> meta = documento.get("metadata") instanceof Map<?, ?> mm ? (Map<String, Object>) mm : Map.of();
        if (kind == null || meta.get("name") == null) throw new IllegalArgumentException("declare(): the document needs `kind` and `metadata.name`");
        return declarar(kind.toString(), String.valueOf(meta.getOrDefault("namespace", "")), String.valueOf(meta.getOrDefault("schema", "default")), meta.get("name").toString(), documento);
    }

    @SuppressWarnings("unchecked")
    private static Map<String, Object> declarar(String kind, String ns, String schema, String nombre, Map<String, Object> cuerpo) throws Exception {
        if (ns.isEmpty()) throw new IllegalArgumentException("declare(): `metadata.namespace` is missing: a document lives in a package");
        // 0038: en su schema, /documentos/{kind}/{base}/{schema}/{n}; la de dos tramos es `default`.
        String ruta = schema.isEmpty() || schema.equals("default")
                ? "/documentos/" + kind + "/" + ns + "/" + nombre
                : "/documentos/" + kind + "/" + ns + "/" + schema + "/" + nombre;
        Respuesta r = puesto.pedir("PUT", ruta, cuerpo, Duration.ofSeconds(120));
        Map<String, Object> c = r.cuerpo();
        if (r.codigo() == 200 || r.codigo() == 201) {
            Result out = new Result();
            out.put("kind", c.getOrDefault("kind", kind));
            String s = String.valueOf(c.getOrDefault("schema", schema));
            out.put("name", c.getOrDefault("namespace", ns) + "." + (s.isEmpty() || s.equals("default") ? "" : s + ".") + c.getOrDefault("name", nombre));
            out.put("file", c.getOrDefault("fichero", ""));
            out.put("commit", c.getOrDefault("commit", ""));
            out.put("created", Boolean.TRUE.equals(c.get("nueva")) || (c.get("nueva") == null && r.codigo() == 201));
            return out;
        }
        if (c.get("diagnosticos") instanceof List<?> ds && !ds.isEmpty()) {
            StringBuilder sb = new StringBuilder();
            for (Object d : ds) { if (d instanceof Map<?, ?> dm) { if (sb.length() > 0) sb.append("; "); sb.append(dm.get("codigo")).append(": ").append(dm.get("mensaje")); } }
            throw new IllegalArgumentException("declare(" + ns + "." + nombre + "): " + sb);
        }
        throw new RuntimeException("declare(" + ns + "." + nombre + "): " + c.getOrDefault("error", "?") + " (" + r.codigo() + ")");
    }

    /** {@code write(name, data, "overwrite")}. */
    public static Map<String, Object> write(String name, Object data) throws Exception { return write(name, data, "overwrite"); }

    /** Writes {@code data} as the lake dataset {@code <base>.<table>}; {@code mode} is {@code overwrite} or {@code append}. */
    public static Map<String, Object> write(String name, Object data, String mode) throws Exception { return write(name, data, mode, null); }

    /** {@code mode} (inglés o español) → el valor que viaja a {@code ore-store} y entra en la semilla de la
     *  operación: el de siempre, en español (cambiarlo cambiaría la clave de operación). */
    private static final Map<String, String> MODO_AL_CABLE = Map.of(
        "overwrite", "sobrescribir", "append", "anexar", "upsert", "upsert", "sobrescribir", "sobrescribir", "anexar", "anexar");

    /**
     * Writes {@code data} as the lake dataset {@code <base>.<table>}.
     *
     * @param name {@code <base>.<schema>.<name>} (or {@code <base>.<name>})
     * @param data {@link Rows}, a {@code List<Map>}, a {@code VectorSchemaRoot} or an {@code ArrowReader}
     * @param mode {@code overwrite}, {@code append} or {@code upsert} (the old {@code sobrescribir} and
     *             {@code anexar} still work)
     * @param key  with {@code upsert}: the columns that identify a row (copy-on-write; the key is
     *             declared on the table); {@code null} otherwise
     * @return a {@link Result} {@code {table, rows, snapshot, metadata_location, operation, repeated}}
     *         (the old keys {@code tabla}, {@code filas}, {@code operacion}, {@code repetida} still answer)
     */
    @SuppressWarnings("unchecked")
    public static Map<String, Object> write(String name, Object data, String mode, List<String> key) throws Exception {
        Object datos = data; List<String> clave = key;
        noAlCargar("write()");
        final String nombre = corto(name, "write(): the name");
        final String modo = mode == null ? null : MODO_AL_CABLE.get(mode);
        if (modo == null) throw new IllegalArgumentException("mode " + mode + ": it is `overwrite`, `append` or `upsert`");
        if (clave != null && !modo.equals("upsert")) throw new IllegalArgumentException("`key` goes with mode `upsert`");
        String[] p = partes(nombre);
        String bd = p[0], ns = p[1], t = p[2]; // el namespace de /v1 es el schema (0038 P4)
        List<Map<String, Object>> campos = new ArrayList<>();
        byte[] ipc = ipcDe(datos, campos);
        Map<String, Object> esquema = new LinkedHashMap<>();
        esquema.put("type", "struct"); esquema.put("schema-id", 0); esquema.put("fields", campos);
        String dataset = "catalogo/" + bd + "/" + ns + "/" + t; // una etiqueta: la ubicación la da el catálogo
        if (transformActivo != null && !nombre.equals(transformActivo.output())) throw new IllegalStateException("`" + nombre + "` is not the output of `" + transformActivo.nombre() + "` (" + transformActivo.output() + "): a transform only writes what it declares");
        // 0055 JT4: en un Preview, lo que escribiría; nada se escribe.
        if (ensayando != null) return ensayarEscritura(nombre, datos, mode);
        String semilla = nombre + "|" + modo + (clave != null && !clave.isEmpty() ? "|" + String.join(",", clave) : "");
        String claveOperacion = "";
        for (int intento = 0; intento < 4; intento++) {
            // 1 · la tabla, con la credencial prestada; o esbozada si no existe
            String base = null; Object esbozo = null; Map<String, String> config; String ubicacion;
            Respuesta r = puesto.pedir("GET", v1Tabla(nombre), null, Duration.ofSeconds(30), DELEGAR);
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
                Respuesta r2 = puesto.pedir("POST", "/v1/" + bd + "/namespaces/" + ns + "/tables", cuerpo, Duration.ofSeconds(30), DELEGAR);
                if (r2.codigo() != 200) throw new IllegalStateException("write(" + nombre + "): " + mensajeDe(r2));
                esbozo = r2.cuerpo().get("metadata");
                config = mapa(r2.cuerpo().get("config"));
                ubicacion = String.valueOf(((Map<String, Object>) esbozo).get("location"));
            } else {
                throw new IllegalStateException("write(" + nombre + "): ore-serve answered " + r.codigo() + ": " + mensajeDe(r));
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
            if (proc.waitFor() != 0) throw new IllegalStateException("write(): " + (err.isEmpty() ? "the writer failed" : err.replaceFirst("^error: ", "")));
            Map<String, Object> escrito = Json.objeto(new String(salida, StandardCharsets.UTF_8));
            claveOperacion = String.valueOf(escrito.getOrDefault("operacion", claveOperacion));
            // 3 · el commit, por el catálogo
            Map<String, Object> commit = new LinkedHashMap<>();
            commit.put("identifier", Map.of("namespace", List.of(ns), "name", t));
            commit.put("requirements", escrito.get("requirements")); commit.put("updates", escrito.get("updates"));
            Respuesta c = puesto.pedir("POST", v1Tabla(nombre), commit, Duration.ofSeconds(120));
            if (c.codigo() == 200) {
                Map<String, Object> md = (Map<String, Object>) c.cuerpo().get("metadata");
                Result out = new Result();
                out.put("table", nombre); out.put("rows", escrito.get("filas")); out.put("snapshot", String.valueOf(md == null ? "" : md.get("current-snapshot-id")));
                out.put("metadata_location", String.valueOf(c.cuerpo().getOrDefault("metadata-location", ""))); out.put("operation", claveOperacion);
                // repetida: el catálogo contestó con lo que ya había (el mismo puntero)
                out.put("repeated", base != null && base.equals(String.valueOf(c.cuerpo().get("metadata-location"))));
                return alInforme(out);
            }
            if (c.codigo() == 409) continue; // alguien escribió mientras tanto: otra vez sobre lo que hay
            if (c.codigo() >= 500) {
                // el commit pudo entrar: se MIRA antes de darlo por perdido
                Respuesta v = puesto.pedir("GET", v1Tabla(nombre), null, Duration.ofSeconds(30));
                if (v.codigo() == 200) {
                    Map<String, Object> md = (Map<String, Object>) v.cuerpo().get("metadata");
                    Object actual = md.get("current-snapshot-id");
                    for (Object sn : (List<Object>) md.getOrDefault("snapshots", List.of())) {
                        Map<String, Object> m = (Map<String, Object>) sn;
                        if (String.valueOf(m.get("snapshot-id")).equals(String.valueOf(actual)) && m.get("summary") instanceof Map<?, ?> su && claveOperacion.equals(String.valueOf(su.get("ore.operacion")))) {
                            Result out = new Result();
                            out.put("table", nombre); out.put("rows", escrito.get("filas")); out.put("snapshot", String.valueOf(actual));
                            out.put("metadata_location", String.valueOf(v.cuerpo().get("metadata-location"))); out.put("operation", claveOperacion); out.put("repeated", false);
                            return alInforme(out);
                        }
                    }
                }
                throw new IllegalStateException("write(" + nombre + "): the catalog answered " + c.codigo() + " and the commit is not there: " + mensajeDe(c));
            }
            throw new IllegalStateException("write(" + nombre + "): " + mensajeDe(c));
        }
        throw new IllegalStateException("write(" + nombre + "): someone else wrote first four times; try again");
    }

    /** 0049 JM3 · en un build cuya salida es una colección escrita: lo confirmado va al informe (los ítems, la transacción). */
    @SuppressWarnings("unchecked")
    static void alInformeDeMedia(String coleccion, Map<String, Object> confirmado) {
        if (BUILD == null || !coleccion.equals(BUILD.get("output"))) return;
        Map<String, Object> m = new LinkedHashMap<>();
        Object items = confirmado.get("items") instanceof Map<?, ?> im ? ((Map<String, Object>) im).get("actuales") : null;
        m.put("filas", items instanceof Number n ? n.longValue() : 0L);
        m.put("snapshot", null);
        m.put("transaccion", confirmado.get("transaccion"));
        paraElInforme(m);
    }

    /** 0055 B2 · en un build, las filas y el snapshot de lo escrito van al informe. */
    private static Result alInforme(Result out) {
        if (BUILD != null) {
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("filas", out.get("rows") instanceof Number n ? n.longValue() : 0L);
            String sn = String.valueOf(out.get("snapshot"));
            m.put("snapshot", sn.isEmpty() || sn.equals("null") ? null : sn);
            paraElInforme(m);
        }
        return out;
    }

    // ── J3 (las salidas de una celda, S3) · display() ───────────────────────
    /** Adonde va lo que {@link #display} enseña mientras corre una celda de una sesión (lo pone el agente); fuera, {@code null}. */
    static volatile java.util.function.Consumer<Object> mostrar;

    /**
     * <b>Show each value now</b>, with its own output —a table, a tree, an
     * image—, in order with what the cell prints; the last expression of the
     * cell is shown too. Outside a session's cell (a job, your machine) it
     * prints them. An {@code Object[]} is spread into its values (varargs).
     *
     * @param values what to show
     */
    public static void display(Object... values) {
        for (Object v : values) {
            java.util.function.Consumer<Object> m = mostrar;
            if (m != null) m.accept(v);
            else System.out.println(v);
        }
    }

    // ── El contrato de tipos (0032 §1) ─────────────────────────────────────

    /** The value at row {@code i} of a vector, in the contract's Java type. */
    public static Object valueAt(FieldVector v, int i) {
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
            for (FieldVector c : st.getChildrenFromFields()) m.put(c.getName(), valueAt(c, i));
            return m;
        }
        // Un map de Arrow ES una lista de struct(key, value): sale como List<Map>.
        if (v instanceof ListVector l) {
            FieldVector datos = l.getDataVector();
            List<Object> out = new ArrayList<>();
            for (int j = l.getElementStartIndex(i); j < l.getElementEndIndex(i); j++) out.add(valueAt(datos, j));
            return out;
        }
        Object o = v.getObject(i);
        return o instanceof org.apache.arrow.vector.util.Text t ? t.toString() : o;
    }

    /** @deprecated use {@link #valueAt(FieldVector, int)}. */
    @Deprecated
    public static Object valorDe(FieldVector v, int i) { alias("valorDe()", "valueAt()"); return valueAt(v, i); }

    private static Instant instante(long n, long porSegundo) {
        return Instant.ofEpochSecond(Math.floorDiv(n, porSegundo), Math.floorMod(n, porSegundo) * (1_000_000_000L / porSegundo));
    }

    /** A field's Arrow type, with the name {@code pyarrow} gives it. */
    public static String arrowName(Field f) {
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
        if (t instanceof ArrowType.List) return "list<item: " + arrowName(f.getChildren().get(0)) + ">";
        if (t instanceof ArrowType.Struct) { StringBuilder b = new StringBuilder("struct<"); for (int i = 0; i < f.getChildren().size(); i++) { Field c = f.getChildren().get(i); if (i > 0) b.append(", "); b.append(c.getName()).append(": ").append(arrowName(c)); } return b.append(">").toString(); }
        if (t instanceof ArrowType.Map) { Field par = f.getChildren().get(0); return "map<" + arrowName(par.getChildren().get(0)) + ", " + arrowName(par.getChildren().get(1)) + ">"; }
        if (t instanceof ArrowType.Null) return "null";
        return t.toString().toLowerCase(java.util.Locale.ROOT);
    }

    /** @deprecated use {@link #arrowName(Field)}. */
    @Deprecated
    public static String nombreArrow(Field f) { alias("nombreArrow()", "arrowName()"); return arrowName(f); }

    private static String unidad(org.apache.arrow.vector.types.TimeUnit u) {
        return switch (u) { case SECOND -> "s"; case MILLISECOND -> "ms"; case MICROSECOND -> "us"; case NANOSECOND -> "ns"; };
    }

    private static final long ENTERO_EXACTO = 1L << 53;

    /**
     * A contract value → the console's JSON (0032 §1): integer → number if
     * |x| ≤ 2⁵³, else string · decimal → always a string (except scale 0,
     * which is an integer and goes as one) · float → number, with
     * NaN/Infinity/-Infinity as strings · date {@code YYYY-MM-DD} · time
     * {@code HH:MM:SS[.ffffff]} · zone-less datetime in ISO with {@code T} ·
     * UTC instant with {@code Z} · bytes in base64 · list → array · struct →
     * object · map → {@code [{key, value}]}. Nothing is silently degraded. The
     * SAME JSON the Python and Node agents emit.
     */
    public static Object toJson(Object v) {
        if (v == null || v instanceof String || v instanceof Boolean) return v;
        if (v instanceof Integer || v instanceof Short || v instanceof Byte) return v;
        if (v instanceof Long n) return n >= -ENTERO_EXACTO && n <= ENTERO_EXACTO ? (Object) n : (Object) n.toString();
        if (v instanceof java.math.BigInteger n) return n.bitLength() < 54 ? (Object) n.longValue() : (Object) n.toString();
        // Un decimal de escala 0 (un HUGEINT: `sum(1)`, `count`) es un entero y va como los enteros; con decimales, cadena siempre.
        if (v instanceof java.math.BigDecimal d) return d.scale() == 0 ? toJson(d.toBigInteger()) : d.toPlainString();
        if (v instanceof Double d) return d.isNaN() ? "NaN" : d.isInfinite() ? (d > 0 ? "Infinity" : "-Infinity") : (Object) d;
        if (v instanceof Float f) return f.isNaN() ? "NaN" : f.isInfinite() ? (f > 0 ? "Infinity" : "-Infinity") : (Object) f.doubleValue();
        if (v instanceof LocalDate d) return d.toString();
        if (v instanceof LocalTime t) return hora(t);
        if (v instanceof LocalDateTime t) return t.toLocalDate() + "T" + hora(t.toLocalTime());
        if (v instanceof Instant t) { LocalDateTime l = LocalDateTime.ofInstant(t, ZoneOffset.UTC); return l.toLocalDate() + "T" + hora(l.toLocalTime()) + "Z"; }
        if (v instanceof byte[] b) return Base64.getEncoder().encodeToString(b);
        if (v instanceof Map<?, ?> m) { Map<String, Object> out = new LinkedHashMap<>(); for (Map.Entry<?, ?> e : m.entrySet()) out.put(String.valueOf(e.getKey()), toJson(e.getValue())); return out; }
        if (v instanceof Iterable<?> it) { List<Object> out = new ArrayList<>(); for (Object x : it) out.add(toJson(x)); return out; }
        if (v instanceof Object[] a) return toJson(java.util.Arrays.asList(a));
        // Lo que JDBC daría si alguien lo usa a mano: se acerca al contrato.
        if (v instanceof java.sql.Timestamp t) return toJson(t.toLocalDateTime());
        if (v instanceof java.sql.Date d) return toJson(d.toLocalDate());
        if (v instanceof java.sql.Time t) return toJson(t.toLocalTime());
        if (v instanceof java.util.Date d) return toJson(d.toInstant());
        return String.valueOf(v);
    }

    /** @deprecated use {@link #toJson(Object)}. */
    @Deprecated
    public static Object jsonDe(Object v) { alias("jsonDe()", "toJson()"); return toJson(v); }

    /** {@code HH:MM:SS}, y la fracción sólo si no es cero, sin ceros de más. */
    private static String hora(LocalTime t) {
        String s = String.format("%02d:%02d:%02d", t.getHour(), t.getMinute(), t.getSecond());
        if (t.getNano() == 0) return s;
        String f = String.format("%09d", t.getNano()).replaceAll("0+$", "");
        return s + "." + f;
    }

    /**
     * What {@code over()} and {@code sql()} return ({@link Rows}), or any list of
     * maps → the console's {@code tabla} output of the contract (0032 §1, «console
     * JSON»): the SAME JSON the Python and Node agents emit. Without declared
     * types they are inferred from the first value.
     *
     * <p>⚠ What it returns IS the wire format the agent posts to ore-serve
     * ({@code {columnas, filas, total, limite}}), so its keys stay as they are.
     *
     * @param value what to show
     * @param limit how many rows at most
     */
    @SuppressWarnings("unchecked")
    public static Map<String, Object> table(Object value, int limit) {
        Object valor = value; int limite = limit;
        if (!(valor instanceof List<?> lista) || lista.isEmpty()) return null;
        for (Object f : lista) if (!(f instanceof Map)) return null;
        Map<String, String> tipos = valor instanceof Rows fs ? fs.types : null;
        List<String> columnas = new ArrayList<>(tipos != null ? tipos.keySet() : ((Map<String, Object>) lista.get(0)).keySet());
        List<Map<String, Object>> cols = new ArrayList<>();
        for (String c : columnas) {
            String tipo = tipos != null ? tipos.get(c) : null;
            if (tipo == null) {
                tipo = "null";
                for (Object f : lista) {
                    Object v = ((Map<String, Object>) f).get(c);
                    if (v != null) { tipo = inferredType(v); break; }
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
            for (String c : columnas) fila.add(toJson(((Map<String, Object>) f).get(c)));
            filas.add(fila);
        }
        Map<String, Object> m = new LinkedHashMap<>();
        m.put("columnas", cols);
        m.put("filas", filas);
        m.put("total", valor instanceof Rows fs && fs.total != null ? fs.total : (Object) lista.size());
        m.put("limite", limite);
        return m;
    }

    /** @deprecated use {@link #table(Object, int)}. */
    @Deprecated
    public static Map<String, Object> tabla(Object valor, int limite) { alias("tabla()", "table()"); return table(valor, limite); }

    /** The Arrow type of a loose value, for what does not come from {@code over()}/{@code sql()}. */
    public static String inferredType(Object v) {
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

    /** @deprecated use {@link #inferredType(Object)}. */
    @Deprecated
    public static String tipoInferido(Object v) { alias("tipoInferido()", "inferredType()"); return inferredType(v); }

    /** A loose value → JSON (the same as {@link #toJson(Object)}). */
    public static Object plain(Object v) { return toJson(v); }

    /** @deprecated use {@link #plain(Object)} (or {@link #toJson(Object)}). */
    @Deprecated
    public static Object llano(Object v) { alias("llano()", "plain()"); return plain(v); }
}
