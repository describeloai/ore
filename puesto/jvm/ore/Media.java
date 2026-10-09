package ore;

import java.io.IOException;
import java.io.InputStream;
import java.net.URI;
import java.net.URLEncoder;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.nio.ByteBuffer;
import java.nio.channels.Channels;
import java.nio.channels.ClosedChannelException;
import java.nio.channels.NonWritableChannelException;
import java.nio.channels.SeekableByteChannel;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.time.Duration;
import java.time.Instant;
import java.util.ArrayList;
import java.util.HexFormat;
import java.util.Iterator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.NoSuchElementException;
import java.util.concurrent.CompletionService;
import java.util.concurrent.ExecutionException;
import java.util.concurrent.ExecutorCompletionService;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;

/**
 * Media in code, from Java (ADR 0049 JM; the contract, {@code docs/media.md}).
 *
 * <pre>{@code
 * var c = Ore.collection("legal.archive.contracts");
 * for (var item : c.items("New folder/")) {            // the listing, by cursor
 *     try (var ch = item.open()) {                      // pinned to its version
 *         ch.read(ByteBuffer.allocate(5));              // "%PDF-"
 *         ch.position(ch.size() - 1024);                // the tail, with a range
 *     }
 * }
 * byte[] data = c.stat("docs/a.pdf").readBytes();       // whole, verified
 * for (var r : Ore.readMany(c.items(), 16)) { ... }     // many at once; an error is a value
 * }</pre>
 *
 * The code never talks to the store or the origin: it asks the cell where the
 * bytes are, and reads that URL without ORE's token. Errors are
 * {@link MediaError} subclasses, the same as in Python.
 *
 * <p>La media en código, desde la JVM: la misma {@code MediaRef}, el mismo
 * contrato y los mismos errores que el SDK de Python ({@code ore/medios.py}),
 * del que esto es la traducción. Lo que difiere, y por qué, está en el ADR
 * 0049 (JM): un flujo cortado se REANUDA desde donde iba, fijado a su versión
 * (D-JM3) —si esa versión ya no se puede leer, {@link MediaChanged}; nunca
 * bytes de otra—.
 */
public final class Media {
    private Media() {}

    /** Lo que se pide de una vez cuando se baja un ítem grande por rangos (no final: las pruebas lo bajan). */
    static long TROZO = 8L << 20;
    /** A partir de cuánto {@code readBytes()} baja por rangos en paralelo. */
    static long EN_PARALELO_DESDE = 32L << 20;
    /** Cuántas veces se pide otro permiso seguido antes de rendirse. */
    static final int RENOVACIONES = 3;
    /** Cuántas veces se reanuda un flujo cortado (D-JM3). */
    static final int REANUDACIONES = 3;
    /**
     * Cuántas veces se repite un GET que no llegó a tener respuesta. El cliente de la JVM
     * guarda las conexiones y puede reusar una que el otro lado ya cerró ("header parser
     * received no bytes"): medido en el banco con 32 hilos, 37 de 500. Un GET se puede repetir.
     */
    static final int REPETICIONES = 2;
    /** Lo que el listado pide por página si no se dice. */
    public static final int LIMIT = 1000;

    /** Los bytes: HTTP/1.1, sin seguir redirecciones, y SIN las cabeceras de ORE. */
    static final HttpClient BYTES = HttpClient.newBuilder().version(HttpClient.Version.HTTP_1_1)
        .followRedirects(HttpClient.Redirect.NEVER).connectTimeout(Duration.ofSeconds(10)).build();

    // ── los errores del contrato (`docs/media.md` §3) ───────────────────────

    /** A media contract error: {@code type} ({@code media/…}, the wire code), {@code status} and {@code detail}. */
    public static class MediaError extends RuntimeException {
        public final String type;
        public final int status;
        public final String detail;

        public MediaError(String type, int status, String detail) {
            super(type + " (" + status + "): " + detail);
            this.type = type;
            this.status = status;
            this.detail = detail;
        }
    }

    /** The collection or the item does not exist ({@code media/no-existe}). */
    public static class MediaNotFound extends MediaError {
        public MediaNotFound(String type, int status, String detail) { super(type, status, detail); }
    }

    /** Not allowed to read it, or the collection is not declared ({@code media/sin-permiso}, {@code media/no-declarada}). */
    public static class MediaForbidden extends MediaError {
        public MediaForbidden(String type, int status, String detail) { super(type, status, detail); }
    }

    /** The pinned version can no longer be read whole: bytes of another version are never returned ({@code media/cambiado}). */
    public static class MediaChanged extends MediaError {
        public MediaChanged(String type, int status, String detail) { super(type, status, detail); }
    }

    /** The bytes do not match {@code size} or {@code digest}, or the stream was cut ({@code media/corrupto}). */
    public static class MediaCorrupt extends MediaError {
        public MediaCorrupt(String type, int status, String detail) { super(type, status, detail); }
    }

    /** A range that cannot be served ({@code media/rango}). */
    public static class MediaRangeError extends MediaError {
        public MediaRangeError(String type, int status, String detail) { super(type, status, detail); }
    }

    /** The collection is not a written one: its origin fills it, not code ({@code media/no-escribible}). */
    public static class MediaNotWritable extends MediaError {
        public MediaNotWritable(String type, int status, String detail) { super(type, status, detail); }
    }

    /** The transaction is not open: it expired, was closed, or belongs to another collection ({@code media/transaccion}). */
    public static class MediaTransactionError extends MediaError {
        public MediaTransactionError(String type, int status, String detail) { super(type, status, detail); }
    }

    static MediaError error(int status, Map<String, Object> cuerpo, String que) {
        Map<String, Object> c = cuerpo == null ? Map.of() : cuerpo;
        String tipo = texto(c.get("type"));
        if (tipo == null) tipo = switch (status) {
            case 404 -> "media/no-existe";
            case 403 -> "media/sin-permiso";
            case 412 -> "media/cambiado";
            case 416 -> "media/rango";
            default -> "media/origen";
        };
        String detalle = texto(c.get("detail"));
        if (detalle == null) detalle = texto(c.get("error"));
        if (detalle == null) detalle = que + " answered " + status;
        return switch (tipo) {
            case "media/no-existe" -> new MediaNotFound(tipo, status, detalle);
            case "media/sin-permiso", "media/no-declarada", "media/permiso" -> new MediaForbidden(tipo, status, detalle);
            case "media/cambiado" -> new MediaChanged(tipo, status, detalle);
            case "media/corrupto", "media/digest-no-casa" -> new MediaCorrupt(tipo, status, detalle);
            case "media/rango", "media/sin-rangos" -> new MediaRangeError(tipo, status, detalle);
            case "media/no-escribible" -> new MediaNotWritable(tipo, status, detalle);
            case "media/transaccion" -> new MediaTransactionError(tipo, status, detalle);
            default -> new MediaError(tipo, status, detalle);
        };
    }

