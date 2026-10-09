package conformidad;

import java.nio.ByteBuffer;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

import ore.Media;
import ore.Ore;
import ore.ParaElBanco;

/**
 * LA MEDIA EN JAVA · el SDK (0049 JM1): las trece de B3·5 que
 * {@code la-media-en-python.py} prueba en Python, sobre la colección de siempre
 * del banco ({@code legal.archivo.contratos}: a.pdf de 3 MB, el permiso que
 * vale una vez, cambia.pdf que da 412, corrupto y cortado). Las cuatro de B4·3
 * (dentro de un transform) son de JM2.
 *
 * Corre antes que la suite: la prueba 6 cuenta con que el primer permiso de
 * a.pdf es el primero que el banco da.
 */
final class Sdk {
    private Sdk() {}

    static int fallos = 0;
    static long fichasAlEmpezar = -1;
    static final String TOKEN = "Bearer secreto-de-ore";

    interface Prueba { String correr() throws Exception; }

    static void caso(int n, Prueba p) {
        try {
            System.out.println("  ✓ sdk " + n + " · " + p.correr());
        } catch (Throwable e) {
            fallos++;
            System.out.println("  ✗ sdk " + n + " · " + (e instanceof AssertionError ? e.getMessage() : e.toString()));
        }
    }

    static void exige(boolean cierto, String porque) { if (!cierto) throw new AssertionError(porque); }

    @SuppressWarnings("unchecked")
    static List<List<Object>> registro(String de) throws Exception {
        return (List<List<Object>>) Banco.mando("registro").get(de);
    }

    @SuppressWarnings("unchecked")
    static Map<String, Object> cabeceras(List<Object> peticion) { return (Map<String, Object>) peticion.get(2); }

    static Media.MediaRef sinTamano(Media.MediaRef r) {
        return new Media.MediaRef(r.uri(), r.collection(), r.path(), r.version(), r.digest(), null, r.contentType(),
            r.contentTypeDetected(), r.checksum(), r.annotations(), r.modified(), r.state(), r.transaction(), r.source(), r.derivation());
    }

    static Media.MediaRef conDigest(Media.MediaRef r, String d) {
        return new Media.MediaRef(r.uri(), r.collection(), r.path(), r.version(), d, r.size(), r.contentType(),
            r.contentTypeDetected(), r.checksum(), r.annotations(), r.modified(), r.state(), r.transaction(), r.source(), r.derivation());
    }

    static byte[] leer(Media.MediaChannel ch, int n) throws Exception {
        ByteBuffer b = ByteBuffer.allocate(n);
        while (b.hasRemaining() && ch.read(b) >= 0) { }
        return Arrays.copyOf(b.array(), b.position());
    }

    static byte[] resto(Media.MediaChannel ch) throws Exception { return Lectura.leerTodo(ch); }

