package conformidad;

import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.time.Instant;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Base64;
import java.util.HashSet;
import java.util.HexFormat;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.function.Supplier;
import java.util.regex.Pattern;

import ore.Media;
import ore.Ore;
import ore.ParaElBanco;

/**
 * JM1 · LEER: las `op` `list`, `stat`, `open`, `url` y `sesion` de la suite,
 * con el SDK de Java. Cada caso corre en las colecciones de su `en`, sobre la
 * muestra recién instalada.
 */
final class Lectura {
    private Lectura() {}

    static void registrar(Map<String, Ejecutor.Op> ops) {
        ops.put("list", Lectura::list);
        ops.put("stat", Lectura::stat);
        ops.put("open", Lectura::open);
        ops.put("url", Lectura::url);
        ops.put("sesion", Lectura::sesion);
    }

    // ── el contexto de un caso: la muestra, lo de `antes`, los huecos ───────

    static final class Ctx {
        final String en, col;
        final Media.Collection c;
        final Map<String, String> huecos = new LinkedHashMap<>();
        final List<String> medidas = new ArrayList<>();

        Ctx(String en) {
            this.en = en;
            this.col = "conformidad.default." + en;
            this.c = Ore.collection(col);
            huecos.put("{coleccion}", col);
        }

        String r(Object o) throws Exception {
            if (o == null) return null;
            String s = String.valueOf(o);
            for (Map.Entry<String, String> h : huecos.entrySet()) s = s.replace(h.getKey(), h.getValue());
            java.util.regex.Matcher m = Pattern.compile("\\{version:([^:}]+):([^}]+)}").matcher(s);
            StringBuilder b = new StringBuilder();
            while (m.find()) {
                Object v = Banco.mando("version", "coleccion", col, "path", m.group(1), "id", m.group(2)).get("version");
                m.appendReplacement(b, java.util.regex.Matcher.quoteReplacement(String.valueOf(v)));
            }
            m.appendTail(b);
            return b.toString();
        }

        void mide(String nombre, Object valor) { medidas.add(nombre + "=" + valor); }
    }

    @SuppressWarnings("unchecked")
    static Map<String, Object> m(Object o) { return o instanceof Map<?, ?> x ? (Map<String, Object>) x : Map.of(); }

    @SuppressWarnings("unchecked")
    static List<Object> l(Object o) { return o instanceof List<?> x ? (List<Object>) x : List.of(); }

    // ── JM2 · `antes.ambito`: el caso corre dentro de un transform ──────────

    /** Lo que sale de la salida de un caso con ámbito: nunca es una de sus entradas. */
    static final String SALIDA = "conformidad.default.salida";

    /** Con {@code antes.ambito.transform}, el caso dentro de {@code Ore.transform(…)} con sus {@code entradas}. */
    static String enSuAmbito(Map<String, Object> caso, java.util.concurrent.Callable<String> cuerpo) throws Exception {
        Map<String, Object> ambito = m(m(caso.get("antes")).get("ambito"));
        if (!Boolean.TRUE.equals(ambito.get("transform"))) return cuerpo.call();
        List<String> entradas = new ArrayList<>();
        for (Object e : l(ambito.get("entradas"))) entradas.add(String.valueOf(e));
        Banco.mando("registro", "limpiar", true);
        return Ore.transform("conformidad", entradas, SALIDA, cuerpo);
    }

