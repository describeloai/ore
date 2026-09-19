package ore;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * JSON a pelo (0031 W3.4): lo justo para hablar con {@code ore-serve} y con el
 * emisor sin traer una librería a la imagen. Lee objetos, listas, cadenas,
 * números, booleanos y {@code null}; escribe {@code Map}, {@code List},
 * {@code String}, {@code Number}, {@code Boolean} y {@code null}.
 */
public final class Json {
    private Json() {}

    // ── escribir ────────────────────────────────────────────────────────────
    public static String escribir(Object v) {
        StringBuilder b = new StringBuilder();
        escribir(v, b);
        return b.toString();
    }

    @SuppressWarnings("unchecked")
    private static void escribir(Object v, StringBuilder b) {
        if (v == null) { b.append("null"); return; }
        if (v instanceof String s) { cadena(s, b); return; }
        if (v instanceof Boolean || v instanceof Integer || v instanceof Long || v instanceof Short || v instanceof Byte) { b.append(v); return; }
        if (v instanceof Double d) { if (d.isNaN() || d.isInfinite()) b.append("null"); else b.append(d); return; }
        if (v instanceof Float f) { if (f.isNaN() || f.isInfinite()) b.append("null"); else b.append(f); return; }
        if (v instanceof Number n) { b.append(n); return; }
        if (v instanceof Map<?, ?> m) {
            b.append('{');
            boolean primero = true;
            for (Map.Entry<?, ?> e : m.entrySet()) {
                if (!primero) b.append(',');
                primero = false;
                cadena(String.valueOf(e.getKey()), b);
                b.append(':');
                escribir(e.getValue(), b);
            }
            b.append('}');
            return;
        }
        if (v instanceof Iterable<?> it) {
            b.append('[');
            boolean primero = true;
            for (Object x : it) {
                if (!primero) b.append(',');
                primero = false;
                escribir(x, b);
            }
            b.append(']');
            return;
        }
        if (v instanceof Object[] a) { escribir(List.of(a), b); return; }
        cadena(String.valueOf(v), b);
    }

    private static void cadena(String s, StringBuilder b) {
        b.append('"');
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            switch (c) {
                case '"' -> b.append("\\\"");
                case '\\' -> b.append("\\\\");
                case '\n' -> b.append("\\n");
                case '\r' -> b.append("\\r");
                case '\t' -> b.append("\\t");
                case '\b' -> b.append("\\b");
                case '\f' -> b.append("\\f");
                default -> {
                    if (c < 0x20) b.append(String.format("\\u%04x", (int) c));
                    else b.append(c);
                }
            }
        }
        b.append('"');
    }

    // ── leer ────────────────────────────────────────────────────────────────
    /** Un documento JSON → {@code Map}/{@code List}/{@code String}/{@code Long}/{@code Double}/{@code Boolean}/{@code null}. */
    public static Object leer(String texto) {
        Lector l = new Lector(texto);
        Object v = l.valor();
        l.blancos();
        if (l.i != texto.length()) throw new IllegalArgumentException("JSON con cola en " + l.i);
        return v;
    }

    /** {@code leer} como objeto, o un mapa vacío si no lo es. */
    @SuppressWarnings("unchecked")
    public static Map<String, Object> objeto(String texto) {
        try {
            Object v = leer(texto);
            return v instanceof Map ? (Map<String, Object>) v : new LinkedHashMap<>();
        } catch (RuntimeException e) {
            Map<String, Object> m = new LinkedHashMap<>();
            m.put("error", texto.strip());
            return m;
        }
    }

    private static final class Lector {
        final String s;
        int i = 0;

        Lector(String s) { this.s = s; }

        void blancos() { while (i < s.length() && Character.isWhitespace(s.charAt(i))) i++; }

        Object valor() {
            blancos();
            if (i >= s.length()) throw new IllegalArgumentException("JSON truncado");
            char c = s.charAt(i);
            if (c == '{') return objeto();
            if (c == '[') return lista();
            if (c == '"') return cadena();
            if (s.startsWith("true", i)) { i += 4; return Boolean.TRUE; }
            if (s.startsWith("false", i)) { i += 5; return Boolean.FALSE; }
            if (s.startsWith("null", i)) { i += 4; return null; }
            return numero();
        }

        Map<String, Object> objeto() {
            Map<String, Object> m = new LinkedHashMap<>();
            i++;
            blancos();
            if (s.charAt(i) == '}') { i++; return m; }
            while (true) {
                blancos();
                String k = cadena();
                blancos();
                if (s.charAt(i) != ':') throw new IllegalArgumentException("se esperaba ':' en " + i);
                i++;
                m.put(k, valor());
                blancos();
                char c = s.charAt(i++);
                if (c == '}') return m;
                if (c != ',') throw new IllegalArgumentException("se esperaba ',' o '}' en " + (i - 1));
            }
        }

        List<Object> lista() {
            List<Object> l = new ArrayList<>();
            i++;
            blancos();
            if (s.charAt(i) == ']') { i++; return l; }
            while (true) {
                l.add(valor());
                blancos();
                char c = s.charAt(i++);
                if (c == ']') return l;
                if (c != ',') throw new IllegalArgumentException("se esperaba ',' o ']' en " + (i - 1));
            }
        }

        String cadena() {
            if (s.charAt(i) != '"') throw new IllegalArgumentException("se esperaba una cadena en " + i);
            i++;
            StringBuilder b = new StringBuilder();
            while (true) {
                char c = s.charAt(i++);
                if (c == '"') return b.toString();
                if (c != '\\') { b.append(c); continue; }
                char e = s.charAt(i++);
                switch (e) {
                    case 'n' -> b.append('\n');
                    case 't' -> b.append('\t');
                    case 'r' -> b.append('\r');
                    case 'b' -> b.append('\b');
                    case 'f' -> b.append('\f');
                    case 'u' -> { b.append((char) Integer.parseInt(s.substring(i, i + 4), 16)); i += 4; }
                    default -> b.append(e);
                }
            }
        }

        Object numero() {
            int j = i;
            while (i < s.length() && "+-0123456789.eE".indexOf(s.charAt(i)) >= 0) i++;
            String t = s.substring(j, i);
            if (t.isEmpty()) throw new IllegalArgumentException("JSON inesperado en " + j);
            if (t.contains(".") || t.contains("e") || t.contains("E")) return Double.parseDouble(t);
            try { return Long.parseLong(t); } catch (NumberFormatException e) { return Double.parseDouble(t); }
        }
    }
}
