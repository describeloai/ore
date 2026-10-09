package conformidad;

import java.net.URI;
import java.net.http.HttpClient;
import java.net.http.HttpRequest;
import java.net.http.HttpResponse;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.time.Duration;
import java.util.Base64;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

import ore.Json;
import ore.Ore;

/** El mando del banco (`POST /_banco/…`), una petición cruda a la celda, y la muestra. */
final class Banco {
    private Banco() {}

    static final HttpClient HTTP = HttpClient.newBuilder().version(HttpClient.Version.HTTP_1_1).build();
    static final double ESCALA = Double.parseDouble(System.getenv().getOrDefault("JM_ESCALA", "0.05"));

    /** {@code POST /_banco/<orden>} con un cuerpo JSON; el cuerpo de la respuesta. */
    static Map<String, Object> mando(String orden, Map<String, Object> cuerpo) throws Exception {
        HttpRequest q = HttpRequest.newBuilder(URI.create(Ore.puesto.servidor + "/_banco/" + orden))
            .header("content-type", "application/json")
            .POST(HttpRequest.BodyPublishers.ofString(Json.escribir(cuerpo == null ? Map.of() : cuerpo))).build();
        HttpResponse<String> r = HTTP.send(q, HttpResponse.BodyHandlers.ofString());
        if (r.statusCode() >= 300) throw new AssertionError("el banco no supo `" + orden + "`: " + r.statusCode() + " " + r.body());
        return Json.objeto(r.body());
    }

    static Map<String, Object> mando(String orden, Object... kv) throws Exception {
        Map<String, Object> m = new LinkedHashMap<>();
        for (int i = 0; i < kv.length; i += 2) m.put((String) kv[i], kv[i + 1]);
        return mando(orden, m);
    }

    /** Una respuesta cruda de la celda: lo que el servidor manda, no lo que el SDK ve. */
    record Crudo(int status, Map<String, List<String>> cabeceras, String cuerpo) {
        String todo() { return cabeceras + "\n" + cuerpo; }
    }

    static Crudo crudo(String metodo, String ruta, String cuerpo) throws Exception {
        HttpRequest.Builder b = HttpRequest.newBuilder(URI.create(Ore.puesto.servidor + ruta)).timeout(Duration.ofSeconds(30))
            .header("x-ore-puesto", Ore.puesto.id);
        b.method(metodo, cuerpo == null ? HttpRequest.BodyPublishers.noBody() : HttpRequest.BodyPublishers.ofString(cuerpo));
        HttpResponse<String> r = HTTP.send(b.build(), HttpResponse.BodyHandlers.ofString());
        return new Crudo(r.statusCode(), r.headers().map(), r.body());
    }

    /** Los bytes de un objeto de la colección de siempre del banco (`OBJETOS`). */
    static byte[] objeto(String path) throws Exception {
        return Base64.getDecoder().decode(String.valueOf(mando("objeto", "path", path).get("base64")));
    }

    /** Una celda que genera `builds.rs`, en `target/celdas/` de la raíz del árbol (`ORE_RAIZ`: `/src` en Docker). */
    static Path celda(String nombre) {
        return Path.of(System.getenv().getOrDefault("ORE_RAIZ", "/src"), "target", "celdas", nombre);
    }

    // ── la muestra ──────────────────────────────────────────────────────────

    static Map<String, Object> muestra;

    @SuppressWarnings("unchecked")
    static void leerMuestra(Path raiz) throws Exception {
        muestra = Json.objeto(Files.readString(raiz.resolve("muestra.json")));
    }

    /** Los bytes de un ítem de la muestra (la versión {@code id}, o la última). */
    @SuppressWarnings("unchecked")
    static byte[] bytes(String path, String id) {
        for (Object o : (List<Object>) muestra.get("items")) {
            Map<String, Object> it = (Map<String, Object>) o;
            if (it.get("generar") instanceof Map<?, ?> g) {
                String pre = String.valueOf(g.get("prefijo"));
                if (path.startsWith(pre)) {
                    int i = Integer.parseInt(path.substring(pre.length()).replaceAll("\\D", ""));
                    return String.valueOf(g.get("texto")).replace("{i}", String.valueOf(i)).getBytes(StandardCharsets.UTF_8);
                }
                continue;
            }
            if (!path.equals(it.get("path"))) continue;
            if (it.get("texto") != null) return String.valueOf(it.get("texto")).getBytes(StandardCharsets.UTF_8);
            if (it.get("base64") != null) return Base64.getDecoder().decode(String.valueOf(it.get("base64")));
            List<Object> vs = (List<Object>) it.get("versiones");
            Map<String, Object> v = (Map<String, Object>) vs.get(vs.size() - 1);
            for (Object x : vs) if (id != null && id.equals(((Map<String, Object>) x).get("id"))) v = (Map<String, Object>) x;
            return String.valueOf(v.get("texto")).getBytes(StandardCharsets.UTF_8);
        }
        throw new AssertionError("la muestra no tiene `" + path + "`");
    }

    /** Cuántos ítems tiene la muestra (los generados incluidos). */
    @SuppressWarnings("unchecked")
    static int total() {
        int n = 0;
        for (Object o : (List<Object>) muestra.get("items")) {
            Map<String, Object> it = (Map<String, Object>) o;
            n += it.get("generar") instanceof Map<?, ?> g ? ((Number) g.get("n")).intValue() : 1;
        }
        return n;
    }

    static void dormir(double segundosDelCaso) throws InterruptedException {
        Thread.sleep(Math.max(1, (long) (segundosDelCaso * ESCALA * 1000)));
    }
}
