package ore;

import java.util.List;
import java.util.Map;
import java.util.function.Supplier;

/**
 * Lo que el ejecutor de la suite necesita del paquete {@code ore} y una celda no:
 * poner la credencial como la pone el agente (0049 D3). Sólo existe en el
 * laboratorio ({@code la-media-en-java.py}); la imagen no lo lleva.
 */
public final class ParaElBanco {
    private ParaElBanco() {}

    /** Como {@code Agente}: quien da la credencial en cada petición, o {@code null}. */
    public static void credencial(Supplier<Map<String, String>> s) { Ore.puesto.credencial = s; }

    /** Otra referencia para un ítem (sin tamaño, otro digest), como la prueba de Python. */
    public static void ref(Media.Item it, Media.MediaRef r) { it.ref = r; }

    /** Lo que la sesión leyó (fuera de un transform), y vaciarlo. */
    public static List<String> leidas(boolean vaciar) {
        List<String> l = List.copyOf(Ore.leidas);
        if (vaciar) Ore.leidas.clear();
        return l;
    }

    /** La procedencia que llevaría lo que se escribiera en {@code nombre}. */
    public static Map<String, Object> procedencia(String nombre) { return Ore.procedencia(nombre); }

    /** Una celda, como la corre el puesto: el kernel del agente (JShell), con su informe. */
    public static Map<String, Object> celda(String texto) { return new Agente.Kernel().correr(texto, "java"); }

    /** Una derivación cruda en el commit de {@code t} (lo que {@code apply()} hace por dentro): los casos del linaje que no cuadra. */
    public static void derivacion(Media.Transaction t, Map<String, Object> d) {
        synchronized (t.linaje) { t.linaje.get("derivations").add(d); }
    }

    /** Lo que corre tras cada commit de {@code apply()} ({@code derivar-008}: cortar una pasada), o {@code null}. */
    public static void trasConfirmar(Runnable r) { Media.trasConfirmar = r; }

    /** D-JM2: la versión que {@code apply()} da a una función que no la dice. */
    public static String version(Object fn) { return Media.versionDe(fn); }

    /** El error del contrato que el SDK da a una respuesta {@code (status, type)}. */
    public static Media.MediaError error(int status, String type) { return Media.error(status, Map.of("type", type), "x"); }

    /** El {@code Repr-Digest} de la próxima subida, en vez del suyo ({@code put-004}). */
    public static void reprDigest(String d) { Media.reprDigestForzado = d; }

    /** Arma un Preview como el arnés ({@code Ore.ensayo}), y lo desarma. */
    public static void ensayo(String output, String transform) { Ore.ensayo(output, transform); }

    public static Object finDelEnsayo() { return Ore.finDelEnsayo(); }

    /** El commit del que un build dice salir ({@code ORE_CODIGO}). */
    public static void codigo(String c) { Ore.CODIGO = c; }

    /** Los umbrales de la lectura por rangos, para probarla con un ítem pequeño. */
    public static void rangos(long desde, long trozo) {
        Media.EN_PARALELO_DESDE = desde;
        Media.TROZO = trozo;
    }
}
