package conformidad;

import java.io.ByteArrayInputStream;
import java.io.FilterInputStream;
import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Random;
import java.util.Set;
import java.util.concurrent.atomic.AtomicInteger;

import ore.Media;
import ore.Ore;
import ore.ParaElBanco;

/**
 * JM3 · ESCRIBIR: las `op` `put` y `verify` de la suite, y las quince de B4b·3
 * que {@code la-media-escrita-en-python.py} prueba en Python, en Java; y el
 * build de un {@code @Transform} cuya salida es una colección escrita, con la
 * celda de {@code builds.rs}.
 */
final class Escritura {
    private Escritura() {}

    static void registrar(Map<String, Ejecutor.Op> ops) {
        ops.put("put", Escritura::put);
        ops.put("verify", Escritura::verify);
    }

    static Long numero(Object o) { return o instanceof Number n ? n.longValue() : null; }

    static void exige(boolean cierto, String porque) { if (!cierto) throw new AssertionError(porque); }

    // ── la suite ────────────────────────────────────────────────────────────

    static Set<String> caminos(Media.Collection c) {
        Set<String> s = new HashSet<>();
        for (Media.Item i : c.items()) s.add(i.ref().path());
        return s;
    }

    /**
     * Un {@code commit} con lo que el caso dice: {@code true}, o {@code {retire, derivations}} (el
     * put derivado de B9: {@code retire} es {@code delete()}; las derivaciones crudas, las que
     * {@code apply()} arma por dentro).
     */
    @SuppressWarnings("unchecked")
    static Map<String, Object> confirmar(Lectura.Ctx x, Media.Transaction t, Object commit) throws Exception {
        Map<String, Object> c = Lectura.m(commit);
        for (Object p : Lectura.l(c.get("retire"))) t.delete(String.valueOf(p));
        for (Object d : Lectura.l(c.get("derivations")))
            ParaElBanco.derivacion(t, (Map<String, Object>) ore.Json.leer(x.r(ore.Json.escribir(d))));
        return t.commit();
    }

    static String put(Map<String, Object> caso, String en) throws Exception {
        Lectura.Ctx x = Lectura.preparar(caso, en);
        Map<String, Object> pide = Lectura.m(caso.get("pide")), espera = Lectura.m(caso.get("espera"));
        String path = pide.get("path") == null ? null : String.valueOf(pide.get("path"));
        byte[] datos = String.valueOf(pide.get("texto")).getBytes(StandardCharsets.UTF_8);
        int veces = pide.get("repetir") instanceof Number n ? n.intValue() : 1;
        List<Media.MediaRef> refs = new ArrayList<>();
        List<Map<String, Object>> commits = new ArrayList<>();
        Throwable error = null;
        try {
            for (int i = 0; i < veces; i++) {
                if (pide.get("repr_digest") != null) ParaElBanco.reprDigest(String.valueOf(pide.get("repr_digest")));
                // D-JM1: lo que no se confirma a mano no se escribe (close() es abort()).
                try (Media.Transaction t = x.c.transaction()) {
                    if (path != null) refs.add(t.put(path, datos));
                    Object commit = pide.get("commit");
                    if (commit != null && !Boolean.FALSE.equals(commit)) commits.add(confirmar(x, t, commit));
                    else if (Boolean.TRUE.equals(pide.get("abort"))) t.abort();
                }
            }
            Map<String, Object> despues = Lectura.m(pide.get("despues"));
            if (!despues.isEmpty())
                try (Media.Transaction t = x.c.transaction()) {
                    commits.add(confirmar(x, t, despues.get("commit")));
                }
        } catch (Media.MediaError e) {
            error = e;
        } finally {
            ParaElBanco.reprDigest(null);
        }
        Lectura.errorEsperado(x, espera, error);
        Set<String> despues = caminos(x.c);
        for (Object p : Lectura.l(espera.get("despues_en_list"))) exige(despues.contains(String.valueOf(p)), "[" + en + "] el listado no tiene " + p + ": " + despues);
        for (Object p : Lectura.l(espera.get("despues_no_en_list"))) exige(!despues.contains(String.valueOf(p)), "[" + en + "] el listado tiene " + p);
        if (error != null || refs.isEmpty()) return null;
        Media.MediaRef ref = refs.get(0);
        if (espera.get("status") instanceof Number n) exige(n.intValue() == 201, "[" + en + "] esperaba " + n + " y put devolvió su ref (201)");
        if (!Lectura.m(espera.get("ref")).isEmpty()) Lectura.ref(x, Lectura.m(espera.get("ref")), ref);
        if (Boolean.TRUE.equals(espera.get("digest_de_lo_enviado")))
            exige(("sha256:" + Lectura.sha(datos)).equals(ref.digest()), "[" + en + "] digest " + ref.digest());
        if (Boolean.TRUE.equals(espera.get("size_de_lo_enviado")))
            exige(ref.size() != null && ref.size() == datos.length, "[" + en + "] size " + ref.size() + " y no " + datos.length);
        for (Object inv : Lectura.l(espera.get("invariante"))) {
            exige("idempotente".equals(inv), "invariante `" + inv + "` desconocida");
            exige(refs.size() == veces && refs.stream().allMatch(r -> r.digest().equals(ref.digest()) && r.version().equals(ref.version())),
                "[" + en + "] las subidas dieron refs distintas: " + refs);
            exige(despues.size() == 1, "[" + en + "] repetir creó " + despues.size() + " ítems");
            // «No crea nada nuevo»: lo que el segundo commit confirmó (sin entrar ni cambiar nada).
            Map<String, Object> ultimo = commits.get(commits.size() - 1), cambios = Lectura.m(ultimo.get("changes"));
            exige(Boolean.TRUE.equals(ultimo.get("sin_cambios"))
                || (Long.valueOf(0).equals(numero(cambios.get("entran"))) && Long.valueOf(0).equals(numero(cambios.get("cambian")))),
                "[" + en + "] el segundo commit escribió algo: " + ultimo);
        }
        return null;
    }

