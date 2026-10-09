package conformidad;

import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;

import ore.LagoEnMemoria;
import ore.Media;
import ore.Ore;
import ore.ParaElBanco;

/**
 * JM4b · {@code apply()} EN FILAS: las trece de B5·2 que
 * {@code la-derivacion-en-python.py} prueba en Python, en Java. El listado lo
 * pone el ejecutor en el banco (`listado`); el lago es {@link LagoEnMemoria}
 * (la tabla anclada que {@code apply()} construye, leída de vuelta). Lo que se
 * prueba es la lógica de {@code apply()} —qué se calcula, qué se queda, qué se
 * va— y la tabla anclada que escribe (v1alpha17 {@code 03}).
 */
final class Filas {
    private Filas() {}

    static String COL = "filas.archivo.contratos";
    static final String SAL = "filas.archivo.textos";
    static final List<Map<String, Object>> LISTADO = new ArrayList<>();
    static final List<String> LLAMADAS = Collections.synchronizedList(new ArrayList<>());
    static final LagoEnMemoria LAGO = new LagoEnMemoria();
    static boolean arreglado = false;

    static void exige(boolean cierto, String porque) { if (!cierto) throw new AssertionError(porque); }

    static Map<String, Object> ref(String path, String contenido, String coleccion, boolean virtual) {
        Map<String, Object> r = new LinkedHashMap<>();
        r.put("uri", "ore://" + coleccion + "/" + path + "?v=v1");
        r.put("collection", coleccion);
        r.put("path", path);
        r.put("version", "v1");
        r.put("digest", virtual ? null : "sha256:" + contenido);
        r.put("size", 10);
        r.put("content_type", "application/pdf");
        r.put("checksum", "etag:" + contenido);
        return r;
    }

    static Map<String, Object> ref(String path, String contenido) { return ref(path, contenido, COL, false); }

    static void listar() throws Exception { Banco.mando("listado", "coleccion", COL, "items", LISTADO); }

    /** Dos páginas por contrato; {@code fallar.pdf} falla (hasta que se arregla). */
    static Object paginas(Media.Item item) {
        LLAMADAS.add(item.ref().path());
        if (item.ref().path().equals("fallar.pdf") && !arreglado) throw new IllegalArgumentException("pdf roto");
        List<Map<String, Object>> out = new ArrayList<>();
        for (int p = 1; p <= 2; p++) out.add(Map.of("anchor", Map.of("kind", "page", "page", p), "texto", item.ref().path() + " p" + p));
        return out;
    }

    static Ore.Result aplicar(Media.Apply o) throws Exception {
        LLAMADAS.clear();
        listar();
        return Ore.collection(COL).apply(Filas::paginas, o.output(SAL).name("paginas"));
    }

    static Ore.Result aplicar() throws Exception { return aplicar(Media.applying().version("1")); }

    static List<Map<String, Object>> filas() { return LAGO.tablas.get(SAL); }

    @SuppressWarnings("unchecked")
    static Map<String, Object> m(Object o) { return (Map<String, Object>) o; }

    static long n(Object o) { return ((Number) o).longValue(); }

    static Set<String> rutas() {
        Set<String> s = new HashSet<>();
        for (Map<String, Object> f : filas()) s.add(String.valueOf(m(f.get("_item")).get("path")));
        return s;
    }