    static String texto(Object o) { return o == null ? null : String.valueOf(o); }

    // ── la referencia ───────────────────────────────────────────────────────

    /**
     * The value of a {@code Media<c>} (OOS v1alpha17 {@code 01} §3): where an
     * item is and what is known about it, without its bytes. Immutable;
     * unknown fields are ignored.
     */
    public record MediaRef(String uri, String collection, String path, String version, String digest, Long size,
                           String contentType, String contentTypeDetected, String checksum,
                           Map<String, Object> annotations, String modified, String state, String transaction,
                           Map<String, Object> source, Map<String, Object> derivation) {

        /** A {@code MediaRef} from its JSON; unknown fields are ignored. */
        @SuppressWarnings("unchecked")
        public static MediaRef fromJson(Map<String, Object> d) {
            Map<String, Object> m = d == null ? Map.of() : d;
            Object t = m.get("size");
            Long size = t == null ? null : t instanceof Number n ? n.longValue() : Long.parseLong(String.valueOf(t));
            return new MediaRef(texto(m.get("uri")), texto(m.get("collection")), texto(m.get("path")),
                texto(m.get("version")), texto(m.get("digest")), size, texto(m.get("content_type")),
                texto(m.get("content_type_detected")), texto(m.get("checksum")),
                m.get("annotations") instanceof Map<?, ?> a ? (Map<String, Object>) a : null, texto(m.get("modified")),
                texto(m.get("state")), texto(m.get("transaction")),
                m.get("source") instanceof Map<?, ?> s ? (Map<String, Object>) s : null,
                m.get("derivation") instanceof Map<?, ?> v ? (Map<String, Object>) v : null);
        }

        /** Its JSON, with the contract's names; what is not known is left out. */
        public Map<String, Object> toJson() {
            Map<String, Object> m = new LinkedHashMap<>();
            Object[] kv = {"uri", uri, "collection", collection, "path", path, "version", version, "digest", digest,
                "size", size, "content_type", contentType, "content_type_detected", contentTypeDetected,
                "checksum", checksum, "annotations", annotations, "modified", modified, "state", state,
                "transaction", transaction, "source", source, "derivation", derivation};
            for (int i = 0; i < kv.length; i += 2) if (kv[i + 1] != null) m.put((String) kv[i], kv[i + 1]);
            return m;
        }

        MediaRef conVersion(String v) {
            return new MediaRef(uri, collection, path, v, digest, size, contentType, contentTypeDetected, checksum,
                annotations, modified, state, transaction, source, derivation);
        }

        MediaRef conTamano(long s) {
            return new MediaRef(uri, collection, path, version, digest, s, contentType, contentTypeDetected, checksum,
                annotations, modified, state, transaction, source, derivation);
        }
    }

    // ── la colección ────────────────────────────────────────────────────────

    /** One page of a listing: the transaction it read ({@code asOf}), its items, and the cursor to the next. */
    public record Page(String asOf, List<Item> items, String cursor) {}

    /** A media collection: {@code items()}, {@code stat()} and {@code urls()}. */
    public static final class Collection {
        /** The name in its short form ({@code base.name} in {@code default}): the one a transform declares. */
        public final String shortName;
        final String base, schema, nombre, ruta;
        private volatile String asOf;
        private Map<String, String> rama;

        Collection(String name) {
            shortName = Ore.corto(name, "collection(): the name");
            String[] p = Ore.partes(shortName);
            base = p[0];
            schema = p[1];
            nombre = p[2];
            ruta = "/media/" + base + "/" + schema + "/" + nombre;
        }

        /** The transaction the last listing read (B4·3): inside a transform, the one pinned when it was declared. */
        public String asOf() { return asOf; }

        @Override public String toString() { return "Collection(" + base + "." + schema + "." + nombre + ")"; }

        synchronized Map<String, String> rama() throws IOException, InterruptedException {
            if (rama == null) rama = Ore.ramaDelPuesto();
            return rama;
        }

        Ore.Respuesta pedir(String metodo, String op, Map<String, Object> consulta, Object cuerpo) {
            // B4·3: leer una colección es leer, como `over()` y `sql()`: dentro de
            // un transform, sólo sus inputs (y sin preguntar), y fuera queda
            // anotada en lo que la sesión leyó.
            lee(shortName);
            StringBuilder q = new StringBuilder();
            for (Map.Entry<String, Object> e : consulta.entrySet()) {
                if (e.getValue() == null) continue;
                q.append(q.length() == 0 ? '?' : '&').append(URLEncoder.encode(e.getKey(), StandardCharsets.UTF_8))
                    .append('=').append(URLEncoder.encode(String.valueOf(e.getValue()), StandardCharsets.UTF_8));
            }
            try {
                for (int vez = 0; ; vez++) {
                    try {
                        return Ore.puesto.pedir(metodo, ruta + "/" + op + q, cuerpo, Duration.ofSeconds(90), rama());
                    } catch (IOException e) {
                        if (!metodo.equals("GET") || vez >= REPETICIONES)
                            throw new MediaError("media/origen", 502, op + "(" + this + "): " + e);
                    }
                }
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                throw new MediaError("media/origen", 499, op + "(" + this + "): interrupted");
            }
        }

        /** Escribir no es leer: sin {@code lee()}, con la rama del puesto. */
        Ore.Respuesta escribir(String metodo, String op, Object cuerpo, String que) {
            try {
                return Ore.puesto.pedir(metodo, ruta + op, cuerpo, Duration.ofSeconds(300), rama());
            } catch (IOException e) {
                throw new MediaError("media/origen", 502, que + ": " + e);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                throw new MediaError("media/origen", 499, que + ": interrupted");
            }
        }

        /** <b>A transaction to write into this collection</b> (B4b·3), for an hour. Inside a transform, only on its {@code output}. */
        public Transaction transaction() { return transaction(3600); }

        /** The same, for {@code ttlS} seconds. */
        public Transaction transaction(int ttlS) {
            Ore.Transform t = Ore.transformActivo();
            if (t != null && !shortName.equals(t.output()))
                throw new MediaForbidden("media/no-declarada", 403, "`" + shortName + "` is not the output of `" + t.nombre()
                    + "` (" + t.output() + "): a transform only writes what it declares");
            return new Transaction(this, ttlS);
        }

