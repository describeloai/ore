package ore;

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

    /** Los umbrales de la lectura por rangos, para probarla con un ítem pequeño. */
    public static void rangos(long desde, long trozo) {
        Media.EN_PARALELO_DESDE = desde;
        Media.TROZO = trozo;
    }
}