    static String verify(Map<String, Object> caso, String en) throws Exception {
        Lectura.Ctx x = Lectura.preparar(caso, en);
        Map<String, Object> pide = Lectura.m(caso.get("pide")), espera = Lectura.m(caso.get("espera"));
        List<Object> paths = new ArrayList<>();
        for (Object o : Lectura.l(pide.get("items"))) paths.add(Lectura.m(o).get("path"));
        List<Media.Verified> vs = x.c.verify(paths);
        exige(vs.size() == paths.size(), "[" + en + "] " + vs.size() + " resultados para " + paths.size());
        if ("ok".equals(espera.get("todos"))) for (Media.Verified v : vs) exige(v.ok(), "[" + en + "] " + v);
        return null;
    }

    // ── las quince de B4b·3, en Java ────────────────────────────────────────

    static final String PAG = "legal.archivo.paginas";
    static final byte[] PNG = "\u0089PNG\r\n\u001a\n-una-pagina-".getBytes(StandardCharsets.ISO_8859_1);

    static byte[] mas(byte[] a, String b) {
        byte[] x = b.getBytes(StandardCharsets.ISO_8859_1), r = new byte[a.length + x.length];
        System.arraycopy(a, 0, r, 0, a.length);
        System.arraycopy(x, 0, r, a.length, x.length);
        return r;
    }

    static String documento(String base, String schema, String nombre) throws Exception {
        return String.valueOf(Banco.mando("documento", "kind", "MediaCollection", "base", base, "schema", schema, "nombre", nombre).get("yaml"));
    }

    static long puntero(String col) throws Exception {
        Object n = Lectura.m(Banco.mando("punteros").get("punteros")).get(col);
        return n instanceof Number x ? x.longValue() : 0;
    }