        /**
         * {@code verify}: reads the bytes of each item and checks them against its digest, in its
         * position (one's error does not stop the others). To audit, not for the hot path.
         * {@code items}: paths, {@code MediaRef}s or {@code Item}s.
         */
        public List<Verified> verify(List<?> items) {
            List<Verified> salida = new ArrayList<>();
            for (Object o : items) {
                Item it;
                try {
                    it = o instanceof Item i ? i : stat(o instanceof MediaRef m ? m.path() : String.valueOf(o),
                        o instanceof MediaRef m ? m.version() : null);
                    byte[] b = it.readBytes(1);
                    salida.add(new Verified(it.ref(), true, sha256(b), null));
                } catch (MediaError e) {
                    salida.add(new Verified(o instanceof MediaRef m ? m : o instanceof Item i ? i.ref() : null, false, null, e));
                }
            }
            return salida;
        }

        /** The listing of the current transaction, lazy, by cursor: {@code Item}s without bytes. */
        public Iterable<Item> items() { return items(null, null, LIMIT, null); }

        /** The items whose path starts with {@code prefix}. */
        public Iterable<Item> items(String prefix) { return items(prefix, null, LIMIT, null); }

        /**
         * The listing, lazy, by cursor. {@code prefix} filters by path, {@code state} by item state
         * ({@code actual}, {@code retirado}, {@code perdido}, {@code todos}), {@code limit} is the page
         * size, {@code asOf} a past transaction of the collection (or {@code null}: the current one).
         */
        public Iterable<Item> items(String prefix, String state, int limit, String asOf) {
            Iterable<Page> paginas = pages(prefix, state, limit, asOf);
            return () -> new Iterator<>() {
                final Iterator<Page> ps = paginas.iterator();
                Iterator<Item> actual = List.<Item>of().iterator();

                @Override public boolean hasNext() {
                    while (!actual.hasNext() && ps.hasNext()) actual = ps.next().items().iterator();
                    return actual.hasNext();
                }

                @Override public Item next() {
                    if (!hasNext()) throw new NoSuchElementException();
                    return actual.next();
                }
            };
        }

        /** The listing page by page, each with the transaction it read: what {@code items()} walks. */
        @SuppressWarnings("unchecked")
        public Iterable<Page> pages(String prefix, String state, int limit, String asOf) {
            Collection col = this;
            return () -> new Iterator<>() {
                String cursor = null;
                boolean fin = false;

                @Override public boolean hasNext() { return !fin; }

                @Override public Page next() {
                    if (fin) throw new NoSuchElementException();
                    Map<String, Object> q = new LinkedHashMap<>();
                    q.put("prefix", prefix);
                    q.put("estado", state);
                    q.put("limit", limit);
                    q.put("as_of", cursor == null ? asOf : null);
                    q.put("cursor", cursor);
                    Ore.Respuesta r = pedir("GET", "items", q, null);
                    if (r.codigo() != 200) throw error(r.codigo(), r.cuerpo(), "items(" + col + ")");
                    String visto = texto(r.cuerpo().get("as_of"));
                    if (visto != null) col.asOf = visto;
                    List<Item> its = new ArrayList<>();
                    if (r.cuerpo().get("items") instanceof List<?> l)
                        for (Object o : l) its.add(new Item(col, MediaRef.fromJson((Map<String, Object>) o)));
                    cursor = texto(r.cuerpo().get("cursor"));
                    if (cursor == null || cursor.isEmpty()) fin = true;
                    return new Page(visto, its, cursor);
                }
            };
        }

        /** One item, fresh, by its path: its {@code MediaRef} and whether it is the current version ({@link Item#current()}). */
        public Item stat(String path) { return stat(path, null); }

        /** One version of an item, by its path. */
        public Item stat(String path, String version) {
            Map<String, Object> q = new LinkedHashMap<>();
            q.put("path", path);
            q.put("version", version);
            return statDe(q, path);
        }

        /** One item by its content ({@code sha256:…}). */
        public Item statByDigest(String digest) {
            Map<String, Object> q = new LinkedHashMap<>();
            q.put("digest", digest);
            return statDe(q, digest);
        }

        @SuppressWarnings("unchecked")
        private Item statDe(Map<String, Object> q, String que) {
            Ore.Respuesta r = pedir("GET", "item", q, null);
            if (r.codigo() != 200) throw error(r.codigo(), r.cuerpo(), "stat(" + que + ")");
            Item it = new Item(this, MediaRef.fromJson(r.cuerpo()));
            it.current = r.cuerpo().get("current") instanceof Boolean b ? b : null;
            return it;
        }

        /**
         * A URL for each item, for whoever needs plain HTTP (a browser, another tool), in its
         * position; one's error goes in its own and does not stop the others. {@code items}: paths,
         * {@code MediaRef}s or {@code Item}s. {@code ttlS}: {@code null} for the default (300 s); the
         * cell clamps it.
         */
        @SuppressWarnings("unchecked")
        public List<Url> urls(List<?> items, Integer ttlS) {
            List<Map<String, Object>> pedidos = new ArrayList<>();
            for (Object o : items) {
                MediaRef ref = o instanceof Item it ? it.ref() : o instanceof MediaRef m ? m : null;
                Map<String, Object> p = new LinkedHashMap<>();
                if (ref != null) {
                    p.put("path", ref.path());
                    if (ref.version() != null) p.put("version", ref.version());
                } else p.put("path", String.valueOf(o));
                pedidos.add(p);
            }
            Map<String, Object> cuerpo = new LinkedHashMap<>();
            cuerpo.put("items", pedidos);
            if (ttlS != null) cuerpo.put("ttl_s", ttlS);
            Ore.Respuesta r = pedir("POST", "urls", Map.of(), cuerpo);
            if (r.codigo() != 200) throw error(r.codigo(), r.cuerpo(), "urls(" + this + ")");
            List<Url> salida = new ArrayList<>();
            if (r.cuerpo().get("urls") instanceof List<?> l) {
                for (Object o : l) {
                    Map<String, Object> u = o instanceof Map<?, ?> m ? (Map<String, Object>) m : Map.of();
                    if (u.get("error") instanceof Map<?, ?> e) {
                        Map<String, Object> pe = (Map<String, Object>) e;
                        int st = pe.get("status") instanceof Number n ? n.intValue() : 502;
                        salida.add(new Url(null, null, null, null, error(st, pe, "urls")));
                    } else {
                        Object ms = u.get("expires_ms");
                        salida.add(new Url(u.get("item") instanceof Map<?, ?> i ? MediaRef.fromJson((Map<String, Object>) i) : null,
                            texto(u.get("url")), ms instanceof Number n ? Instant.ofEpochMilli(n.longValue()) : null,
                            u.get("ttl_s") instanceof Number t ? t.intValue() : null, null));
                    }
                }
            }
            return salida;
        }

