package libro;

import com.zaxxer.hikari.HikariConfig;
import com.zaxxer.hikari.HikariDataSource;

import java.io.BufferedWriter;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardOpenOption;
import java.sql.Connection;
import java.sql.ResultSet;
import java.sql.Statement;
import java.time.Instant;
import java.util.Comparator;

/**
 * libro-informes (ADR 0058, P7·1): Java, JDBC y HikariCP, directo al cómputo (TCP).
 *
 * <p>Cada INTERVALO segundos, el cierre del día: por cuenta, entradas, salidas y saldo, en la tabla
 * {@code cierre}; y una exportación de todos los movimientos a CSV, leída con cursor (fetchSize),
 * que es una consulta larga. Mientras corre, la base no debe dormirse.
 *
 * <p>El pool no guarda conexiones ociosas ({@code minimumIdle} 0): entre cierre y cierre la base
 * puede dormir, y una conexión guardada estaría muerta. Si lo está, Hikari la descarta al pedirla.
 * Cada cierre, una línea JSON en /datos/informes.jsonl y por la salida.
 */
public final class Informes {
    private static final Path DATOS = Path.of("/datos");

    public static void main(String[] args) throws Exception {
        long intervalo = (long) (Double.parseDouble(env("INTERVALO", "300")) * 1000);
        HikariConfig c = new HikariConfig();
        c.setJdbcUrl(System.getenv("JDBC_URL"));
        c.setUsername(System.getenv("PGUSER"));
        c.setPassword(System.getenv("PGPASSWORD"));
        c.setMaximumPoolSize(2);
        c.setMinimumIdle(0);
        c.setIdleTimeout(30_000);
        c.setConnectionTimeout(90_000);
        c.setInitializationFailTimeout(-1);
        c.setPoolName("libro-informes");
        boolean alAnochecer = "1".equals(System.getenv("AL_ANOCHECER"));
        try (HikariDataSource ds = new HikariDataSource(c)) {
            while (true) {
                if (alAnochecer) {
                    Thread.sleep(hastaElAnochecer());
                }
                apunta(cierre(ds));
                if (!alAnochecer) {
                    Thread.sleep(intervalo);
                }
            }
        }
    }

    /**
     * P7·2: con AL_ANOCHECER=1, el cierre se hace al empezar cada noche del reloj de la corrida
     * (/datos/reloj.json, el de los usuarios), como el cierre de un día de verdad. → ms de espera.
     */
    private static long hastaElAnochecer() throws Exception {
        Path ruta = DATOS.resolve("reloj.json");
        while (!Files.exists(ruta)) {
            Thread.sleep(1000);
        }
        String j = Files.readString(ruta);
        double origen = numero(j, "origen"), dia = numero(j, "dia_s"), noche = numero(j, "noche_s");
        double ahora = System.currentTimeMillis() / 1000.0, largo = dia + noche;
        double anochecer = origen + Math.floor((ahora - origen) / largo) * largo + dia;
        if (anochecer <= ahora) {
            anochecer += largo;
        }
        return (long) ((anochecer - ahora) * 1000) + 2000;
    }

    private static double numero(String json, String clave) {
        var m = java.util.regex.Pattern.compile("\"" + clave + "\"\\s*:\\s*([0-9.eE+-]+)").matcher(json);
        if (!m.find()) {
            throw new IllegalStateException("reloj.json sin " + clave);
        }
        return Double.parseDouble(m.group(1));
    }

    private static String cierre(HikariDataSource ds) {
        long t0 = System.nanoTime();
        try (Connection k = ds.getConnection()) {
            long conectar = (System.nanoTime() - t0) / 1_000_000;
            int cuentas;
            try (Statement s = k.createStatement()) {
                cuentas = s.executeUpdate("""
                    insert into cierre (dia, cuenta, entradas, salidas, saldo_final)
                    select current_date, c.id,
                           coalesce(sum(m.importe) filter (where m.importe > 0 and m.cuando >= current_date), 0),
                           coalesce(-sum(m.importe) filter (where m.importe < 0 and m.cuando >= current_date), 0),
                           c.saldo
                      from cuenta c left join movimiento m on m.cuenta = c.id
                     group by c.id, c.saldo
                    on conflict (dia, cuenta) do update
                       set entradas = excluded.entradas, salidas = excluded.salidas,
                           saldo_final = excluded.saldo_final, hecho = now()""");
            }
            long filas = exporta(k);
            long ms = (System.nanoTime() - t0) / 1_000_000;
            return String.format("{\"t\":\"%s\",\"ok\":true,\"cuentas\":%d,\"exportadas\":%d,\"conectar_ms\":%d,\"ms\":%d}",
                Instant.now(), cuentas, filas, conectar, ms);
        } catch (Exception e) {
            String m = String.valueOf(e.getMessage()).replace("\\", "\\\\").replace("\"", "'").replace("\n", " ");
            return String.format("{\"t\":\"%s\",\"ok\":null,\"error\":\"%s: %s\",\"ms\":%d}", Instant.now(),
                e.getClass().getSimpleName(), m.length() > 300 ? m.substring(0, 300) : m, (System.nanoTime() - t0) / 1_000_000);
        }
    }

    /** Todos los movimientos, con cursor: autocommit apagado y fetchSize, como manda pgjdbc. */
    private static long exporta(Connection k) throws Exception {
        Path dir = DATOS.resolve("export");
        Files.createDirectories(dir);
        Path f = dir.resolve("movimientos-" + Instant.now().toEpochMilli() + ".csv");
        long n = 0;
        k.setAutoCommit(false);
        try (Statement s = k.createStatement(); BufferedWriter w = Files.newBufferedWriter(f, StandardCharsets.UTF_8)) {
            s.setFetchSize(1000);
            w.write("id,cuenta,importe,transferencia,aviso,cuando\n");
            try (ResultSet r = s.executeQuery("select id, cuenta, importe, transferencia, aviso, cuando from movimiento order by id")) {
                while (r.next()) {
                    w.write(r.getLong(1) + "," + r.getString(2) + "," + r.getLong(3) + "," + nulo(r.getString(4)) + ","
                        + nulo(r.getString(5)) + "," + r.getTimestamp(6).toInstant() + "\n");
                    n++;
                }
            }
            k.commit();
        } finally {
            k.setAutoCommit(true);
        }
        // Se guardan las tres últimas.
        try (var todos = Files.list(dir)) {
            todos.sorted(Comparator.reverseOrder()).skip(3).forEach(p -> p.toFile().delete());
        }
        return n;
    }

    private static String nulo(String s) {
        return s == null ? "" : s;
    }

    private static void apunta(String linea) throws Exception {
        System.out.println(linea);
        Files.writeString(DATOS.resolve("informes.jsonl"), linea + "\n", StandardCharsets.UTF_8,
            StandardOpenOption.CREATE, StandardOpenOption.APPEND);
    }

    private static String env(String k, String d) {
        String v = System.getenv(k);
        return v == null || v.isEmpty() ? d : v;
    }
}