    static int correr() throws Exception {
        System.out.println("la media escrita en java");
        ParaElBanco.credencial(() -> Map.of("authorization", Sdk.TOKEN));
        int antes = Sdk.fallos;
        try {
            Sdk.caso("escrita", 1, () -> {
                Map<String, Object> r = Ore.createCollection(PAG, "image", List.of("PNG", ".webp"), null, null, Map.of("gdpr.sensitivity", "high"), null, false);
                exige(r.equals(Map.of("collection", PAG, "created", true)), "createCollection dio " + r);
                String y = documento("legal", "archivo", "paginas");
                exige(y.contains("apiVersion: oos.dev/v1alpha19") && !y.contains("from:"), y);
                exige(y.contains("formats: [png, webp]") && y.contains("gdpr.sensitivity: high"), y);
                exige(y.split("owner:", -1).length == 2 && y.contains("owner: user:ana"), y);
                Ore.createCollection("legal.archivo.de_bea", "image", List.of("png"), "user:bea", null, null, null, false);
                y = documento("legal", "archivo", "de_bea");
                exige(y.contains("owner: user:bea") && !y.contains("user:ana"), y);
                return "createCollection(): v1alpha19 sin `from`, formatos en minúscula, etiquetas; sin owner el SDK no lo escribe "
                    + "y es de quien la crea (user:ana), y con owner es de quien se diga";
            });
            Sdk.caso("escrita", 2, () -> {
                try {
                    Ore.createCollection(PAG, "image", List.of("png"));
                    throw new AssertionError("debía fallar");
                } catch (IllegalStateException e) {
                    exige(e.getMessage().contains("already exists"), e.getMessage());
                }
                Banco.mando("registro", "limpiar", true);
                Map<String, Object> r = Ore.createCollection(PAG, "image", List.of("png"), null, null, null, null, true);
                exige(Boolean.FALSE.equals(r.get("created")), "created " + r);
                for (List<Object> q : Sdk.registro("serve")) exige(!"PUT".equals(q.get(0)), "escribió: " + q.get(1));
                return "ya existe: error; con ifNotExists, `created: false` y no se escribe nada";
            });
            Sdk.caso("escrita", 3, () -> {
                try {
                    Ore.createCollection("legal.rota", "document", List.of("pdf"));
                } catch (IllegalArgumentException e) {
                    exige(e.getMessage().contains("OOS1004"), e.getMessage());
                    return "un código OOS del servidor vuelve como IllegalArgumentException";
                }
                throw new AssertionError("debía ser IllegalArgumentException");
            });
            Sdk.caso("escrita", 4, () -> {
                Banco.mando("registro", "limpiar", true);
                List<List<Object>> malos = List.of(List.of("binary", List.of("bin")), List.of("image", List.of()),
                    List.of("image", List.of("png", "png")), List.of("image", List.of("p ng")));
                for (List<Object> m : malos) {
                    try {
                        @SuppressWarnings("unchecked") List<String> fs = (List<String>) m.get(1);
                        Ore.createCollection("legal.fotos", String.valueOf(m.get(0)), fs);
                        throw new AssertionError(m + " debía ser IllegalArgumentException");
                    } catch (IllegalArgumentException e) {
                        // bien
                    }
                }
                exige(Sdk.registro("serve").isEmpty(), "preguntó: " + Sdk.registro("serve"));
                return "un medio que no es, formatos vacíos, repetidos o raros: IllegalArgumentException sin preguntar";
            });
            Sdk.caso("escrita", 5, () -> {
                Banco.mando("registro", "limpiar", true);
                Media.MediaRef ref;
                Media.Transaction t;
                try (Media.Transaction tx = Ore.collection(PAG).transaction()) {
                    t = tx;
                    ref = tx.put("c1/p0.png", PNG, "text/html");
                    tx.commit();
                }
                exige(("sha256:" + Lectura.sha(PNG)).equals(ref.digest()) && "image/png".equals(ref.contentType()), "ref " + ref);
                exige(t.closed() && Long.valueOf(1).equals(((Number) t.result().get("transaccion")).longValue()) && puntero(PAG) == 1, "result " + t.result());
                List<List<Object>> bytes = Sdk.registro("bytes");
                Map<String, Object> h = Sdk.cabeceras(bytes.get(bytes.size() - 1));
                exige(String.valueOf(h.get("repr-digest")).startsWith("sha-256=:") && "text/html".equals(h.get("content-type")), "cabeceras " + h);
                exige(String.valueOf(bytes.get(bytes.size() - 1).get(1)).contains("c1/p0.png"), "ruta " + bytes.get(bytes.size() - 1).get(1));
                return "try (transaction()): put de bytes con Repr-Digest, tipo por los bytes (image/png), commit a mano (D-JM1)";
            });
            Sdk.caso("escrita", 6, () -> {
                Path f = Files.createTempFile("grande", ".bin");
                byte[] datos = new byte[300_000];
                new Random(6).nextBytes(datos);
                Files.write(f, datos);
                Banco.mando("registro", "limpiar", true);
                Media.MediaRef ref;
                try (Media.Transaction t = Ore.collection(PAG).transaction()) {
                    ref = t.put("c1/grande.bin", f);
                    t.commit();
                }
                Map<String, Object> l = Banco.mando("lago", "sha256", Lectura.sha(datos));
                exige(Boolean.TRUE.equals(l.get("esta")) && Long.valueOf(300_000).equals(((Number) l.get("size")).longValue()), "lago " + l);
                List<List<Object>> bytes = Sdk.registro("bytes");
                exige("300000".equals(Sdk.cabeceras(bytes.get(bytes.size() - 1)).get("content-length")), "content-length " + Sdk.cabeceras(bytes.get(bytes.size() - 1)));
                exige(ref.size() == 300_000, "size " + ref.size());
                Files.delete(f);
                return "put de un Path: en flujo, con su largo (300 000 bytes, mismos bytes en el lago)";
            });
            Sdk.caso("escrita", 7, () -> {
                byte[] datos = mas("%PDF-".getBytes(StandardCharsets.ISO_8859_1), "x".repeat(50_000));
                InputStream sinRebobinar = new FilterInputStream(new ByteArrayInputStream(datos)) {
                    @Override public boolean markSupported() { return false; }
                };
                Media.MediaRef ref;
                try (Media.Transaction t = Ore.collection(PAG).transaction()) {
                    ref = t.put("c1/flujo.pdf", sinRebobinar);
                    t.commit();
                }
                exige(("sha256:" + Lectura.sha(datos)).equals(ref.digest()) && "application/pdf".equals(ref.contentType()), "ref " + ref);
                return "put de un InputStream que no se rebobina: se copia antes y sube igual (application/pdf)";
            });
            Sdk.caso("escrita", 8, () -> {
                Banco.mando("rama", "rama", "r1/paginas");
                Banco.mando("registro", "limpiar", true);
                Media.Transaction t;
                try (Media.Transaction tx = Ore.collection(PAG).transaction()) {
                    t = tx;
                    tx.put("c1/p1.png", PNG);
                    tx.commit();
                } finally {
                    Banco.mando("rama", "rama", "r1/trabajo");
                }
                List<Map<String, Object>> puts = new ArrayList<>();
                for (List<Object> q : Sdk.registro("bytes")) if ("PUT".equals(q.get(0))) puts.add(Sdk.cabeceras(q));
                exige(!puts.isEmpty() && puts.stream().noneMatch(h -> h.containsKey("authorization") || h.containsKey("x-ore-puesto")), "subidas " + puts);
                List<Map<String, Object>> celda = new ArrayList<>();
                for (List<Object> q : Sdk.registro("serve")) if (String.valueOf(q.get(1)).contains("/transactions")) celda.add(Sdk.cabeceras(q));
                // La JVM pregunta la rama una vez por proceso (Sdk 13): la que viaja es la de entonces.
                exige(!celda.isEmpty() && celda.stream().allMatch(h -> "r1/trabajo".equals(h.get("x-ore-rama")) && h.containsKey("authorization")), "a la celda " + celda);
                exige(t.toString().contains(t.id()) && !t.toString().contains("permiso") && !t.toString().contains("subida"), t.toString());
                return "el token de ORE nunca va a `upload` (" + puts.size() + " subidas); la rama y el token, a la celda (" + celda.size() + ")";
            });
            Sdk.caso("escrita", 9, () -> {
                Banco.mando("modos", "cortar_subidas", 2);
                Banco.mando("registro", "limpiar", true);
                Media.MediaRef ref;
                try (Media.Transaction t = Ore.collection(PAG).transaction()) {
                    ref = t.put("c1/p2.png", mas(PNG, "-2"));
                    t.commit();
                }
                long intentos = Sdk.registro("bytes").stream().filter(q -> "PUT".equals(q.get(0))).count();
                exige(intentos == 3 && "c1/p2.png".equals(ref.path()), intentos + " intentos");
                return "una subida cortada dos veces se reintenta y entra a la tercera";
            });
            Sdk.caso("escrita", 10, () -> {
                Banco.mando("modos", "conflictos", 2);
                Banco.mando("registro", "limpiar", true);
                long antesP = puntero(PAG);
                try (Media.Transaction t = Ore.collection(PAG).transaction()) {
                    t.put("c1/p3.png", mas(PNG, "-3"));
                    t.commit();
                }
                long commits = Sdk.registro("serve").stream().filter(q -> String.valueOf(q.get(1)).endsWith("/commit")).count();
                exige(commits == 3 && puntero(PAG) == antesP + 1, commits + " commits, puntero " + puntero(PAG));
                return "el commit perdió la carrera de la forja dos veces (409 sin type): confirmado a la tercera";
            });
            Sdk.caso("escrita", 11, () -> {
                Banco.mando("registro", "limpiar", true);
                long antesP = puntero(PAG);
                Media.Transaction t = null;
                try (Media.Transaction tx = Ore.collection(PAG).transaction()) {
                    t = tx;
                    tx.put("c1/p4.png", mas(PNG, "-4"));
                    throw new IllegalStateException("algo se rompió");
                } catch (IllegalStateException e) {
                    exige(e.getMessage().equals("algo se rompió"), "la excepción cambió: " + e);
                }
                exige(Sdk.registro("serve").stream().anyMatch(q -> String.valueOf(q.get(1)).endsWith("/abort")) && puntero(PAG) == antesP, "no abortó");
                exige(t.closed() && t.result() == null, "cerrada " + t.closed() + ", result " + t.result());
                try {
                    t.put("c1/p5.png", PNG);
                    throw new AssertionError("cerrada, put debía ser MediaTransactionError");
                } catch (Media.MediaTransactionError e) {
                    // bien
                }
                return "una excepción dentro: close() es abort (D-JM1), el puntero no se mueve, la excepción sigue; cerrada no admite put";
            });
            Sdk.caso("escrita", 12, () -> {
                Banco.mando("registro", "limpiar", true);
                Map<String, Object> r = Ore.transform("paginar", List.of("legal.archivo.contratos"), PAG, () -> {
                    try {
                        Ore.collection("legal.archivo.otra").transaction();
                        throw new AssertionError("debía ser MediaForbidden");
                    } catch (Media.MediaForbidden e) {
                        exige(e.getMessage().contains("legal.archivo.otra"), e.getMessage());
                    }
                    try (Media.Transaction t = Ore.collection(PAG).transaction()) {
                        t.put("c2/p0.png", mas(PNG, "-c2"));
                        return t.commit();
                    }
                });
                exige(((Number) r.get("transaccion")).longValue() >= 1, "result " + r);
                exige(Sdk.registro("serve").stream().noneMatch(q -> String.valueOf(q.get(1)).startsWith("/media/legal/archivo/otra")), "preguntó por la otra");
                return "dentro de un transform: su output se escribe; otra colección, MediaForbidden sin preguntar";
            });
            Sdk.caso("escrita", 13, () -> {
                try {
                    Ore.collection("legal.archivo.contratos").transaction();
                    throw new AssertionError("una mantenida debía ser MediaNotWritable");
                } catch (Media.MediaNotWritable e) {
                    exige(e.status == 409, "status " + e.status);
                }
                exige(ParaElBanco.error(422, "media/digest-no-casa") instanceof Media.MediaCorrupt, "digest-no-casa");
                exige(ParaElBanco.error(404, "media/transaccion") instanceof Media.MediaTransactionError, "transaccion");
                exige(ParaElBanco.error(401, "media/permiso") instanceof Media.MediaForbidden, "permiso");
                return "una mantenida no se escribe (MediaNotWritable, 409); digest-no-casa, transaccion y permiso, por su tipo";
            });
            Sdk.caso("escrita", 14, () -> {
                AtomicInteger leidos = new AtomicInteger();
                Iterable<Media.Put> pares = () -> new java.util.Iterator<>() {
                    int i = 0;

                    @Override public boolean hasNext() { return i < 100; }

                    @Override public Media.Put next() {
                        leidos.incrementAndGet();
                        int k = i++;
                        return Media.Put.of(String.format("c3/p%03d.png", k), mas(PNG, "-" + k));
                    }
                };
                Media.Transaction t;
                int primeros = -1;
                try (Media.Transaction tx = Ore.collection(PAG).transaction()) {
                    t = tx;
                    for (Media.PutResult r : tx.putMany(pares, 8)) {
                        if (primeros < 0) primeros = leidos.get();
                        exige(r.ok() && r.ref().path().equals(r.path()), r.path() + ": " + r.error());
                    }
                    tx.commit();
                }
                exige(t.uploaded().size() == 100, t.uploaded().size() + " subidos");
                for (int i = 0; i < 100; i++) exige(Boolean.TRUE.equals(Banco.mando("lago", "sha256", Lectura.sha(mas(PNG, "-" + i))).get("esta")), "falta " + i);
                exige(primeros <= 2 * 8 + 1, "al acabar el primero se habían leído " + primeros);
                exige(Long.valueOf(100).equals(((Number) Lectura.m(t.result().get("items")).get("actuales")).longValue()), "result " + t.result());
                return "putMany(): 100 a la vez, en el lago y confirmados; al acabar el primero se habían leído " + primeros;
            });
            Sdk.caso("escrita", 15, () -> {
                Map<String, Media.PutResult> r = new LinkedHashMap<>();
                Media.Transaction t;
                try (Media.Transaction tx = Ore.collection(PAG).transaction()) {
                    t = tx;
                    for (Media.PutResult x : tx.putMany(List.of(Media.Put.of("c4/a.png", mas(PNG, "a")), Media.Put.of("c4/malo.png", 42),
                        Media.Put.of("c4/b.png", mas(PNG, "b"), "image/png")), 2)) r.put(x.path(), x);
                    tx.commit();
                }
                exige(r.get("c4/malo.png").error() instanceof IllegalArgumentException, "malo " + r.get("c4/malo.png"));
                exige(r.get("c4/a.png").ok() && r.get("c4/b.png").ok() && Long.valueOf(2).equals(((Number) Lectura.m(t.result().get("items")).get("actuales")).longValue()), "result " + t.result());
                return "putMany(): el error de uno (IllegalArgumentException) es un valor; los otros dos entran y se confirman";
            });
            // ── la puerta del Preview, en la JVM (0055 P1) ──────────────────────
            Sdk.caso("escrita", 16, () -> {
                Banco.mando("registro", "limpiar", true);
                ParaElBanco.ensayo(PAG, "paginar");
                try {
                    Ore.collection(PAG).transaction();
                    throw new AssertionError("un Preview no debía abrir una transacción");
                } catch (SecurityException e) {
                    exige(e.getMessage().startsWith("Preview writes nothing"), e.getMessage());
                } finally {
                    ParaElBanco.finDelEnsayo();
                }
                exige(Sdk.registro("serve").stream().noneMatch(q -> String.valueOf(q.get(1)).contains("/transactions")), "llegó a la celda");
                return "en un Preview, transaction() no sale de la celda: Preview writes nothing (como Python)";
            });
            // ── el build de un @Transform cuya salida es una colección escrita ──
            Sdk.caso("escrita", 17, () -> {
                Path f = Path.of("/src/target/celdas/paginas-build.jsh");
                exige(Files.exists(f), "no está la celda generada (" + f + ")");
                long antesP = puntero("ventas.archivo.paginas");
                int antesT = Sdk.transforms().size();
                ParaElBanco.codigo("packages/ventas/etl/Paginas.java@c0ffee");
                Map<String, Object> salida;
                try {
                    salida = ParaElBanco.celda(Files.readString(f));
                } finally {
                    ParaElBanco.codigo(null);
                }
                exige(!"error".equals(salida.get("tipo")), "la celda dio error: " + ore.Json.escribir(salida));
                exige(String.valueOf(salida.get("texto")).contains("built from packages/ventas/etl/Paginas.java:paginas · transaction 1 · 2 items"), "texto " + salida.get("texto"));
                Map<String, Object> informe = Lectura.m(salida.get("informe"));
                exige(Long.valueOf(2).equals(((Number) informe.get("filas")).longValue()) && informe.get("transaccion") != null, "informe " + informe);
                exige(puntero("ventas.archivo.paginas") == antesP + 1, "puntero " + puntero("ventas.archivo.paginas"));
                List<Object> ts = Sdk.transforms().subList(antesT, Sdk.transforms().size());
                Map<String, Object> declarado = Sdk.m(Sdk.l(ts.get(0)).get(1));
                exige(Sdk.l(declarado.get("inputs")).equals(List.of("ventas.archivo.contratos")) && "ventas.archivo.paginas".equals(declarado.get("output")), "declarado " + declarado);
                return "el build de Paginas.java (la celda de builds.rs, por el kernel del agente): lee una colección, escribe la otra por una "
                    + "transacción confirmada (2 ítems; el 412 de uno, saltado), y el informe del build lo dice";
            });
        } finally {
            ParaElBanco.credencial(null);
        }
        return Sdk.fallos - antes;
    }
}