        /** {@code content}: a dónde ir por los bytes de {@code ref}, fijado a su versión. */
        Map<String, Object> donde(MediaRef ref) {
            Map<String, Object> q = new LinkedHashMap<>();
            if (ref.path() != null) {
                q.put("path", ref.path());
                q.put("version", ref.version());
            } else q.put("digest", ref.digest());
            Ore.Respuesta r = pedir("GET", "content", q, null);
            if ((r.codigo() != 200 && r.codigo() != 307) || r.cuerpo().get("url") == null)
                throw error(r.codigo(), r.cuerpo(), "open(" + ref.path() + ")");
            return r.cuerpo();
        }
    }

    /** A URL from {@link Collection#urls}: the item, the URL and when it expires; or its {@code error}. */
    public record Url(MediaRef item, String url, Instant expiresAt, Integer ttlS, MediaError error) {
        public boolean ok() { return error == null; }
    }

    /** Lo que {@code over()} y {@code sql()} hacen al leer, con el error del contrato. */
    static void lee(String corto) {
        Ore.Transform t = Ore.transformActivo();
        if (t != null && !t.inputs().contains(corto))
            throw new MediaForbidden("media/no-declarada", 403, "`" + corto + "` is not in the inputs of `" + t.nombre()
                + "` (" + String.join(", ", t.inputs()) + "): a transform only reads what it declares");
        Ore.lee(corto);
    }

    // ── el ítem ─────────────────────────────────────────────────────────────

    /** An item of a collection: its {@code ref()} and how to read its bytes ({@code open}, {@code readBytes}, {@code readRange}). */
    public static final class Item {
        public final Collection collection;
        volatile MediaRef ref;
        volatile Boolean current;
        volatile String sha256Seen;

        Item(Collection collection, MediaRef ref) {
            this.collection = collection;
            this.ref = ref;
        }

        /** Its reference: as listed, and pinned to a version once opened. */
        public MediaRef ref() { return ref; }

        /** From {@code stat()}: whether this is the current version ({@code null} if not asked). */
        public Boolean current() { return current; }

        /** The sha256 a whole read computed, if the item did not bring its digest. */
        public String sha256Seen() { return sha256Seen; }

        @Override public String toString() { return "Item(" + ref.path() + "@" + ref.version() + ")"; }

        /** A read-only channel pinned to the item's version: {@code read}, {@code position}, {@code size}. Close it: closing halfway does not download the rest. */
        public MediaChannel open() { return new MediaChannel(this); }

        /** The same as a stream ({@code try (var in = item.inputStream()) { … }}). */
        public InputStream inputStream() { return Channels.newInputStream(open()); }

        /** All the bytes, verified. A large item is read by ranges, eight at once. */
        public byte[] readBytes() { return readBytes(8); }

        /** All the bytes, verified; a large item by ranges, {@code threads} at once. */
        public byte[] readBytes(int threads) {
            Long size = ref.size();
            if (size != null && size > Integer.MAX_VALUE - 8)
                throw new MediaRangeError("media/rango", 413, ref.path() + " has " + size + " bytes: read it with open()");
            if (size == null || size < EN_PARALELO_DESDE || threads <= 1) {
                try (MediaChannel ch = open(); InputStream in = Channels.newInputStream(ch)) {
                    return in.readAllBytes();
                } catch (IOException e) {
                    throw corte(e);
                }
            }
            Acceso acceso = new Acceso(this);
            List<long[]> tramos = new ArrayList<>();
            for (long a = 0; a < size; a += TROZO) tramos.add(new long[] {a, Math.min(size, a + TROZO) - 1});
            ExecutorService ex = Executors.newFixedThreadPool(threads, Media::hilo);
            try {
                List<Future<byte[]>> fs = new ArrayList<>();
                for (long[] t : tramos) fs.add(ex.submit(() -> acceso.rango(t[0], t[1])));
                byte[] datos = new byte[(int) (long) size];
                int en = 0;
                for (Future<byte[]> f : fs) {
                    byte[] b = esperar(f);
                    System.arraycopy(b, 0, datos, en, Math.min(b.length, datos.length - en));
                    en += b.length;
                }
                verificar(this, en, sha256(datos));
                return datos;
            } finally {
                ex.shutdownNow();
            }
        }

        /** {@code length} bytes from {@code offset} (the contract's {@code read_range}). */
        public byte[] readRange(long offset, int length) {
            try (MediaChannel ch = open()) {
                return ch.readRange(offset, length);
            } catch (IOException e) {
                throw corte(e);
            }
        }
    }

    static Thread hilo(Runnable r) {
        Thread t = new Thread(r, "ore-media");
        t.setDaemon(true);
        return t;
    }

    static <T> T esperar(Future<T> f) {
        try {
            return f.get();
        } catch (ExecutionException e) {
            if (e.getCause() instanceof RuntimeException r) throw r;
            throw new MediaError("media/origen", 502, String.valueOf(e.getCause()));
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            throw new MediaError("media/origen", 499, "interrupted");
        }
    }

    static MediaCorrupt corte(IOException e) {
        if (e.getCause() instanceof MediaError m && m instanceof MediaCorrupt c) return c;
        return new MediaCorrupt("media/corrupto", 502, "the stream was cut: " + e.getMessage());
    }

