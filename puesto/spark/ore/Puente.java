package ore;

/**
 * ORE 0068 C2·3 pieza 4 · <b>Lo que el Java de un build Heavyweight pide al SDK de Python</b>, que
 * lo implementa (py4j, al revés: {@code ore/spark.py}, {@code _Puente}). Lo de ORE —la geometría con
 * su CRS, la procedencia, el techo— vive en un solo sitio, el SDK de Python; Java pasa los
 * {@code Dataset<Row>} por referencia (ni se copian ni se serializan). Interno.
 */
public interface Puente {
    /** {@code ore.over(nombre)}: el {@code Dataset<Row>} (un {@code _jdf}). */
    Object over(String nombre);

    /** {@code ore.sql(consulta)}: el {@code Dataset<Row>}. */
    Object sql(String consulta);

    /** {@code ore.write(nombre, datos, mode)}: lo que devuelve, en JSON. */
    String write(String nombre, Object datos, String modo);
}
