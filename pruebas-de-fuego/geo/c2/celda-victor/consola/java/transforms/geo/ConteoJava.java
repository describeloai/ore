package geo;

import static ore.Ore.*;

import ore.Transform;

/** ORE 0068 6c · el join espacial en Java (Heavyweight): sql() da un Dataset&lt;Row&gt; de Spark. */
public class ConteoJava {
    @Transform(inputs = {"geo.zonas_20k", "geo.puntos_20m"}, output = "geo.conteo_java")
    public static Object conteo() throws Exception {
        return write("geo.conteo_java", sql(
            "SELECT z.id, count(*) AS n FROM geo.zonas_20k z JOIN geo.puntos_20m p ON ST_Contains(z.zona, p.geom) GROUP BY z.id"));
    }
}