    /** Lo que el servidor vio de un caso con ámbito: que no se le preguntó (una no declarada) y lo que consta en el linaje. */
    @SuppressWarnings("unchecked")
    static void despuesDelAmbito(Map<String, Object> caso, String en) throws Exception {
        Map<String, Object> ambito = m(m(caso.get("antes")).get("ambito"));
        if (!Boolean.TRUE.equals(ambito.get("transform"))) return;
        Map<String, Object> espera = m(caso.get("espera"));
        String col = "conformidad.default." + en, corto = "conformidad." + en;
        if ("media/no-declarada".equals(espera.get("error"))) {
            // «Sin preguntar»: el SDK lo dice antes del 403 del servidor.
            for (Object r : l(Banco.mando("registro").get("serve")))
                exige(!String.valueOf(l(r).get(1)).startsWith("/media/conformidad/default/" + en),
                    "[" + en + "] una colección no declarada llegó a la celda: " + l(r).get(1));
        }
        Map<String, Object> linaje = m(espera.get("linaje_incluye"));
        if (!linaje.isEmpty()) {
            // El linaje lo escribe el servidor (B4·4) con lo que fijó al declarar
            // (B4·2): lo del SDK es haberlo declarado, por su nombre.
            Map<String, Object> t = Banco.mando("transforms");
            Map<String, Object> declarado = null;
            for (Object x : l(t.get("transforms"))) if ("POST".equals(l(x).get(0))) declarado = m(l(x).get(1));
            exige(declarado != null && l(declarado.get("inputs")).contains(corto), "[" + en + "] el transform no declaró `" + corto + "`: " + declarado);
            Object asOf = m(t.get("fijadas")).get(corto);
            String quiere = String.valueOf(linaje.get("as_of"));
            exige(String.valueOf(linaje.get("coleccion")).replace("{coleccion}", col).equals(col), "[" + en + "] linaje de otra colección");
            exige(asOf != null && (quiere.equals("*") || quiere.equals(asOf)), "[" + en + "] la celda no fijó `" + corto + "`: " + t.get("fijadas"));
        }
    }

    static Ctx preparar(Map<String, Object> caso, String en) throws Exception {
        Map<String, Object> antes = m(caso.get("antes"));
        Banco.mando("instalar");
        ParaElBanco.credencial(null);
        Ctx x = new Ctx(en);
        if (antes.get("credencial_dura_s") instanceof Number n) {
            Banco.mando("credencial", "dura_s", n);
            ParaElBanco.credencial(proveedor());
        }
        if ("sin-concesion-sobre-la-coleccion".equals(antes.get("credencial"))) Banco.mando("credencial", "modo", "sin-concesion");
        Map<String, Object> sob = m(antes.get("sobrescribir"));
        if (antes.get("recordar_as_of") != null) {
            Map<String, Object> a = Banco.mando("as_of", "coleccion", x.col, "path", sob.get("path"));
            x.huecos.put("{" + antes.get("recordar_as_of") + "}", String.valueOf(a.get("as_of")));
            x.huecos.put("{version_en_" + antes.get("recordar_as_of") + "}", String.valueOf(a.get("version")));
        }
        if (!sob.isEmpty())
            Banco.mando("sobrescribir", "coleccion", x.col, "path", sob.get("path"), "texto", sob.get("texto"),
                "ingerir", !Boolean.FALSE.equals(antes.get("ingerir")));
        return x;
    }

    /** Como el agente (D3): la credencial, renovada cuando le queda menos de un tercio. */
    static Supplier<Map<String, String>> proveedor() {
        return new Supplier<>() {
            String token;
            long caduca, margen;

            @Override public synchronized Map<String, String> get() {
                long ahora = System.currentTimeMillis();
                if (token == null || ahora > caduca - margen) {
                    try {
                        Map<String, Object> t = Banco.mando("token");
                        token = String.valueOf(t.get("token"));
                        long dura = (long) (((Number) t.get("expires_in")).doubleValue() * 1000);
                        caduca = ahora + dura;
                        margen = dura / 3;
                    } catch (Exception e) {
                        throw new IllegalStateException(e);
                    }
                }
                return Map.of("authorization", "Bearer " + token);
            }
        };
    }

    // ── lo que se espera ────────────────────────────────────────────────────

    static void exige(boolean cierto, String porque) { if (!cierto) throw new AssertionError(porque); }