    static int correr() throws Exception {
        System.out.println("la media en java · el sdk");
        // 13: la rama del puesto, antes de la primera pregunta (la JVM la pregunta una vez por proceso).
        Banco.mando("rama", "rama", "r1/trabajo");
        Banco.mando("registro", "limpiar", true);
        ParaElBanco.credencial(() -> Map.of("authorization", TOKEN));
        byte[] a = Banco.objeto("a.pdf");
        Media.Collection c = Ore.collection("legal.archivo.contratos");
        try {
            caso(1, () -> {
                List<String> ps = new ArrayList<>();
                for (Media.Item i : c.items()) ps.add(i.ref().path());
                exige(ps.equals(List.of("a.pdf", "b.pdf", "cambia.pdf")), "items() dio " + ps);
                List<List<Object>> serve = registro("serve");
                long paginas = serve.stream().filter(r -> String.valueOf(r.get(1)).contains("/items")).count();
                exige(paginas == 2, paginas + " páginas y no 2");
                fichasAlEmpezar = serve.stream().filter(r -> "/puestos/p1".equals(r.get(1))).count();
                return "items(): tres ítems en dos páginas, por cursor (MediaRef ignora lo desconocido)";
            });
            caso(2, () -> {
                Media.Item it = c.stat("a.pdf");
                exige(Boolean.TRUE.equals(it.current()) && it.ref().size() == a.length, "current " + it.current() + ", size " + it.ref().size());
                return "stat(): fresco y actual";
            });
            caso(3, () -> {
                Banco.mando("registro", "limpiar", true);
                Media.Item it = c.stat("a.pdf");
                try (Media.MediaChannel ch = it.open()) {
                    exige(new String(leer(ch, 5)).equals("%PDF-"), "la cabeza");
                    ch.position(ch.size() - 10);
                    byte[] cola = resto(ch);
                    exige(Arrays.equals(cola, Arrays.copyOfRange(a, a.length - 10, a.length)), "la cola");
                    exige(ch.position() == a.length, "position " + ch.position());
                }
                List<Object> rangos = new ArrayList<>();
                for (List<Object> r : registro("bytes")) rangos.add(cabeceras(r).get("range"));
                exige(rangos.contains("bytes=" + (a.length - 10) + "-"), "rangos " + rangos);
                return "open(): read, position() desde el final con un Range (" + rangos.size() + " peticiones de bytes), position";
            });
            caso(4, () -> {
                Media.Item it = c.stat("a.pdf");
                try (Media.MediaChannel ch = it.open()) {
                    exige(Arrays.equals(resto(ch), a), "los bytes");
                }
                exige(Lectura.sha(a).equals(it.sha256Seen()), "sha256Seen " + it.sha256Seen());
                return "una lectura entera se verifica y da el sha256 visto";
            });
            caso(5, () -> {
                List<List<Object>> bytes = registro("bytes"), serve = registro("serve");
                for (List<Object> r : bytes)
                    exige(!cabeceras(r).containsKey("authorization") && !cabeceras(r).containsKey("x-ore-puesto"), "la URL de los bytes recibió el token: " + r);
                for (List<Object> r : serve) exige(TOKEN.equals(cabeceras(r).get("authorization")), "una petición a la celda sin el token: " + r.get(1));
                exige(!serve.isEmpty(), "nada llegó a la celda");
                return "el token de ORE va a la celda (" + serve.size() + ") y nunca a los bytes (" + bytes.size() + ")";
            });
            caso(6, () -> {
                List<String> renovadas = new ArrayList<>();
                for (List<Object> r : registro("serve")) {
                    String ruta = String.valueOf(r.get(1));
                    if (ruta.contains("/content") && ruta.contains("a.pdf")) renovadas.add(ruta);
                }
                exige(renovadas.size() >= 2 && renovadas.stream().allMatch(r -> r.contains("version=v1")), "content " + renovadas);
                return "el primer permiso caducó a la primera: se pidió otro con version=v1 y se siguió";
            });
            caso(7, () -> {
                try {
                    c.stat("cambia.pdf").readBytes();
                } catch (Media.MediaChanged e) {
                    exige(e.status == 412, "status " + e.status);
                    return "la versión ya no está: MediaChanged (412)";
                }
                throw new AssertionError("debía ser MediaChanged");
            });
            caso(8, () -> {
                for (String p : List.of("corrupto.pdf", "cortado.pdf")) {
                    Media.Item it = c.stat(p);
                    if (p.equals("corrupto.pdf")) ParaElBanco.ref(it, conDigest(it.ref(), "sha256:" + "0".repeat(64)));
                    try {
                        it.readBytes();
                        throw new AssertionError(p + " debía ser MediaCorrupt");
                    } catch (Media.MediaCorrupt e) {
                        // bien
                    }
                }
                return "el digest no casa, o el flujo se corta (y se reanuda, y se vuelve a cortar): MediaCorrupt";
            });
            caso(9, () -> {
                ParaElBanco.rangos(50_000, 32_768);
                try {
                    Banco.mando("registro", "limpiar", true);
                    exige(Arrays.equals(c.stat("a.pdf").readBytes(4), a), "los bytes");
                    long rangos = registro("bytes").stream().filter(r -> cabeceras(r).get("range") != null).count();
                    exige(rangos >= a.length / 32_768, rangos + " rangos");
                    return "readBytes() de uno grande: " + rangos + " rangos en paralelo, verificado";
                } finally {
                    ParaElBanco.rangos(32L << 20, 8L << 20);
                }
            });
            caso(10, () -> {
                Map<String, Media.Result> r = new HashMap<>();
                for (Media.Result x : Ore.readMany(c.items(), 4)) r.put(x.item().ref().path(), x);
                exige(Arrays.equals(r.get("a.pdf").data(), a), "a.pdf");
                exige(Arrays.equals(r.get("b.pdf").data(), "%PDF-otro".getBytes()), "b.pdf");
                exige(r.get("cambia.pdf").error() instanceof Media.MediaChanged, "cambia.pdf dio " + r.get("cambia.pdf").error());
                return "readMany(): 3 a la vez, el 412 de uno es un valor";
            });
            caso(11, () -> {
                Banco.mando("contados", "enviados", 0);
                try (Media.MediaChannel ch = c.stat("a.pdf").open()) {
                    leer(ch, 10);
                }
                Thread.sleep(300);
                long env = ((Number) Banco.mando("contados").get("enviados")).longValue();
                exige(env < a.length, "el servidor envió " + env + " de " + a.length);
                return "cerrar a medias: el servidor envió " + env + " de " + a.length + " bytes";
            });
            caso(12, () -> {
                Media.Item it = c.stat("a.pdf");
                ParaElBanco.ref(it, sinTamano(it.ref()));
                try (Media.MediaChannel ch = it.open()) {
                    ch.position(ch.size() - 10);
                    exige(Arrays.equals(resto(ch), Arrays.copyOfRange(a, a.length - 10, a.length)), "la cola sin tamaño");
                }
                it = c.stat("a.pdf");
                ParaElBanco.ref(it, sinTamano(it.ref()));
                try (Media.MediaChannel ch = it.open()) {
                    exige(new String(leer(ch, 5)).equals("%PDF-"), "la cabeza");
                    ch.position(ch.size() - 3);
                    exige(Arrays.equals(resto(ch), Arrays.copyOfRange(a, a.length - 3, a.length)), "los tres últimos");
                }
                return "sin tamaño en el listado: se aprende de la respuesta (o de un byte) y position() desde el final va";
            });
            caso(13, () -> {
                // Otra colección: la rama va, y la ficha no se vuelve a preguntar.
                Banco.mando("registro", "limpiar", true);
                Media.Collection otra = Ore.collection("legal.archivo.contratos");
                otra.stat("a.pdf");
                otra.stat("b.pdf");
                List<List<Object>> serve = registro("serve");
                long fichas = serve.stream().filter(r -> "/puestos/p1".equals(r.get(1))).count();
                for (List<Object> r : serve)
                    if (String.valueOf(r.get(1)).startsWith("/media/"))
                        exige("r1/trabajo".equals(cabeceras(r).get("x-ore-rama")), "sin x-ore-rama: " + r.get(1));
                exige(fichas == 0 && fichasAlEmpezar == 1, "la ficha se preguntó " + fichasAlEmpezar + " veces al empezar y " + fichas + " después");
                return "la rama del puesto va en x-ore-rama (la ficha, preguntada una vez en el proceso)";
            });
        } finally {
            ParaElBanco.credencial(null);
            Banco.mando("rama", "rama", null);
        }
        return fallos;
    }
}