    static int correr() throws Exception {
        System.out.println("la derivación en filas, en java");
        int antes = Sdk.fallos;
        LagoEnMemoria.usar(LAGO);
        ParaElBanco.credencial(() -> Map.of("authorization", Sdk.TOKEN));
        try {
            Sdk.caso("filas", 1, () -> {
                LISTADO.clear();
                LISTADO.addAll(List.of(ref("a.pdf", "aa"), ref("b.pdf", "bb"), ref("c.pdf", "cc")));
                Ore.Result r = aplicar();
                exige(n(r.get("items")) == 3 && n(r.get("new")) == 3 && n(r.get("recomputed")) == 0 && n(r.get("skipped")) == 0
                    && n(r.get("errors")) == 0 && n(r.get("removed")) == 0 && n(r.get("rows")) == 6 && Boolean.TRUE.equals(r.get("written")), "resumen " + r);
                exige(LAGO.escrituras.get(LAGO.escrituras.size() - 1).equals(List.of(SAL, 6L, COL)), "escrituras " + LAGO.escrituras);
                List<String> cols = LAGO.esquemas.get(SAL).stream().map(f -> f.getName()).toList();
                exige(cols.subList(0, 6).equals(List.of("_item", "_anchor", "_anchor_id", "_anchor_parent", "_derivation", "_status")), "columnas " + cols);
                var creado = LAGO.esquemas.get(SAL).get(4).getChildren().stream().filter(f -> f.getName().equals("created")).findFirst().orElseThrow();
                exige(creado.getType().toString().equals("Timestamp(MICROSECOND, UTC)"), "created es " + creado.getType());
                Map<String, Object> f = filas().get(0);
                exige("page".equals(m(f.get("_anchor")).get("kind")) && Set.of(1L, 2L).contains(m(f.get("_anchor")).get("page")) && m(f.get("_anchor")).get("bbox") == null, "ancla " + f);
                exige(String.valueOf(m(f.get("_item")).get("digest")).startsWith("sha256:") && "ok".equals(m(f.get("_status")).get("state")) && n(m(f.get("_status")).get("attempts")) == 1, "fila " + f);
                exige("paginas".equals(m(f.get("_derivation")).get("fn")) && "1".equals(m(f.get("_derivation")).get("fn_version")), "derivación " + f.get("_derivation"));
                Set<Object> ids = new HashSet<>();
                for (Map<String, Object> x : filas()) ids.add(x.get("_anchor_id"));
                exige(ids.size() == 6, "anchor ids " + ids.size());
                // El esquema de Iceberg lleva lo anidado con su forma (structs, la lista de puntos).
                Object ice = LAGO.iceberg.get(SAL).get(1);
                exige(ice instanceof Map<?, ?> im && "struct".equals(im.get("type")) && String.valueOf(im).contains("type=list"), "iceberg de _anchor " + ice);
                return "primera vez: 3 ítems, 6 filas, tabla anclada a la colección con sus seis columnas de sistema (y su esquema de Iceberg anidado)";
            });
            Sdk.caso("filas", 2, () -> {
                int antesE = LAGO.escrituras.size();
                Ore.Result r = aplicar();
                exige(n(r.get("skipped")) == 3 && n(r.get("new")) == 0 && !Boolean.TRUE.equals(r.get("written")) && n(r.get("rows")) == 6, "resumen " + r);
                exige(LAGO.escrituras.size() == antesE && LLAMADAS.isEmpty(), "escribió o calculó: " + LLAMADAS);
                return "otra vez sin cambios: 0 calculados, y no se escribe";
            });
            Sdk.caso("filas", 3, () -> {
                LISTADO.add(ref("d.pdf", "dd"));
                Ore.Result r = aplicar();
                exige(n(r.get("new")) == 1 && n(r.get("skipped")) == 3 && n(r.get("rows")) == 8 && LLAMADAS.equals(List.of("d.pdf")), r + " " + LLAMADAS);
                return "un ítem nuevo: sólo ése (d.pdf), y la tabla tiene sus 8 filas";
            });
            Sdk.caso("filas", 4, () -> {
                Map<String, Object> antesIds = new HashMap<>();
                for (Map<String, Object> x : filas()) antesIds.put(m(x.get("_item")).get("path") + "#" + m(x.get("_anchor")).get("page"), x.get("_anchor_id"));
                Ore.Result r = aplicar(Media.applying().version("2"));
                List<String> ll = new ArrayList<>(LLAMADAS);
                Collections.sort(ll);
                exige(n(r.get("recomputed")) == 4 && n(r.get("skipped")) == 0 && ll.equals(List.of("a.pdf", "b.pdf", "c.pdf", "d.pdf")), r + " " + ll);
                Map<String, Object> despues = new HashMap<>();
                Set<Object> versiones = new HashSet<>();
                for (Map<String, Object> x : filas()) {
                    despues.put(m(x.get("_item")).get("path") + "#" + m(x.get("_anchor")).get("page"), x.get("_anchor_id"));
                    versiones.add(m(x.get("_derivation")).get("fn_version"));
                }
                exige(antesIds.equals(despues) && versiones.equals(Set.of("2")), "ids o versiones cambiaron: " + versiones);
                return "otra `version`: los 4 recalculados; los `_anchor_id` son los mismos (no dependen de la versión)";
            });
            Sdk.caso("filas", 5, () -> {
                LISTADO.add(ref("fallar.pdf", "ff"));
                Ore.Result r = aplicar(Media.applying().version("2"));
                exige(n(r.get("errors")) == 1 && n(r.get("skipped")) == 4 && Boolean.TRUE.equals(r.get("written")), "resumen " + r);
                List<Map<String, Object>> e = filas().stream().filter(x -> "fallar.pdf".equals(m(x.get("_item")).get("path"))).toList();
                exige(e.size() == 1 && "error".equals(m(e.get(0).get("_status")).get("state"))
                    && "IllegalArgumentException".equals(m(e.get(0).get("_status")).get("error_type")), "fila de error " + e);
                exige("pdf roto".equals(m(e.get(0).get("_status")).get("error_message")) && "item".equals(m(e.get(0).get("_anchor")).get("kind")), "fila " + e);
                return "un error es un resultado: una fila `error` (IllegalArgumentException: pdf roto, ancla item), y los otros 4 siguen";
            });
            Sdk.caso("filas", 6, () -> {
                Ore.Result r = aplicar(Media.applying().version("2"));
                exige(n(r.get("skipped")) == 5 && !Boolean.TRUE.equals(r.get("written")) && LLAMADAS.isEmpty(), r + " " + LLAMADAS);
                arreglado = true;
                try {
                    r = aplicar(Media.applying().version("2").retryErrors(true));
                } finally {
                    arreglado = false;
                }
                exige(LLAMADAS.equals(List.of("fallar.pdf")) && n(r.get("recomputed")) == 1 && n(r.get("errors")) == 0, r + " " + LLAMADAS);
                List<Map<String, Object>> e = filas().stream().filter(x -> "fallar.pdf".equals(m(x.get("_item")).get("path"))).toList();
                exige(e.size() == 2 && e.stream().allMatch(x -> "ok".equals(m(x.get("_status")).get("state"))) && n(m(e.get(0).get("_status")).get("attempts")) == 2, "filas " + e);
                return "el error no se reintenta solo; con retryErrors(true) sólo ése, sale bien, intento 2";
            });
            Sdk.caso("filas", 7, () -> {
                LISTADO.set(0, ref("archivo/a-renombrado.pdf", "aa"));
                Ore.Result r = aplicar(Media.applying().version("2"));
                exige(LLAMADAS.isEmpty() && n(r.get("skipped")) == 5 && Boolean.TRUE.equals(r.get("written")), r + " " + LLAMADAS);
                exige(rutas().contains("archivo/a-renombrado.pdf") && !rutas().contains("a.pdf"), "rutas " + rutas());
                return "mover a.pdf sin cambiar su contenido: 0 calculados; sus filas dicen la ruta nueva";
            });
            Sdk.caso("filas", 8, () -> {
                LISTADO.remove(1);   // b.pdf
                Ore.Result r = aplicar(Media.applying().version("2"));
                exige(n(r.get("removed")) == 1 && LLAMADAS.isEmpty() && n(r.get("rows")) == 8, "resumen " + r);
                exige(!rutas().contains("b.pdf"), "rutas " + rutas());
                return "b.pdf ya no está: sus 2 filas se van, sin calcular nada";
            });
            Sdk.caso("filas", 9, () -> {
                String viejo = COL;
                COL = "filas.archivo.virtual";
                try {
                    LISTADO.clear();
                    LISTADO.add(ref("v.pdf", "vv", COL, true));
                    LAGO.tablas.remove(SAL);
                    aplicar();
                    Ore.Result r = aplicar();
                    exige(n(r.get("skipped")) == 1 && !Boolean.TRUE.equals(r.get("written")), "resumen " + r);
                    LISTADO.set(0, ref("movido/v.pdf", "vv", COL, true));
                    r = aplicar();
                    exige(LLAMADAS.equals(List.of("movido/v.pdf")) && n(r.get("removed")) == 1, r + " " + LLAMADAS);
                } finally {
                    COL = viejo;
                    LAGO.tablas.remove(SAL);
                }
                return "una virtual sin digest: su identidad es (colección, ruta, versión); otra vez nada, movida sí recalcula";
            });
            Sdk.caso("filas", 10, () -> {
                LISTADO.clear();
                LISTADO.add(ref("a.pdf", "aa"));
                aplicar(Media.applying().version("1").params(Map.of("idioma", "es")));
                Ore.Result r = aplicar(Media.applying().version("1").params(Map.of("idioma", "es")));
                exige(n(r.get("skipped")) == 1, "resumen " + r);
                r = aplicar(Media.applying().version("1").params(Map.of("idioma", "en")));
                exige(n(r.get("recomputed")) == 1, "resumen " + r);
                Object ph = m(filas().get(0).get("_derivation")).get("params_hash");
                exige(ph != null && String.valueOf(ph).length() == 64, "params_hash " + ph);
                LAGO.tablas.remove(SAL);
                java.util.function.Function<Media.Item, Object> sinVersion = i -> List.of(Map.of("texto", "x"));
                listar();
                Ore.collection(COL).apply(sinVersion, Media.applying().output(SAL));
                String v = String.valueOf(m(filas().get(0).get("_derivation")).get("fn_version"));
                exige(v.startsWith("codigo:") && "item".equals(m(filas().get(0).get("_anchor")).get("kind")), "fila " + filas().get(0));
                r = Ore.collection(COL).apply(sinVersion, Media.applying().output(SAL));
                exige(n(r.get("skipped")) == 1, "resumen " + r);
                return "params entran en la clave (es→en recalcula); sin version, la del código (" + v + "), y es estable";
            });
            Sdk.caso("filas", 11, () -> {
                LAGO.tablas.remove(SAL);
                ParaElBanco.leidas(true);
                listar();
                List<Ore.Result> ab = Ore.transform("textos", List.of(COL), SAL, () -> {
                    Media.Collection contratos = Ore.collection(COL);
                    Ore.Result a = contratos.apply(Filas::paginas, Media.applying().version("1").name("paginas"));
                    Ore.Result b = contratos.apply(Filas::paginas, Media.applying().version("1").name("paginas"));
                    return List.of(a, b);
                });
                exige(n(ab.get(0).get("new")) == 1 && n(ab.get(1).get("skipped")) == 1, "resúmenes " + ab);
                List<Object> ultima = LAGO.escrituras.get(LAGO.escrituras.size() - 1);
                exige(SAL.equals(ultima.get(0)) && COL.equals(ultima.get(2)), "escritura " + ultima);
                exige(!ParaElBanco.leidas(false).contains(SAL), "se anotó la salida como leída: " + ParaElBanco.leidas(false));
                return "dentro de un transform: escribe su output, lo lee para saltar lo hecho, y leerlo no es una entrada";
            });
            Sdk.caso("filas", 12, () -> {
                LAGO.tablas.remove(SAL);
                LISTADO.clear();
                LISTADO.addAll(List.of(ref("a.pdf", "aa"), ref("b.pdf", "bb")));
                listar();
                java.util.function.Function<Media.Item, Object> mala = i -> i.ref().path().equals("a.pdf")
                    ? List.of(Map.of("_status", "x")) : List.of(Map.of("texto", "y", "anchor", Map.of("page", 1)));
                Ore.Result r = Ore.collection(COL).apply(mala, Media.applying().output(SAL).version("1"));
                exige(n(r.get("errors")) == 2, "resumen " + r);
                Map<Object, Object> msg = new HashMap<>();
                for (Map<String, Object> x : filas()) msg.put(m(x.get("_item")).get("path"), m(x.get("_status")).get("error_message"));
                exige(String.valueOf(msg.get("a.pdf")).contains("system columns") && String.valueOf(msg.get("b.pdf")).contains("without `kind`"), "mensajes " + msg);
                return "fn que da una columna `_…` o un ancla sin `kind`: el error de ese ítem, con su porqué";
            });
            Sdk.caso("filas", 13, () -> {
                LAGO.tablas.remove(SAL);
                LISTADO.clear();
                LISTADO.addAll(List.of(ref("copia/a.pdf", "aa"), ref("copia/b.pdf", "bb")));
                aplicar();
                int antesE = LAGO.escrituras.size();
                LISTADO.add(0, ref("copia-b5/a.pdf", "aa"));
                Ore.Result r = aplicar();
                exige(n(r.get("skipped")) == 2 && n(r.get("items")) == 2 && !Boolean.TRUE.equals(r.get("written")) && LLAMADAS.isEmpty(), "resumen " + r);
                exige(LAGO.escrituras.size() == antesE && rutas().equals(Set.of("copia/a.pdf", "copia/b.pdf")), "rutas " + rutas());
                LISTADO.remove(1);
                r = aplicar();
                exige(n(r.get("skipped")) == 2 && Boolean.TRUE.equals(r.get("written")) && LLAMADAS.isEmpty(), "resumen " + r);
                exige(rutas().equals(Set.of("copia-b5/a.pdf", "copia/b.pdf")), "rutas " + rutas());
                return "una copia con otra ruta, listada antes: el mismo ítem, su fila no cambia de ruta y no se escribe; si la de siempre se va, la copia la hereda";
            });
            // ── el write() de verdad: el Preview de Textos.java, con la celda de builds.rs ──
            LagoEnMemoria.usar(null);
            Sdk.caso("filas", 14, () -> {
                java.nio.file.Path f = Banco.celda("textos-preview.jsh");
                exige(java.nio.file.Files.exists(f), "no está la celda generada (" + f + ")");
                Map<String, Object> salida = ParaElBanco.celda(java.nio.file.Files.readString(f));
                exige(!"error".equals(salida.get("tipo")), "la celda dio error: " + ore.Json.escribir(salida));
                Map<String, Object> visto = Lectura.m(Lectura.m(salida.get("informe")).get("preview"));
                List<Object> columnas = Lectura.l(visto.get("columnas"));
                List<String> nombres = new ArrayList<>();
                for (Object c : columnas) nombres.add(String.valueOf(Lectura.m(c).get("name")));
                exige(nombres.equals(List.of("_item", "_anchor", "_anchor_id", "_anchor_parent", "_derivation", "_status", "cabeza")), "columnas " + nombres);
                Object ice = Lectura.m(columnas.get(4)).get("iceberg");
                exige(ice instanceof Map<?, ?> im && "struct".equals(im.get("type")) && String.valueOf(im).contains("timestamptz"), "iceberg de _derivation " + ice);
                exige(Long.valueOf(3).equals(((Number) visto.get("total")).longValue()), "total " + visto.get("total"));
                String todo = ore.Json.escribir(visto.get("filas"));
                exige(todo.contains("%PDF-") && todo.contains("MediaChanged"), "filas " + todo);
                return "el Preview de Textos.java (la celda de builds.rs): apply() en filas por el write() de verdad —el Arrow anidado y su esquema "
                    + "de Iceberg (_derivation: struct con timestamptz)—, 3 filas, el 412 de un ítem es su fila de error, nada se escribe";
            });
        } finally {
            LagoEnMemoria.usar(null);
            ParaElBanco.credencial(null);
        }
        return Sdk.fallos - antes;
    }
}
