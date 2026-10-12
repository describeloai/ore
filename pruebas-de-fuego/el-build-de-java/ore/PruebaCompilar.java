package ore;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.LinkedHashMap;
import java.util.Map;

/**
 * <b>El build de Java compila el fichero del build y lo que usa, nada más</b>
 * (2026-10-12, {@link Arnes#compilar}). En el paquete {@code ore} para llamar al paso
 * de compilar tal cual, sin el puesto. Lo corre {@code el-build-de-java.py}.
 */
public final class PruebaCompilar {
    private PruebaCompilar() {}

    private static final String R = "packages/p/repo/";
    private static int fallos = 0;

    private static Map<String, Object> fuentes(String... pares) {
        Map<String, Object> m = new LinkedHashMap<>();
        for (int i = 0; i < pares.length; i += 2) m.put(R + pares[i], pares[i + 1]);
        return m;
    }

    private static void si(boolean cierto, String que) {
        System.out.println((cierto ? "  ✓ " : "  ✗ ") + que);
        if (!cierto) fallos++;
    }

    public static void main(String[] args) throws Exception {
        // 1 · un transform sano, y en el repositorio un snippet de JShell que nadie usa
        //     (la semilla v6): antes, «unnamed classes are a preview feature».
        Path d = Files.createTempDirectory("prueba-compilar");
        Path c = Arnes.compilar(R + "transforms/Sano.java", fuentes(
            "transforms/Sano.java",
            "import static ore.Ore.*;\nimport ore.Transform;\npublic class Sano {\n"
                + "    @Transform(inputs = {}, output = \"b.s.n\")\n"
                + "    public static Object sano() throws Exception { return null; }\n}\n",
            "transforms/Ejemplo.java", "var x = 1;\nSystem.out.println(x);\n"), d);
        si(Files.exists(c.resolve("Sano.class")), "un snippet suelto que nadie usa no tumba el build");
        si(!Files.exists(c.resolve("Ejemplo.class")), "y no se compila");

        // 2 · lo que usa, de otra carpeta del repositorio, sí se compila.
        c = Arnes.compilar(R + "transforms/Usa.java", fuentes(
            "transforms/Usa.java", "public class Usa { public static int f() { return Ayuda.dos(); } }\n",
            "comun/Ayuda.java", "public class Ayuda { static int dos() { return 2; } }\n",
            "transforms/Ejemplo.java", "var x = 1;\n"), Files.createTempDirectory("prueba-compilar"));
        si(Files.exists(c.resolve("Usa.class")) && Files.exists(c.resolve("Ayuda.class")), "lo que usa, de otra carpeta, se compila con él");

        // 3 · si lo que usa no compila, el error es suyo: su fichero y su línea.
        try {
            Arnes.compilar(R + "transforms/Usa.java", fuentes(
                "transforms/Usa.java", "public class Usa { public static int f() { return Roto.x(); } }\n",
                "comun/Roto.java", "public class Roto {\n    static int x() { return ; }\n}\n"), Files.createTempDirectory("prueba-compilar"));
            si(false, "un fichero roto que se usa falla");
        } catch (IllegalStateException e) {
            String m = e.getMessage();
            si(m.startsWith("CompilationError:") && m.endsWith("(" + R + "comun/Roto.java, line 2)"), "un fichero roto que se usa falla con su fichero y su línea: " + m);
        }

        // 4 · con `package`: el sitio de cada uno lo da su paquete, no su carpeta.
        c = Arnes.compilar(R + "etl/Indice.java", fuentes(
            "etl/Indice.java", "package etl;\npublic class Indice { public static int f() { return Util.u(); } }\n",
            "otra/carpeta/Util.java", "package etl;\npublic class Util { static int u() { return 1; } }\n"),
            Files.createTempDirectory("prueba-compilar"));
        si(Files.exists(c.resolve("etl/Indice.class")) && Files.exists(c.resolve("etl/Util.class")), "con `package`, por su paquete");

        // 5 · y el propio fichero roto, con su línea, como siempre.
        try {
            Arnes.compilar(R + "transforms/Mal.java", fuentes(
                "transforms/Mal.java", "public class Mal {\n  int x = ;\n}\n"), Files.createTempDirectory("prueba-compilar"));
            si(false, "el fichero del build roto falla");
        } catch (IllegalStateException e) {
            si(e.getMessage().endsWith("(" + R + "transforms/Mal.java, line 2)"), "el fichero del build roto falla con su línea");
        }

        System.out.println(fallos == 0 ? "el build de Java: 5/5" : "✗ " + fallos + " fallo(s)");
        System.exit(fallos == 0 ? 0 : 1);
    }
}