    static String sha(byte[] b) throws Exception {
        return HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(b));
    }

    /** {@code bytes: {de, rango?, version?}} → lo que la muestra dice. */
    static byte[] esperados(Map<String, Object> b) {
        byte[] d = Banco.bytes(String.valueOf(b.get("de")), b.get("version") == null ? null : String.valueOf(b.get("version")));
        List<Object> rango = l(b.get("rango"));
        if (!rango.isEmpty())
            d = Arrays.copyOfRange(d, ((Number) rango.get(0)).intValue(), ((Number) rango.get(1)).intValue() + 1);
        return d;
    }

    /** {@code error} y {@code status}, de una excepción del SDK (o de que no la hubo). */
    static void errorEsperado(Ctx x, Map<String, Object> espera, Throwable e) {
        Object tipo = espera.get("error");
        Object st = espera.get("status");
        if (tipo == null) {
            if (e != null) throw new AssertionError("[" + x.en + "] no esperaba error y dio " + e);
            return;
        }
        exige(e instanceof Media.MediaError, "[" + x.en + "] esperaba `" + tipo + "` y dio " + (e == null ? "nada" : e));
        Media.MediaError me = (Media.MediaError) e;
        exige(tipo.equals(me.type), "[" + x.en + "] esperaba `" + tipo + "` y dio `" + me.type + "`");
        Class<?> clase = switch (String.valueOf(tipo)) {
            case "media/no-existe" -> Media.MediaNotFound.class;
            case "media/sin-permiso", "media/no-declarada" -> Media.MediaForbidden.class;
            case "media/cambiado" -> Media.MediaChanged.class;
            case "media/corrupto", "media/digest-no-casa" -> Media.MediaCorrupt.class;
            case "media/no-escribible" -> Media.MediaNotWritable.class;
            default -> Media.MediaError.class;
        };
        exige(clase.isInstance(me), "[" + x.en + "] `" + tipo + "` llega como " + me.getClass().getSimpleName() + " y no " + clase.getSimpleName());
        if (st instanceof Number n) exige(me.status == n.intValue(), "[" + x.en + "] status " + me.status + " y no " + n);
    }

    /** {@code ref}: cada campo con ese valor; {@code "*"} presente, {@code null} nulo; un {@code *} dentro, comodín. */
    static void ref(Ctx x, Map<String, Object> esperado, Media.MediaRef ref) throws Exception {
        Map<String, Object> visto = ref.toJson();
        for (Map.Entry<String, Object> e : esperado.entrySet()) {
            Object v = visto.get(e.getKey());
            if (e.getValue() == null) {
                exige(v == null, "[" + x.en + "] ref." + e.getKey() + " es " + v + " y no nulo");
                continue;
            }
            String quiero = x.r(e.getValue());
            if (quiero.equals("*")) {
                exige(v != null, "[" + x.en + "] ref." + e.getKey() + " falta");
            } else if (quiero.contains("*")) {
                String re = Arrays.stream(quiero.split("\\*", -1)).map(Pattern::quote).reduce((a, b) -> a + ".+" + b).orElse("");
                exige(v != null && String.valueOf(v).matches(re), "[" + x.en + "] ref." + e.getKey() + " es " + v + " y no " + quiero);
            } else {
                exige(quiero.equals(String.valueOf(v)), "[" + x.en + "] ref." + e.getKey() + " es " + v + " y no " + quiero);
            }
        }
    }

    static void sinCadenas(Ctx x, List<Object> cadenas, String texto) {
        for (Object c : cadenas) exige(!texto.contains(String.valueOf(c)), "[" + x.en + "] la respuesta dice `" + c + "`");
    }

    static void sinClaves(Ctx x, List<Object> claves, Object o) {
        if (o instanceof Map<?, ?> mp) {
            for (Map.Entry<?, ?> e : mp.entrySet()) {
                exige(!claves.contains(e.getKey()), "[" + x.en + "] la respuesta trae la clave `" + e.getKey() + "`");
                sinClaves(x, claves, e.getValue());
            }
        } else if (o instanceof List<?> ls) for (Object i : ls) sinClaves(x, claves, i);
    }

    static String mediana(List<Long> ms, double p) {
        List<Long> s = new ArrayList<>(ms);
        s.sort(null);
        return String.valueOf(s.get(Math.min(s.size() - 1, (int) Math.floor(p * (s.size() - 1) + 0.5))));
    }

    // ── list ────────────────────────────────────────────────────────────────

    static String list(Map<String, Object> caso, String en) throws Exception {
        Ctx x = preparar(caso, en);
        Map<String, Object> pide = m(caso.get("pide")), espera = m(caso.get("espera"));
        int limite = pide.get("limit") instanceof Number n ? n.intValue() : Media.LIMIT;
        String prefijo = x.r(pide.get("prefix")), asOf = x.r(pide.get("as_of"));
        if (espera.containsKey("sin_claves") || espera.containsKey("sin_cadenas")) {
            // De la celda tal cual: lo que el SDK descarta no prueba nada.
            Banco.Crudo r = Banco.crudo("GET", "/media/conformidad/default/" + en + "/items?limit=" + limite, null);
            sinClaves(x, l(espera.get("sin_claves")), ore.Json.leer(r.cuerpo()));
            sinCadenas(x, l(espera.get("sin_cadenas")), r.todo());
        }
        List<Media.Page> paginas = new ArrayList<>();
        List<Long> ms = new ArrayList<>();
        Throwable error = null;
        long t0 = System.nanoTime();
        try {
            var it = x.c.pages(prefijo, null, limite, asOf).iterator();
            while (it.hasNext()) {
                long t = System.nanoTime();
                paginas.add(it.next());
                ms.add((System.nanoTime() - t) / 1_000_000);
                if (!Boolean.TRUE.equals(pide.get("recorrer"))) break;
            }
        } catch (Media.MediaError e) {
            error = e;
        }
        long total = (System.nanoTime() - t0) / 1_000_000;
        errorEsperado(x, espera, error);
        if (error != null) return null;
        List<Media.Item> items = new ArrayList<>();
        for (Media.Page p : paginas) items.addAll(p.items());
        if (espera.get("paths") instanceof List<?> ps) {
            Set<String> vistas = new HashSet<>();
            for (Media.Item i : items) vistas.add(i.ref().path());
            exige(vistas.equals(new HashSet<>(ps.stream().map(String::valueOf).toList())), "[" + en + "] paths " + vistas + " y no " + ps);
        }
        if (!m(espera.get("ref")).isEmpty()) {
            exige(!items.isEmpty(), "[" + en + "] el listado vino vacío");
            ref(x, m(espera.get("ref")), items.get(0).ref());
        }
        if (espera.get("digest_de") != null) {
            String quiero = "sha256:" + sha(Banco.bytes(String.valueOf(espera.get("digest_de")), null));
            exige(!items.isEmpty() && quiero.equals(items.get(0).ref().digest()), "[" + en + "] digest " + (items.isEmpty() ? "-" : items.get(0).ref().digest()) + " y no " + quiero);
        }
        for (Object inv : l(espera.get("invariante"))) {
            switch (String.valueOf(inv)) {
                case "recorrido_completo" -> {
                    Set<String> unicas = new HashSet<>();
                    for (Media.Item i : items) unicas.add(i.ref().path());
                    exige(unicas.size() == items.size(), "[" + en + "] el recorrido repite " + (items.size() - unicas.size()));
                    exige(items.size() == Banco.total(), "[" + en + "] el recorrido dio " + items.size() + " y la muestra tiene " + Banco.total());
                }
                case "una_transaccion" -> {
                    Set<String> txs = new HashSet<>();
                    for (Media.Page p : paginas) txs.add(p.asOf());
                    exige(txs.size() == 1, "[" + en + "] las páginas leyeron " + txs);
                }
                case "coste_por_pagina_estable" -> {
                    long primera = ms.get(0), ultima = ms.get(ms.size() - 1);
                    x.mide("ms_primera_pagina", primera);
                    x.mide("ms_ultima_pagina", ultima);
                    // Por debajo de 25 ms es ruido del reloj, no coste.
                    exige(ultima <= 2 * Math.max(primera, 25), "[" + en + "] la última página tardó " + ultima + " ms y la primera " + primera);
                }
                default -> throw new AssertionError("invariante `" + inv + "` desconocida");
            }
        }
        for (Object md : medidas(espera)) {
            switch (String.valueOf(md)) {
                case "ms_primera_pagina" -> x.mide("ms_primera_pagina", ms.get(0));
                case "ms_ultima_pagina" -> x.mide("ms_ultima_pagina", ms.get(ms.size() - 1));
                case "ms_recorrido" -> x.mide("ms_recorrido", total);
                default -> x.mide(String.valueOf(md), "?");
            }
        }
        return resumen(x);
    }

    static List<Object> medidas(Map<String, Object> espera) {
        Object md = espera.get("mide");
        return md == null ? List.of() : md instanceof List<?> ls ? new ArrayList<>(ls) : List.of(md);
    }

    static String resumen(Ctx x) { return x.medidas.isEmpty() ? null : String.join(" ", x.medidas); }

    // ── stat ────────────────────────────────────────────────────────────────

    static String stat(Map<String, Object> caso, String en) throws Exception {
        Ctx x = preparar(caso, en);
        Map<String, Object> pide = m(caso.get("pide")), espera = m(caso.get("espera"));
        String path = x.r(pide.get("path")), version = x.r(pide.get("version"));
        Media.Item it = null;
        Throwable error = null;
        try {
            it = x.c.stat(path, version);
        } catch (Media.MediaError e) {
            error = e;
        }
        errorEsperado(x, espera, error);
        if (espera.containsKey("cabeceras") || espera.containsKey("sin_cadenas")) {
            String q = "?path=" + java.net.URLEncoder.encode(path, StandardCharsets.UTF_8);
            Banco.Crudo r = Banco.crudo("GET", "/media/conformidad/default/" + en + "/item" + q, null);
            for (Object c : l(espera.get("cabeceras"))) {
                String[] kv = String.valueOf(c).split(":\\s*", 2);
                List<String> vs = r.cabeceras().get(kv[0].toLowerCase());
                exige(vs != null && (kv.length == 1 || vs.stream().anyMatch(v -> v.startsWith(kv[1]))), "[" + en + "] falta la cabecera `" + c + "`: " + r.cabeceras().keySet());
            }
            sinCadenas(x, l(espera.get("sin_cadenas")), r.todo());
        }
        if (error != null) return null;
        if (espera.get("status") instanceof Number n) exige(n.intValue() == 200, "[" + en + "] esperaba " + n + " y dio 200");
        if (espera.containsKey("current")) exige(espera.get("current").equals(it.current()), "[" + en + "] current " + it.current() + " y no " + espera.get("current"));
        if (!m(espera.get("ref")).isEmpty()) ref(x, m(espera.get("ref")), it.ref());
        return null;
    }

    // ── open ────────────────────────────────────────────────────────────────

    static byte[] leerTodo(Media.MediaChannel ch) throws IOException {
        java.io.ByteArrayOutputStream b = new java.io.ByteArrayOutputStream();
        ByteBuffer buf = ByteBuffer.allocate(1 << 16);
        while (ch.read(buf.clear()) >= 0) b.write(buf.array(), 0, buf.position());
        return b.toByteArray();
    }

    static String open(Map<String, Object> caso, String en) throws Exception {
        Ctx x = preparar(caso, en);
        Map<String, Object> pide = m(caso.get("pide")), espera = m(caso.get("espera"));
        if (pide.get("n") instanceof Number n) return muchos(x, pide, espera, n.intValue());
        String path = x.r(pide.get("path")), version = x.r(pide.get("version"));
        if (pide.get("repetir") instanceof Number n) return repetir(x, path, n.intValue());
        Object lento = pide.get("lento");
        if (lento != null)
            Banco.mando("lento", "coleccion", x.col, "path", path, "total_s", lento instanceof Map<?, ?> lm ? m(lm).get("total_s") : 10);
        Media.Item it = null;
        Media.MediaChannel ch = null;
        byte[] leidos = null;
        Throwable error = null;
        try {
            it = x.c.stat(path, version);
            ch = it.open();
            List<Object> rango = l(pide.get("range"));
            if (!rango.isEmpty()) {
                // El `read_range` del contrato: pide su Range siempre, y el SDK no
                // acepta un 200 por respuesta (`media/sin-rangos`).
                long a = ((Number) rango.get(0)).longValue(), z = ((Number) rango.get(1)).longValue();
                leidos = ch.readRange(a, (int) (z - a + 1));
            } else if (pide.get("leer") instanceof Number n) {
                ByteBuffer b = ByteBuffer.allocate(n.intValue());
                while (b.hasRemaining() && ch.read(b) >= 0) { }
                if (Boolean.TRUE.equals(pide.get("cerrar"))) ch.close();
                Thread.sleep(300);   // lo que el banco alcance a mandar tras el cierre
                Object env = Banco.mando("enviados", "coleccion", x.col, "path", path).get("enviados");
                x.mide("bytes_transferidos_al_cerrar_a_medias", env + "/" + Banco.bytes(path, null).length);
            } else if (!m(caso.get("durante")).isEmpty()) {
                Map<String, Object> durante = m(caso.get("durante"));
                ByteBuffer uno = ByteBuffer.allocate(1);
                ch.read(uno);
                Map<String, Object> sob = m(durante.get("sobrescribir"));
                if (!sob.isEmpty()) Banco.mando("sobrescribir", "coleccion", x.col, "path", sob.get("path"), "texto", sob.get("texto"), "ingerir", true);
                if (Boolean.TRUE.equals(durante.get("borrar_version_leida")))
                    Banco.mando("borrar_version", "coleccion", x.col, "path", path, "version", ch.version());
                leidos = leerTodo(ch);
            } else {
                leidos = leerTodo(ch);
            }
        } catch (Media.MediaError e) {
            error = e;
        } finally {
            if (ch != null) ch.close();
        }
        errorEsperado(x, espera, error);
        if (error != null) return resumen(x);
        if (espera.get("status") instanceof Number n) exige(ch.status() == n.intValue(), "[" + en + "] status " + ch.status() + " y no " + n);
        if (!m(espera.get("bytes")).isEmpty()) {
            Map<String, Object> b = new LinkedHashMap<>(m(espera.get("bytes")));
            exige(Arrays.equals(esperados(b), leidos), "[" + en + "] los bytes no son los de " + b + ": " + new String(leidos, StandardCharsets.UTF_8));
        }
        for (Object c : l(espera.get("cabeceras"))) {
            String v = switch (String.valueOf(c)) {
                case "ETag" -> ch.etag();
                case "ORE-Media-Version" -> ch.version();
                case "Repr-Digest" -> ch.reprDigest();
                default -> throw new AssertionError("cabecera `" + c + "` desconocida");
            };
            exige(v != null, "[" + en + "] la respuesta no trajo `" + c + "`");
        }
        if (espera.get("repr_digest_de") != null) {
            String quiero = "sha-256=:" + Base64.getEncoder().encodeToString(MessageDigest.getInstance("SHA-256").digest(Banco.bytes(String.valueOf(espera.get("repr_digest_de")), null))) + ":";
            exige(quiero.equals(ch.reprDigest()), "[" + en + "] Repr-Digest " + ch.reprDigest() + " y no " + quiero);
        }
        for (Object inv : l(espera.get("invariante"))) {
            if (!"version_fijada".equals(inv)) throw new AssertionError("invariante `" + inv + "` desconocida");
            String actual = String.valueOf(Banco.mando("as_of", "coleccion", x.col, "path", path).get("version"));
            exige(actual.equals(ch.version()) && actual.equals(it.ref().version()),
                "[" + en + "] ORE-Media-Version " + ch.version() + ", fijada " + it.ref().version() + ", la de los bytes " + actual);
        }
        Map<String, Object> despues = m(caso.get("despues"));
        Media.Item luego = despues.isEmpty() ? it : x.c.stat(x.r(m(despues.get("pide")).get("path")));
        if (espera.get("digest_de") != null) {
            String quiero = "sha256:" + sha(Banco.bytes(String.valueOf(espera.get("digest_de")), null));
            exige(quiero.equals(luego.ref().digest()), "[" + en + "] digest " + luego.ref().digest() + " y no " + quiero);
        }
        return resumen(x);
    }

    static String repetir(Ctx x, String path, int veces) throws Exception {
        List<Long> ms = new ArrayList<>();
        for (int i = 0; i < veces; i++) {
            long t = System.nanoTime();
            try (Media.MediaChannel ch = x.c.stat(path).open()) {
                ch.read(ByteBuffer.allocate(1));
            }
            ms.add((System.nanoTime() - t) / 1_000_000);
        }
        x.mide("ms_hasta_el_primer_byte_p50", mediana(ms, 0.5));
        x.mide("ms_hasta_el_primer_byte_p95", mediana(ms, 0.95));
        return resumen(x);
    }

    static String muchos(Ctx x, Map<String, Object> pide, Map<String, Object> espera, int n) throws Exception {
        List<Media.Item> items = new ArrayList<>();
        for (Media.Item i : x.c.items(x.r(pide.get("prefix")))) {
            if (items.size() >= n) break;
            items.add(i);
        }
        int hilos = pide.get("concurrencia") instanceof Number c ? c.intValue() : 16, errores = 0, bien = 0;
        long t = System.nanoTime();
        for (Media.Result r : Ore.readMany(items, hilos)) {
            if (r.ok()) {
                exige(Arrays.equals(r.data(), Banco.bytes(r.item().ref().path(), null)), "[" + x.en + "] los bytes de " + r.item().ref().path());
                bien++;
            } else {
                if (errores == 0) x.mide("primer_error", "«" + r.error() + "»");
                errores++;
            }
        }
        double s = (System.nanoTime() - t) / 1e9;
        exige(bien + errores == items.size(), "[" + x.en + "] readMany dio " + (bien + errores) + " de " + items.size());
        x.mide("items_por_s", Math.round(items.size() / s));
        x.mide("errores", errores);
        return resumen(x);
    }

    // ── url ─────────────────────────────────────────────────────────────────

    static String url(Map<String, Object> caso, String en) throws Exception {
        Ctx x = preparar(caso, en);
        Map<String, Object> pide = m(caso.get("pide")), espera = m(caso.get("espera"));
        List<Object> paths = new ArrayList<>();
        for (Object o : l(pide.get("items"))) paths.add(x.r(m(o).get("path")));
        Instant antes = Instant.now();
        List<Media.Url> us = x.c.urls(paths, pide.get("ttl_s") instanceof Number t ? t.intValue() : null);
        exige(us.size() == paths.size(), "[" + en + "] " + us.size() + " URLs para " + paths.size() + " ítems");
        if (m(espera.get("caduca_en_s")).get("max") instanceof Number max) {
            for (Media.Url u : us) {
                exige(u.ok(), "[" + en + "] " + u.error());
                long s = u.expiresAt().getEpochSecond() - antes.getEpochSecond();
                exige(s <= max.longValue() + 1, "[" + en + "] caduca en " + s + " s, más que " + max);
            }
        }
        Map<String, Object> errores = m(espera.get("errores_en"));
        for (int i = 0; i < us.size(); i++) {
            Media.Url u = us.get(i);
            Object tipo = errores.get(String.valueOf(i));
            if (tipo != null) {
                exige(!u.ok() && tipo.equals(u.error().type), "[" + en + "] la posición " + i + " esperaba `" + tipo + "` y dio " + (u.ok() ? u.url() : u.error().type));
                exige(u.error() instanceof Media.MediaNotFound || !"media/no-existe".equals(tipo), "[" + en + "] `" + tipo + "` no es MediaNotFound");
            } else if (l(espera.get("invariante")).contains("lote_parcial")) {
                exige(u.ok() && paths.get(i).equals(u.item().path()), "[" + en + "] la posición " + i + " no es la URL de " + paths.get(i) + ": " + (u.ok() ? u.item().path() : u.error()));
                try (var in = java.net.URI.create(u.url()).toURL().openStream()) {
                    exige(Arrays.equals(in.readAllBytes(), Banco.bytes(String.valueOf(paths.get(i)), null)), "[" + en + "] la URL de " + paths.get(i) + " no da sus bytes");
                }
            }
        }
        return null;
    }

    // ── sesion: la credencial que caduca a mitad de una celda ───────────────

    static String sesion(Map<String, Object> caso, String en) throws Exception {
        Ctx x = preparar(caso, en);
        Map<String, Object> pide = m(caso.get("pide"));
        for (Object o : l(pide.get("celda"))) {
            Map<String, Object> paso = m(o);
            if (paso.get("esperar_s") instanceof Number n) {
                Banco.dormir(n.doubleValue());
                continue;
            }
            switch (String.valueOf(paso.get("op"))) {
                case "list" -> x.c.pages(null, null, ((Number) paso.getOrDefault("limit", Media.LIMIT)).intValue(), null).iterator().next();
                case "stat" -> x.c.stat(x.r(paso.get("path")));
                default -> throw new AssertionError("paso `" + paso + "` desconocido");
            }
        }
        return null;
    }
}