    static String sha256(byte[] b) {
        try {
            return HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(b));
        } catch (NoSuchAlgorithmException e) {
            throw new IllegalStateException(e);
        }
    }

    static void verificar(Item item, long leidos, String visto) {
        MediaRef ref = item.ref;
        if (ref.size() != null && leidos != ref.size())
            throw new MediaCorrupt("media/corrupto", 502, ref.path() + ": " + leidos + " bytes of " + ref.size());
        String d = ref.digest();
        if (d != null && d.startsWith("sha256:") && !d.substring(7).equalsIgnoreCase(visto))
            throw new MediaCorrupt("media/corrupto", 502, ref.path() + ": sha256 " + visto + ", and the item says " + d.substring(7));
        if (d == null) item.sha256Seen = visto;
    }

    // ── el acceso: dónde leer, y otra vez cuando caduca ─────────────────────

    /** Una respuesta de bytes abierta, en flujo, con lo que dijo de sí. */
    record Abierta(int status, InputStream cuerpo, String version, String etag, String reprDigest) {}

    /** La URL vigente de un ítem, fijada a su versión, y pedir otra si caduca. */
    static final class Acceso {
        final Item item;
        String url;

        Acceso(Item item) { this.item = item; }

        @SuppressWarnings("unchecked")
        synchronized String vigente(boolean otra) {
            if (url == null || otra) {
                Map<String, Object> r = item.collection.donde(item.ref);
                url = texto(r.get("url"));
                // La versión fijada: si el ítem no la decía, la de ahora, y ya no cambia.
                String v = texto(r.get("version"));
                if (v != null && item.ref.version() == null) item.ref = item.ref.conVersion(v);
                if (r.get("item") instanceof Map<?, ?> m && item.ref.size() == null && m.get("size") instanceof Number n)
                    item.ref = item.ref.conTamano(n.longValue());
            }
            return url;
        }

        /** La respuesta en flujo desde {@code desde} (y hasta {@code hasta}); un permiso caducado (401/403) se renueva con la misma versión. */
        Abierta abrir(long desde, Long hasta) {
            String rango = desde > 0 || hasta != null ? "bytes=" + desde + "-" + (hasta == null ? "" : hasta) : null;
            for (int intento = 0; intento <= RENOVACIONES; intento++) {
                HttpRequest.Builder q = HttpRequest.newBuilder(URI.create(vigente(intento > 0))).timeout(Duration.ofSeconds(60)).GET();
                if (rango != null) q.header("Range", rango);
                HttpResponse<InputStream> r = null;
                try {
                    for (int vez = 0; r == null; vez++) {
                        try {
                            r = BYTES.send(q.build(), HttpResponse.BodyHandlers.ofInputStream());
                        } catch (IOException e) {
                            if (vez >= REPETICIONES) throw e;
                        }
                    }
                } catch (IOException e) {
                    throw new MediaError("media/origen", 502, "reading " + item.ref.path() + ": " + e);
                } catch (InterruptedException e) {
                    Thread.currentThread().interrupt();
                    throw new MediaError("media/origen", 499, "reading " + item.ref.path() + ": interrupted");
                }
                int st = r.statusCode();
                if (st == 200 && rango != null) {
                    try { r.body().close(); } catch (IOException e) { /* cerrar, y ya */ }
                    throw new MediaRangeError("media/sin-rangos", 200, "reading " + item.ref.path() + ": asked " + rango + " and the whole item came");
                }
                if (st == 200 || st == 206) {
                    aprender(r);
                    return new Abierta(st, r.body(), r.headers().firstValue("ore-media-version").orElse(null),
                        r.headers().firstValue("etag").orElse(null), r.headers().firstValue("repr-digest").orElse(null));
                }
                Map<String, Object> cuerpo;
                try (InputStream in = r.body()) {
                    String t = new String(in.readAllBytes(), StandardCharsets.UTF_8);
                    cuerpo = t.isBlank() ? Map.of() : Json.objeto(t);
                } catch (Exception e) {
                    cuerpo = Map.of();
                }
                if ((st == 401 || st == 403) && intento < RENOVACIONES) continue;
                if (st == 416 && hasta == null) return null;   // desde el final: no queda nada
                throw error(st, cuerpo, "reading " + item.ref.path());
            }
            throw new MediaForbidden("media/permiso", 401, "could not renew access to " + item.ref.path());
        }

        /** El tamaño, de la respuesta, si el ítem no lo decía: el total de {@code Content-Range}, o el largo de una lectura entera. */
        void aprender(HttpResponse<?> r) {
            if (item.ref.size() != null) return;
            String cr = r.headers().firstValue("content-range").orElse("");
            int barra = cr.lastIndexOf('/');
            if (barra >= 0 && cr.substring(barra + 1).trim().matches("\\d+")) {
                item.ref = item.ref.conTamano(Long.parseLong(cr.substring(barra + 1).trim()));
            } else if (r.statusCode() == 200) {
                r.headers().firstValueAsLong("content-length").ifPresent(n -> item.ref = item.ref.conTamano(n));
            }
        }

        /** El tamaño del ítem; si no se sabe, se pregunta un byte ({@code bytes=0-0}). */
        Long tamano() {
            if (item.ref.size() == null) {
                Abierta a = abrir(0, 0L);
                if (a != null) try { a.cuerpo().close(); } catch (IOException e) { /* cerrar, y ya */ }
            }
            return item.ref.size();
        }

        byte[] rango(long a, long z) {
            Abierta r = abrir(a, z);
            if (r == null) return new byte[0];
            try (InputStream in = r.cuerpo()) {
                return in.readAllBytes();
            } catch (IOException e) {
                throw new MediaCorrupt("media/corrupto", 502, item.ref.path() + ": the stream was cut (" + e.getMessage() + ")");
            }
        }
    }

    // ── el canal: un flujo desde la posición ────────────────────────────────

    /**
     * The channel of {@link Item#open()}: a stream from the position; changing the position opens
     * another from there (a {@code Range}). A whole read from the start, without jumps, is verified.
     * A stream that is cut is resumed from where it was, pinned to the same version.
     */
    public static final class MediaChannel implements SeekableByteChannel {
        final Item item;
        final Acceso acceso;
        long pos = 0;
        InputStream flujo;
        long posFlujo = -1;
        final MessageDigest hash;
        boolean entero = true;
        boolean abierto = true;
        int reanudaciones = 0;
        int status;
        String version, etag, reprDigest;

        MediaChannel(Item item) {
            this.item = item;
            this.acceso = new Acceso(item);
            acceso.vigente(false);
            try {
                hash = MessageDigest.getInstance("SHA-256");
            } catch (NoSuchAlgorithmException e) {
                throw new IllegalStateException(e);
            }
        }

        /** The version whose bytes arrived ({@code ORE-Media-Version}), or the pinned one before reading. */
        public String version() { return version != null ? version : item.ref.version(); }

        /** The {@code ETag} of the last response. */
        public String etag() { return etag; }

        /** The {@code Repr-Digest} of the last response (RFC 9530), if the cell knew it. */
        public String reprDigest() { return reprDigest; }

        /** The HTTP status of the last response: 200, or 206 for a range. */
        public int status() { return status; }

        /** The item it reads. */
        public Item item() { return item; }

        @Override public boolean isOpen() { return abierto; }

        @Override public long position() { return pos; }

        @Override public MediaChannel position(long nueva) throws IOException {
            if (!abierto) throw new ClosedChannelException();
            if (nueva < 0) throw new IllegalArgumentException("position before the start");
            if (nueva != pos) entero = false;
            pos = nueva;
            return this;
        }

        /** The item's size; if the listing did not say, it is asked (one byte). */
        @Override public long size() throws IOException {
            Long s = acceso.tamano();
            if (s == null) throw new MediaRangeError("media/rango", 416, "the origin does not give the size of " + item.ref.path());
            return s;
        }

        @Override public int write(ByteBuffer src) { throw new NonWritableChannelException(); }

        @Override public SeekableByteChannel truncate(long size) { throw new NonWritableChannelException(); }

        @Override public int read(ByteBuffer dst) throws IOException {
            if (!abierto) throw new ClosedChannelException();
            if (!dst.hasRemaining()) return 0;
            while (true) {
                Long size = item.ref.size();
                if (size != null && pos >= size) {
                    alFinal();
                    return -1;
                }
                if (flujo == null || posFlujo != pos) {
                    cerrarFlujo();
                    Abierta a = acceso.abrir(pos, null);
                    if (a == null) return -1;
                    flujo = a.cuerpo();
                    posFlujo = pos;
                    aprender(a);
                }
                int quiero = Math.min(dst.remaining(), 1 << 16);
                byte[] b = new byte[quiero];
                int n;
                try {
                    n = flujo.read(b, 0, quiero);
                } catch (IOException e) {
                    cerrarFlujo();
                    reanudar(e.getMessage());
                    continue;
                }
                if (n < 0) {
                    cerrarFlujo();
                    if (size != null && pos < size) {
                        reanudar("ended at " + pos + " of " + size);
                        continue;
                    }
                    alFinal();
                    return -1;
                }
                if (entero) hash.update(b, 0, n);
                pos += n;
                posFlujo = pos;
                dst.put(b, 0, n);
                return n;
            }
        }

        /** Lo que una respuesta dijo de sí; todo el flujo es de la versión fijada: si el servidor dice otra, no se mezcla. */
        private void aprender(Abierta a) {
            status = a.status();
            etag = a.etag();
            reprDigest = a.reprDigest();
            if (a.version() != null) {
                String fijada = item.ref.version();
                if (fijada != null && !fijada.equals(a.version())) {
                    try { a.cuerpo().close(); } catch (IOException e) { /* cerrar, y ya */ }
                    cerrarFlujo();
                    throw new MediaChanged("media/cambiado", 412, item.ref.path() + ": pinned to " + fijada + ", the cell served " + a.version());
                }
                version = a.version();
            }
        }

        /**
         * {@code length} bytes from {@code offset}, with a {@code Range} always (the contract's
         * {@code read_range}: a 206); the position does not move. Its headers are the channel's.
         */
        public byte[] readRange(long offset, int length) throws IOException {
            if (!abierto) throw new ClosedChannelException();
            if (length <= 0) return new byte[0];
            Abierta a = acceso.abrir(offset, offset + length - 1);
            if (a == null) return new byte[0];
            aprender(a);
            try (InputStream in = a.cuerpo()) {
                return in.readAllBytes();
            } catch (IOException e) {
                throw new MediaCorrupt("media/corrupto", 502, item.ref.path() + ": the stream was cut (" + e.getMessage() + ")");
            }
        }

        /** D-JM3: un flujo cortado se reanuda desde aquí, con la misma versión; sin más intentos, corrupto. */
        private void reanudar(String porque) {
            if (++reanudaciones > REANUDACIONES)
                throw new MediaCorrupt("media/corrupto", 502, item.ref.path() + ": the stream was cut at " + pos + " (" + porque + ")");
        }

        private void alFinal() {
            if (entero) {
                entero = false;
                verificar(item, pos, HexFormat.of().formatHex(hash.digest()));
            }
        }

        private void cerrarFlujo() {
            if (flujo != null) {
                try { flujo.close(); } catch (IOException e) { /* cerrar a medias corta, y ya */ }
            }
            flujo = null;
            posFlujo = -1;
        }

        @Override public void close() {
            cerrarFlujo();
            abierto = false;
        }
    }

    // ── escribir: la transacción (B4b·3) ────────────────────────────────────

    /** Cuántas veces se reintenta una subida cortada, o un commit que perdió la carrera de la forja. */
    static final int REINTENTOS = 3;
    /** Por debajo, lo que no se puede rebobinar se guarda en memoria; por encima, a disco. */
    static final int EN_MEMORIA = 8 << 20;
    /** Sólo para el laboratorio (`put-004`): el {@code Repr-Digest} de la próxima subida, en vez del suyo. */
    static volatile String reprDigestForzado = null;

    /** One {@code put} of {@link Transaction#putMany}: a path and its data ({@code byte[]}, a {@code Path} or an {@code InputStream}). */
    public record Put(String path, Object data, String contentType) {
        public static Put of(String path, Object data) { return new Put(path, data, null); }

        public static Put of(String path, Object data, String contentType) { return new Put(path, data, contentType); }
    }

    /** One result of {@link Transaction#putMany}: the path and its {@code MediaRef}, or the error that was its value. */
    public record PutResult(String path, MediaRef ref, RuntimeException error) {
        public boolean ok() { return error == null; }
    }

    /** Lo que se sube: rebobinable desde el principio, con su largo y su sha256. */
    private record Fuente(byte[] bytes, java.nio.file.Path fichero, long largo, byte[] sha, boolean temporal) {
        HttpRequest.BodyPublisher cuerpo() throws IOException {
            return bytes != null ? HttpRequest.BodyPublishers.ofByteArray(bytes) : HttpRequest.BodyPublishers.ofFile(fichero);
        }

        void cerrar() {
            if (temporal && fichero != null) try { java.nio.file.Files.deleteIfExists(fichero); } catch (IOException e) { /* un temporal */ }
        }
    }

    static MessageDigest sha256() {
        try {
            return MessageDigest.getInstance("SHA-256");
        } catch (NoSuchAlgorithmException e) {
            throw new IllegalStateException(e);
        }
    }

    /** {@code data} → lo que se sube. Un {@code InputStream} se copia antes (a memoria o a disco): la subida dice su largo y se reintenta. */
    private static Fuente fuente(Object data) throws IOException {
        if (data instanceof byte[] b) return new Fuente(b, null, b.length, sha256().digest(b), false);
        if (data instanceof java.nio.file.Path p) return new Fuente(null, p, java.nio.file.Files.size(p), shaDe(p), false);
        if (data instanceof java.io.File f) return fuente(f.toPath());
        if (data instanceof InputStream in) {
            MessageDigest h = sha256();
            byte[] buf = new byte[1 << 16];
            java.io.ByteArrayOutputStream mem = new java.io.ByteArrayOutputStream();
            java.nio.file.Path tmp = null;
            java.io.OutputStream disco = null;
            long n = 0;
            try {
                for (int r; (r = in.read(buf)) >= 0; ) {
                    h.update(buf, 0, r);
                    n += r;
                    if (disco == null && n > EN_MEMORIA) {
                        tmp = java.nio.file.Files.createTempFile("ore-put-", ".bin");
                        disco = java.nio.file.Files.newOutputStream(tmp);
                        mem.writeTo(disco);
                        mem = null;
                    }
                    if (disco != null) disco.write(buf, 0, r);
                    else mem.write(buf, 0, r);
                }
            } finally {
                if (disco != null) disco.close();
            }
            return tmp != null ? new Fuente(null, tmp, n, h.digest(), true) : new Fuente(mem.toByteArray(), null, n, h.digest(), false);
        }
        throw new IllegalArgumentException("put(): `data` is a byte[], a Path or an InputStream, not "
            + (data == null ? "null" : data.getClass().getSimpleName()));
    }

    private static byte[] shaDe(java.nio.file.Path p) throws IOException {
        MessageDigest h = sha256();
        try (InputStream in = java.nio.file.Files.newInputStream(p)) {
            byte[] buf = new byte[1 << 16];
            for (int r; (r = in.read(buf)) >= 0; ) h.update(buf, 0, r);
        }
        return h.digest();
    }

    /**
     * <b>An open transaction</b> on a written collection (B4b·3): {@code put} uploads items,
     * {@code commit()} leaves them written —and the pointer, with its provenance—, {@code abort()}
     * leaves nothing. Get one with {@link Collection#transaction()}.
     *
     * <p>Commit is explicit (D-JM1): {@code close()} without {@code commit()} is {@code abort()}, so
     * {@code try (var t = c.transaction()) { t.put(…); t.commit(); }} writes nothing if it throws.
     */
    public static final class Transaction implements AutoCloseable {
        public final Collection collection;
        private final String id;
        private final String upload;
        private final List<MediaRef> uploaded = java.util.Collections.synchronizedList(new ArrayList<>());
        private volatile Map<String, Object> result;
        private volatile boolean closed;
        /** 0049 B9: lo que el {@code commit} lleva además de lo subido. */
        final Map<String, List<Object>> linaje = new LinkedHashMap<>(Map.of("derivations", new ArrayList<>(),
            "retire_sources", new ArrayList<>(), "retire", new ArrayList<>()));

        Transaction(Collection col, int ttlS) {
            this.collection = col;
            Ore.Respuesta r = col.escribir("POST", "/transactions", Map.of("ttl_s", ttlS), "transaction(" + col + ")");
            if (r.codigo() != 201) throw error(r.codigo(), r.cuerpo(), "transaction(" + col + ")");
            id = texto(r.cuerpo().get("transaction"));
            upload = texto(r.cuerpo().get("upload"));
        }

        /** The transaction's id (the cell's). */
        public String id() { return id; }

        /** What was uploaded so far, as the cell saw it. */
        public List<MediaRef> uploaded() { return List.copyOf(uploaded); }

        /** What {@code commit()} returned, or {@code null}. */
        public Map<String, Object> result() { return result; }

        public boolean closed() { return closed; }

        @Override public String toString() { return "Transaction(" + collection + ", " + id + (closed ? ", closed" : "") + ")"; }

        private void abierta(String que) {
            if (closed) throw new MediaTransactionError("media/transaccion", 409, que + ": transaction " + id + " is already closed");
        }

        /** Uploads {@code data} to {@code path} (relative to the collection); the type is the bytes' (the cell detects it). */
        public MediaRef put(String path, byte[] data) { return put(path, (Object) data, null); }

        /** The same, declaring a type: it counts only if the bytes say nothing. */
        public MediaRef put(String path, byte[] data, String contentType) { return put(path, (Object) data, contentType); }

        /** A file, in stream, with its length. */
        public MediaRef put(String path, java.nio.file.Path file) { return put(path, (Object) file, null); }

        public MediaRef put(String path, java.nio.file.Path file, String contentType) { return put(path, (Object) file, contentType); }

        /** A stream: it is copied first (to memory, or to disk if large), because the upload says its length and is retried. */
        public MediaRef put(String path, InputStream in) { return put(path, (Object) in, null); }

        public MediaRef put(String path, InputStream in, String contentType) { return put(path, (Object) in, contentType); }

        MediaRef put(String path, Object data, String contentType) {
            abierta("put(" + path + ")");
            Fuente f;
            try {
                f = fuente(data);
            } catch (IOException e) {
                throw new MediaError("media/origen", 502, "put(" + path + "): " + e.getMessage());
            }
            try {
                return subir(path, f, contentType);
            } finally {
                f.cerrar();
            }
        }

        private MediaRef subir(String path, Fuente f, String tipo) {
            String destino = upload + (upload.contains("?") ? "&" : "?") + "path="
                + URLEncoder.encode(path, StandardCharsets.UTF_8).replace("%2F", "/").replace("+", "%20");
            String digest = reprDigestForzado;
            reprDigestForzado = null;
            if (digest == null) digest = "sha-256=:" + java.util.Base64.getEncoder().encodeToString(f.sha()) + ":";
            Exception ultimo = null;
            for (int intento = 0; intento <= REINTENTOS; intento++) {
                try {
                    // Sin el token de ORE: `upload` ya es el permiso, como la URL de `open`.
                    HttpRequest.Builder q = HttpRequest.newBuilder(URI.create(destino)).timeout(Duration.ofMinutes(5))
                        .header("repr-digest", digest).PUT(f.cuerpo());
                    if (tipo != null) q.header("content-type", tipo);
                    HttpResponse<String> r = BYTES.send(q.build(), HttpResponse.BodyHandlers.ofString());
                    Map<String, Object> cuerpo = r.body() == null || r.body().isBlank() ? Map.of() : Json.objeto(r.body());
                    if (r.statusCode() != 201) throw error(r.statusCode(), cuerpo, "put(" + path + ")");
                    MediaRef ref = MediaRef.fromJson(cuerpo);
                    uploaded.add(ref);
                    return ref;
                } catch (IOException e) {
                    // Cortada: se vuelve a subir. Es idempotente —el mismo contenido al mismo
                    // camino es la misma fila, y el blob ya está si llegó—.
                    ultimo = e;
                } catch (InterruptedException e) {
                    Thread.currentThread().interrupt();
                    throw new MediaError("media/origen", 499, "put(" + path + "): interrupted");
                }
            }
            throw new MediaError("media/origen", 503, "put(" + path + "): the upload was cut " + (REINTENTOS + 1) + " times: " + ultimo);
        }

        /**
         * Many {@code put}s at once, {@code threads} at a time, as they finish: one's error is a value
         * and does not stop the others. At most {@code 2 × threads} are in flight: {@code puts} is not
         * read whole up front.
         */
        public Iterable<PutResult> putMany(Iterable<Put> puts, int threads) {
            abierta("putMany");
            Transaction t = this;
            return () -> new Iterator<PutResult>() {
                final Iterator<Put> ps = puts.iterator();
                ExecutorService ex;
                CompletionService<PutResult> cs;
                int vivos = 0;

                void lanzar() {
                    if (ex == null) {
                        ex = Executors.newFixedThreadPool(threads, Media::hilo);
                        cs = new ExecutorCompletionService<>(ex);
                    }
                    while (vivos < threads * 2 && ps.hasNext()) {
                        Put p = ps.next();
                        cs.submit(() -> {
                            try {
                                return new PutResult(p.path(), t.put(p.path(), p.data(), p.contentType()), null);
                            } catch (RuntimeException e) {
                                return new PutResult(p.path(), null, e);
                            }
                        });
                        vivos++;
                    }
                }

                @Override public boolean hasNext() {
                    lanzar();
                    if (vivos == 0 && ex != null) ex.shutdown();
                    return vivos > 0;
                }

                @Override public PutResult next() {
                    if (!hasNext()) throw new NoSuchElementException();
                    try {
                        PutResult r = cs.take().get();
                        vivos--;
                        return r;
                    } catch (InterruptedException e) {
                        Thread.currentThread().interrupt();
                        ex.shutdownNow();
                        throw new MediaError("media/origen", 499, "interrupted");
                    } catch (ExecutionException e) {
                        vivos--;
                        throw new MediaError("media/origen", 502, String.valueOf(e.getCause()));
                    }
                }
            };
        }

        /** Retires the item at {@code path} when this transaction commits (0049 B9). */
        public void delete(String path) {
            abierta("delete(" + path + ")");
            synchronized (linaje) { linaje.get("retire").add(path); }
        }

        /**
         * Leaves what was uploaded written —the pointer, with its provenance— and closes it. If
         * someone else committed at the same time, it commits again on top. Returns
         * {@code {transaction, items, commit, metadata_location, …}}.
         */
        public Map<String, Object> commit() {
            abierta("commit");
            long espera = 500;
            for (int intento = 0; intento <= REINTENTOS; intento++) {
                Map<String, Object> cuerpo = new LinkedHashMap<>();
                synchronized (linaje) { for (var e : linaje.entrySet()) if (!e.getValue().isEmpty()) cuerpo.put(e.getKey(), List.copyOf(e.getValue())); }
                Ore.Respuesta r = collection.escribir("POST", "/transactions/" + id + "/commit", cuerpo, "commit(" + id + ")");
                if (r.codigo() == 200) {
                    closed = true;
                    result = Ore.enIngles(r.cuerpo());
                    Ore.alInformeDeMedia(collection.shortName, result);
                    return result;
                }
                boolean carrera = r.codigo() == 409 && r.cuerpo().get("type") == null;
                if (!carrera || intento == REINTENTOS) throw error(r.codigo(), r.cuerpo(), "commit(" + id + ")");
                try {
                    Thread.sleep(espera);
                } catch (InterruptedException e) {
                    Thread.currentThread().interrupt();
                    throw new MediaError("media/origen", 499, "commit(" + id + "): interrupted");
                }
                espera *= 2;
            }
            throw new IllegalStateException("unreachable");
        }

        /** Leaves nothing of what was uploaded, and closes it. */
        public void abort() {
            if (closed) return;
            Ore.Respuesta r = collection.escribir("POST", "/transactions/" + id + "/abort", Map.of(), "abort(" + id + ")");
            closed = true;
            if (r.codigo() != 204 && r.codigo() != 404) throw error(r.codigo(), r.cuerpo(), "abort(" + id + ")");
        }

        /** D-JM1: without {@code commit()}, {@code abort()}. */
        @Override public void close() {
            if (!closed) abort();
        }
    }

    /** {@code verify}: one result per item, in its position: {@code ok}, with the sha256 seen, or its error. */
    public record Verified(MediaRef item, boolean ok, String sha256, MediaError error) {}

    // ── muchos a la vez ─────────────────────────────────────────────────────

    /** One result of {@link Ore#readMany}: the item and its bytes, or the error that was its value. */
    public record Result(Item item, byte[] data, RuntimeException error) {
        public boolean ok() { return error == null; }
    }

    /** {@code Ore.readMany}: lo que va terminando, con un tope de lo que hay en vuelo; {@code items} puede ser perezoso. */
    static Iterable<Result> leerVarios(Iterable<Item> items, int threads) {
        return () -> new Iterator<Result>() {
            final Iterator<Item> its = items.iterator();
            ExecutorService ex;
            CompletionService<Result> cs;
            int vivos = 0;

            void lanzar() {
                if (ex == null) {
                    ex = Executors.newFixedThreadPool(threads, Media::hilo);
                    cs = new ExecutorCompletionService<>(ex);
                }
                while (vivos < threads * 2 && its.hasNext()) {
                    Item it = its.next();
                    cs.submit(() -> {
                        try {
                            return new Result(it, it.readBytes(1), null);
                        } catch (RuntimeException e) {
                            return new Result(it, null, e);
                        }
                    });
                    vivos++;
                }
            }

            @Override public boolean hasNext() {
                lanzar();
                if (vivos == 0 && ex != null) ex.shutdown();
                return vivos > 0;
            }

            @Override public Result next() {
                if (!hasNext()) throw new NoSuchElementException();
                try {
                    Result r = cs.take().get();
                    vivos--;
                    return r;
                } catch (InterruptedException e) {
                    Thread.currentThread().interrupt();
                    ex.shutdownNow();
                    throw new MediaError("media/origen", 499, "interrupted");
                } catch (ExecutionException e) {
                    vivos--;
                    throw new MediaError("media/origen", 502, String.valueOf(e.getCause()));
                }
            }
        };
    }
}
