package ore;

/** El previo de la pieza 4: lo que Java pide a Python (py4j, al revés). */
public interface Puente {
    Object over(String nombre);

    String write(String nombre, Object datos, String modo);
}
