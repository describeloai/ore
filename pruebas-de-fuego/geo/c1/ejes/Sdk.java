import static ore.Ore.*;

/** ORE 0068 · cierre de C1 · Los ejes por la API del SDK de Java ({@code sql}), sin servidor. */
public class Sdk {
    public static void main(String[] a) throws Exception {
        Object p = sql("select ST_AsText(ST_Transform(ST_Point(440000, 4474000), 'EPSG:25830', 'EPSG:4326')) p").get(0).get("p");
        double d = ((Number) sql("select ST_Distance_Spheroid(ST_Point(-3.7072, 40.4145), ST_Point(2.1734, 41.3851)) / 1000 d").get(0).get("d")).doubleValue();
        boolean ok = String.valueOf(p).startsWith("POINT (-3.70") && d > 500 && d < 515;
        System.out.printf("%s · java · %s · Sol–Barcelona %.1f km%n", ok ? "OK   " : "FALLA", p, d);
    }
}
